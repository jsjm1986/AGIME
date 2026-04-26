use std::sync::Arc;

use agime::agents::tool_execution_was_cancelled;
use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use tokio_util::sync::CancellationToken;

use super::chat_channel_manager::ChatChannelManager;
use super::chat_channels::{
    ChatChannelAuthorType, ChatChannelInteractionMode, ChatChannelMessageSurface,
    ChatChannelService, ChatChannelThreadState, ChatWorkspaceFileBlock,
};
use super::delegation_runtime::build_delegation_runtime;
use super::execution_admission;
use super::host_router::HostExecutionRouter;
use super::runtime;
use super::server_harness_host::ServerHarnessHost;
use super::service_mongo::{AgentService, ExecutionSlotAcquireOutcome};
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

fn persisted_thread_root_id(req: &ExecuteChannelMessageRequest) -> Option<String> {
    req.thread_root_id
        .clone()
        .or_else(|| Some(req.root_message_id.clone()))
}

fn diagnostic_note_thread_root_id(req: &ExecuteChannelMessageRequest) -> Option<String> {
    persisted_thread_root_id(req)
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
        || !report.next_steps.is_empty()
        || !outcome.persisted_child_evidence.is_empty()
        || !outcome.persisted_child_transcript_resume.is_empty()
        || outcome.transition_trace.is_some();
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
            "persisted_child_evidence": outcome.persisted_child_evidence,
            "persisted_child_transcript_resume": outcome.persisted_child_transcript_resume,
            "transition_trace": outcome.transition_trace,
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

fn outcome_user_visible_channel_reply(
    outcome: &super::server_harness_host::ServerHarnessHostOutcome,
) -> Option<String> {
    let delegation_runtime = build_delegation_runtime(
        Some("Leader"),
        Some(outcome.execution_status()),
        outcome.user_visible_summary().as_deref(),
        outcome
            .completion_report
            .as_ref()
            .and_then(|report| report.blocking_reason.as_deref()),
        None,
        Some(&outcome.persisted_child_evidence),
    );

    delegation_runtime
        .as_ref()
        .and_then(|runtime| runtime.leader.as_ref())
        .and_then(|leader| {
            leader
                .result_summary
                .as_deref()
                .or(leader.summary.as_deref())
        })
        .filter(|text| !text.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            outcome
                .user_visible_summary()
                .filter(|text| !text.trim().is_empty())
        })
}

fn assistant_message_content_blocks(
    text: &str,
    workspace_files: &[ChatWorkspaceFileBlock],
) -> serde_json::Value {
    let mut blocks = vec![serde_json::json!({
        "type": "text",
        "text": text,
    })];
    for file in workspace_files {
        if let Ok(value) = serde_json::to_value(file) {
            blocks.push(value);
        }
    }
    serde_json::Value::Array(blocks)
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

impl Clone for ExecuteChannelMessageRequest {
    fn clone(&self) -> Self {
        Self {
            channel_id: self.channel_id.clone(),
            root_message_id: self.root_message_id.clone(),
            run_scope_id: self.run_scope_id.clone(),
            thread_root_id: self.thread_root_id.clone(),
            assistant_parent_message_id: self.assistant_parent_message_id.clone(),
            surface: self.surface,
            thread_state: self.thread_state,
            interaction_mode: self.interaction_mode,
            internal_session_id: self.internal_session_id.clone(),
            agent_id: self.agent_id.clone(),
            user_message: self.user_message.clone(),
        }
    }
}

fn should_retry_scheduled_task_completion(
    outcome: &super::server_harness_host::ServerHarnessHostOutcome,
) -> bool {
    if outcome.execution_status().eq_ignore_ascii_case("blocked") {
        return true;
    }
    outcome
        .completion_report
        .as_ref()
        .and_then(|report| report.blocking_reason.as_deref())
        .is_some_and(|reason| reason.contains("without a user-visible assistant response"))
        || outcome
            .completion_outcome
            .as_ref()
            .and_then(|result| result.blocking_reason.as_deref())
            .is_some_and(|reason| reason.contains("without a user-visible assistant response"))
}

fn scheduled_task_continuation_prompt(req: &ExecuteChannelMessageRequest) -> String {
    format!(
        "你上一轮没有完成这个定时任务，也没有给出最终用户可见结果。现在继续完成原任务：{original}\n\
不要只创建目录、不要只描述计划、不要发送状态更新。\n\
如果任务要求工作区产物，必须把最终文件写到系统指令里要求的精确路径；完成后给出最终简洁总结。",
        original = req.user_message
    )
}

fn collaboration_delivery_prompt(req: &ExecuteChannelMessageRequest) -> Option<String> {
    if req.agent_id.trim().is_empty() {
        return None;
    }
    Some(
        "这是一次频道里的 agent 对话。如果用户要求下载、导出、交付文件或提供可预览文档，请优先：1. 在当前工作区生成或导出目标文件；2. 调用 attach_workspace_file_to_message 把文件附加到当前回复；3. 在最终回复中简短说明可直接预览或下载。不要只说“已保存到工作区”。"
            .to_string(),
    )
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
        let cancel_observer = cancel_token.clone();
        let session_source = self
            .agent_service
            .get_session(&req.internal_session_id)
            .await
            .ok()
            .flatten()
            .map(|session| session.session_source);
        let mut exec_result = self
            .execute_channel_message_inner(&req, cancel_token.child_token())
            .await;
        if matches!(req.interaction_mode, ChatChannelInteractionMode::Execution)
            && session_source
                .as_deref()
                .is_some_and(|source| source.eq_ignore_ascii_case("scheduled_task"))
            && !cancel_observer.is_cancelled()
            && matches!(&exec_result, Ok(outcome) if should_retry_scheduled_task_completion(outcome))
        {
            tracing::info!(
                "scheduled task execution produced no final answer; issuing continuation retry for {}",
                req.internal_session_id
            );
            let mut continuation_req = req.clone();
            continuation_req.user_message = scheduled_task_continuation_prompt(&req);
            exec_result = self
                .execute_channel_message_inner(&continuation_req, cancel_token.child_token())
                .await;
        }
        self.persist_channel_execution_messages(&req, &exec_result)
            .await;
        let cancelled_by_runtime = match &exec_result {
            Err(error) => tool_execution_was_cancelled(&error.to_string()),
            _ => false,
        };
        let persisted_cancelled = self
            .agent_service
            .get_session(&req.internal_session_id)
            .await
            .ok()
            .flatten()
            .and_then(|session| session.last_execution_status)
            .map(|status| status.eq_ignore_ascii_case("cancelled"))
            .unwrap_or(false);
        let (final_status, final_error) =
            if cancel_observer.is_cancelled() || cancelled_by_runtime || persisted_cancelled {
                ("cancelled", None)
            } else {
                match req.interaction_mode {
                    ChatChannelInteractionMode::Conversation => match &exec_result {
                        Ok(_) => ("completed", None),
                        Err(error) => ("failed", Some(error.to_string())),
                    },
                    ChatChannelInteractionMode::Execution => match &exec_result {
                        Ok(outcome) => match outcome.execution_status() {
                            "completed" => ("completed", None),
                            "blocked" => (
                                "blocked",
                                outcome
                                    .completion_report
                                    .as_ref()
                                    .and_then(|report| report.blocking_reason.clone()),
                            ),
                            "cancelled" => ("cancelled", None),
                            _ => (
                                "failed",
                                outcome
                                    .completion_report
                                    .as_ref()
                                    .and_then(|report| report.blocking_reason.clone())
                                    .or_else(|| {
                                        Some("execute host returned failed completion".to_string())
                                    }),
                            ),
                        },
                        Err(error) => ("failed", Some(error.to_string())),
                    },
                }
            };

        let delegation_runtime = match &exec_result {
            Ok(outcome) => build_delegation_runtime(
                Some("Leader"),
                Some(final_status),
                outcome.user_visible_summary().as_deref(),
                final_error.as_deref(),
                None,
                Some(&outcome.persisted_child_evidence),
            ),
            Err(_) => None,
        };
        let _ = self
            .agent_service
            .update_session_delegation_runtime(
                &req.internal_session_id,
                delegation_runtime.as_ref(),
            )
            .await;

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

    async fn persist_channel_execution_messages(
        &self,
        req: &ExecuteChannelMessageRequest,
        exec_result: &Result<super::server_harness_host::ServerHarnessHostOutcome>,
    ) {
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
            .and_then(outcome_user_visible_channel_reply);
        let attached_workspace_files = self
            .agent_service
            .list_pending_message_workspace_files(&req.internal_session_id)
            .await
            .unwrap_or_default();
        let effective_reply_text = reply_text.clone().or_else(|| {
            if attached_workspace_files.is_empty() {
                None
            } else {
                Some("已准备文件，可直接预览或下载。".to_string())
            }
        });

        if let Ok(Some(channel)) = channel_service.get_channel(&req.channel_id).await {
            if let Some(text) = effective_reply_text.clone() {
                let _ = channel_service
                    .create_message(
                        &channel,
                        persisted_thread_root_id(req),
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
                        assistant_message_content_blocks(&text, &attached_workspace_files),
                        serde_json::json!({
                            "channel_run_id": req.root_message_id,
                            "runtime_session_id": req.internal_session_id,
                            "workspace_files_count": attached_workspace_files.len(),
                        }),
                    )
                    .await;
                let _ = self
                    .agent_service
                    .clear_pending_message_workspace_files(&req.internal_session_id)
                    .await;
            }

            match exec_result {
                Ok(outcome) => {
                    if effective_reply_text.is_none() {
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
        let effective_user_message = collaboration_delivery_prompt(req)
            .map(|instruction| format!("{}\n\n{}", instruction, req.user_message))
            .unwrap_or_else(|| req.user_message.clone());

        let router = HostExecutionRouter::from_env();
        let using_direct_host = matches!(
            router.path(),
            super::server_harness_host::HostExecutionPath::DirectHarness
        );
        let cancel_token_direct = cancel_token.clone();
        let cancel_token_bridge = cancel_token;
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
        let target_artifacts = targets.clone();
        let result_contract = targets;
        let exec_result = if using_direct_host {
            match self
                .agent_service
                .try_acquire_execution_slot(&req.agent_id)
                .await
                .map_err(|e| anyhow!("Failed to acquire agent execution slot: {}", e))?
            {
                ExecutionSlotAcquireOutcome::Acquired => {
                    let direct_result = async {
                        let agent = self
                            .agent_service
                            .get_agent_with_key(&req.agent_id)
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
                                &effective_user_message,
                                workspace_path.clone(),
                                target_artifacts.clone(),
                                result_contract.clone(),
                                false,
                                cancel_token_direct.clone(),
                                format!("{}::{}", req.channel_id, req.run_scope_id),
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
                    }
                    .await;

                    if let Err(error) = self
                        .agent_service
                        .release_execution_slot(&req.agent_id)
                        .await
                    {
                        tracing::warn!(
                            "Failed to release execution slot for agent {} after channel execution: {}",
                            req.agent_id,
                            error
                        );
                    }
                    if let Err(error) = execution_admission::start_next_queued_tasks_for_agent(
                        &self.db,
                        &self.agent_service,
                        &self.internal_task_manager,
                        &req.agent_id,
                    )
                    .await
                    {
                        tracing::warn!(
                            "Failed to dispatch queued work for agent {} after channel execution: {}",
                            req.agent_id,
                            error
                        );
                    }

                    direct_result
                }
                ExecutionSlotAcquireOutcome::Saturated => {
                    let _ = self
                        .agent_service
                        .set_session_execution_state(&req.internal_session_id, "queued", true)
                        .await;
                    let channel_service = ChatChannelService::new(self.db.clone());
                    let _ = channel_service
                        .set_run_state(&req.channel_id, &req.run_scope_id, "queued", true)
                        .await;
                    self.channel_manager
                        .broadcast(
                            &format!("{}::{}", req.channel_id, req.run_scope_id),
                            StreamEvent::Status {
                                status: "queued".to_string(),
                            },
                        )
                        .await;
                    runtime::execute_bridge_request(
                        &self.db,
                        &self.agent_service,
                        &self.internal_task_manager,
                        &self.channel_manager,
                        runtime::BridgeExecutionRequest {
                            context_id: req.channel_id.clone(),
                            agent_id: req.agent_id.clone(),
                            session_id: req.internal_session_id.clone(),
                            channel_id: Some(req.channel_id.clone()),
                            run_scope_id: Some(req.run_scope_id.clone()),
                            user_message: effective_user_message.clone(),
                            cancel_token: cancel_token_bridge,
                            workspace_path: Some(workspace_path),
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
                        context_runtime_state: updated_session.context_runtime_state.clone(),
                        last_assistant_text: runtime::extract_last_assistant_text(
                            &updated_session.messages_json,
                        ),
                        completion_report: None,
                        persisted_child_evidence: Vec::new(),
                        persisted_child_transcript_resume: Vec::new(),
                        transition_trace: None,
                        events_emitted: 0,
                        signal_summary: None,
                        completion_outcome: None,
                    })
                }
            }
        } else {
            runtime::execute_bridge_request(
                &self.db,
                &self.agent_service,
                &self.internal_task_manager,
                &self.channel_manager,
                runtime::BridgeExecutionRequest {
                    context_id: req.channel_id.clone(),
                    agent_id: req.agent_id.clone(),
                    session_id: req.internal_session_id.clone(),
                    channel_id: Some(req.channel_id.clone()),
                    run_scope_id: Some(req.run_scope_id.clone()),
                    user_message: effective_user_message,
                    cancel_token: cancel_token_bridge,
                    workspace_path: Some(workspace_path),
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
                .ok_or_else(|| anyhow!("Runtime session not found after bridge execution"))?;
            Ok(super::server_harness_host::ServerHarnessHostOutcome {
                messages_json: updated_session.messages_json.clone(),
                message_count: updated_session.message_count,
                total_tokens: updated_session.total_tokens,
                context_runtime_state: updated_session.context_runtime_state.clone(),
                last_assistant_text: runtime::extract_last_assistant_text(
                    &updated_session.messages_json,
                ),
                completion_report: None,
                persisted_child_evidence: Vec::new(),
                persisted_child_transcript_resume: Vec::new(),
                transition_trace: None,
                events_emitted: 0,
                signal_summary: None,
                completion_outcome: None,
            })
        };

        exec_result
    }
}
