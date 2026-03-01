use crate::conversation::message::{ActionRequiredData, Message, MessageContent, MessageMetadata};
use crate::conversation::Conversation;
use crate::prompt_template::render_global_file;
use crate::providers::base::{Provider, ProviderUsage};
use crate::providers::errors::ProviderError;
use crate::{config::Config, token_counter::create_token_counter};
use anyhow::Result;
use rmcp::model::Role;
use serde::Serialize;
use tracing::{debug, info, warn};

mod progressive_memory;

pub const DEFAULT_COMPACTION_THRESHOLD: f64 = 0.8;
pub const CONTEXT_COMPACTION_STRATEGY_KEY: &str = "AGIME_CONTEXT_COMPACTION_STRATEGY";
pub const CFPM_V2_EXTRACTION_KEY: &str = "AGIME_CFPM_V2_EXTRACTION";
pub const CFPM_V2_INJECTION_KEY: &str = "AGIME_CFPM_V2_INJECTION";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextCompactionStrategy {
    LegacySegmented,
    CfpmMemoryV1,
    CfpmMemoryV2,
}

impl ContextCompactionStrategy {
    pub fn from_config() -> Self {
        let config = Config::global();
        let raw = config
            .get_param::<String>(CONTEXT_COMPACTION_STRATEGY_KEY)
            .unwrap_or_else(|_| "legacy".to_string());
        Self::from_config_value(&raw)
    }

    pub fn from_config_value(value: &str) -> Self {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "cfpm" | "cfpm_memory" | "cfpm_memory_v1" | "progressive" | "progressive_memory"
            | "new" => Self::CfpmMemoryV1,
            "cfpm_memory_v2" | "v2" => Self::CfpmMemoryV2,
            _ => Self::LegacySegmented,
        }
    }

    pub fn as_config_value(self) -> &'static str {
        match self {
            Self::LegacySegmented => "legacy_segmented",
            Self::CfpmMemoryV1 => "cfpm_memory_v1",
            Self::CfpmMemoryV2 => "cfpm_memory_v2",
        }
    }

    pub fn is_cfpm(&self) -> bool {
        matches!(self, Self::CfpmMemoryV1 | Self::CfpmMemoryV2)
    }
}

pub fn current_compaction_strategy() -> ContextCompactionStrategy {
    ContextCompactionStrategy::from_config()
}

pub async fn compact_messages_with_strategy(
    provider: &dyn Provider,
    conversation: &Conversation,
    manual_compact: bool,
    strategy: ContextCompactionStrategy,
) -> Result<(Conversation, ProviderUsage)> {
    match strategy {
        ContextCompactionStrategy::LegacySegmented => {
            compact_messages(provider, conversation, manual_compact).await
        }
        ContextCompactionStrategy::CfpmMemoryV1 | ContextCompactionStrategy::CfpmMemoryV2 => {
            progressive_memory::compact_messages_progressive_memory(conversation, manual_compact)
                .await
        }
    }
}

pub async fn compact_messages_with_active_strategy(
    provider: &dyn Provider,
    conversation: &Conversation,
    manual_compact: bool,
) -> Result<(Conversation, ProviderUsage)> {
    compact_messages_with_strategy(
        provider,
        conversation,
        manual_compact,
        current_compaction_strategy(),
    )
    .await
}

// Segmented compaction strategy constants
const MIN_MESSAGES_TO_COMPACT: usize = 20; // Don't compact if fewer messages
const MIN_KEEP_FIRST_MESSAGES: usize = 2;
const MIN_KEEP_LAST_MESSAGES: usize = 6;
const HEAD_TOKEN_BUDGET_RATIO: f64 = 0.12;
const TAIL_TOKEN_BUDGET_RATIO: f64 = 0.30;
const MIN_HEAD_TOKEN_BUDGET: usize = 256;
const MIN_TAIL_TOKEN_BUDGET: usize = 768;
const FALLBACK_KEEP_FIRST_MESSAGES: usize = 3;
const FALLBACK_KEEP_LAST_MESSAGES: usize = 10;
const TOOL_REQUEST_ARG_CHAR_LIMIT: usize = 1200;
const TOOL_RESPONSE_TEXT_CHAR_LIMIT: usize = 2000;

const CONVERSATION_CONTINUATION_TEXT: &str =
    "The previous message contains a summary that was prepared because a context limit was reached.
Do not mention that you read a summary or that conversation summarization occurred.
Just continue the conversation naturally based on the summarized context";

const MANUAL_COMPACT_CONTINUATION_TEXT: &str =
    "The previous message contains a summary that was prepared at the user's request.
Do not mention that you read a summary or that conversation summarization occurred.
Just continue the conversation naturally based on the summarized context";

#[derive(Serialize)]
struct SummarizeContext {
    messages: String,
}

fn fallback_segmented_keep_counts(total: usize) -> (usize, usize) {
    let keep_first = FALLBACK_KEEP_FIRST_MESSAGES.min(total);
    let keep_last = if total > keep_first + FALLBACK_KEEP_LAST_MESSAGES {
        FALLBACK_KEEP_LAST_MESSAGES
    } else {
        total.saturating_sub(keep_first)
    };
    (keep_first, keep_last)
}

async fn compute_segmented_keep_counts(
    provider: &dyn Provider,
    messages: &[Message],
) -> Result<(usize, usize)> {
    let total = messages.len();
    if total == 0 {
        return Ok((0, 0));
    }

    let context_limit = provider.get_model_config().context_limit().max(1);
    let token_counter = create_token_counter()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create token counter: {}", e))?;

    let token_counts: Vec<usize> = messages
        .iter()
        .map(|msg| {
            token_counter
                .count_chat_tokens("", std::slice::from_ref(msg), &[])
                .max(1)
        })
        .collect();

    let head_budget = ((context_limit as f64 * HEAD_TOKEN_BUDGET_RATIO).round() as usize)
        .max(MIN_HEAD_TOKEN_BUDGET);
    let tail_budget = ((context_limit as f64 * TAIL_TOKEN_BUDGET_RATIO).round() as usize)
        .max(MIN_TAIL_TOKEN_BUDGET);

    let mut keep_first = 0usize;
    let mut head_tokens = 0usize;
    while keep_first < total {
        head_tokens = head_tokens.saturating_add(token_counts[keep_first]);
        keep_first += 1;

        let must_leave = MIN_KEEP_LAST_MESSAGES.saturating_add(1);
        if keep_first >= total.saturating_sub(must_leave) {
            break;
        }

        if keep_first >= MIN_KEEP_FIRST_MESSAGES && head_tokens >= head_budget {
            break;
        }
    }

    let mut keep_last = 0usize;
    let mut tail_tokens = 0usize;
    while keep_last < total.saturating_sub(keep_first) {
        let idx = total.saturating_sub(1 + keep_last);
        tail_tokens = tail_tokens.saturating_add(token_counts[idx]);
        keep_last += 1;

        if keep_first + keep_last >= total.saturating_sub(1) {
            break;
        }

        if keep_last >= MIN_KEEP_LAST_MESSAGES && tail_tokens >= tail_budget {
            break;
        }
    }

    if keep_first + keep_last >= total {
        if keep_last > MIN_KEEP_LAST_MESSAGES {
            keep_last = keep_last.saturating_sub(1);
        } else if keep_first > MIN_KEEP_FIRST_MESSAGES {
            keep_first = keep_first.saturating_sub(1);
        } else if keep_last > 0 {
            keep_last = keep_last.saturating_sub(1);
        }
    }

    Ok((
        keep_first.min(total),
        keep_last.min(total.saturating_sub(keep_first)),
    ))
}

/// Compact messages by summarizing them
///
/// This function performs the actual compaction by summarizing messages and updating
/// their visibility metadata. It does not check thresholds - use `check_if_compaction_needed`
/// first to determine if compaction is necessary.
///
/// # Arguments
/// * `provider` - The provider to use for summarization
/// * `conversation` - The current conversation history
/// * `manual_compact` - If true, this is a manual compaction (don't preserve user message)
///
/// # Returns
/// * A tuple containing:
///   - `Conversation`: The compacted messages
///   - `ProviderUsage`: Provider usage from summarization
pub async fn compact_messages(
    provider: &dyn Provider,
    conversation: &Conversation,
    manual_compact: bool,
) -> Result<(Conversation, ProviderUsage)> {
    info!("Performing message compaction");

    let messages = conversation.messages();
    let total = messages.len();

    // If too few messages, skip compaction to avoid losing important context
    if total < MIN_MESSAGES_TO_COMPACT && !manual_compact {
        info!(
            "Skipping compaction: only {} messages (minimum: {})",
            total, MIN_MESSAGES_TO_COMPACT
        );
        return Ok((
            conversation.clone(),
            ProviderUsage::new("none".to_string(), crate::providers::base::Usage::default()),
        ));
    }

    // Segmented compaction strategy:
    // 1. Keep head and tail by token budget (adaptive, context-limit aware)
    // 2. Summarize middle section
    let (keep_first, keep_last) = match compute_segmented_keep_counts(provider, messages).await {
        Ok(counts) => counts,
        Err(e) => {
            warn!(
                "Token-budget segmented compaction fallback to fixed counts due to: {}",
                e
            );
            fallback_segmented_keep_counts(total)
        }
    };

    let middle_start = keep_first;
    let middle_end = total.saturating_sub(keep_last);

    // If no middle section to compress, return original
    if middle_end <= middle_start {
        info!("No middle section to compress, returning original");
        return Ok((
            conversation.clone(),
            ProviderUsage::new("none".to_string(), crate::providers::base::Usage::default()),
        ));
    }

    let first_messages: Vec<Message> = messages[..keep_first].to_vec();
    let middle_messages: &[Message] = &messages[middle_start..middle_end];
    let last_messages: Vec<Message> = messages[(total - keep_last)..].to_vec();

    info!(
        "Segmented compaction: keeping first {}, compressing middle {} (indices {}-{}), keeping last {}",
        first_messages.len(),
        middle_messages.len(),
        middle_start,
        middle_end,
        last_messages.len()
    );

    let (summary_message, summarization_usage) = do_compact(provider, middle_messages).await?;

    // Build final message list:
    // 1. First messages (original, user visible + agent visible)
    // 2. Summary of middle section (agent only)
    // 3. Continuation prompt (agent only)
    // 4. Last messages (original, user visible + agent visible)

    let mut final_messages = Vec::new();

    // Add first messages with full visibility
    for msg in &first_messages {
        final_messages.push(msg.clone());
    }

    // Mark middle messages as user-visible only (for history display)
    for msg in middle_messages {
        let updated_msg = msg
            .clone()
            .with_metadata(msg.metadata.with_agent_invisible());
        final_messages.push(updated_msg);
    }

    // Add summary message (agent only)
    let summary_msg = summary_message.with_metadata(MessageMetadata::agent_only());
    final_messages.push(summary_msg);

    // Add continuation prompt (agent only)
    let continuation_text = if manual_compact {
        MANUAL_COMPACT_CONTINUATION_TEXT
    } else {
        CONVERSATION_CONTINUATION_TEXT
    };
    let continuation_msg = Message::assistant()
        .with_text(continuation_text)
        .with_metadata(MessageMetadata::agent_only());
    final_messages.push(continuation_msg);

    // Add last messages with full visibility
    for msg in &last_messages {
        final_messages.push(msg.clone());
    }

    Ok((
        Conversation::new_unvalidated(final_messages),
        summarization_usage,
    ))
}

/// Check if messages exceed the auto-compaction threshold
pub async fn check_if_compaction_needed(
    provider: &dyn Provider,
    conversation: &Conversation,
    threshold_override: Option<f64>,
    session: &crate::session::Session,
) -> Result<bool> {
    let messages = conversation.messages();
    let config = Config::global();
    let threshold = threshold_override.unwrap_or_else(|| {
        config
            .get_param::<f64>("AGIME_AUTO_COMPACT_THRESHOLD")
            .unwrap_or(DEFAULT_COMPACTION_THRESHOLD)
    });

    let context_limit = provider.get_model_config().context_limit();

    let (current_tokens, token_source) = match session.total_tokens {
        Some(tokens) => (tokens as usize, "session metadata"),
        None => {
            let token_counter = create_token_counter()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to create token counter: {}", e))?;

            let token_counts: Vec<_> = messages
                .iter()
                .filter(|m| m.is_agent_visible())
                .map(|msg| token_counter.count_chat_tokens("", std::slice::from_ref(msg), &[]))
                .collect();

            (token_counts.iter().sum(), "estimated")
        }
    };

    let usage_ratio = current_tokens as f64 / context_limit as f64;

    let needs_compaction = if threshold <= 0.0 || threshold >= 1.0 {
        false // Auto-compact is disabled.
    } else {
        usage_ratio > threshold
    };

    debug!(
        "Compaction check: {} / {} tokens ({:.1}%), threshold: {:.1}%, needs compaction: {}, source: {}",
        current_tokens,
        context_limit,
        usage_ratio * 100.0,
        threshold * 100.0,
        needs_compaction,
        token_source
    );

    Ok(needs_compaction)
}

fn filter_tool_responses<'a>(messages: &[&'a Message], remove_percent: u32) -> Vec<&'a Message> {
    fn has_tool_response(msg: &Message) -> bool {
        msg.content
            .iter()
            .any(|c| matches!(c, MessageContent::ToolResponse(_)))
    }

    if remove_percent == 0 {
        return messages.to_vec();
    }

    let tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, msg)| has_tool_response(msg))
        .map(|(i, _)| i)
        .collect();

    if tool_indices.is_empty() {
        return messages.to_vec();
    }

    let num_to_remove = ((tool_indices.len() * remove_percent as usize) / 100).max(1);

    let middle = tool_indices.len() / 2;
    let mut indices_to_remove = Vec::new();

    // Middle out
    for i in 0..num_to_remove {
        if i % 2 == 0 {
            let offset = i / 2;
            if middle > offset {
                indices_to_remove.push(tool_indices[middle - offset - 1]);
            }
        } else {
            let offset = i / 2;
            if middle + offset < tool_indices.len() {
                indices_to_remove.push(tool_indices[middle + offset]);
            }
        }
    }

    messages
        .iter()
        .enumerate()
        .filter(|(i, _)| !indices_to_remove.contains(i))
        .map(|(_, msg)| *msg)
        .collect()
}

async fn do_compact(
    provider: &dyn Provider,
    messages: &[Message],
) -> Result<(Message, ProviderUsage), anyhow::Error> {
    let agent_visible_messages: Vec<&Message> = messages
        .iter()
        .filter(|msg| msg.is_agent_visible())
        .collect();

    // Try progressively removing more tool response messages from the middle to reduce context length
    let removal_percentages = [0, 10, 20, 50, 100];

    for (attempt, &remove_percent) in removal_percentages.iter().enumerate() {
        let filtered_messages = filter_tool_responses(&agent_visible_messages, remove_percent);

        let messages_text = filtered_messages
            .iter()
            .map(|&msg| format_message_for_compacting(msg))
            .collect::<Vec<_>>()
            .join("\n");

        let context = SummarizeContext {
            messages: messages_text,
        };

        let system_prompt = render_global_file("summarize_oneshot.md", &context)?;

        let user_message = Message::user()
            .with_text("Please summarize the conversation history provided in the system prompt.");
        let summarization_request = vec![user_message];

        match provider
            .complete_fast(&system_prompt, &summarization_request, &[])
            .await
        {
            Ok((mut response, mut provider_usage)) => {
                response.role = Role::User;

                provider_usage
                    .ensure_tokens(&system_prompt, &summarization_request, &response, &[])
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to ensure usage tokens: {}", e))?;

                return Ok((response, provider_usage));
            }
            Err(e) => {
                if matches!(e, ProviderError::ContextLengthExceeded(_)) {
                    if attempt < removal_percentages.len() - 1 {
                        continue;
                    } else {
                        return Err(anyhow::anyhow!(
                            "Failed to compact messages: context length still exceeded after {} attempts with maximum removal",
                            removal_percentages.len()
                        ));
                    }
                }
                return Err(e.into());
            }
        }
    }

    Err(anyhow::anyhow!(
        "Unexpected: exhausted all attempts without returning"
    ))
}

fn format_message_for_compacting(msg: &Message) -> String {
    fn truncate_chars(value: String, limit: usize) -> String {
        if limit == 0 {
            return String::new();
        }
        let current = value.chars().count();
        if current <= limit {
            return value;
        }
        let prefix: String = value.chars().take(limit).collect();
        let omitted = current.saturating_sub(limit);
        format!("{}... [truncated {} chars]", prefix, omitted)
    }

    let content_parts: Vec<String> = msg
        .content
        .iter()
        .map(|content| match content {
            MessageContent::Text(text) => text.text.clone(),
            MessageContent::Image(img) => format!("[image: {}]", img.mime_type),
            MessageContent::ToolRequest(req) => {
                if let Ok(call) = &req.tool_call {
                    let args = serde_json::to_string_pretty(&call.arguments)
                        .unwrap_or_else(|_| "<<invalid json>>".to_string());
                    format!(
                        "tool_request({}): {}",
                        call.name,
                        truncate_chars(args, TOOL_REQUEST_ARG_CHAR_LIMIT)
                    )
                } else {
                    "tool_request: [error]".to_string()
                }
            }
            MessageContent::ToolResponse(res) => {
                if let Ok(result) = &res.tool_result {
                    let text_items: Vec<String> = result
                        .content
                        .iter()
                        .filter_map(|content| {
                            content.as_text().map(|text_str| text_str.text.clone())
                        })
                        .collect();

                    if !text_items.is_empty() {
                        let joined = text_items.join("\n");
                        format!(
                            "tool_response: {}",
                            truncate_chars(joined, TOOL_RESPONSE_TEXT_CHAR_LIMIT)
                        )
                    } else {
                        "tool_response: [non-text content]".to_string()
                    }
                } else {
                    "tool_response: [error]".to_string()
                }
            }
            MessageContent::ToolConfirmationRequest(req) => {
                format!("tool_confirmation_request: {}", req.tool_name)
            }
            MessageContent::ActionRequired(action) => match &action.data {
                ActionRequiredData::ToolConfirmation { tool_name, .. } => {
                    format!("action_required(tool_confirmation): {}", tool_name)
                }
                ActionRequiredData::Elicitation { message, .. } => {
                    format!("action_required(elicitation): {}", message)
                }
                ActionRequiredData::ElicitationResponse { id, .. } => {
                    format!("action_required(elicitation_response): {}", id)
                }
            },
            MessageContent::FrontendToolRequest(req) => {
                if let Ok(call) = &req.tool_call {
                    format!("frontend_tool_request: {}", call.name)
                } else {
                    "frontend_tool_request: [error]".to_string()
                }
            }
            MessageContent::Thinking(thinking) => format!("thinking: {}", thinking.thinking),
            MessageContent::RedactedThinking(_) => "redacted_thinking".to_string(),
            MessageContent::SystemNotification(notification) => {
                format!("system_notification: {}", notification.msg)
            }
        })
        .collect();

    let role_str = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };

    if content_parts.is_empty() {
        format!("[{}]: <empty message>", role_str)
    } else {
        format!("[{}]: {}", role_str, content_parts.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::ModelConfig,
        providers::{
            base::{ProviderMetadata, Usage},
            errors::ProviderError,
        },
    };
    use async_trait::async_trait;
    use rmcp::model::{AnnotateAble, CallToolRequestParams, RawContent, Tool};

    struct MockProvider {
        message: Message,
        config: ModelConfig,
        max_tool_responses: Option<usize>,
    }

    impl MockProvider {
        fn new(message: Message, context_limit: usize) -> Self {
            Self {
                message,
                config: ModelConfig {
                    model_name: "test".to_string(),
                    context_limit: Some(context_limit),
                    temperature: None,
                    max_tokens: None,
                    toolshim: false,
                    toolshim_model: None,
                    fast_model: None,
                },
                max_tool_responses: None,
            }
        }

        fn with_max_tool_responses(mut self, max: usize) -> Self {
            self.max_tool_responses = Some(max);
            self
        }
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::new("mock", "", "", "", vec![""], "", vec![])
        }

        fn get_name(&self) -> &str {
            "mock"
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            // If max_tool_responses is set, fail if we have too many
            if let Some(max) = self.max_tool_responses {
                let tool_response_count = messages
                    .iter()
                    .filter(|m| {
                        m.content
                            .iter()
                            .any(|c| matches!(c, MessageContent::ToolResponse(_)))
                    })
                    .count();

                if tool_response_count > max {
                    return Err(ProviderError::ContextLengthExceeded(format!(
                        "Too many tool responses: {} > {}",
                        tool_response_count, max
                    )));
                }
            }

            Ok((
                self.message.clone(),
                ProviderUsage::new("mock-model".to_string(), Usage::default()),
            ))
        }

        fn get_model_config(&self) -> ModelConfig {
            self.config.clone()
        }
    }

    #[tokio::test]
    async fn test_keeps_tool_request() {
        let response_message = Message::assistant().with_text("<mock summary>");
        let provider = MockProvider::new(response_message, 1);
        let basic_conversation = vec![
            Message::user().with_text("read hello.txt"),
            Message::assistant().with_tool_request(
                "tool_0",
                Ok(CallToolRequestParams {
                    name: "read_file".into(),
                    arguments: None,
                    meta: None,
                    task: None,
                }),
            ),
            Message::user().with_tool_response(
                "tool_0",
                Ok(rmcp::model::CallToolResult {
                    content: vec![RawContent::text("hello, world").no_annotation()],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
        ];

        let conversation = Conversation::new_unvalidated(basic_conversation);
        let (compacted_conversation, _usage) = compact_messages(&provider, &conversation, false)
            .await
            .unwrap();

        let agent_conversation = compacted_conversation.agent_visible_messages();

        let _ = Conversation::new(agent_conversation)
            .expect("compaction should produce a valid conversation");
    }

    #[tokio::test]
    async fn test_progressive_removal_on_context_exceeded() {
        let response_message = Message::assistant().with_text("<mock summary>");
        // Set max to 2 tool responses - will trigger progressive removal
        let provider = MockProvider::new(response_message, 1000).with_max_tool_responses(2);

        // Create a conversation with many tool responses
        let mut messages = vec![Message::user().with_text("start")];
        for i in 0..10 {
            messages.push(Message::assistant().with_tool_request(
                format!("tool_{}", i),
                Ok(CallToolRequestParams {
                    name: "read_file".into(),
                    arguments: None,
                    meta: None,
                    task: None,
                }),
            ));
            messages.push(Message::user().with_tool_response(
                format!("tool_{}", i),
                Ok(rmcp::model::CallToolResult {
                    content: vec![RawContent::text(format!("response{}", i)).no_annotation()],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ));
        }

        let conversation = Conversation::new_unvalidated(messages);
        let result = compact_messages(&provider, &conversation, false).await;

        // Should succeed after progressive removal
        assert!(
            result.is_ok(),
            "Should succeed with progressive removal: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_context_strategy_parsing() {
        assert_eq!(
            ContextCompactionStrategy::from_config_value("legacy"),
            ContextCompactionStrategy::LegacySegmented
        );
        assert_eq!(
            ContextCompactionStrategy::from_config_value("legacy_segmented"),
            ContextCompactionStrategy::LegacySegmented
        );
        assert_eq!(
            ContextCompactionStrategy::from_config_value("cfpm"),
            ContextCompactionStrategy::CfpmMemoryV1
        );
        assert_eq!(
            ContextCompactionStrategy::from_config_value("progressive_memory"),
            ContextCompactionStrategy::CfpmMemoryV1
        );
        assert_eq!(
            ContextCompactionStrategy::from_config_value("cfpm_memory_v2"),
            ContextCompactionStrategy::CfpmMemoryV2
        );
        assert_eq!(
            ContextCompactionStrategy::from_config_value("v2"),
            ContextCompactionStrategy::CfpmMemoryV2
        );
    }
}
