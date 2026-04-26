use anyhow::{anyhow, Result};
use chrono::Utc;
use rmcp::model::Role;
use std::collections::HashSet;

use crate::conversation::message::{Message, MessageContent};
use crate::conversation::Conversation;
use crate::prompt_template::render_global_file;
use crate::providers::base::Provider;
use crate::providers::errors::ProviderError;

use super::store::current_visible_working_window;
use super::types::{
    ContextRuntimeState, SessionMemoryPolicy, SessionMemoryPolicyKind, SessionMemoryState,
    TOOL_REQUEST_ARG_CHAR_LIMIT, TOOL_RESPONSE_TEXT_CHAR_LIMIT,
};

pub(crate) async fn build_session_memory_state(
    provider: &dyn Provider,
    conversation: &Conversation,
    state: &ContextRuntimeState,
    policy_kind: SessionMemoryPolicyKind,
    existing: Option<&SessionMemoryState>,
) -> Result<SessionMemoryState> {
    let messages = conversation.messages();
    if messages.is_empty() {
        return Ok(SessionMemoryState::default());
    }

    let (window_start, window_end_exclusive) =
        current_visible_working_window(messages.len(), state);
    if window_end_exclusive <= window_start || window_end_exclusive > messages.len() {
        return Ok(SessionMemoryState::default());
    }
    let working_messages = &messages[window_start..window_end_exclusive];

    let last_summarized_index =
        existing.and_then(|memory| resolve_last_summarized_index(working_messages, memory));
    let start = calculate_suffix_start(
        working_messages,
        SessionMemoryPolicy::for_kind(policy_kind),
        last_summarized_index,
    );
    if start == 0 {
        return Err(anyhow!(
            "not enough prefix messages to build session memory"
        ));
    }

    let summary_input = build_session_memory_summary_input(working_messages, start, existing);
    let summary = summarize_messages(provider, &summary_input).await?;
    let preserved_message_count = working_messages.len().saturating_sub(start);
    let preserved_token_estimate = working_messages[start..]
        .iter()
        .map(estimate_message_tokens)
        .sum::<usize>();
    Ok(SessionMemoryState {
        summary,
        summarized_through_message_id: working_messages
            .get(start.saturating_sub(1))
            .and_then(|message| message.id.clone()),
        preserved_start_index: window_start + start,
        preserved_end_index: window_start + working_messages.len().saturating_sub(1),
        preserved_start_message_id: working_messages
            .get(start)
            .and_then(|message| message.id.clone()),
        preserved_end_message_id: working_messages
            .last()
            .and_then(|message| message.id.clone()),
        preserved_message_count,
        preserved_token_estimate,
        tail_anchor_index: window_start + working_messages.len().saturating_sub(1),
        tail_anchor_message_id: working_messages
            .last()
            .and_then(|message| message.id.clone()),
        updated_at: Utc::now().timestamp(),
    })
}

fn build_session_memory_summary_input(
    working_messages: &[Message],
    start: usize,
    existing: Option<&SessionMemoryState>,
) -> Vec<Message> {
    let mut input = Vec::new();
    if let Some(existing) = existing {
        if !existing.summary.trim().is_empty() {
            input.push(
                Message::user()
                    .with_text(format!("[PREVIOUS_SESSION_MEMORY]\n{}", existing.summary)),
            );
        }
    }
    input.extend_from_slice(&working_messages[..start]);
    input
}

pub(crate) fn calculate_suffix_start(
    messages: &[Message],
    policy: SessionMemoryPolicy,
    last_summarized_index: Option<usize>,
) -> usize {
    if messages.is_empty() {
        return 0;
    }

    let mut start = messages.len();
    let mut total_tokens = 0usize;
    let mut text_message_count = 0usize;

    for idx in (0..messages.len()).rev() {
        let msg = &messages[idx];
        total_tokens = total_tokens.saturating_add(estimate_message_tokens(msg));
        if has_textual_content(msg) {
            text_message_count = text_message_count.saturating_add(1);
        }
        start = idx;
        if total_tokens >= policy.max_tokens {
            break;
        }
        if total_tokens >= policy.min_tokens && text_message_count >= policy.min_text_messages {
            break;
        }
    }

    let adjusted = adjust_suffix_start(messages, start);
    if let Some(last) = last_summarized_index {
        adjusted.max(last.saturating_add(1))
    } else {
        adjusted
    }
}

pub(crate) fn adjust_suffix_start(messages: &[Message], mut start: usize) -> usize {
    if start == 0 || start >= messages.len() {
        return start.min(messages.len());
    }

    let mut tool_response_ids = HashSet::new();
    let mut kept_assistant_ids = HashSet::new();
    for msg in &messages[start..] {
        for tool_id in msg.get_tool_response_ids() {
            tool_response_ids.insert(tool_id.to_string());
        }
        if msg.role == Role::Assistant {
            if let Some(message_id) = msg.id.as_deref() {
                kept_assistant_ids.insert(message_id.to_string());
            }
        }
    }

    for idx in (0..start).rev() {
        let msg = &messages[idx];
        if msg
            .get_tool_request_ids()
            .iter()
            .any(|request_id| tool_response_ids.contains(*request_id))
        {
            start = idx;
        }
        if msg.role == Role::Assistant {
            if let Some(message_id) = msg.id.as_deref() {
                if kept_assistant_ids.contains(message_id) {
                    start = idx;
                }
            }
        }
    }

    start
}

fn resolve_last_summarized_index(
    messages: &[Message],
    existing: &SessionMemoryState,
) -> Option<usize> {
    existing
        .summarized_through_message_id
        .as_deref()
        .and_then(|message_id| {
            messages
                .iter()
                .position(|message| message.id.as_deref() == Some(message_id))
        })
}

pub(crate) fn has_textual_content(message: &Message) -> bool {
    message.content.iter().any(|content| match content {
        MessageContent::Text(text) => !text.text.trim().is_empty(),
        MessageContent::SystemNotification(notification) => !notification.msg.trim().is_empty(),
        _ => false,
    })
}

pub(crate) fn estimate_message_tokens(message: &Message) -> usize {
    let text = message
        .content
        .iter()
        .map(|content| match content {
            MessageContent::Text(text) => text.text.clone(),
            MessageContent::SystemNotification(notification) => notification.msg.clone(),
            MessageContent::ToolRequest(request) => format!("tool-request:{}", request.id),
            MessageContent::ToolResponse(response) => format!("tool-response:{}", response.id),
            MessageContent::Image(image) => format!("image:{}", image.mime_type),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\n");

    (text.chars().count() / 4).max(1)
}

pub(crate) async fn summarize_messages(
    provider: &dyn Provider,
    messages: &[Message],
) -> Result<String> {
    let agent_visible_messages: Vec<&Message> = messages
        .iter()
        .filter(|message| message.is_agent_visible())
        .collect();
    if agent_visible_messages.is_empty() {
        return Ok("No agent-visible content to summarize.".to_string());
    }

    let removal_percentages = [0, 10, 20, 50, 100];
    for (attempt, remove_percent) in removal_percentages.iter().enumerate() {
        let filtered_messages = filter_tool_responses(&agent_visible_messages, *remove_percent);
        let messages_text = filtered_messages
            .iter()
            .map(|message| format_message_for_compacting(message))
            .collect::<Vec<_>>()
            .join("\n");
        let system_prompt = render_global_file(
            "summarize_oneshot.md",
            &serde_json::json!({ "messages": messages_text }),
        )?;
        let request = vec![Message::user()
            .with_text("Please summarize the conversation history provided in the system prompt.")];

        match provider.complete_fast(&system_prompt, &request, &[]).await {
            Ok((response, _usage)) => return Ok(response.as_concat_text()),
            Err(err) => {
                if matches!(err, ProviderError::ContextLengthExceeded(_))
                    && attempt < removal_percentages.len().saturating_sub(1)
                {
                    continue;
                }
                return Err(err.into());
            }
        }
    }

    Err(anyhow!("exhausted summarization attempts"))
}

pub(crate) fn filter_tool_responses<'a>(
    messages: &[&'a Message],
    remove_percent: u32,
) -> Vec<&'a Message> {
    if remove_percent == 0 {
        return messages.to_vec();
    }

    let tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| {
            message
                .content
                .iter()
                .any(|content| matches!(content, MessageContent::ToolResponse(_)))
        })
        .map(|(idx, _)| idx)
        .collect();

    if tool_indices.is_empty() {
        return messages.to_vec();
    }

    let num_to_remove = ((tool_indices.len() * remove_percent as usize) / 100).max(1);
    let middle = tool_indices.len() / 2;
    let mut indices_to_remove = Vec::new();

    for offset in 0..num_to_remove {
        if offset % 2 == 0 {
            let delta = offset / 2;
            if middle > delta {
                indices_to_remove.push(tool_indices[middle - delta - 1]);
            }
        } else {
            let delta = offset / 2;
            if middle + delta < tool_indices.len() {
                indices_to_remove.push(tool_indices[middle + delta]);
            }
        }
    }

    messages
        .iter()
        .enumerate()
        .filter(|(idx, _)| !indices_to_remove.contains(idx))
        .map(|(_, message)| *message)
        .collect()
}

pub(crate) fn format_message_for_compacting(message: &Message) -> String {
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

    let content_parts = message
        .content
        .iter()
        .map(|content| match content {
            MessageContent::Text(text) => text.text.clone(),
            MessageContent::Image(image) => format!("[image: {}]", image.mime_type),
            MessageContent::ToolRequest(request) => {
                if let Ok(call) = &request.tool_call {
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
            MessageContent::ToolResponse(response) => {
                if let Ok(result) = &response.tool_result {
                    let text_items = result
                        .content
                        .iter()
                        .filter_map(|content| content.as_text().map(|text| text.text.clone()))
                        .collect::<Vec<_>>();
                    if !text_items.is_empty() {
                        format!(
                            "tool_response: {}",
                            truncate_chars(text_items.join("\n"), TOOL_RESPONSE_TEXT_CHAR_LIMIT)
                        )
                    } else {
                        "tool_response: [non-text content]".to_string()
                    }
                } else {
                    "tool_response: [error]".to_string()
                }
            }
            MessageContent::SystemNotification(notification) => {
                format!(
                    "system_notification({:?}): {}",
                    notification.notification_type, notification.msg
                )
            }
            MessageContent::ToolConfirmationRequest(_)
            | MessageContent::ActionRequired(_)
            | MessageContent::FrontendToolRequest(_)
            | MessageContent::Thinking(_)
            | MessageContent::RedactedThinking(_) => String::new(),
        })
        .collect::<Vec<_>>();

    format!("{:?}: {}", message.role, content_parts.join(" | "))
}
