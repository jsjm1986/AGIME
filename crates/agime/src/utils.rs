use tokio_util::sync::CancellationToken;
use unicode_normalization::UnicodeNormalization;

/// Check if a character is in the Unicode Tags Block range (U+E0000-U+E007F)
/// These characters are invisible and can be used for steganographic attacks
fn is_in_unicode_tag_range(c: char) -> bool {
    matches!(c, '\u{E0000}'..='\u{E007F}')
}

pub fn contains_unicode_tags(text: &str) -> bool {
    text.chars().any(is_in_unicode_tag_range)
}

/// Sanitize Unicode Tags Block characters from text
pub fn sanitize_unicode_tags(text: &str) -> String {
    let normalized: String = text.nfc().collect();

    normalized
        .chars()
        .filter(|&c| !is_in_unicode_tag_range(c))
        .collect()
}

fn strip_summary_token_punctuation(token: &str) -> &str {
    token.trim_matches(|c: char| {
        matches!(
            c,
            '`' | '*'
                | '_'
                | '.'
                | ','
                | ':'
                | ';'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '/'
                | '\\'
                | '"'
                | '\''
                | '!'
                | '?'
                | '='
                | '+'
        )
    })
}

fn looks_like_ascii_summary_context(token: Option<&str>) -> bool {
    token.is_some_and(|value| {
        let stripped = strip_summary_token_punctuation(value);
        !stripped.is_empty() && stripped.chars().any(|ch| ch.is_ascii_alphanumeric())
    })
}

fn is_summary_noise_separator(token: &str) -> bool {
    matches!(
        strip_summary_token_punctuation(token),
        "бк" | "БК" | "бд" | "БД"
    )
}

fn normalize_summary_noise_tokens(line: &str) -> String {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() {
        return String::new();
    }

    let mut out = Vec::with_capacity(tokens.len());
    for (idx, token) in tokens.iter().enumerate() {
        if is_summary_noise_separator(token)
            && (looks_like_ascii_summary_context(tokens.get(idx.wrapping_sub(1)).copied())
                || looks_like_ascii_summary_context(tokens.get(idx + 1).copied()))
        {
            if out
                .last()
                .is_none_or(|value: &String| value.as_str() != "-")
            {
                out.push("-".to_string());
            }
            continue;
        }
        out.push((*token).to_string());
    }
    out.join(" ")
}

/// Normalize user-visible delegation summaries without touching general message content.
///
/// This is intentionally narrow:
/// - keeps legitimate Unicode
/// - removes invisible Unicode Tags via `sanitize_unicode_tags`
/// - normalizes whitespace/newlines
/// - replaces clearly broken short separator tokens like standalone `бк`
///   only in ASCII/path-oriented summary contexts
pub fn normalize_delegation_summary_text(text: &str) -> String {
    let normalized = sanitize_unicode_tags(text)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut lines = Vec::new();
    let mut previous_blank = false;

    for raw_line in normalized.lines() {
        let line = normalize_summary_noise_tokens(raw_line.trim());
        if line.is_empty() {
            if !previous_blank {
                lines.push(String::new());
            }
            previous_blank = true;
            continue;
        }
        lines.push(line);
        previous_blank = false;
    }

    lines.join("\n").trim().to_string()
}

/// Safely truncate a string at character boundaries, not byte boundaries
///
/// This function ensures that multi-byte UTF-8 characters (like Japanese, emoji, etc.)
/// are not split in the middle, which would cause a panic.
///
/// # Arguments
/// * `s` - The string to truncate
/// * `max_chars` - Maximum number of characters to keep
///
/// # Returns
/// A truncated string with "..." appended if truncation occurred
pub fn safe_truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

pub fn is_token_cancelled(cancellation_token: &Option<CancellationToken>) -> bool {
    cancellation_token
        .as_ref()
        .is_some_and(|t| t.is_cancelled())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains_unicode_tags() {
        // Test detection of Unicode Tags Block characters
        assert!(contains_unicode_tags("Hello\u{E0041}world"));
        assert!(contains_unicode_tags("\u{E0000}"));
        assert!(contains_unicode_tags("\u{E007F}"));
        assert!(!contains_unicode_tags("Hello world"));
        assert!(!contains_unicode_tags("Hello 世界 🌍"));
        assert!(!contains_unicode_tags(""));
    }

    #[test]
    fn test_sanitize_unicode_tags() {
        // Test that Unicode Tags Block characters are removed
        let malicious = "Hello\u{E0041}\u{E0042}\u{E0043}world"; // Invisible "ABC"
        let cleaned = sanitize_unicode_tags(malicious);
        assert_eq!(cleaned, "Helloworld");
    }

    #[test]
    fn test_sanitize_unicode_tags_preserves_legitimate_unicode() {
        // Test that legitimate Unicode characters are preserved
        let clean_text = "Hello world 世界 🌍";
        let cleaned = sanitize_unicode_tags(clean_text);
        assert_eq!(cleaned, clean_text);
    }

    #[test]
    fn test_sanitize_unicode_tags_empty_string() {
        let empty = "";
        let cleaned = sanitize_unicode_tags(empty);
        assert_eq!(cleaned, "");
    }

    #[test]
    fn test_sanitize_unicode_tags_only_malicious() {
        // Test string containing only Unicode Tags characters
        let only_malicious = "\u{E0041}\u{E0042}\u{E0043}";
        let cleaned = sanitize_unicode_tags(only_malicious);
        assert_eq!(cleaned, "");
    }

    #[test]
    fn test_sanitize_unicode_tags_mixed_content() {
        // Test mixed legitimate and malicious Unicode
        let mixed = "Hello\u{E0041} 世界\u{E0042} 🌍\u{E0043}!";
        let cleaned = sanitize_unicode_tags(mixed);
        assert_eq!(cleaned, "Hello 世界 🌍!");
    }

    #[test]
    fn test_normalize_delegation_summary_text_replaces_noise_separator_in_ascii_context() {
        let raw = "Result: README.md **does not exist** бк file not found.";
        assert_eq!(
            normalize_delegation_summary_text(raw),
            "Result: README.md **does not exist** - file not found."
        );
    }

    #[test]
    fn test_normalize_delegation_summary_text_preserves_legitimate_cyrillic_words() {
        let raw = "Результат: привет бк мир";
        assert_eq!(normalize_delegation_summary_text(raw), raw);
    }

    #[test]
    fn test_normalize_delegation_summary_text_replaces_bd_separator_in_ascii_context() {
        let raw = "swarm бд 7 completed";
        assert_eq!(
            normalize_delegation_summary_text(raw),
            "swarm - 7 completed"
        );
    }

    #[test]
    fn test_safe_truncate_ascii() {
        assert_eq!(safe_truncate("hello world", 20), "hello world");
        assert_eq!(safe_truncate("hello world", 8), "hello...");
        assert_eq!(safe_truncate("hello", 5), "hello");
        assert_eq!(safe_truncate("hello", 3), "...");
    }

    #[test]
    fn test_safe_truncate_japanese() {
        // Japanese characters: "こんにちは世界" (Hello World)
        let japanese = "こんにちは世界";
        assert_eq!(safe_truncate(japanese, 10), japanese);
        assert_eq!(safe_truncate(japanese, 5), "こん...");
        assert_eq!(safe_truncate(japanese, 7), japanese);
    }

    #[test]
    fn test_safe_truncate_mixed() {
        // Mixed ASCII and Japanese
        let mixed = "Hello こんにちは";
        assert_eq!(safe_truncate(mixed, 20), mixed);
        assert_eq!(safe_truncate(mixed, 8), "Hello...");
    }
}
