use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use agime::agents::extension::ExtensionConfig;
#[cfg(test)]
use agime::agents::format_execution_host_completion_text;
use agime::agents::harness::{
    auto_resolve_request, normalize_system_document_analysis_completion_report,
    CompletionSurfacePolicy, PermissionBridgeResolution,
};
use agime::agents::{
    clear_permission_bridge_resolver, native_swarm_tool_enabled, planner_auto_swarm_enabled,
    run_harness_host, set_permission_bridge_resolver, Agent, AgentEvent, CoordinatorExecutionMode,
    DelegationCapabilityContext, DelegationMode, ExecuteCompletionOutcome,
    ExecutionHostCompletionReport, HarnessEventSink, HarnessHostDependencies, HarnessHostRequest,
    HarnessMode, HarnessPersistenceAdapter, ProviderTurnMode, SessionConfig, TaskKind, TaskRuntime,
    TaskRuntimeEvent, TaskRuntimeHost,
};
use agime::conversation::message::{
    ActionRequiredData, FrontendToolRequest, Message, MessageContent, SystemNotificationType,
};
use agime::conversation::Conversation;
use agime::mcp_utils::ToolResult;
use agime::permission::permission_confirmation::PrincipalType;
use agime::permission::{Permission, PermissionConfirmation};
use agime::providers::base::Provider;
use agime::session::SessionType;
use agime_team::models::{ApiFormat, ApprovalMode, TeamAgent};
use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use regex::Regex;
use rmcp::model::{CallToolRequestParams, CallToolResult, Content, ErrorCode, ErrorData};
use serde_json::Value;
use std::sync::OnceLock;
use tokio::sync::{watch, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::agent_prompt_composer::{compose_top_level_prompt, AgentPromptComposerInput};
use super::capability_policy::AgentRuntimePolicyResolver;
use super::chat_channel_manager::ChatChannelManager;
use super::chat_manager::ChatManager;
use super::executor_mongo::{
    agent_has_extension_manager_enabled, build_api_caller, builtin_extension_configs_to_custom,
    find_extension_config_by_name, resolve_agent_attached_team_extensions,
    resolve_agent_custom_extensions, TaskExecutor, TeamRuntimeSettings, TeamSkillMode,
};
use super::extension_installer::ExtensionInstaller;
use super::extension_manager_client::{DynamicExtensionState, TeamExtensionManagerClient};
use super::mcp_connector::{ElicitationBridgeCallback, McpConnector, ToolContentBlock};
use super::platform_runner::PlatformExtensionRunner;
use super::runtime;
use super::service_mongo::AgentService;
use super::session_mongo::AgentSessionDoc;
use super::task_manager::{StreamEvent, TaskManager};
use super::workspace_service::WorkspaceService;

const DIRECT_HARNESS_HOST_FLAG: &str = "TEAM_ENABLE_DIRECT_HARNESS_HOST";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostExecutionPath {
    Bridge,
    DirectHarness,
}

pub fn direct_harness_host_enabled() -> bool {
    std::env::var(DIRECT_HARNESS_HOST_FLAG)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub fn current_host_execution_path() -> HostExecutionPath {
    if direct_harness_host_enabled() {
        HostExecutionPath::DirectHarness
    } else {
        HostExecutionPath::Bridge
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
    pub last_assistant_text: Option<String>,
    pub completion_report: Option<ExecutionHostCompletionReport>,
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

    let cleaned = cleaned_lines.join("\n").trim().to_string();
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
}

struct ServerHarnessEventSink {
    broadcaster: StreamBroadcaster,
    tool_runtime: Arc<DirectToolRuntime>,
}

fn is_server_local_tool_name(server_local_tool_names: &HashSet<String>, tool_name: &str) -> bool {
    server_local_tool_names.contains(tool_name)
}

#[derive(Clone)]
struct SessionHostPersistenceAdapter {
    agent_service: Arc<AgentService>,
    session_id: String,
    fallback_title: Option<String>,
    generated_title: Option<String>,
    runtime_session_tx: Option<watch::Sender<Option<String>>>,
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
    if !session.attached_document_ids.is_empty() {
        targets.extend(
            session
                .attached_document_ids
                .iter()
                .filter(|value| !value.trim().is_empty())
                .map(|value| format!("document:{}", value.trim())),
        );
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
    } else if session_source.eq_ignore_ascii_case("channel_conversation") {
        HarnessMode::Conversation
    } else {
        HarnessMode::Conversation
    }
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
    if session_source.eq_ignore_ascii_case("system") {
        report = normalize_system_document_analysis_completion_report(report, signal_summary);
    } else if report.status == "completed"
        && signal_summary.is_some_and(|summary| summary.has_blocking_signals())
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

#[async_trait::async_trait]
impl HarnessPersistenceAdapter for SessionHostPersistenceAdapter {
    async fn on_started(
        &self,
        _logical_session_id: &str,
        runtime_session_id: &str,
        _initial_conversation: &Conversation,
        _mode: HarnessMode,
    ) -> Result<()> {
        if let Some(tx) = &self.runtime_session_tx {
            let _ = tx.send(Some(runtime_session_id.to_string()));
        }
        self.agent_service
            .set_session_runtime_session_id(&self.session_id, runtime_session_id)
            .await?;
        self.agent_service
            .set_session_processing(&self.session_id, true)
            .await?;
        Ok(())
    }

    async fn on_finished(
        &self,
        _logical_session_id: &str,
        _runtime_session_id: &str,
        final_conversation: &Conversation,
        _mode: HarnessMode,
        total_tokens: Option<i32>,
        _input_tokens: Option<i32>,
        _output_tokens: Option<i32>,
    ) -> Result<()> {
        let messages_json = serde_json::to_string(final_conversation.messages())?;
        let preview = runtime::extract_last_assistant_text(&messages_json).unwrap_or_default();
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
            )
            .await?;
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
) -> JoinHandle<()> {
    tokio::spawn(async move {
        #[derive(Clone)]
        struct TrackedTaskState {
            kind: TaskKind,
            target: Option<String>,
            attempt_identity: Option<agime::agents::WorkerAttemptIdentity>,
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
                            agime::agents::WorkerAttemptIdentity::from_metadata(&snapshot.metadata);
                        tracked_tasks.insert(
                            snapshot.task_id.clone(),
                            TrackedTaskState {
                                kind: snapshot.kind,
                                target: snapshot.target_artifacts.first().cloned(),
                                attempt_identity: attempt_identity.clone(),
                            },
                        );
                        broadcaster
                            .emit(StreamEvent::WorkerStarted {
                                task_id: snapshot.task_id,
                                kind: task_kind_label(snapshot.kind),
                                target: snapshot.target_artifacts.first().cloned(),
                                logical_worker_id: attempt_identity
                                    .as_ref()
                                    .map(|value| value.logical_worker_id.clone()),
                                attempt_id: attempt_identity
                                    .as_ref()
                                    .map(|value| value.attempt_id.clone()),
                                attempt_index: attempt_identity
                                    .as_ref()
                                    .map(|value| value.attempt_index),
                                previous_task_id: attempt_identity
                                    .and_then(|value| value.previous_task_id.clone()),
                            })
                            .await;
                    }
                }
                Ok(TaskRuntimeEvent::Progress {
                    task_id,
                    message,
                    percent,
                }) => {
                    if tracked_tasks.contains_key(&task_id) {
                        broadcaster
                            .emit(StreamEvent::WorkerProgress {
                                task_id,
                                message,
                                percent,
                            })
                            .await;
                    }
                }
                Ok(TaskRuntimeEvent::FollowupRequested {
                    task_id,
                    kind,
                    reason,
                }) => {
                    if let Some(state) = tracked_tasks.get(&task_id) {
                        broadcaster
                            .emit(StreamEvent::WorkerFollowup {
                                task_id,
                                kind,
                                reason,
                                logical_worker_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.logical_worker_id.clone()),
                                attempt_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.attempt_id.clone()),
                                attempt_index: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.attempt_index),
                                previous_task_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .and_then(|value| value.previous_task_id.clone()),
                            })
                            .await;
                    }
                }
                Ok(TaskRuntimeEvent::Idle { task_id, message }) => {
                    if let Some(state) = tracked_tasks.get(&task_id) {
                        broadcaster
                            .emit(StreamEvent::WorkerIdle {
                                task_id,
                                message,
                                logical_worker_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.logical_worker_id.clone()),
                                attempt_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.attempt_id.clone()),
                            })
                            .await;
                    }
                }
                Ok(TaskRuntimeEvent::PermissionRequested {
                    task_id,
                    worker_name,
                    tool_name,
                }) => {
                    if let Some(state) = tracked_tasks.get(&task_id) {
                        broadcaster
                            .emit(StreamEvent::PermissionRequested {
                                task_id,
                                tool_name,
                                worker_name,
                                logical_worker_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.logical_worker_id.clone()),
                                attempt_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.attempt_id.clone()),
                            })
                            .await;
                    }
                }
                Ok(TaskRuntimeEvent::PermissionResolved {
                    task_id,
                    worker_name,
                    tool_name,
                    decision,
                }) => {
                    if let Some(state) = tracked_tasks.get(&task_id) {
                        broadcaster
                            .emit(StreamEvent::PermissionResolved {
                                task_id,
                                tool_name,
                                decision,
                                worker_name,
                                logical_worker_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.logical_worker_id.clone()),
                                attempt_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.attempt_id.clone()),
                            })
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
                        broadcaster
                            .emit(StreamEvent::PermissionTimedOut {
                                task_id,
                                tool_name,
                                timeout_ms,
                                worker_name,
                                logical_worker_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.logical_worker_id.clone()),
                                attempt_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.attempt_id.clone()),
                            })
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
                        broadcaster
                            .emit(StreamEvent::WorkerFinished {
                                task_id: result.task_id,
                                kind: task_kind_label(result.kind),
                                status: "completed".to_string(),
                                summary: result.summary,
                                produced_delta: Some(result.produced_delta),
                                logical_worker_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.logical_worker_id.clone()),
                                attempt_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.attempt_id.clone()),
                                attempt_index: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.attempt_index),
                                previous_task_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .and_then(|value| value.previous_task_id.clone()),
                            })
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
                        broadcaster
                            .emit(StreamEvent::WorkerFinished {
                                task_id: result.task_id,
                                kind: task_kind_label(result.kind),
                                status: "failed".to_string(),
                                summary: result.summary,
                                produced_delta: Some(result.produced_delta),
                                logical_worker_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.logical_worker_id.clone()),
                                attempt_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.attempt_id.clone()),
                                attempt_index: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.attempt_index),
                                previous_task_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .and_then(|value| value.previous_task_id.clone()),
                            })
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
                        broadcaster
                            .emit(StreamEvent::WorkerFinished {
                                task_id,
                                kind: task_kind_label(state.kind),
                                status: "cancelled".to_string(),
                                summary: "worker cancelled".to_string(),
                                produced_delta: Some(false),
                                logical_worker_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.logical_worker_id.clone()),
                                attempt_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.attempt_id.clone()),
                                attempt_index: state
                                    .attempt_identity
                                    .as_ref()
                                    .map(|value| value.attempt_index),
                                previous_task_id: state
                                    .attempt_identity
                                    .as_ref()
                                    .and_then(|value| value.previous_task_id.clone()),
                            })
                            .await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    })
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
            AgentEvent::Message(message) => self.handle_message(agent, message).await,
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
        self.broadcaster
            .emit(StreamEvent::ToolCall {
                name: tool_name.clone(),
                id: request_id.to_string(),
            })
            .await;

        let args = Value::Object(tool_call.arguments.clone().unwrap_or_default());
        let (duration_ms, result) =
            execute_direct_tool_call(self.tool_runtime.clone(), &tool_name, args).await;

        match result {
            Ok(blocks) => {
                let content = TaskExecutor::tool_blocks_summary(&blocks);
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
                self.broadcaster
                    .emit(StreamEvent::ToolResult {
                        id: request_id.to_string(),
                        success: true,
                        content: content.clone(),
                        name: Some(tool_name),
                        duration_ms: Some(duration_ms),
                    })
                    .await;
                agent
                    .handle_tool_result(request_id.to_string(), Ok(blocks_to_call_result(&blocks)))
                    .await;
            }
            Err(err) => {
                let payload = super::hook_runtime::build_post_tool_use_failure_payload(
                    logical_session_id,
                    Some(runtime_session_id.to_string()),
                    request_id,
                    &tool_name,
                    err.clone(),
                );
                super::hook_runtime::emit_post_tool_use_failure_payload(&payload);
                self.broadcaster
                    .emit(StreamEvent::ToolResult {
                        id: request_id.to_string(),
                        success: false,
                        content: err.clone(),
                        name: Some(tool_name),
                        duration_ms: Some(duration_ms),
                    })
                    .await;
                agent
                    .handle_tool_result(
                        request_id.to_string(),
                        Err(ErrorData::new(ErrorCode::INTERNAL_ERROR, err, None)),
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

    async fn handle_message(&self, agent: &Agent, message: &Message) -> Result<()> {
        for content in &message.content {
            match content {
                MessageContent::Text(text)
                    if message.metadata.user_visible
                        && message.role == rmcp::model::Role::Assistant =>
                {
                    self.broadcaster
                        .emit(StreamEvent::Text {
                            content: text.text.clone(),
                        })
                        .await;
                }
                MessageContent::Thinking(thinking) if message.metadata.user_visible => {
                    self.broadcaster
                        .emit(StreamEvent::Thinking {
                            content: thinking.thinking.clone(),
                        })
                        .await;
                }
                MessageContent::SystemNotification(notification)
                    if message.metadata.user_visible =>
                {
                    let event = match notification.notification_type {
                        SystemNotificationType::ThinkingMessage => StreamEvent::Thinking {
                            content: notification.msg.clone(),
                        },
                        SystemNotificationType::InlineMessage => StreamEvent::Text {
                            content: notification.msg.clone(),
                        },
                        SystemNotificationType::RuntimeNotificationAttachment => {
                            continue;
                        }
                    };
                    self.broadcaster.emit(event).await;
                }
                MessageContent::FrontendToolRequest(request) => {
                    self.handle_frontend_tool_request(
                        "frontend-message",
                        "frontend-message",
                        agent,
                        request,
                    )
                    .await?;
                }
                MessageContent::ActionRequired(action) => {
                    if let ActionRequiredData::ToolConfirmation { id, tool_name, .. } = &action.data
                    {
                        self.broadcaster
                            .emit(StreamEvent::Status {
                                status: format!("auto_approve:{}", tool_name),
                            })
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

        self.broadcaster
            .emit(StreamEvent::Status {
                status: format!("unsupported_external_frontend:{}", tool_name),
            })
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
        let inferred_targets = if session.session_source.eq_ignore_ascii_case("system") {
            Vec::new()
        } else {
            infer_targets_from_user_message(user_message)
        };
        let effective_target_artifacts = {
            let base_targets = if target_artifacts.is_empty() {
                host_target_artifacts(session)
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
                host_result_contract(session)
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

        let runtime_snapshot =
            AgentRuntimePolicyResolver::resolve(&effective_agent, Some(session), None);
        let coordinator_execution_mode = if session.session_source.eq_ignore_ascii_case("system") {
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
        let delegation_mode = match coordinator_execution_mode {
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
        };
        let effective_validation_mode =
            validation_mode && runtime_snapshot.delegation_policy.allow_validation_worker;
        let require_final_report =
            session.require_final_report || runtime_snapshot.delegation_policy.require_final_report;

        let prepared = self
            .prepare_runtime(
                session,
                &effective_agent,
                &workspace_path,
                turn_system_instruction,
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
            task_id: format!("direct-host:{}", Uuid::new_v4()),
            cancel_token: cancel_token.clone(),
            tool_timeout_secs: prepared.tool_timeout_secs,
            server_local_tool_names: prepared
                .server_local_tools
                .iter()
                .map(|tool| tool.name.to_string())
                .collect(),
        });
        let generated_title = if session.title.is_none() {
            derive_chat_title_from_user_message(user_message)
        } else {
            None
        };
        let event_sink = Arc::new(ServerHarnessEventSink {
            broadcaster,
            tool_runtime,
        });
        let task_runtime = Arc::new(TaskRuntime::default());
        let (runtime_session_tx, runtime_session_rx) = watch::channel(None);
        let worker_forwarder = spawn_task_runtime_forwarder(
            task_runtime.clone(),
            runtime_session_rx,
            event_sink.broadcaster.clone(),
        );
        let initial_messages: Vec<Message> =
            serde_json::from_str(&session.messages_json).unwrap_or_default();
        let initial_conversation = Conversation::new_unvalidated(initial_messages);
        let agent_instance = Arc::new(Agent::new());
        agent_instance
            .set_delegation_capability_context(Some(DelegationCapabilityContext {
                allow_plan_mode: runtime_snapshot.delegation_policy.allow_plan,
                allow_subagent: runtime_snapshot.delegation_policy.allow_subagent,
                allow_swarm: runtime_snapshot.delegation_policy.allow_swarm,
                allow_worker_messaging: runtime_snapshot.delegation_policy.allow_worker_messaging,
            }))
            .await;
        let harness_mode = harness_mode_for_session_source(&session.session_source);
        let mut server_local_tool_names = prepared
            .server_local_tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        server_local_tool_names.sort();
        server_local_tool_names.dedup();
        let host_extensions = host_extensions_for_mode(
            harness_mode,
            &prepared.server_local_tools,
            &prepared.worker_extensions,
        );
        let completion_surface_policy = if session.session_source.eq_ignore_ascii_case("system") {
            CompletionSurfacePolicy::SystemDocumentAnalysis
        } else if matches!(harness_mode, HarnessMode::Conversation) {
            CompletionSurfacePolicy::Conversation
        } else {
            CompletionSurfacePolicy::Execute
        };
        let completion_contract = if session.session_source.eq_ignore_ascii_case("system") {
            Some(document_analysis_completion_response())
        } else {
            (matches!(harness_mode, HarnessMode::Execute) && require_final_report)
                .then(execution_host_completion_response)
        };
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
                worker_extensions: prepared.worker_extensions.clone(),
                task_runtime: Some(task_runtime.clone()),
                system_prompt_override: Some(prepared.system_prompt.clone()),
                system_prompt_extras: Vec::new(),
                extensions: host_extensions,
                final_output: completion_contract,
                cancel_token: Some(cancel_token.clone()),
            },
            HarnessHostDependencies {
                agent: agent_instance.clone(),
                event_sink,
                persistence: Arc::new(SessionHostPersistenceAdapter {
                    agent_service: self.agent_service.clone(),
                    session_id: session.session_id.clone(),
                    fallback_title: session.title.clone(),
                    generated_title,
                    runtime_session_tx: Some(runtime_session_tx),
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
                let _ = workspace_service.record_completion_artifacts(
                    &workspace,
                    &report.produced_artifacts,
                    &report.accepted_artifacts,
                );
            }
        }
        let preview = sanitize_user_visible_runtime_text(
            runtime::extract_last_assistant_text(&messages_json).as_deref(),
        )
        .or_else(|| {
            host_result
                .completion_outcome
                .as_ref()
                .and_then(|outcome| sanitize_user_visible_runtime_text(outcome.summary.as_deref()))
        })
        .unwrap_or_default();
        let execution_status = completion_report
            .as_ref()
            .map(|report| report.status.as_str())
            .unwrap_or("blocked");
        let execution_error = completion_report.as_ref().and_then(|report| {
            if report.status == "completed" {
                None
            } else {
                report
                    .blocking_reason
                    .clone()
                    .or_else(|| Some("execute host returned blocked completion".to_string()))
            }
        });
        self.agent_service
            .update_session_execution_result(
                &session.session_id,
                execution_status,
                execution_error.as_deref(),
            )
            .await?;
        tracing::info!(
            logical_session_id = %session.session_id,
            execution_status,
            "ServerHarnessHost: logical session marked finished"
        );

        self.persist_extension_overrides(&effective_agent, session, &prepared)
            .await;
        self.shutdown_runtime(prepared.dynamic_state).await;

        Ok(ServerHarnessHostOutcome {
            messages_json,
            message_count: host_result.final_conversation.len() as i32,
            total_tokens: host_result.total_tokens,
            last_assistant_text: if preview.trim().is_empty() {
                None
            } else {
                Some(preview)
            },
            completion_report,
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
        let runtime_snapshot = AgentRuntimePolicyResolver::resolve(agent, Some(session), None);

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
                .retain(|ext| allowed_extension_names.contains(&ext.name.to_ascii_lowercase()));
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
        let overrides = runtime::compute_extension_overrides(agent, &active_set);
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

async fn execute_direct_tool_call(
    runtime: Arc<DirectToolRuntime>,
    tool_name: &str,
    args: Value,
) -> (u64, Result<Vec<ToolContentBlock>, String>) {
    if let Some(manager) = runtime.extension_manager.as_ref() {
        if TeamExtensionManagerClient::can_handle(tool_name) {
            let started_at = std::time::Instant::now();
            let result = if let Some(timeout_secs) = runtime.tool_timeout_secs {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(timeout_secs),
                    manager.call_tool_rich(tool_name, args),
                )
                .await
                {
                    Ok(Ok(blocks)) => Ok(blocks),
                    Ok(Err(error)) => Err(format!("Error: {}", error)),
                    Err(_) => Err(format!(
                        "Error: tool '{}' timed out after {}s",
                        tool_name, timeout_secs
                    )),
                }
            } else {
                manager
                    .call_tool_rich(tool_name, args)
                    .await
                    .map_err(|error| format!("Error: {}", error))
            };
            return (started_at.elapsed().as_millis() as u64, result);
        }
    }

    TaskExecutor::execute_standard_tool_call(
        runtime.dynamic_state.clone(),
        runtime.task_manager.clone(),
        runtime.task_id.clone(),
        runtime.cancel_token.child_token(),
        runtime.tool_timeout_secs,
        tool_name.to_string(),
        args,
    )
    .await
}

fn blocks_to_call_result(blocks: &[ToolContentBlock]) -> CallToolResult {
    let content = blocks
        .iter()
        .filter_map(|block| match block {
            ToolContentBlock::Text(text) => Some(Content::text(text.clone())),
            ToolContentBlock::Image { mime_type, data } => {
                Some(Content::image(data.clone(), mime_type.clone()))
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
    fn direct_host_flag_defaults_off() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::remove_var(DIRECT_HARNESS_HOST_FLAG);
        assert!(!direct_harness_host_enabled());
        assert_eq!(current_host_execution_path(), HostExecutionPath::Bridge);
    }

    #[test]
    fn direct_host_flag_honors_true_like_values() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::set_var(DIRECT_HARNESS_HOST_FLAG, "true");
        assert!(direct_harness_host_enabled());
        assert_eq!(
            current_host_execution_path(),
            HostExecutionPath::DirectHarness
        );
        std::env::remove_var(DIRECT_HARNESS_HOST_FLAG);
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
    fn host_targets_include_channel_thread_and_documents() {
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
            compaction_count: 0,
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
            hidden_from_chat_list: false,
        };

        let targets = host_target_artifacts(&session);
        assert!(targets.contains(&"channel:channel-1/thread:thread-1".to_string()));
        assert!(targets.contains(&"document:doc-a".to_string()));
        assert!(targets.contains(&"document:doc-b".to_string()));
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
            compaction_count: 0,
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
            hidden_from_chat_list: false,
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
            harness_mode_for_session_source("chat"),
            HarnessMode::Conversation
        );
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
            last_assistant_text: None,
            completion_report: None,
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
            last_assistant_text: Some(
                "你好！我们可以先聊清楚问题。\nValidation: not_run\nNext steps:\n- retry later\n(no_tool_backed_progress)"
                    .to_string(),
            ),
            completion_report: None,
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
            Some("document analysis summary is still future intent; final analysis content is missing")
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
