//! DirectHarness V4 runner for `/api/team/agent/tasks`.
//!
//! This replaces the legacy admission path for AgentTask while
//! keeping the HTTP API, queue semantics, and SSE surface unchanged.

use std::sync::Arc;

use agime::context_runtime::ContextRuntimeState;
use agime_runtime::agent_task_payload::{
    direct_host_timeout_secs, is_success_status, parse_task_payload, ParsedTaskPayload,
};
use agime_team::models::{AgentTask, TaskResultType, TaskStatus};
use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::server_harness_host::{ServerHarnessHost, ServerHarnessHostOutcome};
use super::service_mongo::AgentService;
use super::session_mongo::{AgentSessionDoc, CreateSessionRequest};
use super::task_manager::{StreamEvent, TaskManager};
use super::workspace_service::WorkspaceService;

#[derive(Clone)]
pub struct AgentTaskV4Runner {
    db: Arc<MongoDb>,
    agent_service: Arc<AgentService>,
    task_manager: Arc<TaskManager>,
    workspace_root: String,
}

impl AgentTaskV4Runner {
    pub fn new(
        db: Arc<MongoDb>,
        agent_service: Arc<AgentService>,
        task_manager: Arc<TaskManager>,
        workspace_root: String,
    ) -> Self {
        Self {
            db,
            agent_service,
            task_manager,
            workspace_root,
        }
    }

    pub async fn execute_task(&self, task_id: &str, cancel_token: CancellationToken) -> Result<()> {
        let task = self
            .agent_service
            .get_task(task_id)
            .await?
            .ok_or_else(|| anyhow!("Task {} not found", task_id))?;
        if matches!(task.status, TaskStatus::Cancelled) {
            self.settle_cancelled(task_id, None).await;
            return Ok(());
        }
        if !matches!(task.status, TaskStatus::Approved | TaskStatus::Queued) {
            return Err(anyhow!("Task {} is not ready to run", task_id));
        }

        let payload = parse_task_payload(&task.content)?;
        let agent = self
            .agent_service
            .get_agent_with_key(&task.agent_id)
            .await?
            .ok_or_else(|| anyhow!("Agent {} not found", task.agent_id))?;

        let mut session = self.ensure_task_session(&task, &payload).await?;
        let workspace_path = self.bind_workspace(&mut session, &payload).await?;
        self.seed_prior_messages(&mut session, &payload).await?;

        self.agent_service.mark_task_running(task_id).await?;
        let _ = self
            .agent_service
            .set_session_execution_state(&session.session_id, "running", true)
            .await;
        self.task_manager
            .broadcast(
                task_id,
                StreamEvent::SessionId {
                    session_id: session.session_id.clone(),
                },
            )
            .await;
        self.task_manager
            .broadcast(
                task_id,
                StreamEvent::Status {
                    status: "running".to_string(),
                },
            )
            .await;

        let host = ServerHarnessHost::new(
            self.db.clone(),
            self.agent_service.clone(),
            self.task_manager.clone(),
        );
        let timeout_secs = direct_host_timeout_secs();
        let host_result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            host.execute_agent_task_host(
                &session,
                &agent,
                &payload.user_message,
                workspace_path,
                payload.turn_system_instruction.as_deref(),
                payload.target_artifacts.clone(),
                payload.result_contract.clone(),
                payload.validation_mode,
                payload.llm_overrides.clone(),
                cancel_token.clone(),
                task_id.to_string(),
                self.task_manager.clone(),
            ),
        )
        .await;

        match host_result {
            Ok(Ok(outcome)) => {
                self.settle_success_or_contract_failure(task_id, &session, outcome)
                    .await
            }
            Ok(Err(error)) => {
                if cancel_token.is_cancelled() {
                    self.settle_cancelled(task_id, Some(&session)).await;
                    return Ok(());
                }
                self.settle_failure(task_id, Some(&session), &error.to_string())
                    .await;
                Err(error)
            }
            Err(_) => {
                cancel_token.cancel();
                let error = format!(
                    "DirectHarness V4 AgentTask timed out after {}s",
                    timeout_secs
                );
                self.settle_failure(task_id, Some(&session), &error).await;
                Err(anyhow!(error))
            }
        }
    }

    async fn ensure_task_session(
        &self,
        task: &AgentTask,
        payload: &ParsedTaskPayload,
    ) -> Result<AgentSessionDoc> {
        let existing = task
            .content
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let mut session = if let Some(session_id) = existing {
            match self.agent_service.get_session(&session_id).await? {
                Some(session) => session,
                None => {
                    tracing::warn!(
                        task_id = %task.id,
                        session_id = %session_id,
                        "AgentTask session_id was missing; creating an agent_task hidden session"
                    );
                    self.create_hidden_session(task, payload).await?
                }
            }
        } else {
            self.create_hidden_session(task, payload).await?
        };

        // AgentTask is an execution surface. Existing sessions are reused for
        // persistence, but the in-memory execution policy is forced to V4 execute.
        session.session_source = "agent_task".to_string();
        session.hidden_from_chat_list = true;
        session.require_final_report = true;
        if payload.allowed_extensions.is_some() {
            session.allowed_extensions = payload.allowed_extensions.clone();
        }
        if payload.allowed_skill_ids.is_some() {
            session.allowed_skill_ids = payload.allowed_skill_ids.clone();
        }
        if !payload.attached_document_ids.is_empty() {
            session.attached_document_ids = payload.attached_document_ids.clone();
        }
        if session.context_runtime_state.is_none() {
            let context_runtime_state = ContextRuntimeState::default();
            self.agent_service
                .set_session_context_runtime_state(&session.session_id, &context_runtime_state)
                .await?;
            session.context_runtime_state = Some(context_runtime_state);
        }
        Ok(session)
    }

    async fn create_hidden_session(
        &self,
        task: &AgentTask,
        payload: &ParsedTaskPayload,
    ) -> Result<AgentSessionDoc> {
        let session = self
            .agent_service
            .create_session(CreateSessionRequest {
                team_id: task.team_id.clone(),
                agent_id: task.agent_id.clone(),
                user_id: task.submitter_id.clone(),
                name: None,
                attached_document_ids: payload.attached_document_ids.clone(),
                extra_instructions: task
                    .content
                    .get("extra_instructions")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                allowed_extensions: payload.allowed_extensions.clone(),
                allowed_skill_ids: payload.allowed_skill_ids.clone(),
                retry_config: payload.retry_config.clone(),
                max_turns: payload.max_turns,
                tool_timeout_seconds: task
                    .content
                    .get("tool_timeout_seconds")
                    .and_then(Value::as_u64),
                max_portal_retry_rounds: None,
                require_final_report: true,
                portal_restricted: false,
                document_access_mode: None,
                document_scope_mode: None,
                document_write_mode: None,
                delegation_policy_override: None,
                session_source: Some("agent_task".to_string()),
                source_channel_id: None,
                source_channel_name: None,
                source_thread_root_id: None,
                hidden_from_chat_list: Some(true),
            })
            .await?;
        Ok(session)
    }

    async fn bind_workspace(
        &self,
        session: &mut AgentSessionDoc,
        payload: &ParsedTaskPayload,
    ) -> Result<String> {
        if let Some(path) = &payload.workspace_path {
            session.workspace_path = Some(path.clone());
        }
        let binding =
            WorkspaceService::new(self.workspace_root.clone()).ensure_chat_workspace(session)?;
        self.agent_service
            .set_session_workspace_binding(&session.session_id, &binding)
            .await?;
        session.workspace_path = Some(binding.root_path.clone());
        session.workspace_id = Some(binding.workspace_id);
        session.workspace_kind = Some(binding.workspace_kind.as_str().to_string());
        session.workspace_manifest_path = Some(binding.manifest_path);
        Ok(binding.root_path)
    }

    async fn seed_prior_messages(
        &self,
        session: &mut AgentSessionDoc,
        payload: &ParsedTaskPayload,
    ) -> Result<()> {
        if !session.messages_json.trim().eq("[]") || payload.prior_messages.is_empty() {
            return Ok(());
        }
        let messages_json = serde_json::to_string(&payload.prior_messages)?;
        self.agent_service
            .update_session_messages(
                &session.session_id,
                &messages_json,
                payload.prior_messages.len() as i32,
                None,
                None,
                None,
                None,
            )
            .await?;
        session.messages_json = messages_json;
        session.message_count = payload.prior_messages.len() as i32;
        Ok(())
    }

    async fn settle_success_or_contract_failure(
        &self,
        task_id: &str,
        session: &AgentSessionDoc,
        outcome: ServerHarnessHostOutcome,
    ) -> Result<()> {
        if self.task_is_cancelled(task_id).await {
            self.settle_cancelled(task_id, Some(session)).await;
            return Ok(());
        }
        let status = outcome.execution_status().to_string();
        if status.eq_ignore_ascii_case("cancelled") {
            self.settle_cancelled(task_id, Some(session)).await;
            return Ok(());
        }
        let summary = outcome
            .user_visible_summary()
            .or_else(|| {
                outcome
                    .runtime_diagnostics()
                    .as_ref()
                    .map(|report| report.summary.clone())
            })
            .unwrap_or_else(|| "DirectHarness V4 completed without visible summary.".to_string());
        let report = outcome.runtime_diagnostics();
        let result_content = json!({
            "text": summary,
            "runtime": "direct_harness_v4",
            "session_id": &session.session_id,
            "execution_status": status.clone(),
            "completion_report": report.clone(),
            "message_count": outcome.message_count,
            "total_tokens": outcome.total_tokens,
            "context_runtime_state": outcome.context_runtime_state,
            "events_emitted": outcome.events_emitted
        });

        if is_success_status(&status) {
            self.agent_service
                .save_task_result(task_id, TaskResultType::Message, result_content)
                .await?;
            self.agent_service.complete_task(task_id).await?;
            let _ = self
                .agent_service
                .update_session_execution_result(&session.session_id, "completed", None)
                .await;
            self.task_manager
                .broadcast(
                    task_id,
                    StreamEvent::Done {
                        status: "completed".to_string(),
                        error: None,
                    },
                )
                .await;
            return Ok(());
        }

        let error = report
            .as_ref()
            .and_then(|report| report.blocking_reason.clone())
            .unwrap_or_else(|| format!("AgentTask contract was not satisfied: {}", status));
        self.agent_service
            .save_task_result(task_id, TaskResultType::Error, result_content)
            .await?;
        self.agent_service.fail_task(task_id, &error).await?;
        let _ = self
            .agent_service
            .update_session_execution_result(&session.session_id, "failed", Some(&error))
            .await;
        self.task_manager
            .broadcast(
                task_id,
                StreamEvent::Done {
                    status: "failed".to_string(),
                    error: Some(error),
                },
            )
            .await;
        Ok(())
    }

    async fn settle_cancelled(&self, task_id: &str, session: Option<&AgentSessionDoc>) {
        let content = json!({
            "text": "Task execution was cancelled.",
            "runtime": "direct_harness_v4",
            "session_id": session.map(|value| value.session_id.as_str()),
            "execution_status": "cancelled"
        });
        if let Err(db_error) = self
            .agent_service
            .save_task_result(task_id, TaskResultType::Message, content)
            .await
        {
            tracing::warn!(task_id = %task_id, error = %db_error, "failed to save AgentTask cancellation result");
        }
        if let Err(db_error) = self.agent_service.cancel_task(task_id).await {
            tracing::warn!(task_id = %task_id, error = %db_error, "failed to mark AgentTask cancelled");
        }
        if let Some(session) = session {
            let _ = self
                .agent_service
                .update_session_execution_result(&session.session_id, "cancelled", None)
                .await;
        }
        self.task_manager
            .broadcast(
                task_id,
                StreamEvent::Done {
                    status: "cancelled".to_string(),
                    error: None,
                },
            )
            .await;
    }

    async fn settle_failure(&self, task_id: &str, session: Option<&AgentSessionDoc>, error: &str) {
        if self.task_is_cancelled(task_id).await {
            self.settle_cancelled(task_id, session).await;
            return;
        }
        let content = json!({
            "text": error,
            "runtime": "direct_harness_v4",
            "session_id": session.map(|value| value.session_id.as_str()),
            "execution_status": "failed"
        });
        if let Err(db_error) = self
            .agent_service
            .save_task_result(task_id, TaskResultType::Error, content)
            .await
        {
            tracing::warn!(task_id = %task_id, error = %db_error, "failed to save AgentTask error result");
        }
        if let Err(db_error) = self.agent_service.fail_task(task_id, error).await {
            tracing::warn!(task_id = %task_id, error = %db_error, "failed to mark AgentTask failed");
        }
        if let Some(session) = session {
            let _ = self
                .agent_service
                .update_session_execution_result(&session.session_id, "failed", Some(error))
                .await;
        }
        self.task_manager
            .broadcast(
                task_id,
                StreamEvent::Done {
                    status: "failed".to_string(),
                    error: Some(error.to_string()),
                },
            )
            .await;
    }

    async fn task_is_cancelled(&self, task_id: &str) -> bool {
        self.agent_service
            .get_task(task_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|task| matches!(task.status, TaskStatus::Cancelled))
    }
}
