//! Chat executor for direct session-based chat (bypasses Task system)
//!
//! ChatExecutor handles chat message execution directly on sessions.
//! It internally creates a temporary task to reuse TaskExecutor's
//! proven execution logic, but the task is ephemeral and not persisted
//! to the agent_tasks collection.

use agime_team::MongoDb;
use anyhow::{anyhow, Result};

use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::automation::{contract::derive_builder_sync_payload, service::AutomationService};

use super::chat_manager::ChatManager;
use super::host_router::{ChatHostRouteRequest, HostExecutionRouter};
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
        // Run the inner logic; regardless of success/failure/panic,
        // we always clean up processing state and send done event.
        let exec_result = self
            .execute_chat_inner(
                session_id,
                agent_id,
                user_message,
                turn_system_instruction,
                cancel_token,
            )
            .await;

        let (final_status, final_error) = match &exec_result {
            Ok(Some(outcome)) if outcome.execution_status() == "completed" => ("completed", None),
            Ok(Some(outcome)) => (
                "failed",
                outcome
                    .completion_report
                    .as_ref()
                    .and_then(|report| report.blocking_reason.clone())
                    .or_else(|| Some("execute host returned blocked completion".to_string())),
            ),
            Ok(None) => ("completed", None),
            Err(e) => ("failed", Some(e.to_string())),
        };

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
        match &exec_result {
            Ok(Some(outcome)) if outcome.execution_status() == "completed" => {
                self.chat_manager
                    .broadcast(
                        session_id,
                        StreamEvent::Done {
                            status: "completed".to_string(),
                            error: None,
                        },
                    )
                    .await;
            }
            Ok(Some(_)) => {
                self.chat_manager
                    .broadcast(
                        session_id,
                        StreamEvent::Done {
                            status: "failed".to_string(),
                            error: final_error.clone(),
                        },
                    )
                    .await;
            }
            Ok(None) => {
                self.chat_manager
                    .broadcast(
                        session_id,
                        StreamEvent::Done {
                            status: "completed".to_string(),
                            error: None,
                        },
                    )
                    .await;
            }
            Err(e) => {
                self.chat_manager
                    .broadcast(
                        session_id,
                        StreamEvent::Done {
                            status: "failed".to_string(),
                            error: Some(e.to_string()),
                        },
                    )
                    .await;
            }
        }

        // Complete in chat manager (removes from active tracking)
        self.chat_manager.complete(session_id).await;

        exec_result.map(|_| ())
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
        let using_direct_host = matches!(
            router.path(),
            super::server_harness_host::HostExecutionPath::DirectHarness
        );
        let cancel_token_direct = cancel_token.clone();
        let cancel_token_bridge = cancel_token;
        let exec_result = router
            .route_chat(
                ChatHostRouteRequest {
                    session_id: session_id.to_string(),
                    agent_id: agent_id.to_string(),
                    user_message: user_message.to_string(),
                    workspace_path,
                    turn_system_instruction: turn_system_instruction.map(str::to_string),
                    target_artifacts: session
                        .attached_document_ids
                        .iter()
                        .filter(|value| !value.trim().is_empty())
                        .map(|value| format!("document:{}", value.trim()))
                        .collect(),
                    result_contract: session
                        .attached_document_ids
                        .iter()
                        .filter(|value| !value.trim().is_empty())
                        .map(|value| format!("document:{}", value.trim()))
                        .collect(),
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
                        host.execute_chat_host(
                            &session,
                            &agent,
                            &request.user_message,
                            request.workspace_path,
                            request.turn_system_instruction.as_deref(),
                            request.target_artifacts,
                            request.result_contract,
                            request.validation_mode,
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
                },
                |request| async move {
                    runtime::execute_bridge_request(
                        &self.db,
                        &self.agent_service,
                        &self.internal_task_manager,
                        &self.chat_manager,
                        runtime::BridgeExecutionRequest {
                            context_id: request.session_id.clone(),
                            agent_id: request.agent_id,
                            session_id: request.session_id,
                            user_message: request.user_message,
                            cancel_token: cancel_token_bridge,
                            workspace_path: Some(request.workspace_path),
                            llm_overrides: None,
                            turn_system_instruction: request.turn_system_instruction,
                        },
                    )
                    .await?;
                    Ok(None)
                },
            )
            .await;

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
