//! Pure helpers for the team-server `ChatExecutor`.
//!
//! Like [`crate::agent_task_payload`], this is the parsing / formatting layer
//! that is safe to share with the desktop runtime today. The full executor
//! orchestration (`ServerHarnessHost`, workspace bind, automation builder
//! draft sync) currently still lives on the team-server side and will land
//! when the harness host itself is migrated.

use serde_json::Value;

/// Default user-visible body when the agent's only deliverable was workspace
/// files and there is no other text content.
pub const CHAT_DELIVERY_FALLBACK_TEXT: &str = "已准备文件，可直接预览或下载。";

/// Returns `true` for assistant text blocks that are tool-bridge handoff
/// boilerplate (e.g. "document exported to workspace successfully") rather
/// than user-visible reply text. Used by the chat executor to suppress these
/// blocks when emitting the final assistant message.
pub fn is_delivery_tool_handoff_text(text: &str) -> bool {
    let normalized = text.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return true;
    }

    normalized.contains("document exported to workspace successfully")
        || normalized.contains("file exported to workspace successfully")
        || normalized.contains("workspace successfully")
            && normalized.contains("developer shell")
            && normalized.contains("inspect the file content")
        || normalized.contains("use developer shell, mcp, or another local tool")
}

/// Extract a short preview from the most recent assistant message in a
/// JSON-encoded message history. Returns at most 200 characters; empty when
/// the JSON is malformed or no assistant text is present.
pub fn extract_last_assistant_preview(messages_json: &str) -> String {
    let msgs: Vec<Value> = match serde_json::from_str(messages_json) {
        Ok(m) => m,
        Err(_) => return String::new(),
    };

    let msg = msgs
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"));

    let Some(content) = msg.and_then(|m| m.get("content")) else {
        return String::new();
    };

    if let Some(text) = content.as_str() {
        return text.chars().take(200).collect();
    }

    content
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                .find(|text| !text.is_empty())
        })
        .map(|text| text.chars().take(200).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn delivery_handoff_recognizes_tool_boilerplate() {
        assert!(is_delivery_tool_handoff_text(""));
        assert!(is_delivery_tool_handoff_text(
            "Document exported to workspace successfully"
        ));
        assert!(is_delivery_tool_handoff_text(
            "File exported to workspace successfully."
        ));
        assert!(is_delivery_tool_handoff_text(
            "Use developer shell, MCP, or another local tool to inspect the file content."
        ));
        assert!(!is_delivery_tool_handoff_text("Here is your summary: ..."));
    }

    #[test]
    fn preview_returns_empty_for_invalid_json() {
        assert_eq!(extract_last_assistant_preview("not json"), "");
    }

    #[test]
    fn preview_returns_empty_when_no_assistant_message() {
        let json = serde_json::to_string(&json!([
            { "role": "user", "content": "hello" }
        ]))
        .unwrap();
        assert_eq!(extract_last_assistant_preview(&json), "");
    }

    #[test]
    fn preview_extracts_string_assistant_content() {
        let json = serde_json::to_string(&json!([
            { "role": "user", "content": "hello" },
            { "role": "assistant", "content": "hi there" }
        ]))
        .unwrap();
        assert_eq!(extract_last_assistant_preview(&json), "hi there");
    }

    #[test]
    fn preview_picks_first_nonempty_text_block_from_array_content() {
        let json = serde_json::to_string(&json!([
            { "role": "assistant", "content": [
                { "type": "text", "text": "" },
                { "type": "text", "text": "actual reply" },
                { "type": "text", "text": "ignored later" }
            ]}
        ]))
        .unwrap();
        assert_eq!(extract_last_assistant_preview(&json), "actual reply");
    }

    #[test]
    fn preview_picks_last_assistant_when_multiple() {
        let json = serde_json::to_string(&json!([
            { "role": "assistant", "content": "first reply" },
            { "role": "user", "content": "follow up" },
            { "role": "assistant", "content": "second reply" }
        ]))
        .unwrap();
        assert_eq!(extract_last_assistant_preview(&json), "second reply");
    }

    #[test]
    fn preview_truncates_long_text_to_200_chars() {
        let long = "x".repeat(500);
        let json = serde_json::to_string(&json!([
            { "role": "assistant", "content": long }
        ]))
        .unwrap();
        assert_eq!(extract_last_assistant_preview(&json).chars().count(), 200);
    }
}
