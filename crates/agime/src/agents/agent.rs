use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use futures::stream::BoxStream;
use futures::{stream, FutureExt, Stream, StreamExt, TryStreamExt};
use uuid::Uuid;

use super::final_output_tool::FinalOutputTool;
use super::platform_tools;
use super::tool_execution::{ToolCallResult, CHAT_MODE_TOOL_SKIPPED_RESPONSE, DECLINED_RESPONSE};
use crate::action_required_manager::ActionRequiredManager;
use crate::agents::extension::{ExtensionConfig, ExtensionError, ExtensionResult, ToolInfo};
use crate::agents::extension_manager::{get_parameter_names, ExtensionManager};
use crate::agents::extension_manager_extension::MANAGE_EXTENSIONS_TOOL_NAME_COMPLETE;
use crate::agents::final_output_tool::{FINAL_OUTPUT_CONTINUATION_MESSAGE, FINAL_OUTPUT_TOOL_NAME};
use crate::agents::platform_tools::PLATFORM_MANAGE_SCHEDULE_TOOL_NAME;
use crate::agents::prompt_manager::PromptManager;
use crate::agents::retry::{RetryManager, RetryResult};
use crate::agents::router_tools::ROUTER_LLM_SEARCH_TOOL_NAME;
use crate::agents::subagent_task_config::TaskConfig;
use crate::agents::subagent_tool::{
    create_subagent_tool, handle_subagent_tool, SUBAGENT_TOOL_NAME,
};
use crate::agents::tool_route_manager::ToolRouteManager;
use crate::agents::tool_router_index_manager::ToolRouterIndexManager;
use crate::agents::types::SessionConfig;
use crate::agents::types::{FrontendTool, SharedProvider, ToolResultReceiver};
use crate::config::{get_enabled_extensions, AgimeMode, Config};
use crate::context_mgmt::{
    check_if_compaction_needed, compact_messages_with_active_strategy, current_compaction_strategy,
    ContextCompactionStrategy, DEFAULT_COMPACTION_THRESHOLD,
};
use crate::conversation::message::{
    ActionRequiredData, Message, MessageContent, MessageMetadata, SystemNotificationType,
    ToolRequest,
};
use crate::conversation::{debug_conversation_fix, fix_conversation, Conversation};
use crate::mcp_utils::ToolResult;
use crate::permission::permission_inspector::PermissionInspector;
use crate::permission::permission_judge::PermissionCheckResult;
use crate::permission::PermissionConfirmation;
use crate::providers::base::Provider;
use crate::providers::errors::ProviderError;
use crate::recipe::{Author, Recipe, Response, Settings, SubRecipe};
use crate::scheduler_trait::SchedulerTrait;
use crate::security::security_inspector::SecurityInspector;
use crate::session::extension_data::{EnabledExtensionsState, ExtensionState};
use crate::session::session_manager::{CfpmRuntimeReport, MemoryFactStatus};
use crate::session::{Session, SessionManager};
use crate::tool_inspection::ToolInspectionManager;
use crate::tool_monitor::RepetitionInspector;
use crate::utils::is_token_cancelled;
use regex::Regex;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, ErrorCode, ErrorData, GetPromptResult, Prompt,
    Role, ServerNotification, Tool,
};
use serde_json::Value;
use std::sync::OnceLock;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};

const DEFAULT_MAX_TURNS: u32 = 1000;
const COMPACTION_THINKING_TEXT: &str = "AGIME is compacting the conversation...";
const MAX_COMPACTION_ATTEMPTS: u32 = 3; // Maximum compaction attempts per reply to prevent infinite loops
const MAX_COMPACTION_ATTEMPTS_CFPM: u32 = 6;
const MAX_RUNTIME_MEMORY_ITEMS_PER_SECTION: usize = 8;
const MAX_RUNTIME_MEMORY_ITEM_CHARS: usize = 220;
const CFPM_RUNTIME_VISIBILITY_KEY: &str = "AGIME_CFPM_RUNTIME_VISIBILITY";
const CFPM_TOOL_GATE_VISIBILITY_KEY: &str = "AGIME_CFPM_TOOL_GATE_VISIBILITY";
const CFPM_RUNTIME_NOTIFICATION_PREFIX: &str = "[CFPM_RUNTIME_V1]";
const CFPM_TOOL_GATE_NOTIFICATION_PREFIX: &str = "[CFPM_TOOL_GATE_V1]";
const CFPM_PRE_TOOL_GATE_KEY: &str = "AGIME_CFPM_PRE_TOOL_GATE";
pub const MANUAL_COMPACT_TRIGGERS: &[&str] =
    &["Please compact this conversation", "/compact", "/summarize"];

#[derive(Debug, Clone)]
struct CfpmToolGateEvent {
    tool: String,
    target: String,
    path: String,
    original_command: String,
    rewritten_command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CfpmRuntimeVisibility {
    Off,
    Brief,
    Debug,
}

impl CfpmRuntimeVisibility {
    fn from_config_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "disable" | "disabled" | "0" => Self::Off,
            "debug" | "verbose" | "full" | "2" => Self::Debug,
            _ => Self::Brief,
        }
    }
}

fn current_cfpm_runtime_visibility() -> CfpmRuntimeVisibility {
    let config = Config::global();
    let raw = config
        .get_param::<String>(CFPM_RUNTIME_VISIBILITY_KEY)
        .unwrap_or_else(|_| "brief".to_string());
    CfpmRuntimeVisibility::from_config_value(&raw)
}

fn current_cfpm_tool_gate_visibility() -> CfpmRuntimeVisibility {
    let config = Config::global();
    if let Ok(raw) = config.get_param::<String>(CFPM_TOOL_GATE_VISIBILITY_KEY) {
        return CfpmRuntimeVisibility::from_config_value(&raw);
    }
    current_cfpm_runtime_visibility()
}

fn build_cfpm_runtime_inline_message(
    report: &CfpmRuntimeReport,
    visibility: CfpmRuntimeVisibility,
) -> String {
    let verbosity = match visibility {
        CfpmRuntimeVisibility::Off => "off",
        CfpmRuntimeVisibility::Brief => "brief",
        CfpmRuntimeVisibility::Debug => "debug",
    };

    let payload = serde_json::json!({
        "version": "v1",
        "verbosity": verbosity,
        "reason": report.reason,
        "mode": report.mode,
        "acceptedCount": report.accepted_count,
        "rejectedCount": report.rejected_count,
        "prunedCount": report.pruned_count,
        "factCount": report.fact_count,
        "rejectedReasonBreakdown": report.rejected_reason_breakdown,
    });

    format!("{} {}", CFPM_RUNTIME_NOTIFICATION_PREFIX, payload)
}

fn should_emit_cfpm_runtime_notification(
    visibility: CfpmRuntimeVisibility,
    report: &CfpmRuntimeReport,
) -> bool {
    match visibility {
        CfpmRuntimeVisibility::Off => false,
        CfpmRuntimeVisibility::Brief => {
            report.accepted_count > 0
                || report.rejected_count > 0
                || report.pruned_count > 0
                || report.mode != "noop"
        }
        CfpmRuntimeVisibility::Debug => true,
    }
}

fn build_cfpm_tool_gate_inline_message(
    event: &CfpmToolGateEvent,
    visibility: CfpmRuntimeVisibility,
) -> String {
    let verbosity = match visibility {
        CfpmRuntimeVisibility::Off => "off",
        CfpmRuntimeVisibility::Brief => "brief",
        CfpmRuntimeVisibility::Debug => "debug",
    };

    let payload = serde_json::json!({
        "version": "v1",
        "verbosity": verbosity,
        "action": "rewrite_known_folder_probe",
        "tool": event.tool,
        "target": event.target,
        "path": event.path,
        "originalCommand": event.original_command,
        "rewrittenCommand": event.rewritten_command,
    });

    format!("{} {}", CFPM_TOOL_GATE_NOTIFICATION_PREFIX, payload)
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

fn push_runtime_memory_item(target: &mut Vec<String>, seen: &mut HashSet<String>, content: &str) {
    if target.len() >= MAX_RUNTIME_MEMORY_ITEMS_PER_SECTION {
        return;
    }

    let normalized = content.trim().to_ascii_lowercase();
    if normalized.is_empty() || seen.contains(&normalized) {
        return;
    }

    let sanitized = truncate_chars(content.trim(), MAX_RUNTIME_MEMORY_ITEM_CHARS);
    if sanitized.is_empty() {
        return;
    }

    seen.insert(normalized);
    target.push(sanitized);
}

fn looks_like_runtime_date_token(value: &str) -> bool {
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

fn looks_like_runtime_artifact(content: &str) -> bool {
    let token = content.trim();
    if token.len() < 3
        || token.len() > 320
        || token.contains('\n')
        || token.contains('\r')
        || token.contains('\u{1b}')
        || token.contains('�')
        || looks_like_runtime_date_token(token)
    {
        return false;
    }

    let lowered = token.to_ascii_lowercase();
    if lowered.contains("[stdout]")
        || lowered.contains("[stderr]")
        || lowered.contains("running ")
        || lowered.contains("traceback")
        || lowered.contains("stack trace")
        || lowered.contains("private note: output was")
        || lowered.contains("truncated output")
        || lowered.contains("do not show tmp file to user")
        || lowered.contains("categoryinfo")
        || lowered.contains("fullyqualifiederrorid")
        || lowered.contains("itemnotfoundexception")
        || lowered.contains("pathnotfound")
        || lowered.contains("\\appdata\\local\\temp\\")
        || lowered.contains("/appdata/local/temp/")
        || lowered.contains("\\temp\\.")
        || lowered.ends_with(".tmp")
        || lowered.contains("available windows:")
    {
        return false;
    }

    if token.starts_with("http://") || token.starts_with("https://") {
        return false;
    }
    if token.split_whitespace().count() > 8 {
        return false;
    }
    if token.contains(":\\")
        && !matches!(
            (token.chars().nth(0), token.chars().nth(1), token.chars().nth(2)),
            (Some(drive), Some(':'), Some('\\')) if drive.is_ascii_alphabetic()
        )
    {
        return false;
    }
    if token.contains(":\\") && token.chars().skip(2).any(|ch| ch == ':') {
        return false;
    }
    let has_alpha = token.chars().any(|ch| ch.is_alphabetic());
    if token.contains('/') && !token.contains('\\') && !token.contains(':') && !has_alpha {
        return false;
    }

    let is_windows_path = token.contains(":\\")
        || token.starts_with("\\\\")
        || token.starts_with(".\\")
        || token.starts_with("~\\");
    let is_unix_path = token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with('/')
        || token.starts_with("~/");
    let has_path_separator = token.contains('\\') || token.contains('/');

    (is_windows_path || is_unix_path || has_path_separator)
        && !token.starts_with("--")
        && !token.starts_with('-')
}

fn looks_like_failure_context_text(content: &str) -> bool {
    let lowered = content.to_ascii_lowercase();
    let markers = [
        "cannot find path",
        "path not found",
        "the system cannot find the path specified",
        "does not exist",
        "not found",
        "itemnotfoundexception",
        "pathnotfound",
        "cannot access",
        "access denied",
        "permission denied",
        "enoent",
        "no such file",
        "系统找不到指定的路径",
        "找不到指定的路径",
        "找不到路径",
        "未找到",
        "不存在",
        "无法访问",
        "访问不了",
        "权限不足",
        "拒绝访问",
    ];
    markers.iter().any(|marker| lowered.contains(marker))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KnownFolderTarget {
    Desktop,
    Documents,
    Downloads,
}

impl KnownFolderTarget {
    fn segment(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Documents => "documents",
            Self::Downloads => "downloads",
        }
    }
}

fn cfpm_pre_tool_gate_enabled() -> bool {
    let config = Config::global();
    let raw = config
        .get_param::<String>(CFPM_PRE_TOOL_GATE_KEY)
        .unwrap_or_else(|_| "on".to_string())
        .trim()
        .to_ascii_lowercase();
    !matches!(raw.as_str(), "off" | "disable" | "disabled" | "0" | "false")
}

fn is_shell_like_tool(tool_name: &str) -> bool {
    let lowered = tool_name.to_ascii_lowercase();
    lowered.contains("shell") || lowered.contains("terminal")
}

fn supports_known_folder_probe_rewrite(command: &str) -> bool {
    let lowered = command.to_ascii_lowercase();
    lowered.contains("$env:userprofile")
        || lowered.contains("%userprofile%")
        || lowered.contains("$home")
        || lowered.contains("~/")
        || lowered.contains("~\\")
        || lowered.contains("[environment]::getfolderpath")
}

fn target_mentions_in_command(command: &str) -> Vec<KnownFolderTarget> {
    let lowered = command.to_ascii_lowercase();
    let mut targets = Vec::new();
    if lowered.contains("desktop") {
        targets.push(KnownFolderTarget::Desktop);
    }
    if lowered.contains("documents") {
        targets.push(KnownFolderTarget::Documents);
    }
    if lowered.contains("downloads") {
        targets.push(KnownFolderTarget::Downloads);
    }
    targets
}

fn is_explicit_absolute_path_reference(command: &str) -> bool {
    command.contains(":\\")
        || command.contains(":/")
        || command.contains("\\\\")
        || command.contains("/Users/")
        || command.contains("/home/")
}

fn normalize_path_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\''
                    | '`'
                    | ','
                    | ';'
                    | '.'
                    | ':'
                    | '!'
                    | '?'
                    | '，'
                    | '。'
                    | '：'
                    | '；'
                    | '！'
                    | '？'
                    | '、'
                    | '“'
                    | '”'
                    | ')'
                    | '('
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
            )
        })
        .trim()
        .to_string()
}

fn path_candidate_regexes() -> &'static [Regex; 2] {
    static REGEXES: OnceLock<[Regex; 2]> = OnceLock::new();
    REGEXES.get_or_init(|| {
        [
            Regex::new(
                r#"[A-Za-z]:\\(?:[^\\/:*?"<>|\r\n\s`。，：；！？、]+\\)*[^\\/:*?"<>|\r\n\s`。，：；！？、]*"#,
            )
            .expect("valid windows path regex"),
            Regex::new(
                r"(?:\./|\.\./|/)?(?:[A-Za-z0-9._-]+/)+[A-Za-z0-9._-]+(?:\.[A-Za-z0-9._-]+)?",
            )
            .expect("valid unix path regex"),
        ]
    })
}

fn is_symbolic_path_reference(path: &str) -> bool {
    let lowered = path.trim().to_ascii_lowercase();
    lowered.starts_with("$env:")
        || lowered.starts_with("$home")
        || lowered.starts_with("~/")
        || lowered.starts_with("~\\")
        || lowered.starts_with("%userprofile%")
        || lowered.starts_with("%homepath%")
        || lowered.contains("[environment]::getfolderpath")
        || lowered.contains("%userprofile%")
}

fn is_concrete_absolute_path(path: &str) -> bool {
    let token = normalize_path_token(path);
    if token.is_empty() {
        return false;
    }

    token.contains(":\\")
        || token.contains(":/")
        || token.starts_with("\\\\")
        || token.starts_with('/')
}

fn extract_path_candidates_from_text(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    let whole = normalize_path_token(content);
    if !whole.chars().any(|ch| ch.is_whitespace())
        && looks_like_runtime_artifact(&whole)
        && !is_symbolic_path_reference(&whole)
    {
        let key = whole.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(whole);
        }
    }

    for regex in path_candidate_regexes() {
        for captures in regex.find_iter(content) {
            let token = normalize_path_token(captures.as_str());
            if token.len() < 3
                || !looks_like_runtime_artifact(&token)
                || is_symbolic_path_reference(&token)
            {
                continue;
            }
            let key = token.to_ascii_lowercase();
            if seen.insert(key) {
                out.push(token);
            }
        }
    }

    for raw in content.split_whitespace() {
        let token = normalize_path_token(raw);
        if token.len() < 3
            || !looks_like_runtime_artifact(&token)
            || is_symbolic_path_reference(&token)
        {
            continue;
        }
        let key = token.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(token);
        }
    }

    out
}

fn path_matches_known_folder(path: &str, target: KnownFolderTarget) -> bool {
    let normalized_path = normalize_path_token(path);
    if normalized_path.is_empty()
        || is_symbolic_path_reference(&normalized_path)
        || !is_concrete_absolute_path(&normalized_path)
    {
        return false;
    }

    let mut normalized = normalized_path.to_ascii_lowercase().replace('/', "\\");
    while normalized.ends_with('\\') {
        normalized.pop();
    }
    let folder_suffix = format!("\\{}", target.segment());
    normalized.ends_with(&folder_suffix)
}

fn canonicalize_path_for_compare(path: &str) -> Option<String> {
    let normalized = normalize_path_token(path);
    if normalized.is_empty()
        || is_symbolic_path_reference(&normalized)
        || !is_concrete_absolute_path(&normalized)
    {
        return None;
    }
    let mut canonical = normalized.to_ascii_lowercase().replace('/', "\\");
    while canonical.ends_with('\\') {
        canonical.pop();
    }
    if canonical.is_empty() {
        return None;
    }
    Some(canonical)
}

fn collect_invalid_paths_from_facts(
    facts: &[crate::session::session_manager::MemoryFact],
) -> HashSet<String> {
    let mut invalid_paths = HashSet::new();
    for fact in facts {
        if fact.status != MemoryFactStatus::Active && !fact.pinned {
            continue;
        }
        let category = fact.category.to_ascii_lowercase();
        if !(category == "invalid_path" || category.contains("invalid_path")) {
            continue;
        }
        for candidate in extract_path_candidates_from_text(&fact.content) {
            if let Some(canonical) = canonicalize_path_for_compare(&candidate) {
                invalid_paths.insert(canonical);
            }
        }
    }
    invalid_paths
}

fn collect_recent_invalid_paths_from_session(session: &Session) -> HashSet<String> {
    let mut invalid_paths = HashSet::new();
    let Some(conversation) = session.conversation.as_ref() else {
        return invalid_paths;
    };

    for message in conversation.messages().iter().rev().take(40) {
        for content in &message.content {
            match content {
                MessageContent::Text(text) => {
                    for line in text
                        .text
                        .lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                    {
                        if !looks_like_failure_context_text(line) {
                            continue;
                        }
                        for candidate in extract_path_candidates_from_text(line) {
                            if let Some(canonical) = canonicalize_path_for_compare(&candidate) {
                                invalid_paths.insert(canonical);
                            }
                        }
                    }
                }
                MessageContent::ToolResponse(response) => {
                    let output = match &response.tool_result {
                        Ok(result) => result
                            .content
                            .iter()
                            .filter_map(|item| item.as_text().map(|text| text.text.clone()))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        Err(error_message) => error_message.to_string(),
                    };
                    for line in output
                        .lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                    {
                        if !looks_like_failure_context_text(line) {
                            continue;
                        }
                        for candidate in extract_path_candidates_from_text(line) {
                            if let Some(canonical) = canonicalize_path_for_compare(&candidate) {
                                invalid_paths.insert(canonical);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    invalid_paths
}

fn looks_like_known_folder_validation_command(command: Option<&str>) -> bool {
    let Some(command) = command else {
        return false;
    };
    let lowered = command.to_ascii_lowercase();
    lowered.contains("getfolderpath")
        || lowered.contains("user shell folders")
        || lowered.contains("onedrive")
}

fn find_known_folder_path_from_facts(
    facts: &[crate::session::session_manager::MemoryFact],
    target: KnownFolderTarget,
    extra_invalid_paths: Option<&HashSet<String>>,
) -> Option<String> {
    let mut invalid_paths = collect_invalid_paths_from_facts(facts);
    if let Some(extra_paths) = extra_invalid_paths {
        invalid_paths.extend(extra_paths.iter().cloned());
    }
    let mut best: Option<(f64, String)> = None;

    for fact in facts {
        if fact.status != MemoryFactStatus::Active && !fact.pinned {
            continue;
        }
        if looks_like_failure_context_text(&fact.content) {
            continue;
        }

        let category = fact.category.to_ascii_lowercase();
        let preferred_category = category.contains("artifact") || category.contains("path");
        let is_low_trust_auto_fact = fact.source == "cfpm_auto"
            && fact.validation_command.is_none()
            && fact.evidence_count < 2
            && fact.confidence < 0.85;
        if is_low_trust_auto_fact {
            continue;
        }

        for candidate in extract_path_candidates_from_text(&fact.content) {
            if path_matches_known_folder(&candidate, target) {
                let missing_validation_for_auto_known_folder = fact.source == "cfpm_auto"
                    && !looks_like_known_folder_validation_command(
                        fact.validation_command.as_deref(),
                    );
                if missing_validation_for_auto_known_folder {
                    continue;
                }

                let Some(canonical_candidate) = canonicalize_path_for_compare(&candidate) else {
                    continue;
                };
                if invalid_paths.contains(&canonical_candidate) {
                    continue;
                }
                if !std::path::Path::new(&candidate).is_dir() {
                    continue;
                }

                let mut score = fact.confidence;
                if preferred_category {
                    score += 0.2;
                }
                if fact.pinned {
                    score += 0.1;
                }
                if fact.source == "user" {
                    score += 0.1;
                }
                if looks_like_known_folder_validation_command(fact.validation_command.as_deref()) {
                    score += 0.15;
                }

                let should_take = best
                    .as_ref()
                    .map(|(current_score, _)| score > *current_score)
                    .unwrap_or(true);
                if should_take {
                    best = Some((score, candidate));
                }
            }
        }
    }

    best.map(|(_, path)| path)
}

fn quote_shell_string(input: &str) -> String {
    format!("'{}'", input.replace('\'', "''"))
}

fn rewrite_known_folder_probe_in_command(
    command: &str,
    target: KnownFolderTarget,
    resolved_path: &str,
) -> Option<String> {
    if is_symbolic_path_reference(resolved_path) || !is_concrete_absolute_path(resolved_path) {
        return None;
    }

    let segment = target.segment();
    let quoted_path = quote_shell_string(resolved_path);

    let patterns = [
        format!(r"(?i)\$env:userprofile\s*[\\/]\s*{}", segment),
        format!(r"(?i)%userprofile%\s*[\\/]\s*{}", segment),
        format!(r"(?i)\$home\s*[\\/]\s*{}", segment),
        format!(r"(?i)~\s*[\\/]\s*{}", segment),
        format!(
            r#"(?i)\[environment\]::getfolderpath\(\s*['"]{}\s*['"]\s*\)"#,
            segment
        ),
    ];

    let mut rewritten = command.to_string();
    let mut changed = false;

    for pattern in patterns {
        let Ok(regex) = Regex::new(&pattern) else {
            continue;
        };
        let replaced = regex
            .replace_all(&rewritten, quoted_path.as_str())
            .to_string();
        if replaced != rewritten {
            rewritten = replaced;
            changed = true;
        }
    }

    changed.then_some(rewritten)
}

fn user_explicitly_requested_path_revalidation(session: &Session) -> bool {
    let Some(conversation) = &session.conversation else {
        return false;
    };
    let Some(last_user_message) = conversation
        .messages()
        .iter()
        .rev()
        .find(|msg| msg.role == Role::User && msg.is_user_visible())
    else {
        return false;
    };

    let lowered = last_user_message.as_concat_text().to_ascii_lowercase();
    let markers = [
        "verify again",
        "re-verify",
        "recheck",
        "check again",
        "重新验证",
        "重新确认",
        "重新查找",
        "再次查找",
        "再确认",
    ];
    markers.iter().any(|marker| lowered.contains(marker))
}

fn build_runtime_memory_context_text(
    facts: &[crate::session::session_manager::MemoryFact],
) -> Option<String> {
    let mut goals = Vec::new();
    let mut verified_actions = Vec::new();
    let mut artifacts = Vec::new();
    let mut invalid_paths = Vec::new();
    let mut open_items = Vec::new();
    let mut notes = Vec::new();

    let mut goals_seen = HashSet::new();
    let mut verified_seen = HashSet::new();
    let mut artifacts_seen = HashSet::new();
    let mut invalid_seen = HashSet::new();
    let mut open_seen = HashSet::new();
    let mut notes_seen = HashSet::new();

    for fact in facts {
        if fact.status != MemoryFactStatus::Active && !fact.pinned {
            continue;
        }

        let content = fact.content.trim();
        if content.is_empty() {
            continue;
        }

        let category = fact.category.to_ascii_lowercase();
        if category == "invalid_path" || category.contains("invalid_path") {
            if !looks_like_runtime_artifact(content) {
                continue;
            }
            push_runtime_memory_item(&mut invalid_paths, &mut invalid_seen, content);
        } else if category.contains("artifact") || category.contains("path") {
            if !looks_like_runtime_artifact(content) {
                continue;
            }
            push_runtime_memory_item(&mut artifacts, &mut artifacts_seen, content);
        } else if category.contains("verified") || category.contains("action") {
            push_runtime_memory_item(&mut verified_actions, &mut verified_seen, content);
        } else if category.contains("goal") {
            push_runtime_memory_item(&mut goals, &mut goals_seen, content);
        } else if category.contains("open") || category.contains("todo") {
            push_runtime_memory_item(&mut open_items, &mut open_seen, content);
        } else {
            push_runtime_memory_item(&mut notes, &mut notes_seen, content);
        }
    }

    if goals.is_empty()
        && verified_actions.is_empty()
        && artifacts.is_empty()
        && invalid_paths.is_empty()
        && open_items.is_empty()
        && notes.is_empty()
    {
        return None;
    }

    let mut lines = vec![
        "[CFPM_RUNTIME_MEMORY_V1] Use these stable facts before planning tools/commands.".to_string(),
        "Do not re-discover known paths or methods unless the user explicitly asks to verify again.".to_string(),
        "For Desktop/Documents/Downloads style requests, prefer matching known paths directly.".to_string(),
    ];

    if !artifacts.is_empty() {
        lines.push(String::new());
        lines.push("Known artifacts/paths (prefer direct use):".to_string());
        for item in artifacts {
            lines.push(format!("- {}", item));
        }
    }

    if !invalid_paths.is_empty() {
        lines.push(String::new());
        lines.push("Known invalid paths (avoid reuse unless user asks to re-verify):".to_string());
        for item in invalid_paths {
            lines.push(format!("- {}", item));
        }
    }

    if !verified_actions.is_empty() {
        lines.push(String::new());
        lines.push("Verified reusable actions:".to_string());
        for item in verified_actions {
            lines.push(format!("- {}", item));
        }
    }

    if !goals.is_empty() {
        lines.push(String::new());
        lines.push("Persistent user goals:".to_string());
        for item in goals {
            lines.push(format!("- {}", item));
        }
    }

    if !open_items.is_empty() {
        lines.push(String::new());
        lines.push("Open items:".to_string());
        for item in open_items {
            lines.push(format!("- {}", item));
        }
    }

    if !notes.is_empty() {
        lines.push(String::new());
        lines.push("Other stable notes:".to_string());
        for item in notes {
            lines.push(format!("- {}", item));
        }
    }

    Some(lines.join("\n"))
}

fn max_compaction_attempts_for(strategy: ContextCompactionStrategy) -> u32 {
    match strategy {
        ContextCompactionStrategy::LegacySegmented => MAX_COMPACTION_ATTEMPTS,
        ContextCompactionStrategy::CfpmMemoryV1 => MAX_COMPACTION_ATTEMPTS_CFPM,
    }
}

fn compaction_strategy_label(strategy: ContextCompactionStrategy) -> &'static str {
    match strategy {
        ContextCompactionStrategy::LegacySegmented => "legacy_segmented",
        ContextCompactionStrategy::CfpmMemoryV1 => "cfpm_memory_v1",
    }
}

/// Context needed for the reply function
pub struct ReplyContext {
    pub conversation: Conversation,
    pub tools: Vec<Tool>,
    pub toolshim_tools: Vec<Tool>,
    pub system_prompt: String,
    pub agime_mode: AgimeMode,
    pub initial_messages: Vec<Message>,
}

pub struct ToolCategorizeResult {
    pub frontend_requests: Vec<ToolRequest>,
    pub remaining_requests: Vec<ToolRequest>,
    pub filtered_response: Message,
}

/// The main goose Agent
pub struct Agent {
    pub(super) provider: SharedProvider,

    pub extension_manager: Arc<ExtensionManager>,
    pub(super) sub_recipes: Mutex<HashMap<String, SubRecipe>>,
    pub(super) final_output_tool: Arc<Mutex<Option<FinalOutputTool>>>,
    pub(super) frontend_tools: Mutex<HashMap<String, FrontendTool>>,
    pub(super) frontend_instructions: Mutex<Option<String>>,
    pub(super) prompt_manager: Mutex<PromptManager>,
    pub(super) confirmation_tx: mpsc::Sender<(String, PermissionConfirmation)>,
    pub(super) confirmation_rx: Mutex<mpsc::Receiver<(String, PermissionConfirmation)>>,
    pub(super) tool_result_tx: mpsc::Sender<(String, ToolResult<CallToolResult>)>,
    pub(super) tool_result_rx: ToolResultReceiver,

    pub tool_route_manager: Arc<ToolRouteManager>,
    pub(super) scheduler_service: Mutex<Option<Arc<dyn SchedulerTrait>>>,
    pub(super) retry_manager: RetryManager,
    pub(super) tool_inspection_manager: ToolInspectionManager,
    cfpm_tool_gate_events: Arc<Mutex<HashMap<String, CfpmToolGateEvent>>>,
}

#[derive(Clone, Debug)]
pub enum AgentEvent {
    Message(Message),
    McpNotification((String, ServerNotification)),
    ModelChange { model: String, mode: String },
    HistoryReplaced(Conversation),
}

impl Default for Agent {
    fn default() -> Self {
        Self::new()
    }
}

pub enum ToolStreamItem<T> {
    Message(ServerNotification),
    Result(T),
}

pub type ToolStream =
    Pin<Box<dyn Stream<Item = ToolStreamItem<ToolResult<CallToolResult>>> + Send>>;

// tool_stream combines a stream of ServerNotifications with a future representing the
// final result of the tool call. MCP notifications are not request-scoped, but
// this lets us capture all notifications emitted during the tool call for
// simpler consumption
pub fn tool_stream<S, F>(rx: S, done: F) -> ToolStream
where
    S: Stream<Item = ServerNotification> + Send + Unpin + 'static,
    F: Future<Output = ToolResult<CallToolResult>> + Send + 'static,
{
    Box::pin(async_stream::stream! {
        tokio::pin!(done);
        let mut rx = rx;

        loop {
            tokio::select! {
                Some(msg) = rx.next() => {
                    yield ToolStreamItem::Message(msg);
                }
                r = &mut done => {
                    yield ToolStreamItem::Result(r);
                    break;
                }
            }
        }
    })
}

impl Agent {
    pub fn new() -> Self {
        // Create channels with buffer size 32 (adjust if needed)
        let (confirm_tx, confirm_rx) = mpsc::channel(32);
        let (tool_tx, tool_rx) = mpsc::channel(32);
        let provider = Arc::new(Mutex::new(None));

        Self {
            provider: provider.clone(),
            extension_manager: Arc::new(ExtensionManager::new(provider.clone())),
            sub_recipes: Mutex::new(HashMap::new()),
            final_output_tool: Arc::new(Mutex::new(None)),
            frontend_tools: Mutex::new(HashMap::new()),
            frontend_instructions: Mutex::new(None),
            prompt_manager: Mutex::new(PromptManager::new()),
            confirmation_tx: confirm_tx,
            confirmation_rx: Mutex::new(confirm_rx),
            tool_result_tx: tool_tx,
            tool_result_rx: Arc::new(Mutex::new(tool_rx)),
            tool_route_manager: Arc::new(ToolRouteManager::new()),
            scheduler_service: Mutex::new(None),
            retry_manager: RetryManager::new(),
            tool_inspection_manager: Self::create_default_tool_inspection_manager(),
            cfpm_tool_gate_events: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a tool inspection manager with default inspectors
    fn create_default_tool_inspection_manager() -> ToolInspectionManager {
        let mut tool_inspection_manager = ToolInspectionManager::new();

        // Add security inspector (highest priority - runs first)
        tool_inspection_manager.add_inspector(Box::new(SecurityInspector::new()));

        // Add permission inspector (medium-high priority)
        // Note: mode will be updated dynamically based on session config
        tool_inspection_manager.add_inspector(Box::new(PermissionInspector::new(
            AgimeMode::SmartApprove,
            std::collections::HashSet::new(), // readonly tools - will be populated from extension manager
            std::collections::HashSet::new(), // regular tools - will be populated from extension manager
        )));

        // Add repetition inspector (lower priority - basic repetition checking)
        tool_inspection_manager.add_inspector(Box::new(RepetitionInspector::new(None)));

        tool_inspection_manager
    }

    /// Reset the retry attempts counter to 0
    pub async fn reset_retry_attempts(&self) {
        self.retry_manager.reset_attempts().await;
    }

    /// Increment the retry attempts counter and return the new value
    pub async fn increment_retry_attempts(&self) -> u32 {
        self.retry_manager.increment_attempts().await
    }

    /// Get the current retry attempts count
    pub async fn get_retry_attempts(&self) -> u32 {
        self.retry_manager.get_attempts().await
    }

    async fn handle_retry_logic(
        &self,
        messages: &mut Conversation,
        session_config: &SessionConfig,
        initial_messages: &[Message],
    ) -> Result<bool> {
        let result = self
            .retry_manager
            .handle_retry_logic(
                messages,
                session_config,
                initial_messages,
                &self.final_output_tool,
            )
            .await?;

        match result {
            RetryResult::Retried => Ok(true),
            RetryResult::Skipped
            | RetryResult::MaxAttemptsReached
            | RetryResult::SuccessChecksPassed => Ok(false),
        }
    }
    async fn drain_elicitation_messages(session_id: &str) -> Vec<Message> {
        let mut messages = Vec::new();
        let mut elicitation_rx = ActionRequiredManager::global().request_rx.lock().await;
        while let Ok(elicitation_message) = elicitation_rx.try_recv() {
            if let Err(e) = SessionManager::add_message(session_id, &elicitation_message).await {
                warn!("Failed to save elicitation message to session: {}", e);
            }
            messages.push(elicitation_message);
        }
        messages
    }

    async fn prepare_reply_context(
        &self,
        unfixed_conversation: Conversation,
        working_dir: &std::path::Path,
    ) -> Result<ReplyContext> {
        let unfixed_messages = unfixed_conversation.messages().clone();
        let (conversation, issues) = fix_conversation(unfixed_conversation.clone());
        if !issues.is_empty() {
            debug!(
                "Conversation issue fixed: {}",
                debug_conversation_fix(
                    unfixed_messages.as_slice(),
                    conversation.messages(),
                    &issues
                )
            );
        }
        let initial_messages = conversation.messages().clone();
        let config = Config::global();

        let (tools, toolshim_tools, system_prompt) =
            self.prepare_tools_and_prompt(working_dir).await?;
        let agime_mode = config.get_agime_mode().unwrap_or(AgimeMode::Auto);

        self.tool_inspection_manager
            .update_permission_inspector_mode(agime_mode)
            .await;

        Ok(ReplyContext {
            conversation,
            tools,
            toolshim_tools,
            system_prompt,
            agime_mode,
            initial_messages,
        })
    }

    async fn inject_runtime_memory_context(
        &self,
        session_id: &str,
        conversation: &Conversation,
    ) -> Conversation {
        if !matches!(
            current_compaction_strategy(),
            ContextCompactionStrategy::CfpmMemoryV1
        ) {
            return conversation.clone();
        }

        let facts = match SessionManager::list_memory_facts(session_id).await {
            Ok(facts) => facts,
            Err(err) => {
                warn!(
                    "Failed to read CFPM memory facts for runtime context: {}",
                    err
                );
                return conversation.clone();
            }
        };

        let Some(memory_text) = build_runtime_memory_context_text(&facts) else {
            return conversation.clone();
        };

        debug!(
            "Injecting CFPM runtime memory for session {} (facts fetched: {})",
            session_id,
            facts.len()
        );

        let mut messages = conversation.messages().clone();
        messages.push(
            Message::user()
                .with_text(memory_text)
                .with_metadata(MessageMetadata::agent_only()),
        );

        let (fixed, issues) = fix_conversation(Conversation::new_unvalidated(messages));
        let has_unexpected_issues = issues.iter().any(|issue| {
            !issue.contains("Merged consecutive user messages")
                && !issue.contains("Merged consecutive assistant messages")
        });

        if has_unexpected_issues {
            warn!(
                "Runtime CFPM memory injection caused unexpected conversation issues: {:?}",
                issues
            );
            return conversation.clone();
        }

        fixed
    }

    async fn categorize_tools(
        &self,
        response: &Message,
        _tools: &[rmcp::model::Tool],
    ) -> ToolCategorizeResult {
        // Categorize tool requests
        let (frontend_requests, remaining_requests, filtered_response) =
            self.categorize_tool_requests(response).await;

        ToolCategorizeResult {
            frontend_requests,
            remaining_requests,
            filtered_response,
        }
    }

    async fn handle_approved_and_denied_tools(
        &self,
        permission_check_result: &PermissionCheckResult,
        request_to_response_map: &HashMap<String, Arc<Mutex<Message>>>,
        cancel_token: Option<tokio_util::sync::CancellationToken>,
        session: &Session,
    ) -> Result<Vec<(String, ToolStream)>> {
        let mut tool_futures: Vec<(String, ToolStream)> = Vec::new();

        // Handle pre-approved and read-only tools
        for request in &permission_check_result.approved {
            if let Ok(tool_call) = request.tool_call.clone() {
                let (req_id, tool_result) = self
                    .dispatch_tool_call(
                        tool_call,
                        request.id.clone(),
                        cancel_token.clone(),
                        session,
                    )
                    .await;

                tool_futures.push((
                    req_id,
                    match tool_result {
                        Ok(result) => tool_stream(
                            result
                                .notification_stream
                                .unwrap_or_else(|| Box::new(stream::empty())),
                            result.result,
                        ),
                        Err(e) => {
                            tool_stream(Box::new(stream::empty()), futures::future::ready(Err(e)))
                        }
                    },
                ));
            }
        }

        Self::handle_denied_tools(permission_check_result, request_to_response_map).await;
        Ok(tool_futures)
    }

    async fn handle_denied_tools(
        permission_check_result: &PermissionCheckResult,
        request_to_response_map: &HashMap<String, Arc<Mutex<Message>>>,
    ) {
        for request in &permission_check_result.denied {
            if let Some(response_msg) = request_to_response_map.get(&request.id) {
                let mut response = response_msg.lock().await;
                *response = response.clone().with_tool_response(
                    request.id.clone(),
                    Ok(CallToolResult {
                        content: vec![rmcp::model::Content::text(DECLINED_RESPONSE)],
                        structured_content: None,
                        is_error: Some(true),
                        meta: None,
                    }),
                );
            }
        }
    }

    pub async fn set_scheduler(&self, scheduler: Arc<dyn SchedulerTrait>) {
        let mut scheduler_service = self.scheduler_service.lock().await;
        *scheduler_service = Some(scheduler);
    }

    pub async fn disable_router_for_recipe(&self) {
        self.tool_route_manager.disable_router_for_recipe().await;
    }

    /// Get a reference count clone to the provider
    pub async fn provider(&self) -> Result<Arc<dyn Provider>, anyhow::Error> {
        match &*self.provider.lock().await {
            Some(provider) => Ok(Arc::clone(provider)),
            None => Err(anyhow!("Provider not set")),
        }
    }

    /// Check if a tool is a frontend tool
    pub async fn is_frontend_tool(&self, name: &str) -> bool {
        self.frontend_tools.lock().await.contains_key(name)
    }

    /// Get a reference to a frontend tool
    pub async fn get_frontend_tool(&self, name: &str) -> Option<FrontendTool> {
        self.frontend_tools.lock().await.get(name).cloned()
    }

    pub async fn add_final_output_tool(&self, response: Response) {
        let mut final_output_tool = self.final_output_tool.lock().await;
        let created_final_output_tool = FinalOutputTool::new(response);
        let final_output_system_prompt = created_final_output_tool.system_prompt();
        *final_output_tool = Some(created_final_output_tool);
        self.extend_system_prompt(final_output_system_prompt).await;
    }

    async fn store_cfpm_tool_gate_event(&self, request_id: &str, event: CfpmToolGateEvent) {
        let mut events = self.cfpm_tool_gate_events.lock().await;
        events.insert(request_id.to_string(), event);
    }

    async fn take_cfpm_tool_gate_inline_message(&self, request_id: &str) -> Option<String> {
        let event = {
            let mut events = self.cfpm_tool_gate_events.lock().await;
            events.remove(request_id)
        }?;

        let visibility = current_cfpm_tool_gate_visibility();
        if visibility == CfpmRuntimeVisibility::Off {
            return None;
        }

        Some(build_cfpm_tool_gate_inline_message(&event, visibility))
    }

    async fn rewrite_tool_call_with_cfpm_memory(
        &self,
        request_id: &str,
        session: &Session,
        mut tool_call: CallToolRequestParam,
    ) -> CallToolRequestParam {
        if !matches!(
            current_compaction_strategy(),
            ContextCompactionStrategy::CfpmMemoryV1
        ) || !cfpm_pre_tool_gate_enabled()
            || user_explicitly_requested_path_revalidation(session)
            || !is_shell_like_tool(&tool_call.name)
        {
            return tool_call;
        }

        let Some(args) = tool_call.arguments.as_mut() else {
            return tool_call;
        };

        let Some((command_key, command)) = ["command", "cmd", "script"].iter().find_map(|key| {
            args.get(*key)
                .and_then(|value| value.as_str())
                .map(|value| ((*key).to_string(), value.to_string()))
        }) else {
            return tool_call;
        };

        if !supports_known_folder_probe_rewrite(&command)
            || is_explicit_absolute_path_reference(&command)
        {
            return tool_call;
        }

        let targets = target_mentions_in_command(&command);
        if targets.is_empty() {
            return tool_call;
        }

        let facts = match SessionManager::list_memory_facts(&session.id).await {
            Ok(facts) => facts,
            Err(err) => {
                warn!(
                    "Failed to load CFPM memory facts for pre-tool gate rewrite: {}",
                    err
                );
                return tool_call;
            }
        };
        let recent_invalid_paths = collect_recent_invalid_paths_from_session(session);

        let mut rewritten_command = None;
        for target in targets {
            let Some(path) =
                find_known_folder_path_from_facts(&facts, target, Some(&recent_invalid_paths))
            else {
                continue;
            };
            let Some(candidate) = rewrite_known_folder_probe_in_command(&command, target, &path)
            else {
                continue;
            };

            info!(
                session_id = %session.id,
                tool = %tool_call.name,
                target = target.segment(),
                known_path = %path,
                "Applied CFPM pre-tool rewrite for known folder probe"
            );
            self.store_cfpm_tool_gate_event(
                request_id,
                CfpmToolGateEvent {
                    tool: tool_call.name.to_string(),
                    target: target.segment().to_string(),
                    path: path.clone(),
                    original_command: truncate_chars(command.trim(), 320),
                    rewritten_command: truncate_chars(candidate.trim(), 320),
                },
            )
            .await;
            rewritten_command = Some(candidate);
            break;
        }

        if let Some(rewritten) = rewritten_command {
            args.insert(command_key, Value::String(rewritten));
        }

        tool_call
    }

    pub async fn add_sub_recipes(&self, sub_recipes_to_add: Vec<SubRecipe>) {
        let mut sub_recipes = self.sub_recipes.lock().await;
        for sr in sub_recipes_to_add {
            sub_recipes.insert(sr.name.clone(), sr);
        }
    }

    pub async fn apply_recipe_components(
        &self,
        sub_recipes: Option<Vec<SubRecipe>>,
        response: Option<Response>,
        include_final_output: bool,
    ) {
        if let Some(sub_recipes) = sub_recipes {
            self.add_sub_recipes(sub_recipes).await;
        }

        if include_final_output {
            if let Some(response) = response {
                self.add_final_output_tool(response).await;
            }
        }
    }

    /// Dispatch a single tool call to the appropriate client
    #[instrument(skip(self, tool_call, request_id), fields(input, output))]
    pub async fn dispatch_tool_call(
        &self,
        tool_call: CallToolRequestParam,
        request_id: String,
        cancellation_token: Option<CancellationToken>,
        session: &Session,
    ) -> (String, Result<ToolCallResult, ErrorData>) {
        let tool_call = self
            .rewrite_tool_call_with_cfpm_memory(&request_id, session, tool_call)
            .await;

        // Prevent subagents from creating other subagents
        if session.session_type == crate::session::SessionType::SubAgent
            && tool_call.name == SUBAGENT_TOOL_NAME
        {
            return (
                request_id,
                Err(ErrorData::new(
                    ErrorCode::INVALID_REQUEST,
                    "Subagents cannot create other subagents".to_string(),
                    None,
                )),
            );
        }

        if tool_call.name == PLATFORM_MANAGE_SCHEDULE_TOOL_NAME {
            let arguments = tool_call
                .arguments
                .map(Value::Object)
                .unwrap_or(Value::Object(serde_json::Map::new()));
            let result = self
                .handle_schedule_management(arguments, request_id.clone())
                .await;
            let wrapped_result = result.map(|content| CallToolResult {
                content,
                structured_content: None,
                is_error: Some(false),
                meta: None,
            });
            return (request_id, Ok(ToolCallResult::from(wrapped_result)));
        }

        if tool_call.name == FINAL_OUTPUT_TOOL_NAME {
            return if let Some(final_output_tool) = self.final_output_tool.lock().await.as_mut() {
                let result = final_output_tool.execute_tool_call(tool_call.clone()).await;
                (request_id, Ok(result))
            } else {
                (
                    request_id,
                    Err(ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        "Final output tool not defined".to_string(),
                        None,
                    )),
                )
            };
        }

        debug!("WAITING_TOOL_START: {}", tool_call.name);
        let result: ToolCallResult = if tool_call.name == SUBAGENT_TOOL_NAME {
            let provider = match self.provider().await {
                Ok(p) => p,
                Err(_) => {
                    return (
                        request_id,
                        Err(ErrorData::new(
                            ErrorCode::INTERNAL_ERROR,
                            "Provider is required".to_string(),
                            None,
                        )),
                    );
                }
            };

            let extensions = self.get_extension_configs().await;
            let task_config =
                TaskConfig::new(provider, &session.id, &session.working_dir, extensions);
            let sub_recipes = self.sub_recipes.lock().await.clone();

            let arguments = tool_call
                .arguments
                .clone()
                .map(Value::Object)
                .unwrap_or(Value::Object(serde_json::Map::new()));

            handle_subagent_tool(
                arguments,
                task_config,
                sub_recipes,
                session.working_dir.clone(),
                cancellation_token,
            )
        } else if self.is_frontend_tool(&tool_call.name).await {
            // For frontend tools, return an error indicating we need frontend execution
            ToolCallResult::from(Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "Frontend tool execution required".to_string(),
                None,
            )))
        } else if tool_call.name == ROUTER_LLM_SEARCH_TOOL_NAME {
            match self
                .tool_route_manager
                .dispatch_route_search_tool(tool_call.arguments.unwrap_or_default())
                .await
            {
                Ok(tool_result) => tool_result,
                Err(e) => return (request_id, Err(e)),
            }
        } else {
            // Clone the result to ensure no references to extension_manager are returned
            let result = self
                .extension_manager
                .dispatch_tool_call(tool_call.clone(), cancellation_token.unwrap_or_default())
                .await;
            result.unwrap_or_else(|e| {
                ToolCallResult::from(Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    e.to_string(),
                    None,
                )))
            })
        };

        debug!("WAITING_TOOL_END: {}", tool_call.name);

        (
            request_id,
            Ok(ToolCallResult {
                notification_stream: result.notification_stream,
                result: Box::new(
                    result
                        .result
                        .map(super::large_response_handler::process_tool_response),
                ),
            }),
        )
    }

    /// Save current extension state to session metadata
    /// Should be called after any extension add/remove operation
    pub async fn save_extension_state(&self, session: &SessionConfig) -> Result<()> {
        let extension_configs = self.extension_manager.get_extension_configs().await;

        let extensions_state = EnabledExtensionsState::new(extension_configs);

        let mut session_data = SessionManager::get_session(&session.id, false).await?;

        if let Err(e) = extensions_state.to_extension_data(&mut session_data.extension_data) {
            warn!("Failed to serialize extension state: {}", e);
            return Err(anyhow!("Extension state serialization failed: {}", e));
        }

        SessionManager::update_session(&session.id)
            .extension_data(session_data.extension_data)
            .apply()
            .await?;

        Ok(())
    }

    pub async fn add_extension(&self, extension: ExtensionConfig) -> ExtensionResult<()> {
        match &extension {
            ExtensionConfig::Frontend {
                tools,
                instructions,
                ..
            } => {
                // For frontend tools, just store them in the frontend_tools map
                let mut frontend_tools = self.frontend_tools.lock().await;
                for tool in tools {
                    let frontend_tool = FrontendTool {
                        name: tool.name.to_string(),
                        tool: tool.clone(),
                    };
                    frontend_tools.insert(tool.name.to_string(), frontend_tool);
                }
                // Store instructions if provided, using "frontend" as the key
                let mut frontend_instructions = self.frontend_instructions.lock().await;
                if let Some(instructions) = instructions {
                    *frontend_instructions = Some(instructions.clone());
                } else {
                    // Default frontend instructions if none provided
                    *frontend_instructions = Some(
                        "The following tools are provided directly by the frontend and will be executed by the frontend when called.".to_string(),
                    );
                }
            }
            _ => {
                self.extension_manager
                    .add_extension(extension.clone())
                    .await?;
            }
        }

        // If LLM tool selection is functional, index the tools
        if self.tool_route_manager.is_router_functional().await {
            let selector = self.tool_route_manager.get_router_tool_selector().await;
            if let Some(selector) = selector {
                let selector = Arc::new(selector);
                if let Err(e) = ToolRouterIndexManager::update_extension_tools(
                    &selector,
                    &self.extension_manager,
                    &extension.name(),
                    "add",
                )
                .await
                {
                    return Err(ExtensionError::SetupError(format!(
                        "Failed to index tools for extension {}: {}",
                        extension.name(),
                        e
                    )));
                }
            }
        }

        Ok(())
    }

    pub async fn list_tools(&self, extension_name: Option<String>) -> Vec<Tool> {
        let mut prefixed_tools = self
            .extension_manager
            .get_prefixed_tools(extension_name.clone())
            .await
            .unwrap_or_default();

        if extension_name.is_none() || extension_name.as_deref() == Some("platform") {
            prefixed_tools.extend([platform_tools::manage_schedule_tool()]);
        }

        if extension_name.is_none() {
            if let Some(final_output_tool) = self.final_output_tool.lock().await.as_ref() {
                prefixed_tools.push(final_output_tool.tool());
            }

            // Add the unified subagent tool
            let sub_recipes = self.sub_recipes.lock().await;
            let sub_recipes_vec: Vec<_> = sub_recipes.values().cloned().collect();
            prefixed_tools.push(create_subagent_tool(&sub_recipes_vec));
        }

        prefixed_tools
    }

    pub async fn list_tools_for_router(&self) -> Vec<Tool> {
        self.tool_route_manager
            .list_tools_for_router(&self.extension_manager)
            .await
    }

    pub async fn remove_extension(&self, name: &str) -> Result<()> {
        self.extension_manager.remove_extension(name).await?;

        // If LLM tool selection is functional, remove tools from the index
        if self.tool_route_manager.is_router_functional().await {
            let selector = self.tool_route_manager.get_router_tool_selector().await;
            if let Some(selector) = selector {
                ToolRouterIndexManager::update_extension_tools(
                    &selector,
                    &self.extension_manager,
                    name,
                    "remove",
                )
                .await?;
            }
        }

        Ok(())
    }

    pub async fn list_extensions(&self) -> Vec<String> {
        self.extension_manager
            .list_extensions()
            .await
            .expect("Failed to list extensions")
    }

    pub async fn get_extension_configs(&self) -> Vec<ExtensionConfig> {
        self.extension_manager.get_extension_configs().await
    }

    /// Handle a confirmation response for a tool request
    pub async fn handle_confirmation(
        &self,
        request_id: String,
        confirmation: PermissionConfirmation,
    ) {
        if let Err(e) = self.confirmation_tx.send((request_id, confirmation)).await {
            error!("Failed to send confirmation: {}", e);
        }
    }

    #[instrument(skip(self, user_message, session_config), fields(user_message))]
    pub async fn reply(
        &self,
        user_message: Message,
        session_config: SessionConfig,
        cancel_token: Option<CancellationToken>,
    ) -> Result<BoxStream<'_, Result<AgentEvent>>> {
        // Performance monitoring
        let reply_start = std::time::Instant::now();
        tracing::info!("[PERF] reply() started");

        for content in &user_message.content {
            if let MessageContent::ActionRequired(action_required) = content {
                if let ActionRequiredData::ElicitationResponse { id, user_data } =
                    &action_required.data
                {
                    if let Err(e) = ActionRequiredManager::global()
                        .submit_response(id.clone(), user_data.clone())
                        .await
                    {
                        let error_text = format!("Failed to submit elicitation response: {}", e);
                        error!(error_text);
                        return Ok(Box::pin(stream::once(async {
                            Ok(AgentEvent::Message(
                                Message::assistant().with_text(error_text),
                            ))
                        })));
                    }
                    SessionManager::add_message(&session_config.id, &user_message).await?;
                    return Ok(Box::pin(futures::stream::empty()));
                }
            }
        }

        let message_text = user_message.as_concat_text();
        let is_manual_compact = MANUAL_COMPACT_TRIGGERS.contains(&message_text.trim());

        let slash_command_recipe = if message_text.trim().starts_with('/') {
            let command = message_text.split_whitespace().next();
            command.and_then(crate::slash_commands::resolve_slash_command)
        } else {
            None
        };

        if let Some(recipe) = slash_command_recipe {
            let prompt = [recipe.instructions.as_deref(), recipe.prompt.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n\n");
            let prompt_message = Message::user()
                .with_text(prompt)
                .with_visibility(false, true);
            SessionManager::add_message(&session_config.id, &prompt_message).await?;
            SessionManager::add_message(
                &session_config.id,
                &user_message.with_visibility(true, false),
            )
            .await?;
        } else {
            SessionManager::add_message(&session_config.id, &user_message).await?;
        }
        let session = SessionManager::get_session(&session_config.id, true).await?;
        let conversation = session
            .conversation
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Session {} has no conversation", session_config.id))?;

        tracing::info!(
            "[PERF] check_compaction start, elapsed: {:?}",
            reply_start.elapsed()
        );
        let needs_auto_compact = !is_manual_compact
            && check_if_compaction_needed(
                self.provider().await?.as_ref(),
                &conversation,
                None,
                &session,
            )
            .await?;
        tracing::info!(
            "[PERF] check_compaction done, elapsed: {:?}",
            reply_start.elapsed()
        );

        let conversation_to_compact = conversation.clone();
        let active_compaction_strategy = current_compaction_strategy();

        Ok(Box::pin(async_stream::try_stream! {
            let final_conversation = if !needs_auto_compact && !is_manual_compact {
                conversation
            } else {
                if !is_manual_compact {
                    let config = crate::config::Config::global();
                    let threshold = config
                        .get_param::<f64>("AGIME_AUTO_COMPACT_THRESHOLD")
                        .unwrap_or(DEFAULT_COMPACTION_THRESHOLD);
                    let threshold_percentage = (threshold * 100.0) as u32;

                    let inline_msg = format!(
                        "Exceeded auto-compact threshold of {}%. Performing auto-compaction...",
                        threshold_percentage
                    );

                    yield AgentEvent::Message(
                        Message::assistant().with_system_notification(
                            SystemNotificationType::InlineMessage,
                            inline_msg,
                        )
                    );
                }

                yield AgentEvent::Message(
                    Message::assistant().with_system_notification(
                        SystemNotificationType::ThinkingMessage,
                        COMPACTION_THINKING_TEXT,
                    )
                );

                match compact_messages_with_active_strategy(
                    self.provider().await?.as_ref(),
                    &conversation_to_compact,
                    is_manual_compact,
                ).await {
                    Ok((compacted_conversation, summarization_usage)) => {
                        SessionManager::replace_conversation(&session_config.id, &compacted_conversation).await?;
                        if matches!(active_compaction_strategy, ContextCompactionStrategy::CfpmMemoryV1) {
                            let reason = if is_manual_compact {
                                "manual_compaction"
                            } else if needs_auto_compact {
                                "auto_compaction"
                            } else {
                                "compaction"
                            };
                            if let Err(err) = SessionManager::replace_cfpm_memory_facts_from_conversation(
                                &session_config.id,
                                &compacted_conversation,
                                reason,
                            )
                            .await
                            {
                                warn!("Failed to refresh CFPM memory facts: {}", err);
                            }
                        }
                        Self::update_session_metrics(&session_config, &summarization_usage, true).await?;

                        yield AgentEvent::HistoryReplaced(compacted_conversation.clone());

                        yield AgentEvent::Message(
                            Message::assistant().with_system_notification(
                                SystemNotificationType::InlineMessage,
                                format!(
                                    "Compaction complete (strategy: {})",
                                    compaction_strategy_label(active_compaction_strategy)
                                ),
                            )
                        );

                        compacted_conversation
                    }
                    Err(e) => {
                        yield AgentEvent::Message(
                            Message::assistant().with_text(
                                format!("Ran into this error trying to compact: {e}.\n\nPlease try again or create a new session")
                            )
                        );
                        return;
                    }
                }
            };

            if !is_manual_compact {
                let mut reply_stream = self.reply_internal(final_conversation, session_config, session, cancel_token).await?;
                while let Some(event) = reply_stream.next().await {
                    yield event?;
                }
            }
        }))
    }

    async fn reply_internal(
        &self,
        conversation: Conversation,
        session_config: SessionConfig,
        session: Session,
        cancel_token: Option<CancellationToken>,
    ) -> Result<BoxStream<'_, Result<AgentEvent>>> {
        // Performance monitoring
        let internal_start = std::time::Instant::now();
        tracing::info!("[PERF] reply_internal() started");

        let context = self
            .prepare_reply_context(conversation, &session.working_dir)
            .await?;
        tracing::info!(
            "[PERF] prepare_reply_context done, elapsed: {:?}",
            internal_start.elapsed()
        );
        let ReplyContext {
            mut conversation,
            mut tools,
            mut toolshim_tools,
            mut system_prompt,
            agime_mode,
            initial_messages,
        } = context;
        let reply_span = tracing::Span::current();
        self.reset_retry_attempts().await;

        let provider = self.provider().await?;
        let session_id = session_config.id.clone();
        let working_dir = session.working_dir.clone();
        tokio::spawn(async move {
            if let Err(e) = SessionManager::maybe_update_name(&session_id, provider).await {
                warn!("Failed to generate session description: {}", e);
            }
        });

        Ok(Box::pin(async_stream::try_stream! {
            let _ = reply_span.enter();
            let mut turns_taken = 0u32;
            let mut compaction_count = 0u32; // Track compaction attempts to prevent infinite loops
            let active_compaction_strategy = current_compaction_strategy();
            let max_compaction_attempts =
                max_compaction_attempts_for(active_compaction_strategy);
            let max_turns = session_config.max_turns.unwrap_or(DEFAULT_MAX_TURNS);

            loop {
                if is_token_cancelled(&cancel_token) {
                    break;
                }

                if let Some(final_output_tool) = self.final_output_tool.lock().await.as_ref() {
                    if final_output_tool.final_output.is_some() {
                        let final_event = AgentEvent::Message(
                            Message::assistant().with_text(final_output_tool.final_output.clone().unwrap())
                        );
                        yield final_event;
                        break;
                    }
                }

                turns_taken += 1;
                if turns_taken > max_turns {
                    yield AgentEvent::Message(
                        Message::assistant().with_text(
                            "I've reached the maximum number of actions I can do without user input. Would you like me to continue?"
                        )
                    );
                    break;
                }

                let conversation_with_memory = self
                    .inject_runtime_memory_context(&session_config.id, &conversation)
                    .await;

                let conversation_with_moim = super::moim::inject_moim(
                    conversation_with_memory,
                    &self.extension_manager,
                ).await;

                let mut stream = Self::stream_response_from_provider(
                    self.provider().await?,
                    &system_prompt,
                    conversation_with_moim.messages(),
                    &tools,
                    &toolshim_tools,
                ).await?;

                let mut no_tools_called = true;
                let mut messages_to_add = Conversation::default();
                let mut tools_updated = false;
                let mut did_recovery_compact_this_iteration = false;

                while let Some(next) = stream.next().await {
                    if is_token_cancelled(&cancel_token) {
                        break;
                    }

                    match next {
                        Ok((response, usage)) => {
                            // Emit model change event if provider is lead-worker
                            let provider = self.provider().await?;
                            if let Some(lead_worker) = provider.as_lead_worker() {
                                if let Some(ref usage) = usage {
                                    let active_model = usage.model.clone();
                                    let (lead_model, worker_model) = lead_worker.get_model_info();
                                    let mode = if active_model == lead_model {
                                        "lead"
                                    } else if active_model == worker_model {
                                        "worker"
                                    } else {
                                        "unknown"
                                    };

                                    yield AgentEvent::ModelChange {
                                        model: active_model,
                                        mode: mode.to_string(),
                                    };
                                }
                            }

                            if let Some(ref usage) = usage {
                                Self::update_session_metrics(&session_config, usage, false).await?;
                            }

                            if let Some(response) = response {
                                let ToolCategorizeResult {
                                    frontend_requests,
                                    remaining_requests,
                                    filtered_response,
                                } = self.categorize_tools(&response, &tools).await;
                                let requests_to_record: Vec<ToolRequest> = frontend_requests.iter().chain(remaining_requests.iter()).cloned().collect();
                                self.tool_route_manager
                                    .record_tool_requests(&requests_to_record)
                                    .await;

                                yield AgentEvent::Message(filtered_response.clone());
                                tokio::task::yield_now().await;

                                let num_tool_requests = frontend_requests.len() + remaining_requests.len();
                                if num_tool_requests == 0 {
                                    messages_to_add.push(response.clone());
                                    continue;
                                }

                                let tool_response_messages: Vec<Arc<Mutex<Message>>> = (0..num_tool_requests)
                                    .map(|_| Arc::new(Mutex::new(Message::user().with_id(
                                        format!("msg_{}", Uuid::new_v4())
                                    ))))
                                    .collect();

                                let mut request_to_response_map = HashMap::new();
                                for (idx, request) in frontend_requests.iter().chain(remaining_requests.iter()).enumerate() {
                                    request_to_response_map.insert(request.id.clone(), tool_response_messages[idx].clone());
                                }

                                for (idx, request) in frontend_requests.iter().enumerate() {
                                    let mut frontend_tool_stream = self.handle_frontend_tool_request(
                                        request,
                                        tool_response_messages[idx].clone(),
                                    );

                                    while let Some(msg) = frontend_tool_stream.try_next().await? {
                                        yield AgentEvent::Message(msg);
                                    }
                                }
                                if agime_mode == AgimeMode::Chat {
                                    // Skip all remaining tool calls in chat mode
                                    for request in remaining_requests.iter() {
                                        if let Some(response_msg) = request_to_response_map.get(&request.id) {
                                            let mut response = response_msg.lock().await;
                                            *response = response.clone().with_tool_response(
                                                request.id.clone(),
                                                Ok(CallToolResult {
                                                    content: vec![Content::text(CHAT_MODE_TOOL_SKIPPED_RESPONSE)],
                                                    structured_content: None,
                                                    is_error: Some(false),
                                                    meta: None,
                                                }),
                                            );
                                        }
                                    }
                                } else {
                                    // Run all tool inspectors
                                    let inspection_results = self.tool_inspection_manager
                                        .inspect_tools(
                                            &remaining_requests,
                                            conversation.messages(),
                                        )
                                        .await?;

                                    let permission_check_result = self.tool_inspection_manager
                                        .process_inspection_results_with_permission_inspector(
                                            &remaining_requests,
                                            &inspection_results,
                                        )
                                        .unwrap_or_else(|| {
                                            let mut result = PermissionCheckResult {
                                                approved: vec![],
                                                needs_approval: vec![],
                                                denied: vec![],
                                            };
                                            result.needs_approval.extend(remaining_requests.iter().cloned());
                                            result
                                        });

                                    // Track extension requests
                                    let mut enable_extension_request_ids = vec![];
                                    for request in &remaining_requests {
                                        if let Ok(tool_call) = &request.tool_call {
                                            if tool_call.name == MANAGE_EXTENSIONS_TOOL_NAME_COMPLETE {
                                                enable_extension_request_ids.push(request.id.clone());
                                            }
                                        }
                                    }

                                    let mut tool_futures = self.handle_approved_and_denied_tools(
                                        &permission_check_result,
                                        &request_to_response_map,
                                        cancel_token.clone(),
                                        &session,
                                    ).await?;

                                    let tool_futures_arc = Arc::new(Mutex::new(tool_futures));

                                    let mut tool_approval_stream = self.handle_approval_tool_requests(
                                        &permission_check_result.needs_approval,
                                        tool_futures_arc.clone(),
                                        &request_to_response_map,
                                        cancel_token.clone(),
                                        &session,
                                        &inspection_results,
                                    );

                                    while let Some(msg) = tool_approval_stream.try_next().await? {
                                        yield AgentEvent::Message(msg);
                                    }

                                    tool_futures = {
                                        let mut futures_lock = tool_futures_arc.lock().await;
                                        futures_lock.drain(..).collect::<Vec<_>>()
                                    };

                                    let with_id = tool_futures
                                        .into_iter()
                                        .map(|(request_id, stream)| {
                                            stream.map(move |item| (request_id.clone(), item))
                                        })
                                        .collect::<Vec<_>>();

                                    let mut combined = stream::select_all(with_id);
                                    let mut all_install_successful = true;

                                    while let Some((request_id, item)) = combined.next().await {
                                        if is_token_cancelled(&cancel_token) {
                                            break;
                                        }

                                        for msg in Self::drain_elicitation_messages(&session_config.id).await {
                                            yield AgentEvent::Message(msg);
                                        }

                                        match item {
                                            ToolStreamItem::Result(output) => {
                                                if enable_extension_request_ids.contains(&request_id)
                                                    && output.is_err()
                                                {
                                                    all_install_successful = false;
                                                }
                                                if let Some(response_msg) = request_to_response_map.get(&request_id) {
                                                    let mut response = response_msg.lock().await;
                                                    *response = response.clone().with_tool_response(request_id, output);
                                                }
                                            }
                                            ToolStreamItem::Message(msg) => {
                                                yield AgentEvent::McpNotification((request_id, msg));
                                            }
                                        }
                                    }

                                    // check for remaining elicitation messages after all tools complete
                                    for msg in Self::drain_elicitation_messages(&session_config.id).await {
                                        yield AgentEvent::Message(msg);
                                    }

                                    if all_install_successful && !enable_extension_request_ids.is_empty() {
                                        if let Err(e) = self.save_extension_state(&session_config).await {
                                            warn!("Failed to save extension state after runtime changes: {}", e);
                                        }
                                        tools_updated = true;
                                    }
                                }

                                for (idx, request) in frontend_requests.iter().chain(remaining_requests.iter()).enumerate() {
                                    if request.tool_call.is_ok() {
                                        let request_msg = Message::assistant()
                                            .with_id(format!("msg_{}", Uuid::new_v4()))
                                            .with_tool_request(request.id.clone(), request.tool_call.clone());
                                        messages_to_add.push(request_msg);
                                        if let Some(gate_msg) = self
                                            .take_cfpm_tool_gate_inline_message(&request.id)
                                            .await
                                        {
                                            yield AgentEvent::Message(
                                                Message::assistant().with_system_notification(
                                                    SystemNotificationType::InlineMessage,
                                                    gate_msg,
                                                ),
                                            );
                                        }
                                        let final_response = tool_response_messages[idx]
                                                                .lock().await.clone();
                                        yield AgentEvent::Message(final_response.clone());
                                        messages_to_add.push(final_response);
                                    }
                                }

                                no_tools_called = false;
                            }
                        }
                        Err(ref provider_err @ ProviderError::ContextLengthExceeded(_)) => {
                            crate::posthog::emit_error(provider_err.telemetry_type());

                            // Check if we've exceeded maximum compaction attempts
                            if compaction_count >= max_compaction_attempts {
                                error!(
                                    "Exceeded maximum compaction attempts ({}) for strategy {}",
                                    max_compaction_attempts,
                                    compaction_strategy_label(active_compaction_strategy)
                                );
                                yield AgentEvent::Message(
                                    Message::assistant().with_text(
                                        format!(
                                            "已达到最大压缩次数限制 ({} 次)。请创建新会话继续对话。\n\nReached maximum compaction attempts. Please start a new session to continue.",
                                            max_compaction_attempts
                                        )
                                    )
                                );
                                break;
                            }

                            compaction_count += 1;
                            info!(
                                "Recovery compaction attempt {} of {} using {}",
                                compaction_count,
                                max_compaction_attempts,
                                compaction_strategy_label(active_compaction_strategy)
                            );

                            yield AgentEvent::Message(
                                Message::assistant().with_system_notification(
                                    SystemNotificationType::InlineMessage,
                                    "Context limit reached. Compacting to continue conversation...",
                                )
                            );
                            yield AgentEvent::Message(
                                Message::assistant().with_system_notification(
                                    SystemNotificationType::ThinkingMessage,
                                    COMPACTION_THINKING_TEXT,
                                    )
                            );

                            match compact_messages_with_active_strategy(
                                self.provider().await?.as_ref(),
                                &conversation,
                                false,
                            )
                            .await
                            {
                                Ok((compacted_conversation, usage)) => {
                                    SessionManager::replace_conversation(&session_config.id, &compacted_conversation).await?;
                                    if matches!(active_compaction_strategy, ContextCompactionStrategy::CfpmMemoryV1) {
                                        if let Err(err) = SessionManager::replace_cfpm_memory_facts_from_conversation(
                                            &session_config.id,
                                            &compacted_conversation,
                                            "recovery_compaction",
                                        )
                                        .await
                                        {
                                            warn!("Failed to refresh CFPM memory facts: {}", err);
                                        }
                                    }
                                    Self::update_session_metrics(&session_config, &usage, true).await?;
                                    conversation = compacted_conversation;
                                    did_recovery_compact_this_iteration = true;
                                    yield AgentEvent::HistoryReplaced(conversation.clone());
                                    continue;
                                }
                                Err(e) => {
                                    error!("Error: {}", e);
                                    yield AgentEvent::Message(
                                        Message::assistant().with_text(
                                            format!("Ran into this error trying to compact: {e}.\n\nPlease retry if you think this is a transient or recoverable error.")
                                        )
                                    );
                                    break;
                                }
                            }
                        }
                        Err(ref provider_err) => {
                            crate::posthog::emit_error(provider_err.telemetry_type());
                            error!("Error: {}", provider_err);
                            yield AgentEvent::Message(
                                Message::assistant().with_text(
                                    format!("Ran into this error: {provider_err}.\n\nPlease retry if you think this is a transient or recoverable error.")
                                )
                            );
                            break;
                        }
                    }
                }
                if tools_updated {
                    (tools, toolshim_tools, system_prompt) =
                        self.prepare_tools_and_prompt(&working_dir).await?;
                }
                let mut exit_chat = false;
                if no_tools_called {
                    if let Some(final_output_tool) = self.final_output_tool.lock().await.as_ref() {
                        if final_output_tool.final_output.is_none() {
                            warn!("Final output tool has not been called yet. Continuing agent loop.");
                            let message = Message::user().with_text(FINAL_OUTPUT_CONTINUATION_MESSAGE);
                            messages_to_add.push(message.clone());
                            yield AgentEvent::Message(message);
                        } else {
                            let message = Message::assistant().with_text(final_output_tool.final_output.clone().unwrap());
                            messages_to_add.push(message.clone());
                            yield AgentEvent::Message(message);
                            exit_chat = true;
                        }
                    } else if did_recovery_compact_this_iteration {
                        // Avoid setting exit_chat; continue from last user message in the conversation
                    } else {
                        match self.handle_retry_logic(&mut conversation, &session_config, &initial_messages).await {
                            Ok(should_retry) => {
                                if should_retry {
                                    info!("Retry logic triggered, restarting agent loop");
                                } else {
                                    exit_chat = true;
                                }
                            }
                            Err(e) => {
                                error!("Retry logic failed: {}", e);
                                yield AgentEvent::Message(
                                    Message::assistant().with_text(
                                        format!("Retry logic encountered an error: {}", e)
                                    )
                                );
                                exit_chat = true;
                            }
                        }
                    }
                }

                for msg in &messages_to_add {
                    SessionManager::add_message(&session_config.id, msg).await?;
                }
                if matches!(
                    active_compaction_strategy,
                    ContextCompactionStrategy::CfpmMemoryV1
                ) && !messages_to_add.is_empty()
                {
                    match SessionManager::refresh_cfpm_memory_facts_from_recent_messages_with_report(
                        &session_config.id,
                        messages_to_add.messages(),
                        "turn_checkpoint",
                    )
                    .await
                    {
                        Ok(report) => {
                            let visibility = current_cfpm_runtime_visibility();
                            if should_emit_cfpm_runtime_notification(visibility, &report) {
                                let msg = build_cfpm_runtime_inline_message(&report, visibility);
                                yield AgentEvent::Message(
                                    Message::assistant().with_system_notification(
                                        SystemNotificationType::InlineMessage,
                                        msg,
                                    )
                                );
                            }
                        }
                        Err(err) => {
                            warn!("Failed to refresh CFPM memory facts from recent messages: {}", err);
                        }
                    }
                }
                conversation.extend(messages_to_add);
                if exit_chat {
                    break;
                }

                tokio::task::yield_now().await;
            }
        }))
    }

    pub async fn extend_system_prompt(&self, instruction: String) {
        let mut prompt_manager = self.prompt_manager.lock().await;
        prompt_manager.add_system_prompt_extra(instruction);
    }

    pub async fn update_provider(
        &self,
        provider: Arc<dyn Provider>,
        session_id: &str,
    ) -> Result<()> {
        let mut current_provider = self.provider.lock().await;
        *current_provider = Some(provider.clone());

        self.update_router_tool_selector(Some(provider.clone()), None)
            .await?;

        SessionManager::update_session(session_id)
            .provider_name(provider.get_name())
            .model_config(provider.get_model_config())
            .apply()
            .await
            .context("Failed to persist provider config to session")
    }

    pub async fn update_router_tool_selector(
        &self,
        provider: Option<Arc<dyn Provider>>,
        reindex_all: Option<bool>,
    ) -> Result<()> {
        let provider = match provider {
            Some(p) => p,
            None => self.provider().await?,
        };

        // Delegate to ToolRouteManager
        self.tool_route_manager
            .update_router_tool_selector(provider, reindex_all, &self.extension_manager)
            .await
    }

    /// Override the system prompt with a custom template
    pub async fn override_system_prompt(&self, template: String) {
        let mut prompt_manager = self.prompt_manager.lock().await;
        prompt_manager.set_system_prompt_override(template);
    }

    pub async fn list_extension_prompts(&self) -> HashMap<String, Vec<Prompt>> {
        self.extension_manager
            .list_prompts(CancellationToken::default())
            .await
            .expect("Failed to list prompts")
    }

    pub async fn get_prompt(&self, name: &str, arguments: Value) -> Result<GetPromptResult> {
        // First find which extension has this prompt
        let prompts = self
            .extension_manager
            .list_prompts(CancellationToken::default())
            .await
            .map_err(|e| anyhow!("Failed to list prompts: {}", e))?;

        if let Some(extension) = prompts
            .iter()
            .find(|(_, prompt_list)| prompt_list.iter().any(|p| p.name == name))
            .map(|(extension, _)| extension)
        {
            return self
                .extension_manager
                .get_prompt(extension, name, arguments, CancellationToken::default())
                .await
                .map_err(|e| anyhow!("Failed to get prompt: {}", e));
        }

        Err(anyhow!("Prompt '{}' not found", name))
    }

    pub async fn get_plan_prompt(&self) -> Result<String> {
        let tools = self.extension_manager.get_prefixed_tools(None).await?;
        let tools_info = tools
            .into_iter()
            .map(|tool| {
                ToolInfo::new(
                    &tool.name,
                    tool.description
                        .as_ref()
                        .map(|d| d.as_ref())
                        .unwrap_or_default(),
                    get_parameter_names(&tool),
                    None,
                )
            })
            .collect();

        let plan_prompt = self.extension_manager.get_planning_prompt(tools_info).await;

        Ok(plan_prompt)
    }

    pub async fn handle_tool_result(&self, id: String, result: ToolResult<CallToolResult>) {
        if let Err(e) = self.tool_result_tx.send((id, result)).await {
            error!("Failed to send tool result: {}", e);
        }
    }

    pub async fn create_recipe(&self, mut messages: Conversation) -> Result<Recipe> {
        tracing::info!("Starting recipe creation with {} messages", messages.len());

        let extensions_info = self.extension_manager.get_extensions_info().await;
        tracing::debug!("Retrieved {} extensions info", extensions_info.len());
        let (extension_count, tool_count) =
            self.extension_manager.get_extension_and_tool_counts().await;

        // Get model name from provider
        let provider = self.provider().await.map_err(|e| {
            tracing::error!("Failed to get provider for recipe creation: {}", e);
            e
        })?;
        let model_config = provider.get_model_config();
        let model_name = &model_config.model_name;
        tracing::debug!("Using model: {}", model_name);

        let prompt_manager = self.prompt_manager.lock().await;
        let system_prompt = prompt_manager
            .builder(model_name)
            .with_extensions(extensions_info.into_iter())
            .with_frontend_instructions(self.frontend_instructions.lock().await.clone())
            .with_extension_and_tool_counts(extension_count, tool_count)
            .build();

        let recipe_prompt = prompt_manager.get_recipe_prompt().await;
        let tools = self
            .extension_manager
            .get_prefixed_tools(None)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get tools for recipe creation: {}", e);
                e
            })?;

        messages.push(Message::user().with_text(recipe_prompt));

        let (messages, issues) = fix_conversation(messages);
        if !issues.is_empty() {
            issues
                .iter()
                .for_each(|issue| tracing::warn!(recipe.conversation.issue = issue));
        }

        tracing::debug!(
            "Added recipe prompt to messages, total messages: {}",
            messages.len()
        );

        tracing::info!("Calling provider to generate recipe content");
        let (result, _usage) = self
            .provider
            .lock()
            .await
            .as_ref()
            .ok_or_else(|| {
                let error = anyhow!("Provider not available during recipe creation");
                tracing::error!("{}", error);
                error
            })?
            .complete(&system_prompt, messages.messages(), &tools)
            .await
            .map_err(|e| {
                tracing::error!("Provider completion failed during recipe creation: {}", e);
                e
            })?;

        let content = result.as_concat_text();
        tracing::debug!(
            "Provider returned content with {} characters",
            content.len()
        );

        // the response may be contained in ```json ```, strip that before parsing json
        let re = Regex::new(r"(?s)```[^\n]*\n(.*?)\n```").unwrap();
        let clean_content = re
            .captures(&content)
            .and_then(|caps| caps.get(1).map(|m| m.as_str()))
            .unwrap_or(&content)
            .trim()
            .to_string();

        let (instructions, activities) =
            if let Ok(json_content) = serde_json::from_str::<Value>(&clean_content) {
                let instructions = json_content
                    .get("instructions")
                    .ok_or_else(|| anyhow!("Missing 'instructions' in json response"))?
                    .as_str()
                    .ok_or_else(|| anyhow!("instructions' is not a string"))?
                    .to_string();

                let activities = json_content
                    .get("activities")
                    .ok_or_else(|| anyhow!("Missing 'activities' in json response"))?
                    .as_array()
                    .ok_or_else(|| anyhow!("'activities' is not an array'"))?
                    .iter()
                    .map(|act| {
                        act.as_str()
                            .map(|s| s.to_string())
                            .ok_or(anyhow!("'activities' array element is not a string"))
                    })
                    .collect::<Result<_, _>>()?;

                (instructions, activities)
            } else {
                tracing::warn!("Failed to parse JSON, falling back to string parsing");
                // If we can't get valid JSON, try string parsing
                // Use split_once to get the content after "Instructions:".
                let after_instructions = content
                    .split_once("instructions:")
                    .map(|(_, rest)| rest)
                    .unwrap_or(&content);

                // Split once more to separate instructions from activities.
                let (instructions_part, activities_text) = after_instructions
                    .split_once("activities:")
                    .unwrap_or((after_instructions, ""));

                let instructions = instructions_part
                    .trim_end_matches(|c: char| c.is_whitespace() || c == '#')
                    .trim()
                    .to_string();
                let activities_text = activities_text.trim();

                // Regex to remove bullet markers or numbers with an optional dot.
                let bullet_re = Regex::new(r"^[•\-*\d]+\.?\s*").expect("Invalid regex");

                // Process each line in the activities section.
                let activities: Vec<String> = activities_text
                    .lines()
                    .map(|line| bullet_re.replace(line, "").to_string())
                    .map(|s| s.trim().to_string())
                    .filter(|line| !line.is_empty())
                    .collect();

                (instructions, activities)
            };

        let extension_configs = get_enabled_extensions();

        let author = Author {
            contact: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .ok(),
            metadata: None,
        };

        // Ideally we'd get the name of the provider we are using from the provider itself,
        // but it doesn't know and the plumbing looks complicated.
        let config = Config::global();
        let provider_name: String = config
            .get_agime_provider()
            .expect("No provider configured. Run 'goose configure' first");

        let settings = Settings {
            goose_provider: Some(provider_name.clone()),
            agime_model: Some(model_name.clone()),
            temperature: Some(model_config.temperature.unwrap_or(0.0)),
        };

        tracing::debug!(
            "Building recipe with {} activities and {} extensions",
            activities.len(),
            extension_configs.len()
        );

        let (title, description) =
            if let Ok(json_content) = serde_json::from_str::<Value>(&clean_content) {
                let title = json_content
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("从对话创建的预设任务")
                    .to_string();

                let description = json_content
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("基于当前对话会话创建的自定义预设任务")
                    .to_string();

                (title, description)
            } else {
                (
                    "从对话创建的预设任务".to_string(),
                    "基于当前对话会话创建的自定义预设任务".to_string(),
                )
            };

        let recipe = Recipe::builder()
            .title(title)
            .description(description)
            .instructions(instructions)
            .activities(activities)
            .extensions(extension_configs)
            .settings(settings)
            .author(author)
            .build()
            .map_err(|e| {
                tracing::error!("Failed to build recipe: {}", e);
                anyhow!("Recipe build failed: {}", e)
            })?;

        tracing::info!("Recipe creation completed successfully");
        Ok(recipe)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::Response;
    use chrono::Utc;
    use std::fs;

    #[tokio::test]
    async fn test_add_final_output_tool() -> Result<()> {
        let agent = Agent::new();

        let response = Response {
            json_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "result": {"type": "string"}
                }
            })),
        };

        agent.add_final_output_tool(response).await;

        let tools = agent.list_tools(None).await;
        let final_output_tool = tools
            .iter()
            .find(|tool| tool.name == FINAL_OUTPUT_TOOL_NAME);

        assert!(
            final_output_tool.is_some(),
            "Final output tool should be present after adding"
        );

        let prompt_manager = agent.prompt_manager.lock().await;
        let system_prompt = prompt_manager.builder("gpt-4o").build();

        let final_output_tool_ref = agent.final_output_tool.lock().await;
        let final_output_tool_system_prompt =
            final_output_tool_ref.as_ref().unwrap().system_prompt();
        assert!(system_prompt.contains(&final_output_tool_system_prompt));
        Ok(())
    }

    #[tokio::test]
    async fn test_tool_inspection_manager_has_all_inspectors() -> Result<()> {
        let agent = Agent::new();

        // Verify that the tool inspection manager has all expected inspectors
        let inspector_names = agent.tool_inspection_manager.inspector_names();

        assert!(
            inspector_names.contains(&"repetition"),
            "Tool inspection manager should contain repetition inspector"
        );
        assert!(
            inspector_names.contains(&"permission"),
            "Tool inspection manager should contain permission inspector"
        );
        assert!(
            inspector_names.contains(&"security"),
            "Tool inspection manager should contain security inspector"
        );

        Ok(())
    }

    #[test]
    fn test_build_runtime_memory_context_text_prefers_active_facts() {
        let now = Utc::now();
        let facts = vec![
            crate::session::session_manager::MemoryFact {
                id: "mem_1".to_string(),
                session_id: "session_1".to_string(),
                category: "artifact".to_string(),
                content: "C:\\Users\\jsjm\\OneDrive\\Desktop".to_string(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.9,
                evidence_count: 2,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
            crate::session::session_manager::MemoryFact {
                id: "mem_2".to_string(),
                session_id: "session_1".to_string(),
                category: "artifact".to_string(),
                content: "C:\\temp\\outdated".to_string(),
                status: MemoryFactStatus::Stale,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.6,
                evidence_count: 1,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
        ];

        let text = build_runtime_memory_context_text(&facts)
            .expect("runtime memory context should be generated");
        assert!(text.contains("C:\\Users\\jsjm\\OneDrive\\Desktop"));
        assert!(!text.contains("C:\\temp\\outdated"));
        assert!(text.contains("Do not re-discover known paths"));
    }

    #[test]
    fn test_build_runtime_memory_context_text_filters_date_artifact_noise() {
        let now = Utc::now();
        let facts = vec![
            crate::session::session_manager::MemoryFact {
                id: "mem_date".to_string(),
                session_id: "session_1".to_string(),
                category: "artifact".to_string(),
                content: "2025/12/29".to_string(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.9,
                evidence_count: 1,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
            crate::session::session_manager::MemoryFact {
                id: "mem_path".to_string(),
                session_id: "session_1".to_string(),
                category: "artifact".to_string(),
                content: "C:\\Users\\jsjm\\OneDrive\\Desktop".to_string(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.9,
                evidence_count: 1,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
        ];

        let text = build_runtime_memory_context_text(&facts)
            .expect("runtime memory context should be generated");
        assert!(!text.contains("2025/12/29"));
        assert!(text.contains("C:\\Users\\jsjm\\OneDrive\\Desktop"));
    }

    #[test]
    fn test_build_runtime_memory_context_text_includes_invalid_path_section() {
        let now = Utc::now();
        let facts = vec![
            crate::session::session_manager::MemoryFact {
                id: "mem_invalid".to_string(),
                session_id: "session_1".to_string(),
                category: "invalid_path".to_string(),
                content: "C:\\Users\\jsjm\\Desktop".to_string(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.95,
                evidence_count: 1,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
            crate::session::session_manager::MemoryFact {
                id: "mem_valid".to_string(),
                session_id: "session_1".to_string(),
                category: "artifact".to_string(),
                content: "C:\\Users\\jsjm\\OneDrive\\Desktop".to_string(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.9,
                evidence_count: 2,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
        ];

        let text =
            build_runtime_memory_context_text(&facts).expect("runtime memory context should exist");
        assert!(text.contains("Known invalid paths (avoid reuse unless user asks to re-verify):"));
        assert!(text.contains("C:\\Users\\jsjm\\Desktop"));
        assert!(text.contains("C:\\Users\\jsjm\\OneDrive\\Desktop"));
    }

    #[test]
    fn test_build_cfpm_runtime_inline_message_uses_structured_payload() {
        let report = CfpmRuntimeReport {
            reason: "turn_checkpoint".to_string(),
            mode: "merge".to_string(),
            accepted_count: 2,
            rejected_count: 1,
            rejected_reason_breakdown: vec!["artifact_unhelpful=1".to_string()],
            pruned_count: 0,
            fact_count: 9,
        };

        let msg = build_cfpm_runtime_inline_message(&report, CfpmRuntimeVisibility::Debug);
        assert!(msg.starts_with("[CFPM_RUNTIME_V1] "));

        let payload = msg.trim_start_matches("[CFPM_RUNTIME_V1] ").trim();
        let value: serde_json::Value = serde_json::from_str(payload).expect("valid cfpm payload");

        assert_eq!(value["verbosity"], "debug");
        assert_eq!(value["reason"], "turn_checkpoint");
        assert_eq!(value["mode"], "merge");
        assert_eq!(value["acceptedCount"], 2);
        assert_eq!(value["rejectedCount"], 1);
        assert_eq!(value["factCount"], 9);
    }

    #[test]
    fn test_build_cfpm_tool_gate_inline_message_uses_structured_payload() {
        let event = CfpmToolGateEvent {
            tool: "developer__shell_command".to_string(),
            target: "desktop".to_string(),
            path: "C:\\Users\\jsjm\\OneDrive\\Desktop".to_string(),
            original_command: "Get-ChildItem \"$env:USERPROFILE/Desktop\"".to_string(),
            rewritten_command: "Get-ChildItem 'C:\\Users\\jsjm\\OneDrive\\Desktop'".to_string(),
        };

        let msg = build_cfpm_tool_gate_inline_message(&event, CfpmRuntimeVisibility::Debug);
        assert!(msg.starts_with("[CFPM_TOOL_GATE_V1] "));

        let payload = msg.trim_start_matches("[CFPM_TOOL_GATE_V1] ").trim();
        let value: serde_json::Value = serde_json::from_str(payload).expect("valid gate payload");

        assert_eq!(value["verbosity"], "debug");
        assert_eq!(value["action"], "rewrite_known_folder_probe");
        assert_eq!(value["tool"], "developer__shell_command");
        assert_eq!(value["target"], "desktop");
        assert_eq!(value["path"], "C:\\Users\\jsjm\\OneDrive\\Desktop");
    }

    #[tokio::test]
    async fn test_cfpm_tool_gate_event_roundtrip_message() {
        let agent = Agent::new();
        agent
            .store_cfpm_tool_gate_event(
                "req_1",
                CfpmToolGateEvent {
                    tool: "developer__shell_command".to_string(),
                    target: "desktop".to_string(),
                    path: "C:\\Users\\jsjm\\OneDrive\\Desktop".to_string(),
                    original_command: "Get-ChildItem \"$env:USERPROFILE/Desktop\"".to_string(),
                    rewritten_command: "Get-ChildItem 'C:\\Users\\jsjm\\OneDrive\\Desktop'"
                        .to_string(),
                },
            )
            .await;

        let msg = agent
            .take_cfpm_tool_gate_inline_message("req_1")
            .await
            .expect("expected gate inline message");
        assert!(msg.starts_with("[CFPM_TOOL_GATE_V1] "));
        assert!(msg.contains("\"target\":\"desktop\""));
        assert!(agent
            .take_cfpm_tool_gate_inline_message("req_1")
            .await
            .is_none());
    }

    #[test]
    fn test_target_mentions_in_command_extracts_known_folders() {
        let targets = target_mentions_in_command(
            "Get-ChildItem \"$env:USERPROFILE/Desktop\"; Get-ChildItem \"$env:USERPROFILE/Documents\"",
        );
        assert!(targets.contains(&KnownFolderTarget::Desktop));
        assert!(targets.contains(&KnownFolderTarget::Documents));
        assert!(!targets.contains(&KnownFolderTarget::Downloads));
    }

    #[test]
    fn test_rewrite_known_folder_probe_in_command_with_env_userprofile() {
        let rewritten = rewrite_known_folder_probe_in_command(
            "Get-ChildItem \"$env:USERPROFILE/Desktop\" | Select-Object Name",
            KnownFolderTarget::Desktop,
            "C:\\Users\\jsjm\\OneDrive\\Desktop",
        )
        .expect("command should be rewritten");
        assert!(rewritten.contains("'C:\\Users\\jsjm\\OneDrive\\Desktop'"));
        assert!(!rewritten.to_ascii_lowercase().contains("$env:userprofile"));
    }

    #[test]
    fn test_rewrite_known_folder_probe_in_command_with_getfolderpath() {
        let rewritten = rewrite_known_folder_probe_in_command(
            "[Environment]::GetFolderPath('Desktop')",
            KnownFolderTarget::Desktop,
            "C:\\Users\\jsjm\\OneDrive\\Desktop",
        )
        .expect("command should be rewritten");
        assert_eq!(rewritten, "'C:\\Users\\jsjm\\OneDrive\\Desktop'");
    }

    #[test]
    fn test_find_known_folder_path_from_facts_prefers_active_artifacts() {
        let now = Utc::now();
        let facts = vec![
            crate::session::session_manager::MemoryFact {
                id: "mem_1".to_string(),
                session_id: "session_1".to_string(),
                category: "artifact".to_string(),
                content: "C:\\Users\\jsjm\\OneDrive\\Desktop".to_string(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.9,
                evidence_count: 1,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
            crate::session::session_manager::MemoryFact {
                id: "mem_2".to_string(),
                session_id: "session_1".to_string(),
                category: "artifact".to_string(),
                content: "C:\\Users\\jsjm\\Desktop\\notes.txt".to_string(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.85,
                evidence_count: 1,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
        ];

        let desktop = find_known_folder_path_from_facts(&facts, KnownFolderTarget::Desktop, None)
            .expect("desktop path should be found");
        assert_eq!(desktop, "C:\\Users\\jsjm\\OneDrive\\Desktop");
    }

    #[test]
    fn test_find_known_folder_path_from_facts_ignores_symbolic_candidates() {
        let now = Utc::now();
        let facts = vec![
            crate::session::session_manager::MemoryFact {
                id: "mem_1".to_string(),
                session_id: "session_1".to_string(),
                category: "artifact".to_string(),
                content: "$env:USERPROFILE/Desktop".to_string(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.8,
                evidence_count: 1,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
            crate::session::session_manager::MemoryFact {
                id: "mem_2".to_string(),
                session_id: "session_1".to_string(),
                category: "note".to_string(),
                content: "C:\\Users\\jsjm\\OneDrive\\Desktop".to_string(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "user".to_string(),
                confidence: 1.0,
                evidence_count: 2,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
        ];

        let desktop = find_known_folder_path_from_facts(&facts, KnownFolderTarget::Desktop, None)
            .expect("desktop path should be found");
        assert_eq!(desktop, "C:\\Users\\jsjm\\OneDrive\\Desktop");
    }

    #[test]
    fn test_find_known_folder_path_from_facts_prefers_existing_known_folder() {
        let now = Utc::now();
        let existing_root = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("target_cfpm_test")
            .join("cfpm_known_folder_exists");
        let existing = existing_root.join("desktop");
        let _ = fs::remove_dir_all(&existing_root);
        fs::create_dir_all(&existing).expect("should create temp desktop-like directory");
        let existing_path = existing.to_string_lossy().to_string();

        let facts = vec![
            crate::session::session_manager::MemoryFact {
                id: "mem_old".to_string(),
                session_id: "session_1".to_string(),
                category: "artifact".to_string(),
                content: "C:\\Users\\jsjm\\Desktop".to_string(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.9,
                evidence_count: 1,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
            crate::session::session_manager::MemoryFact {
                id: "mem_new".to_string(),
                session_id: "session_1".to_string(),
                category: "artifact".to_string(),
                content: existing_path.clone(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.95,
                evidence_count: 3,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
        ];

        let desktop = find_known_folder_path_from_facts(&facts, KnownFolderTarget::Desktop, None)
            .expect("desktop path should be found");
        assert_eq!(desktop, existing_path);

        let _ = fs::remove_dir_all(existing_root);
    }

    #[test]
    fn test_find_known_folder_path_from_facts_skips_nonexistent_fallback() {
        let now = Utc::now();
        let missing_root = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("target_cfpm_test")
            .join("cfpm_known_folder_missing");
        let missing = missing_root.join("desktop");
        let _ = fs::remove_dir_all(&missing_root);
        let missing_path = missing.to_string_lossy().to_string();

        let facts = vec![crate::session::session_manager::MemoryFact {
            id: "mem_missing".to_string(),
            session_id: "session_1".to_string(),
            category: "artifact".to_string(),
            content: missing_path,
            status: MemoryFactStatus::Active,
            pinned: false,
            source: "cfpm_auto".to_string(),
            confidence: 0.9,
            evidence_count: 1,
            last_validated_at: Some(now),
            validation_command: None,
            created_at: now,
            updated_at: now,
        }];

        let desktop = find_known_folder_path_from_facts(&facts, KnownFolderTarget::Desktop, None);
        assert!(desktop.is_none());
    }

    #[test]
    fn test_find_known_folder_path_from_facts_skips_invalid_path_fact() {
        let now = Utc::now();
        let facts = vec![
            crate::session::session_manager::MemoryFact {
                id: "mem_artifact".to_string(),
                session_id: "session_1".to_string(),
                category: "artifact".to_string(),
                content: "C:\\Users\\jsjm\\Desktop".to_string(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.7,
                evidence_count: 2,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
            crate::session::session_manager::MemoryFact {
                id: "mem_invalid".to_string(),
                session_id: "session_1".to_string(),
                category: "invalid_path".to_string(),
                content: "C:\\Users\\jsjm\\Desktop".to_string(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.95,
                evidence_count: 1,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
        ];

        let desktop = find_known_folder_path_from_facts(&facts, KnownFolderTarget::Desktop, None);
        assert!(desktop.is_none());
    }

    #[test]
    fn test_find_known_folder_path_from_facts_skips_recent_invalid_paths() {
        let now = Utc::now();
        let facts = vec![crate::session::session_manager::MemoryFact {
            id: "mem_artifact".to_string(),
            session_id: "session_1".to_string(),
            category: "artifact".to_string(),
            content: "C:\\Users\\jsjm\\Desktop".to_string(),
            status: MemoryFactStatus::Active,
            pinned: false,
            source: "cfpm_auto".to_string(),
            confidence: 0.95,
            evidence_count: 5,
            last_validated_at: Some(now),
            validation_command: None,
            created_at: now,
            updated_at: now,
        }];
        let mut extra_invalid_paths = HashSet::new();
        extra_invalid_paths.insert("c:\\users\\jsjm\\desktop".to_string());

        let desktop = find_known_folder_path_from_facts(
            &facts,
            KnownFolderTarget::Desktop,
            Some(&extra_invalid_paths),
        );
        assert!(desktop.is_none());
    }

    #[test]
    fn test_find_known_folder_path_from_facts_skips_low_trust_auto_fact() {
        let now = Utc::now();
        let facts = vec![crate::session::session_manager::MemoryFact {
            id: "mem_low_trust".to_string(),
            session_id: "session_1".to_string(),
            category: "artifact".to_string(),
            content: "C:\\Users\\jsjm\\Desktop".to_string(),
            status: MemoryFactStatus::Active,
            pinned: false,
            source: "cfpm_auto".to_string(),
            confidence: 0.7,
            evidence_count: 1,
            last_validated_at: Some(now),
            validation_command: None,
            created_at: now,
            updated_at: now,
        }];

        let desktop = find_known_folder_path_from_facts(&facts, KnownFolderTarget::Desktop, None);
        assert!(desktop.is_none());
    }

    #[test]
    fn test_find_known_folder_path_from_facts_skips_failure_context_lines() {
        let now = Utc::now();
        let facts = vec![
            crate::session::session_manager::MemoryFact {
                id: "mem_bad_sentence".to_string(),
                session_id: "session_1".to_string(),
                category: "artifact".to_string(),
                content:
                    "系统显示桌面路径是 C:\\Users\\jsjm\\Desktop，但访问不了。让我尝试列出用户目录来找到实际位置。"
                        .to_string(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.99,
                evidence_count: 10,
                last_validated_at: Some(now),
                validation_command: None,
                created_at: now,
                updated_at: now,
            },
            crate::session::session_manager::MemoryFact {
                id: "mem_good".to_string(),
                session_id: "session_1".to_string(),
                category: "artifact".to_string(),
                content: "C:\\Users\\jsjm\\OneDrive\\Desktop".to_string(),
                status: MemoryFactStatus::Active,
                pinned: false,
                source: "cfpm_auto".to_string(),
                confidence: 0.7,
                evidence_count: 2,
                last_validated_at: Some(now),
                validation_command: Some("[Environment]::GetFolderPath('Desktop')".to_string()),
                created_at: now,
                updated_at: now,
            },
        ];

        let desktop = find_known_folder_path_from_facts(&facts, KnownFolderTarget::Desktop, None);
        assert_eq!(
            desktop.as_deref(),
            Some("C:\\Users\\jsjm\\OneDrive\\Desktop")
        );
    }

    #[test]
    fn test_find_known_folder_path_from_facts_requires_validation_for_cfpm_auto() {
        let now = Utc::now();
        let facts = vec![crate::session::session_manager::MemoryFact {
            id: "mem_unvalidated".to_string(),
            session_id: "session_1".to_string(),
            category: "artifact".to_string(),
            content: "C:\\Users\\jsjm\\Desktop".to_string(),
            status: MemoryFactStatus::Active,
            pinned: false,
            source: "cfpm_auto".to_string(),
            confidence: 0.99,
            evidence_count: 8,
            last_validated_at: Some(now),
            validation_command: None,
            created_at: now,
            updated_at: now,
        }];

        let desktop = find_known_folder_path_from_facts(&facts, KnownFolderTarget::Desktop, None);
        assert!(desktop.is_none());
    }

    #[test]
    fn test_find_known_folder_path_from_facts_accepts_validation_with_onedrive_hint() {
        let now = Utc::now();
        let facts = vec![crate::session::session_manager::MemoryFact {
            id: "mem_validated".to_string(),
            session_id: "session_1".to_string(),
            category: "artifact".to_string(),
            content: "C:\\Users\\jsjm\\OneDrive\\Desktop".to_string(),
            status: MemoryFactStatus::Active,
            pinned: false,
            source: "cfpm_auto".to_string(),
            confidence: 0.7,
            evidence_count: 2,
            last_validated_at: Some(now),
            validation_command: Some(
                "Get-ChildItem 'C:\\Users\\jsjm\\OneDrive\\Desktop'".to_string(),
            ),
            created_at: now,
            updated_at: now,
        }];

        let desktop = find_known_folder_path_from_facts(&facts, KnownFolderTarget::Desktop, None);
        assert_eq!(
            desktop.as_deref(),
            Some("C:\\Users\\jsjm\\OneDrive\\Desktop")
        );
    }

    #[test]
    fn test_rewrite_known_folder_probe_in_command_skips_symbolic_resolved_path() {
        let rewritten = rewrite_known_folder_probe_in_command(
            "Get-ChildItem \"$env:USERPROFILE/Desktop\" | Select-Object Name",
            KnownFolderTarget::Desktop,
            "$env:USERPROFILE/Desktop",
        );
        assert!(rewritten.is_none());
    }

    #[test]
    fn test_is_explicit_absolute_path_reference() {
        assert!(is_explicit_absolute_path_reference(
            "Get-ChildItem 'C:\\Users\\jsjm\\OneDrive\\Desktop'"
        ));
        assert!(!is_explicit_absolute_path_reference(
            "Get-ChildItem \"$env:USERPROFILE/Desktop\""
        ));
    }
}
