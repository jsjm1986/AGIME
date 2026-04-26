use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::final_output_tool::FinalOutputTool;
use super::harness::transcript::{HarnessCheckpointStore, HarnessTranscriptStore};
use super::harness::{
    apply_runtime_policy, create_send_message_tool, drain_unread_messages_from_root,
    format_runtime_policy_denial, handle_send_message_tool, load_host_session_state,
    load_worker_runtime_state, mailbox_message_to_notification, parse_harness_mode_command,
    shared_transition_trace, spawn_task_runtime_signal_bridge, task_runtime_for_session,
    update_host_notification_summary, update_host_transition_trace, CompletionSurfacePolicy,
    CoordinatorExecutionMode, DelegationRuntimeState, HarnessContext, HarnessMode, HarnessPolicy,
    HarnessRunLoop, HarnessState, HarnessWorkerRuntimeContext, PolicyDecision, ProviderTurnMode,
    SessionHarnessStore, TaskRuntime, ToolBatchMode, ToolInvocationSurface, ToolTransportKind,
    SEND_MESSAGE_TOOL_NAME,
};
use super::tool_execution::{ToolCallResult, DECLINED_RESPONSE};
use crate::action_required_manager::ActionRequiredManager;
use crate::agents::extension::{
    ExtensionConfig, ExtensionError, ExtensionResult, PlatformExtensionContext, ToolInfo,
};
use crate::agents::extension_manager::{get_parameter_names, ExtensionManager};
use crate::agents::final_output_tool::FINAL_OUTPUT_TOOL_NAME;
use crate::agents::prompt_manager::PromptManager;
use crate::agents::retry::{RetryManager, RetryResult};
use crate::agents::router_tools::ROUTER_LLM_SEARCH_TOOL_NAME;
use crate::agents::subagent_task_config::{TaskConfig, WorkerCapabilityContext};
use crate::agents::subagent_tool::{
    create_subagent_tool, handle_subagent_tool, SUBAGENT_TOOL_NAME,
};
use crate::agents::swarm_tool::{create_swarm_tool, handle_swarm_tool, SWARM_TOOL_NAME};
use crate::agents::task_board::TaskBoardContext;
use crate::agents::tool_route_manager::ToolRouteManager;
use crate::agents::tool_router_index_manager::ToolRouterIndexManager;
use crate::agents::types::SessionConfig;
use crate::agents::types::{FrontendTool, SharedProvider, ToolResultReceiver};
use crate::config::{get_enabled_extensions, AgimeMode, Config};
use crate::conversation::message::{
    ActionRequiredData, Message, MessageContent, SystemNotificationType, ToolRequest,
};
use crate::conversation::{debug_conversation_fix, fix_conversation, Conversation};
use crate::mcp_utils::ToolResult;
use crate::permission::permission_inspector::PermissionInspector;
use crate::permission::permission_judge::PermissionCheckResult;
use crate::permission::PermissionConfirmation;
use crate::providers::base::Provider;
use crate::recipe::{Author, Recipe, Response, Settings, SubRecipe};
use crate::security::security_inspector::SecurityInspector;
use crate::session::extension_data::{EnabledExtensionsState, ExtensionState};
use crate::session::{Session, SessionManager};
use crate::tool_inspection::ToolInspectionManager;
use crate::tool_monitor::RepetitionInspector;
use crate::utils::is_token_cancelled;
use anyhow::{anyhow, Context, Result};
use futures::stream::BoxStream;
use futures::{stream, FutureExt, Stream, StreamExt};
use regex::Regex;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, ErrorCode, ErrorData, GetPromptResult, Prompt,
    ServerNotification, Tool,
};
use serde_json::Value;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, instrument, warn};

const DEFAULT_MAX_TURNS: u32 = 1000;
const MAX_COMPACTION_ATTEMPTS: u32 = 3; // Maximum compaction attempts per reply to prevent infinite loops
pub const MANUAL_COMPACT_TRIGGERS: &[&str] =
    &["Please compact this conversation", "/compact", "/summarize"];

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

pub(crate) struct FrontendTransportHandling {
    pub(crate) events: Vec<AgentEvent>,
    pub(crate) pending_request_ids: Vec<String>,
    pub(crate) request_transports:
        std::collections::HashMap<String, crate::agents::harness::ToolTransportKind>,
}

pub(crate) struct ScheduledBatchExecutionResult {
    pub(crate) executed_tool_calls: usize,
    pub(crate) all_install_successful: bool,
    pub(crate) events: Vec<AgentEvent>,
}

pub(crate) struct BackendToolExecutionResult {
    pub(crate) approval_messages: Vec<Message>,
    pub(crate) batch_events: Vec<AgentEvent>,
    pub(crate) tools_updated: bool,
    pub(crate) executed_tool_calls: usize,
}

pub(crate) struct ModelResponseHandling {
    pub(crate) events: Vec<AgentEvent>,
    pub(crate) no_tools_called: bool,
    pub(crate) tools_updated: bool,
    pub(crate) yield_after_first_event: bool,
    pub(crate) deferred_tool_handling: Option<DeferredToolHandling>,
}

pub(crate) struct DeferredToolHandling {
    pub(crate) tool_response_plan: crate::agents::harness::ToolResponsePlan,
    pub(crate) pending_frontend_request_ids: Vec<String>,
    pub(crate) frontend_request_ids: Vec<String>,
    pub(crate) frontend_request_transports:
        std::collections::HashMap<String, crate::agents::harness::ToolTransportKind>,
}

pub(crate) struct ProviderSuccessHandling {
    pub(crate) pre_response_events: Vec<AgentEvent>,
    pub(crate) response_handling: Option<ModelResponseHandling>,
}

pub(crate) enum ProviderErrorHandling {
    ContinueTurn {
        conversation: Conversation,
        events: Vec<AgentEvent>,
        did_recovery_compact_this_iteration: bool,
    },
    BreakLoop {
        events: Vec<AgentEvent>,
    },
}

pub(crate) struct TurnFinalization {
    pub(crate) events: Vec<AgentEvent>,
    pub(crate) next_mode: HarnessMode,
    pub(crate) exit_chat: bool,
}

pub(crate) enum TurnStartHandling {
    Continue,
    BreakWithMessage(Message),
}

pub(crate) struct PreparedTurnInput {
    pub(crate) conversation_for_model: Conversation,
    pub(crate) effective_system_prompt: String,
}

pub(crate) struct ReplyBootstrap {
    pub(crate) session: Session,
    pub(crate) conversation: Conversation,
    pub(crate) context_runtime_state: crate::context_runtime::ContextRuntimeState,
    pub(crate) current_mode: HarnessMode,
    pub(crate) needs_auto_compact: bool,
    pub(crate) is_manual_compact: bool,
}

pub(crate) struct PreparedReplyConversation {
    pub(crate) events: Vec<AgentEvent>,
    pub(crate) conversation: Option<Conversation>,
    pub(crate) should_enter_reply_loop: bool,
}

pub(crate) struct NoToolTurnHandling {
    pub(crate) events: Vec<AgentEvent>,
    pub(crate) exit_chat: bool,
    pub(crate) retry_requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoryCapturePolicy {
    SystemNotificationsOnly,
    AllMessages,
}

#[derive(Debug, Clone, Default)]
pub struct DelegationCapabilityContext {
    pub allow_plan_mode: bool,
    pub allow_subagent: bool,
    pub allow_swarm: bool,
    pub allow_worker_messaging: bool,
}

pub(crate) enum RecoveryCompactionHandling {
    Continue {
        conversation: Conversation,
        events: Vec<AgentEvent>,
    },
    Abort {
        events: Vec<AgentEvent>,
    },
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
    pub(super) worker_capability_context: Mutex<Option<WorkerCapabilityContext>>,
    pub(super) delegation_capability_context: Mutex<Option<DelegationCapabilityContext>>,
    pub(super) confirmation_tx: mpsc::Sender<(String, PermissionConfirmation)>,
    pub(super) confirmation_rx: Mutex<mpsc::Receiver<(String, PermissionConfirmation)>>,
    pub(super) tool_result_tx: mpsc::Sender<(String, ToolResult<CallToolResult>)>,
    pub(super) tool_result_rx: ToolResultReceiver,

    pub tool_route_manager: Arc<ToolRouteManager>,
    pub(super) retry_manager: RetryManager,
    pub(super) tool_inspection_manager: ToolInspectionManager,
}

#[derive(Clone, Debug)]
pub struct ToolTransportRequestEvent {
    pub request: ToolRequest,
    pub transport: ToolTransportKind,
    pub surface: ToolInvocationSurface,
}

#[derive(Clone, Debug)]
pub enum AgentEvent {
    Message(Message),
    ToolTransportRequest(ToolTransportRequestEvent),
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
    async fn sync_extension_runtime_context(&self, session_id: &str) {
        let worker_runtime = load_worker_runtime_state(session_id).await.ok().flatten();
        let context = PlatformExtensionContext {
            session_id: Some(session_id.to_string()),
            task_board_context: Some(TaskBoardContext::from_runtime(
                session_id.to_string(),
                worker_runtime.as_ref(),
            )),
            extension_manager: Some(Arc::downgrade(&self.extension_manager)),
            tool_route_manager: Some(Arc::downgrade(&self.tool_route_manager)),
        };
        self.extension_manager.set_context(context).await;
    }

    async fn effective_extension_configs(&self) -> Vec<ExtensionConfig> {
        let mut configs = self.extension_manager.get_extension_configs().await;

        let frontend_tools = self.frontend_tools.lock().await;
        if !frontend_tools.is_empty() {
            let mut tools = frontend_tools
                .values()
                .map(|frontend_tool| frontend_tool.tool.clone())
                .collect::<Vec<_>>();
            tools.sort_by(|a, b| a.name.cmp(&b.name));

            let instructions = self.frontend_instructions.lock().await.clone();
            configs.push(ExtensionConfig::Frontend {
                name: "frontend_runtime".to_string(),
                description: "Frontend/runtime-provided tools".to_string(),
                tools,
                instructions,
                bundled: Some(true),
                available_tools: Vec::new(),
            });
        }

        configs
    }

    async fn task_config_from_session(
        &self,
        provider: Arc<dyn Provider>,
        session: &Session,
    ) -> TaskConfig {
        let mut extensions = self.effective_extension_configs().await;
        let mut task_config =
            TaskConfig::new(provider, &session.id, &session.working_dir, extensions);
        if let Ok(Some(host_state)) = load_host_session_state(&session.id).await {
            if !host_state.worker_extensions.is_empty() {
                extensions = host_state.worker_extensions.clone();
                task_config = TaskConfig::new(
                    task_config.provider.clone(),
                    &session.id,
                    &session.working_dir,
                    extensions,
                );
            }
            task_config = task_config
                .with_delegation_mode(host_state.delegation_mode)
                .with_write_scope(host_state.write_scope)
                .with_runtime_contract(host_state.target_artifacts, host_state.result_contract)
                .with_parallelism_budget(host_state.parallelism_budget, host_state.swarm_budget)
                .with_task_board_session_id(
                    host_state
                        .task_board_session_id
                        .clone()
                        .or_else(|| Some(session.id.clone())),
                );
        }
        let allow_worker_messaging = self
            .delegation_capability_context
            .lock()
            .await
            .clone()
            .map(|context| context.allow_worker_messaging)
            .unwrap_or(true);
        task_config = task_config.with_worker_messaging_policy(allow_worker_messaging);
        task_config = task_config.with_task_runtime(task_runtime_for_session(&session.id), None);
        task_config
    }

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
            worker_capability_context: Mutex::new(None),
            delegation_capability_context: Mutex::new(None),
            confirmation_tx: confirm_tx,
            confirmation_rx: Mutex::new(confirm_rx),
            tool_result_tx: tool_tx,
            tool_result_rx: Arc::new(Mutex::new(tool_rx)),
            tool_route_manager: Arc::new(ToolRouteManager::new()),
            retry_manager: RetryManager::new(),
            tool_inspection_manager: Self::create_default_tool_inspection_manager(),
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

    pub(crate) async fn handle_retry_logic(
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
        session_id: &str,
        unfixed_conversation: Conversation,
        working_dir: &std::path::Path,
    ) -> Result<ReplyContext> {
        self.sync_extension_runtime_context(session_id).await;

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

    pub(crate) async fn inject_runtime_memory_context(
        &self,
        conversation: &Conversation,
        _runtime_compactions: u32,
    ) -> (Conversation, Option<String>) {
        (conversation.clone(), None)
    }

    pub(crate) async fn categorize_tools(
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

    #[allow(dead_code)]
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

    pub(crate) async fn handle_denied_tools(
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

    pub(crate) async fn apply_runtime_policy_to_calls(
        &self,
        scheduled_calls: Vec<crate::agents::harness::tools::ScheduledToolCall>,
        harness_policy: &HarnessPolicy,
        coordinator_execution_mode: CoordinatorExecutionMode,
        delegation_state: &DelegationRuntimeState,
        request_to_response_map: &HashMap<String, Arc<Mutex<Message>>>,
    ) -> Vec<crate::agents::harness::tools::ScheduledToolCall> {
        let mut filtered_calls = Vec::new();
        let mut subagent_calls_this_turn = delegation_state.subagent_calls_this_turn;
        for scheduled in scheduled_calls {
            match apply_runtime_policy(
                harness_policy,
                coordinator_execution_mode,
                delegation_state,
                &scheduled.meta,
                &scheduled.request,
                subagent_calls_this_turn,
            ) {
                PolicyDecision::Allow => {
                    if scheduled.meta.is_subagent {
                        subagent_calls_this_turn = subagent_calls_this_turn.saturating_add(1);
                    }
                    filtered_calls.push(scheduled);
                }
                PolicyDecision::Deny { reason } => {
                    if let Some(response_msg) = request_to_response_map.get(&scheduled.request.id) {
                        let mut response = response_msg.lock().await;
                        *response = response.clone().with_tool_response(
                            scheduled.request.id.clone(),
                            Ok(CallToolResult {
                                content: vec![Content::text(format_runtime_policy_denial(
                                    &scheduled.meta.name,
                                    &reason,
                                ))],
                                structured_content: None,
                                is_error: Some(true),
                                meta: None,
                            }),
                        );
                    }
                }
            }
        }
        filtered_calls
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn execute_scheduled_tool_batches(
        &self,
        tool_batches: Vec<crate::agents::harness::tool_scheduler::ToolBatch>,
        request_to_response_map: &HashMap<String, Arc<Mutex<Message>>>,
        enable_extension_request_ids: &[String],
        cancel_token: Option<CancellationToken>,
        session: &Session,
        session_id: &str,
        save_extension_state_on_success: bool,
        turns_taken: u32,
        current_mode: HarnessMode,
        transcript_store: &SessionHarnessStore,
        delegation_state: &mut DelegationRuntimeState,
        task_runtime: &crate::agents::harness::TaskRuntime,
        tool_result_budget: &crate::agents::harness::ToolResultBudget,
        transition_trace: &crate::agents::harness::SharedTransitionTrace,
    ) -> Result<ScheduledBatchExecutionResult> {
        let mut all_install_successful = true;
        let mut executed_tool_calls = 0usize;
        let mut events = Vec::new();
        let default_target_artifacts = delegation_state.target_artifacts.clone();
        let summarize_tool_output =
            |output: &Result<CallToolResult, rmcp::model::ErrorData>| match output {
                Ok(result) => {
                    let text = result
                        .content
                        .iter()
                        .filter_map(|content| content.as_text().map(|text| text.text.clone()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if text.is_empty() {
                        "child task completed".to_string()
                    } else {
                        text.chars().take(200).collect()
                    }
                }
                Err(err) => err.to_string(),
            };
        let classify_child_tool_output =
            |output: &Result<CallToolResult, rmcp::model::ErrorData>| -> (Vec<String>, bool) {
                let Some(result) = output.as_ref().ok() else {
                    return (Vec::new(), false);
                };
                let Some(structured) = result.structured_content.as_ref() else {
                    return (default_target_artifacts.clone(), true);
                };
                let accepted_targets = structured
                    .get("accepted_targets")
                    .or_else(|| structured.get("produced_targets"))
                    .and_then(serde_json::Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_else(|| default_target_artifacts.clone());
                let produced_delta = structured
                    .get("produced_delta")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true);
                (accepted_targets, produced_delta)
            };

        for batch in tool_batches {
            let mut tool_streams = Vec::new();
            for scheduled in batch.calls {
                if let Ok(tool_call) = scheduled.request.tool_call.clone() {
                    if scheduled.meta.is_subagent {
                        delegation_state.note_subagent_call();
                    } else if scheduled.meta.name == SWARM_TOOL_NAME {
                        delegation_state.note_swarm_call();
                    }
                    let request_id = scheduled.request.id.clone();
                    let dispatch_cancel_token = if scheduled.meta.supports_child_tasks {
                        let mut task_metadata = HashMap::new();
                        crate::agents::harness::WorkerAttemptIdentity::fresh(
                            request_id.clone(),
                            request_id.clone(),
                        )
                        .write_to_metadata(&mut task_metadata);
                        let task_kind = if scheduled.meta.is_subagent {
                            crate::agents::harness::TaskKind::Subagent
                        } else if scheduled.meta.name.contains("validate") {
                            crate::agents::harness::TaskKind::ValidationWorker
                        } else {
                            crate::agents::harness::TaskKind::SwarmWorker
                        };
                        let child_task_handle =
                            crate::agents::harness::TaskRuntimeHost::spawn_task(
                                task_runtime,
                                crate::agents::harness::TaskSpec {
                                    task_id: request_id.clone(),
                                    parent_session_id: session_id.to_string(),
                                    depth: delegation_state.current_depth.saturating_add(1),
                                    kind: task_kind,
                                    description: Some(format!(
                                        "child task for {}",
                                        scheduled.meta.name
                                    )),
                                    write_scope: delegation_state.write_scope.clone(),
                                    target_artifacts: delegation_state.target_artifacts.clone(),
                                    result_contract: delegation_state.result_contract.clone(),
                                    metadata: task_metadata,
                                },
                            )
                            .await
                            .ok();
                        if let Some(handle) = child_task_handle {
                            let task_cancel = handle.cancel_token.clone();
                            if let Some(parent_cancel) = cancel_token.clone() {
                                let combined_cancel = parent_cancel.child_token();
                                let relay_cancel = combined_cancel.clone();
                                tokio::spawn(async move {
                                    task_cancel.cancelled().await;
                                    relay_cancel.cancel();
                                });
                                Some(combined_cancel)
                            } else {
                                Some(task_cancel)
                            }
                        } else {
                            cancel_token.clone()
                        }
                    } else {
                        cancel_token.clone()
                    };
                    let (_req_id, tool_result) = self
                        .dispatch_tool_call(
                            tool_call,
                            request_id.clone(),
                            dispatch_cancel_token,
                            session,
                        )
                        .await;
                    let stream = match tool_result {
                        Ok(result) => tool_stream(
                            result
                                .notification_stream
                                .unwrap_or_else(|| Box::new(stream::empty())),
                            result.result,
                        ),
                        Err(e) => {
                            tool_stream(Box::new(stream::empty()), futures::future::ready(Err(e)))
                        }
                    };
                    tool_streams.push((request_id, scheduled.meta.clone(), stream));
                }
            }

            let is_concurrent = matches!(batch.mode, ToolBatchMode::ConcurrentReadOnly);

            if is_concurrent {
                let with_id = tool_streams
                    .into_iter()
                    .map(|(request_id, meta, stream)| {
                        stream.map(move |item| (request_id.clone(), meta.clone(), item))
                    })
                    .collect::<Vec<_>>();

                let mut combined = stream::select_all(with_id);
                while let Some((request_id, meta, item)) = combined.next().await {
                    if is_token_cancelled(&cancel_token) {
                        break;
                    }

                    for msg in Self::drain_elicitation_messages(session_id).await {
                        events.push(AgentEvent::Message(msg));
                    }

                    match item {
                        ToolStreamItem::Result(output) => {
                            executed_tool_calls = executed_tool_calls.saturating_add(1);
                            let budgeted = crate::agents::harness::apply_tool_result_budget(
                                &request_id,
                                &meta.name,
                                meta.result_budget_bucket,
                                meta.max_result_chars,
                                output,
                                tool_result_budget,
                            )
                            .await;
                            if let Some(handle) = &budgeted.handle {
                                let mut metadata = std::collections::BTreeMap::new();
                                metadata.insert("tool".to_string(), meta.name.clone());
                                metadata.insert("handle".to_string(), handle.id.clone());
                                metadata
                                    .insert("action".to_string(), format!("{:?}", budgeted.action));
                                crate::agents::harness::record_transition(
                                    transition_trace,
                                    turns_taken,
                                    current_mode,
                                    crate::agents::harness::TransitionKind::ToolBudgetFallback,
                                    "tool_result_budget_applied",
                                    metadata,
                                )
                                .await;
                            }
                            let output = budgeted.result;
                            if enable_extension_request_ids.contains(&request_id) && output.is_err()
                            {
                                all_install_successful = false;
                            }
                            if meta.is_subagent && output.is_err() {
                                let reason = output
                                    .as_ref()
                                    .err()
                                    .map(|err| err.to_string())
                                    .unwrap_or_else(|| "subagent execution failed".to_string());
                                delegation_state.note_subagent_failure(reason);
                                crate::agents::harness::record_transition(
                                    transition_trace,
                                    turns_taken,
                                    current_mode,
                                    crate::agents::harness::TransitionKind::ChildTaskDowngrade,
                                    "subagent_execution_failed",
                                    std::collections::BTreeMap::new(),
                                )
                                .await;
                            }
                            if meta.supports_child_tasks {
                                let (accepted_targets, produced_delta) =
                                    classify_child_tool_output(&output);
                                let envelope = crate::agents::harness::TaskResultEnvelope {
                                    task_id: request_id.clone(),
                                    kind: if meta.is_subagent {
                                        crate::agents::harness::TaskKind::Subagent
                                    } else if meta.name.contains("validate") {
                                        crate::agents::harness::TaskKind::ValidationWorker
                                    } else {
                                        crate::agents::harness::TaskKind::SwarmWorker
                                    },
                                    status: if output.is_ok() {
                                        crate::agents::harness::TaskStatus::Completed
                                    } else {
                                        crate::agents::harness::TaskStatus::Failed
                                    },
                                    summary: summarize_tool_output(&output),
                                    accepted_targets,
                                    produced_delta,
                                    metadata: {
                                        let mut metadata = HashMap::new();
                                        crate::agents::harness::WorkerAttemptIdentity::fresh(
                                            request_id.clone(),
                                            request_id.clone(),
                                        )
                                        .write_to_metadata(&mut metadata);
                                        metadata
                                    },
                                };
                                if output.is_ok() {
                                    let _ = crate::agents::harness::TaskRuntimeHost::complete(
                                        task_runtime,
                                        envelope,
                                    )
                                    .await;
                                } else {
                                    let _ = crate::agents::harness::TaskRuntimeHost::fail(
                                        task_runtime,
                                        envelope,
                                    )
                                    .await;
                                }
                            }
                            if let Some(response_msg) = request_to_response_map.get(&request_id) {
                                let mut response = response_msg.lock().await;
                                *response = response.clone().with_tool_response(request_id, output);
                            }
                        }
                        ToolStreamItem::Message(msg) => {
                            if meta.supports_child_tasks {
                                let _ = crate::agents::harness::TaskRuntimeHost::record_progress(
                                    task_runtime,
                                    &request_id,
                                    format!("{:?}", msg),
                                    None,
                                )
                                .await;
                            }
                            events.push(AgentEvent::McpNotification((request_id, msg)));
                        }
                    }
                }
            } else {
                for (request_id, meta, mut stream) in tool_streams {
                    while let Some(item) = stream.next().await {
                        if is_token_cancelled(&cancel_token) {
                            break;
                        }

                        for msg in Self::drain_elicitation_messages(session_id).await {
                            events.push(AgentEvent::Message(msg));
                        }

                        match item {
                            ToolStreamItem::Result(output) => {
                                executed_tool_calls = executed_tool_calls.saturating_add(1);
                                let budgeted = crate::agents::harness::apply_tool_result_budget(
                                    &request_id,
                                    &meta.name,
                                    meta.result_budget_bucket,
                                    meta.max_result_chars,
                                    output,
                                    tool_result_budget,
                                )
                                .await;
                                if let Some(handle) = &budgeted.handle {
                                    let mut metadata = std::collections::BTreeMap::new();
                                    metadata.insert("tool".to_string(), meta.name.clone());
                                    metadata.insert("handle".to_string(), handle.id.clone());
                                    metadata.insert(
                                        "action".to_string(),
                                        format!("{:?}", budgeted.action),
                                    );
                                    crate::agents::harness::record_transition(
                                        transition_trace,
                                        turns_taken,
                                        current_mode,
                                        crate::agents::harness::TransitionKind::ToolBudgetFallback,
                                        "tool_result_budget_applied",
                                        metadata,
                                    )
                                    .await;
                                }
                                let output = budgeted.result;
                                if enable_extension_request_ids.contains(&request_id)
                                    && output.is_err()
                                {
                                    all_install_successful = false;
                                }
                                if meta.is_subagent && output.is_err() {
                                    let reason = output
                                        .as_ref()
                                        .err()
                                        .map(|err| err.to_string())
                                        .unwrap_or_else(|| "subagent execution failed".to_string());
                                    delegation_state.note_subagent_failure(reason);
                                    crate::agents::harness::record_transition(
                                        transition_trace,
                                        turns_taken,
                                        current_mode,
                                        crate::agents::harness::TransitionKind::ChildTaskDowngrade,
                                        "subagent_execution_failed",
                                        std::collections::BTreeMap::new(),
                                    )
                                    .await;
                                }
                                if meta.supports_child_tasks {
                                    let (accepted_targets, produced_delta) =
                                        classify_child_tool_output(&output);
                                    let envelope = crate::agents::harness::TaskResultEnvelope {
                                        task_id: request_id.clone(),
                                        kind: if meta.is_subagent {
                                            crate::agents::harness::TaskKind::Subagent
                                        } else if meta.name.contains("validate") {
                                            crate::agents::harness::TaskKind::ValidationWorker
                                        } else {
                                            crate::agents::harness::TaskKind::SwarmWorker
                                        },
                                        status: if output.is_ok() {
                                            crate::agents::harness::TaskStatus::Completed
                                        } else {
                                            crate::agents::harness::TaskStatus::Failed
                                        },
                                        summary: summarize_tool_output(&output),
                                        accepted_targets,
                                        produced_delta,
                                        metadata: {
                                            let mut metadata = HashMap::new();
                                            crate::agents::harness::WorkerAttemptIdentity::fresh(
                                                request_id.clone(),
                                                request_id.clone(),
                                            )
                                            .write_to_metadata(&mut metadata);
                                            metadata
                                        },
                                    };
                                    if output.is_ok() {
                                        let _ = crate::agents::harness::TaskRuntimeHost::complete(
                                            task_runtime,
                                            envelope,
                                        )
                                        .await;
                                    } else {
                                        let _ = crate::agents::harness::TaskRuntimeHost::fail(
                                            task_runtime,
                                            envelope,
                                        )
                                        .await;
                                    }
                                }
                                if let Some(response_msg) = request_to_response_map.get(&request_id)
                                {
                                    let mut response = response_msg.lock().await;
                                    *response = response
                                        .clone()
                                        .with_tool_response(request_id.clone(), output);
                                }
                            }
                            ToolStreamItem::Message(msg) => {
                                if meta.supports_child_tasks {
                                    let _ =
                                        crate::agents::harness::TaskRuntimeHost::record_progress(
                                            task_runtime,
                                            &request_id,
                                            format!("{:?}", msg),
                                            None,
                                        )
                                        .await;
                                }
                                events.push(AgentEvent::McpNotification((request_id.clone(), msg)));
                            }
                        }
                    }
                }
            }
        }

        for msg in Self::drain_elicitation_messages(session_id).await {
            events.push(AgentEvent::Message(msg));
        }

        if all_install_successful
            && save_extension_state_on_success
            && !enable_extension_request_ids.is_empty()
        {
            self.save_extension_state(&SessionConfig {
                id: session_id.to_string(),
                schedule_id: None,
                max_turns: None,
                retry_config: None,
            })
            .await?;
        }

        if let Some(downgrade_message) = delegation_state.downgrade_message.take() {
            let _ = transcript_store
                .record_checkpoint(
                    session_id,
                    crate::agents::harness::HarnessCheckpoint::delegation_downgraded(
                        turns_taken,
                        current_mode,
                    ),
                )
                .await;
            let downgrade = Message::assistant()
                .with_system_notification(SystemNotificationType::InlineMessage, downgrade_message);
            events.push(AgentEvent::Message(downgrade));
        }

        Ok(ScheduledBatchExecutionResult {
            executed_tool_calls,
            all_install_successful,
            events,
        })
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

    pub async fn final_output_string(&self) -> Option<String> {
        self.final_output_tool
            .lock()
            .await
            .as_ref()
            .and_then(|tool| tool.final_output.clone())
    }

    pub async fn clear_final_output_state(&self) {
        if let Some(final_output_tool) = self.final_output_tool.lock().await.as_mut() {
            final_output_tool.final_output = None;
        }
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
        tool_call: CallToolRequestParams,
        request_id: String,
        cancellation_token: Option<CancellationToken>,
        session: &Session,
    ) -> (String, Result<ToolCallResult, ErrorData>) {
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

            let task_config = self
                .task_config_from_session(provider, session)
                .await
                .with_task_runtime(
                    task_runtime_for_session(&session.id),
                    Some(request_id.clone()),
                );
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
        } else if tool_call.name == SWARM_TOOL_NAME {
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

            let task_config = self
                .task_config_from_session(provider, session)
                .await
                .with_delegation_mode(crate::agents::harness::DelegationMode::Swarm)
                .with_task_runtime(
                    task_runtime_for_session(&session.id),
                    Some(request_id.clone()),
                );

            let arguments = tool_call
                .arguments
                .clone()
                .map(Value::Object)
                .unwrap_or(Value::Object(serde_json::Map::new()));

            handle_swarm_tool(
                arguments,
                task_config,
                session.working_dir.clone(),
                cancellation_token,
            )
        } else if tool_call.name == SEND_MESSAGE_TOOL_NAME {
            handle_send_message_tool(
                Value::Object(tool_call.arguments.clone().unwrap_or_default()),
                &session.id,
            )
            .await
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

    pub async fn set_worker_capability_context(&self, context: Option<WorkerCapabilityContext>) {
        let mut guard = self.worker_capability_context.lock().await;
        *guard = context;
    }

    pub async fn set_delegation_capability_context(
        &self,
        context: Option<DelegationCapabilityContext>,
    ) {
        let mut guard = self.delegation_capability_context.lock().await;
        *guard = context;
    }

    pub async fn list_tools(&self, extension_name: Option<String>) -> Vec<Tool> {
        let mut prefixed_tools = self
            .extension_manager
            .get_prefixed_tools(extension_name.clone())
            .await
            .unwrap_or_default();

        if extension_name.is_none() {
            if let Some(final_output_tool) = self.final_output_tool.lock().await.as_ref() {
                prefixed_tools.push(final_output_tool.tool());
            }
            if self
                .worker_capability_context
                .lock()
                .await
                .clone()
                .is_some_and(|context| context.allow_worker_messaging)
            {
                prefixed_tools.push(create_send_message_tool());
            }

            let delegation_capability_context =
                self.delegation_capability_context.lock().await.clone();
            let allow_subagent = delegation_capability_context
                .as_ref()
                .map(|context| context.allow_subagent)
                .unwrap_or(true);
            let allow_swarm = delegation_capability_context
                .as_ref()
                .map(|context| context.allow_swarm)
                .unwrap_or(true);

            if allow_subagent {
                let sub_recipes = self.sub_recipes.lock().await;
                let sub_recipes_vec: Vec<_> = sub_recipes.values().cloned().collect();
                prefixed_tools.push(create_subagent_tool(&sub_recipes_vec));
            }
            if allow_swarm && crate::agents::harness::native_swarm_tool_enabled() {
                prefixed_tools.push(create_swarm_tool());
            }
        }

        if let Some(worker_capability_context) = self.worker_capability_context.lock().await.clone()
        {
            prefixed_tools.retain(|tool| {
                let hidden = worker_capability_context
                    .hidden_coordinator_tools
                    .iter()
                    .any(|hidden| hidden == &tool.name);
                if hidden {
                    return false;
                }

                let in_builtin_allowlist = worker_capability_context
                    .allowed_builtin_tools
                    .iter()
                    .any(|name| name == &tool.name);
                let extension_allowlist_empty =
                    worker_capability_context.allowed_extension_tools.is_empty();
                let in_extension_allowlist = worker_capability_context
                    .allowed_extension_tools
                    .iter()
                    .any(|name| name == &tool.name);

                in_builtin_allowlist || extension_allowlist_empty || in_extension_allowlist
            });
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
        let transcript_store = SessionHarnessStore;

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
                    transcript_store
                        .append_message(&session_config.id, &user_message)
                        .await?;
                    return Ok(Box::pin(futures::stream::empty()));
                }
            }
        }

        let message_text = user_message.as_concat_text();
        let is_manual_compact = MANUAL_COMPACT_TRIGGERS.contains(&message_text.trim());

        if let Some(mode_command) = parse_harness_mode_command(message_text.trim()) {
            if mode_command.mode == HarnessMode::Plan {
                let allow_plan_mode = self
                    .delegation_capability_context
                    .lock()
                    .await
                    .clone()
                    .map(|context| context.allow_plan_mode)
                    .unwrap_or(true);
                if !allow_plan_mode {
                    let notification = Message::assistant().with_system_notification(
                        SystemNotificationType::InlineMessage,
                        "Harness plan mode is disabled by the current runtime policy.".to_string(),
                    );

                    SessionManager::add_message(&session_config.id, &user_message).await?;
                    SessionManager::add_message(&session_config.id, &notification).await?;

                    return Ok(Box::pin(stream::once(async move {
                        Ok(AgentEvent::Message(notification))
                    })));
                }
            }
            transcript_store
                .save_mode(&session_config.id, mode_command.mode)
                .await?;

            let notification = Message::assistant().with_system_notification(
                SystemNotificationType::InlineMessage,
                format!("Harness mode set to {}.", mode_command.mode),
            );

            SessionManager::add_message(&session_config.id, &user_message).await?;
            SessionManager::add_message(&session_config.id, &notification).await?;

            return Ok(Box::pin(stream::once(async move {
                Ok(AgentEvent::Message(notification))
            })));
        }

        self.record_incoming_message_for_reply(
            &transcript_store,
            &session_config,
            &user_message,
            &message_text,
        )
        .await?;
        self.clear_final_output_state().await;
        let transition_trace = shared_transition_trace();
        let bootstrap = self
            .bootstrap_reply_state(
                &transcript_store,
                &session_config,
                is_manual_compact,
                reply_start,
                &transition_trace,
            )
            .await?;

        Ok(Box::pin(async_stream::try_stream! {
            let prepared_reply = self
                .prepare_conversation_for_reply_loop(
                    &transcript_store,
                    &session_config,
                    &bootstrap,
                    &transition_trace,
                )
                .await?;
            let _ = update_host_transition_trace(
                &session_config.id,
                transition_trace.lock().await.clone(),
            )
            .await;

            for event in prepared_reply.events {
                yield event;
            }

            if prepared_reply.should_enter_reply_loop {
                let final_conversation = prepared_reply
                    .conversation
                    .expect("conversation should exist when entering reply loop");
                let mut reply_stream = self
                    .reply_internal(
                        final_conversation,
                        session_config,
                        bootstrap.session,
                        cancel_token,
                    )
                    .await?;
                while let Some(event) = reply_stream.next().await {
                    yield event?;
                }
            }
        }))
    }

    #[allow(clippy::too_many_lines)]
    async fn reply_internal(
        &self,
        conversation: Conversation,
        session_config: SessionConfig,
        session: Session,
        cancel_token: Option<CancellationToken>,
    ) -> Result<BoxStream<'_, Result<AgentEvent>>> {
        let transcript_store = SessionHarnessStore;
        let mode = transcript_store.load_mode(&session_config.id).await?;
        let max_turns = session_config.max_turns.unwrap_or(DEFAULT_MAX_TURNS);
        let host_state = load_host_session_state(&session_config.id).await?;
        let server_local_tool_names = host_state
            .as_ref()
            .map(|state| state.server_local_tool_names.clone())
            .unwrap_or_default();
        let required_tool_prefixes = host_state
            .as_ref()
            .map(|state| state.required_tool_prefixes.clone())
            .unwrap_or_default();
        let validation_mode = host_state
            .as_ref()
            .map(|state| state.validation_mode)
            .unwrap_or(false);
        let (coordinator_execution_mode, provider_turn_mode, completion_surface_policy, delegation) =
            if let Some(host_state) = host_state {
                (
                    host_state.coordinator_execution_mode,
                    host_state.provider_turn_mode,
                    host_state.completion_surface_policy,
                    DelegationRuntimeState::new(
                        host_state.delegation_mode,
                        if matches!(
                            session.session_type,
                            crate::session::session_manager::SessionType::SubAgent
                        ) {
                            1
                        } else {
                            0
                        },
                        super::harness::bounded_subagent_depth_from_env(),
                        host_state.write_scope.clone(),
                        host_state.target_artifacts.clone(),
                        host_state.result_contract.clone(),
                    ),
                )
            } else {
                (
                    super::harness::CoordinatorExecutionMode::SingleWorker,
                    ProviderTurnMode::Streaming,
                    CompletionSurfacePolicy::Conversation,
                    DelegationRuntimeState::for_session_type(session.session_type),
                )
            };
        let worker_runtime = load_worker_runtime_state(&session_config.id)
            .await?
            .filter(|state| state.is_configured())
            .map(|state| HarnessWorkerRuntimeContext {
                swarm_run_id: state.swarm_run_id,
                worker_name: state.worker_name,
                leader_name: state.leader_name,
                logical_worker_id: state.logical_worker_id,
                coordinator_role: state.coordinator_role,
                mailbox_dir: state.mailbox_dir,
                permission_dir: state.permission_dir,
                scratchpad_dir: state.scratchpad_dir,
                enable_permission_bridge: state.enable_permission_bridge,
                allow_worker_messaging: state.allow_worker_messaging,
                peer_worker_addresses: state.peer_worker_addresses,
                peer_worker_catalog: state.peer_worker_catalog,
                validation_mode: state.validation_mode,
            });
        let context = HarnessContext::new(
            session_config.id.clone(),
            session.working_dir.clone(),
            Config::global().get_agime_mode().unwrap_or(AgimeMode::Auto),
            max_turns,
            cancel_token.clone(),
            mode,
            coordinator_execution_mode,
            provider_turn_mode,
            completion_surface_policy,
            delegation.clone(),
            delegation.write_scope.clone(),
            delegation.target_artifacts.clone(),
            delegation.result_contract.clone(),
            validation_mode,
            server_local_tool_names,
            required_tool_prefixes,
            None,
            task_runtime_for_session(&session_config.id)
                .unwrap_or_else(|| Arc::new(TaskRuntime::default())),
            worker_runtime,
        );
        let state = HarnessState::new(conversation, mode, coordinator_execution_mode, delegation);
        let policy = HarnessPolicy::new(mode);

        HarnessRunLoop::new(context, state, policy, transcript_store)
            .run(|context, state, policy, transcript_store| {
                self.run_harness_main_loop(
                    context,
                    state,
                    policy,
                    transcript_store,
                    session_config,
                    session,
                )
            })
            .await
    }

    #[allow(clippy::too_many_lines)]
    async fn run_harness_main_loop(
        &self,
        harness_context: HarnessContext,
        harness_state: HarnessState,
        mut harness_policy: HarnessPolicy,
        transcript_store: SessionHarnessStore,
        session_config: SessionConfig,
        session: Session,
    ) -> Result<BoxStream<'_, Result<AgentEvent>>> {
        // Performance monitoring
        let internal_start = std::time::Instant::now();
        tracing::info!("[PERF] run_harness_main_loop() started");

        let context = self
            .prepare_reply_context(
                &session_config.id,
                harness_state.conversation.clone(),
                &session.working_dir,
            )
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

        let working_dir = session.working_dir.clone();
        if !matches!(
            session.session_type,
            crate::session::session_manager::SessionType::Hidden
                | crate::session::session_manager::SessionType::SubAgent
        ) {
            let provider = self.provider().await?;
            let session_id = session_config.id.clone();
            tokio::spawn(async move {
                if let Err(e) = SessionManager::maybe_update_name(&session_id, provider).await {
                    warn!("Failed to generate session description: {}", e);
                }
            });
        }

        Ok(Box::pin(async_stream::try_stream! {
            let _ = reply_span.enter();
            let mut turns_taken = harness_state.turns_taken;
            let mut runtime_compaction_count = harness_state.runtime_compaction_count; // Tracks runtime recovery compaction attempts within this reply.
            let max_compaction_attempts = MAX_COMPACTION_ATTEMPTS;
            let max_turns = harness_context.max_turns;
            let mut current_mode = harness_state.mode;
            let mut delegation_state = harness_state.delegation.clone();
            let cancel_token = harness_context.cancel_token.clone();
            let signal_bridge = spawn_task_runtime_signal_bridge(
                harness_context.task_runtime.clone(),
                session_config.id.clone(),
                harness_context.coordinator_signals.clone(),
            );

            loop {
                delegation_state.reset_turn();
                harness_policy.mode = current_mode;
                if is_token_cancelled(&cancel_token) {
                    break;
                }

                if let Some(worker_runtime) = harness_context.worker_runtime.as_ref() {
                    let mailbox_identity = worker_runtime
                        .logical_worker_id
                        .as_deref()
                        .or(worker_runtime.worker_name.as_deref())
                        .or(worker_runtime.leader_name.as_deref());
                    if let (Some(mailbox_dir), Some(identity)) =
                        (worker_runtime.mailbox_dir.as_ref(), mailbox_identity)
                    {
                        if let Ok(mailbox_messages) =
                            drain_unread_messages_from_root(mailbox_dir, identity)
                        {
                            for mailbox_message in mailbox_messages {
                                harness_context
                                    .coordinator_signals
                                    .record(mailbox_message_to_notification(&mailbox_message))
                                    .await;
                            }
                        }
                    }
                }

                let drained_notifications: crate::agents::harness::NotificationDrainResult =
                    harness_context.coordinator_signals.drain_notifications().await;
                let _ = update_host_notification_summary(
                    &session_config.id,
                    drained_notifications
                        .has_notifications()
                        .then(|| drained_notifications.summary.clone()),
                )
                .await;
                let runtime_notification_input = drained_notifications.into_runtime_input();

                match self
                    .begin_turn(
                        &transcript_store,
                        &session_config,
                        &mut turns_taken,
                        max_turns,
                        current_mode,
                    )
                    .await?
                {
                    TurnStartHandling::Continue => {}
                    TurnStartHandling::BreakWithMessage(message) => {
                        yield AgentEvent::Message(message);
                        break;
                    }
                }

                let prepared_turn = self
                    .prepare_turn_input(
                        &session_config.id,
                        &conversation,
                        runtime_notification_input.as_ref(),
                        runtime_compaction_count,
                        &system_prompt,
                        current_mode,
                        harness_context.coordinator_execution_mode,
                        &delegation_state,
                    )
                    .await?;
                let visible_tools = if matches!(current_mode, HarnessMode::Complete) {
                    tools
                        .iter()
                        .filter(|tool| tool.name == FINAL_OUTPUT_TOOL_NAME)
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    tools.clone()
                };
                let visible_toolshim_tools = if matches!(current_mode, HarnessMode::Complete) {
                    toolshim_tools
                        .iter()
                        .filter(|tool| tool.name == FINAL_OUTPUT_TOOL_NAME)
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    toolshim_tools.clone()
                };

                let mut stream = Self::stream_response_from_provider(
                    self.provider().await?,
                    &prepared_turn.effective_system_prompt,
                    prepared_turn.conversation_for_model.messages(),
                    &visible_tools,
                    &visible_toolshim_tools,
                    harness_context.provider_turn_mode,
                ).await?;

                let mut no_tools_called = true;
                let mut messages_to_add = Conversation::default();
                let mut tools_updated = false;
                let mut did_recovery_compact_this_iteration = false;
                let mut auto_swarm_injected_this_reply = false;
                let mut terminal_provider_error = false;
                let mut terminal_provider_error_text: Option<String> = None;
                let mut received_provider_turn_output = false;

                while let Some(next) = stream.next().await {
                    if is_token_cancelled(&cancel_token) {
                        break;
                    }
                    received_provider_turn_output = true;

                    match next {
                        Ok((response, usage)) => {
                            let success_handling = self
                                .process_provider_success_result(
                                    response,
                                    usage,
                                    &tools,
                                    &conversation,
                                    &mut messages_to_add,
                                    &harness_policy,
                                    &mut delegation_state,
                                    &harness_context,
                                    agime_mode,
                                    cancel_token.clone(),
                                    &session,
                                    &session_config,
                                    turns_taken,
                                    current_mode,
                                    &transcript_store,
                                    &mut auto_swarm_injected_this_reply,
                                )
                                .await?;

                            for event in success_handling.pre_response_events {
                                yield event;
                            }

                            if let Some(response_handling) = success_handling.response_handling {
                                let ModelResponseHandling {
                                    events,
                                    no_tools_called: response_no_tools_called,
                                    tools_updated: response_tools_updated,
                                    yield_after_first_event,
                                    deferred_tool_handling,
                                } = response_handling;

                                no_tools_called = response_no_tools_called;
                                if response_tools_updated {
                                    tools_updated = true;
                                }

                                for (idx, event) in events.into_iter().enumerate() {
                                    yield event;
                                    if idx == 0 && yield_after_first_event {
                                        tokio::task::yield_now().await;
                                    }
                                }

                                if let Some(deferred_tool_handling) = deferred_tool_handling {
                                    let deferred_events = self
                                        .complete_deferred_tool_handling(
                                            deferred_tool_handling,
                                            &mut messages_to_add,
                                            &harness_context,
                                        )
                                        .await?;
                                    for event in deferred_events {
                                        yield event;
                                    }
                                }
                            }
                        }
                        Err(ref provider_err) => {
                            match self
                                .handle_provider_stream_error(
                                    provider_err,
                                    &conversation,
                                    &session_config,
                                    &transcript_store,
                                    turns_taken,
                                    current_mode,
                                    max_compaction_attempts,
                                    &mut runtime_compaction_count,
                                    &harness_context.transition_trace,
                                )
                                .await?
                            {
                                ProviderErrorHandling::ContinueTurn {
                                    conversation: compacted_conversation,
                                    events,
                                    did_recovery_compact_this_iteration: did_compact,
                                } => {
                                    did_recovery_compact_this_iteration = did_compact;
                                    conversation = compacted_conversation;
                                    for event in events {
                                        yield event;
                                    }
                                    continue;
                                }
                                ProviderErrorHandling::BreakLoop { events } => {
                                    terminal_provider_error = true;
                                    if terminal_provider_error_text.is_none() {
                                        terminal_provider_error_text =
                                            Some(provider_err.to_string());
                                    }
                                    for event in events {
                                        Self::capture_event_message(
                                            &mut messages_to_add,
                                            &event,
                                            HistoryCapturePolicy::AllMessages,
                                        );
                                        yield event;
                                    }
                                    if messages_to_add.is_empty() {
                                        let fallback = Message::assistant().with_text(format!(
                                            "Ran into this error: {}.\n\nPlease retry if you think this is a transient or recoverable error.",
                                            terminal_provider_error_text
                                                .as_deref()
                                                .unwrap_or("unknown provider error"),
                                        ));
                                        messages_to_add.push(fallback.clone());
                                        yield AgentEvent::Message(fallback);
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
                if !received_provider_turn_output && !terminal_provider_error && !is_token_cancelled(&cancel_token) {
                    let success_handling = self
                        .process_provider_success_result(
                            None,
                            None,
                            &tools,
                            &conversation,
                            &mut messages_to_add,
                            &harness_policy,
                            &mut delegation_state,
                            &harness_context,
                            agime_mode,
                            cancel_token.clone(),
                            &session,
                            &session_config,
                            turns_taken,
                            current_mode,
                            &transcript_store,
                            &mut auto_swarm_injected_this_reply,
                        )
                        .await?;

                    for event in success_handling.pre_response_events {
                        yield event;
                    }

                    if let Some(response_handling) = success_handling.response_handling {
                        let ModelResponseHandling {
                            events,
                            no_tools_called: response_no_tools_called,
                            tools_updated: response_tools_updated,
                            yield_after_first_event,
                            deferred_tool_handling,
                        } = response_handling;

                        no_tools_called = response_no_tools_called;
                        if response_tools_updated {
                            tools_updated = true;
                        }

                        for (idx, event) in events.into_iter().enumerate() {
                            yield event;
                            if idx == 0 && yield_after_first_event {
                                tokio::task::yield_now().await;
                            }
                        }

                        if let Some(deferred_tool_handling) = deferred_tool_handling {
                            let deferred_events = self
                                .complete_deferred_tool_handling(
                                    deferred_tool_handling,
                                    &mut messages_to_add,
                                    &harness_context,
                                )
                                .await?;
                            for event in deferred_events {
                                yield event;
                            }
                        }
                    }
                }
                self
                    .refresh_tools_after_update(
                        tools_updated,
                        &working_dir,
                        &mut tools,
                        &mut toolshim_tools,
                        &mut system_prompt,
                    )
                    .await?;
                let turn_finalization = self
                    .finalize_turn(
                        &mut conversation,
                        &mut messages_to_add,
                        no_tools_called,
                        did_recovery_compact_this_iteration,
                        &session_config,
                        &initial_messages,
                    terminal_provider_error,
                    &transcript_store,
                        turns_taken,
                        current_mode,
                        runtime_compaction_count,
                        harness_context.completion_surface_policy,
                        delegation_state.current_depth,
                        harness_context.coordinator_execution_mode,
                        &harness_context.required_tool_prefixes,
                    harness_context.task_runtime.as_ref(),
                    &harness_context.coordinator_signals,
                    &harness_context.transition_trace,
                )
                .await?;
                let latest_transition_trace: crate::agents::harness::TransitionTrace =
                    harness_context.transition_trace.lock().await.clone();
                let _ =
                    update_host_transition_trace(&session_config.id, latest_transition_trace).await;
                current_mode = turn_finalization.next_mode;
                harness_policy.mode = current_mode;
                for event in turn_finalization.events {
                    yield event;
                }
                if turn_finalization.exit_chat {
                    break;
                }

                tokio::task::yield_now().await;
            }

            signal_bridge.abort();
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
        self.sync_extension_runtime_context(session_id).await;

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

    #[allow(clippy::too_many_lines)]
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

    #[tokio::test]
    async fn task_config_includes_frontend_runtime_extensions() -> Result<()> {
        let agent = Agent::new();
        let frontend_tool = Tool::new(
            "text_editor",
            "Write bounded file content",
            serde_json::Map::new(),
        );
        agent
            .add_extension(ExtensionConfig::Frontend {
                name: "direct_host_tools".to_string(),
                description: "Direct host tools".to_string(),
                tools: vec![frontend_tool.clone()],
                instructions: Some("Use direct host tools when needed.".to_string()),
                bundled: Some(true),
                available_tools: Vec::new(),
            })
            .await?;

        let configs = agent.effective_extension_configs().await;
        let frontend = configs.into_iter().find_map(|config| match config {
            ExtensionConfig::Frontend {
                name,
                tools,
                instructions,
                ..
            } => Some((name, tools, instructions)),
            _ => None,
        });

        let (name, tools, instructions) = frontend.expect("frontend runtime config");
        assert_eq!(name, "frontend_runtime");
        assert!(tools.iter().any(|tool| tool.name == frontend_tool.name));
        assert_eq!(
            instructions.as_deref(),
            Some("Use direct host tools when needed.")
        );
        Ok(())
    }

    #[tokio::test]
    async fn worker_capability_context_hides_coordinator_only_tools_from_list_tools() -> Result<()>
    {
        let agent = Agent::new();
        agent
            .set_worker_capability_context(Some(WorkerCapabilityContext {
                allowed_builtin_tools: vec![FINAL_OUTPUT_TOOL_NAME.to_string()],
                allowed_extension_tools: Vec::new(),
                runtime_tool_surface: vec!["developer (tools: shell_command)".to_string()],
                hidden_coordinator_tools: vec!["subagent".to_string(), "swarm".to_string()],
                permission_policy: "bounded worker".to_string(),
                allow_worker_messaging: false,
                peer_worker_addresses: Vec::new(),
                peer_worker_catalog: Vec::new(),
                leader_address: None,
                current_worker_address: None,
            }))
            .await;

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
        assert!(tools.iter().all(|tool| tool.name != "subagent"));
        assert!(tools.iter().all(|tool| tool.name != "swarm"));
        assert!(tools.iter().any(|tool| tool.name == FINAL_OUTPUT_TOOL_NAME));
        Ok(())
    }
}
