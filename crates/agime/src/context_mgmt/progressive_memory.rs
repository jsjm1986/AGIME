use crate::conversation::message::{Message, MessageContent, MessageMetadata, ToolRequest};
use crate::conversation::Conversation;
use crate::providers::base::{ProviderUsage, Usage};
use anyhow::Result;
use regex::Regex;
use rmcp::model::Role;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use tracing::info;

const KEEP_FIRST_MESSAGES: usize = 2;
const KEEP_LAST_MESSAGES: usize = 8;
const MIN_MESSAGES_TO_COMPACT: usize = 12;
const MAX_MEMORY_ITEMS_PER_SECTION: usize = 8;
const MAX_ITEM_CHARS: usize = 220;

const CONTINUATION_TEXT: &str =
    "The previous message contains a stable memory snapshot for this session.
Do not mention that memory compaction happened.
Continue naturally and prefer these stable facts over repeated re-discovery.";

const MANUAL_CONTINUATION_TEXT: &str =
    "The previous message contains a stable memory snapshot prepared at the user's request.
Do not mention that memory compaction happened.
Continue naturally and use these facts as persistent context.";

#[derive(Default)]
struct MemorySnapshot {
    goals: Vec<String>,
    verified_actions: Vec<String>,
    artifacts: Vec<String>,
    open_items: Vec<String>,
}

pub(super) async fn compact_messages_progressive_memory(
    conversation: &Conversation,
    manual_compact: bool,
) -> Result<(Conversation, ProviderUsage)> {
    let messages = conversation.messages();
    let total = messages.len();

    if total < MIN_MESSAGES_TO_COMPACT && !manual_compact {
        info!(
            "Skipping CFPM compaction: only {} messages (minimum: {})",
            total, MIN_MESSAGES_TO_COMPACT
        );
        return Ok((
            conversation.clone(),
            ProviderUsage::new("cfpm-memory".to_string(), Usage::default()),
        ));
    }

    let keep_first = KEEP_FIRST_MESSAGES.min(total);
    let keep_last = if total > keep_first + KEEP_LAST_MESSAGES {
        KEEP_LAST_MESSAGES
    } else {
        (total - keep_first).max(0)
    };

    let middle_start = keep_first;
    let middle_end = total.saturating_sub(keep_last);

    if middle_end <= middle_start {
        return Ok((
            conversation.clone(),
            ProviderUsage::new("cfpm-memory".to_string(), Usage::default()),
        ));
    }

    let first_messages = messages[..keep_first].to_vec();
    let middle_messages = &messages[middle_start..middle_end];
    let last_messages = messages[(total - keep_last)..].to_vec();

    let snapshot = build_memory_snapshot(middle_messages);
    let memory_text = format_memory_text(snapshot);

    let mut final_messages = Vec::new();
    final_messages.extend(first_messages);

    for msg in middle_messages {
        final_messages.push(
            msg.clone()
                .with_metadata(msg.metadata.with_agent_invisible()),
        );
    }

    final_messages.push(
        Message::user()
            .with_text(memory_text)
            .with_metadata(MessageMetadata::agent_only()),
    );

    final_messages.push(
        Message::assistant()
            .with_text(if manual_compact {
                MANUAL_CONTINUATION_TEXT
            } else {
                CONTINUATION_TEXT
            })
            .with_metadata(MessageMetadata::agent_only()),
    );

    final_messages.extend(last_messages);

    Ok((
        Conversation::new_unvalidated(final_messages),
        ProviderUsage::new("cfpm-memory".to_string(), Usage::default()),
    ))
}

fn build_memory_snapshot(messages: &[Message]) -> MemorySnapshot {
    let mut snapshot = MemorySnapshot::default();

    let mut goals_seen = HashSet::new();
    let mut verified_seen = HashSet::new();
    let mut artifact_seen = HashSet::new();
    let mut open_seen = HashSet::new();

    let request_map = build_tool_request_map(messages);

    for msg in messages {
        for text in extract_text_blocks(msg) {
            if is_unstable_error_noise(&text) {
                continue;
            }

            match msg.role {
                Role::User => {
                    collect_goal_candidates(&text, &mut snapshot.goals, &mut goals_seen);
                    collect_open_item_candidates(&text, &mut snapshot.open_items, &mut open_seen);
                }
                Role::Assistant => {
                    collect_decision_candidates(
                        &text,
                        &mut snapshot.verified_actions,
                        &mut verified_seen,
                    );
                    collect_open_item_candidates(&text, &mut snapshot.open_items, &mut open_seen);
                }
            }
        }

        for content in &msg.content {
            if let MessageContent::ToolResponse(res) = content {
                let Ok(result) = &res.tool_result else {
                    continue;
                };
                if result.is_error.unwrap_or(false) {
                    continue;
                }

                let text_output = result
                    .content
                    .iter()
                    .filter_map(|item| item.as_text().map(|text| text.text.clone()))
                    .collect::<Vec<_>>()
                    .join("\n");

                if text_output.is_empty() || is_unstable_error_noise(&text_output) {
                    continue;
                }

                if let Some(request_desc) = request_map.get(&res.id) {
                    push_unique_limited(
                        &mut snapshot.verified_actions,
                        &mut verified_seen,
                        request_desc.clone(),
                    );
                }

                if looks_like_success_output(&text_output) {
                    let first_line = text_output
                        .lines()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or("")
                        .trim();
                    if !first_line.is_empty() {
                        push_unique_limited(
                            &mut snapshot.verified_actions,
                            &mut verified_seen,
                            first_line.to_string(),
                        );
                    }
                }

                for path in extract_paths(&text_output) {
                    push_unique_limited(&mut snapshot.artifacts, &mut artifact_seen, path);
                }
            }
        }
    }

    if snapshot.goals.is_empty() {
        snapshot
            .goals
            .push("No stable user goal extracted yet.".to_string());
    }
    if snapshot.verified_actions.is_empty() {
        snapshot
            .verified_actions
            .push("No verified successful action extracted yet.".to_string());
    }

    snapshot
}

fn build_tool_request_map(messages: &[Message]) -> HashMap<String, String> {
    let mut map = HashMap::new();

    for msg in messages {
        for content in &msg.content {
            if let MessageContent::ToolRequest(req) = content {
                if let Some(description) = describe_tool_request(req) {
                    map.insert(req.id.clone(), description);
                }
            }
        }
    }

    map
}

fn describe_tool_request(req: &ToolRequest) -> Option<String> {
    let call = req.tool_call.as_ref().ok()?;
    let args = call.arguments.as_ref();
    let tool_name = call.name.as_ref();

    let command = args
        .and_then(|value| value.get("command"))
        .and_then(|value| value.as_str());
    let path = args
        .and_then(|value| value.get("path"))
        .and_then(|value| value.as_str());

    let description = match (tool_name, command, path) {
        ("shell", Some(cmd), _)
        | ("shell_command", Some(cmd), _)
        | ("developer__shell", Some(cmd), _) => {
            format!("Executed command successfully: {}", cmd.trim())
        }
        ("text_editor", Some(cmd), Some(file_path)) => {
            format!(
                "Edited file successfully: {} {}",
                cmd.trim(),
                file_path.trim()
            )
        }
        ("text_editor", Some(cmd), None) => format!("Text editor action succeeded: {}", cmd.trim()),
        (_, _, Some(file_path)) => format!("Tool {} affected {}", tool_name, file_path.trim()),
        _ => format!("Tool {} completed successfully", tool_name),
    };

    Some(description)
}

fn extract_text_blocks(msg: &Message) -> Vec<String> {
    msg.content
        .iter()
        .filter_map(|content| match content {
            MessageContent::Text(text) => Some(text.text.clone()),
            MessageContent::SystemNotification(notification) => Some(notification.msg.clone()),
            _ => None,
        })
        .collect()
}

fn collect_goal_candidates(text: &str, output: &mut Vec<String>, seen: &mut HashSet<String>) {
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || !is_goal_line(line) || is_unstable_error_noise(line) {
            continue;
        }
        push_unique_limited(output, seen, line.to_string());
    }
}

fn collect_decision_candidates(text: &str, output: &mut Vec<String>, seen: &mut HashSet<String>) {
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || !is_decision_line(line) || is_unstable_error_noise(line) {
            continue;
        }
        push_unique_limited(output, seen, line.to_string());
    }
}

fn collect_open_item_candidates(text: &str, output: &mut Vec<String>, seen: &mut HashSet<String>) {
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || !is_open_item_line(line) || is_unstable_error_noise(line) {
            continue;
        }
        push_unique_limited(output, seen, line.to_string());
    }
}

fn is_goal_line(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    let keywords = [
        "must", "should", "need to", "require", "keep", "不要", "必须", "需要", "目标", "要求",
    ];
    keywords.iter().any(|keyword| lowered.contains(keyword))
}

fn is_decision_line(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    let keywords = [
        "plan",
        "next",
        "decide",
        "will",
        "use",
        "采用",
        "方案",
        "下一步",
        "执行",
        "已完成",
    ];
    keywords.iter().any(|keyword| lowered.contains(keyword))
}

fn is_open_item_line(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    let keywords = [
        "todo",
        "remaining",
        "pending",
        "next",
        "待办",
        "后续",
        "继续",
        "下一步",
    ];
    keywords.iter().any(|keyword| lowered.contains(keyword))
}

fn looks_like_success_output(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    let markers = [
        "exit code: 0",
        "success",
        "succeeded",
        "completed",
        "done",
        "saved",
        "written",
        "created",
        "updated",
        "renamed",
        "moved",
        "成功",
        "完成",
        "已保存",
    ];
    markers.iter().any(|marker| lowered.contains(marker))
}

fn is_unstable_error_noise(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    let error_markers = [
        "error",
        "failed",
        "exception",
        "traceback",
        "contextlengthexceeded",
        "not found",
        "enoent",
        "permission denied",
        "exit code: 1",
        "exit code: 2",
        "报错",
        "失败",
        "未找到",
        "错误",
    ];
    let resolved_markers = [
        "resolved",
        "fixed",
        "workaround",
        "success",
        "已解决",
        "修复",
        "成功",
    ];

    let has_error = error_markers.iter().any(|marker| lowered.contains(marker));
    let has_resolved = resolved_markers
        .iter()
        .any(|marker| lowered.contains(marker));

    has_error && !has_resolved
}

fn looks_like_date_token(value: &str) -> bool {
    let token = value
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | ',' | ';' | '.' | ')' | '(' | '[' | ']' | '{' | '}' | '<' | '>'
            )
        })
        .trim();
    if token.is_empty() {
        return false;
    }

    for separator in ['/', '-', '.'] {
        if !token.contains(separator) {
            continue;
        }

        let parts = token.split(separator).collect::<Vec<_>>();
        if parts.len() != 3 {
            continue;
        }
        if parts
            .iter()
            .any(|part| part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()))
        {
            continue;
        }

        let Ok(first) = parts[0].parse::<u32>() else {
            continue;
        };
        let Ok(second) = parts[1].parse::<u32>() else {
            continue;
        };
        let Ok(third) = parts[2].parse::<u32>() else {
            continue;
        };

        if parts[0].len() == 4
            && (1900..=2200).contains(&first)
            && (1..=12).contains(&second)
            && (1..=31).contains(&third)
        {
            return true;
        }

        if parts[2].len() == 4
            && (1900..=2200).contains(&third)
            && (1..=12).contains(&second)
            && (1..=31).contains(&first)
        {
            return true;
        }
    }

    false
}

fn extract_paths(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for regex in path_regexes() {
        for captures in regex.find_iter(text) {
            let candidate = captures.as_str().trim();
            if candidate.starts_with("http://") || candidate.starts_with("https://") {
                continue;
            }
            if candidate.len() < 3 {
                continue;
            }
            if looks_like_date_token(candidate) {
                continue;
            }
            paths.push(candidate.to_string());
        }
    }
    paths
}

fn path_regexes() -> &'static [Regex; 2] {
    static REGEXES: OnceLock<[Regex; 2]> = OnceLock::new();
    REGEXES.get_or_init(|| {
        [
            Regex::new(r#"[A-Za-z]:\\[^\s"'<>|]+"#).expect("valid windows path regex"),
            Regex::new(
                r"(?:\./|\.\./|/)?(?:[A-Za-z0-9._-]+/)+[A-Za-z0-9._-]+(?:\.[A-Za-z0-9._-]+)?",
            )
            .expect("valid unix path regex"),
        ]
    })
}

fn push_unique_limited(target: &mut Vec<String>, seen: &mut HashSet<String>, value: String) {
    if target.len() >= MAX_MEMORY_ITEMS_PER_SECTION {
        return;
    }

    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() || seen.contains(&normalized) {
        return;
    }

    let sanitized = truncate_chars(value.trim(), MAX_ITEM_CHARS);
    if sanitized.is_empty() {
        return;
    }

    seen.insert(normalized);
    target.push(sanitized);
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

fn format_memory_text(snapshot: MemorySnapshot) -> String {
    let mut lines = vec![
        "[CFPM_MEMORY_V1] Stable session memory (errors and transient retries removed)."
            .to_string(),
        String::new(),
        "User goals:".to_string(),
    ];

    for item in snapshot.goals {
        lines.push(format!("- {}", item));
    }

    lines.push(String::new());
    lines.push("Verified actions:".to_string());
    for item in snapshot.verified_actions {
        lines.push(format!("- {}", item));
    }

    if !snapshot.artifacts.is_empty() {
        lines.push(String::new());
        lines.push("Important artifacts/paths:".to_string());
        for item in snapshot.artifacts {
            lines.push(format!("- {}", item));
        }
    }

    if !snapshot.open_items.is_empty() {
        lines.push(String::new());
        lines.push("Open items:".to_string());
        for item in snapshot.open_items {
            lines.push(format!("- {}", item));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{AnnotateAble, CallToolRequestParam, RawContent};
    use rmcp::object;

    #[tokio::test]
    async fn cfpm_keeps_success_and_ignores_failed_noise() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("Please keep the script location stable."),
            Message::assistant().with_text("I will locate the file and save output."),
            Message::assistant().with_tool_request(
                "tool_1",
                Ok(CallToolRequestParam {
                    name: "shell_command".into(),
                    arguments: Some(object!({"command":"Get-ChildItem -Recurse"})),
                }),
            ),
            Message::user().with_tool_response(
                "tool_1",
                Ok(rmcp::model::CallToolResult {
                    content: vec![RawContent::text(
                        "Exit code: 0\nSaved to C:\\work\\project\\result.txt\nArchive date: 2024/7/19",
                    )
                    .no_annotation()],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
            Message::assistant().with_tool_request(
                "tool_2",
                Ok(CallToolRequestParam {
                    name: "shell_command".into(),
                    arguments: Some(object!({"command":"Get-Content missing.txt"})),
                }),
            ),
            Message::user().with_tool_response(
                "tool_2",
                Ok(rmcp::model::CallToolResult {
                    content: vec![RawContent::text("Exit code: 1\nFile not found").no_annotation()],
                    structured_content: None,
                    is_error: Some(true),
                    meta: None,
                }),
            ),
            Message::assistant().with_text("Next step is to continue with validated paths."),
            Message::user().with_text("Great, continue."),
            Message::assistant().with_text("ack"),
            Message::user().with_text("keep going"),
            Message::assistant().with_text("step"),
            Message::user().with_text("finish"),
            Message::assistant().with_text("done"),
            Message::user().with_text("thanks"),
        ]);

        let (compacted, _) = compact_messages_progressive_memory(&conversation, true)
            .await
            .expect("cfpm compaction should succeed");

        let memory_message = compacted
            .messages()
            .iter()
            .find(|message| {
                message.metadata == MessageMetadata::agent_only()
                    && message.content.iter().any(|content| {
                        content
                            .as_text()
                            .is_some_and(|text| text.contains("CFPM_MEMORY_V1"))
                    })
            })
            .expect("memory snapshot should exist");

        let text = memory_message.as_concat_text();
        assert!(text.contains("Executed command successfully"));
        assert!(text.contains("C:\\work\\project\\result.txt"));
        assert!(!text.contains("2024/7/19"));
        assert!(!text.contains("Exit code: 1"));
        assert!(!text.contains("File not found"));
    }

    #[test]
    fn extract_paths_ignores_date_tokens() {
        let text = "Saved to C:\\work\\project\\result.txt on 2024/7/19";
        let paths = extract_paths(text);
        assert!(paths
            .iter()
            .any(|path| path == "C:\\work\\project\\result.txt"));
        assert!(!paths.iter().any(|path| path == "2024/7/19"));
    }

    #[tokio::test]
    async fn cfpm_skips_when_too_few_messages_and_not_manual() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("one"),
            Message::assistant().with_text("two"),
            Message::user().with_text("three"),
        ]);

        let (compacted, _) = compact_messages_progressive_memory(&conversation, false)
            .await
            .expect("compaction should not fail");

        assert_eq!(conversation, compacted);
    }
}
