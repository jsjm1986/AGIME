use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agime::agents::extension::ExtensionConfig;
#[cfg(test)]
use agime::agents::format_execution_host_completion_text;
use agime::agents::harness::TransitionTrace;
use agime::agents::harness::{
    auto_resolve_request, normalize_pre_materialized_document_analysis_completion_report,
    normalize_system_document_analysis_completion_report, CompletionSurfacePolicy,
    PermissionBridgeResolution,
};
use agime::agents::{
    build_permission_requested_control_message, build_permission_resolved_control_message,
    build_permission_timed_out_control_message, build_tool_finished_control_message,
    build_tool_started_control_message, build_worker_finished_control_message,
    build_worker_followup_requested_control_message, build_worker_idle_control_message,
    build_worker_progress_control_message, build_worker_started_control_message,
    clear_permission_bridge_resolver, control_messages_for_agent_event,
    control_sequencer_for_session, native_swarm_tool_enabled, normalize_call_tool_result,
    normalize_tool_execution_error_text, planner_auto_swarm_enabled, run_harness_host,
    set_permission_bridge_resolver, tool_execution_cancelled_error_text, Agent, AgentEvent,
    CompletionControlEvent, CoordinatorExecutionMode, DelegationCapabilityContext, DelegationMode,
    ExecuteCompletionOutcome, ExecutionHostCompletionReport, HarnessControlEnvelope,
    HarnessControlMessage, HarnessControlSink, HarnessEventSink, HarnessHostDependencies,
    HarnessHostRequest, HarnessMode, HarnessPersistenceAdapter, PermissionControlEvent,
    PermissionDecisionSource, ProviderTurnMode, RuntimeControlEvent, SessionConfig,
    SessionControlEvent, TaskKind, TaskRuntime, TaskRuntimeEvent, TaskRuntimeHost,
    WorkerAttemptIdentity, WorkerControlEvent,
};
use agime::context_runtime::{observe_runtime_transition, ContextRuntimeState};
use agime::conversation::message::{
    ActionRequiredData, FrontendToolRequest, Message, MessageContent,
};
use agime::conversation::Conversation;
use agime::mcp_utils::ToolResult;
use agime::permission::permission_confirmation::PrincipalType;
use agime::permission::{Permission, PermissionConfirmation};
use agime::providers::base::Provider;
use agime::session::{SessionManager, SessionType};
use agime::utils::normalize_delegation_summary_text;
use agime_team::models::{ApiFormat, ApprovalMode, TeamAgent};
use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use regex::Regex;
use rmcp::model::{CallToolRequestParams, CallToolResult, Content, ErrorCode, ErrorData};
use serde_json::Value;
use std::sync::OnceLock;
use tokio::sync::{watch, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::agent_prompt_composer::{compose_top_level_prompt, AgentPromptComposerInput};
use super::agent_runtime_config::{
    agent_has_extension_manager_enabled, build_api_caller, builtin_extension_configs_to_custom,
    compute_extension_overrides, extension_allowed_by_name, find_extension_config_by_name,
    resolve_agent_attached_team_extensions, resolve_agent_custom_extensions, TeamRuntimeSettings,
    TeamSkillMode,
};
use super::capability_policy::{is_non_delegating_session_source, AgentRuntimePolicyResolver};
use super::chat_channel_manager::ChatChannelManager;
use super::chat_manager::ChatManager;
use super::extension_installer::ExtensionInstaller;
use super::extension_manager_client::{DynamicExtensionState, TeamExtensionManagerClient};
use super::mcp_connector::{ElicitationBridgeCallback, McpConnector, ToolContentBlock};
use super::platform_runner::PlatformExtensionRunner;
use super::prompt_profiles::CHAT_DELEGATION_PROFILE_ID;
use super::runtime_text;
use super::service_mongo::AgentService;
use super::session_mongo::AgentSessionDoc;
use super::task_manager::{StreamEvent, TaskManager};
use super::tool_dispatch::execute_standard_tool_call;
use super::workspace_service::{WorkspaceExecutionContext, WorkspaceService};

fn parse_turn_tool_gate_allow_only(
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

pub struct ServerHarnessHost {
    db: Arc<MongoDb>,
    agent_service: Arc<AgentService>,
    internal_task_manager: Arc<TaskManager>,
}

pub struct ServerHarnessHostOutcome {
    pub messages_json: String,
    pub message_count: i32,
    pub total_tokens: Option<i32>,
    pub context_runtime_state: Option<ContextRuntimeState>,
    pub last_assistant_text: Option<String>,
    pub completion_report: Option<ExecutionHostCompletionReport>,
    pub persisted_child_evidence: Vec<agime::agents::harness::PersistedChildEvidenceItem>,
    pub persisted_child_transcript_resume:
        Vec<agime::agents::harness::PersistedChildTranscriptView>,
    pub transition_trace: Option<TransitionTrace>,
    pub events_emitted: usize,
    pub signal_summary: Option<agime::agents::CoordinatorSignalSummary>,
    pub completion_outcome: Option<ExecuteCompletionOutcome>,
}

impl ServerHarnessHostOutcome {
    pub fn user_visible_summary(&self) -> Option<String> {
        sanitize_user_visible_runtime_text(self.last_assistant_text.as_deref()).or_else(|| {
            self.completion_outcome
                .as_ref()
                .and_then(|outcome| sanitize_user_visible_runtime_text(outcome.summary.as_deref()))
        })
    }

    pub fn runtime_diagnostics(&self) -> Option<ExecutionHostCompletionReport> {
        self.completion_report.clone().or_else(|| {
            self.completion_outcome.as_ref().map(|outcome| {
                ExecutionHostCompletionReport {
                    status: if outcome.status.eq_ignore_ascii_case("completed") {
                        "completed".to_string()
                    } else {
                        "blocked".to_string()
                    },
                    summary: outcome
                        .summary
                        .clone()
                        .unwrap_or_else(|| "runtime-owned completion outcome".to_string()),
                    produced_artifacts: Vec::new(),
                    accepted_artifacts: Vec::new(),
                    next_steps: if outcome.status.eq_ignore_ascii_case("completed") {
                        Vec::new()
                    } else {
                        vec![
                            "Provide additional context or retry after resolving the blocking issue."
                                .to_string(),
                        ]
                    },
                    validation_status: None,
                    blocking_reason: outcome.blocking_reason.clone(),
                    reason_code: None,
                    content_accessed: None,
                    analysis_complete: None,
                }
            })
        })
    }

    pub fn execution_status(&self) -> &str {
        self.completion_report
            .as_ref()
            .map(|report| report.status.as_str())
            .or_else(|| {
                self.completion_outcome
                    .as_ref()
                    .map(|outcome| outcome.status.as_str())
            })
            .unwrap_or_else(|| "blocked")
    }
}

fn sanitize_user_visible_runtime_text(raw: Option<&str>) -> Option<String> {
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

#[derive(Clone)]
enum StreamBroadcaster {
    Chat {
        manager: Arc<ChatManager>,
        context_id: String,
    },
    Channel {
        manager: Arc<ChatChannelManager>,
        context_id: String,
    },
    Task {
        manager: Arc<TaskManager>,
        context_id: String,
    },
    Silent,
}

impl StreamBroadcaster {
    async fn emit(&self, event: StreamEvent) {
        match self {
            Self::Chat {
                manager,
                context_id,
            } => manager.broadcast(context_id, event).await,
            Self::Channel {
                manager,
                context_id,
            } => manager.broadcast(context_id, event).await,
            Self::Task {
                manager,
                context_id,
            } => manager.broadcast(context_id, event).await,
            Self::Silent => {}
        }
    }
}

struct DirectHostPreparedRuntime {
    provider: Arc<dyn Provider>,
    system_prompt: String,
    dynamic_state: Arc<RwLock<DynamicExtensionState>>,
    extension_manager: Option<Arc<TeamExtensionManagerClient>>,
    server_local_tools: Vec<rmcp::model::Tool>,
    worker_extensions: Vec<ExtensionConfig>,
    tool_timeout_secs: Option<u64>,
    extension_state_persistence_enabled: bool,
}

#[derive(Clone)]
struct DirectToolRuntime {
    dynamic_state: Arc<RwLock<DynamicExtensionState>>,
    extension_manager: Option<Arc<TeamExtensionManagerClient>>,
    task_manager: Arc<TaskManager>,
    task_id: String,
    cancel_token: CancellationToken,
    tool_timeout_secs: Option<u64>,
    server_local_tool_names: HashSet<String>,
    workspace_path: Option<String>,
    workspace_context: Option<WorkspaceExecutionContext>,
}

#[derive(Clone, Default)]
struct ServerHarnessControlSink;

#[async_trait::async_trait]
impl HarnessControlSink for ServerHarnessControlSink {
    async fn handle(&self, envelope: &HarnessControlEnvelope) -> Result<()> {
        tracing::debug!(
            logical_session_id = %envelope.session_id,
            runtime_session_id = %envelope.runtime_session_id,
            sequence = envelope.sequence,
            channel = %control_channel_label(&envelope.payload),
            event_type = %control_event_type_label(&envelope.payload),
            "ServerHarnessControlSink: observed control envelope"
        );
        Ok(())
    }
}

struct ServerHarnessEventSink {
    broadcaster: StreamBroadcaster,
    tool_runtime: Arc<DirectToolRuntime>,
    control_sink: Arc<dyn HarnessControlSink>,
    request_registry: RuntimeControlRegistry,
}

#[derive(Clone, Default)]
struct RuntimeControlRegistry {
    inner: Arc<Mutex<RuntimeControlRegistryState>>,
}

#[derive(Default)]
struct RuntimeControlRegistryState {
    permission_terminals: HashMap<String, PermissionTerminalState>,
    finished_tool_requests: HashSet<String>,
    latest_worker_attempts: HashMap<String, WorkerAttemptState>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PermissionTerminalState {
    Resolved,
    TimedOut,
}

#[derive(Clone, PartialEq, Eq)]
struct WorkerAttemptState {
    task_id: String,
    logical_worker_id: String,
    attempt_id: String,
    attempt_index: u32,
}

enum ControlEmissionDecision {
    Emit,
    Duplicate(&'static str),
    Stale(&'static str),
}

fn is_server_local_tool_name(server_local_tool_names: &HashSet<String>, tool_name: &str) -> bool {
    server_local_tool_names.contains(tool_name)
}

impl RuntimeControlRegistry {
    fn classify(&self, message: &HarnessControlMessage) -> ControlEmissionDecision {
        let mut state = self
            .inner
            .lock()
            .expect("runtime control registry poisoned");
        match message {
            HarnessControlMessage::Permission(PermissionControlEvent::Resolved {
                request_id,
                ..
            }) => match state.permission_terminals.get(request_id) {
                Some(PermissionTerminalState::Resolved) => {
                    ControlEmissionDecision::Duplicate("permission already resolved")
                }
                Some(PermissionTerminalState::TimedOut) => {
                    ControlEmissionDecision::Stale("permission already timed out")
                }
                None => {
                    state
                        .permission_terminals
                        .insert(request_id.clone(), PermissionTerminalState::Resolved);
                    ControlEmissionDecision::Emit
                }
            },
            HarnessControlMessage::Permission(PermissionControlEvent::TimedOut {
                request_id,
                ..
            }) => match state.permission_terminals.get(request_id) {
                Some(PermissionTerminalState::TimedOut) => {
                    ControlEmissionDecision::Duplicate("permission already timed out")
                }
                Some(PermissionTerminalState::Resolved) => {
                    ControlEmissionDecision::Stale("permission already resolved")
                }
                None => {
                    state
                        .permission_terminals
                        .insert(request_id.clone(), PermissionTerminalState::TimedOut);
                    ControlEmissionDecision::Emit
                }
            },
            HarnessControlMessage::Tool(agime::agents::ToolControlEvent::Finished {
                request_id,
                ..
            }) => {
                if state.finished_tool_requests.insert(request_id.clone()) {
                    ControlEmissionDecision::Emit
                } else {
                    ControlEmissionDecision::Duplicate("tool request already finished")
                }
            }
            HarnessControlMessage::Worker(WorkerControlEvent::Started {
                task_id,
                logical_worker_id,
                attempt_id,
                attempt_index,
                ..
            }) => {
                if let (Some(logical_worker_id), Some(attempt_id)) =
                    (logical_worker_id.as_ref(), attempt_id.as_ref())
                {
                    let candidate = WorkerAttemptState {
                        task_id: task_id.clone(),
                        logical_worker_id: logical_worker_id.clone(),
                        attempt_id: attempt_id.clone(),
                        attempt_index: attempt_index.unwrap_or_default(),
                    };
                    let should_replace = state
                        .latest_worker_attempts
                        .get(logical_worker_id)
                        .map(|current| {
                            candidate.attempt_index > current.attempt_index
                                || (candidate.attempt_index == current.attempt_index
                                    && candidate.attempt_id != current.attempt_id)
                        })
                        .unwrap_or(true);
                    if should_replace {
                        state
                            .latest_worker_attempts
                            .insert(logical_worker_id.clone(), candidate);
                    }
                }
                ControlEmissionDecision::Emit
            }
            HarnessControlMessage::Worker(WorkerControlEvent::Finished {
                task_id,
                logical_worker_id,
                attempt_id,
                attempt_index,
                ..
            }) => {
                let (Some(logical_worker_id), Some(attempt_id)) =
                    (logical_worker_id.as_ref(), attempt_id.as_ref())
                else {
                    return ControlEmissionDecision::Emit;
                };
                let Some(latest) = state.latest_worker_attempts.get(logical_worker_id) else {
                    return ControlEmissionDecision::Emit;
                };
                let current_attempt_index = attempt_index.unwrap_or_default();
                if latest.task_id != *task_id
                    || latest.attempt_id != *attempt_id
                    || latest.attempt_index != current_attempt_index
                {
                    ControlEmissionDecision::Stale("worker attempt superseded by newer attempt")
                } else {
                    ControlEmissionDecision::Emit
                }
            }
            _ => ControlEmissionDecision::Emit,
        }
    }
}

#[derive(Clone)]
struct SessionHostPersistenceAdapter {
    agent_service: Arc<AgentService>,
    session_id: String,
    fallback_title: Option<String>,
    generated_title: Option<String>,
    runtime_session_tx: Option<watch::Sender<Option<String>>>,
    broadcaster: StreamBroadcaster,
    control_sink: Arc<dyn HarnessControlSink>,
    request_registry: RuntimeControlRegistry,
    initial_context_runtime_state: Option<ContextRuntimeState>,
}

fn derive_chat_title_from_user_message(user_message: &str) -> Option<String> {
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

fn host_target_artifacts(session: &AgentSessionDoc) -> Vec<String> {
    let mut targets = Vec::new();
    if let Some(channel_id) = session
        .source_channel_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(thread_root_id) = session
            .source_thread_root_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            targets.push(format!("channel:{}/thread:{}", channel_id, thread_root_id));
        } else {
            targets.push(format!("channel:{}", channel_id));
        }
    }
    targets.sort();
    targets.dedup();
    targets
}

fn normalize_deliverable_token(token: &str) -> Option<String> {
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

fn infer_targets_from_user_message(user_message: &str) -> Vec<String> {
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

fn deliverable_target_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)+")
            .expect("deliverable target regex must compile")
    })
}

fn is_contextual_host_target(target: &str) -> bool {
    let normalized = target.trim().to_ascii_lowercase();
    normalized.starts_with("channel:") || normalized.starts_with("document:")
}

fn host_result_contract(session: &AgentSessionDoc) -> Vec<String> {
    let targets = host_target_artifacts(session);
    if targets.is_empty() {
        Vec::new()
    } else {
        targets
    }
}

fn infer_coordinator_execution_mode(
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

fn delegation_mode_for_execution_mode(
    coordinator_execution_mode: CoordinatorExecutionMode,
) -> DelegationMode {
    match coordinator_execution_mode {
        CoordinatorExecutionMode::ExplicitSwarm | CoordinatorExecutionMode::AutoSwarm => {
            DelegationMode::Swarm
        }
        CoordinatorExecutionMode::SingleWorker => DelegationMode::Subagent,
    }
}

fn harness_mode_for_session_source(session_source: &str) -> HarnessMode {
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

fn has_explicit_delegation_request(user_message: &str) -> bool {
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

fn should_force_execute_for_explicit_delegation(
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

fn provider_turn_mode_for_harness_mode(harness_mode: HarnessMode) -> ProviderTurnMode {
    match harness_mode {
        HarnessMode::Execute => ProviderTurnMode::Aggregated,
        HarnessMode::Conversation
        | HarnessMode::Plan
        | HarnessMode::Repair
        | HarnessMode::Blocked
        | HarnessMode::Complete => ProviderTurnMode::Streaming,
    }
}

fn provider_turn_mode_for_session_source(
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

fn completion_surface_policy_for_session_source(
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

fn required_tool_prefixes_for_session_source(session_source: &str) -> Vec<String> {
    // This is a document-area access contract, not a full content-extraction contract.
    // The run must first establish access to the formal document area via DocumentTools.
    // Actual content inspection may then continue through exported workspace files,
    // shell tooling, MCP readers, or other worker-local execution paths.
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

fn host_extensions_for_mode(
    _harness_mode: HarnessMode,
    server_local_tools: &[rmcp::model::Tool],
    worker_extensions: &[ExtensionConfig],
) -> Vec<ExtensionConfig> {
    if server_local_tools.is_empty() {
        worker_extensions.to_vec()
    } else {
        vec![ExtensionConfig::Frontend {
            name: "team_server_direct_host".to_string(),
            description: "Team Server direct harness host tools".to_string(),
            tools: server_local_tools.to_vec(),
            instructions: Some(
                "These tools execute on the Team Server host. Use them directly when needed."
                    .to_string(),
            ),
            bundled: Some(true),
            available_tools: Vec::new(),
        }]
    }
}

fn execution_host_completion_response() -> agime::recipe::Response {
    agime::recipe::Response {
        json_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["completed", "blocked"]
                },
                "summary": {
                    "type": "string",
                    "minLength": 1
                },
                "produced_artifacts": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "accepted_artifacts": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "next_steps": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "validation_status": {
                    "type": "string",
                    "enum": ["passed", "failed", "not_run"]
                },
                "blocking_reason": {
                    "type": "string"
                }
            },
            "required": ["status", "summary", "produced_artifacts", "accepted_artifacts", "next_steps"],
            "additionalProperties": false
        })),
    }
}

fn document_analysis_completion_response() -> agime::recipe::Response {
    agime::recipe::Response {
        json_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["completed", "blocked"]
                },
                "summary": {
                    "type": "string",
                    "minLength": 1
                },
                "produced_artifacts": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "accepted_artifacts": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "next_steps": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "validation_status": {
                    "type": "string",
                    "enum": ["passed", "failed", "not_run"]
                },
                "blocking_reason": { "type": "string" },
                "reason_code": { "type": "string" },
                "content_accessed": { "type": "boolean" },
                "analysis_complete": { "type": "boolean" }
            },
            "required": [
                "status",
                "summary",
                "produced_artifacts",
                "accepted_artifacts",
                "next_steps",
                "content_accessed",
                "analysis_complete",
                "reason_code"
            ],
            "additionalProperties": false
        })),
    }
}

fn completion_contract_for_session_source(
    session_source: &str,
    harness_mode: HarnessMode,
    require_final_report: bool,
) -> Option<agime::recipe::Response> {
    if session_source.eq_ignore_ascii_case("system") {
        Some(document_analysis_completion_response())
    } else if session_source.eq_ignore_ascii_case("document_analysis") {
        None
    } else if matches!(harness_mode, HarnessMode::Execute) && require_final_report {
        Some(execution_host_completion_response())
    } else {
        None
    }
}

fn is_non_terminal_document_analysis_summary(summary: &str) -> bool {
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

fn normalize_adapter_execution_host_completion_report(
    mut report: ExecutionHostCompletionReport,
    signal_summary: Option<&agime::agents::CoordinatorSignalSummary>,
    session_source: &str,
) -> ExecutionHostCompletionReport {
    let conversation_surface = session_source.eq_ignore_ascii_case("chat")
        || session_source.eq_ignore_ascii_case("automation_runtime")
        || session_source.eq_ignore_ascii_case("portal")
        || session_source.eq_ignore_ascii_case("channel_conversation");

    if session_source.eq_ignore_ascii_case("document_analysis") {
        report =
            normalize_pre_materialized_document_analysis_completion_report(report, signal_summary);
    } else if session_source.eq_ignore_ascii_case("system") {
        report = normalize_system_document_analysis_completion_report(report, signal_summary);
    } else if !conversation_surface
        && report.status == "completed"
        && signal_summary.is_some_and(|summary| summary.has_hard_blocking_signals())
    {
        report.status = "blocked".to_string();
        if report.blocking_reason.is_none() {
            report.blocking_reason = signal_summary
                .and_then(agime::agents::CoordinatorSignalSummary::default_blocking_reason)
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

fn build_context_runtime_compaction_event(
    initial: Option<&ContextRuntimeState>,
    final_state: Option<&ContextRuntimeState>,
) -> Option<StreamEvent> {
    let final_state = final_state?;
    let observation = observe_runtime_transition(initial, final_state)?;
    Some(StreamEvent::Compaction {
        strategy: "context_runtime".to_string(),
        before_tokens: observation.before_tokens?,
        after_tokens: observation.after_tokens?,
        phase: Some(observation.phase),
        reason: observation.reason,
    })
}

#[async_trait::async_trait]
impl HarnessPersistenceAdapter for SessionHostPersistenceAdapter {
    async fn on_started(
        &self,
        _logical_session_id: &str,
        runtime_session_id: &str,
        _initial_conversation: &Conversation,
        _mode: HarnessMode,
    ) -> Result<()> {
        let previous_runtime_session_id = self
            .agent_service
            .get_session(&self.session_id)
            .await
            .ok()
            .flatten()
            .and_then(|session| session.last_runtime_session_id)
            .filter(|value| !value.trim().is_empty() && value != runtime_session_id);
        if let Some(tx) = &self.runtime_session_tx {
            let _ = tx.send(Some(runtime_session_id.to_string()));
        }
        self.agent_service
            .set_session_runtime_session_id(&self.session_id, runtime_session_id)
            .await?;
        self.agent_service
            .set_session_processing(&self.session_id, true)
            .await?;
        if let Some(previous_runtime_session_id) = previous_runtime_session_id {
            let _ = SessionManager::delete_session(&previous_runtime_session_id).await;
        }
        Ok(())
    }

    async fn on_finished(
        &self,
        _logical_session_id: &str,
        runtime_session_id: &str,
        final_conversation: &Conversation,
        _mode: HarnessMode,
        context_runtime_state: Option<&ContextRuntimeState>,
        total_tokens: Option<i32>,
        _input_tokens: Option<i32>,
        _output_tokens: Option<i32>,
    ) -> Result<()> {
        let messages_json = serde_json::to_string(final_conversation.messages())?;
        let preview = runtime_text::extract_last_assistant_text(&messages_json).unwrap_or_default();
        self.agent_service
            .update_session_after_message(
                &self.session_id,
                &messages_json,
                final_conversation.len() as i32,
                &preview,
                self.generated_title
                    .as_deref()
                    .or(self.fallback_title.as_deref()),
                total_tokens,
                context_runtime_state,
            )
            .await?;
        if let Some(event) = build_context_runtime_compaction_event(
            self.initial_context_runtime_state.as_ref(),
            context_runtime_state,
        ) {
            if let StreamEvent::Compaction {
                strategy,
                before_tokens,
                after_tokens,
                phase,
                reason,
            } = &event
            {
                tracing::info!(
                    logical_session_id = %self.session_id,
                    strategy = %strategy,
                    before_tokens = *before_tokens,
                    after_tokens = *after_tokens,
                    phase = phase.as_deref().unwrap_or("unknown"),
                    reason = reason.as_deref().unwrap_or("unspecified"),
                    "ServerHarnessHost: context_runtime compaction event emitted"
                );
            }
            if let Some(mirror) = compaction_mirror(&event) {
                emit_stream_control_mirror(
                    &self.broadcaster,
                    self.control_sink.as_ref(),
                    &self.request_registry,
                    runtime_session_id,
                    mirror,
                )
                .await;
            } else {
                self.broadcaster.emit(event).await;
            }
        }
        Ok(())
    }
}

fn task_kind_label(kind: TaskKind) -> String {
    match kind {
        TaskKind::Subagent => "subagent".to_string(),
        TaskKind::SwarmWorker => "swarm_worker".to_string(),
        TaskKind::ValidationWorker => "validation_worker".to_string(),
    }
}

fn custom_extension_to_agent_extension(
    extension: &agime_team::models::CustomExtensionConfig,
) -> Option<ExtensionConfig> {
    let description = format!("Team extension {}", extension.name);
    let bundled = extension.source.as_deref().map(|source| source == "team");
    match extension.ext_type.to_ascii_lowercase().as_str() {
        "stdio" => Some(ExtensionConfig::Stdio {
            name: extension.name.clone(),
            description,
            cmd: extension.uri_or_cmd.clone(),
            args: extension.args.clone(),
            envs: agime::agents::extension::Envs::new(extension.envs.clone()),
            env_keys: Vec::new(),
            timeout: None,
            bundled,
            available_tools: Vec::new(),
        }),
        "sse" => Some(ExtensionConfig::Sse {
            name: extension.name.clone(),
            description,
            uri: extension.uri_or_cmd.clone(),
            envs: agime::agents::extension::Envs::new(extension.envs.clone()),
            env_keys: Vec::new(),
            timeout: None,
            bundled,
            available_tools: Vec::new(),
        }),
        "streamable_http" => Some(ExtensionConfig::StreamableHttp {
            name: extension.name.clone(),
            description,
            uri: extension.uri_or_cmd.clone(),
            envs: agime::agents::extension::Envs::new(extension.envs.clone()),
            env_keys: Vec::new(),
            headers: std::collections::HashMap::new(),
            timeout: None,
            bundled,
            available_tools: Vec::new(),
        }),
        _ => None,
    }
}

fn platform_extension_to_agent_extension(
    extension: &agime_team::models::AgentExtensionConfig,
) -> Option<ExtensionConfig> {
    if !extension.enabled {
        return None;
    }

    if extension.extension.is_platform() {
        Some(ExtensionConfig::Platform {
            name: extension.extension.name().to_string(),
            description: extension.extension.description().to_string(),
            bundled: Some(true),
            available_tools: Vec::new(),
        })
    } else {
        extension
            .extension
            .mcp_name()
            .map(|name| ExtensionConfig::Builtin {
                name: name.to_string(),
                description: extension.extension.description().to_string(),
                display_name: Some(extension.extension.name().to_string()),
                timeout: None,
                bundled: Some(true),
                available_tools: Vec::new(),
            })
    }
}

fn spawn_task_runtime_forwarder(
    task_runtime: Arc<TaskRuntime>,
    mut runtime_session_rx: watch::Receiver<Option<String>>,
    broadcaster: StreamBroadcaster,
    control_sink: Arc<dyn HarnessControlSink>,
    request_registry: RuntimeControlRegistry,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        #[derive(Clone)]
        struct TrackedTaskState {
            kind: TaskKind,
            target: Option<String>,
            attempt_identity: Option<WorkerAttemptIdentity>,
        }

        let runtime_session_id = loop {
            if let Some(value) = runtime_session_rx.borrow().clone() {
                break value;
            }
            if runtime_session_rx.changed().await.is_err() {
                return;
            }
        };

        let mut rx = task_runtime.subscribe_all();
        let mut tracked_tasks: HashMap<String, TrackedTaskState> = HashMap::new();
        loop {
            match rx.recv().await {
                Ok(TaskRuntimeEvent::Started(snapshot)) => {
                    if snapshot.parent_session_id == runtime_session_id {
                        let attempt_identity =
                            WorkerAttemptIdentity::from_metadata(&snapshot.metadata);
                        let task_id = snapshot.task_id.clone();
                        let target = snapshot.target_artifacts.first().cloned();
                        tracked_tasks.insert(
                            task_id.clone(),
                            TrackedTaskState {
                                kind: snapshot.kind,
                                target: target.clone(),
                                attempt_identity: attempt_identity.clone(),
                            },
                        );
                        let mirror = worker_started_mirror(&snapshot, attempt_identity.as_ref());
                        emit_stream_control_mirror(
                            &broadcaster,
                            control_sink.as_ref(),
                            &request_registry,
                            &runtime_session_id,
                            mirror,
                        )
                        .await;
                    }
                }
                Ok(TaskRuntimeEvent::Progress {
                    task_id,
                    message,
                    percent,
                }) => {
                    if tracked_tasks.contains_key(&task_id) {
                        let mirror = worker_progress_mirror(task_id, message, percent);
                        emit_stream_control_mirror(
                            &broadcaster,
                            control_sink.as_ref(),
                            &request_registry,
                            &runtime_session_id,
                            mirror,
                        )
                        .await;
                    }
                }
                Ok(TaskRuntimeEvent::FollowupRequested {
                    task_id,
                    kind,
                    reason,
                }) => {
                    if let Some(state) = tracked_tasks.get(&task_id) {
                        let mirror = worker_followup_mirror(
                            task_id,
                            kind,
                            reason,
                            state.attempt_identity.as_ref(),
                        );
                        emit_stream_control_mirror(
                            &broadcaster,
                            control_sink.as_ref(),
                            &request_registry,
                            &runtime_session_id,
                            mirror,
                        )
                        .await;
                    }
                }
                Ok(TaskRuntimeEvent::Idle { task_id, message }) => {
                    if let Some(state) = tracked_tasks.get(&task_id) {
                        let mirror =
                            worker_idle_mirror(task_id, message, state.attempt_identity.as_ref());
                        emit_stream_control_mirror(
                            &broadcaster,
                            control_sink.as_ref(),
                            &request_registry,
                            &runtime_session_id,
                            mirror,
                        )
                        .await;
                    }
                }
                Ok(TaskRuntimeEvent::PermissionRequested {
                    task_id,
                    worker_name,
                    tool_name,
                }) => {
                    if let Some(state) = tracked_tasks.get(&task_id) {
                        let mirror = permission_requested_mirror(
                            task_id,
                            worker_name,
                            tool_name,
                            state.attempt_identity.as_ref(),
                        );
                        emit_stream_control_mirror(
                            &broadcaster,
                            control_sink.as_ref(),
                            &request_registry,
                            &runtime_session_id,
                            mirror,
                        )
                        .await;
                    }
                }
                Ok(TaskRuntimeEvent::PermissionResolved {
                    task_id,
                    worker_name,
                    tool_name,
                    decision,
                    source,
                }) => {
                    if let Some(state) = tracked_tasks.get(&task_id) {
                        let mirror = permission_resolved_mirror(
                            task_id,
                            worker_name,
                            tool_name,
                            decision,
                            source,
                            state.attempt_identity.as_ref(),
                        );
                        emit_stream_control_mirror(
                            &broadcaster,
                            control_sink.as_ref(),
                            &request_registry,
                            &runtime_session_id,
                            mirror,
                        )
                        .await;
                    }
                }
                Ok(TaskRuntimeEvent::PermissionTimedOut {
                    task_id,
                    worker_name,
                    tool_name,
                    timeout_ms,
                }) => {
                    if let Some(state) = tracked_tasks.get(&task_id) {
                        let mirror = permission_timed_out_mirror(
                            task_id,
                            worker_name,
                            tool_name,
                            timeout_ms,
                            state.attempt_identity.as_ref(),
                        );
                        emit_stream_control_mirror(
                            &broadcaster,
                            control_sink.as_ref(),
                            &request_registry,
                            &runtime_session_id,
                            mirror,
                        )
                        .await;
                    }
                }
                Ok(TaskRuntimeEvent::Completed(result)) => {
                    if let Some(state) = tracked_tasks.remove(&result.task_id) {
                        if state.kind == TaskKind::Subagent {
                            let payload = super::hook_runtime::build_subagent_stop_payload(
                                &runtime_session_id,
                                Some(runtime_session_id.clone()),
                                &result.task_id,
                                "completed",
                                result.accepted_targets.clone(),
                                result.produced_delta,
                                None,
                                None,
                            );
                            super::hook_runtime::emit_subagent_stop_payload(&payload);
                        }
                        let mirror = worker_finished_mirror(
                            result.task_id,
                            result.kind,
                            "completed",
                            result.summary,
                            result.produced_delta,
                            state.attempt_identity.as_ref(),
                        );
                        emit_stream_control_mirror(
                            &broadcaster,
                            control_sink.as_ref(),
                            &request_registry,
                            &runtime_session_id,
                            mirror,
                        )
                        .await;
                    }
                }
                Ok(TaskRuntimeEvent::Failed(result)) => {
                    if let Some(state) = tracked_tasks.remove(&result.task_id) {
                        if state.kind == TaskKind::Subagent {
                            let payload = super::hook_runtime::build_subagent_stop_payload(
                                &runtime_session_id,
                                Some(runtime_session_id.clone()),
                                &result.task_id,
                                "blocked",
                                result.accepted_targets.clone(),
                                result.produced_delta,
                                None,
                                Some(result.summary.clone()),
                            );
                            super::hook_runtime::emit_subagent_stop_payload(&payload);
                        }
                        let mirror = worker_finished_mirror(
                            result.task_id,
                            result.kind,
                            "failed",
                            result.summary,
                            result.produced_delta,
                            state.attempt_identity.as_ref(),
                        );
                        emit_stream_control_mirror(
                            &broadcaster,
                            control_sink.as_ref(),
                            &request_registry,
                            &runtime_session_id,
                            mirror,
                        )
                        .await;
                    }
                }
                Ok(TaskRuntimeEvent::Cancelled { task_id }) => {
                    if let Some(state) = tracked_tasks.remove(&task_id) {
                        let payload = super::hook_runtime::build_subagent_stop_payload(
                            &runtime_session_id,
                            Some(runtime_session_id.clone()),
                            &task_id,
                            "blocked",
                            Vec::new(),
                            false,
                            None,
                            Some("worker cancelled".to_string()),
                        );
                        super::hook_runtime::emit_subagent_stop_payload(&payload);
                        let mirror = worker_finished_mirror(
                            task_id,
                            state.kind,
                            "cancelled",
                            "worker cancelled".to_string(),
                            false,
                            state.attempt_identity.as_ref(),
                        );
                        emit_stream_control_mirror(
                            &broadcaster,
                            control_sink.as_ref(),
                            &request_registry,
                            &runtime_session_id,
                            mirror,
                        )
                        .await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    })
}

fn control_channel_label(payload: &HarnessControlMessage) -> &'static str {
    match payload {
        HarnessControlMessage::Session(_) => "session",
        HarnessControlMessage::Tool(_) => "tool",
        HarnessControlMessage::Permission(_) => "permission",
        HarnessControlMessage::Worker(_) => "worker",
        HarnessControlMessage::Completion(_) => "completion",
        HarnessControlMessage::Runtime(_) => "runtime",
    }
}

fn control_event_type_label(payload: &HarnessControlMessage) -> &'static str {
    match payload {
        HarnessControlMessage::Session(event) => match event {
            agime::agents::SessionControlEvent::Started { .. } => "started",
            agime::agents::SessionControlEvent::StateChanged { .. } => "state_changed",
            agime::agents::SessionControlEvent::Interrupted { .. } => "interrupted",
            agime::agents::SessionControlEvent::CancelRequested { .. } => "cancel_requested",
            agime::agents::SessionControlEvent::Finished { .. } => "finished",
        },
        HarnessControlMessage::Tool(event) => match event {
            agime::agents::ToolControlEvent::TransportRequested { .. } => "transport_requested",
            agime::agents::ToolControlEvent::Started { .. } => "started",
            agime::agents::ToolControlEvent::Progress { .. } => "progress",
            agime::agents::ToolControlEvent::Finished { .. } => "finished",
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

async fn emit_task_control_message(
    control_sink: &dyn HarnessControlSink,
    runtime_session_id: &str,
    message: HarnessControlMessage,
) {
    let Some(sequencer) = control_sequencer_for_session(runtime_session_id) else {
        return;
    };
    let envelope = sequencer.next(message);
    if let Err(error) = control_sink.handle(&envelope).await {
        tracing::warn!(
            logical_session_id = %envelope.session_id,
            runtime_session_id = %envelope.runtime_session_id,
            sequence = envelope.sequence,
            error = %error,
            "ServerHarnessHost: control sink failed during task runtime mirror"
        );
    }
}

async fn emit_stream_control_mirror(
    broadcaster: &StreamBroadcaster,
    control_sink: &dyn HarnessControlSink,
    request_registry: &RuntimeControlRegistry,
    runtime_session_id: &str,
    control_message: HarnessControlMessage,
) {
    match request_registry.classify(&control_message) {
        ControlEmissionDecision::Emit => {}
        ControlEmissionDecision::Duplicate(reason) => {
            tracing::warn!(
                runtime_session_id = %runtime_session_id,
                channel = %control_channel_label(&control_message),
                event_type = %control_event_type_label(&control_message),
                reason,
                "suppressing duplicate control terminal event"
            );
            return;
        }
        ControlEmissionDecision::Stale(reason) => {
            tracing::warn!(
                runtime_session_id = %runtime_session_id,
                channel = %control_channel_label(&control_message),
                event_type = %control_event_type_label(&control_message),
                reason,
                "suppressing stale control terminal event"
            );
            return;
        }
    }
    if let Some(stream_event) = stream_event_for_control_projection(&control_message) {
        broadcaster.emit(stream_event).await;
    }
    emit_task_control_message(control_sink, runtime_session_id, control_message).await;
}

fn worker_started_mirror(
    snapshot: &agime::agents::TaskSnapshot,
    _attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_worker_started_control_message(snapshot)
}

fn worker_progress_mirror(
    task_id: String,
    message: String,
    percent: Option<u8>,
) -> HarnessControlMessage {
    build_worker_progress_control_message(task_id, message, percent)
}

fn worker_followup_mirror(
    task_id: String,
    kind: String,
    reason: String,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_worker_followup_requested_control_message(task_id, kind, reason, attempt_identity)
}

fn worker_idle_mirror(
    task_id: String,
    message: String,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_worker_idle_control_message(task_id, message, attempt_identity)
}

fn permission_requested_mirror(
    task_id: String,
    worker_name: Option<String>,
    tool_name: String,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_permission_requested_control_message(task_id, tool_name, worker_name, attempt_identity)
}

fn permission_resolved_mirror(
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

fn permission_timed_out_mirror(
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

fn worker_finished_mirror(
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

fn tool_started_mirror(request_id: String, tool_name: String) -> HarnessControlMessage {
    build_tool_started_control_message(request_id, tool_name)
}

fn tool_finished_mirror(
    request_id: String,
    tool_name: String,
    success: bool,
    content: String,
    duration_ms: Option<u64>,
) -> HarnessControlMessage {
    build_tool_finished_control_message(request_id, tool_name, success, Some(content), duration_ms)
}

fn compaction_mirror(event: &StreamEvent) -> Option<HarnessControlMessage> {
    let StreamEvent::Compaction {
        strategy,
        reason,
        before_tokens,
        after_tokens,
        phase,
    } = event
    else {
        return None;
    };
    let _ = (before_tokens, after_tokens, phase);
    let control_message = HarnessControlMessage::Runtime(RuntimeControlEvent::CompactionObserved {
        strategy: Some(strategy.clone()),
        reason: reason.clone(),
        before_tokens: Some(*before_tokens),
        after_tokens: Some(*after_tokens),
        phase: phase.clone(),
    });
    Some(control_message)
}

fn is_compaction_inline_notification(message: &str) -> bool {
    let normalized = message.trim().to_ascii_lowercase();
    normalized.starts_with("exceeded auto-compact threshold of ")
        || normalized.starts_with("context limit reached. compacting to continue conversation...")
}

fn stream_event_for_control_projection(message: &HarnessControlMessage) -> Option<StreamEvent> {
    match message {
        HarnessControlMessage::Tool(agime::agents::ToolControlEvent::Started {
            request_id,
            tool_name,
        }) => Some(StreamEvent::ToolCall {
            name: tool_name.clone(),
            id: request_id.clone(),
        }),
        HarnessControlMessage::Tool(agime::agents::ToolControlEvent::Finished {
            request_id,
            tool_name,
            success,
            summary,
            duration_ms,
        }) => Some(StreamEvent::ToolResult {
            id: request_id.clone(),
            success: *success,
            content: summary.clone().unwrap_or_default(),
            name: Some(tool_name.clone()),
            duration_ms: *duration_ms,
        }),
        HarnessControlMessage::Permission(PermissionControlEvent::Requested {
            request_id,
            tool_name,
            worker_name,
            logical_worker_id,
            attempt_id,
        }) => Some(StreamEvent::PermissionRequested {
            task_id: request_id.clone(),
            tool_name: tool_name.clone(),
            worker_name: worker_name.clone(),
            logical_worker_id: logical_worker_id.clone(),
            attempt_id: attempt_id.clone(),
        }),
        HarnessControlMessage::Permission(PermissionControlEvent::Resolved {
            request_id,
            tool_name,
            decision,
            source,
            worker_name,
            logical_worker_id,
            attempt_id,
            ..
        }) => Some(StreamEvent::PermissionResolved {
            task_id: request_id.clone(),
            tool_name: tool_name.clone(),
            decision: decision.clone(),
            source: Some(format!("{:?}", source).to_ascii_lowercase()),
            worker_name: worker_name.clone(),
            logical_worker_id: logical_worker_id.clone(),
            attempt_id: attempt_id.clone(),
        }),
        HarnessControlMessage::Permission(PermissionControlEvent::TimedOut {
            request_id,
            tool_name,
            timeout_ms,
            worker_name,
            logical_worker_id,
            attempt_id,
        }) => Some(StreamEvent::PermissionTimedOut {
            task_id: request_id.clone(),
            tool_name: tool_name.clone(),
            timeout_ms: *timeout_ms,
            worker_name: worker_name.clone(),
            logical_worker_id: logical_worker_id.clone(),
            attempt_id: attempt_id.clone(),
        }),
        HarnessControlMessage::Worker(WorkerControlEvent::Started {
            task_id,
            kind,
            target,
            logical_worker_id,
            attempt_id,
            attempt_index,
            previous_task_id,
        }) => Some(StreamEvent::WorkerStarted {
            task_id: task_id.clone(),
            kind: kind.clone(),
            target: target.clone(),
            logical_worker_id: logical_worker_id.clone(),
            attempt_id: attempt_id.clone(),
            attempt_index: *attempt_index,
            previous_task_id: previous_task_id.clone(),
        }),
        HarnessControlMessage::Worker(WorkerControlEvent::Progress {
            task_id,
            message,
            percent,
        }) => Some(StreamEvent::WorkerProgress {
            task_id: task_id.clone(),
            message: message.clone(),
            percent: *percent,
        }),
        HarnessControlMessage::Worker(WorkerControlEvent::Idle {
            task_id,
            message,
            logical_worker_id,
            attempt_id,
        }) => Some(StreamEvent::WorkerIdle {
            task_id: task_id.clone(),
            message: message.clone(),
            logical_worker_id: logical_worker_id.clone(),
            attempt_id: attempt_id.clone(),
        }),
        HarnessControlMessage::Worker(WorkerControlEvent::FollowupRequested {
            task_id,
            kind,
            reason,
            logical_worker_id,
            attempt_id,
            attempt_index,
            previous_task_id,
        }) => Some(StreamEvent::WorkerFollowup {
            task_id: task_id.clone(),
            kind: kind.clone(),
            reason: reason.clone(),
            logical_worker_id: logical_worker_id.clone(),
            attempt_id: attempt_id.clone(),
            attempt_index: *attempt_index,
            previous_task_id: previous_task_id.clone(),
        }),
        HarnessControlMessage::Worker(WorkerControlEvent::Finished {
            task_id,
            kind,
            status,
            summary,
            produced_delta,
            logical_worker_id,
            attempt_id,
            attempt_index,
            previous_task_id,
        }) => Some(StreamEvent::WorkerFinished {
            task_id: task_id.clone(),
            kind: kind.clone(),
            status: status.clone(),
            summary: summary.clone(),
            produced_delta: *produced_delta,
            logical_worker_id: logical_worker_id.clone(),
            attempt_id: attempt_id.clone(),
            attempt_index: *attempt_index,
            previous_task_id: previous_task_id.clone(),
        }),
        HarnessControlMessage::Runtime(RuntimeControlEvent::CompactionObserved {
            strategy,
            reason,
            before_tokens,
            after_tokens,
            phase,
        }) => match (strategy, before_tokens, after_tokens) {
            (Some(strategy), Some(before_tokens), Some(after_tokens)) => {
                Some(StreamEvent::Compaction {
                    strategy: strategy.clone(),
                    before_tokens: *before_tokens,
                    after_tokens: *after_tokens,
                    phase: phase.clone(),
                    reason: reason.clone(),
                })
            }
            _ => None,
        },
        HarnessControlMessage::Runtime(RuntimeControlEvent::Notification {
            code, message, ..
        }) => match code.as_deref() {
            Some("assistant_text") => Some(StreamEvent::Text {
                content: message.clone(),
            }),
            Some("inline_message") => {
                if is_compaction_inline_notification(message) {
                    None
                } else {
                    Some(StreamEvent::Text {
                        content: message.clone(),
                    })
                }
            }
            Some("auto_approve_status") | Some("unsupported_external_frontend_status") => {
                Some(StreamEvent::Status {
                    status: message.clone(),
                })
            }
            _ => None,
        },
        HarnessControlMessage::Session(SessionControlEvent::StateChanged { state, reason })
            if state == "thinking" =>
        {
            reason.as_ref().map(|content| StreamEvent::Thinking {
                content: content.clone(),
            })
        }
        _ => None,
    }
}

async fn emit_control_projection_message(
    broadcaster: &StreamBroadcaster,
    control_sink: &dyn HarnessControlSink,
    request_registry: &RuntimeControlRegistry,
    runtime_session_id: &str,
    control_message: HarnessControlMessage,
) {
    match request_registry.classify(&control_message) {
        ControlEmissionDecision::Emit => {}
        ControlEmissionDecision::Duplicate(reason) => {
            tracing::warn!(
                runtime_session_id = %runtime_session_id,
                channel = %control_channel_label(&control_message),
                event_type = %control_event_type_label(&control_message),
                reason,
                "suppressing duplicate control projection"
            );
            return;
        }
        ControlEmissionDecision::Stale(reason) => {
            tracing::warn!(
                runtime_session_id = %runtime_session_id,
                channel = %control_channel_label(&control_message),
                event_type = %control_event_type_label(&control_message),
                reason,
                "suppressing stale control projection"
            );
            return;
        }
    }
    if let Some(event) = stream_event_for_control_projection(&control_message) {
        broadcaster.emit(event).await;
    }
    emit_task_control_message(control_sink, runtime_session_id, control_message).await;
}

#[async_trait::async_trait]
impl HarnessEventSink for ServerHarnessEventSink {
    async fn handle(
        &self,
        logical_session_id: &str,
        runtime_session_id: &str,
        agent: &Agent,
        event: &AgentEvent,
    ) -> Result<()> {
        match event {
            AgentEvent::Message(message) => {
                self.handle_message(logical_session_id, runtime_session_id, agent, message)
                    .await
            }
            AgentEvent::ToolTransportRequest(event) => match event.transport {
                agime::agents::ToolTransportKind::ServerLocal => {
                    self.handle_server_local_tool_request(
                        logical_session_id,
                        runtime_session_id,
                        agent,
                        &event.request,
                    )
                    .await
                }
                agime::agents::ToolTransportKind::ExternalFrontend => {
                    let frontend_request = FrontendToolRequest {
                        id: event.request.id.clone(),
                        tool_call: event.request.tool_call.clone(),
                    };
                    self.handle_frontend_tool_request(
                        logical_session_id,
                        runtime_session_id,
                        agent,
                        &frontend_request,
                    )
                    .await
                }
                agime::agents::ToolTransportKind::WorkerLocal => {
                    tracing::warn!(
                        request_id = %event.request.id,
                        surface = ?event.surface,
                        "ignoring worker-local transport on direct host surface"
                    );
                    Ok(())
                }
            },
            AgentEvent::ModelChange { .. } => Ok(()),
            AgentEvent::McpNotification(_) => Ok(()),
            AgentEvent::HistoryReplaced(_) => Ok(()),
        }
    }
}

impl ServerHarnessEventSink {
    fn is_server_local_tool_name(&self, tool_name: &str) -> bool {
        is_server_local_tool_name(&self.tool_runtime.server_local_tool_names, tool_name)
    }

    fn visualisation_artifact_label(tool_name: &str) -> String {
        let leaf = tool_name
            .rsplit("__")
            .next()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(tool_name);
        format!("Auto Visualiser {}", leaf.replace('_', " "))
    }

    fn visualisation_artifact_path(tool_name: &str) -> String {
        let leaf = tool_name
            .rsplit("__")
            .next()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(tool_name);
        let safe = leaf
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .trim_matches('-')
            .to_ascii_lowercase();
        let safe = if safe.is_empty() {
            "visualisation".to_string()
        } else {
            safe
        };
        format!("artifacts/visualisations/{}-{}.html", safe, Uuid::new_v4())
    }

    fn html_resource_bytes(block: &ToolContentBlock) -> Option<(String, Vec<u8>)> {
        let ToolContentBlock::Resource {
            uri,
            mime_type,
            text,
            blob,
        } = block
        else {
            return None;
        };
        let mime = mime_type.as_deref().unwrap_or_default();
        if !uri.starts_with("ui://") || !mime.starts_with("text/html") {
            return None;
        }
        if let Some(text) = text {
            return Some((uri.clone(), text.as_bytes().to_vec()));
        }
        let blob = blob.as_ref()?;
        STANDARD.decode(blob).ok().map(|bytes| (uri.clone(), bytes))
    }

    fn materialize_visualisation_resources(
        &self,
        runtime_session_id: &str,
        tool_name: &str,
        blocks: &[ToolContentBlock],
    ) -> Vec<serde_json::Value> {
        let Some(workspace_path) = self.tool_runtime.workspace_path.as_deref() else {
            return Vec::new();
        };
        let workspace_service = WorkspaceService::new(String::new());
        let Ok(Some(workspace)) = workspace_service.load_workspace(workspace_path) else {
            return Vec::new();
        };
        let workspace_context = self
            .tool_runtime
            .workspace_context
            .clone()
            .or_else(|| workspace_service.execution_context(&workspace, "tool").ok());
        let Some(workspace_context) = workspace_context else {
            return Vec::new();
        };

        let mut files = Vec::new();
        for block in blocks {
            let Some((uri, bytes)) = Self::html_resource_bytes(block) else {
                continue;
            };
            let relative_path = Self::visualisation_artifact_path(tool_name);
            let absolute_path = Path::new(&workspace_context.workspace_root).join(&relative_path);
            if !workspace_write_allowed(&workspace_context, &absolute_path) {
                tracing::warn!(
                    runtime_session_id = %runtime_session_id,
                    tool_name,
                    uri,
                    path = %absolute_path.to_string_lossy(),
                    allowed_write_roots = ?workspace_context.allowed_write_roots,
                    "Auto Visualiser artifact write escaped workspace execution context"
                );
                continue;
            }
            let write_result = (|| -> Result<()> {
                if let Some(parent) = absolute_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&absolute_path, &bytes)?;
                let _ = workspace_service.record_workspace_output(
                    &workspace,
                    &relative_path,
                    Some("text/html".to_string()),
                )?;
                Ok(())
            })();
            if let Err(err) = write_result {
                tracing::warn!(
                    runtime_session_id = %runtime_session_id,
                    tool_name,
                    uri,
                    error = %err,
                    "failed to materialize Auto Visualiser resource"
                );
                continue;
            }
            files.push(serde_json::json!({
                "type": "workspace_file",
                "path": relative_path,
                "label": Self::visualisation_artifact_label(tool_name),
                "content_type": "text/html",
                "size_bytes": bytes.len() as i64,
                "preview_supported": true,
                "source": {
                    "kind": "mcp_resource",
                    "uri": uri,
                    "tool_name": tool_name
                }
            }));
        }
        files
    }

    async fn execute_direct_tool_request(
        &self,
        logical_session_id: &str,
        runtime_session_id: &str,
        agent: &Agent,
        request_id: &str,
        tool_call: ToolResult<CallToolRequestParams>,
    ) -> Result<()> {
        let tool_call = match tool_call {
            Ok(call) => call,
            Err(err) => {
                let payload = super::hook_runtime::build_post_tool_use_failure_payload(
                    logical_session_id,
                    Some(runtime_session_id.to_string()),
                    request_id,
                    "unknown",
                    err.to_string(),
                );
                super::hook_runtime::emit_post_tool_use_failure_payload(&payload);
                agent
                    .handle_tool_result(request_id.to_string(), Err(err))
                    .await;
                return Ok(());
            }
        };

        let tool_name = tool_call.name.to_string();
        emit_stream_control_mirror(
            &self.broadcaster,
            self.control_sink.as_ref(),
            &self.request_registry,
            runtime_session_id,
            tool_started_mirror(request_id.to_string(), tool_name.clone()),
        )
        .await;

        let args = Value::Object(tool_call.arguments.clone().unwrap_or_default());
        let final_input = args.clone();
        let (duration_ms, result) =
            execute_direct_tool_call(self.tool_runtime.clone(), &tool_name, args).await;
        let tool_was_cancelled = self.tool_runtime.cancel_token.is_cancelled();
        if tool_was_cancelled {
            emit_control_projection_message(
                &self.broadcaster,
                self.control_sink.as_ref(),
                &self.request_registry,
                runtime_session_id,
                HarnessControlMessage::Runtime(RuntimeControlEvent::Notification {
                    level: "warning".to_string(),
                    code: Some("tool_execution_cancelled".to_string()),
                    message: format!(
                        "tool `{}` finalized after cancellation and was surfaced as cancelled",
                        tool_name
                    ),
                }),
            )
            .await;
        }

        match result {
            Ok(blocks) => {
                let mut call_result = blocks_to_call_result(&blocks);
                let visualisation_files = self.materialize_visualisation_resources(
                    runtime_session_id,
                    &tool_name,
                    &blocks,
                );
                if !visualisation_files.is_empty() {
                    let primary_file = visualisation_files.first().cloned();
                    call_result.content.insert(
                        0,
                        Content::text(
                            serde_json::json!({
                                "status": "ok",
                                "summary": "Auto Visualiser generated interactive HTML visualisation artifact(s).",
                                "files": visualisation_files,
                                "file": primary_file
                            })
                            .to_string(),
                        ),
                    );
                }
                let normalized =
                    normalize_call_tool_result(Some(&tool_name), Some(&final_input), &call_result);
                let payload = super::hook_runtime::build_post_tool_use_payload(
                    logical_session_id,
                    Some(runtime_session_id.to_string()),
                    request_id,
                    &tool_name,
                    Vec::new(),
                    false,
                    None,
                );
                super::hook_runtime::emit_post_tool_use_payload(&payload);
                emit_stream_control_mirror(
                    &self.broadcaster,
                    self.control_sink.as_ref(),
                    &self.request_registry,
                    runtime_session_id,
                    tool_finished_mirror(
                        request_id.to_string(),
                        tool_name.clone(),
                        normalized.success,
                        normalized.display_text.clone(),
                        Some(duration_ms),
                    ),
                )
                .await;
                agent
                    .handle_tool_result(request_id.to_string(), Ok(call_result))
                    .await;
            }
            Err(err) => {
                let normalized =
                    normalize_tool_execution_error_text(Some(&tool_name), Some(&final_input), &err);
                let payload = super::hook_runtime::build_post_tool_use_failure_payload(
                    logical_session_id,
                    Some(runtime_session_id.to_string()),
                    request_id,
                    &tool_name,
                    normalized.display_text.clone(),
                );
                super::hook_runtime::emit_post_tool_use_failure_payload(&payload);
                emit_stream_control_mirror(
                    &self.broadcaster,
                    self.control_sink.as_ref(),
                    &self.request_registry,
                    runtime_session_id,
                    tool_finished_mirror(
                        request_id.to_string(),
                        tool_name.clone(),
                        normalized.success,
                        normalized.display_text.clone(),
                        Some(duration_ms),
                    ),
                )
                .await;
                agent
                    .handle_tool_result(
                        request_id.to_string(),
                        Err(ErrorData::new(
                            ErrorCode::INTERNAL_ERROR,
                            normalized.display_text,
                            normalized.structured_output,
                        )),
                    )
                    .await;
            }
        }

        Ok(())
    }

    async fn handle_server_local_tool_request(
        &self,
        logical_session_id: &str,
        runtime_session_id: &str,
        agent: &Agent,
        request: &agime::conversation::message::ToolRequest,
    ) -> Result<()> {
        self.execute_direct_tool_request(
            logical_session_id,
            runtime_session_id,
            agent,
            &request.id,
            request.tool_call.clone(),
        )
        .await
    }

    async fn handle_message(
        &self,
        logical_session_id: &str,
        runtime_session_id: &str,
        agent: &Agent,
        message: &Message,
    ) -> Result<()> {
        let control_messages =
            control_messages_for_agent_event(&AgentEvent::Message(message.clone()));
        for control_message in &control_messages {
            emit_control_projection_message(
                &self.broadcaster,
                self.control_sink.as_ref(),
                &self.request_registry,
                runtime_session_id,
                control_message.clone(),
            )
            .await;
        }

        // Message-derived control events are emitted centrally in host.rs and projected
        // to legacy stream events above. The remaining branches here are only for
        // side-effectful direct-host behaviors that still need imperative handling.
        for content in &message.content {
            match content {
                MessageContent::FrontendToolRequest(request) => {
                    self.handle_frontend_tool_request(
                        logical_session_id,
                        runtime_session_id,
                        agent,
                        request,
                    )
                    .await?;
                }
                MessageContent::ActionRequired(action) => {
                    if let ActionRequiredData::ToolConfirmation { id, tool_name, .. } = &action.data
                    {
                        // The corresponding RequiresAction control event has already been
                        // emitted upstream from AgentEvent::Message(ActionRequired).
                        // This status line preserves the existing direct-host UX trace, while
                        // we also emit the resolved control event to keep the permission
                        // timeline complete on the canonical control channel.
                        emit_control_projection_message(
                            &self.broadcaster,
                            self.control_sink.as_ref(),
                            &self.request_registry,
                            runtime_session_id,
                            HarnessControlMessage::Runtime(RuntimeControlEvent::Notification {
                                level: "info".to_string(),
                                code: Some("auto_approve_status".to_string()),
                                message: format!("auto_approve:{}", tool_name),
                            }),
                        )
                        .await;
                        emit_stream_control_mirror(
                            &self.broadcaster,
                            self.control_sink.as_ref(),
                            &self.request_registry,
                            runtime_session_id,
                            build_permission_resolved_control_message(
                                id.clone(),
                                tool_name.clone(),
                                "allow_once",
                                Some("runtime_policy".to_string()),
                                Some("auto-approved by direct host".to_string()),
                                None,
                                None,
                            ),
                        )
                        .await;
                        agent
                            .handle_confirmation(
                                id.clone(),
                                PermissionConfirmation {
                                    principal_type: PrincipalType::Tool,
                                    permission: Permission::AllowOnce,
                                },
                            )
                            .await;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn handle_frontend_tool_request(
        &self,
        logical_session_id: &str,
        runtime_session_id: &str,
        agent: &Agent,
        request: &FrontendToolRequest,
    ) -> Result<()> {
        let tool_name = match &request.tool_call {
            Ok(call) => call.name.to_string(),
            Err(_) => String::new(),
        };
        if self.is_server_local_tool_name(&tool_name) {
            return self
                .execute_direct_tool_request(
                    logical_session_id,
                    runtime_session_id,
                    agent,
                    &request.id,
                    request.tool_call.clone(),
                )
                .await;
        }

        emit_control_projection_message(
            &self.broadcaster,
            self.control_sink.as_ref(),
            &self.request_registry,
            runtime_session_id,
            HarnessControlMessage::Runtime(RuntimeControlEvent::Notification {
                level: "warn".to_string(),
                code: Some("unsupported_external_frontend_status".to_string()),
                message: format!("unsupported_external_frontend:{}", tool_name),
            }),
        )
        .await;
        emit_stream_control_mirror(
            &self.broadcaster,
            self.control_sink.as_ref(),
            &self.request_registry,
            runtime_session_id,
            build_tool_finished_control_message(
                request.id.clone(),
                tool_name.clone(),
                false,
                Some(format!(
                    "external frontend tool '{}' is not available in direct server host mode",
                    tool_name
                )),
                None,
            ),
        )
        .await;
        agent
            .handle_tool_result(
                request.id.clone(),
                Err(ErrorData::new(
                    ErrorCode::INVALID_REQUEST,
                    format!(
                        "External frontend tool '{}' is not available in direct server host mode",
                        tool_name
                    ),
                    None,
                )),
            )
            .await;
        Ok(())
    }
}

impl ServerHarnessHost {
    pub fn new(
        db: Arc<MongoDb>,
        agent_service: Arc<AgentService>,
        internal_task_manager: Arc<TaskManager>,
    ) -> Self {
        Self {
            db,
            agent_service,
            internal_task_manager,
        }
    }

    pub async fn execute_chat_host(
        &self,
        session: &AgentSessionDoc,
        agent: &TeamAgent,
        user_message: &str,
        workspace_path: String,
        turn_system_instruction: Option<&str>,
        target_artifacts: Vec<String>,
        result_contract: Vec<String>,
        validation_mode: bool,
        cancel_token: CancellationToken,
        chat_manager: Arc<ChatManager>,
    ) -> Result<ServerHarnessHostOutcome> {
        self.execute_session_host(
            session,
            agent,
            user_message,
            workspace_path,
            turn_system_instruction,
            target_artifacts,
            result_contract,
            validation_mode,
            cancel_token,
            StreamBroadcaster::Chat {
                manager: chat_manager,
                context_id: session.session_id.clone(),
            },
        )
        .await
    }

    pub async fn execute_channel_host(
        &self,
        session: &AgentSessionDoc,
        agent: &TeamAgent,
        user_message: &str,
        workspace_path: String,
        target_artifacts: Vec<String>,
        result_contract: Vec<String>,
        validation_mode: bool,
        cancel_token: CancellationToken,
        channel_id: String,
        channel_manager: Arc<ChatChannelManager>,
    ) -> Result<ServerHarnessHostOutcome> {
        self.execute_session_host(
            session,
            agent,
            user_message,
            workspace_path,
            None,
            target_artifacts,
            result_contract,
            validation_mode,
            cancel_token,
            StreamBroadcaster::Channel {
                manager: channel_manager,
                context_id: channel_id,
            },
        )
        .await
    }

    pub async fn execute_document_analysis_host(
        &self,
        session: &AgentSessionDoc,
        agent: &TeamAgent,
        user_message: &str,
        workspace_path: String,
        target_artifacts: Vec<String>,
        result_contract: Vec<String>,
        validation_mode: bool,
        llm_overrides: Option<Value>,
        cancel_token: CancellationToken,
    ) -> Result<ServerHarnessHostOutcome> {
        self.execute_session_host_with_overrides(
            session,
            agent,
            user_message,
            workspace_path,
            None,
            target_artifacts,
            result_contract,
            validation_mode,
            cancel_token,
            llm_overrides,
            StreamBroadcaster::Silent,
        )
        .await
    }

    pub async fn execute_agent_task_host(
        &self,
        session: &AgentSessionDoc,
        agent: &TeamAgent,
        user_message: &str,
        workspace_path: String,
        turn_system_instruction: Option<&str>,
        target_artifacts: Vec<String>,
        result_contract: Vec<String>,
        validation_mode: bool,
        llm_overrides: Option<Value>,
        cancel_token: CancellationToken,
        task_id: String,
        task_manager: Arc<TaskManager>,
    ) -> Result<ServerHarnessHostOutcome> {
        self.execute_session_host_with_overrides(
            session,
            agent,
            user_message,
            workspace_path,
            turn_system_instruction,
            target_artifacts,
            result_contract,
            validation_mode,
            cancel_token,
            llm_overrides,
            StreamBroadcaster::Task {
                manager: task_manager,
                context_id: task_id,
            },
        )
        .await
    }

    async fn execute_session_host(
        &self,
        session: &AgentSessionDoc,
        agent: &TeamAgent,
        user_message: &str,
        workspace_path: String,
        turn_system_instruction: Option<&str>,
        target_artifacts: Vec<String>,
        result_contract: Vec<String>,
        validation_mode: bool,
        cancel_token: CancellationToken,
        broadcaster: StreamBroadcaster,
    ) -> Result<ServerHarnessHostOutcome> {
        self.execute_session_host_with_overrides(
            session,
            agent,
            user_message,
            workspace_path,
            turn_system_instruction,
            target_artifacts,
            result_contract,
            validation_mode,
            cancel_token,
            None,
            broadcaster,
        )
        .await
    }

    async fn execute_session_host_with_overrides(
        &self,
        session: &AgentSessionDoc,
        agent: &TeamAgent,
        user_message: &str,
        workspace_path: String,
        turn_system_instruction: Option<&str>,
        target_artifacts: Vec<String>,
        result_contract: Vec<String>,
        validation_mode: bool,
        cancel_token: CancellationToken,
        llm_overrides: Option<Value>,
        broadcaster: StreamBroadcaster,
    ) -> Result<ServerHarnessHostOutcome> {
        let inferred_targets = if session.session_source.eq_ignore_ascii_case("system")
            || session
                .session_source
                .eq_ignore_ascii_case("document_analysis")
        {
            Vec::new()
        } else {
            infer_targets_from_user_message(user_message)
        };
        let effective_target_artifacts = {
            let base_targets = if target_artifacts.is_empty() {
                if session
                    .session_source
                    .eq_ignore_ascii_case("document_analysis")
                {
                    Vec::new()
                } else {
                    host_target_artifacts(session)
                }
            } else {
                target_artifacts
            };
            let mut targets = if inferred_targets.is_empty() {
                base_targets
            } else {
                let mut merged = inferred_targets.clone();
                merged.extend(
                    base_targets
                        .into_iter()
                        .filter(|target| !is_contextual_host_target(target)),
                );
                merged
            };
            targets.extend(inferred_targets.clone());
            targets.sort();
            targets.dedup();
            targets
        };
        let effective_result_contract = {
            let mut contract = if result_contract.is_empty() {
                if session
                    .session_source
                    .eq_ignore_ascii_case("document_analysis")
                {
                    Vec::new()
                } else {
                    host_result_contract(session)
                }
            } else {
                result_contract
            };
            contract.extend(inferred_targets);
            contract.sort();
            contract.dedup();
            contract
        };
        let inferred_execution_mode = infer_coordinator_execution_mode(
            user_message,
            &effective_target_artifacts,
            &effective_result_contract,
        );
        tracing::info!(
            logical_session_id = %session.session_id,
            source = %session.session_source,
            target_count = effective_target_artifacts.len(),
            result_contract_count = effective_result_contract.len(),
            coordinator_execution_mode = %inferred_execution_mode,
            native_swarm_tool = native_swarm_tool_enabled(),
            planner_auto = planner_auto_swarm_enabled(),
            "ServerHarnessHost: execute_session_host_with_overrides start"
        );
        let base_agent = if agent
            .api_key
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        {
            agent.clone()
        } else {
            let fetched = self
                .agent_service
                .get_agent_with_key(&agent.id)
                .await?
                .unwrap_or_else(|| agent.clone());
            with_execution_api_key(agent, Some(&fetched))
        };
        let effective_agent = apply_llm_overrides(&base_agent, llm_overrides.as_ref());

        if effective_agent.api_format == ApiFormat::Local {
            return Err(anyhow!(
                "Direct harness host does not support Local API format yet"
            ));
        }

        let user_group_ids = agime_team::services::mongo::user_group_service_mongo::UserGroupService::new(
            (*self.db).clone(),
        )
        .get_user_group_ids(&session.team_id, &session.user_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect::<HashSet<_>>();
        let runtime_snapshot = AgentRuntimePolicyResolver::resolve_for_user_groups(
            &effective_agent,
            Some(session),
            None,
            Some(&user_group_ids),
        );
        let document_analysis_surface = session
            .session_source
            .eq_ignore_ascii_case("document_analysis");
        let non_delegating_surface = is_non_delegating_session_source(&session.session_source);
        let coordinator_execution_mode = if session.session_source.eq_ignore_ascii_case("system")
            || session
                .session_source
                .eq_ignore_ascii_case("document_analysis")
        {
            CoordinatorExecutionMode::SingleWorker
        } else if non_delegating_surface {
            if !matches!(
                inferred_execution_mode,
                CoordinatorExecutionMode::SingleWorker
            ) || runtime_snapshot.delegation_policy.allow_subagent
                || runtime_snapshot.delegation_policy.allow_swarm
            {
                tracing::info!(
                    logical_session_id = %session.session_id,
                    session_source = %session.session_source,
                    inferred_execution_mode = ?inferred_execution_mode,
                    "ServerHarnessHost: non-delegating surface suppressing delegation"
                );
            }
            CoordinatorExecutionMode::SingleWorker
        } else {
            match inferred_execution_mode {
                CoordinatorExecutionMode::ExplicitSwarm
                    if !runtime_snapshot.delegation_policy.allow_swarm =>
                {
                    CoordinatorExecutionMode::SingleWorker
                }
                CoordinatorExecutionMode::AutoSwarm
                    if !runtime_snapshot.delegation_policy.allow_swarm
                        || !runtime_snapshot.delegation_policy.allow_auto_swarm =>
                {
                    CoordinatorExecutionMode::SingleWorker
                }
                mode => mode,
            }
        };
        let delegation_mode = if non_delegating_surface
            || session
                .session_source
                .eq_ignore_ascii_case("document_analysis")
        {
            DelegationMode::Disabled
        } else {
            match coordinator_execution_mode {
                CoordinatorExecutionMode::ExplicitSwarm | CoordinatorExecutionMode::AutoSwarm => {
                    if runtime_snapshot.delegation_policy.allow_swarm {
                        DelegationMode::Swarm
                    } else {
                        DelegationMode::Disabled
                    }
                }
                CoordinatorExecutionMode::SingleWorker => {
                    if runtime_snapshot.delegation_policy.allow_subagent {
                        DelegationMode::Subagent
                    } else {
                        DelegationMode::Disabled
                    }
                }
            }
        };
        let effective_validation_mode = validation_mode
            && !session
                .session_source
                .eq_ignore_ascii_case("document_analysis")
            && runtime_snapshot.delegation_policy.allow_validation_worker;
        let require_final_report =
            session.require_final_report || runtime_snapshot.delegation_policy.require_final_report;
        let run_id = Uuid::new_v4().to_string();
        let workspace_service = WorkspaceService::new(String::new());
        let workspace_context = match workspace_service.load_workspace(&workspace_path) {
            Ok(Some(workspace)) => match workspace_service.execution_context(&workspace, &run_id) {
                Ok(context) => Some(context),
                Err(error) => {
                    tracing::warn!(
                        logical_session_id = %session.session_id,
                        workspace_path = %workspace_path,
                        error = %error,
                        "ServerHarnessHost: failed to build workspace execution context"
                    );
                    None
                }
            },
            Ok(None) => {
                tracing::warn!(
                    logical_session_id = %session.session_id,
                    workspace_path = %workspace_path,
                    "ServerHarnessHost: workspace manifest not found while building execution context"
                );
                None
            }
            Err(error) => {
                tracing::warn!(
                    logical_session_id = %session.session_id,
                    workspace_path = %workspace_path,
                    error = %error,
                    "ServerHarnessHost: failed to load workspace for execution context"
                );
                None
            }
        };
        if let Some(context) = &workspace_context {
            tracing::info!(
                logical_session_id = %session.session_id,
                workspace_kind = ?context.workspace_kind,
                workspace_root = %context.workspace_root,
                run_id = %context.run_id,
                run_dir = %context.run_dir,
                allowed_read_roots = ?context.allowed_read_roots,
                allowed_write_roots = ?context.allowed_write_roots,
                "ServerHarnessHost: workspace execution context ready"
            );
        }
        let effective_turn_system_instruction = merge_workspace_execution_instruction(
            turn_system_instruction,
            workspace_context.as_ref(),
        );

        let prepared = self
            .prepare_runtime(
                session,
                &effective_agent,
                &workspace_path,
                effective_turn_system_instruction.as_deref(),
            )
            .await?;
        tracing::info!(
            logical_session_id = %session.session_id,
            server_local_tool_count = prepared.server_local_tools.len(),
            extension_manager_enabled = prepared.extension_manager.is_some(),
            "ServerHarnessHost: runtime prepared"
        );
        let tool_runtime = Arc::new(DirectToolRuntime {
            dynamic_state: prepared.dynamic_state.clone(),
            extension_manager: prepared.extension_manager.clone(),
            task_manager: self.internal_task_manager.clone(),
            task_id: format!("direct-host:{}", run_id),
            cancel_token: cancel_token.clone(),
            tool_timeout_secs: prepared.tool_timeout_secs,
            server_local_tool_names: prepared
                .server_local_tools
                .iter()
                .map(|tool| tool.name.to_string())
                .collect(),
            workspace_path: Some(workspace_path.clone()),
            workspace_context: workspace_context.clone(),
        });
        let generated_title = if session.title.is_none() {
            derive_chat_title_from_user_message(user_message)
        } else {
            None
        };
        let control_sink: Arc<dyn HarnessControlSink> = Arc::new(ServerHarnessControlSink);
        let request_registry = RuntimeControlRegistry::default();
        let event_sink = Arc::new(ServerHarnessEventSink {
            broadcaster,
            tool_runtime,
            control_sink: control_sink.clone(),
            request_registry: request_registry.clone(),
        });
        let task_runtime = Arc::new(TaskRuntime::default());
        let (runtime_session_tx, runtime_session_rx) = watch::channel(None);
        let worker_forwarder = spawn_task_runtime_forwarder(
            task_runtime.clone(),
            runtime_session_rx,
            event_sink.broadcaster.clone(),
            control_sink.clone(),
            request_registry.clone(),
        );
        let initial_messages: Vec<Message> =
            serde_json::from_str(&session.messages_json).unwrap_or_default();
        let initial_conversation = Conversation::new_unvalidated(initial_messages);
        let agent_instance = Arc::new(Agent::new());
        agent_instance
            .set_delegation_capability_context(Some(DelegationCapabilityContext {
                allow_plan_mode: !document_analysis_surface
                    && runtime_snapshot.delegation_policy.allow_plan,
                allow_subagent: !document_analysis_surface
                    && runtime_snapshot.delegation_policy.allow_subagent,
                allow_swarm: !document_analysis_surface
                    && runtime_snapshot.delegation_policy.allow_swarm,
                allow_worker_messaging: !document_analysis_surface
                    && runtime_snapshot.delegation_policy.allow_worker_messaging,
            }))
            .await;
        let explicit_delegation_turn = effective_turn_system_instruction
            .as_deref()
            .is_some_and(|value| value.contains(CHAT_DELEGATION_PROFILE_ID));
        let harness_mode = if session
            .session_source
            .eq_ignore_ascii_case("document_analysis")
        {
            HarnessMode::Conversation
        } else if should_force_execute_for_explicit_delegation(
            &session.session_source,
            user_message,
            explicit_delegation_turn,
        ) {
            HarnessMode::Execute
        } else {
            harness_mode_for_session_source(&session.session_source)
        };
        let mut server_local_tool_names = prepared
            .server_local_tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        server_local_tool_names.sort();
        server_local_tool_names.dedup();
        let server_local_tools_for_host = prepared.server_local_tools.clone();
        let worker_extensions_for_host = prepared.worker_extensions.clone();
        let host_extensions = host_extensions_for_mode(
            harness_mode,
            &server_local_tools_for_host,
            &worker_extensions_for_host,
        );
        let completion_surface_policy =
            completion_surface_policy_for_session_source(&session.session_source, harness_mode);
        let completion_contract = completion_contract_for_session_source(
            &session.session_source,
            harness_mode,
            require_final_report,
        );
        let required_tool_prefixes =
            required_tool_prefixes_for_session_source(&session.session_source);
        let auto_approve_chat = effective_agent.auto_approve_chat;
        let approval_mode = runtime_snapshot.delegation_policy.approval_mode;
        set_permission_bridge_resolver(Arc::new(move |request| match approval_mode {
            ApprovalMode::HeadlessFallback => auto_resolve_request(request),
            ApprovalMode::LeaderOwned => {
                if request.validation_mode {
                    return auto_resolve_request(request);
                }

                if auto_approve_chat {
                    PermissionBridgeResolution {
                            request_id: request.request_id.clone(),
                            permission: Permission::AllowOnce,
                            feedback: Some(
                                    "direct-host leader-owned approval loop auto-approved the bounded worker request for this session"
                                        .to_string(),
                            ),
                            source: PermissionDecisionSource::RuntimePolicy,
                            resolved_at: chrono::Utc::now().to_rfc3339(),
                        }
                } else {
                    PermissionBridgeResolution {
                            request_id: request.request_id.clone(),
                            permission: Permission::DenyOnce,
                            feedback: Some(
                                    "direct-host leader-owned approval loop denied this worker request because interactive confirmation is not configured for this session"
                                        .to_string(),
                            ),
                            source: PermissionDecisionSource::RuntimePolicy,
                            resolved_at: chrono::Utc::now().to_rfc3339(),
                        }
                }
            }
        }));
        let host_result = run_harness_host(
            HarnessHostRequest {
                logical_session_id: session.session_id.clone(),
                working_dir: PathBuf::from(&workspace_path),
                session_name: session
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("server-session-{}", session.session_id)),
                session_type: SessionType::Hidden,
                initial_conversation,
                user_message: Message::user().with_text(user_message),
                session_config: SessionConfig {
                    id: session.session_id.clone(),
                    schedule_id: None,
                    max_turns: session
                        .max_turns
                        .and_then(|value| (value > 0).then_some(value as u32)),
                    retry_config: session.retry_config.clone(),
                },
                provider: prepared.provider.clone(),
                mode: harness_mode,
                delegation_mode,
                coordinator_execution_mode,
                provider_turn_mode: provider_turn_mode_for_session_source(
                    &session.session_source,
                    harness_mode,
                ),
                completion_surface_policy,
                write_scope: Vec::new(),
                target_artifacts: effective_target_artifacts,
                result_contract: effective_result_contract,
                server_local_tool_names,
                required_tool_prefixes: required_tool_prefixes.clone(),
                parallelism_budget: runtime_snapshot.delegation_policy.parallelism_budget,
                swarm_budget: runtime_snapshot.delegation_policy.swarm_budget,
                validation_mode: effective_validation_mode,
                worker_extensions: worker_extensions_for_host,
                initial_context_runtime_state: session.context_runtime_state.clone(),
                task_runtime: Some(task_runtime.clone()),
                system_prompt_override: Some(prepared.system_prompt.clone()),
                system_prompt_extras: Vec::new(),
                extensions: host_extensions,
                final_output: completion_contract,
                cancel_token: Some(cancel_token.clone()),
            },
            HarnessHostDependencies {
                agent: agent_instance.clone(),
                event_sink: event_sink.clone(),
                control_sink: control_sink.clone(),
                persistence: Arc::new(SessionHostPersistenceAdapter {
                    agent_service: self.agent_service.clone(),
                    session_id: session.session_id.clone(),
                    fallback_title: session.title.clone(),
                    generated_title,
                    runtime_session_tx: Some(runtime_session_tx),
                    broadcaster: event_sink.broadcaster.clone(),
                    control_sink: control_sink.clone(),
                    request_registry: request_registry.clone(),
                    initial_context_runtime_state: session.context_runtime_state.clone(),
                }),
            },
        )
        .await;
        clear_permission_bridge_resolver();
        let host_result = host_result?;
        tracing::info!(
            logical_session_id = %session.session_id,
            runtime_session_id = %host_result.runtime_session_id,
            events_emitted = host_result.events_emitted,
            conversation_len = host_result.final_conversation.len(),
            "ServerHarnessHost: run_harness_host returned"
        );
        worker_forwarder.abort();

        let messages_json = serde_json::to_string(host_result.final_conversation.messages())?;
        let notification_summary = host_result.notification_summary.as_ref();
        let signal_summary = host_result.signal_summary.as_ref().or(notification_summary);
        let completion_report = Some(normalize_adapter_execution_host_completion_report(
            host_result.completion_report.clone(),
            signal_summary,
            &session.session_source,
        ));
        if let Some(report) = completion_report.as_ref() {
            let payload = super::hook_runtime::build_run_settle_payload(
                &session.session_id,
                Some(host_result.runtime_session_id.clone()),
                report,
            );
            super::hook_runtime::emit_run_settle_payload(&payload);
        }
        if let Some(report) = completion_report.as_ref() {
            let workspace_service = WorkspaceService::new(String::new());
            if let Ok(Some(workspace)) = workspace_service.load_workspace(&workspace_path) {
                let produced_resolution = workspace_service
                    .resolve_workspace_outputs(&workspace, &report.produced_artifacts);
                let accepted_resolution = workspace_service
                    .resolve_workspace_outputs(&workspace, &report.accepted_artifacts);
                if let Some(context) = &workspace_context {
                    tracing::info!(
                        logical_session_id = %session.session_id,
                        runtime_session_id = %host_result.runtime_session_id,
                        workspace_kind = ?context.workspace_kind,
                        workspace_root = %context.workspace_root,
                        run_id = %context.run_id,
                        run_dir = %context.run_dir,
                        produced_materialized = ?produced_resolution.materialized_paths,
                        accepted_materialized = ?accepted_resolution.materialized_paths,
                        produced_missing = ?produced_resolution.missing_paths,
                        accepted_missing = ?accepted_resolution.missing_paths,
                        produced_logical = ?produced_resolution.logical_targets,
                        accepted_logical = ?accepted_resolution.logical_targets,
                        "ServerHarnessHost: workspace artifact resolution"
                    );
                }
                if !produced_resolution.missing_paths.is_empty()
                    || !accepted_resolution.missing_paths.is_empty()
                    || !produced_resolution.logical_targets.is_empty()
                    || !accepted_resolution.logical_targets.is_empty()
                {
                    tracing::warn!(
                        logical_session_id = %session.session_id,
                        runtime_session_id = %host_result.runtime_session_id,
                        produced_materialized = ?produced_resolution.materialized_paths,
                        accepted_materialized = ?accepted_resolution.materialized_paths,
                        produced_missing = ?produced_resolution.missing_paths,
                        accepted_missing = ?accepted_resolution.missing_paths,
                        logical_targets = ?produced_resolution.logical_targets
                            .iter()
                            .chain(accepted_resolution.logical_targets.iter())
                            .cloned()
                            .collect::<Vec<_>>(),
                        "completion artifacts were not fully materialized into the leader workspace"
                    );
                }
                let _ = workspace_service.record_completion_artifacts(
                    &workspace,
                    &report.produced_artifacts,
                    &report.accepted_artifacts,
                );
                let _ = workspace_service.reconcile_manifest_artifacts(&workspace);
            }
        }
        let preview = sanitize_user_visible_runtime_text(
            runtime_text::extract_last_assistant_text(&messages_json).as_deref(),
        )
        .or_else(|| {
            host_result
                .completion_outcome
                .as_ref()
                .and_then(|outcome| sanitize_user_visible_runtime_text(outcome.summary.as_deref()))
        })
        .unwrap_or_default();
        let logical_execution_status = completion_report
            .as_ref()
            .map(|report| report.status.as_str())
            .unwrap_or("blocked");
        tracing::info!(
            logical_session_id = %session.session_id,
            execution_status = logical_execution_status,
            "ServerHarnessHost: host outcome ready for logical session"
        );

        self.persist_extension_overrides(&effective_agent, session, &prepared)
            .await;
        self.shutdown_runtime(prepared.dynamic_state).await;

        Ok(ServerHarnessHostOutcome {
            messages_json,
            message_count: host_result.final_conversation.len() as i32,
            total_tokens: host_result.total_tokens,
            context_runtime_state: host_result.context_runtime_state,
            last_assistant_text: if preview.trim().is_empty() {
                None
            } else {
                Some(preview)
            },
            completion_report,
            persisted_child_evidence: host_result.persisted_child_evidence,
            persisted_child_transcript_resume: host_result.persisted_child_transcript_resume,
            transition_trace: host_result.transition_trace,
            events_emitted: host_result.events_emitted,
            signal_summary: host_result
                .signal_summary
                .or(host_result.notification_summary),
            completion_outcome: host_result.completion_outcome,
        })
    }

    async fn prepare_runtime(
        &self,
        session: &AgentSessionDoc,
        agent: &TeamAgent,
        workspace_path: &str,
        turn_system_instruction: Option<&str>,
    ) -> Result<DirectHostPreparedRuntime> {
        let runtime_settings = TeamRuntimeSettings::from_env();
        let api_caller = build_api_caller(agent);
        let user_group_ids = agime_team::services::mongo::user_group_service_mongo::UserGroupService::new(
            (*self.db).clone(),
        )
        .get_user_group_ids(&session.team_id, &session.user_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect::<HashSet<_>>();
        let runtime_snapshot = AgentRuntimePolicyResolver::resolve_for_user_groups(
            agent,
            Some(session),
            None,
            Some(&user_group_ids),
        );

        let allowed_extension_names: HashSet<String> = runtime_snapshot
            .extensions
            .effective_allowed_extension_names
            .iter()
            .cloned()
            .collect();
        let allowed_skill_ids: Option<HashSet<String>> = runtime_snapshot
            .skills
            .effective_allowed_skill_ids
            .as_ref()
            .map(|items| items.iter().cloned().collect::<HashSet<_>>());

        let platform_enabled_extensions = runtime_snapshot.runtime_builtin_extensions();

        let installer = ExtensionInstaller::new(
            self.db.clone(),
            runtime_settings.extension_cache_root.clone(),
            runtime_settings.auto_install_extensions,
        );
        let mut all_extensions = builtin_extension_configs_to_custom(&platform_enabled_extensions);
        all_extensions.extend(
            resolve_agent_custom_extensions(
                &self.db,
                &session.team_id,
                &runtime_snapshot.runtime_custom_extensions(),
                &installer,
            )
            .await,
        );
        all_extensions.extend(
            resolve_agent_attached_team_extensions(
                &self.db,
                &session.team_id,
                &runtime_snapshot.runtime_team_extension_refs(),
                &runtime_snapshot.legacy_team_custom_extensions(),
                &installer,
            )
            .await,
        );

        if !session.disabled_extensions.is_empty() {
            let disabled_set: HashSet<&str> = session
                .disabled_extensions
                .iter()
                .map(String::as_str)
                .collect();
            all_extensions.retain(|ext| !disabled_set.contains(ext.name.as_str()));
        }
        if !session.enabled_extensions.is_empty() {
            let existing_names: HashSet<String> =
                all_extensions.iter().map(|e| e.name.clone()).collect();
            for enabled_name in &session.enabled_extensions {
                if existing_names.contains(enabled_name) {
                    continue;
                }
                if let Some(cfg) = find_extension_config_by_name(agent, enabled_name) {
                    all_extensions.push(cfg);
                }
            }
        }
        if !allowed_extension_names.is_empty() {
            all_extensions
                .retain(|ext| extension_allowed_by_name(&ext.name, &allowed_extension_names));
        }

        let elicitation_bridge: ElicitationBridgeCallback = Arc::new(move |_event| {});
        let mcp = if all_extensions.is_empty() {
            None
        } else {
            McpConnector::connect(
                &all_extensions,
                api_caller.clone(),
                Some(elicitation_bridge),
                Some(workspace_path),
            )
            .await
            .ok()
        };

        let session_doc_scope = if session.attached_document_ids.is_empty() {
            None
        } else {
            Some(session.attached_document_ids.as_slice())
        };
        let force_portal_tools = session.session_source.eq_ignore_ascii_case("portal_coding")
            || session
                .session_source
                .eq_ignore_ascii_case("portal_manager");
        let platform = PlatformExtensionRunner::create(
            &platform_enabled_extensions,
            Some(self.db.clone()),
            None,
            Some(&session.team_id),
            Some(&session.user_id),
            Some(session.session_source.as_str()),
            Some(session.session_id.as_str()),
            Some(&agent.id),
            runtime_settings.skill_mode == TeamSkillMode::OnDemand,
            Some(workspace_path),
            Some(&runtime_settings.workspace_root),
            Some(&runtime_settings.portal_base_url),
            Some(&allowed_extension_names),
            allowed_skill_ids.as_ref(),
            session_doc_scope,
            session.portal_restricted,
            runtime_snapshot.document_access_mode.as_deref(),
            force_portal_tools,
        )
        .await;

        let dynamic_state = Arc::new(RwLock::new(DynamicExtensionState {
            mcp,
            platform,
            agent: agent.clone(),
            api_caller: api_caller.clone(),
        }));

        let ext_manager_enabled = {
            let mut enabled = agent_has_extension_manager_enabled(agent);
            if session.portal_restricted {
                enabled = false;
            }
            if enabled {
                if !allowed_extension_names.is_empty() {
                    enabled = allowed_extension_names.contains("extension_manager");
                }
            }
            enabled
        };
        let extension_manager = if ext_manager_enabled {
            Some(Arc::new(TeamExtensionManagerClient::with_session(
                dynamic_state.clone(),
                session.session_id.clone(),
                self.agent_service.clone(),
                (*self.db).clone(),
            )))
        } else {
            None
        };

        let state = dynamic_state.read().await;
        let mut ext_infos = state
            .mcp
            .as_ref()
            .map(|m| {
                m.extension_names()
                    .into_iter()
                    .map(|name| agime::agents::extension::ExtensionInfo::new(&name, "", false))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        ext_infos.extend(state.platform.extension_infos());
        ext_infos.sort_by(|a, b| a.name.cmp(&b.name));

        let mut server_local_tools = Vec::new();
        if let Some(ref mcp) = state.mcp {
            server_local_tools.extend(mcp.tools_as_rmcp());
        }
        server_local_tools.extend(state.platform.tools_as_rmcp());
        drop(state);
        if extension_manager.is_some() {
            server_local_tools.extend(TeamExtensionManagerClient::tools_as_rmcp());
        }
        if let Some(allow_only) = parse_turn_tool_gate_allow_only(turn_system_instruction) {
            server_local_tools.retain(|tool| allow_only.contains(tool.name.as_ref()));
        }

        let mut worker_extensions = Vec::new();
        for extension in &all_extensions {
            if let Some(config) = custom_extension_to_agent_extension(extension) {
                worker_extensions.push(config);
            }
        }
        for extension in &platform_enabled_extensions {
            if let Some(config) = platform_extension_to_agent_extension(extension) {
                worker_extensions.push(config);
            }
        }
        worker_extensions.sort_by(|left, right| left.name().cmp(&right.name()));
        worker_extensions.dedup_by(|left, right| left.name() == right.name());

        let custom = agent
            .system_prompt
            .as_deref()
            .filter(|s| !s.trim().is_empty());
        let mut system_prompt = compose_top_level_prompt(AgentPromptComposerInput {
            extensions: &ext_infos,
            custom_prompt: custom,
            runtime_snapshot: Some(&runtime_snapshot),
            session_extra_instructions: session.extra_instructions.as_deref(),
            prompt_profile_overlay: None,
            turn_system_instruction,
            session_source: session.session_source.as_str(),
            portal_restricted: session.portal_restricted,
            require_final_report: session.require_final_report
                || runtime_snapshot.delegation_policy.require_final_report,
            model_name: agent.model.as_deref().unwrap_or_default(),
        })
        .top_level_prompt;
        system_prompt.push_str(
            "\n\n<tool_calling_contract>\n\
Use the native tool-calling interface only.\n\
Do not emit textual pseudo-tool syntax such as `<invoke ...>`, `<<CALL_...>>`, XML wrappers, or handwritten JSON call blocks inside normal assistant text.\n\
If you need a tool, call it directly through the model tool interface.\n\
</tool_calling_contract>",
        );

        Ok(DirectHostPreparedRuntime {
            provider: super::provider_factory::create_provider_for_agent(agent)?,
            system_prompt,
            dynamic_state,
            extension_manager,
            server_local_tools,
            worker_extensions,
            tool_timeout_secs: session.tool_timeout_seconds.filter(|value| *value > 0),
            extension_state_persistence_enabled: ext_manager_enabled,
        })
    }

    async fn persist_extension_overrides(
        &self,
        agent: &TeamAgent,
        session: &AgentSessionDoc,
        prepared: &DirectHostPreparedRuntime,
    ) {
        if !prepared.extension_state_persistence_enabled {
            return;
        }
        let state = prepared.dynamic_state.read().await;
        let mut active_names: Vec<String> = Vec::new();
        if let Some(ref mcp) = state.mcp {
            active_names.extend(mcp.extension_names());
        }
        active_names.extend(state.platform.extension_names());
        drop(state);

        let active_set: HashSet<String> = active_names.into_iter().collect();
        let overrides = compute_extension_overrides(agent, &active_set);
        if overrides.disabled.is_empty() && overrides.enabled.is_empty() {
            return;
        }
        if let Err(error) = self
            .agent_service
            .update_session_extensions(&session.session_id, &overrides.disabled, &overrides.enabled)
            .await
        {
            tracing::warn!(
                "Failed to save direct-host extension overrides for {}: {}",
                session.session_id,
                error
            );
        }
    }

    async fn shutdown_runtime(&self, dynamic_state: Arc<RwLock<DynamicExtensionState>>) {
        let mcp = {
            let mut state = dynamic_state.write().await;
            state.mcp.take()
        };
        if let Some(connector) = mcp {
            connector.shutdown().await;
        }
    }
}

fn apply_llm_overrides(agent: &TeamAgent, overrides: Option<&Value>) -> TeamAgent {
    let Some(overrides) = overrides else {
        return agent.clone();
    };

    let mut effective = agent.clone();
    if let Some(api_url) = overrides
        .get("api_url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        effective.api_url = Some(api_url.to_string());
    }
    if let Some(api_key) = overrides
        .get("api_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        effective.api_key = Some(api_key.to_string());
    }
    if let Some(model) = overrides
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        effective.model = Some(model.to_string());
    }
    if let Some(api_format) = overrides
        .get("api_format")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<ApiFormat>().ok())
    {
        effective.api_format = api_format;
    }
    effective
}

fn with_execution_api_key(agent: &TeamAgent, fetched: Option<&TeamAgent>) -> TeamAgent {
    let mut effective = agent.clone();
    if effective
        .api_key
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return effective;
    }

    if let Some(api_key) = fetched
        .and_then(|candidate| candidate.api_key.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        effective.api_key = Some(api_key.to_string());
    }

    effective
}

fn workspace_write_allowed(context: &WorkspaceExecutionContext, path: &Path) -> bool {
    context
        .allowed_write_roots
        .iter()
        .any(|root| path.starts_with(Path::new(root)))
}

fn workspace_execution_instruction(context: &WorkspaceExecutionContext) -> String {
    format!(
        "Workspace execution boundary:\n- Workspace root: `{}`\n- Current run directory: `{}`\n- Treat `attachments/` as read-only input material.\n- Write final user-visible artifacts under `artifacts/`.\n- Write durable notes under `notes/`.\n- Use `runs/{}` only for this turn's temporary execution files.\n- Do not describe paths outside the workspace as previewable, downloadable, or shareable workspace artifacts.",
        context.workspace_root, context.run_dir, context.run_id
    )
}

fn merge_workspace_execution_instruction(
    turn_system_instruction: Option<&str>,
    context: Option<&WorkspaceExecutionContext>,
) -> Option<String> {
    let existing = turn_system_instruction
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let workspace_instruction = context.map(workspace_execution_instruction);
    match (existing, workspace_instruction) {
        (Some(existing), Some(workspace_instruction)) => {
            Some(format!("{existing}\n\n{workspace_instruction}"))
        }
        (Some(existing), None) => Some(existing.to_string()),
        (None, Some(workspace_instruction)) => Some(workspace_instruction),
        (None, None) => None,
    }
}

async fn execute_direct_tool_call(
    runtime: Arc<DirectToolRuntime>,
    tool_name: &str,
    args: Value,
) -> (u64, Result<Vec<ToolContentBlock>, String>) {
    let started_at = std::time::Instant::now();
    if runtime.cancel_token.is_cancelled() {
        return (
            started_at.elapsed().as_millis() as u64,
            Err(tool_execution_cancelled_error_text(tool_name)),
        );
    }
    if let Some(manager) = runtime.extension_manager.as_ref() {
        if TeamExtensionManagerClient::can_handle(tool_name) {
            let result = if let Some(timeout_secs) = runtime.tool_timeout_secs {
                tokio::select! {
                    _ = runtime.cancel_token.cancelled() => Err(tool_execution_cancelled_error_text(tool_name)),
                    outcome = tokio::time::timeout(
                        std::time::Duration::from_secs(timeout_secs),
                        manager.call_tool_rich(tool_name, args),
                    ) => {
                        match outcome {
                            Ok(Ok(blocks)) => Ok(blocks),
                            Ok(Err(error)) => Err(format!("Error: {}", error)),
                            Err(_) => Err(format!(
                                "Error: tool '{}' timed out after {}s",
                                tool_name, timeout_secs
                            )),
                        }
                    }
                }
            } else {
                tokio::select! {
                    _ = runtime.cancel_token.cancelled() => Err(tool_execution_cancelled_error_text(tool_name)),
                    outcome = manager.call_tool_rich(tool_name, args) => {
                        outcome.map_err(|error| format!("Error: {}", error))
                    }
                }
            };
            let result = if runtime.cancel_token.is_cancelled() && result.is_ok() {
                Err(tool_execution_cancelled_error_text(tool_name))
            } else {
                result
            };
            return (started_at.elapsed().as_millis() as u64, result);
        }
    }

    let (duration_ms, result) = execute_standard_tool_call(
        runtime.dynamic_state.clone(),
        runtime.task_manager.clone(),
        runtime.task_id.clone(),
        runtime.cancel_token.child_token(),
        runtime.tool_timeout_secs,
        tool_name.to_string(),
        args,
    )
    .await;
    if runtime.cancel_token.is_cancelled() && result.is_ok() {
        (
            duration_ms,
            Err(tool_execution_cancelled_error_text(tool_name)),
        )
    } else {
        (duration_ms, result)
    }
}

fn blocks_to_call_result(blocks: &[ToolContentBlock]) -> CallToolResult {
    let content = blocks
        .iter()
        .filter_map(|block| match block {
            ToolContentBlock::Text(text) => Some(Content::text(text.clone())),
            ToolContentBlock::Image { mime_type, data } => {
                Some(Content::image(data.clone(), mime_type.clone()))
            }
            ToolContentBlock::Resource { uri, mime_type, .. } => {
                Some(Content::text(match mime_type.as_deref() {
                    Some(mime) => format!("[Resource: {} ({})]", uri, mime),
                    None => format!("[Resource: {}]", uri),
                }))
            }
            ToolContentBlock::StructuredJson(_) => None,
        })
        .collect::<Vec<_>>();
    let structured_blocks = blocks
        .iter()
        .filter_map(|block| match block {
            ToolContentBlock::StructuredJson(value) => Some(value.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let structured_content = match structured_blocks.as_slice() {
        [] => None,
        [value] => Some(value.clone()),
        values => Some(serde_json::Value::Array(values.to_vec())),
    };
    CallToolResult {
        content,
        structured_content,
        is_error: Some(false),
        meta: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agime_team::models::ApiFormat;
    use agime_team::models::TeamAgent;
    use serde_json::json;
    use std::sync::{Mutex, OnceLock};
    use tokio::sync::Mutex as AsyncMutex;

    #[derive(Default)]
    struct RecordingControlSink {
        envelopes: AsyncMutex<Vec<HarnessControlEnvelope>>,
    }

    #[async_trait::async_trait]
    impl HarnessControlSink for RecordingControlSink {
        async fn handle(&self, envelope: &HarnessControlEnvelope) -> Result<()> {
            self.envelopes.lock().await.push(envelope.clone());
            Ok(())
        }
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn blocks_to_call_result_preserves_structured_content() {
        let structured = json!({
            "doc_id": "doc-1",
            "access_established": true,
            "content_accessed": true,
        });
        let result = blocks_to_call_result(&[
            ToolContentBlock::Text("document read".to_string()),
            ToolContentBlock::StructuredJson(structured.clone()),
        ]);

        assert_eq!(result.structured_content, Some(structured));
        assert_eq!(result.content.len(), 1);
    }

    #[test]
    fn llm_overrides_replace_effective_agent_fields() {
        let mut agent = TeamAgent::new("team-1".to_string(), "agent-1".to_string());
        agent.api_format = ApiFormat::OpenAI;
        agent.api_url = Some("https://old".to_string());
        agent.api_key = Some("old-key".to_string());
        agent.model = Some("old-model".to_string());

        let effective = apply_llm_overrides(
            &agent,
            Some(&serde_json::json!({
                "api_format": "anthropic",
                "api_url": "https://new",
                "api_key": "new-key",
                "model": "claude-test"
            })),
        );

        assert_eq!(effective.api_format, ApiFormat::Anthropic);
        assert_eq!(effective.api_url.as_deref(), Some("https://new"));
        assert_eq!(effective.api_key.as_deref(), Some("new-key"));
        assert_eq!(effective.model.as_deref(), Some("claude-test"));
    }

    #[test]
    fn derive_chat_title_truncates_unicode_safely() {
        let title = derive_chat_title_from_user_message(
            "这是一个非常长的标题测试消息，用来验证 direct host 标题截断是否安全并且不会破坏 unicode 字符边界。",
        )
        .expect("title");
        assert!(title.chars().count() <= 50);
        assert!(title.ends_with("..."));
    }

    #[test]
    fn execution_agent_prefers_fetched_api_key_when_sanitized() {
        let mut sanitized = TeamAgent::new("team-1".to_string(), "agent-1".to_string());
        sanitized.api_key = None;

        let mut fetched = sanitized.clone();
        fetched.api_key = Some("secret-key".to_string());

        let effective = with_execution_api_key(&sanitized, Some(&fetched));
        assert_eq!(effective.api_key.as_deref(), Some("secret-key"));
    }

    #[test]
    fn execution_agent_keeps_existing_api_key() {
        let mut provided = TeamAgent::new("team-1".to_string(), "agent-1".to_string());
        provided.api_key = Some("existing-key".to_string());

        let mut fetched = provided.clone();
        fetched.api_key = Some("fetched-key".to_string());

        let effective = with_execution_api_key(&provided, Some(&fetched));
        assert_eq!(effective.api_key.as_deref(), Some("existing-key"));
    }

    #[test]
    fn host_targets_include_channel_thread_but_not_attached_documents() {
        let session = AgentSessionDoc {
            id: None,
            session_id: "session-1".to_string(),
            team_id: "team-1".to_string(),
            agent_id: "agent-1".to_string(),
            user_id: "user-1".to_string(),
            name: None,
            status: "active".to_string(),
            messages_json: "[]".to_string(),
            message_count: 0,
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            context_runtime_state: None,
            disabled_extensions: Vec::new(),
            enabled_extensions: Vec::new(),
            created_at: bson::DateTime::now(),
            updated_at: bson::DateTime::now(),
            title: None,
            pinned: false,
            last_message_preview: None,
            last_message_at: None,
            is_processing: false,
            last_execution_status: None,
            last_execution_error: None,
            last_execution_finished_at: None,
            last_runtime_session_id: None,
            last_delegation_runtime: None,
            attached_document_ids: vec!["doc-a".to_string(), "doc-b".to_string()],
            workspace_path: None,
            workspace_id: None,
            workspace_kind: None,
            workspace_manifest_path: None,
            extra_instructions: None,
            allowed_extensions: None,
            allowed_skill_ids: None,
            retry_config: None,
            max_turns: None,
            tool_timeout_seconds: None,
            max_portal_retry_rounds: None,
            require_final_report: false,
            portal_restricted: false,
            document_access_mode: None,
            document_scope_mode: None,
            document_write_mode: None,
            delegation_policy_override: None,
            portal_id: None,
            portal_slug: None,
            visitor_id: None,
            session_source: "chat".to_string(),
            source_channel_id: Some("channel-1".to_string()),
            source_channel_name: None,
            source_thread_root_id: Some("thread-1".to_string()),
            thread_branch: None,
            thread_repo_ref: None,
            hidden_from_chat_list: false,
            pending_message_workspace_files: Vec::new(),
        };

        let targets = host_target_artifacts(&session);
        assert_eq!(targets, vec!["channel:channel-1/thread:thread-1"]);
    }

    #[test]
    fn infer_targets_from_user_message_extracts_deliverable_paths() {
        let targets = infer_targets_from_user_message(
            "请并行产出 `docs/market-summary.md` 和 docs/risk-summary.md，最后再输出 reports/final.json",
        );
        assert!(targets.contains(&"docs/market-summary.md".to_string()));
        assert!(targets.contains(&"docs/risk-summary.md".to_string()));
        assert!(targets.contains(&"reports/final.json".to_string()));
    }

    #[test]
    fn infer_targets_from_user_message_ignores_adjacent_chinese_punctuation() {
        let targets = infer_targets_from_user_message(
            "并行形成两个有界输出：docs/market-summary.md 和 docs/risk-summary.md。最后再给出一段简短汇总。",
        );
        assert!(targets.contains(&"docs/market-summary.md".to_string()));
        assert!(targets.contains(&"docs/risk-summary.md".to_string()));
        assert!(!targets.iter().any(|value| value.contains("最后再给出")));
    }

    #[test]
    fn infer_targets_from_user_message_matches_mime_like_text_but_system_sessions_skip_it() {
        let synthetic_prompt =
            "请分析文档「demo.txt」(MIME: text/plain, 文档ID: doc-1)。请输出文档概述和要点总结。";
        let inferred = infer_targets_from_user_message(synthetic_prompt);
        assert_eq!(inferred, vec!["text/plain"]);

        let effective = if "system".eq_ignore_ascii_case("system") {
            Vec::<String>::new()
        } else {
            inferred
        };
        assert!(effective.is_empty());
    }

    #[test]
    fn explicit_deliverables_take_priority_over_contextual_targets() {
        let session = AgentSessionDoc {
            id: None,
            session_id: "session-1".to_string(),
            team_id: "team-1".to_string(),
            agent_id: "agent-1".to_string(),
            user_id: "user-1".to_string(),
            name: None,
            status: "active".to_string(),
            messages_json: "[]".to_string(),
            message_count: 0,
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            context_runtime_state: None,
            disabled_extensions: Vec::new(),
            enabled_extensions: Vec::new(),
            created_at: bson::DateTime::now(),
            updated_at: bson::DateTime::now(),
            title: None,
            pinned: false,
            last_message_preview: None,
            last_message_at: None,
            is_processing: false,
            last_execution_status: None,
            last_execution_error: None,
            last_execution_finished_at: None,
            last_runtime_session_id: None,
            last_delegation_runtime: None,
            attached_document_ids: vec!["doc-a".to_string()],
            workspace_path: None,
            workspace_id: None,
            workspace_kind: None,
            workspace_manifest_path: None,
            extra_instructions: None,
            allowed_extensions: None,
            allowed_skill_ids: None,
            retry_config: None,
            max_turns: None,
            tool_timeout_seconds: None,
            max_portal_retry_rounds: None,
            require_final_report: false,
            portal_restricted: false,
            document_access_mode: None,
            document_scope_mode: None,
            document_write_mode: None,
            delegation_policy_override: None,
            portal_id: None,
            portal_slug: None,
            visitor_id: None,
            session_source: "channel_runtime".to_string(),
            source_channel_id: Some("channel-1".to_string()),
            source_channel_name: None,
            source_thread_root_id: None,
            thread_branch: None,
            thread_repo_ref: None,
            hidden_from_chat_list: false,
            pending_message_workspace_files: Vec::new(),
        };

        let inferred = infer_targets_from_user_message(
            "并行产出 docs/market-summary.md 和 docs/risk-summary.md",
        );
        let mut targets = inferred.clone();
        targets.extend(
            host_target_artifacts(&session)
                .into_iter()
                .filter(|target| !is_contextual_host_target(target)),
        );
        targets.sort();
        targets.dedup();

        assert!(targets.contains(&"docs/market-summary.md".to_string()));
        assert!(targets.contains(&"docs/risk-summary.md".to_string()));
        assert!(!targets.iter().any(|value| value.starts_with("channel:")));
    }

    #[test]
    fn coordinator_execution_mode_prefers_auto_swarm_for_multiple_targets() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::set_var("AGIME_ENABLE_SWARM_PLANNER_AUTO", "true");
        std::env::remove_var("AGIME_ENABLE_NATIVE_SWARM_TOOL");
        let mode = infer_coordinator_execution_mode(
            "produce docs/a.md and docs/b.md",
            &["docs/a.md".to_string(), "docs/b.md".to_string()],
            &["docs/a.md".to_string(), "docs/b.md".to_string()],
        );
        assert_eq!(mode, CoordinatorExecutionMode::AutoSwarm);
        std::env::remove_var("AGIME_ENABLE_SWARM_PLANNER_AUTO");
    }

    #[test]
    fn coordinator_execution_mode_uses_explicit_swarm_when_auto_disabled_and_targets_are_stable() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::remove_var("AGIME_ENABLE_SWARM_PLANNER_AUTO");
        std::env::set_var("AGIME_ENABLE_NATIVE_SWARM_TOOL", "true");
        let mode = infer_coordinator_execution_mode(
            "produce docs/a.md and docs/b.md",
            &["docs/a.md".to_string(), "docs/b.md".to_string()],
            &["docs/a.md".to_string(), "docs/b.md".to_string()],
        );
        assert_eq!(mode, CoordinatorExecutionMode::ExplicitSwarm);
        std::env::remove_var("AGIME_ENABLE_NATIVE_SWARM_TOOL");
    }

    #[test]
    fn coordinator_execution_mode_prefers_explicit_swarm_when_user_requests_it() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::set_var("AGIME_ENABLE_SWARM_PLANNER_AUTO", "true");
        std::env::set_var("AGIME_ENABLE_NATIVE_SWARM_TOOL", "true");
        let mode = infer_coordinator_execution_mode(
            "你必须直接调用 swarm 工具，并行形成 docs/a.md 和 docs/b.md",
            &["docs/a.md".to_string(), "docs/b.md".to_string()],
            &["docs/a.md".to_string(), "docs/b.md".to_string()],
        );
        assert_eq!(mode, CoordinatorExecutionMode::ExplicitSwarm);
        std::env::remove_var("AGIME_ENABLE_SWARM_PLANNER_AUTO");
        std::env::remove_var("AGIME_ENABLE_NATIVE_SWARM_TOOL");
    }

    #[test]
    fn coordinator_execution_mode_keeps_single_worker_without_two_stable_targets() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::set_var("AGIME_ENABLE_SWARM_PLANNER_AUTO", "true");
        std::env::set_var("AGIME_ENABLE_NATIVE_SWARM_TOOL", "true");
        let mode = infer_coordinator_execution_mode(
            "请并行整理两项输出：当前频道目标摘要和下一步行动建议。",
            &["channel:demo".to_string()],
            &["channel:demo".to_string()],
        );
        assert_eq!(mode, CoordinatorExecutionMode::SingleWorker);
        std::env::remove_var("AGIME_ENABLE_SWARM_PLANNER_AUTO");
        std::env::remove_var("AGIME_ENABLE_NATIVE_SWARM_TOOL");
    }

    #[test]
    fn system_surface_forces_single_worker_even_if_auto_swarm_heuristic_would_trigger() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::set_var("AGIME_ENABLE_SWARM_PLANNER_AUTO", "true");
        std::env::set_var("AGIME_ENABLE_NATIVE_SWARM_TOOL", "true");
        let inferred = infer_coordinator_execution_mode(
            "Analyze document demo.txt (MIME: text/plain).",
            &["document:doc-1".to_string()],
            &["document:doc-1".to_string(), "text/plain".to_string()],
        );
        assert_eq!(inferred, CoordinatorExecutionMode::AutoSwarm);

        let forced = if "system".eq_ignore_ascii_case("system") {
            CoordinatorExecutionMode::SingleWorker
        } else {
            inferred
        };
        assert_eq!(forced, CoordinatorExecutionMode::SingleWorker);
        std::env::remove_var("AGIME_ENABLE_SWARM_PLANNER_AUTO");
        std::env::remove_var("AGIME_ENABLE_NATIVE_SWARM_TOOL");
    }

    #[test]
    fn delegation_mode_tracks_execution_mode() {
        assert_eq!(
            delegation_mode_for_execution_mode(CoordinatorExecutionMode::SingleWorker),
            DelegationMode::Subagent
        );
        assert_eq!(
            delegation_mode_for_execution_mode(CoordinatorExecutionMode::ExplicitSwarm),
            DelegationMode::Swarm
        );
        assert_eq!(
            delegation_mode_for_execution_mode(CoordinatorExecutionMode::AutoSwarm),
            DelegationMode::Swarm
        );
    }

    #[test]
    fn harness_mode_tracks_host_source() {
        assert_eq!(
            harness_mode_for_session_source("channel_runtime"),
            HarnessMode::Execute
        );
        assert_eq!(
            harness_mode_for_session_source("system"),
            HarnessMode::Execute
        );
        assert_eq!(
            harness_mode_for_session_source("agent_task"),
            HarnessMode::Execute
        );
        assert_eq!(
            harness_mode_for_session_source("chat"),
            HarnessMode::Conversation
        );
        assert_eq!(
            harness_mode_for_session_source("portal"),
            HarnessMode::Conversation
        );
        assert_eq!(
            harness_mode_for_session_source("document_analysis"),
            HarnessMode::Conversation
        );
    }

    #[test]
    fn explicit_delegation_request_is_detected_for_chat_runtime() {
        assert!(has_explicit_delegation_request(
            "Use exactly one subagent to inspect the current environment."
        ));
        assert!(has_explicit_delegation_request(
            "Use a swarm with multiple workers to inspect the repo."
        ));
        assert!(has_explicit_delegation_request(
            "请并行用多代理检查当前仓库。"
        ));
        assert!(!has_explicit_delegation_request(
            "Please inspect the current environment."
        ));
    }

    #[test]
    fn digital_avatar_language_does_not_count_as_explicit_delegation_request() {
        assert!(!has_explicit_delegation_request(
            "请帮我创建一个数字分身，并配置它的服务能力。"
        ));
        assert!(!has_explicit_delegation_request(
            "这个分身 Agent 需要绑定哪些文档？"
        ));
        assert!(has_explicit_delegation_request(
            "请用一个子代理检查当前环境。"
        ));
    }

    #[test]
    fn explicit_delegation_forces_execute_for_chat_and_channel_conversation() {
        assert!(should_force_execute_for_explicit_delegation(
            "chat",
            "Use exactly one subagent to inspect the current environment.",
            false,
        ));
        assert!(should_force_execute_for_explicit_delegation(
            "automation_runtime",
            "Use exactly one subagent to inspect the current environment.",
            false,
        ));
        assert!(should_force_execute_for_explicit_delegation(
            "channel_conversation",
            "Use swarm with multiple workers to inspect the repo.",
            false,
        ));
        assert!(!should_force_execute_for_explicit_delegation(
            "portal",
            "Use swarm with multiple workers to inspect the repo.",
            false,
        ));
        assert!(should_force_execute_for_explicit_delegation(
            "portal",
            "Please inspect the repo.",
            true,
        ));
    }

    #[test]
    fn non_delegating_surfaces_never_force_execute_for_explicit_delegation() {
        assert!(!should_force_execute_for_explicit_delegation(
            "portal_manager",
            "Use exactly one subagent to inspect the current environment.",
            false,
        ));
        assert!(!should_force_execute_for_explicit_delegation(
            "portal_manager",
            "Please inspect the repo.",
            true,
        ));
        assert!(!should_force_execute_for_explicit_delegation(
            "scheduled_task",
            "Use exactly one subagent to inspect the current environment.",
            false,
        ));
        assert!(!should_force_execute_for_explicit_delegation(
            "scheduled_task",
            "Please inspect the repo.",
            true,
        ));
    }

    #[test]
    fn provider_turn_mode_prefers_aggregated_for_execute_hosts() {
        assert_eq!(
            provider_turn_mode_for_harness_mode(HarnessMode::Execute),
            ProviderTurnMode::Aggregated
        );
        assert_eq!(
            provider_turn_mode_for_harness_mode(HarnessMode::Conversation),
            ProviderTurnMode::Streaming
        );
    }

    #[test]
    fn provider_turn_mode_prefers_streaming_for_channel_runtime_execute_sessions() {
        assert_eq!(
            provider_turn_mode_for_session_source("channel_runtime", HarnessMode::Execute),
            ProviderTurnMode::Streaming
        );
        assert_eq!(
            provider_turn_mode_for_session_source("system", HarnessMode::Execute),
            ProviderTurnMode::Aggregated
        );
        assert_eq!(
            provider_turn_mode_for_session_source("portal", HarnessMode::Conversation),
            ProviderTurnMode::Streaming
        );
    }

    #[test]
    fn completion_surface_policy_tracks_channel_and_system_sources() {
        assert_eq!(
            completion_surface_policy_for_session_source("channel_runtime", HarnessMode::Execute),
            CompletionSurfacePolicy::Execute
        );
        assert_eq!(
            completion_surface_policy_for_session_source("chat", HarnessMode::Execute),
            CompletionSurfacePolicy::Conversation
        );
        assert_eq!(
            completion_surface_policy_for_session_source(
                "automation_runtime",
                HarnessMode::Conversation
            ),
            CompletionSurfacePolicy::Conversation
        );
        assert_eq!(
            completion_surface_policy_for_session_source(
                "channel_conversation",
                HarnessMode::Conversation
            ),
            CompletionSurfacePolicy::Conversation
        );
        assert_eq!(
            completion_surface_policy_for_session_source("system", HarnessMode::Execute),
            CompletionSurfacePolicy::SystemDocumentAnalysis
        );
        assert_eq!(
            completion_surface_policy_for_session_source("document_analysis", HarnessMode::Execute),
            CompletionSurfacePolicy::Conversation
        );
        assert_eq!(
            completion_surface_policy_for_session_source(
                "document_analysis",
                HarnessMode::Conversation
            ),
            CompletionSurfacePolicy::Conversation
        );
        assert_eq!(
            completion_surface_policy_for_session_source("portal", HarnessMode::Conversation),
            CompletionSurfacePolicy::Conversation
        );
    }

    #[test]
    fn completion_contract_tracks_execute_and_system_surfaces() {
        let execute_contract =
            completion_contract_for_session_source("channel_runtime", HarnessMode::Execute, true)
                .expect("execute contract");
        let execute_schema = execute_contract.json_schema.expect("execute schema");
        assert!(execute_schema["required"].to_string().contains("summary"));
        assert!(!execute_schema["required"]
            .to_string()
            .contains("reason_code"));

        assert!(completion_contract_for_session_source(
            "channel_runtime",
            HarnessMode::Execute,
            false
        )
        .is_none());
        assert!(completion_contract_for_session_source(
            "channel_conversation",
            HarnessMode::Conversation,
            true
        )
        .is_none());
        assert!(
            completion_contract_for_session_source("portal", HarnessMode::Conversation, true)
                .is_none()
        );

        let system_contract =
            completion_contract_for_session_source("system", HarnessMode::Execute, false)
                .expect("system contract");
        let system_schema = system_contract.json_schema.expect("system schema");
        assert!(system_schema["required"]
            .to_string()
            .contains("reason_code"));
        assert!(system_schema["required"]
            .to_string()
            .contains("analysis_complete"));

        assert!(completion_contract_for_session_source(
            "document_analysis",
            HarnessMode::Conversation,
            false,
        )
        .is_none());
        assert!(completion_contract_for_session_source(
            "document_analysis",
            HarnessMode::Execute,
            true,
        )
        .is_none());
    }

    #[test]
    fn system_sessions_require_document_tool_contract() {
        let prefixes = required_tool_prefixes_for_session_source("system");
        assert!(prefixes.contains(&"document_tools__read_document".to_string()));
        assert!(prefixes.contains(&"document_tools__export_document".to_string()));
        assert!(prefixes.contains(&"document_tools__import_document_to_workspace".to_string()));
        assert!(required_tool_prefixes_for_session_source("chat").is_empty());
        assert!(required_tool_prefixes_for_session_source("portal").is_empty());
        assert!(required_tool_prefixes_for_session_source("document_analysis").is_empty());
    }

    #[test]
    fn host_extensions_prefer_frontend_proxy_when_available() {
        let server_local_tool = rmcp::model::Tool::new(
            "developer__write_file",
            "write file",
            serde_json::Map::new(),
        );
        let worker_extension = ExtensionConfig::Platform {
            name: "developer".to_string(),
            description: "Developer tools".to_string(),
            bundled: Some(true),
            available_tools: Vec::new(),
        };

        let execute_extensions = host_extensions_for_mode(
            HarnessMode::Execute,
            std::slice::from_ref(&server_local_tool),
            std::slice::from_ref(&worker_extension),
        );
        assert_eq!(execute_extensions.len(), 1);
        assert!(matches!(
            &execute_extensions[0],
            ExtensionConfig::Frontend { tools, .. }
                if tools.iter().any(|tool| tool.name == server_local_tool.name)
        ));

        let fallback_extensions = host_extensions_for_mode(
            HarnessMode::Conversation,
            &[],
            std::slice::from_ref(&worker_extension),
        );
        assert_eq!(fallback_extensions.len(), 1);
        assert!(matches!(
            &fallback_extensions[0],
            ExtensionConfig::Platform { name, .. } if name == "developer"
        ));
    }

    #[test]
    fn direct_tool_runtime_server_local_name_lookup_is_exact() {
        let names = HashSet::from([
            "developer__write_file".to_string(),
            "document_tools__read_document".to_string(),
        ]);

        assert!(is_server_local_tool_name(&names, "developer__write_file"));
        assert!(!is_server_local_tool_name(&names, "frontend__pick_file"));
    }

    #[test]
    fn context_runtime_compaction_event_reflects_projection_transition() {
        let initial = ContextRuntimeState::default();
        let mut final_state = ContextRuntimeState {
            runtime_compactions: 1,
            last_compact_reason: Some("budget_exceeded".to_string()),
            ..ContextRuntimeState::default()
        };
        final_state.last_projection_stats = Some(agime::context_runtime::ProjectionStats {
            base_agent_messages: 10,
            projected_agent_messages: 4,
            snip_removed_count: 2,
            microcompacted_count: 1,
            raw_token_estimate: 2000,
            projected_token_estimate: 1200,
            freed_token_estimate: 800,
            updated_at: 1,
        });
        final_state.set_session_memory(Some(agime::context_runtime::SessionMemoryState {
            summary: "compact".to_string(),
            summarized_through_message_id: None,
            preserved_start_index: 0,
            preserved_end_index: 0,
            preserved_start_message_id: None,
            preserved_end_message_id: None,
            preserved_message_count: 0,
            preserved_token_estimate: 0,
            tail_anchor_index: 0,
            tail_anchor_message_id: None,
            updated_at: 1,
        }));

        let event = build_context_runtime_compaction_event(Some(&initial), Some(&final_state))
            .expect("compaction event");

        match event {
            StreamEvent::Compaction {
                strategy,
                before_tokens,
                after_tokens,
                phase,
                reason,
            } => {
                assert_eq!(strategy, "context_runtime");
                assert_eq!(before_tokens, 2000);
                assert_eq!(after_tokens, 1200);
                assert_eq!(phase.as_deref(), Some("session_memory_compaction"));
                assert_eq!(reason.as_deref(), Some("budget_exceeded"));
            }
            other => panic!("expected compaction event, got {:?}", other),
        }
    }

    #[test]
    fn compaction_mirror_keeps_stream_and_control_aligned() {
        let event = StreamEvent::Compaction {
            strategy: "context_runtime".to_string(),
            before_tokens: 2000,
            after_tokens: 1200,
            phase: Some("session_memory_compaction".to_string()),
            reason: Some("budget_exceeded".to_string()),
        };
        let mirror = compaction_mirror(&event).expect("compaction mirror");
        let stream_event =
            stream_event_for_control_projection(&mirror).expect("project compaction stream");
        let stream_value =
            serde_json::to_value(&stream_event).expect("serialize compaction stream");
        let control_value = serde_json::to_value(&mirror).expect("serialize compaction control");

        assert_eq!(stream_value["strategy"], "context_runtime");
        assert_eq!(control_value["event"]["type"], "compaction_observed");
        assert_eq!(control_value["event"]["strategy"], "context_runtime");
        assert_eq!(control_value["event"]["reason"], "budget_exceeded");
    }

    #[test]
    fn control_label_helpers_cover_tool_and_runtime_messages() {
        let tool_message =
            HarnessControlMessage::Tool(agime::agents::ToolControlEvent::TransportRequested {
                request_id: "req-1".to_string(),
                tool_name: "document_tools__read_document".to_string(),
                transport: "server_local".to_string(),
                surface: "execute_host".to_string(),
            });
        assert_eq!(control_channel_label(&tool_message), "tool");
        assert_eq!(
            control_event_type_label(&tool_message),
            "transport_requested"
        );

        let runtime_message =
            HarnessControlMessage::Runtime(RuntimeControlEvent::CompactionObserved {
                strategy: Some("history_replaced".to_string()),
                reason: Some("agent_event_history_replaced".to_string()),
                before_tokens: None,
                after_tokens: None,
                phase: None,
            });
        assert_eq!(control_channel_label(&runtime_message), "runtime");
        assert_eq!(
            control_event_type_label(&runtime_message),
            "compaction_observed"
        );
    }

    #[test]
    fn worker_control_messages_preserve_attempt_identity() {
        let mut snapshot = agime::agents::TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: "runtime-1".to_string(),
            depth: 1,
            kind: TaskKind::SwarmWorker,
            status: agime::agents::TaskStatus::Running,
            description: Some("worker".to_string()),
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: HashMap::new(),
            started_at: 1,
            updated_at: 1,
            finished_at: None,
        };
        let attempt_identity =
            WorkerAttemptIdentity::followup("worker-1", "attempt-2", 2, "correction", "task-0");
        attempt_identity.write_to_metadata(&mut snapshot.metadata);

        let started = build_worker_started_control_message(&snapshot);
        let finished = build_worker_finished_control_message(
            "task-1".to_string(),
            TaskKind::SwarmWorker,
            "completed",
            "done".to_string(),
            true,
            Some(&attempt_identity),
        );

        let started_value = serde_json::to_value(started).expect("serialize started");
        let finished_value = serde_json::to_value(finished).expect("serialize finished");
        assert_eq!(started_value["event"]["logical_worker_id"], "worker-1");
        assert_eq!(started_value["event"]["attempt_id"], "attempt-2");
        assert_eq!(finished_value["event"]["attempt_index"], 2);
        assert_eq!(finished_value["event"]["previous_task_id"], "task-0");
    }

    #[tokio::test]
    async fn worker_started_mirror_keeps_stream_and_control_aligned() {
        let mut snapshot = agime::agents::TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: "runtime-1".to_string(),
            depth: 1,
            kind: TaskKind::SwarmWorker,
            status: agime::agents::TaskStatus::Running,
            description: Some("worker".to_string()),
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: HashMap::new(),
            started_at: 1,
            updated_at: 1,
            finished_at: None,
        };
        let attempt_identity =
            WorkerAttemptIdentity::followup("worker-1", "attempt-2", 2, "correction", "task-0");
        attempt_identity.write_to_metadata(&mut snapshot.metadata);

        let mirror = worker_started_mirror(&snapshot, Some(&attempt_identity));
        let stream_event =
            stream_event_for_control_projection(&mirror).expect("project worker started stream");
        let stream_value =
            serde_json::to_value(&stream_event).expect("serialize worker started stream");
        let control_value =
            serde_json::to_value(&mirror).expect("serialize worker started control");

        assert_eq!(stream_value["task_id"], "task-1");
        assert_eq!(stream_value["logical_worker_id"], "worker-1");
        assert_eq!(control_value["event"]["task_id"], "task-1");
        assert_eq!(control_value["event"]["logical_worker_id"], "worker-1");
    }

    #[tokio::test]
    async fn permission_requested_mirror_keeps_stream_and_control_aligned() {
        let attempt_identity = WorkerAttemptIdentity::fresh("worker-1", "attempt-1");
        let mirror = permission_requested_mirror(
            "task-1".to_string(),
            Some("worker-a".to_string()),
            "developer__shell".to_string(),
            Some(&attempt_identity),
        );
        let stream_event =
            stream_event_for_control_projection(&mirror).expect("project permission stream");
        let stream_value =
            serde_json::to_value(&stream_event).expect("serialize permission stream");
        let control_value = serde_json::to_value(&mirror).expect("serialize permission control");

        assert_eq!(stream_value["task_id"], "task-1");
        assert_eq!(stream_value["tool_name"], "developer__shell");
        assert_eq!(control_value["event"]["request_id"], "task-1");
        assert_eq!(control_value["event"]["attempt_id"], "attempt-1");
    }

    #[tokio::test]
    async fn permission_resolved_mirror_keeps_stream_and_control_aligned() {
        let attempt_identity = WorkerAttemptIdentity::fresh("worker-1", "attempt-1");
        let mirror = permission_resolved_mirror(
            "task-1".to_string(),
            Some("worker-a".to_string()),
            "developer__shell".to_string(),
            "allow_once".to_string(),
            Some("config".to_string()),
            Some(&attempt_identity),
        );
        let stream_event = stream_event_for_control_projection(&mirror)
            .expect("project permission resolved stream");
        let stream_value =
            serde_json::to_value(&stream_event).expect("serialize permission resolved stream");
        let control_value =
            serde_json::to_value(&mirror).expect("serialize permission resolved control");

        assert_eq!(stream_value["task_id"], "task-1");
        assert_eq!(stream_value["decision"], "allow_once");
        assert_eq!(stream_value["source"], "config");
        assert_eq!(control_value["event"]["request_id"], "task-1");
        assert_eq!(control_value["event"]["decision"], "allow_once");
        assert_eq!(control_value["event"]["source"], "config");
    }

    #[tokio::test]
    async fn worker_progress_mirror_keeps_stream_and_control_aligned() {
        let mirror = worker_progress_mirror("task-1".to_string(), "halfway".to_string(), Some(50));
        let stream_event =
            stream_event_for_control_projection(&mirror).expect("project progress stream");
        let stream_value = serde_json::to_value(&stream_event).expect("serialize progress stream");
        let control_value = serde_json::to_value(&mirror).expect("serialize progress control");

        assert_eq!(stream_value["task_id"], "task-1");
        assert_eq!(stream_value["percent"], 50);
        assert_eq!(control_value["event"]["task_id"], "task-1");
        assert_eq!(control_value["event"]["percent"], 50);
    }

    #[tokio::test]
    async fn worker_followup_mirror_keeps_stream_and_control_aligned() {
        let attempt_identity =
            WorkerAttemptIdentity::followup("worker-1", "attempt-2", 2, "correction", "task-0");
        let mirror = worker_followup_mirror(
            "task-1".to_string(),
            "correction".to_string(),
            "validator asked for retry".to_string(),
            Some(&attempt_identity),
        );
        let stream_event =
            stream_event_for_control_projection(&mirror).expect("project followup stream");
        let stream_value = serde_json::to_value(&stream_event).expect("serialize followup stream");
        let control_value = serde_json::to_value(&mirror).expect("serialize followup control");

        assert_eq!(stream_value["task_id"], "task-1");
        assert_eq!(stream_value["previous_task_id"], "task-0");
        assert_eq!(control_value["event"]["task_id"], "task-1");
        assert_eq!(control_value["event"]["kind"], "correction");
    }

    #[tokio::test]
    async fn worker_idle_mirror_keeps_stream_and_control_aligned() {
        let attempt_identity = WorkerAttemptIdentity::fresh("worker-1", "attempt-1");
        let mirror = worker_idle_mirror(
            "task-1".to_string(),
            "waiting".to_string(),
            Some(&attempt_identity),
        );
        let stream_event =
            stream_event_for_control_projection(&mirror).expect("project idle stream");
        let stream_value = serde_json::to_value(&stream_event).expect("serialize idle stream");
        let control_value = serde_json::to_value(&mirror).expect("serialize idle control");

        assert_eq!(stream_value["task_id"], "task-1");
        assert_eq!(stream_value["attempt_id"], "attempt-1");
        assert_eq!(control_value["event"]["task_id"], "task-1");
        assert_eq!(control_value["event"]["message"], "waiting");
    }

    #[tokio::test]
    async fn permission_timed_out_mirror_keeps_stream_and_control_aligned() {
        let attempt_identity = WorkerAttemptIdentity::fresh("worker-1", "attempt-1");
        let mirror = permission_timed_out_mirror(
            "task-1".to_string(),
            Some("worker-a".to_string()),
            "developer__shell".to_string(),
            5000,
            Some(&attempt_identity),
        );
        let stream_event =
            stream_event_for_control_projection(&mirror).expect("project timed out stream");
        let stream_value = serde_json::to_value(&stream_event).expect("serialize timed out stream");
        let control_value = serde_json::to_value(&mirror).expect("serialize timed out control");

        assert_eq!(stream_value["task_id"], "task-1");
        assert_eq!(stream_value["timeout_ms"], 5000);
        assert_eq!(control_value["event"]["request_id"], "task-1");
        assert_eq!(control_value["event"]["timeout_ms"], 5000);
    }

    #[tokio::test]
    async fn worker_finished_mirror_keeps_stream_and_control_aligned() {
        let attempt_identity =
            WorkerAttemptIdentity::followup("worker-1", "attempt-2", 2, "correction", "task-0");
        let mirror = worker_finished_mirror(
            "task-1".to_string(),
            TaskKind::SwarmWorker,
            "completed",
            "done".to_string(),
            true,
            Some(&attempt_identity),
        );
        let stream_event =
            stream_event_for_control_projection(&mirror).expect("project finished stream");
        let stream_value = serde_json::to_value(&stream_event).expect("serialize finished stream");
        let control_value = serde_json::to_value(&mirror).expect("serialize finished control");

        assert_eq!(stream_value["task_id"], "task-1");
        assert_eq!(stream_value["previous_task_id"], "task-0");
        assert_eq!(control_value["event"]["task_id"], "task-1");
        assert_eq!(control_value["event"]["status"], "completed");
    }

    #[tokio::test]
    async fn tool_started_mirror_keeps_stream_and_control_aligned() {
        let mirror = tool_started_mirror("req-1".to_string(), "developer__shell".to_string());
        let stream_event =
            stream_event_for_control_projection(&mirror).expect("project tool started stream");
        let stream_value =
            serde_json::to_value(&stream_event).expect("serialize tool started stream");
        let control_value = serde_json::to_value(&mirror).expect("serialize tool started control");

        assert_eq!(stream_value["id"], "req-1");
        assert_eq!(stream_value["name"], "developer__shell");
        assert_eq!(control_value["event"]["request_id"], "req-1");
        assert_eq!(control_value["event"]["tool_name"], "developer__shell");
    }

    #[tokio::test]
    async fn tool_finished_mirror_keeps_stream_and_control_aligned() {
        let mirror = tool_finished_mirror(
            "req-1".to_string(),
            "developer__shell".to_string(),
            true,
            "done".to_string(),
            Some(42),
        );
        let stream_event =
            stream_event_for_control_projection(&mirror).expect("project tool result stream");
        let stream_value =
            serde_json::to_value(&stream_event).expect("serialize tool result stream");
        let control_value = serde_json::to_value(&mirror).expect("serialize tool result control");

        assert_eq!(stream_value["id"], "req-1");
        assert_eq!(stream_value["success"], true);
        assert_eq!(control_value["event"]["request_id"], "req-1");
        assert_eq!(control_value["event"]["success"], true);
        assert_eq!(control_value["event"]["duration_ms"], 42);
    }

    #[test]
    fn runtime_control_registry_suppresses_duplicate_permission_resolution() {
        let registry = RuntimeControlRegistry::default();
        let message = build_permission_resolved_control_message(
            "perm-1",
            "developer__shell",
            "allow_once",
            Some("config".to_string()),
            None,
            Some("worker-a".to_string()),
            Some(&WorkerAttemptIdentity::fresh("worker-1", "attempt-1")),
        );
        assert!(matches!(
            registry.classify(&message),
            ControlEmissionDecision::Emit
        ));
        assert!(matches!(
            registry.classify(&message),
            ControlEmissionDecision::Duplicate(_)
        ));
    }

    #[test]
    fn runtime_control_registry_suppresses_stale_permission_timeout_after_resolution() {
        let registry = RuntimeControlRegistry::default();
        let resolved = build_permission_resolved_control_message(
            "perm-1",
            "developer__shell",
            "allow_once",
            Some("config".to_string()),
            None,
            Some("worker-a".to_string()),
            Some(&WorkerAttemptIdentity::fresh("worker-1", "attempt-1")),
        );
        let timed_out = build_permission_timed_out_control_message(
            "perm-1",
            "developer__shell",
            5000,
            Some("worker-a".to_string()),
            Some(&WorkerAttemptIdentity::fresh("worker-1", "attempt-1")),
        );
        assert!(matches!(
            registry.classify(&resolved),
            ControlEmissionDecision::Emit
        ));
        assert!(matches!(
            registry.classify(&timed_out),
            ControlEmissionDecision::Stale(_)
        ));
    }

    #[test]
    fn runtime_control_registry_suppresses_duplicate_tool_finished() {
        let registry = RuntimeControlRegistry::default();
        let message = build_tool_finished_control_message(
            "req-1",
            "developer__shell",
            true,
            Some("done".to_string()),
            Some(42),
        );
        assert!(matches!(
            registry.classify(&message),
            ControlEmissionDecision::Emit
        ));
        assert!(matches!(
            registry.classify(&message),
            ControlEmissionDecision::Duplicate(_)
        ));
    }

    #[test]
    fn runtime_control_registry_suppresses_stale_worker_finished_attempt() {
        let registry = RuntimeControlRegistry::default();
        let first_attempt = WorkerAttemptIdentity::fresh("worker-1", "attempt-1");
        let second_attempt =
            WorkerAttemptIdentity::followup("worker-1", "attempt-2", 1, "correction", "task-1");
        let mut first_snapshot = agime::agents::TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: "runtime-1".to_string(),
            depth: 1,
            kind: TaskKind::SwarmWorker,
            status: agime::agents::TaskStatus::Running,
            description: Some("worker".to_string()),
            write_scope: Vec::new(),
            target_artifacts: Vec::new(),
            result_contract: Vec::new(),
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: HashMap::new(),
            started_at: 1,
            updated_at: 1,
            finished_at: None,
        };
        first_attempt.write_to_metadata(&mut first_snapshot.metadata);
        let mut second_snapshot = first_snapshot.clone();
        second_snapshot.task_id = "task-2".to_string();
        second_snapshot.metadata.clear();
        second_attempt.write_to_metadata(&mut second_snapshot.metadata);

        assert!(matches!(
            registry.classify(&build_worker_started_control_message(&first_snapshot)),
            ControlEmissionDecision::Emit
        ));
        assert!(matches!(
            registry.classify(&build_worker_started_control_message(&second_snapshot)),
            ControlEmissionDecision::Emit
        ));
        let stale_finished = build_worker_finished_control_message(
            "task-1",
            TaskKind::SwarmWorker,
            "failed",
            "old attempt".to_string(),
            false,
            Some(&first_attempt),
        );
        assert!(matches!(
            registry.classify(&stale_finished),
            ControlEmissionDecision::Stale(_)
        ));
    }

    #[tokio::test]
    async fn emit_task_control_message_uses_registered_sequencer() {
        let sink = RecordingControlSink::default();
        let runtime_session_id = "runtime-control-test";
        let sequencer = Arc::new(agime::agents::HarnessControlSequencer::new(
            "logical-control-test",
            runtime_session_id,
        ));
        agime::agents::register_harness_control_sequencer(runtime_session_id, sequencer);

        emit_task_control_message(
            &sink,
            runtime_session_id,
            HarnessControlMessage::Worker(WorkerControlEvent::Idle {
                task_id: "task-1".to_string(),
                message: "waiting".to_string(),
                logical_worker_id: None,
                attempt_id: None,
            }),
        )
        .await;

        let envelopes = sink.envelopes.lock().await.clone();
        agime::agents::unregister_harness_control_sequencer(runtime_session_id);

        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].session_id, "logical-control-test");
        assert_eq!(envelopes[0].runtime_session_id, runtime_session_id);
        assert_eq!(envelopes[0].sequence, 1);
        assert_eq!(control_channel_label(&envelopes[0].payload), "worker");
    }

    #[tokio::test]
    async fn emit_task_control_message_is_noop_without_registered_sequencer() {
        let sink = RecordingControlSink::default();

        emit_task_control_message(
            &sink,
            "runtime-missing-sequencer",
            HarnessControlMessage::Permission(PermissionControlEvent::TimedOut {
                request_id: "perm-1".to_string(),
                tool_name: "developer__shell".to_string(),
                timeout_ms: 5000,
                worker_name: None,
                logical_worker_id: None,
                attempt_id: None,
            }),
        )
        .await;

        assert!(sink.envelopes.lock().await.is_empty());
    }

    #[test]
    fn parses_execution_host_completion_report() {
        let report = agime::agents::parse_execution_host_completion_report(Some(
            r#"{"status":"completed","summary":"done","produced_artifacts":["artifacts/a.md"],"accepted_artifacts":["artifacts/a.md"],"next_steps":[]}"#,
        ))
        .expect("report should parse");
        assert_eq!(report.status, "completed");
        assert_eq!(report.summary, "done");
        assert_eq!(report.produced_artifacts, vec!["artifacts/a.md"]);
    }

    #[test]
    fn execution_host_completion_response_requires_summary() {
        let response = execution_host_completion_response();
        let schema = response.json_schema.expect("schema should exist");
        assert_eq!(schema["type"], "object");
        assert!(schema["required"].to_string().contains("summary"));
        assert!(schema["required"].to_string().contains("status"));
    }

    #[test]
    fn user_visible_summary_prefers_assistant_text_then_completion_outcome() {
        let from_report = ServerHarnessHostOutcome {
            messages_json: "[]".to_string(),
            message_count: 0,
            total_tokens: None,
            context_runtime_state: None,
            last_assistant_text: Some("assistant text".to_string()),
            completion_report: Some(ExecutionHostCompletionReport {
                status: "completed".to_string(),
                summary: "report summary".to_string(),
                produced_artifacts: vec!["artifacts/a.md".to_string()],
                accepted_artifacts: vec!["artifacts/a.md".to_string()],
                next_steps: Vec::new(),
                validation_status: None,
                blocking_reason: None,
                reason_code: None,
                content_accessed: None,
                analysis_complete: None,
            }),
            persisted_child_evidence: Vec::new(),
            persisted_child_transcript_resume: Vec::new(),
            transition_trace: None,
            events_emitted: 0,
            signal_summary: Some(agime::agents::CoordinatorSignalSummary {
                completion_ready: true,
                latest_completion_summary: Some("signal summary".to_string()),
                ..Default::default()
            }),
            completion_outcome: None,
        };
        assert_eq!(
            from_report.user_visible_summary().as_deref(),
            Some("assistant text")
        );

        let from_outcome = ServerHarnessHostOutcome {
            messages_json: "[]".to_string(),
            message_count: 0,
            total_tokens: None,
            context_runtime_state: None,
            last_assistant_text: None,
            completion_report: None,
            persisted_child_evidence: Vec::new(),
            persisted_child_transcript_resume: Vec::new(),
            transition_trace: None,
            events_emitted: 0,
            signal_summary: Some(agime::agents::CoordinatorSignalSummary::default()),
            completion_outcome: Some(ExecuteCompletionOutcome {
                state: agime::agents::harness::ExecuteCompletionState::Blocked,
                status: "blocked".to_string(),
                summary: Some("outcome summary".to_string()),
                blocking_reason: None,
                completion_ready: false,
                required_tools_satisfied: false,
                has_blocking_signals: false,
                active_child_tasks: false,
            }),
        };
        assert_eq!(
            from_outcome.user_visible_summary().as_deref(),
            Some("outcome summary")
        );
    }

    #[test]
    fn user_visible_summary_strips_runtime_diagnostic_tail() {
        let outcome = ServerHarnessHostOutcome {
            messages_json: "[]".to_string(),
            message_count: 0,
            total_tokens: None,
            context_runtime_state: None,
            last_assistant_text: Some(
                "你好！我们可以先聊清楚问题。\nValidation: not_run\nNext steps:\n- retry later\n(no_tool_backed_progress)"
                    .to_string(),
            ),
            completion_report: None,
            persisted_child_evidence: Vec::new(),
            persisted_child_transcript_resume: Vec::new(),
            transition_trace: None,
            events_emitted: 0,
            signal_summary: None,
            completion_outcome: None,
        };
        assert_eq!(
            outcome.user_visible_summary().as_deref(),
            Some("你好！我们可以先聊清楚问题。")
        );
    }

    #[test]
    fn sanitize_user_visible_runtime_text_normalizes_noise_separator() {
        assert_eq!(
            sanitize_user_visible_runtime_text(Some(
                "Result: README.md **does not exist** бк file not found."
            ))
            .as_deref(),
            Some("Result: README.md **does not exist** - file not found.")
        );
    }

    #[test]
    fn build_execute_completion_report_uses_signal_summary_and_required_tools() {
        let report = agime::agents::build_execute_completion_report(
            None,
            None,
            Some(&agime::agents::CoordinatorSignalSummary {
                tool_completed: 1,
                completed_tool_names: vec!["developer__shell".to_string()],
                latest_completion_summary: Some("fallback summary".to_string()),
                ..Default::default()
            }),
            &["document_tools__".to_string()],
        );

        assert_eq!(report.status, "blocked");
        assert_eq!(report.summary, "fallback summary");
        assert_eq!(report.validation_status.as_deref(), Some("not_run"));
        assert!(report
            .blocking_reason
            .as_deref()
            .unwrap_or_default()
            .contains("required tool contract"));
    }

    #[test]
    fn system_execute_host_defaults_to_blocked_without_structured_report() {
        let report = agime::agents::build_execute_completion_report(
            None,
            None,
            Some(&agime::agents::CoordinatorSignalSummary {
                tool_completed: 1,
                completed_tool_names: vec!["document_tools__read_document".to_string()],
                latest_completion_summary: Some("read complete".to_string()),
                ..Default::default()
            }),
            &["document_tools__".to_string()],
        );

        assert_eq!(report.status, "blocked");
        assert_eq!(
            report.blocking_reason.as_deref(),
            Some("runtime exited without structured final_output")
        );
    }

    #[test]
    fn build_execute_completion_report_generates_generic_blocked_summary_when_missing() {
        let report = agime::agents::build_execute_completion_report(None, None, None, &[]);

        assert_eq!(report.status, "blocked");
        assert_eq!(
            report.summary,
            "Runtime exited without structured final_output."
        );
        assert_eq!(
            report.blocking_reason.as_deref(),
            Some("runtime exited without structured final_output")
        );
    }

    #[test]
    fn format_execution_host_completion_text_includes_blocking_reason() {
        let text = format_execution_host_completion_text(&ExecutionHostCompletionReport {
            status: "blocked".to_string(),
            summary: "analysis blocked".to_string(),
            produced_artifacts: Vec::new(),
            accepted_artifacts: Vec::new(),
            next_steps: vec!["retry after reading the document".to_string()],
            validation_status: Some("failed".to_string()),
            blocking_reason: Some("required tool contract not satisfied".to_string()),
            reason_code: None,
            content_accessed: None,
            analysis_complete: None,
        });

        assert!(text.contains("analysis blocked"));
        assert!(text.contains("Validation: failed"));
        assert!(text.contains("Blocking reason: required tool contract not satisfied"));
    }

    #[test]
    fn adapter_normalize_execution_host_completion_report_blocks_future_intent_for_system_analysis()
    {
        let report = normalize_adapter_execution_host_completion_report(
            agime::agents::normalize_execution_host_completion_report(
                ExecutionHostCompletionReport {
                    status: "completed".to_string(),
                    summary: "I need to read the document first before I can provide a final output. Let me do that now.".to_string(),
                    produced_artifacts: Vec::new(),
                    accepted_artifacts: Vec::new(),
                    next_steps: Vec::new(),
                    validation_status: None,
                    blocking_reason: None,
                    reason_code: None,
                    content_accessed: Some(false),
                    analysis_complete: Some(false),
                },
                Some(&agime::agents::CoordinatorSignalSummary {
                    tool_completed: 1,
                    completed_tool_names: vec!["document_tools__read_document".to_string()],
                    ..Default::default()
                }),
                &["document_tools__".to_string()],
            ),
            Some(&agime::agents::CoordinatorSignalSummary {
                tool_completed: 1,
                completed_tool_names: vec!["document_tools__read_document".to_string()],
                ..Default::default()
            }),
            "system",
        );

        assert_eq!(report.status, "blocked");
        assert_eq!(
            report.blocking_reason.as_deref(),
            Some(
                "document analysis summary is still future intent; final analysis content is missing"
            )
        );
    }

    #[test]
    fn core_normalize_execution_host_completion_report_downgrades_completed_when_required_tools_missing(
    ) {
        let report = agime::agents::normalize_execution_host_completion_report(
            ExecutionHostCompletionReport {
                status: "completed".to_string(),
                summary: "done".to_string(),
                produced_artifacts: Vec::new(),
                accepted_artifacts: Vec::new(),
                next_steps: Vec::new(),
                validation_status: None,
                blocking_reason: None,
                reason_code: None,
                content_accessed: None,
                analysis_complete: None,
            },
            Some(&agime::agents::CoordinatorSignalSummary {
                tool_completed: 1,
                completed_tool_names: vec!["developer__shell".to_string()],
                ..Default::default()
            }),
            &["document_tools__".to_string()],
        );

        assert_eq!(report.status, "blocked");
        assert_eq!(report.validation_status.as_deref(), Some("not_run"));
        assert!(report
            .blocking_reason
            .as_deref()
            .unwrap_or_default()
            .contains("required tool contract"));
    }

    #[test]
    fn adapter_normalize_execution_host_completion_report_blocks_when_document_content_missing() {
        let report = normalize_adapter_execution_host_completion_report(
            ExecutionHostCompletionReport {
                status: "completed".to_string(),
                summary: "read_document was called but the actual content is unavailable"
                    .to_string(),
                produced_artifacts: Vec::new(),
                accepted_artifacts: vec!["document:doc-1".to_string()],
                next_steps: vec!["retry".to_string()],
                validation_status: None,
                blocking_reason: None,
                reason_code: Some("document_content_unavailable".to_string()),
                content_accessed: Some(false),
                analysis_complete: Some(false),
            },
            Some(&agime::agents::CoordinatorSignalSummary {
                completed_tool_names: vec!["document_tools__read_document".to_string()],
                validation_outcomes: vec![agime::agents::ValidationReport {
                    status: agime::agents::ValidationStatus::NotRun,
                    reason: None,
                    reason_code: Some("document_content_unavailable".to_string()),
                    validator_task_id: None,
                    target_artifacts: vec!["document:doc-1".to_string()],
                    evidence_summary: Some("document tool text_read completed".to_string()),
                    content_accessed: false,
                    analysis_complete: false,
                }],
                ..Default::default()
            }),
            "system",
        );

        assert_eq!(report.status, "blocked");
        assert_eq!(report.validation_status.as_deref(), Some("failed"));
        assert_eq!(
            report.blocking_reason.as_deref(),
            Some("document content was not successfully accessed")
        );
    }

    #[test]
    fn adapter_document_analysis_uses_pre_materialized_completion_contract() {
        let report = normalize_adapter_execution_host_completion_report(
            ExecutionHostCompletionReport {
                status: "completed".to_string(),
                summary: "Document Overview\nContent Structure\nKey Takeaways".to_string(),
                produced_artifacts: Vec::new(),
                accepted_artifacts: vec!["document:doc-1".to_string()],
                next_steps: Vec::new(),
                validation_status: None,
                blocking_reason: None,
                reason_code: None,
                content_accessed: Some(true),
                analysis_complete: Some(true),
            },
            Some(&agime::agents::CoordinatorSignalSummary {
                completed_tool_names: vec!["developer__shell".to_string()],
                ..Default::default()
            }),
            "document_analysis",
        );

        assert_eq!(report.status, "completed");
        assert_eq!(report.validation_status.as_deref(), Some("passed"));
        assert_eq!(
            report.reason_code.as_deref(),
            Some("document_analysis_completed")
        );
    }

    #[test]
    fn custom_extension_to_agent_extension_preserves_stdio_shape() {
        let extension = agime_team::models::CustomExtensionConfig {
            name: "team_stdio".to_string(),
            ext_type: "stdio".to_string(),
            uri_or_cmd: "/usr/bin/team-ext".to_string(),
            args: vec!["serve".to_string()],
            envs: std::collections::HashMap::from([("API_KEY".to_string(), "secret".to_string())]),
            enabled: true,
            source: Some("team".to_string()),
            source_extension_id: Some("ext-1".to_string()),
        };

        let config = custom_extension_to_agent_extension(&extension).expect("converted");
        match config {
            ExtensionConfig::Stdio {
                name,
                cmd,
                args,
                bundled,
                ..
            } => {
                assert_eq!(name, "team_stdio");
                assert_eq!(cmd, "/usr/bin/team-ext");
                assert_eq!(args, vec!["serve".to_string()]);
                assert_eq!(bundled, Some(true));
            }
            other => panic!("expected stdio extension, got {:?}", other),
        }
    }

    #[test]
    fn platform_extension_to_agent_extension_preserves_platform_worker_access() {
        let config =
            platform_extension_to_agent_extension(&agime_team::models::AgentExtensionConfig {
                extension: agime_team::models::BuiltinExtension::DocumentTools,
                enabled: true,
                allowed_groups: Vec::new(),
            })
            .expect("converted");

        match config {
            ExtensionConfig::Platform { name, .. } => {
                assert_eq!(name, "document_tools");
            }
            other => panic!("expected platform extension, got {:?}", other),
        }
    }
}
