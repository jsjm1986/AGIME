//! Mission executor for multi-step autonomous task execution (Phase 2)
//!
//! MissionExecutor orchestrates mission lifecycle:
//! 1. Create dedicated AgentSession for cross-step context
//! 2. Generate execution plan via Agent (Planning phase)
//! 3. Execute steps sequentially, bridging to TaskExecutor
//! 4. Handle checkpoints, approvals, and cancellation
//! 5. Track token budget and artifacts

use agime::prompt_template;
use agime_team::MongoDb;
use anyhow::{anyhow, Result};

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use super::adaptive_executor::AdaptiveExecutor;
use super::mission_manager::MissionManager;
use super::mission_mongo::{
    resolve_execution_profile, ApprovalPolicy, ExecutionMode, ExecutionProfile, MissionDoc,
    MissionStatus, MissionStep, RuntimeContract, RuntimeContractVerification, StepStatus,
    ToolCallRecord,
};
use super::mission_verifier;
use super::runtime;
use super::service_mongo::AgentService;
use super::task_manager::{StreamEvent, TaskManager};

/// Maximum number of re-plan evaluations per mission execution.
const MAX_REPLAN_COUNT: u32 = 5;
/// Fast profile defaults (for simple missions).
const DEFAULT_FAST_SESSION_MAX_TURNS: i32 = 8;
const MAX_FAST_SESSION_MAX_TURNS: i32 = 128;
const DEFAULT_FAST_STEP_TIMEOUT_SECS: u64 = 300;
const DEFAULT_FAST_STEP_MAX_RETRIES: u32 = 1;
/// Full profile default max turns (0 or negative via env disables this cap).
const DEFAULT_FULL_SESSION_MAX_TURNS: i32 = 48;
const MAX_FULL_SESSION_MAX_TURNS: i32 = 5000;

/// Default timeout for a single step execution (20 minutes).
const DEFAULT_STEP_EXECUTION_TIMEOUT_SECS: u64 = 1200;
/// Minimum timeout floor for any step execution.
const DEFAULT_MIN_STEP_EXECUTION_TIMEOUT_SECS: u64 = 180;
/// Minimum timeout floor for complex steps (artifact/check/subagent heavy).
const DEFAULT_COMPLEX_STEP_EXECUTION_TIMEOUT_SECS: u64 = 600;
/// Maximum allowed step timeout from config/request (2 hours).
const MAX_STEP_EXECUTION_TIMEOUT_SECS: u64 = 7200;
/// Default timeout for planning phase (5 minutes).
const DEFAULT_MISSION_PLANNING_TIMEOUT_SECS: u64 = 300;
/// Maximum planning timeout (30 minutes).
const MAX_MISSION_PLANNING_TIMEOUT_SECS: u64 = 1800;
/// Grace window after planning timeout cancellation for cleanup.
const DEFAULT_PLANNING_TIMEOUT_CANCEL_GRACE_SECS: u64 = 20;
/// Max allowed planning timeout cancellation grace.
const MAX_PLANNING_TIMEOUT_CANCEL_GRACE_SECS: u64 = 120;
/// Grace window after timeout cancellation for bridge/task cleanup.
const DEFAULT_STEP_TIMEOUT_CANCEL_GRACE_SECS: u64 = 20;
/// Max allowed timeout cancellation grace.
const MAX_STEP_TIMEOUT_CANCEL_GRACE_SECS: u64 = 120;
/// Default cap for how many timeout failures can be retried within one step.
const DEFAULT_STEP_TIMEOUT_RETRY_LIMIT: u32 = 1;
/// Hard cap for step retries to avoid pathological settings.
const MAX_STEP_RETRY_LIMIT: u32 = 8;
/// Keep retry context prompt compact and focused.
const RETRY_CONTEXT_TOOL_CALL_LIMIT: usize = 12;
const RETRY_CONTEXT_OUTPUT_LIMIT: usize = 1200;
/// Guardrails for post-step completion checks.
const MAX_STEP_REQUIRED_ARTIFACTS: usize = 16;
const MAX_STEP_COMPLETION_CHECKS: usize = 8;
const MAX_STEP_COMPLETION_CHECK_CMD_LEN: usize = 300;
const DEFAULT_COMPLETION_CHECK_TIMEOUT_SECS: u64 = 45;
const MAX_COMPLETION_CHECK_TIMEOUT_SECS: u64 = 600;
const MISSION_PREFLIGHT_TOOL_NAME: &str = "mission_preflight__preflight";
const MISSION_VERIFY_CONTRACT_TOOL_NAME: &str = "mission_preflight__verify_contract";

#[derive(Debug, Clone)]
struct ExecutionRuntimeConfig {
    requested_profile: ExecutionProfile,
    resolved_profile: ExecutionProfile,
    skip_planning: bool,
    session_max_turns: Option<i32>,
    mission_step_timeout_seconds: Option<u64>,
    mission_step_max_retries: Option<u32>,
    synthesize_summary: bool,
}

#[derive(Debug, Clone)]
struct RecoveredExternalOutput {
    source_path: String,
    recovered_relative_path: String,
}

#[derive(serde::Serialize)]
struct MissionPlanTemplateContext<'a> {
    goal: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<&'a str>,
}

#[derive(serde::Serialize)]
struct MissionReplanTemplateContext<'a> {
    completed_steps: &'a str,
    remaining_steps: &'a str,
}

#[derive(serde::Serialize)]
struct MissionSummaryTemplateContext<'a> {
    step_summaries: &'a str,
}

/// Mission executor that orchestrates multi-step task execution
pub struct MissionExecutor {
    db: Arc<MongoDb>,
    mission_manager: Arc<MissionManager>,
    agent_service: Arc<AgentService>,
    internal_task_manager: Arc<TaskManager>,
    workspace_root: String,
}

impl MissionExecutor {
    pub fn new(
        db: Arc<MongoDb>,
        mission_manager: Arc<MissionManager>,
        workspace_root: String,
    ) -> Self {
        let agent_service = Arc::new(AgentService::new(db.clone()));
        let internal_task_manager = Arc::new(TaskManager::new());
        Self {
            db,
            mission_manager,
            agent_service,
            internal_task_manager,
            workspace_root,
        }
    }

    /// Execute a mission (outer method with guaranteed cleanup).
    pub async fn execute_mission(
        &self,
        mission_id: &str,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        let exec_result = self.execute_mission_inner(mission_id, cancel_token).await;

        // Guaranteed cleanup: send done event and complete in manager.
        // Read actual mission status to determine the correct Done event
        // (handles Planned pause, checkpoint Paused, Completed, etc.)
        match &exec_result {
            Ok(_) => {
                let done_status = self
                    .agent_service
                    .get_mission(mission_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|m| match m.status {
                        MissionStatus::Planned => "planned",
                        MissionStatus::Paused => "paused",
                        MissionStatus::Completed => "completed",
                        MissionStatus::Cancelled => "cancelled",
                        _ => "completed",
                    })
                    .unwrap_or("completed");

                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Done {
                            status: done_status.to_string(),
                            error: None,
                        },
                    )
                    .await;
            }
            Err(e) => {
                let mut done_status = "failed";
                let mut done_error = Some(e.to_string());
                let mut should_persist_failure = true;

                if let Ok(Some(mission)) = self.agent_service.get_mission(mission_id).await {
                    match mission.status {
                        // User initiated pause/cancel can surface as cancellation error.
                        // Keep the semantic status instead of force-overwriting to failed.
                        MissionStatus::Paused => {
                            done_status = "paused";
                            done_error = None;
                            should_persist_failure = false;
                        }
                        MissionStatus::Cancelled => {
                            done_status = "cancelled";
                            done_error = None;
                            should_persist_failure = false;
                        }
                        _ => {}
                    }
                }

                if should_persist_failure {
                    self.persist_failure_state(mission_id, &e.to_string()).await;
                }

                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Done {
                            status: done_status.to_string(),
                            error: done_error,
                        },
                    )
                    .await;
            }
        }

        self.archive_terminal_mission_session(mission_id).await;

        self.mission_manager.complete(mission_id).await;
        exec_result
    }

    async fn archive_terminal_mission_session(&self, mission_id: &str) {
        let mission = match self.agent_service.get_mission(mission_id).await {
            Ok(Some(m)) => m,
            _ => return,
        };

        if !matches!(
            mission.status,
            MissionStatus::Completed | MissionStatus::Cancelled
        ) {
            return;
        }

        let Some(session_id) = mission.session_id.as_deref() else {
            return;
        };

        if let Err(e) = self.agent_service.archive_session(session_id).await {
            tracing::warn!(
                "Failed to archive terminal mission session {} for mission {}: {}",
                session_id,
                mission_id,
                e
            );
        }
    }

    /// Inner execution logic for mission lifecycle.
    async fn execute_mission_inner(
        &self,
        mission_id: &str,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        // 1. Load mission
        let mission = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Mission not found"))?;

        // Dispatch to AdaptiveExecutor for adaptive mode
        if mission.execution_mode == ExecutionMode::Adaptive {
            let adaptive = AdaptiveExecutor::new(
                self.db.clone(),
                self.mission_manager.clone(),
                self.workspace_root.clone(),
            );
            return adaptive.execute_adaptive(mission_id, cancel_token).await;
        }

        if mission.status != MissionStatus::Draft && mission.status != MissionStatus::Planned {
            return Err(anyhow!(
                "Mission must be in Draft or Planned status to start"
            ));
        }

        let runtime_cfg = Self::resolve_execution_runtime(&mission);

        // Create workspace directory for this mission
        let workspace_path = runtime::create_workspace_dir(
            &self.workspace_root,
            &[
                (&mission.team_id, "team_id"),
                ("missions", "category"),
                (mission_id, "mission_id"),
            ],
        )?;
        self.agent_service
            .set_mission_workspace(mission_id, &workspace_path)
            .await
            .map_err(|e| anyhow!("Failed to set workspace: {}", e))?;

        // 2. Create dedicated AgentSession (with mission's attached documents)
        let session = self
            .agent_service
            .create_chat_session(
                &mission.team_id,
                &mission.agent_id,
                &mission.creator_id,
                mission.attached_document_ids.clone(),
                None,
                None,
                None,
                None,
                runtime_cfg.session_max_turns,
                runtime_cfg.mission_step_timeout_seconds,
                None,
                false,
                false,
                None,
                Some("mission".to_string()),
                Some(mission_id.to_string()),
                Some(true),
            )
            .await
            .map_err(|e| anyhow!("Failed to create session: {}", e))?;

        let session_id = session.session_id.clone();
        self.agent_service
            .set_mission_session(mission_id, &session_id)
            .await
            .map_err(|e| anyhow!("Failed to set session: {}", e))?;

        // 3. Planning phase
        self.agent_service
            .update_mission_status(mission_id, &MissionStatus::Planning)
            .await
            .map_err(|e| anyhow!("Failed to update status: {}", e))?;

        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::Status {
                    status: r#"{"type":"mission_planning"}"#.to_string(),
                },
            )
            .await;

        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::Status {
                    status: format!(
                        r#"{{"type":"execution_profile","requested":"{}","resolved":"{}"}}"#,
                        Self::profile_label(&runtime_cfg.requested_profile),
                        Self::profile_label(&runtime_cfg.resolved_profile),
                    ),
                },
            )
            .await;

        // 4. Generate plan (bounded by planning timeout to avoid stuck "planning")
        let steps = if runtime_cfg.skip_planning {
            vec![Self::fallback_step_from_goal(
                &mission.goal,
                runtime_cfg.mission_step_max_retries,
                runtime_cfg.mission_step_timeout_seconds,
            )]
        } else {
            let planning_timeout = Self::planning_timeout();
            let planning_cancel = CancellationToken::new();
            {
                let linked = planning_cancel.clone();
                let external = cancel_token.clone();
                tokio::spawn(async move {
                    external.cancelled().await;
                    linked.cancel();
                });
            }

            match tokio::time::timeout(
                planning_timeout,
                self.generate_plan(
                    mission_id,
                    &mission,
                    &session_id,
                    planning_cancel.clone(),
                    Some(&workspace_path),
                ),
            )
            .await
            {
                Ok(result) => result?,
                Err(_) => {
                    planning_cancel.cancel();
                    let grace = Self::planning_timeout_cancel_grace();
                    tokio::time::sleep(grace).await;
                    return Err(anyhow!(
                        "Mission planning timed out after {}s",
                        planning_timeout.as_secs()
                    ));
                }
            }
        };

        if steps.is_empty() {
            return Err(anyhow!("Agent generated empty plan"));
        }

        // 5. Save steps
        self.agent_service
            .save_mission_plan(mission_id, steps.clone())
            .await
            .map_err(|e| anyhow!("Failed to save plan: {}", e))?;

        // Check cancellation — return Ok so outer cleanup reads actual DB status
        if cancel_token.is_cancelled() {
            // Only set Cancelled if not already Paused (pause route sets Paused before cancelling token)
            if let Ok(Some(current)) = self.agent_service.get_mission(mission_id).await {
                if current.status != MissionStatus::Paused {
                    if let Err(e) = self
                        .agent_service
                        .update_mission_status(mission_id, &MissionStatus::Cancelled)
                        .await
                    {
                        tracing::warn!(
                            "Failed to mark mission {} cancelled during pre-run cancel: {}",
                            mission_id,
                            e
                        );
                    }
                }
            }
            return Ok(());
        }

        // 6. Running phase - execute steps
        self.agent_service
            .update_mission_status(mission_id, &MissionStatus::Running)
            .await
            .map_err(|e| anyhow!("Failed to update status: {}", e))?;

        self.execute_steps(
            mission_id,
            &mission,
            &session_id,
            steps,
            vec![],
            cancel_token,
            Some(&workspace_path),
            None,
            runtime_cfg,
        )
        .await
    }

    /// Execute steps sequentially with checkpoint/approval handling.
    /// Tracks completed steps for structured context passing (P0),
    /// evaluates re-planning after checkpoint steps (P1),
    /// and supports dynamic step replacement mid-execution.
    #[allow(clippy::too_many_arguments)]
    async fn execute_steps(
        &self,
        mission_id: &str,
        mission: &MissionDoc,
        session_id: &str,
        steps: Vec<MissionStep>,
        prior_completed: Vec<MissionStep>,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
        operator_hint: Option<&str>,
        runtime_cfg: ExecutionRuntimeConfig,
    ) -> Result<()> {
        let mut current_steps = steps;
        let mut completed_steps: Vec<MissionStep> = prior_completed;
        let mut replan_count: u32 = 0;
        let mut i = 0;
        let mut total_steps = completed_steps.len() + current_steps.len();

        while i < current_steps.len() {
            let step = &current_steps[i];
            let idx = step.index;
            let total = total_steps;

            // Check cancellation — return Ok so outer cleanup reads actual DB status
            if cancel_token.is_cancelled() {
                // Only set Cancelled if not already Paused
                if let Ok(Some(current)) = self.agent_service.get_mission(mission_id).await {
                    if current.status != MissionStatus::Paused {
                        if let Err(e) = self
                            .agent_service
                            .update_mission_status(mission_id, &MissionStatus::Cancelled)
                            .await
                        {
                            tracing::warn!(
                                "Failed to mark mission {} cancelled during step loop: {}",
                                mission_id,
                                e
                            );
                        }
                    }
                }
                return Ok(());
            }

            // Check token budget
            if mission.token_budget > 0 {
                let m = self
                    .agent_service
                    .get_mission(mission_id)
                    .await
                    .ok()
                    .flatten();
                if let Some(m) = m {
                    if m.total_tokens_used >= mission.token_budget {
                        if let Err(e) = self
                            .agent_service
                            .update_mission_status(mission_id, &MissionStatus::Failed)
                            .await
                        {
                            tracing::warn!(
                                "Failed to mark mission {} failed on token budget exceed: {}",
                                mission_id,
                                e
                            );
                        }
                        return Err(anyhow!("Token budget exceeded"));
                    }
                }
            }

            // Check if approval is needed. Once approved, do not re-pause this step.
            let already_approved = step.approved_by.is_some();
            let needs_approval = match mission.approval_policy {
                ApprovalPolicy::Manual => !already_approved,
                ApprovalPolicy::Checkpoint => step.is_checkpoint && !already_approved,
                ApprovalPolicy::Auto => false,
            };

            if needs_approval {
                // Pause for approval
                self.agent_service
                    .update_step_status(mission_id, idx, &StepStatus::AwaitingApproval)
                    .await
                    .map_err(|e| anyhow!("Failed to set step {} awaiting approval: {}", idx, e))?;
                self.agent_service
                    .update_mission_status(mission_id, &MissionStatus::Paused)
                    .await
                    .map_err(|e| anyhow!("Failed to pause mission {}: {}", mission_id, e))?;

                let reason = if step.is_checkpoint {
                    "checkpoint"
                } else {
                    "manual"
                };
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: format!(
                                r#"{{"type":"mission_paused","step_index":{},"reason":"{}"}}"#,
                                idx, reason
                            ),
                        },
                    )
                    .await;

                // Return - will be resumed via resume_mission
                return Ok(());
            }

            // Execute step with completed steps context
            let step_clone = step.clone();
            let policy_str = match mission.approval_policy {
                ApprovalPolicy::Auto => "auto",
                ApprovalPolicy::Checkpoint => "checkpoint",
                ApprovalPolicy::Manual => "manual",
            };
            self.run_single_step(
                mission_id,
                &mission.agent_id,
                session_id,
                idx,
                &step_clone,
                total,
                &completed_steps,
                cancel_token.clone(),
                workspace_path,
                &mission.goal,
                policy_str,
                runtime_cfg
                    .mission_step_timeout_seconds
                    .or(mission.step_timeout_seconds),
                operator_hint,
            )
            .await?;

            // Reload step from DB to get the saved output_summary
            let updated = self
                .agent_service
                .get_mission(mission_id)
                .await
                .ok()
                .flatten();
            if let Some(ref m) = updated {
                if matches!(m.status, MissionStatus::Paused | MissionStatus::Cancelled) {
                    return Ok(());
                }
                if let Some(s) = m.steps.iter().find(|s| s.index == step_clone.index) {
                    if s.status == StepStatus::Completed {
                        completed_steps.push(s.clone());
                        // Truncate old summaries to bound memory (only last 3 need full text)
                        let len = completed_steps.len();
                        if len > 3 {
                            for old in &mut completed_steps[..len - 3] {
                                old.output_summary = None;
                            }
                        }
                    }
                }
            }

            // P1: Evaluate re-planning after checkpoint steps
            // D1: Cap replan attempts to avoid infinite loops
            if step_clone.is_checkpoint
                && i + 1 < current_steps.len()
                && replan_count < MAX_REPLAN_COUNT
            {
                // B3: Replan failure is non-fatal — log and continue
                match self
                    .evaluate_replan(
                        mission_id,
                        &mission.agent_id,
                        session_id,
                        &completed_steps,
                        &current_steps[i + 1..],
                        cancel_token.clone(),
                        workspace_path,
                    )
                    .await
                {
                    Ok(Some(new_remaining)) if !new_remaining.is_empty() => {
                        replan_count += 1;

                        // Replace remaining steps with re-planned ones
                        let mut all_steps = completed_steps
                            .iter()
                            .map(|s| {
                                let mut cs = s.clone();
                                cs.status = StepStatus::Completed;
                                cs
                            })
                            .collect::<Vec<_>>();
                        all_steps.extend(new_remaining.clone());

                        self.agent_service
                            .replan_remaining_steps(mission_id, all_steps)
                            .await
                            .unwrap_or_else(|e| {
                                tracing::warn!(
                                    "Failed to persist replan for mission {}: {}",
                                    mission_id,
                                    e
                                );
                            });

                        // Continue with new remaining steps
                        current_steps = new_remaining;
                        total_steps = completed_steps.len() + current_steps.len();
                        i = 0;
                        continue;
                    }
                    Ok(Some(_)) => {
                        tracing::warn!(
                            "Mission {} replan returned empty steps, keeping current plan",
                            mission_id
                        );
                    }
                    Ok(None) => { /* keep current plan */ }
                    Err(e) => {
                        tracing::warn!(
                            "Mission {} replan evaluation failed, keeping current plan: {}",
                            mission_id,
                            e
                        );
                    }
                }
            }

            i += 1;
        }

        // Sequential mode final synthesis (best-effort, non-fatal)
        if runtime_cfg.synthesize_summary {
            if let Err(e) = self
                .synthesize_mission_summary(
                    mission_id,
                    &mission.agent_id,
                    session_id,
                    cancel_token.clone(),
                    workspace_path,
                )
                .await
            {
                tracing::warn!("Mission {} summary synthesis failed: {}", mission_id, e);
            }
        }

        // All steps completed
        self.agent_service
            .update_mission_status(mission_id, &MissionStatus::Completed)
            .await
            .map_err(|e| anyhow!("Failed to mark mission {} completed: {}", mission_id, e))?;

        Ok(())
    }

    /// Execute a single step by bridging to TaskExecutor.
    /// Includes retry logic for transient failures and output summary extraction.
    #[allow(clippy::too_many_arguments)]
    async fn run_single_step(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        step_index: u32,
        step: &MissionStep,
        total_steps: usize,
        completed_steps: &[MissionStep],
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
        mission_goal: &str,
        approval_policy: &str,
        mission_step_timeout_seconds: Option<u64>,
        operator_hint: Option<&str>,
    ) -> Result<()> {
        let tokens_before = self.get_session_total_tokens(session_id).await;
        let messages_before = self
            .agent_service
            .get_session(session_id)
            .await
            .ok()
            .flatten()
            .map(|s| runtime::count_session_messages(&s.messages_json))
            .unwrap_or(0);

        // Mark step as running
        self.agent_service
            .update_step_status(mission_id, step_index, &StepStatus::Running)
            .await
            .map_err(|e| anyhow!("Failed to mark step {} running: {}", step_index, e))?;
        self.agent_service
            .advance_mission_step(mission_id, step_index)
            .await
            .map_err(|e| anyhow!("Failed to advance mission step {}: {}", step_index, e))?;

        // Broadcast step_start
        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::Status {
                    status: format!(
                        r#"{{"type":"step_start","step_index":{},"step_title":"{}","total_steps":{}}}"#,
                        step_index,
                        step.title.replace('"', r#"\""#),
                        total_steps
                    ),
                },
            )
            .await;

        // Build base step prompt with previous step summaries (P0)
        let base_prompt = Self::build_step_prompt(
            mission_goal,
            step_index,
            step,
            total_steps,
            completed_steps,
            workspace_path,
            operator_hint,
        );

        let workspace_before = match workspace_path {
            Some(wp) => runtime::snapshot_workspace_files(wp).ok(),
            None => None,
        };

        // Build mission context for system prompt injection
        let mc_json = serde_json::json!({
            "goal": mission_goal,
            "approval_policy": approval_policy,
            "total_steps": total_steps,
            "current_step": step_index + 1,
        });

        // Execute with retry logic (P2)
        let max_retries = step.max_retries.min(MAX_STEP_RETRY_LIMIT);
        let step_timeout = Self::resolve_step_timeout(step, mission_step_timeout_seconds);
        let timeout_retry_limit = Self::step_timeout_retry_limit().min(max_retries);
        let timeout_cancel_grace = Self::step_timeout_cancel_grace();
        let mut timeout_retries_used: u32 = 0;
        let mut last_err: Option<anyhow::Error> = None;

        for attempt in 0..=max_retries {
            // B4: On retry, use prompt-driven recovery playbook with bounded context.
            let prompt = if attempt == 0 {
                base_prompt.clone()
            } else {
                let prev_err = last_err
                    .as_ref()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "unknown error".to_string());
                let (recent_tool_calls, previous_output) =
                    match self.agent_service.get_session(session_id).await {
                        Ok(Some(sess)) => (
                            runtime::recent_tool_calls_for_retry(
                                &sess.messages_json,
                                RETRY_CONTEXT_TOOL_CALL_LIMIT,
                            ),
                            runtime::latest_assistant_output_for_retry(
                                &sess.messages_json,
                                RETRY_CONTEXT_OUTPUT_LIMIT,
                            ),
                        ),
                        Ok(None) => (Vec::new(), None),
                        Err(err) => {
                            tracing::debug!(
                                "Failed to load session {} for retry context: {}",
                                session_id,
                                err
                            );
                            (Vec::new(), None)
                        }
                    };
                let playbook = runtime::render_retry_playbook(&runtime::RetryPlaybookContext {
                    mode_label: "step".to_string(),
                    unit_title: step.title.clone(),
                    attempt_number: attempt + 1,
                    max_attempts: max_retries + 1,
                    failure_message: prev_err,
                    workspace_path: workspace_path.map(|s| s.to_string()),
                    previous_output,
                    recent_tool_calls,
                });
                format!("{}\n\n{}", base_prompt, playbook)
            };

            if attempt > 0 {
                // Record retry
                self.agent_service
                    .increment_step_retry(mission_id, step_index)
                    .await
                    .unwrap_or_else(|e| {
                        tracing::warn!(
                            "Failed to increment retry count for mission {} step {}: {}",
                            mission_id,
                            step_index,
                            e
                        );
                    });
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: format!(
                                r#"{{"type":"step_retry","step_index":{},"attempt":{}}}"#,
                                step_index, attempt
                            ),
                        },
                    )
                    .await;

                // Exponential backoff with cap: 2s, 4s, 8s, 16s, 16s...
                let delay = std::time::Duration::from_secs(2u64.saturating_pow(attempt.min(4)));
                tokio::time::sleep(delay).await;
            }

            let attempt_cancel = cancel_token.child_token();
            let exec_fut = self.execute_via_bridge(
                agent_id,
                session_id,
                mission_id,
                &prompt,
                attempt_cancel.clone(),
                workspace_path,
                Some(mc_json.clone()),
            );
            tokio::pin!(exec_fut);

            let attempt_result = match tokio::time::timeout(step_timeout, &mut exec_fut).await {
                Ok(res) => res,
                Err(_) => {
                    // Timeout hit: request cancellation and allow a short grace period
                    // so bridge cleanup (Done/cleanup temp task) can complete.
                    attempt_cancel.cancel();
                    match tokio::time::timeout(timeout_cancel_grace, &mut exec_fut).await {
                        Ok(Ok(_)) => {
                            tracing::warn!(
                                "Mission {} step {} exceeded {}s timeout but completed during {}s cancel grace",
                                mission_id,
                                step_index,
                                step_timeout.as_secs(),
                                timeout_cancel_grace.as_secs()
                            );
                        }
                        Ok(Err(err)) => {
                            tracing::debug!(
                                "Mission {} step {} stopped after timeout cancellation: {}",
                                mission_id,
                                step_index,
                                err
                            );
                        }
                        Err(_) => {
                            tracing::warn!(
                                "Mission {} step {} did not stop within {}s cancel grace after timeout",
                                mission_id,
                                step_index,
                                timeout_cancel_grace.as_secs()
                            );
                        }
                    }

                    Err(anyhow!(
                        "Step {} timed out after {}s",
                        step_index + 1,
                        step_timeout.as_secs()
                    ))
                }
            };

            match attempt_result {
                Ok(_) => {
                    let tokens_after = self.get_session_total_tokens(session_id).await;
                    let tokens_used = (tokens_after - tokens_before).max(0);

                    // Extract and save output summary (P0)
                    let summary = self.extract_step_summary(session_id).await;
                    if let Some(ref s) = summary {
                        if let Err(e) = self
                            .agent_service
                            .set_step_output_summary(mission_id, step_index, s)
                            .await
                        {
                            tracing::warn!(
                                "Failed to save output summary for mission {} step {}: {}",
                                mission_id,
                                step_index,
                                e
                            );
                        }
                    }

                    // Extract and save tool call records
                    let mut step_tool_calls: Vec<ToolCallRecord> = Vec::new();
                    let mut preflight_contract: Option<runtime::MissionPreflightContract> = None;
                    let mut verify_contract_status: Option<bool> = None;
                    let mut guard_signals = runtime::ExecutionGuardSignals::default();
                    if let Ok(Some(sess)) = self.agent_service.get_session(session_id).await {
                        preflight_contract = runtime::extract_latest_preflight_contract_since(
                            &sess.messages_json,
                            messages_before,
                            MISSION_PREFLIGHT_TOOL_NAME,
                        );
                        verify_contract_status =
                            runtime::extract_latest_verify_contract_status_since(
                                &sess.messages_json,
                                messages_before,
                                MISSION_VERIFY_CONTRACT_TOOL_NAME,
                            );
                        let raw =
                            runtime::extract_tool_calls_since(&sess.messages_json, messages_before);
                        if !raw.is_empty() {
                            step_tool_calls = raw
                                .into_iter()
                                .map(|(name, success)| ToolCallRecord { name, success })
                                .collect();
                            if let Err(e) = self
                                .agent_service
                                .set_step_tool_calls(mission_id, step_index, &step_tool_calls)
                                .await
                            {
                                tracing::warn!(
                                    "Failed to save tool calls for mission {} step {}: {}",
                                    mission_id,
                                    step_index,
                                    e
                                );
                            }
                        }
                        guard_signals = runtime::collect_execution_guard_signals_since(
                            &sess.messages_json,
                            messages_before,
                            workspace_path,
                        );
                    }

                    let effective_contract = mission_verifier::resolve_effective_contract(
                        preflight_contract,
                        MISSION_PREFLIGHT_TOOL_NAME,
                        mission_verifier::VerifierLimits {
                            max_required_artifacts: MAX_STEP_REQUIRED_ARTIFACTS,
                            max_completion_checks: MAX_STEP_COMPLETION_CHECKS,
                            max_completion_check_cmd_len: MAX_STEP_COMPLETION_CHECK_CMD_LEN,
                        },
                    )?;

                    if let Some(wp) = workspace_path {
                        if !guard_signals.external_output_paths.is_empty() {
                            let unrecovered = self
                                .recover_external_outputs_to_workspace(
                                    mission_id,
                                    step_index,
                                    wp,
                                    &guard_signals.external_output_paths,
                                    &effective_contract.required_artifacts,
                                )
                                .await;
                            guard_signals.external_output_paths = unrecovered;
                        }
                    }

                    if let Err(e) = self
                        .agent_service
                        .set_step_runtime_contract(
                            mission_id,
                            step_index,
                            &Self::to_runtime_contract_doc(&effective_contract),
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist runtime contract for mission {} step {}: {}",
                            mission_id,
                            step_index,
                            e
                        );
                    }

                    let workspace_artifact_count = if let Some(wp) = workspace_path {
                        match runtime::scan_workspace_artifacts(wp, workspace_before.as_ref()) {
                            Ok(items) => items
                                .into_iter()
                                .filter(|item| {
                                    !runtime::is_low_signal_artifact_path(&item.relative_path)
                                })
                                .count(),
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to pre-scan workspace artifacts for mission {} step {}: {}",
                                    mission_id,
                                    step_index,
                                    e
                                );
                                0
                            }
                        }
                    } else {
                        0
                    };

                    if let Err(check_err) = mission_verifier::validate_contract_outputs(
                        &effective_contract,
                        workspace_path,
                        summary.as_deref(),
                        &step_tool_calls,
                        workspace_artifact_count,
                        MISSION_PREFLIGHT_TOOL_NAME,
                        mission_verifier::CompletionCheckMode::AllowShell {
                            timeout: Self::completion_check_timeout(),
                        },
                        true,
                    )
                    .await
                    {
                        let reason = check_err.to_string();
                        self.mission_manager
                            .broadcast(
                                mission_id,
                                StreamEvent::Status {
                                    status: format!(
                                        r#"{{"type":"step_validation_failed","step_index":{},"attempt":{},"reason":"{}"}}"#,
                                        step_index,
                                        attempt + 1,
                                        reason.replace('"', r#"\""#).replace('\n', " ")
                                    ),
                                },
                            )
                            .await;

                        let retry_err = anyhow!("Step completion validation failed: {}", reason);
                        if attempt < max_retries {
                            tracing::warn!(
                                "Step {}/{} attempt {} failed completion validation (will retry): {}",
                                step_index + 1,
                                total_steps,
                                attempt + 1,
                                retry_err
                            );
                            last_err = Some(retry_err);
                            continue;
                        }

                        return self
                            .finalize_step_failure(
                                mission_id,
                                session_id,
                                step_index,
                                tokens_before,
                                retry_err,
                            )
                            .await;
                    }

                    let gate_mode = runtime::contract_verify_gate_mode();
                    let verify_tool_called = mission_verifier::has_verify_contract_tool_call(
                        &step_tool_calls,
                        MISSION_VERIFY_CONTRACT_TOOL_NAME,
                    );
                    let verify_status_label = mission_verifier::verify_contract_status_label(
                        verify_tool_called,
                        verify_contract_status,
                    );
                    let gate_error = mission_verifier::enforce_verify_contract_gate(
                        gate_mode,
                        verify_tool_called,
                        verify_contract_status,
                        MISSION_VERIFY_CONTRACT_TOOL_NAME,
                    )
                    .err();
                    let gate_reason = gate_error
                        .as_ref()
                        .map(|e| e.to_string())
                        .unwrap_or_default();
                    if let Err(e) = self
                        .agent_service
                        .set_step_contract_verification(
                            mission_id,
                            step_index,
                            &RuntimeContractVerification {
                                tool_called: verify_tool_called,
                                status: Some(verify_status_label.to_string()),
                                gate_mode: Some(
                                    runtime::contract_verify_gate_mode_label(gate_mode).to_string(),
                                ),
                                accepted: Some(gate_error.is_none()),
                                reason: if gate_reason.trim().is_empty() {
                                    None
                                } else {
                                    Some(gate_reason.clone())
                                },
                                checked_at: Some(mongodb::bson::DateTime::now()),
                            },
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist contract verification for mission {} step {}: {}",
                            mission_id,
                            step_index,
                            e
                        );
                    }
                    self.mission_manager
                        .broadcast(
                            mission_id,
                            StreamEvent::Status {
                                status: format!(
                                    r#"{{"type":"step_contract_verification","step_index":{},"gate":"{}","tool_called":{},"verify_status":"{}","accepted":{},"reason":"{}"}}"#,
                                    step_index,
                                    runtime::contract_verify_gate_mode_label(gate_mode),
                                    verify_tool_called,
                                    verify_status_label,
                                    gate_error.is_none(),
                                    gate_reason.replace('"', r#"\""#).replace('\n', " ")
                                ),
                            },
                        )
                        .await;
                    if let Some(gate_err) = gate_error {
                        let retry_err =
                            anyhow!("Step contract verification gate failed: {}", gate_err);
                        if attempt < max_retries {
                            tracing::warn!(
                                "Step {}/{} attempt {} failed contract verify gate (will retry): {}",
                                step_index + 1,
                                total_steps,
                                attempt + 1,
                                retry_err
                            );
                            last_err = Some(retry_err);
                            continue;
                        }
                        return self
                            .finalize_step_failure(
                                mission_id,
                                session_id,
                                step_index,
                                tokens_before,
                                retry_err,
                            )
                            .await;
                    }

                    if guard_signals.max_turn_limit_warning {
                        let retry_err =
                            anyhow!("Step reached maximum turn limit; task may be incomplete");
                        self.mission_manager
                            .broadcast(
                                mission_id,
                                StreamEvent::Status {
                                    status: format!(
                                        r#"{{"type":"step_guard_failed","step_index":{},"attempt":{},"guard":"max_turn_limit","reason":"{}"}}"#,
                                        step_index,
                                        attempt + 1,
                                        retry_err.to_string().replace('"', r#"\""#).replace('\n', " ")
                                    ),
                                },
                            )
                            .await;
                        if attempt < max_retries {
                            tracing::warn!(
                                "Step {}/{} attempt {} hit max-turn guard (will retry): {}",
                                step_index + 1,
                                total_steps,
                                attempt + 1,
                                retry_err
                            );
                            last_err = Some(retry_err);
                            continue;
                        }
                        return self
                            .finalize_step_failure(
                                mission_id,
                                session_id,
                                step_index,
                                tokens_before,
                                retry_err,
                            )
                            .await;
                    }

                    if let Some(path) = guard_signals.external_output_paths.first() {
                        let retry_err = anyhow!(
                            "Step wrote files outside workspace: {}. Save outputs under workspace-relative paths (for example output/...)",
                            path
                        );
                        self.mission_manager
                            .broadcast(
                                mission_id,
                                StreamEvent::Status {
                                    status: format!(
                                        r#"{{"type":"step_guard_failed","step_index":{},"attempt":{},"guard":"external_workspace_path","path":"{}","reason":"{}"}}"#,
                                        step_index,
                                        attempt + 1,
                                        path.replace('"', r#"\""#).replace('\n', " "),
                                        retry_err.to_string().replace('"', r#"\""#).replace('\n', " ")
                                    ),
                                },
                            )
                            .await;
                        if attempt < max_retries {
                            tracing::warn!(
                                "Step {}/{} attempt {} hit workspace-path guard (will retry): {}",
                                step_index + 1,
                                total_steps,
                                attempt + 1,
                                retry_err
                            );
                            last_err = Some(retry_err);
                            continue;
                        }
                        return self
                            .finalize_step_failure(
                                mission_id,
                                session_id,
                                step_index,
                                tokens_before,
                                retry_err,
                            )
                            .await;
                    }

                    self.agent_service
                        .complete_step(mission_id, step_index, tokens_used)
                        .await
                        .map_err(|e| {
                            anyhow!(
                                "Failed to complete mission {} step {}: {}",
                                mission_id,
                                step_index,
                                e
                            )
                        })?;

                    if let Some(wp) = workspace_path {
                        if let Err(e) = self
                            .register_step_artifacts(
                                mission_id,
                                step_index,
                                &effective_contract.required_artifacts,
                                wp,
                                workspace_before.as_ref(),
                            )
                            .await
                        {
                            tracing::warn!(
                                "Artifact scan failed for mission {} step {}: {}",
                                mission_id,
                                step_index,
                                e
                            );
                        }
                    }

                    self.mission_manager
                        .broadcast(
                            mission_id,
                            StreamEvent::Status {
                                status: format!(
                                    r#"{{"type":"step_complete","step_index":{},"tokens_used":{}}}"#,
                                    step_index, tokens_used
                                ),
                            },
                        )
                        .await;
                    return Ok(());
                }
                Err(e) => {
                    if cancel_token.is_cancelled() {
                        if let Ok(Some(current)) = self.agent_service.get_mission(mission_id).await
                        {
                            if matches!(
                                current.status,
                                MissionStatus::Paused | MissionStatus::Cancelled
                            ) {
                                self.agent_service
                                    .update_step_status(
                                        mission_id,
                                        step_index,
                                        &StepStatus::Pending,
                                    )
                                    .await
                                    .unwrap_or_else(|err| {
                                        tracing::warn!(
                                            "Failed to reset mission {} step {} to pending after cancel: {}",
                                            mission_id,
                                            step_index,
                                            err
                                        );
                                    });
                                return Ok(());
                            }
                        }
                    }

                    let is_timeout = Self::is_timeout_error(&e);
                    let is_retryable = runtime::is_retryable_error(&e);
                    let can_retry_timeout =
                        !is_timeout || timeout_retries_used < timeout_retry_limit;
                    if is_retryable && can_retry_timeout && attempt < max_retries {
                        if is_timeout {
                            timeout_retries_used = timeout_retries_used.saturating_add(1);
                        }
                        tracing::warn!(
                            "Step {}/{} attempt {} failed (retryable, timeout={}, timeout_retries={}/{}): {}",
                            step_index + 1,
                            total_steps,
                            attempt + 1,
                            is_timeout,
                            timeout_retries_used,
                            timeout_retry_limit,
                            e
                        );
                        last_err = Some(e);
                        continue;
                    }
                    // Non-retryable or exhausted retries
                    return self
                        .finalize_step_failure(mission_id, session_id, step_index, tokens_before, e)
                        .await;
                }
            }
        }

        // Should not reach here, but handle exhausted retries
        Err(last_err.unwrap_or_else(|| anyhow!("Step failed after retries")))
    }

    fn env_u64(name: &str) -> Option<u64> {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
    }

    fn env_u32(name: &str) -> Option<u32> {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v > 0)
    }

    fn clamp_step_timeout_secs(timeout_secs: u64) -> u64 {
        timeout_secs.clamp(1, MAX_STEP_EXECUTION_TIMEOUT_SECS)
    }

    fn resolve_min_step_timeout_secs() -> u64 {
        Self::env_u64("TEAM_MISSION_MIN_STEP_TIMEOUT_SECS")
            .unwrap_or(DEFAULT_MIN_STEP_EXECUTION_TIMEOUT_SECS)
            .clamp(1, MAX_STEP_EXECUTION_TIMEOUT_SECS)
    }

    fn resolve_complex_step_timeout_secs(min_step_timeout_secs: u64) -> u64 {
        Self::env_u64("TEAM_MISSION_COMPLEX_STEP_TIMEOUT_SECS")
            .unwrap_or(DEFAULT_COMPLEX_STEP_EXECUTION_TIMEOUT_SECS)
            .clamp(min_step_timeout_secs, MAX_STEP_EXECUTION_TIMEOUT_SECS)
    }

    fn step_requires_extended_timeout(step: &MissionStep) -> bool {
        step.use_subagent
            || !step.required_artifacts.is_empty()
            || !step.completion_checks.is_empty()
    }

    fn resolve_step_timeout(
        step: &MissionStep,
        mission_step_timeout_seconds: Option<u64>,
    ) -> Duration {
        let configured_secs = step
            .timeout_seconds
            .or(mission_step_timeout_seconds)
            .or_else(|| Self::env_u64("TEAM_MISSION_STEP_TIMEOUT_SECS"))
            .unwrap_or(DEFAULT_STEP_EXECUTION_TIMEOUT_SECS);
        let clamped_secs = Self::clamp_step_timeout_secs(configured_secs);
        let min_step_timeout_secs = Self::resolve_min_step_timeout_secs();
        let min_complex_timeout_secs =
            Self::resolve_complex_step_timeout_secs(min_step_timeout_secs);
        let floor_secs = if Self::step_requires_extended_timeout(step) {
            min_complex_timeout_secs
        } else {
            min_step_timeout_secs
        };
        Duration::from_secs(clamped_secs.max(floor_secs))
    }

    fn resolve_step_max_retries(
        step_max_retries: Option<u32>,
        mission_step_max_retries: Option<u32>,
    ) -> u32 {
        step_max_retries
            .or(mission_step_max_retries)
            .unwrap_or(2)
            .min(MAX_STEP_RETRY_LIMIT)
    }

    fn env_i32(name: &str) -> Option<i32> {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<i32>().ok())
            .filter(|v| *v > 0)
    }

    fn resolve_fast_session_max_turns() -> i32 {
        Self::env_i32("TEAM_MISSION_FAST_MAX_TURNS")
            .unwrap_or(DEFAULT_FAST_SESSION_MAX_TURNS)
            .clamp(1, MAX_FAST_SESSION_MAX_TURNS)
    }

    fn resolve_fast_step_timeout_secs() -> u64 {
        Self::env_u64("TEAM_MISSION_FAST_STEP_TIMEOUT_SECS")
            .unwrap_or(DEFAULT_FAST_STEP_TIMEOUT_SECS)
            .clamp(1, MAX_STEP_EXECUTION_TIMEOUT_SECS)
    }

    fn resolve_fast_step_max_retries() -> u32 {
        Self::env_u32("TEAM_MISSION_FAST_STEP_MAX_RETRIES")
            .unwrap_or(DEFAULT_FAST_STEP_MAX_RETRIES)
            .min(MAX_STEP_RETRY_LIMIT)
    }

    fn resolve_full_session_max_turns() -> Option<i32> {
        let configured = std::env::var("TEAM_MISSION_FULL_MAX_TURNS")
            .ok()
            .and_then(|v| v.parse::<i32>().ok());
        match configured {
            Some(v) if v <= 0 => None,
            Some(v) => Some(v.clamp(1, MAX_FULL_SESSION_MAX_TURNS)),
            None => Some(DEFAULT_FULL_SESSION_MAX_TURNS),
        }
    }

    fn resolve_execution_runtime(mission: &MissionDoc) -> ExecutionRuntimeConfig {
        let requested_profile = mission.execution_profile.clone();
        let resolved_profile = resolve_execution_profile(mission);

        match resolved_profile {
            ExecutionProfile::Fast => ExecutionRuntimeConfig {
                requested_profile,
                resolved_profile,
                skip_planning: true,
                session_max_turns: Some(Self::resolve_fast_session_max_turns()),
                mission_step_timeout_seconds: Some(Self::resolve_fast_step_timeout_secs()),
                mission_step_max_retries: Some(Self::resolve_fast_step_max_retries()),
                synthesize_summary: false,
            },
            ExecutionProfile::Auto | ExecutionProfile::Full => ExecutionRuntimeConfig {
                requested_profile,
                resolved_profile,
                skip_planning: false,
                session_max_turns: Self::resolve_full_session_max_turns(),
                mission_step_timeout_seconds: mission.step_timeout_seconds,
                mission_step_max_retries: mission.step_max_retries,
                synthesize_summary: true,
            },
        }
    }

    fn profile_label(profile: &ExecutionProfile) -> &'static str {
        match profile {
            ExecutionProfile::Auto => "auto",
            ExecutionProfile::Fast => "fast",
            ExecutionProfile::Full => "full",
        }
    }

    fn planning_timeout() -> Duration {
        let secs = Self::env_u64("TEAM_MISSION_PLANNING_TIMEOUT_SECS")
            .unwrap_or(DEFAULT_MISSION_PLANNING_TIMEOUT_SECS)
            .min(MAX_MISSION_PLANNING_TIMEOUT_SECS);
        Duration::from_secs(secs)
    }

    fn planning_timeout_cancel_grace() -> Duration {
        let secs = Self::env_u64("TEAM_MISSION_PLANNING_CANCEL_GRACE_SECS")
            .unwrap_or(DEFAULT_PLANNING_TIMEOUT_CANCEL_GRACE_SECS)
            .min(MAX_PLANNING_TIMEOUT_CANCEL_GRACE_SECS);
        Duration::from_secs(secs)
    }

    fn normalize_required_artifacts(items: Vec<String>) -> Vec<String> {
        items
            .into_iter()
            .map(|s| s.trim().replace('\\', "/"))
            .filter(|s| !s.is_empty())
            .take(MAX_STEP_REQUIRED_ARTIFACTS)
            .collect()
    }

    fn normalize_completion_checks(items: Vec<String>) -> Vec<String> {
        items
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| {
                if let Some(path) = Self::extract_exists_check_path(&s) {
                    format!("exists:{}", path)
                } else {
                    s
                }
            })
            .map(|s| {
                if s.chars().count() > MAX_STEP_COMPLETION_CHECK_CMD_LEN {
                    s.chars()
                        .take(MAX_STEP_COMPLETION_CHECK_CMD_LEN)
                        .collect::<String>()
                } else {
                    s
                }
            })
            .take(MAX_STEP_COMPLETION_CHECKS)
            .collect()
    }

    fn trim_wrapping_quotes(value: &str) -> String {
        let trimmed = value.trim();
        let bytes = trimmed.as_bytes();
        if bytes.len() >= 2 {
            let first = bytes[0];
            let last = bytes[bytes.len() - 1];
            if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
                return trimmed[1..trimmed.len() - 1].trim().to_string();
            }
        }
        trimmed.to_string()
    }

    fn extract_exists_check_path(command: &str) -> Option<String> {
        let trimmed = command.trim();
        let lower = trimmed.to_ascii_lowercase();
        let raw = trimmed
            .strip_prefix("exists:")
            .or_else(|| {
                if lower.starts_with("test -f ") || lower.starts_with("test -e ") {
                    trimmed.get(8..)
                } else {
                    None
                }
            })
            .or_else(|| {
                trimmed
                    .strip_prefix("[ -f ")
                    .or_else(|| trimmed.strip_prefix("[ -e "))
                    .and_then(|s| s.strip_suffix(" ]"))
            })?;

        let path = Self::trim_wrapping_quotes(raw).replace('\\', "/");
        (!path.is_empty()).then_some(path)
    }

    fn completion_check_timeout() -> Duration {
        let secs = Self::env_u64("TEAM_MISSION_COMPLETION_CHECK_TIMEOUT_SECS")
            .unwrap_or(DEFAULT_COMPLETION_CHECK_TIMEOUT_SECS)
            .min(MAX_COMPLETION_CHECK_TIMEOUT_SECS);
        Duration::from_secs(secs)
    }

    fn to_runtime_contract_doc(contract: &runtime::MissionPreflightContract) -> RuntimeContract {
        RuntimeContract {
            required_artifacts: contract.required_artifacts.clone(),
            completion_checks: contract.completion_checks.clone(),
            no_artifact_reason: contract.no_artifact_reason.clone(),
            source: Some(MISSION_PREFLIGHT_TOOL_NAME.to_string()),
            captured_at: Some(mongodb::bson::DateTime::now()),
        }
    }

    fn escape_json_for_prompt(input: &str) -> String {
        input
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "")
    }

    async fn finalize_step_failure(
        &self,
        mission_id: &str,
        session_id: &str,
        step_index: u32,
        tokens_before: i32,
        err: anyhow::Error,
    ) -> Result<()> {
        let err_msg = err.to_string();
        let tokens_after = self.get_session_total_tokens(session_id).await;
        let tokens_used = (tokens_after - tokens_before).max(0);
        self.agent_service
            .add_mission_tokens(mission_id, tokens_used)
            .await
            .unwrap_or_else(|db_err| {
                tracing::warn!(
                    "Failed to add mission {} tokens on step failure: {}",
                    mission_id,
                    db_err
                );
            });
        if let Err(db_err) = self
            .agent_service
            .fail_step(mission_id, step_index, &err_msg)
            .await
        {
            tracing::warn!(
                "Failed to mark mission {} step {} failed: {}",
                mission_id,
                step_index,
                db_err
            );
        }
        if let Err(db_err) = self
            .agent_service
            .update_mission_status(mission_id, &MissionStatus::Failed)
            .await
        {
            tracing::warn!(
                "Failed to mark mission {} failed after step failure: {}",
                mission_id,
                db_err
            );
        }
        if let Err(db_err) = self
            .agent_service
            .set_mission_error(mission_id, &err_msg)
            .await
        {
            tracing::warn!(
                "Failed to persist mission {} error message: {}",
                mission_id,
                db_err
            );
        }
        Err(err)
    }

    fn step_timeout_cancel_grace() -> Duration {
        let secs = Self::env_u64("TEAM_MISSION_TIMEOUT_CANCEL_GRACE_SECS")
            .unwrap_or(DEFAULT_STEP_TIMEOUT_CANCEL_GRACE_SECS)
            .min(MAX_STEP_TIMEOUT_CANCEL_GRACE_SECS);
        Duration::from_secs(secs)
    }

    fn step_timeout_retry_limit() -> u32 {
        Self::env_u32("TEAM_MISSION_TIMEOUT_RETRY_LIMIT")
            .unwrap_or(DEFAULT_STEP_TIMEOUT_RETRY_LIMIT)
            .min(MAX_STEP_RETRY_LIMIT)
    }

    fn is_timeout_error(e: &anyhow::Error) -> bool {
        let msg = e.to_string().to_ascii_lowercase();
        msg.contains("timed out") || msg.contains("timeout")
    }

    async fn get_session_total_tokens(&self, session_id: &str) -> i32 {
        self.agent_service
            .get_session(session_id)
            .await
            .ok()
            .flatten()
            .and_then(|s| s.total_tokens)
            .unwrap_or(0)
    }

    /// Bridge to TaskExecutor: create temp task, execute, forward events.
    #[allow(clippy::too_many_arguments)]
    async fn execute_via_bridge(
        &self,
        agent_id: &str,
        session_id: &str,
        mission_id: &str,
        user_message: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
        mission_context: Option<serde_json::Value>,
    ) -> Result<()> {
        runtime::execute_via_bridge(
            &self.db,
            &self.agent_service,
            &self.internal_task_manager,
            &self.mission_manager,
            mission_id,
            agent_id,
            session_id,
            user_message,
            cancel_token,
            workspace_path,
            Some(mission_id),
            None,
            mission_context,
        )
        .await
    }

    /// Generate execution plan by asking the Agent.
    async fn generate_plan(
        &self,
        mission_id: &str,
        mission: &MissionDoc,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<Vec<MissionStep>> {
        let prompt = Self::render_mission_plan_prompt(&mission.goal, mission.context.as_deref());

        // Execute via bridge to get Agent response
        self.execute_via_bridge(
            &mission.agent_id,
            session_id,
            mission_id,
            &prompt,
            cancel_token,
            workspace_path,
            None, // no mission_context during planning phase
        )
        .await?;

        // Parse plan from session messages
        let session = self
            .agent_service
            .get_session(session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Session not found"))?;

        let steps = self.parse_plan_from_messages(&session.messages_json, mission)?;
        Ok(steps)
    }

    fn render_mission_plan_prompt(goal: &str, context: Option<&str>) -> String {
        let ctx = MissionPlanTemplateContext {
            goal,
            context: context.and_then(|c| {
                let trimmed = c.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            }),
        };
        match prompt_template::render_global_file("mission_plan.md", &ctx) {
            Ok(rendered) => rendered,
            Err(e) => {
                tracing::warn!("Failed to render mission_plan.md template: {}", e);
                let extra = ctx
                    .context
                    .map(|c| format!("\n## Additional Context\n{}", c))
                    .unwrap_or_default();
                format!(
                    "You are planning a mission. Before creating the plan, analyze the goal carefully.\n\n\
                     ## Mission Goal\n\
                     {}\n\
                     {}\n\n\
                     ## Instructions\n\
                     1. Analyze dependencies and possible blockers\n\
                     2. Create a concrete execution plan as JSON\n\
                     3. Prefer verifiable completion conditions and artifacts",
                    goal, extra
                )
            }
        }
    }

    /// Parse plan JSON from the last assistant message.
    fn parse_plan_from_messages(
        &self,
        messages_json: &str,
        mission: &MissionDoc,
    ) -> Result<Vec<MissionStep>> {
        let msgs: Vec<serde_json::Value> =
            serde_json::from_str(messages_json).map_err(|e| anyhow!("Invalid messages: {}", e))?;

        // Find last assistant message
        let assistant_text = msgs
            .iter()
            .rev()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
            .and_then(|m| {
                let content = m.get("content")?;
                if let Some(s) = content.as_str() {
                    return Some(s.to_string());
                }
                // Handle array content
                content.as_array().and_then(|arr| {
                    arr.iter().find_map(|item| {
                        item.get("text").and_then(|t| t.as_str()).map(String::from)
                    })
                })
            });

        let Some(assistant_text) = assistant_text else {
            tracing::warn!(
                "Mission {} planning has no assistant response, using single-step fallback",
                mission.mission_id
            );
            return Ok(vec![Self::fallback_step_from_goal(
                &mission.goal,
                mission.step_max_retries,
                mission.step_timeout_seconds,
            )]);
        };

        // Extract JSON from ```json ... ``` block or try direct parse
        let json_str = runtime::extract_json_block(&assistant_text);
        match Self::parse_steps_json(
            &json_str,
            0,
            mission.step_max_retries,
            mission.step_timeout_seconds,
        ) {
            Ok(steps) if !steps.is_empty() => Ok(steps),
            Ok(_) => {
                tracing::warn!(
                    "Mission {} planning produced empty steps, using single-step fallback",
                    mission.mission_id
                );
                Ok(vec![Self::fallback_step_from_goal(
                    &mission.goal,
                    mission.step_max_retries,
                    mission.step_timeout_seconds,
                )])
            }
            Err(e) => {
                tracing::warn!(
                    "Mission {} planning JSON parse failed: {}. Using single-step fallback",
                    mission.mission_id,
                    e
                );
                Ok(vec![Self::fallback_step_from_goal(
                    &mission.goal,
                    mission.step_max_retries,
                    mission.step_timeout_seconds,
                )])
            }
        }
    }

    /// Shared: parse a JSON string of step definitions into MissionStep entries.
    /// `start_index` offsets the step indices (0 for initial plan, N for replan).
    /// Tolerant of missing fields: title defaults to "Step N", description defaults to title.
    fn parse_steps_json(
        json_str: &str,
        start_index: usize,
        mission_step_max_retries: Option<u32>,
        mission_step_timeout_seconds: Option<u64>,
    ) -> Result<Vec<MissionStep>> {
        #[derive(serde::Deserialize)]
        struct PlanStep {
            #[serde(default)]
            title: Option<String>,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            is_checkpoint: bool,
            #[serde(default)]
            max_retries: Option<u32>,
            #[serde(default)]
            timeout_seconds: Option<u64>,
            #[serde(default)]
            required_artifacts: Vec<String>,
            #[serde(default)]
            completion_checks: Vec<String>,
            #[serde(default)]
            use_subagent: bool,
        }

        fn parse_plan_steps_value(
            value: serde_json::Value,
        ) -> Result<Vec<PlanStep>, serde_json::Error> {
            if value.is_array() {
                return serde_json::from_value(value);
            }
            if let Some(arr) = value
                .get("steps")
                .or_else(|| value.get("plan"))
                .and_then(|v| v.as_array())
            {
                return serde_json::from_value(serde_json::Value::Array(arr.clone()));
            }
            serde_json::from_value(value)
        }

        let normalized = runtime::normalize_loose_json(json_str);
        let candidates: [&str; 2] = [json_str, &normalized];
        let mut plan_steps: Option<Vec<PlanStep>> = None;
        let mut last_err = None;
        for candidate in candidates {
            match serde_json::from_str::<serde_json::Value>(candidate)
                .and_then(parse_plan_steps_value)
            {
                Ok(steps) => {
                    plan_steps = Some(steps);
                    break;
                }
                Err(e) => {
                    last_err = Some(e.to_string());
                }
            }
        }

        let plan_steps = plan_steps.ok_or_else(|| {
            anyhow!(
                "Failed to parse steps JSON: {}",
                last_err.unwrap_or_else(|| "unknown error".to_string())
            )
        })?;

        let steps = plan_steps
            .into_iter()
            .enumerate()
            .map(|(i, ps)| {
                let default_title = format!("Step {}", start_index + i + 1);
                let title = ps.title.unwrap_or(default_title);
                let description = ps.description.unwrap_or_else(|| title.clone());
                let max_retries =
                    Self::resolve_step_max_retries(ps.max_retries, mission_step_max_retries);
                let timeout_seconds = ps
                    .timeout_seconds
                    .or(mission_step_timeout_seconds)
                    .map(Self::clamp_step_timeout_secs);
                let mut required_artifacts =
                    Self::normalize_required_artifacts(ps.required_artifacts);
                for path in ps
                    .completion_checks
                    .iter()
                    .filter_map(|c| Self::extract_exists_check_path(c))
                {
                    if required_artifacts.len() >= MAX_STEP_REQUIRED_ARTIFACTS {
                        break;
                    }
                    if !required_artifacts.iter().any(|existing| existing == &path) {
                        required_artifacts.push(path);
                    }
                }
                let completion_checks = Self::normalize_completion_checks(ps.completion_checks);
                MissionStep {
                    index: (start_index + i) as u32,
                    title,
                    description,
                    status: StepStatus::Pending,
                    is_checkpoint: ps.is_checkpoint,
                    approved_by: None,
                    started_at: None,
                    completed_at: None,
                    error_message: None,
                    tokens_used: 0,
                    output_summary: None,
                    retry_count: 0,
                    max_retries,
                    timeout_seconds,
                    required_artifacts,
                    completion_checks,
                    runtime_contract: None,
                    contract_verification: None,
                    use_subagent: ps.use_subagent,
                    tool_calls: vec![],
                }
            })
            .collect();

        Ok(steps)
    }

    fn fallback_step_from_goal(
        mission_goal: &str,
        mission_step_max_retries: Option<u32>,
        mission_step_timeout_seconds: Option<u64>,
    ) -> MissionStep {
        MissionStep {
            index: 0,
            title: "执行核心目标".to_string(),
            description: mission_goal.to_string(),
            status: StepStatus::Pending,
            is_checkpoint: false,
            approved_by: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            tokens_used: 0,
            output_summary: None,
            retry_count: 0,
            max_retries: Self::resolve_step_max_retries(None, mission_step_max_retries),
            timeout_seconds: mission_step_timeout_seconds.map(Self::clamp_step_timeout_secs),
            required_artifacts: Vec::new(),
            completion_checks: Vec::new(),
            runtime_contract: None,
            contract_verification: None,
            use_subagent: false,
            tool_calls: vec![],
        }
    }

    /// Build step prompt with mission goal and previous step summaries.
    fn build_step_prompt(
        mission_goal: &str,
        step_index: u32,
        step: &MissionStep,
        total_steps: usize,
        completed_steps: &[MissionStep],
        workspace_path: Option<&str>,
        operator_hint: Option<&str>,
    ) -> String {
        let mut prompt = format!(
            "## Current Task: Step {}/{} — {}\n\n{}\n",
            step_index + 1,
            total_steps,
            step.title,
            step.description
        );

        prompt.push_str(&format!(
            "\n## Mission Goal (for reference)\n{}\n",
            mission_goal
        ));

        // Only keep last 3 steps to avoid context bloat
        if !completed_steps.is_empty() {
            prompt.push_str("\n## Previous Steps Summary\n");
            let recent: Vec<_> = completed_steps.iter().rev().take(3).collect();
            for cs in recent.into_iter().rev() {
                let full = cs.output_summary.as_deref().unwrap_or("(no summary)");
                let summary = if full.chars().count() > 300 {
                    let truncated: String = full.chars().take(297).collect();
                    format!("{}...", truncated)
                } else {
                    full.to_string()
                };
                prompt.push_str(&format!(
                    "- Step {}: {} → {}\n",
                    cs.index + 1,
                    cs.title,
                    summary
                ));
            }
            prompt.push('\n');
        }

        if let Some(hint) = operator_hint.map(str::trim).filter(|h| !h.is_empty()) {
            prompt.push_str("\n## Operator Guidance (Highest Priority)\n");
            prompt.push_str(hint);
            prompt.push('\n');
        }

        if !step.required_artifacts.is_empty() || !step.completion_checks.is_empty() {
            prompt.push_str("\n## Completion Contract\n");
            if !step.required_artifacts.is_empty() {
                prompt.push_str("- Required artifacts (relative workspace paths):\n");
                for artifact in &step.required_artifacts {
                    prompt.push_str(&format!("  - {}\n", artifact));
                }
            }
            if !step.completion_checks.is_empty() {
                prompt.push_str("- Completion checks (must pass):\n");
                for check in &step.completion_checks {
                    prompt.push_str(&format!("  - {}\n", check));
                }
            }
            prompt.push_str(
                "- If any contract item cannot be satisfied, explain the blocker and propose a concrete fix.\n",
            );
            prompt.push_str(
                "- Do not create placeholder scripts/files only to bypass checks; produce the real required artifact.\n",
            );
            prompt.push_str(
                "- If a required artifact is a binary deliverable (pptx/pdf/xlsx/etc.) and needs document-store archival, use `create_document_from_file` with the real artifact path.\n",
            );
        }

        if step.use_subagent {
            prompt.push_str("\n## Delegation Strategy (Subagent Preferred)\n");
            prompt
                .push_str("- This step is suitable for delegated execution via `subagent` tool.\n");
            prompt.push_str(
                "- Split work into focused subtasks, delegate, then synthesize final result.\n",
            );
            prompt.push_str(
                "- If subagent is unavailable, continue directly and clearly explain what fallback path you used.\n",
            );
        }

        prompt.push_str("\n## Mandatory Preflight Gate (Must Run First)\n");
        prompt.push_str(&format!(
            "- Before any other tool call, you MUST call `{}`.\n",
            MISSION_PREFLIGHT_TOOL_NAME
        ));
        prompt.push_str(
            "- If preflight is skipped or fails, this step will be marked as failed and retried.\n",
        );
        if step.required_artifacts.is_empty() && step.completion_checks.is_empty() {
            prompt.push_str("- This step has no preset contract. In preflight, you MUST provide at least one of:\n");
            prompt.push_str("  - `required_artifacts` (preferred for file outputs)\n");
            prompt.push_str("  - `completion_checks`\n");
            prompt.push_str("  - `no_artifact_reason` (only for non-file outcomes)\n");
        } else {
            prompt.push_str(
                "- The following contract is a planning baseline. Refine it in preflight to match real execution.\n",
            );
        }
        prompt.push_str("- Use these arguments in the preflight call:\n");
        let preflight_step_title = Self::escape_json_for_prompt(&step.title);
        let preflight_step_goal = Self::escape_json_for_prompt(&step.description);
        let preflight_workspace = workspace_path.unwrap_or("");
        let preflight_workspace = Self::escape_json_for_prompt(preflight_workspace);
        let required_artifacts_json =
            serde_json::to_string(&step.required_artifacts).unwrap_or_else(|_| "[]".to_string());
        let completion_checks_json =
            serde_json::to_string(&step.completion_checks).unwrap_or_else(|_| "[]".to_string());
        prompt.push_str("```json\n");
        prompt.push_str("{\n");
        prompt.push_str(&format!(
            "  \"step_title\": \"{}\",\n",
            preflight_step_title
        ));
        prompt.push_str(&format!("  \"step_goal\": \"{}\",\n", preflight_step_goal));
        prompt.push_str(&format!(
            "  \"workspace_path\": \"{}\",\n",
            preflight_workspace
        ));
        prompt.push_str(&format!(
            "  \"required_artifacts\": {},\n",
            required_artifacts_json
        ));
        prompt.push_str(&format!(
            "  \"completion_checks\": {},\n",
            completion_checks_json
        ));
        prompt.push_str("  \"no_artifact_reason\": \"\",\n");
        prompt.push_str("  \"attempt\": 1,\n");
        prompt.push_str("  \"last_error\": \"\"\n");
        prompt.push_str("}\n");
        prompt.push_str("```\n");
        prompt.push_str(
            "- For retries, call preflight again and increase `attempt`; include the latest failure in `last_error`.\n",
        );
        prompt.push_str(
            "- Optional but recommended: call `mission_preflight__workspace_overview` to inspect current workspace before execution.\n",
        );
        prompt.push_str(
            "- Before final completion response, call `mission_preflight__verify_contract` with your final contract to self-verify outputs.\n",
        );
        if runtime::contract_verify_gate_mode() == runtime::ContractVerifyGateMode::Hard {
            prompt.push_str(
                "- HARD GATE ENABLED: calling `mission_preflight__verify_contract` and getting `status=pass` is mandatory before completion.\n",
            );
        }

        prompt.push_str("## Instructions\n");
        prompt.push_str("- Complete this step as described above\n");
        prompt.push_str("- Verify your work matches the expected outcome in the description\n");
        prompt.push_str(
            "- If this step produces files, write the real deliverables under `output/` and report exact relative paths\n",
        );
        prompt.push_str(
            "- For binary deliverables that should be archived, use `create_document_from_file` with the real artifact path\n",
        );
        prompt.push_str("- Do not claim completion without verifiable outputs\n");
        prompt.push_str("- Be concise — your response will be saved as step summary");
        prompt
    }

    /// Extract the full output text from the last assistant message in the session.
    /// Saved as-is to output_summary for debugging; truncated only when injected into prompts.
    async fn extract_step_summary(&self, session_id: &str) -> Option<String> {
        let session = self.agent_service.get_session(session_id).await.ok()??;
        runtime::extract_last_assistant_text(&session.messages_json).filter(|t| !t.is_empty())
    }

    async fn register_step_artifacts(
        &self,
        mission_id: &str,
        step_index: u32,
        required_artifacts: &[String],
        workspace_path: &str,
        before: Option<&runtime::WorkspaceSnapshot>,
    ) -> Result<()> {
        runtime::save_scanned_artifacts(
            &self.agent_service,
            mission_id,
            step_index,
            workspace_path,
            before,
            Some(required_artifacts),
        )
        .await?;

        runtime::save_required_artifacts(
            &self.agent_service,
            mission_id,
            step_index,
            workspace_path,
            required_artifacts,
        )
        .await
    }

    fn normalize_workspace_relative_path(path: &str) -> Option<String> {
        let replaced = path.trim().replace('\\', "/");
        let trimmed = replaced.trim_start_matches('/').trim();
        if trimmed.is_empty() {
            return None;
        }
        let pb = Path::new(trimmed);
        if pb
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return None;
        }
        Some(trimmed.to_string())
    }

    fn choose_recovery_target_path(
        workspace_path: &str,
        source_path: &Path,
        required_artifacts: &[String],
    ) -> Option<PathBuf> {
        let source_name = source_path.file_name()?.to_string_lossy().to_string();

        for required in required_artifacts {
            let Some(required_rel) = Self::normalize_workspace_relative_path(required) else {
                continue;
            };
            let required_pb = Path::new(&required_rel);
            let required_name = required_pb.file_name()?.to_string_lossy().to_string();
            if required_name.eq_ignore_ascii_case(&source_name) {
                return Some(Path::new(workspace_path).join(required_rel));
            }
        }

        Some(
            Path::new(workspace_path)
                .join("output")
                .join("recovered")
                .join(source_name),
        )
    }

    fn ensure_unique_target_path(target: PathBuf) -> PathBuf {
        if !target.exists() {
            return target;
        }
        let parent = target.parent().map(Path::to_path_buf).unwrap_or_default();
        let stem = target
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "artifact".to_string());
        let ext = target.extension().map(|e| e.to_string_lossy().to_string());

        for idx in 1..=256 {
            let candidate_name = match &ext {
                Some(ext) if !ext.is_empty() => format!("{stem}-{idx}.{ext}"),
                _ => format!("{stem}-{idx}"),
            };
            let candidate = parent.join(candidate_name);
            if !candidate.exists() {
                return candidate;
            }
        }

        target
    }

    async fn recover_external_outputs_to_workspace(
        &self,
        mission_id: &str,
        step_index: u32,
        workspace_path: &str,
        external_paths: &[String],
        required_artifacts: &[String],
    ) -> Vec<String> {
        let mut unresolved = Vec::new();
        let mut recovered = Vec::<RecoveredExternalOutput>::new();

        for external in external_paths {
            let source = PathBuf::from(external);
            let metadata = match tokio::fs::metadata(&source).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(
                        "Mission {} step {} external output path not accessible: {} ({})",
                        mission_id,
                        step_index,
                        external,
                        e
                    );
                    unresolved.push(external.clone());
                    continue;
                }
            };
            if !metadata.is_file() {
                unresolved.push(external.clone());
                continue;
            }

            let Some(candidate_target) =
                Self::choose_recovery_target_path(workspace_path, &source, required_artifacts)
            else {
                unresolved.push(external.clone());
                continue;
            };
            let target = Self::ensure_unique_target_path(candidate_target);
            if let Some(parent) = target.parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    tracing::warn!(
                        "Mission {} step {} failed to create recovery directory for {}: {}",
                        mission_id,
                        step_index,
                        external,
                        e
                    );
                    unresolved.push(external.clone());
                    continue;
                }
            }

            if let Err(e) = tokio::fs::copy(&source, &target).await {
                tracing::warn!(
                    "Mission {} step {} failed to recover external output {} -> {}: {}",
                    mission_id,
                    step_index,
                    external,
                    target.to_string_lossy(),
                    e
                );
                unresolved.push(external.clone());
                continue;
            }

            let recovered_relative_path = target
                .strip_prefix(workspace_path)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| target.to_string_lossy().replace('\\', "/"));
            recovered.push(RecoveredExternalOutput {
                source_path: external.clone(),
                recovered_relative_path,
            });
        }

        for item in &recovered {
            self.mission_manager
                .broadcast(
                    mission_id,
                    StreamEvent::Status {
                        status: format!(
                            r#"{{"type":"step_guard_recovered","step_index":{},"guard":"external_workspace_path","from":"{}","to":"{}"}}"#,
                            step_index,
                            item.source_path.replace('"', r#"\""#).replace('\n', " "),
                            item.recovered_relative_path
                                .replace('"', r#"\""#)
                                .replace('\n', " ")
                        ),
                    },
                )
                .await;
        }

        if !recovered.is_empty() {
            tracing::info!(
                "Mission {} step {} recovered {} external outputs into workspace",
                mission_id,
                step_index,
                recovered.len()
            );
        }

        unresolved
    }

    fn truncate_chars(text: &str, max_chars: usize) -> String {
        if text.chars().count() <= max_chars {
            return text.to_string();
        }
        let mut s: String = text.chars().take(max_chars.saturating_sub(3)).collect();
        s.push_str("...");
        s
    }

    async fn synthesize_mission_summary(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<()> {
        let mission = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Mission not found"))?;

        let mut step_summaries = String::new();
        for step in &mission.steps {
            let status = match step.status {
                StepStatus::Completed => "completed",
                StepStatus::Failed => "failed",
                StepStatus::Skipped => "skipped",
                StepStatus::Running => "running",
                StepStatus::Pending => "pending",
                StepStatus::AwaitingApproval => "awaiting_approval",
            };
            let summary = step
                .output_summary
                .as_deref()
                .map(|s| Self::truncate_chars(s, 300))
                .unwrap_or_else(|| "(no summary)".to_string());
            step_summaries.push_str(&format!(
                "- Step {}: {} [{}] -> {}\n",
                step.index + 1,
                step.title,
                status,
                summary
            ));
        }

        if step_summaries.trim().is_empty() {
            return Ok(());
        }

        let prompt = Self::render_mission_summary_prompt(&step_summaries);

        // Best-effort: summary failure must not change mission completion.
        if let Err(e) = self
            .execute_via_bridge(
                agent_id,
                session_id,
                mission_id,
                &prompt,
                cancel_token,
                workspace_path,
                None,
            )
            .await
        {
            tracing::warn!("Mission {} summary bridge failed: {}", mission_id, e);
            return Ok(());
        }

        if let Some(summary) = self.extract_step_summary(session_id).await {
            if let Err(e) = self
                .agent_service
                .set_mission_final_summary(mission_id, &summary)
                .await
            {
                tracing::warn!("Failed to save mission {} final summary: {}", mission_id, e);
            }
        }

        Ok(())
    }

    fn render_mission_summary_prompt(step_summaries: &str) -> String {
        let ctx = MissionSummaryTemplateContext { step_summaries };
        match prompt_template::render_global_file("mission_final_summary.md", &ctx) {
            Ok(rendered) => rendered,
            Err(e) => {
                tracing::warn!("Failed to render mission_final_summary.md template: {}", e);
                format!(
                    "All steps have been completed. Please synthesize the final results.\n\n\
                     ## Step Execution Results\n\
                     {}\n\
                     Provide a concise final summary including key achievements and issues.",
                    step_summaries
                )
            }
        }
    }

    /// Build the prompt for re-plan evaluation after a checkpoint step.
    fn build_replan_prompt(
        completed_steps: &[MissionStep],
        remaining_steps: &[MissionStep],
    ) -> String {
        let mut completed = String::new();
        for cs in completed_steps {
            let full = cs.output_summary.as_deref().unwrap_or("(no summary)");
            let summary = if full.chars().count() > 500 {
                let truncated: String = full.chars().take(497).collect();
                format!("{}...", truncated)
            } else {
                full.to_string()
            };
            completed.push_str(&format!(
                "- Step {}: {} → {}\n",
                cs.index + 1,
                cs.title,
                summary
            ));
        }

        let mut remaining = String::new();
        for rs in remaining_steps {
            remaining.push_str(&format!(
                "- Step {}: {} — {}\n",
                rs.index + 1,
                rs.title,
                rs.description
            ));
        }

        let completed = if completed.trim().is_empty() {
            "- (none)\n".to_string()
        } else {
            completed
        };
        let remaining = if remaining.trim().is_empty() {
            "- (none)\n".to_string()
        } else {
            remaining
        };

        let ctx = MissionReplanTemplateContext {
            completed_steps: &completed,
            remaining_steps: &remaining,
        };
        match prompt_template::render_global_file("mission_replan.md", &ctx) {
            Ok(rendered) => rendered,
            Err(e) => {
                tracing::warn!("Failed to render mission_replan.md template: {}", e);
                format!(
                    "## Re-plan Evaluation\n\n\
                     ### Completed Steps\n\
                     {}\n\
                     ### Current Remaining Plan\n\
                     {}\n\
                     Respond with JSON:\n\
                     - keep: {{\"decision\":\"keep\"}}\n\
                     - replan: {{\"decision\":\"replan\",\"steps\":[...]}}",
                    completed, remaining
                )
            }
        }
    }

    /// Parse the Agent's re-plan response into new MissionStep entries.
    /// `start_index` is the index offset for the new steps (= number of completed steps).
    fn parse_replan_response(
        &self,
        response: &str,
        start_index: usize,
        mission_step_max_retries: Option<u32>,
        mission_step_timeout_seconds: Option<u64>,
    ) -> Result<Vec<MissionStep>> {
        let json_str = runtime::extract_json_block(response);
        let steps = Self::parse_steps_json(
            &json_str,
            start_index,
            mission_step_max_retries,
            mission_step_timeout_seconds,
        )?;
        if steps.is_empty() {
            return Err(anyhow!("Re-plan produced empty step list"));
        }
        Ok(steps)
    }

    /// P1: Evaluate whether remaining steps need re-planning after a checkpoint.
    ///
    /// Sends a structured prompt to the Agent with completed step summaries
    /// and the current remaining plan. The Agent decides:
    /// - "keep" → no change, continue with existing plan
    /// - JSON array → replacement steps for the remaining work
    ///
    /// Returns `Ok(None)` if no re-plan needed, `Ok(Some(steps))` if re-planned.
    #[allow(clippy::too_many_arguments)]
    async fn evaluate_replan(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        completed_steps: &[MissionStep],
        remaining_steps: &[MissionStep],
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<Option<Vec<MissionStep>>> {
        // Build the evaluation prompt
        let prompt = Self::build_replan_prompt(completed_steps, remaining_steps);
        let mission_defaults = self
            .agent_service
            .get_mission(mission_id)
            .await
            .ok()
            .flatten();
        let mission_step_max_retries = mission_defaults.as_ref().and_then(|m| m.step_max_retries);
        let mission_step_timeout_seconds = mission_defaults
            .as_ref()
            .and_then(|m| m.step_timeout_seconds);

        // Execute via bridge
        self.execute_via_bridge(
            agent_id,
            session_id,
            mission_id,
            &prompt,
            cancel_token,
            workspace_path,
            None,
        )
        .await?;

        // Parse response
        let session = self
            .agent_service
            .get_session(session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Session not found"))?;

        let response =
            runtime::extract_last_assistant_text(&session.messages_json).unwrap_or_default();

        // Try structured JSON parsing first
        let json_str = runtime::extract_json_block(&response);
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
            if val.get("decision").and_then(|d| d.as_str()) == Some("keep") {
                tracing::info!(
                    "Mission {} replan evaluation: keep current plan",
                    mission_id
                );
                return Ok(None);
            }
            // Extract steps from structured response
            if let Some(steps_val) = val.get("steps") {
                let steps_str = steps_val.to_string();
                match Self::parse_steps_json(
                    &steps_str,
                    completed_steps.len(),
                    mission_step_max_retries,
                    mission_step_timeout_seconds,
                ) {
                    Ok(new_steps) if !new_steps.is_empty() => {
                        tracing::info!(
                            "Mission {} re-planned: {} remaining steps replaced with {}",
                            mission_id,
                            remaining_steps.len(),
                            new_steps.len()
                        );
                        self.mission_manager
                            .broadcast(
                                mission_id,
                                StreamEvent::Status {
                                    status: format!(
                                        r#"{{"type":"mission_replanned","new_step_count":{}}}"#,
                                        new_steps.len()
                                    ),
                                },
                            )
                            .await;
                        return Ok(Some(new_steps));
                    }
                    _ => {}
                }
            }
        }

        // Fallback: check for "keep" text
        let trimmed = response.trim().to_lowercase();
        if trimmed == "keep" || trimmed.starts_with("keep") {
            tracing::info!(
                "Mission {} replan evaluation: keep current plan (text fallback)",
                mission_id
            );
            return Ok(None);
        }

        // Fallback: try to parse as raw steps array
        match self.parse_replan_response(
            &response,
            completed_steps.len(),
            mission_step_max_retries,
            mission_step_timeout_seconds,
        ) {
            Ok(new_steps) => {
                tracing::info!(
                    "Mission {} re-planned: {} remaining steps replaced with {}",
                    mission_id,
                    remaining_steps.len(),
                    new_steps.len()
                );
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: format!(
                                r#"{{"type":"mission_replanned","new_step_count":{}}}"#,
                                new_steps.len()
                            ),
                        },
                    )
                    .await;
                Ok(Some(new_steps))
            }
            Err(e) => {
                // Parse failure is non-fatal; keep current plan
                tracing::warn!(
                    "Mission {} replan parse failed, keeping current plan: {}",
                    mission_id,
                    e
                );
                Ok(None)
            }
        }
    }

    /// Resume a paused mission from the next pending step.
    pub async fn resume_mission(
        &self,
        mission_id: &str,
        cancel_token: CancellationToken,
        resume_feedback: Option<String>,
    ) -> Result<()> {
        let mission = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Mission not found"))?;

        // Dispatch to AdaptiveExecutor for adaptive mode
        // (resume_adaptive has its own cleanup wrapper)
        if mission.execution_mode == ExecutionMode::Adaptive {
            let adaptive = AdaptiveExecutor::new(
                self.db.clone(),
                self.mission_manager.clone(),
                self.workspace_root.clone(),
            );
            return adaptive
                .resume_adaptive(mission_id, cancel_token, resume_feedback)
                .await;
        }

        // Sequential resume with guaranteed cleanup
        let exec_result = self
            .resume_mission_sequential(mission_id, &mission, cancel_token, resume_feedback)
            .await;

        // Guaranteed cleanup: broadcast Done + complete in manager
        match &exec_result {
            Ok(_) => {
                let done_status = self
                    .agent_service
                    .get_mission(mission_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|m| match m.status {
                        MissionStatus::Paused => "paused",
                        MissionStatus::Completed => "completed",
                        MissionStatus::Cancelled => "cancelled",
                        _ => "completed",
                    })
                    .unwrap_or("completed");

                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Done {
                            status: done_status.to_string(),
                            error: None,
                        },
                    )
                    .await;
            }
            Err(e) => {
                let mut done_status = "failed";
                let mut done_error = Some(e.to_string());
                let mut should_persist_failure = true;

                if let Ok(Some(current)) = self.agent_service.get_mission(mission_id).await {
                    match current.status {
                        MissionStatus::Paused => {
                            done_status = "paused";
                            done_error = None;
                            should_persist_failure = false;
                        }
                        MissionStatus::Cancelled => {
                            done_status = "cancelled";
                            done_error = None;
                            should_persist_failure = false;
                        }
                        _ => {}
                    }
                }

                if should_persist_failure {
                    self.persist_failure_state(mission_id, &e.to_string()).await;
                }
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Done {
                            status: done_status.to_string(),
                            error: done_error,
                        },
                    )
                    .await;
            }
        }

        self.mission_manager.complete(mission_id).await;
        exec_result
    }

    async fn persist_failure_state(&self, mission_id: &str, error_message: &str) {
        if let Err(e) = self
            .agent_service
            .update_mission_status(mission_id, &MissionStatus::Failed)
            .await
        {
            tracing::warn!(
                "Failed to mark mission {} as failed during cleanup: {}",
                mission_id,
                e
            );
        }

        if let Err(e) = self
            .agent_service
            .set_mission_error(mission_id, error_message)
            .await
        {
            tracing::warn!(
                "Failed to persist mission {} error message during cleanup: {}",
                mission_id,
                e
            );
        }
    }

    /// Inner sequential resume logic (separated for cleanup wrapper).
    async fn resume_mission_sequential(
        &self,
        mission_id: &str,
        mission: &MissionDoc,
        cancel_token: CancellationToken,
        resume_feedback: Option<String>,
    ) -> Result<()> {
        if !matches!(
            mission.status,
            MissionStatus::Paused | MissionStatus::Failed
        ) {
            return Err(anyhow!("Mission is not paused/failed"));
        }

        let session_id = mission
            .session_id
            .as_deref()
            .ok_or_else(|| anyhow!("Mission has no session"))?
            .to_string();
        let runtime_cfg = Self::resolve_execution_runtime(mission);

        // Read workspace_path from mission doc (set during initial execution)
        let workspace_path = mission.workspace_path.clone();
        let mut working_steps = mission.steps.clone();

        // Failed mission can be manually resumed:
        // reset failed/running steps to pending and clear mission-level error state.
        // Paused mission may also contain stale running steps after abnormal interruption.
        if mission.status == MissionStatus::Failed {
            if let Err(e) = self.agent_service.clear_mission_error(mission_id).await {
                tracing::warn!(
                    "Failed to clear mission {} error before resume: {}",
                    mission_id,
                    e
                );
            }
        }
        for step in &mut working_steps {
            let should_reset = if mission.status == MissionStatus::Failed {
                matches!(step.status, StepStatus::Failed | StepStatus::Running)
            } else {
                // Mission paused: clean up stale running step left by interrupted cancel/pause flow.
                matches!(step.status, StepStatus::Running)
            };
            if !should_reset {
                continue;
            }

            if let Err(e) = self
                .agent_service
                .reset_step_for_retry(mission_id, step.index)
                .await
            {
                tracing::warn!(
                    "Failed to reset mission {} step {} for retry: {}",
                    mission_id,
                    step.index,
                    e
                );
            }
            step.status = StepStatus::Pending;
            step.error_message = None;
            step.started_at = None;
            step.completed_at = None;
            step.output_summary = None;
            step.tool_calls.clear();
        }

        // Mission paused during planning may have no steps yet.
        // Restart from planning path instead of failing resume.
        if working_steps.is_empty() {
            self.agent_service
                .update_mission_status(mission_id, &MissionStatus::Planned)
                .await
                .map_err(|e| {
                    anyhow!(
                        "Failed to set mission {} planned for resume-from-planning: {}",
                        mission_id,
                        e
                    )
                })?;
            return self.execute_mission_inner(mission_id, cancel_token).await;
        }

        // Collect completed steps for context injection on resume
        let prior_completed: Vec<MissionStep> = working_steps
            .iter()
            .filter(|s| s.status == StepStatus::Completed)
            .cloned()
            .collect();

        // Find remaining steps starting from current
        let remaining: Vec<MissionStep> = working_steps
            .iter()
            .filter(|s| s.status == StepStatus::Pending || s.status == StepStatus::AwaitingApproval)
            .cloned()
            .collect();

        if remaining.is_empty() {
            // No remaining work to execute. Transition to Completed.
            // When resuming from Failed/Paused, strict transition rules may reject
            // direct -> Completed. In that case, hop through Running.
            if let Err(first_err) = self
                .agent_service
                .update_mission_status(mission_id, &MissionStatus::Completed)
                .await
            {
                tracing::warn!(
                    "Direct complete transition rejected for mission {} (resume/no-pending): {}. Retrying via Running.",
                    mission_id,
                    first_err
                );
                self.agent_service
                    .update_mission_status(mission_id, &MissionStatus::Running)
                    .await
                    .map_err(|e| {
                        anyhow!(
                            "Failed to set mission {} running before completion fallback: {}",
                            mission_id,
                            e
                        )
                    })?;
                self.agent_service
                    .update_mission_status(mission_id, &MissionStatus::Completed)
                    .await
                    .map_err(|e| {
                        anyhow!(
                            "Failed to set mission {} completed when no pending steps on resume: {}",
                            mission_id,
                            e
                        )
                    })?;
            }
            return Ok(());
        }

        // Update status to Running only when there is work to continue.
        self.agent_service
            .update_mission_status(mission_id, &MissionStatus::Running)
            .await
            .map_err(|e| {
                anyhow!(
                    "Failed to set mission {} running on resume: {}",
                    mission_id,
                    e
                )
            })?;

        let mut resumed_mission = mission.clone();
        resumed_mission.steps = working_steps;

        self.execute_steps(
            mission_id,
            &resumed_mission,
            &session_id,
            remaining,
            prior_completed,
            cancel_token,
            workspace_path.as_deref(),
            resume_feedback.as_deref(),
            runtime_cfg,
        )
        .await
    }
}
