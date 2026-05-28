//! Host-side runtime helpers.
//!
//! Pure-logic counterparts of the team-server `server_harness_host.rs`
//! decision helpers, kept as a flat copy on the desktop side. Each function
//! is **byte-for-byte derived** from the team source (commit-time snapshot)
//! so the two sides start from a known-equal baseline; from here they drift
//! independently per the project's "copy + dual-track" plan.
//!
//! SOURCE: crates/agime-team-server/src/agent/server_harness_host.rs
//! (see `crates/agime-team-server/src/agent/server_harness_host.rs` lines
//! 84..1030 and 3380..3580 for the upstream implementations).
//!
//! Only helpers that depend on the public `agime` API or on plain primitives
//! are copied here. Helpers that operate on team-server-only types
//! (`AgentSessionDoc`, `TeamAgent`, `WorkspaceExecutionContext`,
//! `task_manager::StreamEvent`) are intentionally **not** included; the
//! desktop side rewires them through value types in the sibling
//! `host_provider`, `host_workspace`, and `host_prompt` modules.

#![allow(dead_code)]

use std::collections::HashSet;

use agime::agents::harness::CompletionSurfacePolicy;
use agime::agents::{
    build_permission_requested_control_message, build_permission_resolved_control_message,
    build_permission_timed_out_control_message, build_tool_finished_control_message,
    build_tool_started_control_message, build_worker_finished_control_message,
    build_worker_followup_requested_control_message, build_worker_idle_control_message,
    build_worker_progress_control_message, build_worker_started_control_message,
    native_swarm_tool_enabled, planner_auto_swarm_enabled, CompletionControlEvent,
    CoordinatorExecutionMode, CoordinatorSignalSummary, DelegationMode,
    ExecutionHostCompletionReport, HarnessControlMessage, HarnessMode, PermissionControlEvent,
    ProviderTurnMode, RuntimeControlEvent, SessionControlEvent, TaskKind, TaskSnapshot,
    ToolControlEvent, WorkerAttemptIdentity, WorkerControlEvent,
};
use agime::context_runtime::{observe_runtime_transition, ContextRuntimeState};
use agime::utils::normalize_delegation_summary_text;

/// Return whether the given `session_source` should never delegate work to a
/// subagent / swarm worker. Mirrors `agime_runtime::capability_policy::is_non_delegating_session_source`.
pub fn is_non_delegating_session_source(session_source: &str) -> bool {
    session_source.eq_ignore_ascii_case("portal_manager")
        || session_source.eq_ignore_ascii_case("scheduled_task")
        || session_source.eq_ignore_ascii_case("automation_builder")
}

/// Detect explicit user requests for delegation / swarm / parallel work in
/// either English or Chinese. Mirrors
/// `agime_runtime::harness_host_helpers::has_explicit_delegation_request`.
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

/// Parse the `<turn_tool_gate mode="allow_only">` block out of a turn-system
/// instruction string and return the unique tool names listed inside.
pub fn parse_turn_tool_gate_allow_only(
    turn_system_instruction: Option<&str>,
) -> Option<HashSet<String>> {
    let text = turn_system_instruction?;
    let start = text.find("<turn_tool_gate mode=\"allow_only\">")?;
    let after = &text[start + "<turn_tool_gate mode=\"allow_only\">".len()..];
    let end = after.find("</turn_tool_gate>")?;
    let block = &after[..end];
    let names = block
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect::<HashSet<_>>();
    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

/// Strip diagnostic / "next-steps" boilerplate from raw runtime text so that
/// the remaining text is appropriate to surface back to the end user.
pub fn sanitize_user_visible_runtime_text(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }

    let mut text = raw.replace("\r\n", "\n");
    if let Some(index) = text.find("Harness mode changed:") {
        text.truncate(index);
    }
    text = text.replace("(no_tool_backed_progress)", "");

    let mut cleaned_lines = Vec::new();
    let mut skipping_next_steps = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !skipping_next_steps {
                cleaned_lines.push(String::new());
            }
            continue;
        }

        let is_diagnostic_line = trimmed == "Runtime exited without structured final_output."
            || trimmed.starts_with("Validation:")
            || trimmed.starts_with("Blocking reason:")
            || trimmed.starts_with("Harness mode changed:")
            || trimmed == "(no_tool_backed_progress)";

        if trimmed == "Next steps:" {
            skipping_next_steps = true;
            continue;
        }

        if skipping_next_steps {
            if trimmed.starts_with("- ") || trimmed.starts_with("• ") || trimmed.starts_with("* ")
            {
                continue;
            }
            skipping_next_steps = false;
        }

        if is_diagnostic_line {
            continue;
        }

        cleaned_lines.push(trimmed.to_string());
    }

    let cleaned = normalize_delegation_summary_text(&cleaned_lines.join("\n"));
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// Whether `tool_name` is provided directly by the host process rather than
/// by an extension subprocess.
pub fn is_server_local_tool_name(
    server_local_tool_names: &HashSet<String>,
    tool_name: &str,
) -> bool {
    server_local_tool_names.contains(tool_name)
}

/// Decide which coordinator execution mode (single worker / auto swarm /
/// explicit swarm) fits a turn given the user's message and the deliverable
/// targets the host has decided on.
pub fn infer_coordinator_execution_mode(
    user_message: &str,
    target_artifacts: &[String],
    result_contract: &[String],
) -> CoordinatorExecutionMode {
    let lowered = user_message.to_ascii_lowercase();
    let explicit_swarm_request = lowered.contains("swarm")
        || lowered.contains("parallel")
        || user_message.contains("并行")
        || user_message.contains("平行")
        || user_message.contains("多个 worker")
        || user_message.contains("多 worker");

    let stable_target_count = target_artifacts
        .iter()
        .chain(result_contract.iter())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<HashSet<_>>()
        .len();

    if explicit_swarm_request && native_swarm_tool_enabled() && stable_target_count >= 2 {
        CoordinatorExecutionMode::ExplicitSwarm
    } else if planner_auto_swarm_enabled() && stable_target_count >= 2 {
        CoordinatorExecutionMode::AutoSwarm
    } else {
        CoordinatorExecutionMode::SingleWorker
    }
}

/// Map a coordinator execution mode to the delegation mode that the harness
/// turn loop should run in.
pub fn delegation_mode_for_execution_mode(
    coordinator_execution_mode: CoordinatorExecutionMode,
) -> DelegationMode {
    match coordinator_execution_mode {
        CoordinatorExecutionMode::ExplicitSwarm | CoordinatorExecutionMode::AutoSwarm => {
            DelegationMode::Swarm
        }
        CoordinatorExecutionMode::SingleWorker => DelegationMode::Subagent,
    }
}

/// Map an opaque session-source string (e.g. `chat`, `agent_task`, `system`)
/// to the appropriate harness mode.
pub fn harness_mode_for_session_source(session_source: &str) -> HarnessMode {
    if session_source.eq_ignore_ascii_case("system") {
        HarnessMode::Execute
    } else if session_source.eq_ignore_ascii_case("channel_runtime") {
        HarnessMode::Execute
    } else if session_source.eq_ignore_ascii_case("agent_task")
        || session_source.eq_ignore_ascii_case("subagent")
    {
        HarnessMode::Execute
    } else if session_source.eq_ignore_ascii_case("channel_conversation") {
        HarnessMode::Conversation
    } else {
        HarnessMode::Conversation
    }
}

/// Decide whether to flip the harness from Conversation to Execute on the
/// strength of an explicit "delegate this" hint in the user's message.
pub fn should_force_execute_for_explicit_delegation(
    session_source: &str,
    user_message: &str,
    explicit_delegation_turn: bool,
) -> bool {
    if is_non_delegating_session_source(session_source) {
        return false;
    }
    if explicit_delegation_turn {
        return true;
    }

    let delegation_surface = session_source.eq_ignore_ascii_case("chat")
        || session_source.eq_ignore_ascii_case("automation_runtime")
        || session_source.eq_ignore_ascii_case("channel_conversation");
    delegation_surface && has_explicit_delegation_request(user_message)
}

/// Map a harness mode to the provider-turn mode (aggregated vs streaming)
/// the LLM call should run with.
pub fn provider_turn_mode_for_harness_mode(harness_mode: HarnessMode) -> ProviderTurnMode {
    match harness_mode {
        HarnessMode::Execute => ProviderTurnMode::Aggregated,
        HarnessMode::Conversation
        | HarnessMode::Plan
        | HarnessMode::Repair
        | HarnessMode::Blocked
        | HarnessMode::Complete => ProviderTurnMode::Streaming,
    }
}

/// Channel-side overrides for the provider-turn mode: channel surfaces
/// always stream, regardless of harness mode.
pub fn provider_turn_mode_for_session_source(
    session_source: &str,
    harness_mode: HarnessMode,
) -> ProviderTurnMode {
    if session_source.eq_ignore_ascii_case("channel_runtime")
        || session_source.eq_ignore_ascii_case("channel_conversation")
    {
        ProviderTurnMode::Streaming
    } else {
        provider_turn_mode_for_harness_mode(harness_mode)
    }
}

/// Pick the completion-surface policy (which closing message format the
/// harness should require / accept) that fits a session source + harness
/// mode pair.
pub fn completion_surface_policy_for_session_source(
    session_source: &str,
    harness_mode: HarnessMode,
) -> CompletionSurfacePolicy {
    if session_source.eq_ignore_ascii_case("system") {
        CompletionSurfacePolicy::SystemDocumentAnalysis
    } else if session_source.eq_ignore_ascii_case("document_analysis") {
        CompletionSurfacePolicy::Conversation
    } else if session_source.eq_ignore_ascii_case("chat")
        || session_source.eq_ignore_ascii_case("automation_runtime")
    {
        CompletionSurfacePolicy::Conversation
    } else if matches!(harness_mode, HarnessMode::Conversation) {
        CompletionSurfacePolicy::Conversation
    } else {
        CompletionSurfacePolicy::Execute
    }
}

/// Tool-name prefixes the harness must allow through for a given session
/// source's required-tools contract.
pub fn required_tool_prefixes_for_session_source(session_source: &str) -> Vec<String> {
    if session_source.eq_ignore_ascii_case("system") {
        vec![
            "document_tools__read_document".to_string(),
            "document_tools__export_document".to_string(),
            "document_tools__import_document_to_workspace".to_string(),
        ]
    } else {
        Vec::new()
    }
}

/// Heuristic: detect summaries that read like "I will read the document
/// next" and should therefore be considered non-terminal in document
/// analysis flows.
pub fn is_non_terminal_document_analysis_summary(summary: &str) -> bool {
    let normalized = summary.trim().to_ascii_lowercase();
    let future_intent_patterns = [
        "i need to read the document first",
        "i'll start by reading",
        "let me do that now",
        "before i can provide a final output",
        "i need to read the document",
        "我需要先读取文档",
        "我先读取文档",
        "我需要先阅读文档",
        "让我先",
        "在给出最终答案之前",
    ];
    future_intent_patterns
        .iter()
        .any(|pattern| normalized.contains(pattern))
}

/// Apply the team-side blocking-signal / next-steps normalization to an
/// `ExecutionHostCompletionReport`. Pure data — no IO.
pub fn normalize_adapter_execution_host_completion_report(
    mut report: ExecutionHostCompletionReport,
    signal_summary: Option<&CoordinatorSignalSummary>,
    session_source: &str,
) -> ExecutionHostCompletionReport {
    let conversation_surface = session_source.eq_ignore_ascii_case("chat")
        || session_source.eq_ignore_ascii_case("automation_runtime")
        || session_source.eq_ignore_ascii_case("portal")
        || session_source.eq_ignore_ascii_case("channel_conversation");

    if session_source.eq_ignore_ascii_case("document_analysis") {
        report =
            agime::agents::harness::normalize_pre_materialized_document_analysis_completion_report(
                report,
                signal_summary,
            );
    } else if session_source.eq_ignore_ascii_case("system") {
        report = agime::agents::harness::normalize_system_document_analysis_completion_report(
            report,
            signal_summary,
        );
    } else if !conversation_surface
        && report.status == "completed"
        && signal_summary.is_some_and(|summary| summary.has_hard_blocking_signals())
    {
        report.status = "blocked".to_string();
        if report.blocking_reason.is_none() {
            report.blocking_reason = signal_summary
                .and_then(CoordinatorSignalSummary::default_blocking_reason)
                .or_else(|| Some("runtime signals indicate incomplete execution".to_string()));
        }
    }

    if report.status == "blocked" && report.next_steps.is_empty() {
        report.next_steps.push(
            "Provide additional context or retry after resolving the blocking issue.".to_string(),
        );
    }

    if report.status == "completed" {
        report.blocking_reason = None;
    }

    report
}

/// Build a "context_runtime"-strategy compaction observation suitable for
/// emitting on the runtime control channel.
pub fn build_context_runtime_compaction_observation(
    initial: Option<&ContextRuntimeState>,
    final_state: Option<&ContextRuntimeState>,
) -> Option<HarnessControlMessage> {
    let final_state = final_state?;
    let observation = observe_runtime_transition(initial, final_state)?;
    Some(HarnessControlMessage::Runtime(
        RuntimeControlEvent::CompactionObserved {
            strategy: Some("context_runtime".to_string()),
            reason: observation.reason.clone(),
            before_tokens: observation.before_tokens,
            after_tokens: observation.after_tokens,
            phase: Some(observation.phase),
        },
    ))
}

/// Render a `TaskKind` as the host's wire-protocol label.
pub fn task_kind_label(kind: TaskKind) -> String {
    match kind {
        TaskKind::Subagent => "subagent".to_string(),
        TaskKind::SwarmWorker => "swarm_worker".to_string(),
        TaskKind::ValidationWorker => "validation_worker".to_string(),
    }
}

/// Stable label for a control message's logical channel.
pub fn control_channel_label(payload: &HarnessControlMessage) -> &'static str {
    match payload {
        HarnessControlMessage::Session(_) => "session",
        HarnessControlMessage::Tool(_) => "tool",
        HarnessControlMessage::Permission(_) => "permission",
        HarnessControlMessage::Worker(_) => "worker",
        HarnessControlMessage::Completion(_) => "completion",
        HarnessControlMessage::Runtime(_) => "runtime",
    }
}

/// Stable label for a control message's event variant.
pub fn control_event_type_label(payload: &HarnessControlMessage) -> &'static str {
    match payload {
        HarnessControlMessage::Session(event) => match event {
            SessionControlEvent::Started { .. } => "started",
            SessionControlEvent::StateChanged { .. } => "state_changed",
            SessionControlEvent::Interrupted { .. } => "interrupted",
            SessionControlEvent::CancelRequested { .. } => "cancel_requested",
            SessionControlEvent::Finished { .. } => "finished",
        },
        HarnessControlMessage::Tool(event) => match event {
            ToolControlEvent::TransportRequested { .. } => "transport_requested",
            ToolControlEvent::Started { .. } => "started",
            ToolControlEvent::Progress { .. } => "progress",
            ToolControlEvent::Finished { .. } => "finished",
        },
        HarnessControlMessage::Permission(event) => match event {
            PermissionControlEvent::Requested { .. } => "requested",
            PermissionControlEvent::Resolved { .. } => "resolved",
            PermissionControlEvent::TimedOut { .. } => "timed_out",
            PermissionControlEvent::RequiresAction { .. } => "requires_action",
        },
        HarnessControlMessage::Worker(event) => match event {
            WorkerControlEvent::Started { .. } => "started",
            WorkerControlEvent::Progress { .. } => "progress",
            WorkerControlEvent::Idle { .. } => "idle",
            WorkerControlEvent::FollowupRequested { .. } => "followup_requested",
            WorkerControlEvent::Finished { .. } => "finished",
        },
        HarnessControlMessage::Completion(event) => match event {
            CompletionControlEvent::StructuredPublished { .. } => "structured_published",
            CompletionControlEvent::OutcomeObserved { .. } => "outcome_observed",
        },
        HarnessControlMessage::Runtime(event) => match event {
            RuntimeControlEvent::CompactionObserved { .. } => "compaction_observed",
            RuntimeControlEvent::Notification { .. } => "notification",
        },
    }
}

pub fn worker_started_mirror(
    snapshot: &TaskSnapshot,
    _attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_worker_started_control_message(snapshot)
}

pub fn worker_progress_mirror(
    task_id: String,
    message: String,
    percent: Option<u8>,
) -> HarnessControlMessage {
    build_worker_progress_control_message(task_id, message, percent)
}

pub fn worker_followup_mirror(
    task_id: String,
    kind: String,
    reason: String,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_worker_followup_requested_control_message(task_id, kind, reason, attempt_identity)
}

pub fn worker_idle_mirror(
    task_id: String,
    message: String,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_worker_idle_control_message(task_id, message, attempt_identity)
}

pub fn permission_requested_mirror(
    task_id: String,
    worker_name: Option<String>,
    tool_name: String,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_permission_requested_control_message(task_id, tool_name, worker_name, attempt_identity)
}

pub fn permission_resolved_mirror(
    task_id: String,
    worker_name: Option<String>,
    tool_name: String,
    decision: String,
    source: Option<String>,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_permission_resolved_control_message(
        task_id,
        tool_name,
        decision,
        source,
        None,
        worker_name,
        attempt_identity,
    )
}

pub fn permission_timed_out_mirror(
    task_id: String,
    worker_name: Option<String>,
    tool_name: String,
    timeout_ms: u64,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_permission_timed_out_control_message(
        task_id,
        tool_name,
        timeout_ms,
        worker_name,
        attempt_identity,
    )
}

pub fn worker_finished_mirror(
    task_id: String,
    kind: TaskKind,
    status: &str,
    summary: String,
    produced_delta: bool,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_worker_finished_control_message(
        task_id,
        kind,
        status,
        summary,
        produced_delta,
        attempt_identity,
    )
}

pub fn tool_started_mirror(request_id: String, tool_name: String) -> HarnessControlMessage {
    build_tool_started_control_message(request_id, tool_name)
}

pub fn tool_finished_mirror(
    request_id: String,
    tool_name: String,
    success: bool,
    content: String,
    duration_ms: Option<u64>,
) -> HarnessControlMessage {
    build_tool_finished_control_message(request_id, tool_name, success, Some(content), duration_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_only_block_extracts_tool_names() {
        let text =
            "<turn_tool_gate mode=\"allow_only\">\n  developer__shell\n  developer__rg\n</turn_tool_gate>";
        let parsed = parse_turn_tool_gate_allow_only(Some(text)).unwrap();
        assert!(parsed.contains("developer__shell"));
        assert!(parsed.contains("developer__rg"));
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn allow_only_block_returns_none_when_empty() {
        let text = "<turn_tool_gate mode=\"allow_only\">\n</turn_tool_gate>";
        assert!(parse_turn_tool_gate_allow_only(Some(text)).is_none());
    }

    #[test]
    fn explicit_delegation_request_recognises_chinese_phrases() {
        assert!(has_explicit_delegation_request("请并行处理"));
        assert!(has_explicit_delegation_request("Use a swarm of workers"));
        assert!(!has_explicit_delegation_request("just the regular flow"));
    }

    #[test]
    fn harness_mode_routes_session_sources() {
        assert_eq!(
            harness_mode_for_session_source("system"),
            HarnessMode::Execute
        );
        assert_eq!(
            harness_mode_for_session_source("channel_conversation"),
            HarnessMode::Conversation
        );
        assert_eq!(
            harness_mode_for_session_source("chat"),
            HarnessMode::Conversation
        );
    }

    #[test]
    fn delegation_mode_maps_execution_mode() {
        assert_eq!(
            delegation_mode_for_execution_mode(CoordinatorExecutionMode::SingleWorker),
            DelegationMode::Subagent
        );
        assert_eq!(
            delegation_mode_for_execution_mode(CoordinatorExecutionMode::AutoSwarm),
            DelegationMode::Swarm
        );
    }

    #[test]
    fn task_kind_label_emits_expected_strings() {
        assert_eq!(task_kind_label(TaskKind::Subagent), "subagent");
        assert_eq!(task_kind_label(TaskKind::SwarmWorker), "swarm_worker");
        assert_eq!(
            task_kind_label(TaskKind::ValidationWorker),
            "validation_worker"
        );
    }

    #[test]
    fn sanitize_strips_diagnostic_lines_and_next_steps() {
        let raw = "\
Hello.\n\
Validation: passed\n\
Next steps:\n\
- step 1\n\
- step 2\n\
\n\
Final note.";
        let cleaned = sanitize_user_visible_runtime_text(Some(raw)).unwrap();
        assert!(cleaned.contains("Hello."));
        assert!(cleaned.contains("Final note."));
        assert!(!cleaned.contains("Validation:"));
        assert!(!cleaned.contains("step 1"));
    }
}
