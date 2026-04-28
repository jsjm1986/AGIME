use anyhow::{anyhow, Result};

/// Extract JSON from LLM output with multiple fallback strategies:
/// fenced json, fenced block, array/object slice, then raw text.
pub fn extract_json_block(text: &str) -> String {
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(end) = after.find("```") {
            let inner = after[..end].trim();
            if inner.starts_with('[') || inner.starts_with('{') {
                return inner.to_string();
            }
        }
    }
    if let (Some(start), Some(end)) = (text.find('['), text.rfind(']')) {
        if start < end {
            return text[start..=end].to_string();
        }
    }
    if let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) {
        if start < end {
            return text[start..=end].to_string();
        }
    }
    text.trim().to_string()
}

fn strip_trailing_json_commas(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        if ch == ',' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                i += 1;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

pub fn normalize_loose_json(input: &str) -> String {
    let trimmed = input.trim();
    let s = trimmed.strip_prefix("json\n").unwrap_or(trimmed);
    let s = s
        .replace(['“', '”'], "\"")
        .replace(['\u{2018}', '\u{2019}'], "'")
        .replace('：', ":")
        .replace('，', ",");
    strip_trailing_json_commas(&s)
}

pub fn parse_first_json_value(text: &str) -> Result<serde_json::Value> {
    let json_str = extract_json_block(text);
    let normalized = normalize_loose_json(&json_str);
    let candidates: [&str; 2] = [&json_str, &normalized];
    let mut last_err = None;

    for candidate in candidates {
        let mut stream =
            serde_json::Deserializer::from_str(candidate).into_iter::<serde_json::Value>();
        match stream.next() {
            Some(Ok(parsed)) => return Ok(parsed),
            Some(Err(err)) => last_err = Some(err.to_string()),
            None => last_err = Some("empty json payload".to_string()),
        }
    }

    Err(anyhow!(
        "Failed to parse JSON payload: {}",
        last_err.unwrap_or_else(|| "unknown error".to_string())
    ))
}

pub fn extract_last_assistant_text(messages_json: &str) -> Option<String> {
    let msgs: Vec<serde_json::Value> = serde_json::from_str(messages_json).ok()?;
    for msg in msgs.iter().rev() {
        if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }
        let content = msg.get("content")?;
        if let Some(s) = content.as_str() {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
        if let Some(arr) = content.as_array() {
            for item in arr {
                if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                    if !t.is_empty() {
                        return Some(t.to_string());
                    }
                }
            }
        }
    }
    None
}

pub fn is_non_retryable_external_provider_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    let direct_blockers = [
        "authentication token has been invalidated",
        "authentication failed",
        "401 unauthorized",
        "auth_unavailable",
        "no auth available",
        "provider credentials unavailable",
        "credentials unavailable",
        "no valid coding plan subscription",
        "valid coding plan subscription",
        "subscription has expired",
        "subscription expired",
        "subscription is not active",
        "billing account not active",
    ];
    direct_blockers
        .iter()
        .any(|pattern| lower.contains(pattern))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeBoundarySignal {
    Timeout,
    WaitingExternalProvider,
    TransientProvider,
}

pub fn classify_runtime_boundary_signal(message: &str) -> Option<RuntimeBoundarySignal> {
    let lower = message.to_ascii_lowercase();

    if lower.contains("timed out") || lower.contains("timeout") {
        return Some(RuntimeBoundarySignal::Timeout);
    }
    if is_non_retryable_external_provider_message(message) {
        return Some(RuntimeBoundarySignal::WaitingExternalProvider);
    }
    if lower.contains("524 <unknown status code>")
        || lower.contains("server error (524")
        || lower.contains("server error: server error (524")
    {
        return Some(RuntimeBoundarySignal::WaitingExternalProvider);
    }

    let mentions_capacity = lower.contains("rate limit exceeded")
        || lower.contains("usage limit has been reached")
        || lower.contains("too many requests")
        || lower.contains("resource exhausted");
    let mentions_provider_context = lower.contains("cooling down")
        || lower.contains("all credentials")
        || lower.contains("model")
        || lower.contains("provider")
        || lower.contains("usage limit")
        || lower.contains("credential");
    if mentions_capacity && mentions_provider_context {
        return Some(RuntimeBoundarySignal::WaitingExternalProvider);
    }

    let transient_provider_markers = [
        "server_error",
        "server error",
        "524 <unknown status code>",
        "stream decode error",
        "stream ended without producing a message",
        "error decoding response body",
        "an error occurred while processing your request",
        "fallback complete call timed out",
        "fallback complete call cancelled",
        "fallback complete timed out",
        "fallback complete watchdog timed out",
        "help.openai.com",
        "connection reset",
        "connection closed",
        "connection aborted",
        "failed to connect",
        "error sending request for url",
        "unexpected eof",
    ];
    if transient_provider_markers
        .iter()
        .any(|pattern| lower.contains(pattern))
    {
        return Some(RuntimeBoundarySignal::TransientProvider);
    }

    None
}

pub fn is_waiting_external_provider_message(message: &str) -> bool {
    matches!(
        classify_runtime_boundary_signal(message),
        Some(RuntimeBoundarySignal::WaitingExternalProvider)
    )
}

pub fn is_waiting_external_message(message: &str) -> bool {
    is_waiting_external_provider_message(message)
        || is_transient_provider_execution_message(message)
}

pub fn waiting_external_cooldown_secs(_message: &str) -> i64 {
    std::env::var("TEAM_WAITING_EXTERNAL_COOLDOWN_SECS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(300)
}

pub fn is_transient_provider_execution_message(message: &str) -> bool {
    matches!(
        classify_runtime_boundary_signal(message),
        Some(RuntimeBoundarySignal::TransientProvider)
    )
}

pub fn blocker_fingerprint(message: &str) -> Option<String> {
    let normalized = message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    let mut compact = normalized;
    if compact.len() > 180 {
        compact.truncate(180);
    }
    Some(compact)
}

pub fn is_retryable_error(e: &anyhow::Error) -> bool {
    let msg = e.to_string().to_lowercase();
    if is_non_retryable_external_provider_message(&msg) {
        return false;
    }
    if is_waiting_external_message(&msg) {
        return false;
    }
    let retryable_patterns = [
        "timeout",
        "timed out",
        "context deadline exceeded",
        "rate limit",
        "too many requests",
        "429",
        "503",
        "502",
        "504",
        "service unavailable",
        "bad gateway",
        "gateway timeout",
        "connection reset",
        "connection closed",
        "connection aborted",
        "connection refused",
        "broken pipe",
        "unexpected eof",
        "end of file",
        "error decoding response body",
        "stream decode error",
        "stream ended without producing a message",
        "temporarily unavailable",
        "overloaded",
        "upstream connect error",
        "tls handshake timeout",
        "tls handshake eof",
        "handshake eof",
        "transport error",
    ];
    retryable_patterns.iter().any(|p| msg.contains(p))
}
