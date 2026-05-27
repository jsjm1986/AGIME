//! Pure helpers harvested from `server_harness_host`.
//!
//! These are side-effect-free utilities (string trimming, regex matching,
//! deliverable-token normalization) that the team-side `ServerHarnessHost`
//! uses today. Extracting them here keeps the team-server file focused on
//! orchestration and lets the desktop runtime reuse the same canonical
//! parsers when the harness host itself is migrated.
//!
//! The orchestration body of `ServerHarnessHost` (Mongo persistence, stream
//! broadcasters, runtime control registry, worker mirrors) still lives on
//! the team-server side and is the next major migration target.

use std::sync::OnceLock;

use regex::Regex;

/// Build a chat title from the user's first message.
///
/// Returns `None` when the message is whitespace-only. Long messages are
/// truncated to 47 characters plus an ellipsis (so the final string is at
/// most 50 characters, counting unicode code points).
pub fn derive_chat_title_from_user_message(user_message: &str) -> Option<String> {
    if user_message.trim().is_empty() {
        return None;
    }
    let preview: String = if user_message.chars().count() > 50 {
        let truncated: String = user_message.chars().take(47).collect();
        format!("{}...", truncated)
    } else {
        user_message.to_string()
    };
    Some(preview)
}

/// Normalize a deliverable target token (path-like / namespaced identifier).
///
/// Strips bracketing punctuation, rejects pure URLs, normalizes backslashes
/// to forward slashes, and accepts only tokens that look like paths or
/// namespaced identifiers (containing `/` or `:`) or tokens with one of the
/// known artifact extensions.
pub fn normalize_deliverable_token(token: &str) -> Option<String> {
    let trimmed = token
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'
            )
        })
        .trim();
    if trimmed.is_empty() || trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return None;
    }
    let normalized = trimmed.replace('\\', "/");
    let stable = normalized.contains('/')
        || normalized.contains(':')
        || [
            ".md", ".txt", ".json", ".yaml", ".yml", ".csv", ".html", ".rs", ".py", ".ts", ".tsx",
            ".js",
        ]
        .iter()
        .any(|suffix| normalized.to_ascii_lowercase().ends_with(suffix));
    if !stable {
        return None;
    }
    Some(normalized)
}

/// Cached regex used to find candidate deliverable targets in a user message.
pub fn deliverable_target_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)+")
            .expect("deliverable target regex must compile")
    })
}

/// Pull deliverable targets out of a user message, deduplicating in order.
pub fn infer_targets_from_user_message(user_message: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let regex = deliverable_target_regex();
    for capture in regex.find_iter(user_message) {
        let raw = capture.as_str();
        if let Some(target) = normalize_deliverable_token(raw) {
            if !targets.iter().any(|existing| existing == &target) {
                targets.push(target);
            }
        }
    }
    targets
}

/// Whether a target identifier names a contextual surface (channel, document)
/// rather than a concrete deliverable artifact.
pub fn is_contextual_host_target(target: &str) -> bool {
    let normalized = target.trim().to_ascii_lowercase();
    normalized.starts_with("channel:") || normalized.starts_with("document:")
}

/// Detect explicit user requests for delegation / swarm / parallel work in
/// either English or Chinese.
pub fn has_explicit_delegation_request(user_message: &str) -> bool {
    let lowered = user_message.to_ascii_lowercase();
    lowered.contains("subagent")
        || lowered.contains("swarm")
        || lowered.contains("single worker")
        || lowered.contains("one worker")
        || lowered.contains("multiple workers")
        || lowered.contains("multiple worker")
        || user_message.contains("子代理")
        || user_message.contains("多 worker")
        || user_message.contains("多个 worker")
        || user_message.contains("多代理")
        || user_message.contains("并行")
}

/// Detect the inline assistant-text notifications the harness emits when the
/// context-runtime auto-compactor kicks in. Used by the stream layer to
/// suppress duplicate user-visible compaction notices.
pub fn is_compaction_inline_notification(message: &str) -> bool {
    let normalized = message.trim().to_ascii_lowercase();
    normalized.starts_with("exceeded auto-compact threshold of ")
        || normalized.starts_with("context limit reached. compacting to continue conversation...")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_short_message_is_returned_verbatim() {
        assert_eq!(
            derive_chat_title_from_user_message("hello"),
            Some("hello".to_string())
        );
    }

    #[test]
    fn title_long_message_is_truncated_to_50_chars_with_ellipsis() {
        let long = "a".repeat(80);
        let title = derive_chat_title_from_user_message(&long).unwrap();
        assert_eq!(title.chars().count(), 50);
        assert!(title.ends_with("..."));
    }

    #[test]
    fn title_whitespace_message_yields_none() {
        assert_eq!(derive_chat_title_from_user_message("   \n\t"), None);
    }

    #[test]
    fn deliverable_token_accepts_paths() {
        assert_eq!(
            normalize_deliverable_token("docs/spec.md"),
            Some("docs/spec.md".to_string())
        );
        assert_eq!(
            normalize_deliverable_token(r"docs\spec.md"),
            Some("docs/spec.md".to_string())
        );
    }

    #[test]
    fn deliverable_token_strips_bracket_punctuation() {
        assert_eq!(
            normalize_deliverable_token("[docs/a.md]"),
            Some("docs/a.md".to_string())
        );
        assert_eq!(
            normalize_deliverable_token("\"channel:abc\""),
            Some("channel:abc".to_string())
        );
    }

    #[test]
    fn deliverable_token_rejects_urls_and_bare_words() {
        assert_eq!(normalize_deliverable_token("https://example.com/x"), None);
        assert_eq!(normalize_deliverable_token("hello"), None);
        assert_eq!(normalize_deliverable_token(""), None);
    }

    #[test]
    fn deliverable_token_accepts_known_extensions_without_path() {
        assert_eq!(
            normalize_deliverable_token("notes.md"),
            Some("notes.md".to_string())
        );
        assert_eq!(
            normalize_deliverable_token("script.RS"),
            Some("script.RS".to_string())
        );
    }

    #[test]
    fn infer_targets_dedups_in_order() {
        let targets =
            infer_targets_from_user_message("see docs/a.md and again docs/a.md plus src/b.rs");
        assert_eq!(
            targets,
            vec!["docs/a.md".to_string(), "src/b.rs".to_string()]
        );
    }

    #[test]
    fn infer_targets_extracts_post_scheme_path_for_urls() {
        // The regex deliberately matches the path-like tail of URLs (the
        // scheme separator `://` breaks the character class), so URLs
        // contribute their host+path as a target. Pure URL rejection only
        // happens for tokens whose *full* text is the URL (the
        // `normalize_deliverable_token` http(s) prefix check).
        let targets = infer_targets_from_user_message("download from https://example.com/spec");
        assert_eq!(targets, vec!["example.com/spec".to_string()]);
    }

    #[test]
    fn contextual_host_target_recognizes_prefixes() {
        assert!(is_contextual_host_target("channel:abc"));
        assert!(is_contextual_host_target("Document:1"));
        assert!(!is_contextual_host_target("docs/a.md"));
    }

    #[test]
    fn delegation_request_recognizes_english_and_chinese() {
        assert!(has_explicit_delegation_request("please use a subagent"));
        assert!(has_explicit_delegation_request("Run a Swarm of workers"));
        assert!(has_explicit_delegation_request("启用多个 worker 并行执行"));
        assert!(has_explicit_delegation_request("这个任务请并行处理"));
        assert!(!has_explicit_delegation_request("just answer please"));
    }

    #[test]
    fn compaction_inline_notification_recognizes_known_strings() {
        assert!(is_compaction_inline_notification(
            "Exceeded auto-compact threshold of 80%"
        ));
        assert!(is_compaction_inline_notification(
            "Context limit reached. Compacting to continue conversation..."
        ));
        assert!(!is_compaction_inline_notification(
            "regular assistant reply"
        ));
    }
}
