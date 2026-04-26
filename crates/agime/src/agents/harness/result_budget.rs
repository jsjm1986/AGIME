use chrono::Utc;
use rmcp::model::{CallToolResult, Content, ErrorData};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultBudgetBucket {
    Small,
    Standard,
    Large,
    Unlimited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultBudgetAction {
    Inline,
    Truncate,
    Summarize,
    ReplaceWithHandle,
}

#[derive(Debug, Clone)]
pub struct ToolResultBudget {
    pub small_limit: usize,
    pub standard_limit: usize,
    pub large_limit: usize,
    pub summary_limit: usize,
    pub replace_threshold: usize,
}

impl Default for ToolResultBudget {
    fn default() -> Self {
        Self {
            small_limit: 2_000,
            standard_limit: 8_000,
            large_limit: 24_000,
            summary_limit: 512,
            replace_threshold: 64_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolResultHandle {
    pub id: String,
    pub request_id: String,
    pub tool_name: String,
    pub bucket: ToolResultBudgetBucket,
    pub original_length: usize,
    pub created_at: i64,
}

#[derive(Debug)]
pub struct BudgetedToolResult {
    pub result: Result<CallToolResult, ErrorData>,
    pub handle: Option<ToolResultHandle>,
    pub action: ResultBudgetAction,
}

fn bucket_limit(
    budget: &ToolResultBudget,
    bucket: ToolResultBudgetBucket,
    explicit_limit: Option<usize>,
) -> Option<usize> {
    if let Some(explicit_limit) = explicit_limit {
        return Some(explicit_limit);
    }

    match bucket {
        ToolResultBudgetBucket::Small => Some(budget.small_limit),
        ToolResultBudgetBucket::Standard => Some(budget.standard_limit),
        ToolResultBudgetBucket::Large => Some(budget.large_limit),
        ToolResultBudgetBucket::Unlimited => None,
    }
}

fn summarize_text(input: &str, max_chars: usize) -> String {
    let snippet = input.chars().take(max_chars).collect::<String>();
    if snippet.len() == input.len() {
        snippet
    } else {
        format!("{}...", snippet)
    }
}

fn replacement_message(
    action: ResultBudgetAction,
    tool_name: &str,
    handle: &ToolResultHandle,
    text: &str,
    budget: &ToolResultBudget,
    visible_limit: usize,
) -> String {
    match action {
        ResultBudgetAction::Truncate => {
            let visible = summarize_text(text, visible_limit.min(handle.original_length));
            format!(
                "Tool result for `{}` was truncated due to budget limits. Preview:\n{}",
                tool_name, visible
            )
        }
        ResultBudgetAction::Summarize => {
            let summary = summarize_text(text, budget.summary_limit);
            format!(
                "Tool result for `{}` was summarized due to size. Summary:\n{}",
                tool_name, summary
            )
        }
        ResultBudgetAction::ReplaceWithHandle => {
            format!(
                "Tool result for `{}` was replaced with stored handle {} because it exceeded the result budget.",
                tool_name, handle.id
            )
        }
        ResultBudgetAction::Inline => text.to_string(),
    }
}

pub async fn apply_tool_result_budget(
    request_id: &str,
    tool_name: &str,
    bucket: ToolResultBudgetBucket,
    explicit_limit: Option<usize>,
    result: Result<CallToolResult, ErrorData>,
    budget: &ToolResultBudget,
) -> BudgetedToolResult {
    let Some(limit) = bucket_limit(budget, bucket, explicit_limit) else {
        return BudgetedToolResult {
            result,
            handle: None,
            action: ResultBudgetAction::Inline,
        };
    };

    match result {
        Ok(mut tool_result) => {
            let mut action = ResultBudgetAction::Inline;
            let mut first_handle = None;
            let mut processed_content = Vec::with_capacity(tool_result.content.len());

            for content in tool_result.content {
                if let Some(text) = content.as_text() {
                    let char_count = text.text.chars().count();
                    if char_count > limit {
                        let handle = ToolResultHandle {
                            id: format!("toolres_{}", Uuid::new_v4().simple()),
                            request_id: request_id.to_string(),
                            tool_name: tool_name.to_string(),
                            bucket,
                            original_length: char_count,
                            created_at: Utc::now().timestamp(),
                        };
                        let next_action = if char_count > budget.replace_threshold {
                            // We do not yet have a production readback path for
                            // ToolResultHandle, so an unreadable handle-only fallback
                            // would discard information. Prefer a bounded summary until
                            // resume/readback semantics actually exist.
                            ResultBudgetAction::Summarize
                        } else if char_count > limit.saturating_mul(4) {
                            ResultBudgetAction::Summarize
                        } else {
                            ResultBudgetAction::Truncate
                        };
                        processed_content.push(Content::text(replacement_message(
                            next_action,
                            tool_name,
                            &handle,
                            &text.text,
                            budget,
                            limit,
                        )));
                        if first_handle.is_none() {
                            first_handle = Some(handle);
                        }
                        action = next_action;
                    } else {
                        processed_content.push(content);
                    }
                } else {
                    processed_content.push(content);
                }
            }

            tool_result.content = processed_content;
            BudgetedToolResult {
                result: Ok(tool_result),
                handle: first_handle,
                action,
            }
        }
        Err(error) => BudgetedToolResult {
            result: Err(error),
            handle: None,
            action: ResultBudgetAction::Inline,
        },
    }
}
