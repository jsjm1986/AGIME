//! Pure parsing layer for `AgentTask` payloads.
//!
//! Extracted from `agime-team-server::agent::agent_task_v4_runner` so the
//! desktop runtime can reuse the same payload shape without depending on
//! Mongo, the team-server `AgentService`, or `ServerHarnessHost`.
//!
//! The orchestration around these payloads (session ensure, workspace bind,
//! settle success/failure) currently still lives on the team-server side and
//! will land in [`crate::execution_admission`]'s spawner adapter or in the
//! eventual harness-host extraction (batch 9).

use agime::agents::types::RetryConfig;
use agime::conversation::message::Message;
use anyhow::{anyhow, Result};
use serde_json::Value;

/// Normalized AgentTask payload extracted from the Mongo `task.content` blob.
#[derive(Debug, Clone)]
pub struct ParsedTaskPayload {
    pub prior_messages: Vec<Message>,
    pub user_message: String,
    pub turn_system_instruction: Option<String>,
    pub workspace_path: Option<String>,
    pub target_artifacts: Vec<String>,
    pub result_contract: Vec<String>,
    pub allowed_extensions: Option<Vec<String>>,
    pub allowed_skill_ids: Option<Vec<String>>,
    pub attached_document_ids: Vec<String>,
    pub llm_overrides: Option<Value>,
    pub retry_config: Option<RetryConfig>,
    pub max_turns: Option<i32>,
    pub validation_mode: bool,
}

/// Resolve the DirectHarness timeout (seconds) from the `TEAM_DIRECT_HOST_TIMEOUT_SECS`
/// environment variable, falling back to 600s when unset/invalid.
pub fn direct_host_timeout_secs() -> u64 {
    std::env::var("TEAM_DIRECT_HOST_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(600)
}

/// Whether `status` should be treated as a success (terminal, non-failure)
/// for AgentTask settlement.
pub fn is_success_status(status: &str) -> bool {
    status.eq_ignore_ascii_case("completed")
        || status.eq_ignore_ascii_case("completed_with_warnings")
}

/// Parse a Mongo `task.content` JSON value into a normalized payload.
pub fn parse_task_payload(content: &Value) -> Result<ParsedTaskPayload> {
    let raw_messages = content
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Invalid AgentTask content: missing messages"))?;
    let (prior_messages, user_message) = parse_messages(raw_messages)?;

    Ok(ParsedTaskPayload {
        prior_messages,
        user_message,
        turn_system_instruction: string_field(
            content,
            &["turn_system_instruction", "turnSystemInstruction"],
        ),
        workspace_path: string_field(content, &["workspace_path", "workspacePath"]),
        target_artifacts: string_array_field(content, &["target_artifacts", "targetArtifacts"]),
        result_contract: string_array_field(content, &["result_contract", "resultContract"]),
        allowed_extensions: optional_string_array_field(
            content,
            &["allowed_extensions", "allowedExtensions"],
        ),
        allowed_skill_ids: optional_string_array_field(
            content,
            &["allowed_skill_ids", "allowedSkillIds"],
        ),
        attached_document_ids: string_array_field(
            content,
            &[
                "attached_document_ids",
                "attachedDocumentIds",
                "document_ids",
                "documentIds",
            ],
        ),
        llm_overrides: content
            .get("llm_overrides")
            .or_else(|| content.get("llmOverrides"))
            .cloned(),
        retry_config: content
            .get("retry_config")
            .or_else(|| content.get("retryConfig"))
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        max_turns: content
            .get("max_turns")
            .or_else(|| content.get("maxTurns"))
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .filter(|value| *value > 0),
        validation_mode: content
            .get("validation_mode")
            .or_else(|| content.get("validationMode"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn parse_messages(raw_messages: &[Value]) -> Result<(Vec<Message>, String)> {
    let parsed = raw_messages
        .iter()
        .filter_map(|value| {
            let role = value
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .trim()
                .to_ascii_lowercase();
            let text = extract_message_text(value.get("content")?)?;
            (!text.trim().is_empty()).then_some((role, text))
        })
        .collect::<Vec<_>>();

    let Some(last_user_index) = parsed
        .iter()
        .rposition(|(role, _)| role.eq_ignore_ascii_case("user"))
        .or_else(|| parsed.len().checked_sub(1))
    else {
        return Err(anyhow!(
            "Invalid AgentTask content: messages contain no text"
        ));
    };

    let user_message = parsed[last_user_index].1.clone();
    let prior_messages = parsed[..last_user_index]
        .iter()
        .filter_map(|(role, text)| match role.as_str() {
            "user" => Some(Message::user().with_text(text.clone())),
            "assistant" => Some(Message::assistant().with_text(text.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    Ok((prior_messages, user_message))
}

fn extract_message_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let text = items
                .iter()
                .filter_map(|item| {
                    item.get("text")
                        .and_then(Value::as_str)
                        .or_else(|| item.get("content").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
                .join("\n");
            (!text.trim().is_empty()).then_some(text)
        }
        Value::Object(map) => map.get("text").and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

fn string_field(content: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        content
            .get(*name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn optional_string_array_field(content: &Value, names: &[&str]) -> Option<Vec<String>> {
    let values = string_array_field(content, names);
    (!values.is_empty()).then_some(values)
}

fn string_array_field(content: &Value, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .find_map(|name| content.get(*name).and_then(Value::as_array))
        .map(|items| {
            let mut values = items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            values.sort();
            values.dedup();
            values
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_minimal_payload_with_user_message() {
        let content = json!({
            "messages": [
                { "role": "user", "content": "hello world" }
            ]
        });
        let payload = parse_task_payload(&content).unwrap();
        assert_eq!(payload.user_message, "hello world");
        assert!(payload.prior_messages.is_empty());
        assert!(payload.target_artifacts.is_empty());
        assert!(!payload.validation_mode);
    }

    #[test]
    fn picks_last_user_message_and_keeps_prior_history() {
        let content = json!({
            "messages": [
                { "role": "user", "content": "first" },
                { "role": "assistant", "content": "ack" },
                { "role": "user", "content": "second" }
            ]
        });
        let payload = parse_task_payload(&content).unwrap();
        assert_eq!(payload.user_message, "second");
        assert_eq!(payload.prior_messages.len(), 2);
    }

    #[test]
    fn extracts_camel_and_snake_case_fields() {
        let content = json!({
            "messages": [{ "role": "user", "content": "go" }],
            "turnSystemInstruction": "be concise",
            "target_artifacts": ["a", "b", "a"],
            "allowedExtensions": ["x"],
            "validation_mode": true,
            "max_turns": 7
        });
        let payload = parse_task_payload(&content).unwrap();
        assert_eq!(
            payload.turn_system_instruction.as_deref(),
            Some("be concise")
        );
        assert_eq!(
            payload.target_artifacts,
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(
            payload.allowed_extensions.as_deref(),
            Some(&["x".to_string()][..])
        );
        assert!(payload.validation_mode);
        assert_eq!(payload.max_turns, Some(7));
    }

    #[test]
    fn rejects_payload_without_messages_array() {
        let content = json!({});
        assert!(parse_task_payload(&content).is_err());
    }

    #[test]
    fn rejects_empty_messages() {
        let content = json!({ "messages": [] });
        assert!(parse_task_payload(&content).is_err());
    }

    #[test]
    fn extracts_text_from_array_content_blocks() {
        let content = json!({
            "messages": [
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "alpha" },
                        { "type": "text", "text": "beta" }
                    ]
                }
            ]
        });
        let payload = parse_task_payload(&content).unwrap();
        assert_eq!(payload.user_message, "alpha\nbeta");
    }

    #[test]
    fn is_success_status_recognizes_completed_variants() {
        assert!(is_success_status("completed"));
        assert!(is_success_status("Completed_With_Warnings"));
        assert!(!is_success_status("failed"));
        assert!(!is_success_status("cancelled"));
    }
}
