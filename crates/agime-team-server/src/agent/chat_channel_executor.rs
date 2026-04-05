use std::sync::Arc;

use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use tokio_util::sync::CancellationToken;

use super::chat_channel_manager::ChatChannelManager;
use super::chat_channels::{
    ChatChannelAuthorType, ChatChannelInteractionMode, ChatChannelMessageSurface,
    ChatChannelService, ChatChannelThreadState,
};
use super::host_router::{ChannelHostRouteRequest, HostExecutionRouter};
use super::runtime;
use super::server_harness_host::ServerHarnessHost;
use super::service_mongo::AgentService;
use super::task_manager::{StreamEvent, TaskManager};
use super::workspace_service::WorkspaceService;

fn direct_host_timeout_secs() -> u64 {
    std::env::var("TEAM_DIRECT_HOST_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(600)
}

fn channel_failure_message(surface: ChatChannelMessageSurface) -> &'static str {
    match surface {
        ChatChannelMessageSurface::Temporary => "这轮临时协作未能完成，可补充上下文后重试。",
        ChatChannelMessageSurface::Issue => "这轮正式协作未能完成，可补充上下文后重试。",
        ChatChannelMessageSurface::Activity => "这条讨论消息暂时未能处理，可稍后重试。",
    }
}

fn diagnostic_note_thread_root_id(req: &ExecuteChannelMessageRequest) -> Option<String> {
    Some(req.root_message_id.clone())
}

fn base_channel_runtime_metadata(
    req: &ExecuteChannelMessageRequest,
    diagnostic_source: &str,
    runtime_diagnostics: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "channel_run_id": req.root_message_id,
        "runtime_session_id": req.internal_session_id,
        "diagnostic_visibility": "collapsed",
        "diagnostic_source": diagnostic_source,
        "runtime_diagnostics": runtime_diagnostics,
    })
}

fn outcome_runtime_diagnostics(
    req: &ExecuteChannelMessageRequest,
    outcome: &super::server_harness_host::ServerHarnessHostOutcome,
) -> Option<serde_json::Value> {
    let report = outcome.runtime_diagnostics()?;
    let has_details = report.status != "completed"
        || report
            .validation_status
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || report
            .blocking_reason
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || report
            .reason_code
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || !report.next_steps.is_empty();
    if !has_details {
        return None;
    }
    Some(base_channel_runtime_metadata(
        req,
        "execution_host_completion_report",
        serde_json::json!({
            "status": report.status,
            "summary": report.summary,
            "validation_status": report.validation_status,
            "blocking_reason": report.blocking_reason,
            "next_steps": report.next_steps,
            "reason_code": report.reason_code,
            "content_accessed": report.content_accessed,
            "analysis_complete": report.analysis_complete,
        }),
    ))
}

fn error_runtime_diagnostics(req: &ExecuteChannelMessageRequest, error: &str) -> serde_json::Value {
    base_channel_runtime_metadata(
        req,
        "executor_error",
        serde_json::json!({
            "status": "failed",
            "summary": "执行器异常",
            "blocking_reason": error,
            "next_steps": ["补充上下文后重试，或在处理问题后再次发起协作。"],
        }),
    )
}

pub struct ChatChannelExecutor {
    db: Arc<MongoDb>,
    channel_manager: Arc<ChatChannelManager>,
    agent_service: Arc<AgentService>,
    internal_task_manager: Arc<TaskManager>,
    workspace_root: String,
}

pub struct ExecuteChannelMessageRequest {
    pub channel_id: String,
    pub root_message_id: String,
    pub run_scope_id: String,
    pub thread_root_id: Option<String>,
    pub assistant_parent_message_id: String,
    pub surface: ChatChannelMessageSurface,
    pub thread_state: ChatChannelThreadState,
    pub interaction_mode: ChatChannelInteractionMode,
    pub internal_session_id: String,
    pub agent_id: String,
    pub user_message: String,
}

impl ChatChannelExecutor {
    pub fn new(
        db: Arc<MongoDb>,
        channel_manager: Arc<ChatChannelManager>,
        workspace_root: String,
    ) -> Self {
        Self {
            agent_service: Arc::new(AgentService::new(db.clone())),
            internal_task_manager: Arc::new(TaskManager::new()),
            db,
            channel_manager,
            workspace_root,
        }
    }

    pub async fn execute_channel_message(
        &self,
        req: ExecuteChannelMessageRequest,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        let exec_result = self.execute_channel_message_inner(&req, cancel_token).await;
        let (final_status, final_error) = match req.interaction_mode {
            ChatChannelInteractionMode::Conversation => match &exec_result {
                Ok(_) => ("completed", None),
                Err(error) => ("failed", Some(error.to_string())),
            },
            ChatChannelInteractionMode::Execution => match &exec_result {
                Ok(outcome) if outcome.execution_status() == "completed" => ("completed", None),
                Ok(outcome) => (
                    "failed",
                    outcome
                        .completion_report
                        .as_ref()
                        .and_then(|report| report.blocking_reason.clone())
                        .or_else(|| Some("execute host returned blocked completion".to_string())),
                ),
                Err(error) => ("failed", Some(error.to_string())),
            },
        };

        for attempt in 0..3 {
            match self
                .agent_service
                .update_session_execution_result(
                    &req.internal_session_id,
                    final_status,
                    final_error.as_deref(),
                )
                .await
            {
                Ok(_) => break,
                Err(error) => {
                    tracing::warn!(
                        "Failed to persist channel runtime session result for {} (attempt {}): {}",
                        req.internal_session_id,
                        attempt + 1,
                        error
                    );
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }
        }

        let channel_service = ChatChannelService::new(self.db.clone());
        if matches!(req.interaction_mode, ChatChannelInteractionMode::Execution) {
            let _ = channel_service
                .update_run_result(&req.channel_id, final_status, final_error.as_deref())
                .await;
        }

        self.channel_manager
            .broadcast(
                &format!("{}::{}", req.channel_id, req.run_scope_id),
                StreamEvent::Done {
                    status: final_status.to_string(),
                    error: final_error.clone(),
                },
            )
            .await;
        self.channel_manager
            .complete(&req.channel_id, Some(&req.run_scope_id))
            .await;

        exec_result.map(|_| ())
    }

    async fn execute_channel_message_inner(
        &self,
        req: &ExecuteChannelMessageRequest,
        cancel_token: CancellationToken,
    ) -> Result<super::server_harness_host::ServerHarnessHostOutcome> {
        let session = self
            .agent_service
            .get_session(&req.internal_session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Runtime session not found"))?;

        let workspace_binding = WorkspaceService::new(self.workspace_root.clone())
            .ensure_channel_workspace(&session)?;
        self.agent_service
            .set_session_workspace_binding(&req.internal_session_id, &workspace_binding)
            .await
            .map_err(|e| anyhow!("Failed to bind channel workspace: {}", e))?;
        let workspace_path = workspace_binding.root_path;

        let router = HostExecutionRouter::from_env();
        let cancel_token_direct = cancel_token.clone();
        let cancel_token_bridge = cancel_token;
        let exec_result = router
            .route_channel(
                ChannelHostRouteRequest {
                    channel_id: req.channel_id.clone(),
                    session_id: req.internal_session_id.clone(),
                    agent_id: req.agent_id.clone(),
                    user_message: req.user_message.clone(),
                    workspace_path,
                    target_artifacts: {
                        let mut targets = Vec::new();
                        if let Some(thread_root_id) = req.thread_root_id.as_deref() {
                            targets.push(format!(
                                "channel:{}/thread:{}",
                                req.channel_id, thread_root_id
                            ));
                        } else {
                            targets.push(format!("channel:{}", req.channel_id));
                        }
                        targets.extend(
                            session
                                .attached_document_ids
                                .iter()
                                .filter(|value| !value.trim().is_empty())
                                .map(|value| format!("document:{}", value.trim())),
                        );
                        targets
                    },
                    result_contract: {
                        let mut targets = Vec::new();
                        if let Some(thread_root_id) = req.thread_root_id.as_deref() {
                            targets.push(format!(
                                "channel:{}/thread:{}",
                                req.channel_id, thread_root_id
                            ));
                        } else {
                            targets.push(format!("channel:{}", req.channel_id));
                        }
                        targets.extend(
                            session
                                .attached_document_ids
                                .iter()
                                .filter(|value| !value.trim().is_empty())
                                .map(|value| format!("document:{}", value.trim())),
                        );
                        targets
                    },
                    validation_mode: false,
                },
                |request| async move {
                    let agent = self
                        .agent_service
                        .get_agent_with_key(&request.agent_id)
                        .await
                        .map_err(|e| anyhow!("DB error: {}", e))?
                        .ok_or_else(|| anyhow!("Agent not found"))?;
                    let host = ServerHarnessHost::new(
                        self.db.clone(),
                        self.agent_service.clone(),
                        self.internal_task_manager.clone(),
                    );
                    let timeout_secs = direct_host_timeout_secs();
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(timeout_secs),
                        host.execute_channel_host(
                            &session,
                            &agent,
                            &request.user_message,
                            request.workspace_path,
                            request.target_artifacts,
                            request.result_contract,
                            request.validation_mode,
                            cancel_token_direct.clone(),
                            format!("{}::{}", request.channel_id, req.run_scope_id),
                            self.channel_manager.clone(),
                        ),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => {
                            cancel_token_direct.cancel();
                            Err(anyhow!(
                                "Direct harness channel host timed out after {}s",
                                timeout_secs
                            ))
                        }
                    }
                },
                |request| async move {
                    runtime::execute_bridge_request(
                        &self.db,
                        &self.agent_service,
                        &self.internal_task_manager,
                        &self.channel_manager,
                        runtime::BridgeExecutionRequest {
                            context_id: request.channel_id,
                            agent_id: request.agent_id,
                            session_id: request.session_id,
                            user_message: request.user_message,
                            cancel_token: cancel_token_bridge,
                            workspace_path: Some(request.workspace_path),
                            llm_overrides: None,
                            turn_system_instruction: None,
                        },
                    )
                    .await?;
                    let updated_session = self
                        .agent_service
                        .get_session(&req.internal_session_id)
                        .await
                        .map_err(|e| anyhow!("DB error: {}", e))?
                        .ok_or_else(|| {
                            anyhow!("Runtime session not found after bridge execution")
                        })?;
                    Ok(super::server_harness_host::ServerHarnessHostOutcome {
                        messages_json: updated_session.messages_json.clone(),
                        message_count: updated_session.message_count,
                        total_tokens: updated_session.total_tokens,
                        last_assistant_text: runtime::extract_last_assistant_text(
                            &updated_session.messages_json,
                        ),
                        completion_report: None,
                        events_emitted: 0,
                        signal_summary: None,
                        completion_outcome: None,
                    })
                },
            )
            .await;

        let channel_service = ChatChannelService::new(self.db.clone());
        let agent = self
            .agent_service
            .get_agent(&req.agent_id)
            .await
            .ok()
            .flatten();

        let reply_text = exec_result
            .as_ref()
            .ok()
            .and_then(|outcome| outcome.user_visible_summary())
            .filter(|text| !text.trim().is_empty());

        if let Some(channel) = channel_service.get_channel(&req.channel_id).await? {
            if let Some(text) = reply_text.clone() {
                let _ = channel_service
                    .create_message(
                        &channel,
                        req.thread_root_id.clone(),
                        Some(req.assistant_parent_message_id.clone()),
                        req.surface,
                        req.thread_state,
                        None,
                        ChatChannelAuthorType::Agent,
                        None,
                        Some(req.agent_id.clone()),
                        agent
                            .as_ref()
                            .map(|item| item.name.clone())
                            .unwrap_or_else(|| req.agent_id.clone()),
                        Some(req.agent_id.clone()),
                        text.clone(),
                        serde_json::json!([{ "type": "text", "text": text }]),
                        serde_json::json!({
                            "channel_run_id": req.root_message_id,
                            "runtime_session_id": req.internal_session_id,
                        }),
                    )
                    .await;
            }

            match &exec_result {
                Ok(outcome) => {
                    if reply_text.is_none() {
                        if let Some(diagnostic_metadata) = outcome_runtime_diagnostics(req, outcome)
                        {
                            let diagnostic_text = channel_failure_message(req.surface).to_string();
                            let _ = channel_service
                                .create_message(
                                    &channel,
                                    diagnostic_note_thread_root_id(req),
                                    Some(req.assistant_parent_message_id.clone()),
                                    req.surface,
                                    req.thread_state,
                                    None,
                                    ChatChannelAuthorType::System,
                                    None,
                                    None,
                                    "System".to_string(),
                                    None,
                                    diagnostic_text.clone(),
                                    serde_json::json!([{ "type": "text", "text": diagnostic_text }]),
                                    diagnostic_metadata,
                                )
                                .await;
                        } else {
                            let fallback_text = channel_failure_message(req.surface).to_string();
                            let _ = channel_service
                                .create_message(
                                    &channel,
                                    diagnostic_note_thread_root_id(req),
                                    Some(req.assistant_parent_message_id.clone()),
                                    req.surface,
                                    req.thread_state,
                                    None,
                                    ChatChannelAuthorType::System,
                                    None,
                                    None,
                                    "System".to_string(),
                                    None,
                                    fallback_text.clone(),
                                    serde_json::json!([{ "type": "text", "text": fallback_text }]),
                                    serde_json::json!({
                                        "channel_run_id": req.root_message_id,
                                        "runtime_session_id": req.internal_session_id,
                                    }),
                                )
                                .await;
                        }
                    }
                }
                Err(error) => {
                    let diagnostic_text = channel_failure_message(req.surface).to_string();
                    let _ = channel_service
                        .create_message(
                            &channel,
                            diagnostic_note_thread_root_id(req),
                            Some(req.assistant_parent_message_id.clone()),
                            req.surface,
                            req.thread_state,
                            None,
                            ChatChannelAuthorType::System,
                            None,
                            None,
                            "System".to_string(),
                            None,
                            diagnostic_text.clone(),
                            serde_json::json!([{ "type": "text", "text": diagnostic_text }]),
                            error_runtime_diagnostics(req, &error.to_string()),
                        )
                        .await;
                }
            }
        }

        let _ = channel_service.refresh_channel_stats(&req.channel_id).await;
        exec_result
    }
}
