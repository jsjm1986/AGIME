//! Chat executor for direct session-based chat (bypasses Task system)
//!
//! ChatExecutor handles chat message execution directly on sessions.
//! It internally creates a temporary task to reuse TaskExecutor's
//! proven execution logic, but the task is ephemeral and not persisted
//! to the agent_tasks collection.

use agime::agents::tool_execution_was_cancelled;
use agime_team::MongoDb;
use anyhow::{anyhow, Result};

use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::automation::{contract::derive_builder_sync_payload, service::AutomationService};

use super::chat_channels::ChatWorkspaceFileBlock;
use super::chat_manager::ChatManager;
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

/// Chat executor that reuses TaskExecutor logic for session-based chat
pub struct ChatExecutor {
    db: Arc<MongoDb>,
    chat_manager: Arc<ChatManager>,
    agent_service: Arc<AgentService>,
    /// Internal task manager used to drive TaskExecutor
    internal_task_manager: Arc<TaskManager>,
    workspace_root: String,
}

impl ChatExecutor {
    pub fn new(db: Arc<MongoDb>, chat_manager: Arc<ChatManager>, workspace_root: String) -> Self {
        let agent_service = Arc::new(AgentService::new(db.clone()));
        let internal_task_manager = Arc::new(TaskManager::new());
        Self {
            db,
            chat_manager,
            agent_service,
            internal_task_manager,
            workspace_root,
        }
    }

    /// Execute a chat message in a session (bypasses Task system).
    ///
    /// Strategy: We create a temporary task record in MongoDB, use
    /// TaskExecutor to run it, then bridge events from the internal
    /// TaskManager to the ChatManager. After completion, the temp
    /// task is cleaned up and session metadata is updated.
    ///
    /// C3 fix: All cleanup (is_processing, temp task, done event) is
    /// guaranteed via the inner method + cleanup-on-all-paths pattern.
    pub async fn execute_chat(
        &self,
        session_id: &str,
        agent_id: &str,
        user_message: &str,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        self.execute_chat_with_turn_instruction(
            session_id,
            agent_id,
            user_message,
            None,
            cancel_token,
        )
        .await
    }

    pub async fn execute_chat_with_turn_instruction(
        &self,
        session_id: &str,
        agent_id: &str,
        user_message: &str,
        turn_system_instruction: Option<&str>,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        let cancel_observer = cancel_token.clone();
        // Run the inner logic; regardless of success/failure/panic,
        // we always clean up processing state and send done event.
        let mut exec_result = self
            .execute_chat_inner(
                session_id,
                agent_id,
                user_message,
                turn_system_instruction,
                cancel_token,
            )
            .await;

        if let Ok(Some(outcome)) = &mut exec_result {
            if let Err(error) = self
                .attach_pending_workspace_files_to_latest_assistant_message(session_id, outcome)
                .await
            {
                tracing::warn!(
                    session_id = %session_id,
                    "Failed to attach pending workspace files to direct chat assistant reply: {}",
                    error
                );
            }
        }

        let cancelled_by_runtime = match &exec_result {
            Err(error) => tool_execution_was_cancelled(&error.to_string()),
            _ => false,
        };
        let persisted_cancelled = self
            .agent_service
            .get_session(session_id)
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
                match &exec_result {
                    Ok(Some(outcome)) => match outcome.execution_status() {
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
                    Ok(None) => ("completed", None),
                    Err(e) => ("failed", Some(e.to_string())),
                }
            };

        let delegation_runtime = match &exec_result {
            Ok(Some(outcome)) => build_delegation_runtime(
                Some("Leader"),
                Some(final_status),
                outcome.user_visible_summary().as_deref(),
                final_error.as_deref(),
                None,
                Some(&outcome.persisted_child_evidence),
            ),
            _ => None,
        };
        let _ = self
            .agent_service
            .update_session_delegation_runtime(session_id, delegation_runtime.as_ref())
            .await;

        // Guaranteed cleanup: persist terminal execution state (retry up to 3 times)
        for attempt in 0..3 {
            match self
                .agent_service
                .update_session_execution_result(session_id, final_status, final_error.as_deref())
                .await
            {
                Ok(_) => break,
                Err(e) => {
                    tracing::warn!(
                        "Failed to persist execution result for {} (attempt {}): {}",
                        session_id,
                        attempt + 1,
                        e
                    );
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }
        }

        if let Err(e) = self
            .sync_automation_builder_draft(session_id, final_status)
            .await
        {
            tracing::warn!(
                "Failed to sync automation builder draft for session {}: {}",
                session_id,
                e
            );
        }

        // Send done event to chat subscribers
        self.chat_manager
            .broadcast(
                session_id,
                StreamEvent::Done {
                    status: final_status.to_string(),
                    error: final_error.clone(),
                },
            )
            .await;

        // Complete in chat manager (removes from active tracking)
        self.chat_manager.complete(session_id).await;

        exec_result.map(|_| ())
    }

    fn append_workspace_files_to_messages_json(
        messages_json: &str,
        files: &[ChatWorkspaceFileBlock],
    ) -> Option<String> {
        if files.is_empty() {
            return None;
        }
        let mut messages: Vec<serde_json::Value> = serde_json::from_str(messages_json).ok()?;
        let assistant = messages.iter_mut().rev().find(|item| {
            item.get("role")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|role| role.eq_ignore_ascii_case("assistant"))
        })?;

        let mut content = match assistant.get("content") {
            Some(serde_json::Value::Array(items)) => items.clone(),
            Some(serde_json::Value::String(text)) => {
                vec![serde_json::json!({ "type": "text", "text": text })]
            }
            _ => Vec::new(),
        };

        let existing_paths = content
            .iter()
            .filter_map(|item| {
                let record = item.as_object()?;
                if record.get("type").and_then(serde_json::Value::as_str)? != "workspace_file" {
                    return None;
                }
                record
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .collect::<std::collections::HashSet<_>>();

        for file in files {
            if existing_paths.contains(&file.path) {
                continue;
            }
            if let Ok(value) = serde_json::to_value(file) {
                content.push(value);
            }
        }

        if content.iter().all(|item| {
            item.get("type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind == "workspace_file")
        }) {
            content.insert(
                0,
                serde_json::json!({
                    "type": "text",
                    "text": "已准备文件，可直接预览或下载。"
                }),
            );
        }

        assistant["content"] = serde_json::Value::Array(content);
        serde_json::to_string(&messages).ok()
    }

    async fn attach_pending_workspace_files_to_latest_assistant_message(
        &self,
        session_id: &str,
        outcome: &mut super::server_harness_host::ServerHarnessHostOutcome,
    ) -> Result<()> {
        let files = self
            .agent_service
            .list_pending_message_workspace_files(session_id)
            .await?;
        if files.is_empty() {
            return Ok(());
        }
        let Some(patched_messages_json) =
            Self::append_workspace_files_to_messages_json(&outcome.messages_json, &files)
        else {
            return Ok(());
        };

        outcome.messages_json = patched_messages_json.clone();
        let preview = Self::extract_last_assistant_preview(&patched_messages_json);
        outcome.last_assistant_text = if preview.trim().is_empty() {
            None
        } else {
            Some(preview.clone())
        };
        self.agent_service
            .update_session_after_message(
                session_id,
                &patched_messages_json,
                outcome.message_count,
                &preview,
                None,
                outcome.total_tokens,
                outcome.context_runtime_state.as_ref(),
            )
            .await?;
        self.agent_service
            .clear_pending_message_workspace_files(session_id)
            .await?;
        Ok(())
    }

    /// Inner execution logic, separated so that cleanup is guaranteed
    /// by the caller (`execute_chat`) regardless of how this returns.
    async fn execute_chat_inner(
        &self,
        session_id: &str,
        agent_id: &str,
        user_message: &str,
        turn_system_instruction: Option<&str>,
        cancel_token: CancellationToken,
    ) -> Result<Option<super::server_harness_host::ServerHarnessHostOutcome>> {
        // Resolve workspace_path for this chat session
        let session = self
            .agent_service
            .get_session(session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Session not found"))?;

        let workspace_binding =
            WorkspaceService::new(self.workspace_root.clone()).ensure_chat_workspace(&session)?;
        self.agent_service
            .set_session_workspace_binding(session_id, &workspace_binding)
            .await
            .map_err(|e| anyhow!("Failed to bind workspace: {}", e))?;
        let workspace_path = workspace_binding.root_path;

        let router = HostExecutionRouter::from_env();
        let mut using_direct_host = matches!(
            router.path(),
            super::server_harness_host::HostExecutionPath::DirectHarness
        );
        let cancel_token_direct = cancel_token.clone();
        let cancel_token_bridge = cancel_token;
        let target_artifacts = session
            .attached_document_ids
            .iter()
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!("document:{}", value.trim()))
            .collect::<Vec<_>>();
        let result_contract = target_artifacts.clone();
        let exec_result = if using_direct_host {
            match self
                .agent_service
                .try_acquire_execution_slot(agent_id)
                .await
                .map_err(|e| anyhow!("Failed to acquire agent execution slot: {}", e))?
            {
                ExecutionSlotAcquireOutcome::Acquired => {
                    let direct_result = async {
                        let agent = self
                            .agent_service
                            .get_agent_with_key(agent_id)
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
                            host.execute_chat_host(
                                &session,
                                &agent,
                                user_message,
                                workspace_path.clone(),
                                turn_system_instruction,
                                target_artifacts.clone(),
                                result_contract.clone(),
                                false,
                                cancel_token_direct.clone(),
                                self.chat_manager.clone(),
                            ),
                        )
                        .await
                        {
                            Ok(result) => result.map(Some),
                            Err(_) => {
                                cancel_token_direct.cancel();
                                Err(anyhow!(
                                    "Direct harness chat host timed out after {}s",
                                    timeout_secs
                                ))
                            }
                        }
                    }
                    .await;

                    if let Err(error) = self.agent_service.release_execution_slot(agent_id).await {
                        tracing::warn!(
                            "Failed to release execution slot for agent {} after direct chat: {}",
                            agent_id,
                            error
                        );
                    }
                    if let Err(error) = execution_admission::start_next_queued_tasks_for_agent(
                        &self.db,
                        &self.agent_service,
                        &self.internal_task_manager,
                        agent_id,
                    )
                    .await
                    {
                        tracing::warn!(
                            "Failed to dispatch queued work for agent {} after direct chat: {}",
                            agent_id,
                            error
                        );
                    }

                    direct_result
                }
                ExecutionSlotAcquireOutcome::Saturated => {
                    using_direct_host = false;
                    let _ = self
                        .agent_service
                        .set_session_execution_state(session_id, "queued", true)
                        .await;
                    self.chat_manager
                        .broadcast(
                            session_id,
                            StreamEvent::Status {
                                status: "queued".to_string(),
                            },
                        )
                        .await;
                    runtime::execute_bridge_request(
                        &self.db,
                        &self.agent_service,
                        &self.internal_task_manager,
                        &self.chat_manager,
                        runtime::BridgeExecutionRequest {
                            context_id: session_id.to_string(),
                            agent_id: agent_id.to_string(),
                            session_id: session_id.to_string(),
                            channel_id: None,
                            run_scope_id: None,
                            user_message: user_message.to_string(),
                            cancel_token: cancel_token_bridge,
                            workspace_path: Some(workspace_path),
                            llm_overrides: None,
                            turn_system_instruction: turn_system_instruction.map(str::to_string),
                        },
                    )
                    .await?;
                    Ok(None)
                }
            }
        } else {
            runtime::execute_bridge_request(
                &self.db,
                &self.agent_service,
                &self.internal_task_manager,
                &self.chat_manager,
                runtime::BridgeExecutionRequest {
                    context_id: session_id.to_string(),
                    agent_id: agent_id.to_string(),
                    session_id: session_id.to_string(),
                    channel_id: None,
                    run_scope_id: None,
                    user_message: user_message.to_string(),
                    cancel_token: cancel_token_bridge,
                    workspace_path: Some(workspace_path),
                    llm_overrides: None,
                    turn_system_instruction: turn_system_instruction.map(str::to_string),
                },
            )
            .await?;
            Ok(None)
        };

        // Bridge path still needs the legacy metadata updater.
        // Direct host persists history/preview/title via SessionHostPersistenceAdapter.
        if !using_direct_host {
            self.update_session_metadata(session_id, user_message).await;
        }

        exec_result
    }

    /// Update session metadata after message completion
    async fn update_session_metadata(&self, session_id: &str, user_message: &str) {
        // Load updated session to get latest messages
        let session = match self.agent_service.get_session(session_id).await {
            Ok(Some(s)) => s,
            _ => return,
        };

        // Auto-generate title from first user message if not set
        let title = if session.title.is_none() {
            // H2 fix: use chars() for safe Unicode slicing
            let preview: String = if user_message.chars().count() > 50 {
                let truncated: String = user_message.chars().take(47).collect();
                format!("{}...", truncated)
            } else {
                user_message.to_string()
            };
            Some(preview)
        } else {
            None
        };

        // Build last message preview from assistant response
        let preview = Self::extract_last_assistant_preview(&session.messages_json);

        let _ = self
            .agent_service
            .update_session_after_message(
                session_id,
                &session.messages_json,
                session.message_count,
                &preview,
                title.as_deref(),
                session.total_tokens,
                session.context_runtime_state.as_ref(),
            )
            .await;
    }

    fn extract_last_assistant_preview(messages_json: &str) -> String {
        let msgs: Vec<serde_json::Value> = match serde_json::from_str(messages_json) {
            Ok(m) => m,
            Err(_) => return String::new(),
        };

        let msg = msgs
            .iter()
            .rev()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"));

        let Some(content) = msg.and_then(|m| m.get("content")) else {
            return String::new();
        };

        // String content
        if let Some(text) = content.as_str() {
            return text.chars().take(200).collect();
        }

        // Array content format: find first non-empty text block
        content
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                    .find(|text| !text.is_empty())
            })
            .map(|text| text.chars().take(200).collect())
            .unwrap_or_default()
    }

    async fn sync_automation_builder_draft(
        &self,
        session_id: &str,
        final_status: &str,
    ) -> Result<()> {
        let automation_service = AutomationService::new(self.db.clone());
        let Some(draft) = automation_service
            .get_task_draft_by_builder_session(session_id)
            .await?
        else {
            return Ok(());
        };

        let Some(session) = self.agent_service.get_session(session_id).await? else {
            return Ok(());
        };
        let integrations = automation_service
            .get_integrations_by_ids(&draft.team_id, &draft.integration_ids)
            .await?;
        let sync_payload = derive_builder_sync_payload(
            session_id,
            &session.messages_json,
            session.last_message_preview.as_deref(),
            final_status,
            draft.status,
            &integrations,
        );

        automation_service
            .complete_task_draft_probe(
                &draft.team_id,
                &draft.draft_id,
                sync_payload.status,
                sync_payload.probe_report,
                sync_payload.candidate_plan,
            )
            .await?;

        let _ = self
            .agent_service
            .update_session_preview(session_id, &sync_payload.session_preview)
            .await;

        Ok(())
    }
}
