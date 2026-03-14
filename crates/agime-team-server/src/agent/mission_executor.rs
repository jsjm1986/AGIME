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

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use super::adaptive_executor::AdaptiveExecutor;
use super::mission_manager::MissionManager;
use super::mission_mongo::{
    resolve_execution_profile, ApprovalPolicy, ExecutionMode, ExecutionProfile, MissionDoc,
    MissionStatus, MissionStep, RuntimeContract, RuntimeContractVerification, StepEvidenceBundle,
    StepProgressEvent, StepProgressEventKind, StepProgressEventSource, StepProgressLayer,
    StepStatus, StepSupervisorState, ToolCallRecord,
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
/// Timeout multiplier applied to retries after a timeout failure.
const DEFAULT_STEP_TIMEOUT_RETRY_MULTIPLIER: u64 = 2;
const MAX_STEP_TIMEOUT_RETRY_MULTIPLIER: u64 = 4;
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
const MAX_STEP_COMPLETION_CHECK_CMD_LEN: usize = 1200;
const MAX_STEP_PROGRESS_EVENTS: usize = 24;
const DEFAULT_COMPLETION_CHECK_TIMEOUT_SECS: u64 = 45;
const MAX_COMPLETION_CHECK_TIMEOUT_SECS: u64 = 600;
const MISSION_PREFLIGHT_TOOL_NAME: &str = "mission_preflight__preflight";
const MISSION_VERIFY_CONTRACT_TOOL_NAME: &str = "mission_preflight__verify_contract";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepFailureKind {
    PreflightMissing,
    ContractValidation,
    ContractVerifyGate,
    WorkspaceGuard,
    MissingParentDirectory,
    MissingSummary,
    RepeatedToolDenied,
    SecurityToolBlocked,
    MaxTurnLimit,
    Timeout,
    ToolParameterSchema,
    ToolExecution,
    Unknown,
}

#[derive(Debug, Clone)]
struct StepSupervisorDecision {
    state: StepSupervisorState,
    blocker: Option<String>,
    should_generate_hint: bool,
}

impl StepFailureKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::PreflightMissing => "preflight_missing",
            Self::ContractValidation => "contract_validation",
            Self::ContractVerifyGate => "contract_verify_gate",
            Self::WorkspaceGuard => "workspace_guard",
            Self::MissingParentDirectory => "missing_parent_directory",
            Self::MissingSummary => "missing_summary",
            Self::RepeatedToolDenied => "repeated_tool_denied",
            Self::SecurityToolBlocked => "security_tool_blocked",
            Self::MaxTurnLimit => "max_turn_limit",
            Self::Timeout => "timeout",
            Self::ToolParameterSchema => "tool_parameter_schema",
            Self::ToolExecution => "tool_execution",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Default)]
struct StepProgressSnapshot {
    message_delta: usize,
    token_delta: i32,
    tool_call_count: usize,
    artifact_count: usize,
    required_artifact_hits: usize,
    has_output_summary: bool,
    artifact_paths: Vec<String>,
    required_artifact_paths: Vec<String>,
    planning_evidence_paths: Vec<String>,
    quality_evidence_paths: Vec<String>,
    runtime_evidence_paths: Vec<String>,
    deployment_evidence_paths: Vec<String>,
    review_evidence_paths: Vec<String>,
    risk_evidence_paths: Vec<String>,
}

impl StepProgressSnapshot {
    fn has_activity(&self) -> bool {
        self.message_delta > 0 || self.token_delta > 0 || self.tool_call_count > 0
    }

    fn has_progress(&self) -> bool {
        self.has_delivery_progress() || self.has_work_progress()
    }

    fn has_delivery_progress(&self) -> bool {
        self.artifact_count > 0
            || self.required_artifact_hits > 0
            || !self.quality_evidence_paths.is_empty()
            || !self.runtime_evidence_paths.is_empty()
            || !self.deployment_evidence_paths.is_empty()
            || !self.review_evidence_paths.is_empty()
    }

    fn has_work_progress(&self) -> bool {
        if self.has_delivery_progress() || self.has_output_summary {
            return true;
        }

        if !self.planning_evidence_paths.is_empty() || !self.risk_evidence_paths.is_empty() {
            return true;
        }

        // Research, planning, and preparation steps may legitimately spend time
        // reading, comparing, and structuring information before a deliverable
        // file appears. Treat sustained tool-backed work as progress, but keep
        // the threshold high enough to avoid classifying light chatter as busy.
        let sustained_tool_work =
            self.tool_call_count > 0 && (self.token_delta >= 64 || self.message_delta >= 2);
        let sustained_reasoning = self.token_delta >= 256 && self.message_delta >= 2;

        sustained_tool_work || sustained_reasoning
    }

    fn progress_score(&self) -> i32 {
        let mut score = 0;
        if self.has_activity() {
            score += 1;
        }
        if self.message_delta > 0 {
            score += 1;
        }
        if self.token_delta > 0 {
            score += 1;
        }
        score += (self.tool_call_count.min(3)) as i32;
        score += (self.artifact_count.min(3) * 2) as i32;
        score += (self.required_artifact_hits.min(2) * 2) as i32;
        score += (self.planning_evidence_paths.len().min(2)) as i32;
        score += (self.quality_evidence_paths.len().min(2) * 2) as i32;
        score += (self.runtime_evidence_paths.len().min(2) * 2) as i32;
        score += (self.deployment_evidence_paths.len().min(2) * 2) as i32;
        score += (self.review_evidence_paths.len().min(1) * 2) as i32;
        score += (self.risk_evidence_paths.len().min(1)) as i32;
        if self.has_output_summary {
            score += 2;
        }
        score
    }

    fn summary(&self) -> String {
        format!(
            "messages_delta={}, tokens_delta={}, tool_calls={}, changed_artifacts={}, required_artifacts_hit={}, planning_evidence={}, risk_evidence={}, has_output_summary={}",
            self.message_delta,
            self.token_delta,
            self.tool_call_count,
            self.artifact_count,
            self.required_artifact_hits,
            self.planning_evidence_paths.len(),
            self.risk_evidence_paths.len(),
            self.has_output_summary
        )
    }
}

#[derive(Debug, Clone)]
struct SupervisorGuidance {
    diagnosis: String,
    resume_hint: String,
    semantic_tags: Vec<String>,
    observed_evidence: Vec<String>,
}

struct SilentEventBroadcaster;

impl runtime::EventBroadcaster for SilentEventBroadcaster {
    fn broadcast(
        &self,
        _context_id: &str,
        _event: StreamEvent,
    ) -> impl std::future::Future<Output = ()> + Send {
        std::future::ready(())
    }
}

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
                idx,
                &step_clone,
                total,
                &completed_steps,
                cancel_token.clone(),
                workspace_path,
                mission,
                policy_str,
                runtime_cfg
                    .mission_step_timeout_seconds
                    .or(mission.step_timeout_seconds),
                runtime_cfg
                    .mission_step_max_retries
                    .or(mission.step_max_retries),
                runtime_cfg.session_max_turns,
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
        step_index: u32,
        step: &MissionStep,
        total_steps: usize,
        completed_steps: &[MissionStep],
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
        mission: &MissionDoc,
        approval_policy: &str,
        mission_step_timeout_seconds: Option<u64>,
        mission_step_max_retries: Option<u32>,
        session_max_turns: Option<i32>,
        operator_hint: Option<&str>,
    ) -> Result<()> {
        let mut step_runtime = step.clone();
        let step_session_id = self
            .create_isolated_step_session(mission, agent_id, mission_id, session_max_turns)
            .await?;
        let session_id = step_session_id.as_str();
        self.agent_service
            .set_mission_session(mission_id, session_id)
            .await
            .map_err(|e| anyhow!("Failed to bind current step session: {}", e))?;
        let run_result: Result<()> = async {
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
        self.update_step_supervision(
            mission_id,
            &mut step_runtime,
            step_index,
            StepSupervisorState::Busy,
            &StepProgressSnapshot {
                message_delta: 1,
                token_delta: 0,
                tool_call_count: 0,
                artifact_count: 0,
                required_artifact_hits: 0,
                has_output_summary: false,
                artifact_paths: Vec::new(),
                required_artifact_paths: Vec::new(),
                planning_evidence_paths: Vec::new(),
                quality_evidence_paths: Vec::new(),
                runtime_evidence_paths: Vec::new(),
                deployment_evidence_paths: Vec::new(),
                review_evidence_paths: Vec::new(),
                risk_evidence_paths: Vec::new(),
            },
            None,
            None,
        )
        .await;
        step_runtime.recent_progress_events = Self::append_progress_events(
            &step_runtime.recent_progress_events,
            vec![StepProgressEvent {
                kind: StepProgressEventKind::StepStarted,
                message: format!("step {} started: {}", step_index + 1, step_runtime.title),
                source: Some(StepProgressEventSource::Executor),
                layer: Some(StepProgressLayer::Activity),
                semantic_tags: Self::semantic_tags(&["step_started", "execution_started"]),
                ai_annotation: None,
                paths: Vec::new(),
                checks: Vec::new(),
                score_delta: Some(1),
                recorded_at: Some(mongodb::bson::DateTime::now()),
            }],
        );
        if let Err(err) = self
            .agent_service
            .set_step_observability(
                mission_id,
                step_index,
                &step_runtime.recent_progress_events,
                step_runtime.evidence_bundle.as_ref(),
            )
            .await
        {
            tracing::warn!(
                "Failed to persist step start observability for mission {} step {}: {}",
                mission_id,
                step_index,
                err
            );
        }

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

        let workspace_before = match workspace_path {
            Some(wp) => runtime::snapshot_workspace_files(wp).ok(),
            None => None,
        };

        // Build mission context for system prompt injection
        let mc_json = serde_json::json!({
            "goal": mission.goal,
            "approval_policy": approval_policy,
            "total_steps": total_steps,
            "current_step": step_index + 1,
        });

        // Execute with retry logic (P2)
        let max_retries =
            Self::resolve_effective_step_max_retries(step.max_retries, mission_step_max_retries);
        let step_timeout = Self::resolve_step_timeout(step, mission_step_timeout_seconds);
        let timeout_retry_limit = Self::step_timeout_retry_limit().min(max_retries);
        let timeout_cancel_grace = Self::step_timeout_cancel_grace();
        let mut timeout_retries_used: u32 = 0;
        let mut last_err: Option<anyhow::Error> = None;
        let mut previous_retry_failure_kind: Option<StepFailureKind> = None;
        let mut repeated_failure_streak: u32 = 0;
        let mut reusable_preflight_contract = step
            .runtime_contract
            .as_ref()
            .map(Self::runtime_contract_doc_to_preflight);
        let mut reusable_verify_contract_state =
            Self::persisted_verify_contract_state(step.contract_verification.as_ref());

        for attempt in 0..=max_retries {
            let current_attempt_number = Self::current_step_attempt_number(step, attempt);
            let attempt_step_timeout = Self::resolve_retry_attempt_timeout(
                step_timeout,
                Self::infer_prior_timeout_retry_level(step, step_timeout),
                timeout_retries_used,
            );
            let retry_failure_message = if attempt == 0 {
                None
            } else {
                Some(
                    last_err
                        .as_ref()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "unknown error".to_string()),
                )
            };
            let (recent_tool_calls, previous_output) = if attempt == 0 {
                (Vec::new(), None)
            } else {
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
                }
            };
            let retry_failure_kind = retry_failure_message
                .as_deref()
                .map(|msg| Self::classify_retry_failure(msg, &recent_tool_calls));
            if let Some(kind) = retry_failure_kind {
                if previous_retry_failure_kind == Some(kind) {
                    repeated_failure_streak = repeated_failure_streak.saturating_add(1);
                } else {
                    previous_retry_failure_kind = Some(kind);
                    repeated_failure_streak = 1;
                }
            } else {
                previous_retry_failure_kind = None;
                repeated_failure_streak = 0;
            }
            let repeated_failed_tool = Self::detect_repeated_failed_tool(&recent_tool_calls);
            let mut supervisor_guidance = None;
            let retry_progress = if attempt > 0 {
                self.collect_step_progress_snapshot(
                    session_id,
                    messages_before,
                    tokens_before,
                    workspace_path,
                    workspace_before.as_ref(),
                    &step.required_artifacts,
                )
                .await
            } else {
                StepProgressSnapshot::default()
            };
            if attempt > 0 {
                let supervisor_decision = Self::decide_supervisor_response(
                    step_runtime.supervisor_state.as_ref(),
                    step_runtime.stall_count,
                    retry_failure_kind,
                    &retry_progress,
                    repeated_failure_streak,
                    repeated_failed_tool.as_deref(),
                );
                if supervisor_decision.should_generate_hint {
                    supervisor_guidance = self
                        .maybe_generate_supervisor_guidance(
                            mission,
                            agent_id,
                            mission_id,
                            &step_runtime,
                            retry_failure_kind,
                            retry_failure_message.as_deref().unwrap_or("unknown failure"),
                            &retry_progress,
                            previous_output.as_deref(),
                            &recent_tool_calls,
                            repeated_failure_streak,
                            repeated_failed_tool.as_deref(),
                            workspace_path,
                        )
                        .await;
                }
                let blocker = supervisor_guidance
                    .as_ref()
                    .map(|guidance| guidance.diagnosis.as_str())
                    .or_else(|| supervisor_decision.blocker.as_deref())
                    .or_else(|| retry_failure_message.as_deref());
                self.update_step_supervision(
                    mission_id,
                    &mut step_runtime,
                    step_index,
                    supervisor_decision.state,
                    &retry_progress,
                    blocker,
                    supervisor_guidance.as_ref(),
                )
                .await;
            }
            let retry_turn_instruction = Self::compose_retry_turn_instruction(
                retry_failure_kind.and_then(|kind| {
                    Self::build_retry_turn_instruction(
                        kind,
                        &step_runtime,
                        &retry_progress,
                        repeated_failure_streak,
                        repeated_failed_tool.as_deref(),
                        workspace_path,
                    )
                }),
                supervisor_guidance
                    .as_ref()
                    .map(|guidance| guidance.resume_hint.as_str()),
            );
            // B4: On retry, use prompt-driven recovery playbook with bounded context.
            let base_prompt = Self::build_step_prompt(
                &mission.goal,
                step_index,
                &step_runtime,
                total_steps,
                completed_steps,
                workspace_path,
                operator_hint,
                current_attempt_number,
                retry_failure_message.as_deref(),
            );
            let prompt = if attempt == 0 {
                base_prompt
            } else {
                let prev_err = retry_failure_message
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| "unknown error".to_string());
                let playbook = runtime::render_retry_playbook(&runtime::RetryPlaybookContext {
                    mode_label: "step".to_string(),
                    unit_title: step.title.clone(),
                    attempt_number: current_attempt_number,
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
                                r#"{{"type":"step_retry","step_index":{},"attempt":{},"failure_kind":"{}"}}"#,
                                step_index,
                                attempt,
                                retry_failure_kind
                                    .unwrap_or(StepFailureKind::Unknown)
                                    .as_str()
                            ),
                        },
                    )
                    .await;
                step_runtime.retry_count = step_runtime.retry_count.saturating_add(1);
                step_runtime.recent_progress_events = Self::append_progress_events(
                    &step_runtime.recent_progress_events,
                    vec![StepProgressEvent {
                        kind: StepProgressEventKind::RetryScheduled,
                        message: format!(
                            "retry {} scheduled after {}",
                            attempt,
                            retry_failure_kind
                                .unwrap_or(StepFailureKind::Unknown)
                                .as_str()
                        ),
                        source: Some(StepProgressEventSource::Supervisor),
                        layer: Some(StepProgressLayer::Recovery),
                        semantic_tags: Self::semantic_tags(&["retry_scheduled", "recovery"]),
                        ai_annotation: None,
                        paths: Vec::new(),
                        checks: Vec::new(),
                        score_delta: None,
                        recorded_at: Some(mongodb::bson::DateTime::now()),
                    }],
                );
                if let Err(err) = self
                    .agent_service
                    .set_step_observability(
                        mission_id,
                        step_index,
                        &step_runtime.recent_progress_events,
                        step_runtime.evidence_bundle.as_ref(),
                    )
                    .await
                {
                    tracing::warn!(
                        "Failed to persist retry observability for mission {} step {}: {}",
                        mission_id,
                        step_index,
                        err
                    );
                }

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
                retry_turn_instruction.as_deref(),
            );
            tokio::pin!(exec_fut);

            let attempt_result =
                match tokio::time::timeout(attempt_step_timeout, &mut exec_fut).await {
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
                                attempt_step_timeout.as_secs(),
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
                        attempt_step_timeout.as_secs()
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
                        step_runtime.output_summary = Some(s.clone());
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
                            step_runtime.tool_calls = step_tool_calls.clone();
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

                    let reused_persisted_preflight =
                        preflight_contract.is_none() && reusable_preflight_contract.is_some();
                    preflight_contract = Self::resolve_retry_preflight_contract(
                        preflight_contract,
                        reusable_preflight_contract.as_ref(),
                        step,
                        retry_failure_message.as_deref(),
                        operator_hint,
                    );

                    let effective_contract = mission_verifier::resolve_effective_contract(
                        preflight_contract,
                        MISSION_PREFLIGHT_TOOL_NAME,
                        mission_verifier::VerifierLimits {
                            max_required_artifacts: MAX_STEP_REQUIRED_ARTIFACTS,
                            max_completion_checks: MAX_STEP_COMPLETION_CHECKS,
                            max_completion_check_cmd_len: MAX_STEP_COMPLETION_CHECK_CMD_LEN,
                        },
                    )?;
                    reusable_preflight_contract = Some(effective_contract.clone());

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
                    step_runtime.runtime_contract =
                        Some(Self::to_runtime_contract_doc(&effective_contract));
                    step_runtime.recent_progress_events = Self::append_progress_events(
                        &step_runtime.recent_progress_events,
                        vec![StepProgressEvent {
                            kind: StepProgressEventKind::RuntimeContractCaptured,
                            message: "runtime contract captured for step".to_string(),
                            source: Some(StepProgressEventSource::Verifier),
                            layer: Some(StepProgressLayer::WorkProgress),
                            semantic_tags: Self::semantic_tags(&["contract_captured", "preflight"]),
                            ai_annotation: None,
                            paths: effective_contract.required_artifacts.clone(),
                            checks: effective_contract.completion_checks.clone(),
                            score_delta: Some(2),
                            recorded_at: Some(mongodb::bson::DateTime::now()),
                        }],
                    );

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
                        reused_persisted_preflight,
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
                                        current_attempt_number,
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
                                current_attempt_number,
                                retry_err
                            );
                            last_err = Some(retry_err);
                            continue;
                        }

                        return self
                            .finalize_step_failure(
                                mission_id,
                                session_id,
                                &mut step_runtime,
                                step_index,
                                tokens_before,
                                retry_err,
                            )
                            .await;
                    }

                    let gate_mode = runtime::contract_verify_gate_mode();
                    let fresh_verify_tool_called = mission_verifier::has_verify_contract_tool_call(
                        &step_tool_calls,
                        MISSION_VERIFY_CONTRACT_TOOL_NAME,
                    );
                    let (verify_tool_called, verify_contract_status) =
                        Self::resolve_retry_verify_contract_state(
                            fresh_verify_tool_called,
                            verify_contract_status,
                            reusable_verify_contract_state,
                            step,
                            retry_failure_message.as_deref(),
                            operator_hint,
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
                    if gate_error.is_none() && verify_tool_called {
                        reusable_verify_contract_state =
                            Some((verify_tool_called, verify_contract_status));
                    }
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
                    step_runtime.contract_verification = Some(RuntimeContractVerification {
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
                    });
                    step_runtime.recent_progress_events = Self::append_progress_events(
                        &step_runtime.recent_progress_events,
                        vec![StepProgressEvent {
                            kind: StepProgressEventKind::ContractVerified,
                            message: format!(
                                "contract verification {} via {}",
                                verify_status_label,
                                runtime::contract_verify_gate_mode_label(gate_mode)
                            ),
                            source: Some(StepProgressEventSource::Verifier),
                            layer: Some(StepProgressLayer::DeliveryProgress),
                            semantic_tags: Self::semantic_tags(&["contract_verified", "verification"]),
                            ai_annotation: None,
                            paths: Vec::new(),
                            checks: effective_contract.completion_checks.clone(),
                            score_delta: gate_error.is_none().then_some(2),
                            recorded_at: Some(mongodb::bson::DateTime::now()),
                        }],
                    );
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
                                current_attempt_number,
                                retry_err
                            );
                            last_err = Some(retry_err);
                            continue;
                        }
                        return self
                            .finalize_step_failure(
                                mission_id,
                                session_id,
                                &mut step_runtime,
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
                                        current_attempt_number,
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
                                current_attempt_number,
                                retry_err
                            );
                            last_err = Some(retry_err);
                            continue;
                        }
                        return self
                            .finalize_step_failure(
                                mission_id,
                                session_id,
                                &mut step_runtime,
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
                                        current_attempt_number,
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
                                current_attempt_number,
                                retry_err
                            );
                            last_err = Some(retry_err);
                            continue;
                        }
                        return self
                            .finalize_step_failure(
                                mission_id,
                                session_id,
                                &mut step_runtime,
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

                    let success_progress = self
                        .collect_step_progress_snapshot(
                            session_id,
                            messages_before,
                            tokens_before,
                            workspace_path,
                            workspace_before.as_ref(),
                            &effective_contract.required_artifacts,
                        )
                        .await;
                    self.update_step_supervision(
                        mission_id,
                        &mut step_runtime,
                        step_index,
                        StepSupervisorState::Healthy,
                        &success_progress,
                        None,
                        None,
                    )
                    .await;

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
                    step_runtime.status = StepStatus::Completed;
                    step_runtime.recent_progress_events = Self::append_progress_events(
                        &step_runtime.recent_progress_events,
                        vec![StepProgressEvent {
                            kind: StepProgressEventKind::StepCompleted,
                            message: format!("step completed with {} tokens", tokens_used),
                            source: Some(StepProgressEventSource::Executor),
                            layer: Some(StepProgressLayer::DeliveryProgress),
                            semantic_tags: Self::semantic_tags(&["step_completed", "delivery_progress"]),
                            ai_annotation: None,
                            paths: success_progress.artifact_paths.clone(),
                            checks: effective_contract.completion_checks.clone(),
                            score_delta: Some(success_progress.progress_score()),
                            recorded_at: Some(mongodb::bson::DateTime::now()),
                        }],
                    );
                    let final_bundle = Self::merge_step_evidence_bundle(
                        step_runtime.evidence_bundle.as_ref(),
                        &success_progress,
                        step_runtime.output_summary.as_deref(),
                    );
                    if let Err(err) = self
                        .agent_service
                        .set_step_observability(
                            mission_id,
                            step_index,
                            &step_runtime.recent_progress_events,
                            final_bundle.as_ref(),
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist final observability for mission {} step {}: {}",
                            mission_id,
                            step_index,
                            err
                        );
                    } else {
                        step_runtime.evidence_bundle = final_bundle;
                    }
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
                    let timeout_progress = if is_timeout {
                        self.collect_step_progress_snapshot(
                            session_id,
                            messages_before,
                            tokens_before,
                            workspace_path,
                            workspace_before.as_ref(),
                            &step.required_artifacts,
                        )
                        .await
                    } else {
                        StepProgressSnapshot::default()
                    };
                    let productive_timeout = is_timeout && timeout_progress.has_progress();
                    let is_retryable = runtime::is_retryable_error(&e);
                    let can_retry_timeout = !is_timeout
                        || productive_timeout
                        || timeout_retries_used < timeout_retry_limit;
                    if is_retryable && can_retry_timeout && attempt < max_retries {
                        if is_timeout {
                            // Productive timeouts should earn a larger lease on the next attempt
                            // instead of looping forever with the same deadline.
                            timeout_retries_used = timeout_retries_used.saturating_add(1);
                        }
                        tracing::warn!(
                            "Step {}/{} attempt {} failed (retryable, timeout={}, productive_timeout={}, timeout_retries={}/{}): {}",
                            step_index + 1,
                            total_steps,
                            current_attempt_number,
                            is_timeout,
                            productive_timeout,
                            timeout_retries_used,
                            timeout_retry_limit,
                            e
                        );
                        last_err = Some(e);
                        continue;
                    }
                    // Non-retryable or exhausted retries
                    return self
                        .finalize_step_failure(
                            mission_id,
                            session_id,
                            &mut step_runtime,
                            step_index,
                            tokens_before,
                            e,
                        )
                        .await;
                }
            }
        }

        // Should not reach here, but handle exhausted retries
        Err(last_err.unwrap_or_else(|| anyhow!("Step failed after retries")))
        }
        .await;

        match self
            .agent_service
            .delete_session_if_idle(&step_session_id)
            .await
        {
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    "Failed to delete isolated step session {} for mission {} step {}: {}",
                    step_session_id,
                    mission_id,
                    step_index,
                    e
                );
            }
        }

        run_result
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
        let explicit_secs = match (step.timeout_seconds, mission_step_timeout_seconds) {
            (Some(step_secs), Some(mission_secs)) => Some(step_secs.max(mission_secs)),
            (Some(step_secs), None) => Some(step_secs),
            (None, Some(mission_secs)) => Some(mission_secs),
            (None, None) => None,
        };
        let configured_secs = explicit_secs
            .or_else(|| Self::env_u64("TEAM_MISSION_STEP_TIMEOUT_SECS"))
            .unwrap_or(DEFAULT_STEP_EXECUTION_TIMEOUT_SECS);
        let clamped_secs = Self::clamp_step_timeout_secs(configured_secs);
        let min_step_timeout_secs = Self::resolve_min_step_timeout_secs();
        if explicit_secs.is_some() {
            return Duration::from_secs(clamped_secs.max(min_step_timeout_secs));
        }
        let min_complex_timeout_secs =
            Self::resolve_complex_step_timeout_secs(min_step_timeout_secs);
        let floor_secs = if Self::step_requires_extended_timeout(step) {
            min_complex_timeout_secs
        } else {
            min_step_timeout_secs
        };
        Duration::from_secs(clamped_secs.max(floor_secs))
    }

    fn resolve_effective_step_max_retries(
        step_max_retries: u32,
        mission_step_max_retries: Option<u32>,
    ) -> u32 {
        let mission_floor = mission_step_max_retries.unwrap_or(0);
        step_max_retries
            .max(mission_floor)
            .min(MAX_STEP_RETRY_LIMIT)
    }

    fn resolve_timeout_retry_multiplier() -> u64 {
        Self::env_u64("TEAM_MISSION_STEP_TIMEOUT_RETRY_MULTIPLIER")
            .unwrap_or(DEFAULT_STEP_TIMEOUT_RETRY_MULTIPLIER)
            .clamp(1, MAX_STEP_TIMEOUT_RETRY_MULTIPLIER)
    }

    fn resolve_retry_attempt_timeout(
        base_timeout: Duration,
        prior_timeout_retry_level: u32,
        timeout_retries_used: u32,
    ) -> Duration {
        let retry_level = prior_timeout_retry_level.saturating_add(timeout_retries_used);
        let multiplier =
            Self::resolve_timeout_retry_multiplier().saturating_pow(retry_level.min(8));
        let boosted_secs = base_timeout
            .as_secs()
            .saturating_mul(multiplier)
            .min(MAX_STEP_EXECUTION_TIMEOUT_SECS)
            .max(1);
        Duration::from_secs(boosted_secs)
    }

    fn infer_prior_timeout_retry_level(step: &MissionStep, base_timeout: Duration) -> u32 {
        let Some(error) = step.error_message.as_deref() else {
            return 0;
        };
        let Some(previous_secs) = Self::extract_timeout_seconds(error) else {
            return 0;
        };
        let base_secs = base_timeout.as_secs().max(1);
        let multiplier = Self::resolve_timeout_retry_multiplier().max(1);
        let mut level = 0u32;
        let mut expected_secs = base_secs;
        while expected_secs < previous_secs && level < 8 {
            expected_secs = expected_secs
                .saturating_mul(multiplier)
                .min(MAX_STEP_EXECUTION_TIMEOUT_SECS);
            level = level.saturating_add(1);
        }
        if expected_secs == previous_secs {
            level
        } else {
            0
        }
    }

    fn extract_timeout_seconds(message: &str) -> Option<u64> {
        let lower = message.to_ascii_lowercase();
        let marker = "timed out after ";
        let start = lower.find(marker)? + marker.len();
        let digits = lower[start..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        if digits.is_empty() {
            None
        } else {
            digits.parse().ok()
        }
    }

    fn resolve_planned_step_timeout_seconds(
        planned_timeout_seconds: Option<u64>,
        mission_step_timeout_seconds: Option<u64>,
    ) -> Option<u64> {
        match (planned_timeout_seconds, mission_step_timeout_seconds) {
            (Some(step_secs), Some(mission_secs)) => {
                Some(Self::clamp_step_timeout_secs(step_secs.max(mission_secs)))
            }
            (Some(step_secs), None) => Some(Self::clamp_step_timeout_secs(step_secs)),
            (None, Some(mission_secs)) => Some(Self::clamp_step_timeout_secs(mission_secs)),
            (None, None) => None,
        }
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
            .filter_map(|s| {
                mission_verifier::normalize_completion_check(&s, MAX_STEP_COMPLETION_CHECK_CMD_LEN)
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

    fn current_step_attempt_number(step: &MissionStep, local_attempt_index: u32) -> u32 {
        step.retry_count
            .saturating_add(local_attempt_index)
            .saturating_add(1)
    }

    fn classify_supervisor_state(
        previous_state: Option<&StepSupervisorState>,
        stall_count: u32,
        failure_kind: Option<StepFailureKind>,
        snapshot: &StepProgressSnapshot,
        repeated_failure_streak: u32,
        repeated_failed_tool: Option<&str>,
    ) -> StepSupervisorState {
        Self::decide_supervisor_response(
            previous_state,
            stall_count,
            failure_kind,
            snapshot,
            repeated_failure_streak,
            repeated_failed_tool,
        )
        .state
    }

    fn decide_supervisor_response(
        previous_state: Option<&StepSupervisorState>,
        stall_count: u32,
        failure_kind: Option<StepFailureKind>,
        snapshot: &StepProgressSnapshot,
        repeated_failure_streak: u32,
        repeated_failed_tool: Option<&str>,
    ) -> StepSupervisorDecision {
        let repeated_failure_pattern =
            repeated_failure_streak >= 2 || repeated_failed_tool.is_some();
        let blocker = match failure_kind {
            Some(kind) => Some(format!("step_failure:{}", kind.as_str())),
            None if repeated_failure_pattern => Some("repeated_failure_pattern".to_string()),
            _ => None,
        };

        let observed_state = if failure_kind.is_none() {
            if snapshot.has_delivery_progress() {
                StepSupervisorState::Healthy
            } else if snapshot.has_work_progress() || snapshot.has_activity() {
                StepSupervisorState::Busy
            } else {
                StepSupervisorState::Drifting
            }
        } else {
            if repeated_failure_pattern && !snapshot.has_progress() && !snapshot.has_activity() {
                StepSupervisorState::Stalled
            } else {
                let has_delivery_progress = snapshot.has_delivery_progress();
                let has_work_progress = snapshot.has_work_progress();

                if matches!(
                    failure_kind,
                    Some(
                        StepFailureKind::Timeout
                            | StepFailureKind::ToolExecution
                            | StepFailureKind::MaxTurnLimit
                    )
                ) && !has_delivery_progress
                {
                    if has_work_progress && !repeated_failure_pattern {
                        StepSupervisorState::Busy
                    } else if snapshot.has_activity() {
                        StepSupervisorState::Drifting
                    } else {
                        StepSupervisorState::Stalled
                    }
                } else if snapshot.has_progress() {
                    StepSupervisorState::Busy
                } else if snapshot.has_activity() || repeated_failure_pattern {
                    StepSupervisorState::Drifting
                } else {
                    StepSupervisorState::Stalled
                }
            }
        };

        let state = Self::apply_supervisor_transition(
            previous_state,
            stall_count,
            observed_state,
            snapshot,
            repeated_failure_pattern,
        );

        StepSupervisorDecision {
            state: state.clone(),
            blocker,
            should_generate_hint: matches!(
                state,
                StepSupervisorState::Drifting | StepSupervisorState::Stalled
            ),
        }
    }

    fn apply_supervisor_transition(
        previous_state: Option<&StepSupervisorState>,
        stall_count: u32,
        observed_state: StepSupervisorState,
        snapshot: &StepProgressSnapshot,
        repeated_failure_pattern: bool,
    ) -> StepSupervisorState {
        if snapshot.has_delivery_progress() {
            return StepSupervisorState::Healthy;
        }
        if snapshot.has_work_progress() {
            return StepSupervisorState::Busy;
        }

        match observed_state {
            StepSupervisorState::Healthy => StepSupervisorState::Healthy,
            StepSupervisorState::Busy => StepSupervisorState::Busy,
            StepSupervisorState::Drifting => {
                if matches!(previous_state, Some(StepSupervisorState::Busy))
                    && snapshot.has_activity()
                    && !repeated_failure_pattern
                    && stall_count == 0
                {
                    StepSupervisorState::Busy
                } else {
                    StepSupervisorState::Drifting
                }
            }
            StepSupervisorState::Stalled => {
                if snapshot.has_activity() {
                    if matches!(
                        previous_state,
                        Some(StepSupervisorState::Healthy | StepSupervisorState::Busy)
                    ) && !repeated_failure_pattern
                        && stall_count == 0
                    {
                        StepSupervisorState::Busy
                    } else {
                        StepSupervisorState::Drifting
                    }
                } else if matches!(previous_state, Some(StepSupervisorState::Drifting))
                    || repeated_failure_pattern
                    || stall_count >= 1
                {
                    StepSupervisorState::Stalled
                } else {
                    StepSupervisorState::Drifting
                }
            }
        }
    }

    fn compose_retry_turn_instruction(
        base_instruction: Option<String>,
        supervisor_hint: Option<&str>,
    ) -> Option<String> {
        match (base_instruction, supervisor_hint.map(str::trim)) {
            (Some(base), Some(hint)) if !hint.is_empty() => {
                Some(format!("{}\nSupervisor guidance: {}", base, hint))
            }
            (Some(base), _) => Some(base),
            (None, Some(hint)) if !hint.is_empty() => {
                Some(format!("Supervisor guidance: {}", hint))
            }
            _ => None,
        }
    }

    fn runtime_contract_doc_to_preflight(
        contract: &RuntimeContract,
    ) -> runtime::MissionPreflightContract {
        runtime::MissionPreflightContract {
            required_artifacts: contract.required_artifacts.clone(),
            completion_checks: contract.completion_checks.clone(),
            no_artifact_reason: contract.no_artifact_reason.clone(),
        }
    }

    fn error_requires_fresh_preflight(message: &str) -> bool {
        let lower = message.to_ascii_lowercase();
        [
            "missing preflight contract payload",
            "missing preflight contract",
            "mandatory preflight",
            "empty contract",
            "required_artifacts contain invalid",
            "completion_checks contain invalid",
            "unsupported completion check",
            "verify_contract",
            "contract verification gate failed",
            "step completion validation failed",
            "no_artifact_reason",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
    }

    fn should_force_fresh_preflight(
        step: &MissionStep,
        last_error: Option<&str>,
        operator_hint: Option<&str>,
    ) -> bool {
        if operator_hint
            .map(str::trim)
            .is_some_and(|hint| !hint.is_empty())
        {
            return true;
        }
        if last_error.is_some_and(Self::error_requires_fresh_preflight) {
            return true;
        }
        if step
            .error_message
            .as_deref()
            .is_some_and(Self::error_requires_fresh_preflight)
        {
            return true;
        }
        if let Some(verification) = &step.contract_verification {
            if verification.accepted == Some(false) {
                return true;
            }
            if verification.status.as_deref().is_some_and(|status| {
                matches!(
                    status.trim().to_ascii_lowercase().as_str(),
                    "fail" | "error"
                )
            }) {
                return true;
            }
            if verification
                .reason
                .as_deref()
                .is_some_and(Self::error_requires_fresh_preflight)
            {
                return true;
            }
        }
        false
    }

    fn resolve_retry_preflight_contract(
        fresh_contract: Option<runtime::MissionPreflightContract>,
        reusable_contract: Option<&runtime::MissionPreflightContract>,
        step: &MissionStep,
        last_error: Option<&str>,
        operator_hint: Option<&str>,
    ) -> Option<runtime::MissionPreflightContract> {
        if fresh_contract.is_some() {
            return fresh_contract;
        }
        if Self::should_force_fresh_preflight(step, last_error, operator_hint) {
            return None;
        }
        reusable_contract.cloned()
    }

    fn parse_verify_status_label(status: Option<&str>) -> Option<bool> {
        match status.map(str::trim).map(|s| s.to_ascii_lowercase()) {
            Some(status) if status == "pass" || status == "ok" => Some(true),
            Some(status) if status == "fail" || status == "error" => Some(false),
            _ => None,
        }
    }

    fn persisted_verify_contract_state(
        verification: Option<&RuntimeContractVerification>,
    ) -> Option<(bool, Option<bool>)> {
        let verification = verification?;
        if verification.accepted != Some(true) {
            return None;
        }
        let status = Self::parse_verify_status_label(verification.status.as_deref());
        if !verification.tool_called && status.is_none() {
            return None;
        }
        Some((verification.tool_called || status.is_some(), status))
    }

    fn resolve_retry_verify_contract_state(
        fresh_tool_called: bool,
        fresh_status: Option<bool>,
        reusable_state: Option<(bool, Option<bool>)>,
        step: &MissionStep,
        last_error: Option<&str>,
        operator_hint: Option<&str>,
    ) -> (bool, Option<bool>) {
        if fresh_tool_called || fresh_status.is_some() {
            return (fresh_tool_called || fresh_status.is_some(), fresh_status);
        }
        if Self::should_force_fresh_preflight(step, last_error, operator_hint) {
            return (false, None);
        }
        reusable_state.unwrap_or((false, None))
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

    fn compact_prompt_text(input: &str, max_chars: usize) -> String {
        let normalized = input
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if normalized.chars().count() <= max_chars {
            return normalized;
        }
        let truncated = normalized.chars().take(max_chars).collect::<String>();
        format!("{} ...[truncated]", truncated)
    }

    fn compact_list_for_prompt(
        items: &[String],
        max_items: usize,
        max_item_chars: usize,
    ) -> String {
        if items.is_empty() {
            return "none".to_string();
        }
        let mut normalized = items
            .iter()
            .map(|item| Self::compact_prompt_text(item, max_item_chars))
            .collect::<Vec<_>>();
        normalized.sort();
        normalized.dedup();
        let extra = normalized.len().saturating_sub(max_items);
        let visible = normalized
            .into_iter()
            .take(max_items)
            .collect::<Vec<_>>()
            .join(", ");
        if extra > 0 {
            format!("{visible} (+{extra} more)")
        } else {
            visible
        }
    }

    fn compact_recent_progress_for_prompt(
        events: &[StepProgressEvent],
        max_items: usize,
    ) -> String {
        if events.is_empty() {
            return "- none".to_string();
        }

        let non_activity = events
            .iter()
            .filter(|event| !matches!(event.layer, Some(StepProgressLayer::Activity)))
            .collect::<Vec<_>>();
        let source_events = if non_activity.is_empty() {
            events.iter().collect::<Vec<_>>()
        } else {
            non_activity
        };

        let selected = source_events
            .into_iter()
            .rev()
            .scan(
                std::collections::HashSet::<(Option<StepProgressLayer>, String)>::new(),
                |seen, event| {
                    let semantic_key = event
                        .semantic_tags
                        .first()
                        .cloned()
                        .unwrap_or_else(|| Self::compact_prompt_text(&event.message, 48));
                    let key = (event.layer.clone(), semantic_key);
                    if seen.insert(key) {
                        Some(Some(event))
                    } else {
                        Some(None)
                    }
                },
            )
            .flatten()
            .take(max_items)
            .collect::<Vec<_>>();

        if selected.is_empty() {
            return "- none".to_string();
        }

        selected
            .into_iter()
            .rev()
            .map(|event| {
                let source = event
                    .source
                    .as_ref()
                    .map(|v| format!("{:?}", v).to_ascii_lowercase())
                    .unwrap_or_else(|| "unknown".to_string());
                let layer = event
                    .layer
                    .as_ref()
                    .map(|v| format!("{:?}", v).to_ascii_lowercase())
                    .unwrap_or_else(|| "unknown".to_string());
                let message = Self::compact_prompt_text(&event.message, 140);
                format!("- [{}:{}] {}", source, layer, message)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn compact_evidence_for_prompt(bundle: Option<&StepEvidenceBundle>) -> String {
        let Some(bundle) = bundle else {
            return "Evidence digest:\n- none\n".to_string();
        };

        let mut lines = Vec::new();

        if !bundle.artifact_paths.is_empty() {
            lines.push(format!(
                "- artifacts: {} recorded",
                bundle.artifact_paths.len()
            ));
        }

        let categories = [
            (
                "planning",
                Self::compact_list_for_prompt(&bundle.planning_signals, 4, 48),
            ),
            (
                "quality",
                Self::compact_list_for_prompt(&bundle.quality_signals, 4, 48),
            ),
            (
                "runtime",
                Self::compact_list_for_prompt(&bundle.runtime_signals, 4, 48),
            ),
            (
                "deployment",
                Self::compact_list_for_prompt(&bundle.deployment_signals, 4, 48),
            ),
            (
                "review",
                Self::compact_list_for_prompt(&bundle.review_signals, 3, 48),
            ),
            (
                "risk",
                Self::compact_list_for_prompt(&bundle.risk_signals, 4, 48),
            ),
        ];

        for (label, value) in categories {
            if value != "none" {
                lines.push(format!("- {label}: {value}"));
            }
        }

        if let Some(summary) = bundle
            .latest_summary
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            lines.push(format!(
                "- observed_summary: {}",
                Self::compact_prompt_text(summary, 180)
            ));
        }

        if lines.is_empty() {
            "Evidence digest:\n- none\n".to_string()
        } else {
            format!("Evidence digest:\n{}\n", lines.join("\n"))
        }
    }

    fn build_supervisor_hint_prompt(
        mission: &MissionDoc,
        step: &MissionStep,
        failure_kind: Option<StepFailureKind>,
        failure_message: &str,
        progress: &StepProgressSnapshot,
        previous_output: Option<&str>,
        recent_tool_calls: &[runtime::RetryPlaybookToolCall],
        repeated_failure_streak: u32,
        repeated_failed_tool: Option<&str>,
        workspace_path: Option<&str>,
    ) -> String {
        let tool_lines = if recent_tool_calls.is_empty() {
            "- none".to_string()
        } else {
            recent_tool_calls
                .iter()
                .take(RETRY_CONTEXT_TOOL_CALL_LIMIT)
                .map(|call| {
                    format!(
                        "- {} ({})",
                        call.name,
                        if call.success { "success" } else { "failed" }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let previous_output = previous_output
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| Self::compact_prompt_text(text, 1200))
            .unwrap_or_else(|| "none".to_string());
        let required_artifacts = if step.required_artifacts.is_empty() {
            "none".to_string()
        } else {
            Self::compact_list_for_prompt(&step.required_artifacts, 6, 96)
        };
        let completion_checks = if step.completion_checks.is_empty() {
            "none".to_string()
        } else {
            Self::compact_list_for_prompt(&step.completion_checks, 4, 120)
        };
        let evidence_summary = Self::compact_evidence_for_prompt(step.evidence_bundle.as_ref());
        let recent_progress =
            Self::compact_recent_progress_for_prompt(&step.recent_progress_events, 5);
        let content_step_guidance = if Self::step_is_content_heavy(step) {
            "Special handling for this content-heavy step:\n\
- Prefer continuing from existing progress instead of restarting the whole step.\n\
- Recommend the next smallest verifiable intermediate result to persist immediately.\n\
- Favor a staged path such as structured source -> partial output -> validated deliverable -> optional supporting outputs.\n\
- Do not suggest a single giant one-shot rewrite when incremental progress can be saved first.\n\n"
                .to_string()
        } else {
            String::new()
        };
        let repeated_failure_summary =
            if repeated_failure_streak >= 2 || repeated_failed_tool.is_some() {
                format!(
                "Repeated failure pattern:\n- failure_streak: {}\n- repeated_failed_tool: {}\n\n",
                repeated_failure_streak,
                repeated_failed_tool.unwrap_or("none")
            )
            } else {
                String::new()
            };
        format!(
            "You are supervising a long-running mission step retry.\n\
Return JSON only.\n\
- diagnosis: one concise sentence explaining the current blocker or drift.\n\
- resume_hint: concrete next-step guidance that continues from existing outputs, narrows scope, and asks for immediate intermediate persistence when useful.\n\
- semantic_tags (optional): 1-4 short generic tags that describe the blocker or continuation strategy. Prefer broad task-agnostic tags such as research, planning, implementation, verification, recovery, narrowing_scope, incremental_delivery, evidence_gap.\n\
- observed_evidence (optional): 1-3 brief observations grounded in the current evidence or progress signals.\n\
Do not restart the whole step unless absolutely necessary.\n\n\
Keep the language evidence-driven and low-commitment.\n\
Do not declare a phase fully complete unless the evidence shown here directly supports it.\n\
Do not assume a specific deliverable type unless it is explicitly present in the step or evidence.\n\n\
Mission goal:\n{}\n\n\
Current step:\n- title: {}\n- description: {}\n\n\
Failure kind: {}\n\
Failure message: {}\n\
Workspace: {}\n\
Progress snapshot: {}\n\n\
Current supervisor state: {}\n\
Current stall count: {}\n\n\
Required artifacts: {}\n\
Completion checks: {}\n\n\
{}\
{}\
{}\
Recent tool calls:\n{}\n\n\
Recent progress events:\n{}\n\n\
Latest assistant output:\n{}\n",
            mission.goal,
            step.title,
            step.description,
            failure_kind.unwrap_or(StepFailureKind::Unknown).as_str(),
            Self::compact_prompt_text(failure_message, 240),
            workspace_path.unwrap_or("unknown"),
            progress.summary(),
            step.supervisor_state
                .as_ref()
                .map(|v| format!("{:?}", v).to_ascii_lowercase())
                .unwrap_or_else(|| "none".to_string()),
            step.stall_count,
            required_artifacts,
            completion_checks,
            evidence_summary,
            content_step_guidance,
            repeated_failure_summary,
            tool_lines,
            recent_progress,
            previous_output
        )
    }

    fn parse_supervisor_guidance_response(assistant_text: &str) -> Option<SupervisorGuidance> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum StringListOrString {
            List(Vec<String>),
            Single(String),
        }

        impl StringListOrString {
            fn into_vec(self) -> Vec<String> {
                match self {
                    Self::List(items) => items,
                    Self::Single(item) => vec![item],
                }
            }
        }

        #[derive(serde::Deserialize)]
        struct GuidancePayload {
            diagnosis: Option<String>,
            resume_hint: Option<String>,
            semantic_tags: Option<StringListOrString>,
            observed_evidence: Option<StringListOrString>,
        }

        let json_str = runtime::extract_json_block(assistant_text);
        let normalized = runtime::normalize_loose_json(&json_str);
        let payload = serde_json::from_str::<GuidancePayload>(&json_str)
            .or_else(|_| serde_json::from_str::<GuidancePayload>(&normalized))
            .ok()?;
        let diagnosis = payload.diagnosis?.trim().to_string();
        let resume_hint = payload.resume_hint?.trim().to_string();
        if diagnosis.is_empty() || resume_hint.is_empty() {
            return None;
        }
        let semantic_tags = Self::normalize_unique_paths(
            payload
                .semantic_tags
                .map(StringListOrString::into_vec)
                .unwrap_or_default()
                .into_iter()
                .map(|tag| {
                    tag.trim()
                        .to_ascii_lowercase()
                        .replace(char::is_whitespace, "_")
                })
                .filter(|tag| !tag.is_empty())
                .take(4),
        );
        let observed_evidence = Self::normalize_unique_paths(
            payload
                .observed_evidence
                .map(StringListOrString::into_vec)
                .unwrap_or_default()
                .into_iter()
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .take(3),
        );
        Some(SupervisorGuidance {
            diagnosis,
            resume_hint,
            semantic_tags,
            observed_evidence,
        })
    }

    fn classify_retry_failure(
        failure_message: &str,
        recent_tool_calls: &[runtime::RetryPlaybookToolCall],
    ) -> StepFailureKind {
        let lower = failure_message.to_ascii_lowercase();

        if lower.contains("missing preflight contract payload")
            || lower.contains("mandatory preflight")
        {
            return StepFailureKind::PreflightMissing;
        }
        if lower.contains("contract verification gate failed") || lower.contains("verify_contract")
        {
            return StepFailureKind::ContractVerifyGate;
        }
        if lower.contains("outside workspace") || lower.contains("workspace-path guard") {
            return StepFailureKind::WorkspaceGuard;
        }
        if lower.contains("failed to write file") && lower.contains("no such file or directory") {
            return StepFailureKind::MissingParentDirectory;
        }
        if lower.contains("empty assistant output summary") {
            return StepFailureKind::MissingSummary;
        }
        if lower.contains("completion validation failed") {
            return StepFailureKind::ContractValidation;
        }
        if lower.contains("repeated tool call denied") {
            return StepFailureKind::RepeatedToolDenied;
        }
        if lower.contains("security: blocked tool") {
            return StepFailureKind::SecurityToolBlocked;
        }
        if lower.contains("maximum turn limit") || lower.contains("max turn limit") {
            return StepFailureKind::MaxTurnLimit;
        }
        if lower.contains("timed out after") || lower.contains("timeout") {
            return StepFailureKind::Timeout;
        }
        if lower.contains("failed to deserialize parameters")
            || lower.contains("unknown variant")
            || lower.contains("missing field")
            || lower.contains("invalid type")
        {
            return StepFailureKind::ToolParameterSchema;
        }
        if recent_tool_calls.iter().any(|call| !call.success) || lower.contains("mcp error") {
            return StepFailureKind::ToolExecution;
        }

        StepFailureKind::Unknown
    }

    fn build_retry_turn_instruction(
        failure_kind: StepFailureKind,
        step: &MissionStep,
        progress: &StepProgressSnapshot,
        repeated_failure_streak: u32,
        repeated_failed_tool: Option<&str>,
        workspace_path: Option<&str>,
    ) -> Option<String> {
        let workspace_hint = workspace_path
            .map(|path| format!("Current workspace root: {}.", path))
            .unwrap_or_default();
        let incremental_persistence_hint =
            Self::render_incremental_persistence_hint(step, progress, workspace_path);
        let repeated_failure_hint =
            Self::render_repeated_failure_hint(repeated_failure_streak, repeated_failed_tool);

        let instruction = match failure_kind {
            StepFailureKind::PreflightMissing => format!(
                "Retry focus: call `{}` first, before any other tool. Declare the real required_artifacts/completion_checks for this step, then continue. Before concluding, call `{}`. {}",
                MISSION_PREFLIGHT_TOOL_NAME,
                MISSION_VERIFY_CONTRACT_TOOL_NAME,
                workspace_hint
            ),
            StepFailureKind::ContractValidation => format!(
                "Retry focus: do not claim completion until every required artifact exists and every completion check passes. Produce the real artifact first, verify it explicitly, then conclude. {}",
                workspace_hint
            ),
            StepFailureKind::ContractVerifyGate => format!(
                "Retry focus: after producing outputs, run `{}` and satisfy its result before finishing. Do not skip the verification tool on this retry. {}",
                MISSION_VERIFY_CONTRACT_TOOL_NAME,
                workspace_hint
            ),
            StepFailureKind::WorkspaceGuard => format!(
                "Retry focus: only save deliverables under workspace-relative paths. Treat absolute system paths, process metadata, logs, and diagnostic text as non-artifacts. Verify the final artifact path with pwd/ls before finishing. {}",
                workspace_hint
            ),
            StepFailureKind::MissingParentDirectory => format!(
                "Retry focus: create the parent directory before writing files into nested paths. Use workspace-relative paths, run mkdir -p or equivalent once, then retry the smallest write. {}",
                workspace_hint
            ),
            StepFailureKind::MissingSummary => "Retry focus: do not rerun the whole step. First inspect the files or service state already produced, then emit a concise completion summary that states exactly what was created, how it was verified, and where the outputs are.".to_string(),
            StepFailureKind::RepeatedToolDenied => "Retry focus: do not repeat the same tool call unchanged. Inspect the current file or state first, then make a smaller, different edit or verification step.".to_string(),
            StepFailureKind::SecurityToolBlocked => "Retry focus: simplify the blocked shell command. Avoid command substitution, nested quoting, and chained one-liners. Split the action into smaller explicit commands or use safer non-shell tools where possible.".to_string(),
            StepFailureKind::MaxTurnLimit => "Retry focus: avoid re-exploration. Use the shortest path to satisfy the current step contract, with direct tools and concise checks only.".to_string(),
            StepFailureKind::Timeout => {
                if Self::step_is_content_heavy(step) {
                    format!(
                        "Retry focus: continue from the progress already made. Do not restart the whole step. {} {} {}",
                        incremental_persistence_hint,
                        repeated_failure_hint,
                        workspace_hint
                    )
                } else {
                    format!(
                        "Retry focus: reduce scope and avoid repeating expensive scans or installs. Reuse existing outputs, make one concrete change, then verify. {} {}",
                        repeated_failure_hint,
                        workspace_hint
                    )
                }
            }
            StepFailureKind::ToolParameterSchema => "Retry focus: call the failing tool again using canonical schema values only. For enum-like parameters, use short exact values from the tool schema, not descriptive labels.".to_string(),
            StepFailureKind::ToolExecution => format!(
                "Retry focus: inspect the last failing tool call, change arguments or environment minimally, and retry with the smallest viable action. Do not repeat the same failing call unchanged. {}",
                repeated_failure_hint
            ),
            StepFailureKind::Unknown => format!(
                "Retry focus: diagnose the previous failure first, choose a different strategy, and verify the result before concluding. {}",
                repeated_failure_hint
            ),
        };

        Some(instruction)
    }

    fn step_is_content_heavy(step: &MissionStep) -> bool {
        let lower = format!("{} {}", step.title, step.description).to_ascii_lowercase();
        let keywords = [
            "html",
            "report",
            "slides",
            "slide",
            "markdown",
            "svg",
            "chart",
            "ppt",
            "presentation",
            "spreadsheet",
            "xlsx",
            "table",
            "document",
            "pdf",
            "dashboard",
            "调研报告",
            "报告",
            "演示稿",
            "幻灯",
            "幻灯片",
            "汇报",
            "页面",
            "中文",
        ];
        keywords
            .iter()
            .filter(|keyword| lower.contains(**keyword))
            .count()
            >= 2
    }

    fn render_incremental_persistence_hint(
        step: &MissionStep,
        progress: &StepProgressSnapshot,
        workspace_path: Option<&str>,
    ) -> String {
        let required_targets = step
            .required_artifacts
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>();
        let target_hint = if required_targets.is_empty() {
            "a small contract-aligned intermediate file, note, dataset snapshot, or partial deliverable"
                .to_string()
        } else {
            required_targets.join(", ")
        };
        let progress_hint = if progress.has_progress() {
            "Reuse the partial outputs already present in the workspace."
        } else {
            "Start with the smallest viable intermediate artifact before attempting the final deliverable."
        };
        let workspace_hint = workspace_path
            .map(|path| {
                format!(
                    " Save it under workspace-relative paths rooted at {}.",
                    path
                )
            })
            .unwrap_or_default();
        format!(
            "{} Persist a smaller intermediate result first, such as {}. Verify that it exists immediately after writing it, then continue with the next layer. Avoid one giant one-shot rewrite when incremental progress can be saved first.{}",
            progress_hint, target_hint, workspace_hint
        )
    }

    fn render_repeated_failure_hint(
        repeated_failure_streak: u32,
        repeated_failed_tool: Option<&str>,
    ) -> String {
        if repeated_failure_streak < 2 && repeated_failed_tool.is_none() {
            return String::new();
        }
        let tool_hint = repeated_failed_tool
            .map(|tool| format!(" Stop retrying `{}` unchanged.", tool))
            .unwrap_or_default();
        format!(
            "A repeated failure pattern is present (streak={}). Change strategy now instead of replaying the same action.{}",
            repeated_failure_streak, tool_hint
        )
    }

    fn detect_repeated_failed_tool(
        recent_tool_calls: &[runtime::RetryPlaybookToolCall],
    ) -> Option<String> {
        let last_failed = recent_tool_calls
            .iter()
            .rev()
            .find(|call| !call.success)
            .map(|call| call.name.as_str())?;
        let failure_count = recent_tool_calls
            .iter()
            .filter(|call| !call.success && call.name == last_failed)
            .count();
        (failure_count >= 2).then(|| last_failed.to_string())
    }

    async fn finalize_step_failure(
        &self,
        mission_id: &str,
        session_id: &str,
        step: &mut MissionStep,
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
        self.update_step_supervision(
            mission_id,
            step,
            step_index,
            StepSupervisorState::Stalled,
            &StepProgressSnapshot::default(),
            Some(&err_msg),
            None,
        )
        .await;
        step.status = StepStatus::Failed;
        step.error_message = Some(err_msg.clone());
        step.recent_progress_events = Self::append_progress_events(
            &step.recent_progress_events,
            vec![StepProgressEvent {
                kind: StepProgressEventKind::StepFailed,
                message: err_msg.clone(),
                source: Some(StepProgressEventSource::Supervisor),
                layer: Some(StepProgressLayer::Recovery),
                semantic_tags: Self::semantic_tags(&["step_failed", "recovery"]),
                ai_annotation: None,
                paths: Vec::new(),
                checks: Vec::new(),
                score_delta: None,
                recorded_at: Some(mongodb::bson::DateTime::now()),
            }],
        );
        if let Err(db_err) = self
            .agent_service
            .set_step_observability(
                mission_id,
                step_index,
                &step.recent_progress_events,
                step.evidence_bundle.as_ref(),
            )
            .await
        {
            tracing::warn!(
                "Failed to persist failure observability for mission {} step {}: {}",
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
        turn_system_instruction: Option<&str>,
    ) -> Result<()> {
        runtime::execute_via_bridge_with_turn_instruction(
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
            turn_system_instruction,
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
            cancel_token.clone(),
            workspace_path,
            None, // no mission_context during planning phase
            None,
        )
        .await?;

        // Parse plan from session messages
        let session = self
            .agent_service
            .get_session(session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Session not found"))?;
        let assistant_text = Self::extract_last_assistant_message_text(&session.messages_json)?;
        let initial_plan = assistant_text
            .as_deref()
            .ok_or_else(|| anyhow!("planning has no assistant response"))
            .and_then(|text| Self::parse_plan_response(text, mission));

        match initial_plan {
            Ok(steps) if !Self::plan_requires_expansion(&mission.goal, &steps) => Ok(steps),
            Ok(steps) => {
                tracing::warn!(
                    "Mission {} planning produced overly coarse plan ({} step); attempting repair",
                    mission_id,
                    steps.len()
                );
                if let Some(repaired) = self
                    .repair_generated_plan(
                        mission_id,
                        mission,
                        session_id,
                        cancel_token,
                        workspace_path,
                        "planning response was too coarse for a complex goal",
                        assistant_text.as_deref(),
                    )
                    .await?
                {
                    return Ok(repaired);
                }
                Ok(Self::fallback_steps_from_goal(
                    &mission.goal,
                    mission.step_max_retries,
                    mission.step_timeout_seconds,
                ))
            }
            Err(e) => {
                tracing::warn!(
                    "Mission {} planning parse failed: {}. Attempting repair before fallback.",
                    mission_id,
                    e
                );
                if let Some(repaired) = self
                    .repair_generated_plan(
                        mission_id,
                        mission,
                        session_id,
                        cancel_token,
                        workspace_path,
                        &e.to_string(),
                        assistant_text.as_deref(),
                    )
                    .await?
                {
                    return Ok(repaired);
                }
                Ok(Self::fallback_steps_from_goal(
                    &mission.goal,
                    mission.step_max_retries,
                    mission.step_timeout_seconds,
                ))
            }
        }
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

    fn extract_last_assistant_message_text(messages_json: &str) -> Result<Option<String>> {
        let msgs: Vec<serde_json::Value> =
            serde_json::from_str(messages_json).map_err(|e| anyhow!("Invalid messages: {}", e))?;

        Ok(msgs
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
            }))
    }

    fn parse_plan_response(assistant_text: &str, mission: &MissionDoc) -> Result<Vec<MissionStep>> {
        // Extract JSON from ```json ... ``` block or try direct parse
        let json_str = runtime::extract_json_block(assistant_text);
        let steps = Self::parse_steps_json(
            &json_str,
            0,
            mission.step_max_retries,
            mission.step_timeout_seconds,
        )?;
        if steps.is_empty() {
            return Err(anyhow!("planning produced empty steps"));
        }
        Ok(steps)
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
                let max_retries = Self::resolve_effective_step_max_retries(
                    Self::resolve_step_max_retries(ps.max_retries, mission_step_max_retries),
                    mission_step_max_retries,
                );
                let timeout_seconds = Self::resolve_planned_step_timeout_seconds(
                    ps.timeout_seconds,
                    mission_step_timeout_seconds,
                );
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
                    supervisor_state: None,
                    last_activity_at: None,
                    last_progress_at: None,
                    progress_score: None,
                    current_blocker: None,
                    last_supervisor_hint: None,
                    stall_count: 0,
                    recent_progress_events: vec![],
                    evidence_bundle: None,
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
            supervisor_state: None,
            last_activity_at: None,
            last_progress_at: None,
            progress_score: None,
            current_blocker: None,
            last_supervisor_hint: None,
            stall_count: 0,
            recent_progress_events: vec![],
            evidence_bundle: None,
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

    fn fallback_steps_from_goal(
        mission_goal: &str,
        mission_step_max_retries: Option<u32>,
        mission_step_timeout_seconds: Option<u64>,
    ) -> Vec<MissionStep> {
        if Self::goal_requires_multi_step_plan(mission_goal)
            && Self::goal_requires_multi_artifact_delivery_plan(mission_goal)
        {
            return Self::build_multi_artifact_runtime_fallback_steps(
                mission_goal,
                mission_step_max_retries,
                mission_step_timeout_seconds,
            );
        }
        if Self::goal_requires_multi_step_plan(mission_goal) {
            return Self::build_runtime_fallback_steps(
                mission_goal,
                mission_step_max_retries,
                mission_step_timeout_seconds,
            );
        }
        if Self::goal_requires_multi_artifact_delivery_plan(mission_goal) {
            return Self::build_multi_artifact_delivery_fallback_steps(
                mission_goal,
                mission_step_max_retries,
                mission_step_timeout_seconds,
            );
        }
        vec![Self::fallback_step_from_goal(
            mission_goal,
            mission_step_max_retries,
            mission_step_timeout_seconds,
        )]
    }

    fn goal_requires_multi_step_plan(goal: &str) -> bool {
        let lower = goal.to_ascii_lowercase();
        let keywords = [
            "express",
            "node",
            "server",
            "service",
            "deploy",
            "pm2",
            "port",
            "endpoint",
            "api",
            "ui",
            "frontend",
            "web",
            "health",
            "search",
            "database",
            "long-running",
            "long running",
            "部署",
            "服务",
            "接口",
            "端口",
            "前端",
            "页面",
            "验证",
        ];
        let score = keywords.iter().filter(|kw| lower.contains(**kw)).count();
        score >= 3
            || (score >= 2
                && [" and ", ",", "/api/", "/health", " + "]
                    .iter()
                    .any(|marker| lower.contains(marker)))
    }

    fn goal_requires_multi_artifact_delivery_plan(goal: &str) -> bool {
        let lower = goal.to_ascii_lowercase();
        let deliverable_keywords = [
            "html",
            "report",
            "slides",
            "slide",
            "presentation",
            "ppt",
            "pdf",
            "spreadsheet",
            "xlsx",
            "table",
            "dashboard",
            "document",
            "调研报告",
            "报告",
            "演示稿",
            "幻灯",
            "幻灯片",
            "汇报",
            "文档",
            "表格",
            "图表",
        ];
        let score = deliverable_keywords
            .iter()
            .filter(|keyword| lower.contains(**keyword))
            .count();
        score >= 2
    }

    fn plan_requires_expansion(goal: &str, steps: &[MissionStep]) -> bool {
        steps.len() <= 1 && Self::goal_requires_multi_step_plan(goal)
    }

    fn build_runtime_fallback_steps(
        mission_goal: &str,
        mission_step_max_retries: Option<u32>,
        mission_step_timeout_seconds: Option<u64>,
    ) -> Vec<MissionStep> {
        let base_retries = Self::resolve_effective_step_max_retries(2, mission_step_max_retries);
        let base_timeout =
            Self::resolve_planned_step_timeout_seconds(None, mission_step_timeout_seconds);
        let make_step = |index: u32, title: &str, description: String| MissionStep {
            index,
            title: title.to_string(),
            description,
            status: StepStatus::Pending,
            is_checkpoint: false,
            approved_by: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            supervisor_state: None,
            last_activity_at: None,
            last_progress_at: None,
            progress_score: None,
            current_blocker: None,
            last_supervisor_hint: None,
            stall_count: 0,
            recent_progress_events: vec![],
            evidence_bundle: None,
            tokens_used: 0,
            output_summary: None,
            retry_count: 0,
            max_retries: base_retries,
            timeout_seconds: base_timeout,
            required_artifacts: vec![],
            completion_checks: vec![],
            runtime_contract: None,
            contract_verification: None,
            use_subagent: false,
            tool_calls: vec![],
        };
        vec![
            make_step(
                0,
                "Preflight + workspace scaffold",
                format!(
                    "Inspect the mission workspace, confirm tool/runtime prerequisites, and scaffold the project skeleton needed for this goal: {}",
                    mission_goal
                ),
            ),
            make_step(
                1,
                "Implement backend/core functionality",
                "Build the core application logic, APIs, and data model required by the mission goal.".to_string(),
            ),
            make_step(
                2,
                "Add UI/content/supporting assets",
                "Create the required UI, sample content, supporting files, and integration glue so the deliverable is actually usable.".to_string(),
            ),
            make_step(
                3,
                "Install dependencies + local quality verification",
                "Install dependencies, run the app locally in the workspace, and perform the smallest meaningful quality pass before deployment (for example build, lint, typecheck, tests, or smoke checks when available). Save concise evidence for what passed, what was skipped, and why.".to_string(),
            ),
            make_step(
                4,
                "Deploy long-running service + runtime checks",
                "Deploy the app as a long-running service, verify live endpoints with runtime checks, and capture concise deployment evidence including commands, process state, and endpoint results.".to_string(),
            ),
            make_step(
                5,
                "Finalize reports and artifacts",
                "Save concise verification, quality, and deployment evidence under reports/final and ensure all final artifacts are available in the workspace.".to_string(),
            ),
        ]
    }

    fn build_multi_artifact_delivery_fallback_steps(
        mission_goal: &str,
        mission_step_max_retries: Option<u32>,
        mission_step_timeout_seconds: Option<u64>,
    ) -> Vec<MissionStep> {
        let base_retries = Self::resolve_effective_step_max_retries(2, mission_step_max_retries);
        let base_timeout =
            Self::resolve_planned_step_timeout_seconds(None, mission_step_timeout_seconds);
        let make_step = |index: u32, title: &str, description: String| MissionStep {
            index,
            title: title.to_string(),
            description,
            status: StepStatus::Pending,
            is_checkpoint: false,
            approved_by: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            supervisor_state: None,
            last_activity_at: None,
            last_progress_at: None,
            progress_score: None,
            current_blocker: None,
            last_supervisor_hint: None,
            stall_count: 0,
            recent_progress_events: vec![],
            evidence_bundle: None,
            tokens_used: 0,
            output_summary: None,
            retry_count: 0,
            max_retries: base_retries,
            timeout_seconds: base_timeout,
            required_artifacts: vec![],
            completion_checks: vec![],
            runtime_contract: None,
            contract_verification: None,
            use_subagent: false,
            tool_calls: vec![],
        };
        vec![
            make_step(
                0,
                "Preflight + workspace scaffold",
                format!(
                    "Inspect the workspace, confirm prerequisites, and scaffold the minimal structure required for this goal: {}",
                    mission_goal
                ),
            ),
            make_step(
                1,
                "Prepare structured source material",
                "Create the source data, outline, notes, or intermediate assets that will drive the final deliverables.".to_string(),
            ),
            make_step(
                2,
                "Generate primary deliverable incrementally",
                "Produce the primary deliverable through small verifiable iterations, saving intermediate results before the final version.".to_string(),
            ),
            make_step(
                3,
                "Generate companion deliverables and supporting assets",
                "Produce any companion outputs, presentation assets, tables, or supporting files needed by the mission goal.".to_string(),
            ),
            make_step(
                4,
                "Local quality verification and review",
                "Run the smallest meaningful quality pass for the current deliverables, record strong evidence for what passed, and note any skipped checks with reasons.".to_string(),
            ),
            make_step(
                5,
                "Finalize evidence bundle",
                "Save concise verification, quality, and delivery evidence under reports/final and ensure all expected deliverables are present in the workspace.".to_string(),
            ),
        ]
    }

    fn build_multi_artifact_runtime_fallback_steps(
        mission_goal: &str,
        mission_step_max_retries: Option<u32>,
        mission_step_timeout_seconds: Option<u64>,
    ) -> Vec<MissionStep> {
        let base_retries = Self::resolve_effective_step_max_retries(2, mission_step_max_retries);
        let base_timeout =
            Self::resolve_planned_step_timeout_seconds(None, mission_step_timeout_seconds);
        let make_step = |index: u32, title: &str, description: String| MissionStep {
            index,
            title: title.to_string(),
            description,
            status: StepStatus::Pending,
            is_checkpoint: false,
            approved_by: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            supervisor_state: None,
            last_activity_at: None,
            last_progress_at: None,
            progress_score: None,
            current_blocker: None,
            last_supervisor_hint: None,
            stall_count: 0,
            recent_progress_events: vec![],
            evidence_bundle: None,
            tokens_used: 0,
            output_summary: None,
            retry_count: 0,
            max_retries: base_retries,
            timeout_seconds: base_timeout,
            required_artifacts: vec![],
            completion_checks: vec![],
            runtime_contract: None,
            contract_verification: None,
            use_subagent: false,
            tool_calls: vec![],
        };
        vec![
            make_step(
                0,
                "Preflight + workspace scaffold",
                format!(
                    "Inspect the workspace, confirm prerequisites, and scaffold the minimal runtime surface required for this goal: {}",
                    mission_goal
                ),
            ),
            make_step(
                1,
                "Implement core runtime surface",
                "Build the service shell, application entrypoints, and integration surface required by the mission goal.".to_string(),
            ),
            make_step(
                2,
                "Prepare structured source material and intermediate assets",
                "Create the source data, outline, notes, or intermediate assets that will drive the final deliverables.".to_string(),
            ),
            make_step(
                3,
                "Generate primary deliverable incrementally",
                "Produce the primary deliverable through small verifiable iterations, saving intermediate results before the final version.".to_string(),
            ),
            make_step(
                4,
                "Generate companion deliverables and supporting assets",
                "Produce any companion outputs, presentation assets, tables, or supporting files needed by the mission goal.".to_string(),
            ),
            make_step(
                5,
                "Local quality verification and review",
                "Run the smallest meaningful quality pass before deployment, record strong evidence for what passed, and note any skipped checks with reasons.".to_string(),
            ),
            make_step(
                6,
                "Deploy runtime and verify live interfaces",
                "Deploy the long-running runtime, verify live interfaces with runtime checks, and capture concise deployment evidence including commands, process state, and endpoint results.".to_string(),
            ),
            make_step(
                7,
                "Finalize evidence bundle",
                "Save concise verification, quality, deployment, and delivery evidence under reports/final and ensure all expected deliverables are present in the workspace.".to_string(),
            ),
        ]
    }

    fn render_mission_plan_repair_prompt(
        goal: &str,
        context: Option<&str>,
        failure_reason: &str,
        previous_response: Option<&str>,
    ) -> String {
        let previous_response = previous_response
            .map(|text| Self::truncate_prompt_excerpt(text.trim(), 1800))
            .unwrap_or_else(|| "(no previous response captured)".to_string());
        let extra_context = context
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| format!("\n## Additional Context\n{}\n", text))
            .unwrap_or_default();
        format!(
            "Your previous mission plan was invalid or too coarse.\n\n\
## Mission Goal\n{goal}\n\
{extra_context}\
## Repair Requirement\n\
- Return ONLY one JSON array in a ```json code block.\n\
- Produce 4-8 concrete executable steps for complex build/deploy/service goals.\n\
- Do not collapse the entire mission into a single step.\n\
- Each step must be actionable, dependency-ordered, and verifiable.\n\
- Include deployment/runtime verification steps when the goal mentions ports, services, APIs, UI, or pm2.\n\
- Keep titles short and descriptions concrete.\n\n\
## Why the previous response was rejected\n\
{failure_reason}\n\n\
## Previous response (for repair)\n\
```text\n{previous_response}\n```\n",
        )
    }

    async fn repair_generated_plan(
        &self,
        mission_id: &str,
        mission: &MissionDoc,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
        failure_reason: &str,
        previous_response: Option<&str>,
    ) -> Result<Option<Vec<MissionStep>>> {
        let prompt = Self::render_mission_plan_repair_prompt(
            &mission.goal,
            mission.context.as_deref(),
            failure_reason,
            previous_response,
        );
        self.execute_via_bridge(
            &mission.agent_id,
            session_id,
            mission_id,
            &prompt,
            cancel_token,
            workspace_path,
            None,
            None,
        )
        .await?;
        let session = self
            .agent_service
            .get_session(session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Session not found"))?;
        let repaired_text = Self::extract_last_assistant_message_text(&session.messages_json)?;
        let Some(repaired_text) = repaired_text else {
            return Ok(None);
        };
        let repaired_steps = match Self::parse_plan_response(&repaired_text, mission) {
            Ok(steps) => steps,
            Err(err) => {
                tracing::warn!(
                    "Mission {} planning repair still failed to parse: {}",
                    mission_id,
                    err
                );
                return Ok(None);
            }
        };
        if Self::plan_requires_expansion(&mission.goal, &repaired_steps) {
            tracing::warn!(
                "Mission {} planning repair still returned overly coarse plan ({} step)",
                mission_id,
                repaired_steps.len()
            );
            return Ok(None);
        }
        Ok(Some(repaired_steps))
    }

    fn truncate_prompt_excerpt(value: &str, max_chars: usize) -> String {
        if value.chars().count() <= max_chars {
            value.to_string()
        } else {
            let mut out = value
                .chars()
                .take(max_chars.saturating_sub(3))
                .collect::<String>();
            out.push_str("...");
            out
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
        preflight_attempt: u32,
        preflight_last_error: Option<&str>,
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
            if Self::step_should_prefer_runtime_completion_checks(step) {
                prompt.push_str(
                    "- This is a runtime/service verification step. In preflight, include at least one deterministic runtime completion check (for example `curl ...`, `ss ...`, `pm2 ...`, or `rg ...`) instead of relying on file existence alone.\n",
                );
            }
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

        if Self::step_should_collect_quality_evidence(step) {
            prompt.push_str("\n## Delivery Quality Guidance (Recommended, Not a Hard Gate)\n");
            prompt.push_str(
                "- Prefer the smallest meaningful quality pass for this step instead of exhaustive bureaucracy.\n",
            );
            prompt.push_str(
                "- When relevant, run available checks such as build, lint, typecheck, unit test, smoke test, runtime health checks, or a concise code review.\n",
            );
            prompt.push_str(
                "- If a tool or check is unavailable, do not fake success; note it briefly with a reason and continue with the strongest available evidence.\n",
            );
            prompt.push_str(
                "- Preserve incremental quality evidence in the workspace as you go instead of waiting for the entire mission to finish.\n",
            );
            prompt.push_str(&format!(
                "- Recommended evidence note path: `{}`.\n",
                Self::recommended_quality_evidence_path(step_index)
            ));
            if Self::step_should_prefer_runtime_completion_checks(step) {
                prompt.push_str(
                    "- For runtime/deployment steps, capture endpoint, process, and command evidence alongside the final health result.\n",
                );
            }
            if Self::step_is_content_heavy(step) {
                prompt.push_str(
                    "- For content-heavy deliverables, record the intermediate source material, outline, or partial output you relied on before final assembly.\n",
                );
            }
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
        let preflight_last_error =
            Self::escape_json_for_prompt(preflight_last_error.unwrap_or_default());
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
        prompt.push_str(&format!("  \"attempt\": {},\n", preflight_attempt.max(1)));
        prompt.push_str(&format!("  \"last_error\": \"{}\"\n", preflight_last_error));
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
        prompt.push_str(
            "- `completion_checks` may be either `exists:<relative_path>` or deterministic shell commands that can run inside the workspace.\n",
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
            "- For normal file edits, prefer `text_editor` with `write`, `str_replace` (old_str/new_str), or `insert`. Use `diff` only when you can provide a proper unified diff with `---` and `+++` headers.\n",
        );
        prompt.push_str(
            "- Treat `.env.example`, `.env.sample`, and `.env.template` as safe templates you may inspect or edit. Do not print secret-bearing `.env` files with shell; if runtime values are already known, write or update `.env` directly instead of reading it.\n",
        );
        prompt.push_str(
            "- For binary deliverables that should be archived, use `create_document_from_file` with the real artifact path\n",
        );
        prompt.push_str("- Do not claim completion without verifiable outputs\n");
        prompt.push_str("- Be concise — your response will be saved as step summary");
        prompt
    }

    fn step_should_prefer_runtime_completion_checks(step: &MissionStep) -> bool {
        let combined = format!("{} {}", step.title, step.description).to_ascii_lowercase();
        [
            "port",
            "endpoint",
            "api",
            "health",
            "pm2",
            "deploy",
            "verification",
            "verify",
            "service",
            "smoke",
            "ui",
        ]
        .iter()
        .any(|keyword| combined.contains(keyword))
    }

    fn step_should_collect_quality_evidence(step: &MissionStep) -> bool {
        let combined = format!("{} {}", step.title, step.description).to_ascii_lowercase();
        let engineering_or_runtime = [
            "build",
            "lint",
            "typecheck",
            "test",
            "verification",
            "verify",
            "deploy",
            "service",
            "api",
            "ui",
            "code review",
            "quality",
            "验证",
            "部署",
            "质量",
            "代码审查",
        ]
        .iter()
        .any(|keyword| combined.contains(keyword));

        engineering_or_runtime || Self::step_is_content_heavy(step)
    }

    fn recommended_quality_evidence_path(step_index: u32) -> String {
        format!("reports/final/quality/step-{}-quality.md", step_index + 1)
    }

    fn normalize_unique_paths<I>(paths: I) -> Vec<String>
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();
        for raw in paths {
            let normalized = raw.into().trim().replace('\\', "/");
            if normalized.is_empty() {
                continue;
            }
            if seen.insert(normalized.to_ascii_lowercase()) {
                ordered.push(normalized);
            }
        }
        ordered
    }

    fn semantic_tags(tags: &[&str]) -> Vec<String> {
        Self::normalize_unique_paths(tags.iter().map(|tag| (*tag).to_string()))
    }

    fn path_matches_keywords(path: &str, keywords: &[&str]) -> bool {
        let lower = path.to_ascii_lowercase();
        keywords.iter().any(|keyword| lower.contains(keyword))
    }

    fn infer_signal_keys(paths: &[String], mapping: &[(&str, &str)]) -> Vec<String> {
        let mut signals = Vec::new();
        for path in paths {
            let lower = path.to_ascii_lowercase();
            for (needle, signal) in mapping {
                if lower.contains(needle) {
                    signals.push((*signal).to_string());
                }
            }
        }
        Self::normalize_unique_paths(signals)
    }

    fn classify_step_evidence_paths(paths: &[String]) -> StepEvidenceBundle {
        let mut bundle = StepEvidenceBundle::default();
        let normalized = Self::normalize_unique_paths(paths.iter().cloned());
        bundle.artifact_paths = normalized.clone();
        bundle.required_artifact_paths = normalized.clone();
        bundle.planning_evidence_paths = normalized
            .iter()
            .filter(|path| {
                Self::path_matches_keywords(
                    path,
                    &[
                        "mission-plan",
                        "/plan/",
                        "plan/",
                        "outline",
                        "notes",
                        "research",
                        "analysis",
                        "index.md",
                        "workspace-overview",
                    ],
                )
            })
            .cloned()
            .collect();
        bundle.planning_signals = Self::infer_signal_keys(
            &bundle.planning_evidence_paths,
            &[
                ("mission-plan", "mission_plan_evidence"),
                ("/plan/", "plan_evidence"),
                ("outline", "outline_evidence"),
                ("notes", "notes_evidence"),
                ("research", "research_evidence"),
                ("analysis", "analysis_evidence"),
                ("workspace-overview", "workspace_overview_evidence"),
            ],
        );
        bundle.quality_evidence_paths = normalized
            .iter()
            .filter(|path| {
                Self::path_matches_keywords(
                    path,
                    &[
                        "/quality/",
                        "quality/",
                        "build.log",
                        "lint.log",
                        "test.log",
                        "smoke.log",
                        "typecheck.log",
                        "commands-run",
                        "quality-skip-reasons",
                    ],
                )
            })
            .cloned()
            .collect();
        bundle.quality_signals = Self::infer_signal_keys(
            &bundle.quality_evidence_paths,
            &[
                ("build.log", "build_evidence"),
                ("lint.log", "lint_evidence"),
                ("test.log", "test_evidence"),
                ("smoke.log", "smoke_evidence"),
                ("typecheck.log", "typecheck_evidence"),
                ("commands-run", "commands_recorded"),
                ("quality-skip-reasons", "quality_skip_reasons"),
            ],
        );
        bundle.runtime_evidence_paths = normalized
            .iter()
            .filter(|path| {
                Self::path_matches_keywords(
                    path,
                    &[
                        "/runtime/",
                        "health.",
                        "report-meta",
                        "report.http",
                        "slides.http",
                        "runtime/",
                        "smoke-server",
                    ],
                )
            })
            .cloned()
            .collect();
        bundle.runtime_signals = Self::infer_signal_keys(
            &bundle.runtime_evidence_paths,
            &[
                ("health.", "health_check_evidence"),
                ("report-meta", "report_meta_evidence"),
                ("report.http", "report_http_evidence"),
                ("slides.http", "slides_http_evidence"),
                ("smoke-server", "smoke_server_evidence"),
            ],
        );
        bundle.deployment_evidence_paths = normalized
            .iter()
            .filter(|path| {
                Self::path_matches_keywords(
                    path,
                    &[
                        "deployment",
                        "pm2",
                        "ecosystem.config",
                        "deploy",
                        "verification",
                    ],
                )
            })
            .cloned()
            .collect();
        bundle.deployment_signals = Self::infer_signal_keys(
            &bundle.deployment_evidence_paths,
            &[
                ("pm2", "pm2_evidence"),
                ("ecosystem.config", "pm2_config_evidence"),
                ("deployment", "deployment_evidence"),
                ("verification", "deployment_verification_evidence"),
                ("deploy", "deploy_evidence"),
            ],
        );
        bundle.review_evidence_paths = normalized
            .iter()
            .filter(|path| Self::path_matches_keywords(path, &["review", "code-review"]))
            .cloned()
            .collect();
        bundle.review_signals = Self::infer_signal_keys(
            &bundle.review_evidence_paths,
            &[
                ("code-review", "code_review_evidence"),
                ("review", "review_evidence"),
            ],
        );
        bundle.risk_evidence_paths = normalized
            .iter()
            .filter(|path| {
                Self::path_matches_keywords(
                    path,
                    &[
                        "risk",
                        "blocker",
                        "limitation",
                        "warning",
                        "known-issues",
                        "quality-skip-reasons",
                        "gap",
                    ],
                )
            })
            .cloned()
            .collect();
        bundle.risk_signals = Self::infer_signal_keys(
            &bundle.risk_evidence_paths,
            &[
                ("risk", "risk_evidence"),
                ("blocker", "blocker_evidence"),
                ("limitation", "limitation_evidence"),
                ("warning", "warning_evidence"),
                ("known-issues", "known_issues_evidence"),
                ("quality-skip-reasons", "quality_skip_reasons"),
                ("gap", "gap_evidence"),
            ],
        );
        bundle
    }

    fn merge_step_evidence_bundle(
        existing: Option<&StepEvidenceBundle>,
        progress: &StepProgressSnapshot,
        summary: Option<&str>,
    ) -> Option<StepEvidenceBundle> {
        let mut bundle = existing.cloned().unwrap_or_default();
        let merged_artifacts = Self::normalize_unique_paths(
            bundle
                .artifact_paths
                .iter()
                .cloned()
                .chain(progress.artifact_paths.iter().cloned()),
        );
        if merged_artifacts.is_empty()
            && progress.required_artifact_paths.is_empty()
            && progress.planning_evidence_paths.is_empty()
            && progress.quality_evidence_paths.is_empty()
            && progress.runtime_evidence_paths.is_empty()
            && progress.deployment_evidence_paths.is_empty()
            && progress.review_evidence_paths.is_empty()
            && progress.risk_evidence_paths.is_empty()
            && summary.map(str::trim).unwrap_or_default().is_empty()
        {
            return existing.cloned();
        }

        bundle.artifact_paths = merged_artifacts;
        bundle.required_artifact_paths = Self::normalize_unique_paths(
            bundle
                .required_artifact_paths
                .iter()
                .cloned()
                .chain(progress.required_artifact_paths.iter().cloned()),
        );
        bundle.planning_evidence_paths = Self::normalize_unique_paths(
            bundle
                .planning_evidence_paths
                .iter()
                .cloned()
                .chain(progress.planning_evidence_paths.iter().cloned()),
        );
        bundle.planning_signals = Self::infer_signal_keys(
            &bundle.planning_evidence_paths,
            &[
                ("mission-plan", "mission_plan_evidence"),
                ("/plan/", "plan_evidence"),
                ("outline", "outline_evidence"),
                ("notes", "notes_evidence"),
                ("research", "research_evidence"),
                ("analysis", "analysis_evidence"),
                ("workspace-overview", "workspace_overview_evidence"),
            ],
        );
        bundle.quality_evidence_paths = Self::normalize_unique_paths(
            bundle
                .quality_evidence_paths
                .iter()
                .cloned()
                .chain(progress.quality_evidence_paths.iter().cloned()),
        );
        bundle.quality_signals = Self::infer_signal_keys(
            &bundle.quality_evidence_paths,
            &[
                ("build.log", "build_evidence"),
                ("lint.log", "lint_evidence"),
                ("test.log", "test_evidence"),
                ("smoke.log", "smoke_evidence"),
                ("typecheck.log", "typecheck_evidence"),
                ("commands-run", "commands_recorded"),
                ("quality-skip-reasons", "quality_skip_reasons"),
            ],
        );
        bundle.runtime_evidence_paths = Self::normalize_unique_paths(
            bundle
                .runtime_evidence_paths
                .iter()
                .cloned()
                .chain(progress.runtime_evidence_paths.iter().cloned()),
        );
        bundle.runtime_signals = Self::infer_signal_keys(
            &bundle.runtime_evidence_paths,
            &[
                ("health.", "health_check_evidence"),
                ("report-meta", "report_meta_evidence"),
                ("report.http", "report_http_evidence"),
                ("slides.http", "slides_http_evidence"),
                ("smoke-server", "smoke_server_evidence"),
            ],
        );
        bundle.deployment_evidence_paths = Self::normalize_unique_paths(
            bundle
                .deployment_evidence_paths
                .iter()
                .cloned()
                .chain(progress.deployment_evidence_paths.iter().cloned()),
        );
        bundle.deployment_signals = Self::infer_signal_keys(
            &bundle.deployment_evidence_paths,
            &[
                ("pm2", "pm2_evidence"),
                ("ecosystem.config", "pm2_config_evidence"),
                ("deployment", "deployment_evidence"),
                ("verification", "deployment_verification_evidence"),
                ("deploy", "deploy_evidence"),
            ],
        );
        bundle.review_evidence_paths = Self::normalize_unique_paths(
            bundle
                .review_evidence_paths
                .iter()
                .cloned()
                .chain(progress.review_evidence_paths.iter().cloned()),
        );
        bundle.review_signals = Self::infer_signal_keys(
            &bundle.review_evidence_paths,
            &[
                ("code-review", "code_review_evidence"),
                ("review", "review_evidence"),
            ],
        );
        bundle.risk_evidence_paths = Self::normalize_unique_paths(
            bundle
                .risk_evidence_paths
                .iter()
                .cloned()
                .chain(progress.risk_evidence_paths.iter().cloned()),
        );
        bundle.risk_signals = Self::infer_signal_keys(
            &bundle.risk_evidence_paths,
            &[
                ("risk", "risk_evidence"),
                ("blocker", "blocker_evidence"),
                ("limitation", "limitation_evidence"),
                ("warning", "warning_evidence"),
                ("known-issues", "known_issues_evidence"),
                ("quality-skip-reasons", "quality_skip_reasons"),
                ("gap", "gap_evidence"),
            ],
        );
        if let Some(text) = summary.map(str::trim).filter(|text| !text.is_empty()) {
            bundle.latest_summary = Some(text.to_string());
        }
        bundle.updated_at = Some(mongodb::bson::DateTime::now());
        Some(bundle)
    }

    fn append_progress_events(
        existing: &[StepProgressEvent],
        mut new_events: Vec<StepProgressEvent>,
    ) -> Vec<StepProgressEvent> {
        if new_events.is_empty() {
            return existing.to_vec();
        }
        let mut merged = existing.to_vec();
        merged.append(&mut new_events);
        if merged.len() > MAX_STEP_PROGRESS_EVENTS {
            let drain_len = merged.len() - MAX_STEP_PROGRESS_EVENTS;
            merged.drain(0..drain_len);
        }
        merged
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

    #[cfg(not(windows))]
    fn is_known_system_binary_path(source: &Path) -> bool {
        let normalized = source.to_string_lossy().replace('\\', "/");
        [
            "/bin/",
            "/sbin/",
            "/usr/bin/",
            "/usr/sbin/",
            "/usr/local/bin/",
            "/usr/local/sbin/",
        ]
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
    }

    #[cfg(windows)]
    fn is_known_system_binary_path(_source: &Path) -> bool {
        false
    }

    fn should_ignore_external_output_candidate(source: &Path, is_file: bool) -> bool {
        if !is_file {
            return true;
        }
        Self::is_known_system_binary_path(source)
    }

    fn should_ignore_external_output_access_error(error: &std::io::Error) -> bool {
        matches!(error.kind(), std::io::ErrorKind::NotFound)
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
                    if Self::should_ignore_external_output_access_error(&e) {
                        tracing::debug!(
                            "Mission {} step {} ignoring missing external output candidate: {}",
                            mission_id,
                            step_index,
                            external
                        );
                        continue;
                    }
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
            if Self::should_ignore_external_output_candidate(&source, metadata.is_file()) {
                tracing::debug!(
                    "Mission {} step {} ignoring non-artifact external path candidate: {}",
                    mission_id,
                    step_index,
                    external
                );
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

        let fallback_summary = Self::render_mission_summary_fallback(&step_summaries);
        let prompt = Self::render_mission_summary_prompt(&step_summaries);

        let session_exists = self
            .agent_service
            .get_session(session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .is_some();

        if !session_exists {
            if let Err(e) = self
                .agent_service
                .set_mission_final_summary(mission_id, &fallback_summary)
                .await
            {
                tracing::warn!(
                    "Failed to save fallback mission {} final summary: {}",
                    mission_id,
                    e
                );
            }
            return Ok(());
        }

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
                None,
            )
            .await
        {
            tracing::warn!("Mission {} summary bridge failed: {}", mission_id, e);
            if let Err(save_err) = self
                .agent_service
                .set_mission_final_summary(mission_id, &fallback_summary)
                .await
            {
                tracing::warn!(
                    "Failed to save fallback mission {} final summary after bridge error: {}",
                    mission_id,
                    save_err
                );
            }
            return Ok(());
        }

        let final_summary = self
            .extract_step_summary(session_id)
            .await
            .filter(|summary| !summary.trim().is_empty())
            .unwrap_or(fallback_summary);
        if let Err(e) = self
            .agent_service
            .set_mission_final_summary(mission_id, &final_summary)
            .await
        {
            tracing::warn!("Failed to save mission {} final summary: {}", mission_id, e);
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

    fn render_mission_summary_fallback(step_summaries: &str) -> String {
        format!(
            "Mission execution completed.\n\n## Step Execution Results\n{}\n\nProvide this as the final mission summary baseline.",
            step_summaries
        )
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

    async fn execute_replan_in_isolated_session(
        &self,
        mission: &MissionDoc,
        agent_id: &str,
        mission_id: &str,
        prompt: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<String> {
        let temp_session = self
            .agent_service
            .create_chat_session(
                &mission.team_id,
                agent_id,
                &mission.creator_id,
                mission.attached_document_ids.clone(),
                None,
                None,
                None,
                None,
                Self::resolve_execution_runtime(mission).session_max_turns,
                None,
                None,
                false,
                false,
                None,
                Some("system".to_string()),
                Some(mission_id.to_string()),
                Some(true),
            )
            .await
            .map_err(|e| anyhow!("Failed to create isolated replan session: {}", e))?;
        let temp_session_id = temp_session.session_id.clone();
        let silent_broadcaster = Arc::new(SilentEventBroadcaster);

        let exec_result = runtime::execute_via_bridge(
            &self.db,
            &self.agent_service,
            &self.internal_task_manager,
            &silent_broadcaster,
            &temp_session_id,
            agent_id,
            &temp_session_id,
            prompt,
            cancel_token,
            workspace_path,
            None,
            None,
            None,
        )
        .await;

        let response = self
            .agent_service
            .get_session(&temp_session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .and_then(|session| runtime::extract_last_assistant_text(&session.messages_json))
            .unwrap_or_default();

        match self
            .agent_service
            .delete_session_if_idle(&temp_session_id)
            .await
        {
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    "Failed to delete isolated replan session {} for mission {}: {}",
                    temp_session_id,
                    mission_id,
                    e
                );
            }
        }

        exec_result?;
        if response.trim().is_empty() {
            return Err(anyhow!(
                "Mission {} replan returned empty assistant output",
                mission_id
            ));
        }
        Ok(response)
    }

    async fn collect_step_progress_snapshot(
        &self,
        session_id: &str,
        messages_before: usize,
        tokens_before: i32,
        workspace_path: Option<&str>,
        workspace_before: Option<&runtime::WorkspaceSnapshot>,
        required_artifacts: &[String],
    ) -> StepProgressSnapshot {
        let tokens_after = self.get_session_total_tokens(session_id).await;
        let (message_delta, tool_call_count, has_output_summary) =
            match self.agent_service.get_session(session_id).await {
                Ok(Some(session)) => {
                    let message_count = runtime::count_session_messages(&session.messages_json);
                    let tool_call_count =
                        runtime::extract_tool_calls_since(&session.messages_json, messages_before)
                            .len();
                    let has_output_summary =
                        runtime::extract_last_assistant_text(&session.messages_json)
                            .map(|text| !text.trim().is_empty())
                            .unwrap_or(false);
                    (
                        message_count.saturating_sub(messages_before),
                        tool_call_count,
                        has_output_summary,
                    )
                }
                _ => (0, 0, false),
            };

        let mut artifact_count = 0usize;
        let mut required_artifact_hits = 0usize;
        let mut artifact_paths = Vec::new();
        let mut required_artifact_paths = Vec::new();
        if let Some(workspace_root) = workspace_path {
            if let Ok(artifacts) =
                runtime::scan_workspace_artifacts(workspace_root, workspace_before)
            {
                artifact_paths = artifacts
                    .into_iter()
                    .filter(|item| !runtime::is_low_signal_artifact_path(&item.relative_path))
                    .map(|item| item.relative_path)
                    .collect();
                artifact_count = artifact_paths.len();
            }
            required_artifact_paths = required_artifacts
                .iter()
                .filter(|relative| Path::new(workspace_root).join(relative).exists())
                .cloned()
                .collect();
            required_artifact_hits = required_artifact_paths.len();
        }
        let mut evidence_bundle = Self::classify_step_evidence_paths(&artifact_paths);
        evidence_bundle.required_artifact_paths =
            Self::normalize_unique_paths(required_artifact_paths);

        StepProgressSnapshot {
            message_delta,
            token_delta: (tokens_after - tokens_before).max(0),
            tool_call_count,
            artifact_count,
            required_artifact_hits,
            has_output_summary,
            artifact_paths: evidence_bundle.artifact_paths,
            required_artifact_paths: evidence_bundle.required_artifact_paths,
            planning_evidence_paths: evidence_bundle.planning_evidence_paths,
            quality_evidence_paths: evidence_bundle.quality_evidence_paths,
            runtime_evidence_paths: evidence_bundle.runtime_evidence_paths,
            deployment_evidence_paths: evidence_bundle.deployment_evidence_paths,
            review_evidence_paths: evidence_bundle.review_evidence_paths,
            risk_evidence_paths: evidence_bundle.risk_evidence_paths,
        }
    }

    async fn maybe_generate_supervisor_guidance(
        &self,
        mission: &MissionDoc,
        agent_id: &str,
        mission_id: &str,
        step: &MissionStep,
        failure_kind: Option<StepFailureKind>,
        failure_message: &str,
        progress: &StepProgressSnapshot,
        previous_output: Option<&str>,
        recent_tool_calls: &[runtime::RetryPlaybookToolCall],
        repeated_failure_streak: u32,
        repeated_failed_tool: Option<&str>,
        workspace_path: Option<&str>,
    ) -> Option<SupervisorGuidance> {
        let prompt = Self::build_supervisor_hint_prompt(
            mission,
            step,
            failure_kind,
            failure_message,
            progress,
            previous_output,
            recent_tool_calls,
            repeated_failure_streak,
            repeated_failed_tool,
            workspace_path,
        );
        let response = self
            .execute_replan_in_isolated_session(
                mission,
                agent_id,
                mission_id,
                &prompt,
                CancellationToken::new(),
                workspace_path,
            )
            .await
            .ok()?;
        Self::parse_supervisor_guidance_response(&response)
    }

    fn build_progress_events(
        state: StepSupervisorState,
        progress: &StepProgressSnapshot,
        current_blocker: Option<&str>,
        supervisor_guidance: Option<&SupervisorGuidance>,
    ) -> Vec<StepProgressEvent> {
        let mut events = Vec::new();
        let now = mongodb::bson::DateTime::now();

        if progress.has_activity() {
            events.push(StepProgressEvent {
                kind: StepProgressEventKind::ActivityObserved,
                message: format!("step activity observed: {}", progress.summary()),
                source: Some(StepProgressEventSource::Executor),
                layer: Some(StepProgressLayer::Activity),
                semantic_tags: Self::semantic_tags(&["activity", "heartbeat"]),
                ai_annotation: None,
                paths: Vec::new(),
                checks: Vec::new(),
                score_delta: Some(progress.progress_score()),
                recorded_at: Some(now),
            });
        }
        if progress.has_work_progress() {
            let mut work_paths = Self::normalize_unique_paths(
                progress
                    .planning_evidence_paths
                    .iter()
                    .cloned()
                    .chain(progress.risk_evidence_paths.iter().cloned()),
            );
            if work_paths.len() > 6 {
                work_paths.truncate(6);
            }
            events.push(StepProgressEvent {
                kind: StepProgressEventKind::WorkProgressObserved,
                message: format!(
                    "work progress observed: tools={}, planning_evidence={}, risk_evidence={}, summary={}",
                    progress.tool_call_count,
                    progress.planning_evidence_paths.len(),
                    progress.risk_evidence_paths.len(),
                    progress.has_output_summary
                ),
                source: Some(StepProgressEventSource::Executor),
                layer: Some(StepProgressLayer::WorkProgress),
                semantic_tags: Self::semantic_tags(&["work_progress", "intermediate_progress"]),
                ai_annotation: None,
                paths: work_paths,
                checks: Vec::new(),
                score_delta: Some((progress.planning_evidence_paths.len().min(2) as i32) + 1),
                recorded_at: Some(now),
            });
        }
        if progress.has_output_summary {
            events.push(StepProgressEvent {
                kind: StepProgressEventKind::SummaryObserved,
                message: "step output summary updated".to_string(),
                source: Some(StepProgressEventSource::Executor),
                layer: Some(StepProgressLayer::WorkProgress),
                semantic_tags: Self::semantic_tags(&["summary", "work_progress"]),
                ai_annotation: None,
                paths: Vec::new(),
                checks: Vec::new(),
                score_delta: Some(2),
                recorded_at: Some(now),
            });
        }
        if !progress.artifact_paths.is_empty() {
            events.push(StepProgressEvent {
                kind: StepProgressEventKind::ArtifactObserved,
                message: format!(
                    "observed {} changed artifacts",
                    progress.artifact_paths.len()
                ),
                source: Some(StepProgressEventSource::Workspace),
                layer: Some(StepProgressLayer::DeliveryProgress),
                semantic_tags: Self::semantic_tags(&["artifact", "delivery_progress"]),
                ai_annotation: None,
                paths: progress.artifact_paths.clone(),
                checks: Vec::new(),
                score_delta: Some((progress.artifact_paths.len().min(3) * 2) as i32),
                recorded_at: Some(now),
            });
        }
        if !progress.required_artifact_paths.is_empty() {
            events.push(StepProgressEvent {
                kind: StepProgressEventKind::RequiredArtifactSatisfied,
                message: format!(
                    "required artifacts available: {}",
                    progress.required_artifact_paths.join(", ")
                ),
                source: Some(StepProgressEventSource::Verifier),
                layer: Some(StepProgressLayer::DeliveryProgress),
                semantic_tags: Self::semantic_tags(&["contract_artifact", "delivery_progress"]),
                ai_annotation: None,
                paths: progress.required_artifact_paths.clone(),
                checks: Vec::new(),
                score_delta: Some((progress.required_artifact_paths.len().min(2) * 2) as i32),
                recorded_at: Some(now),
            });
        }
        let evidence_groups = [
            (
                StepProgressEventKind::PlanningEvidenceObserved,
                "planning evidence observed",
                progress.planning_evidence_paths.as_slice(),
                StepProgressLayer::WorkProgress,
                "planning_evidence",
            ),
            (
                StepProgressEventKind::QualityEvidenceObserved,
                "quality evidence observed",
                progress.quality_evidence_paths.as_slice(),
                StepProgressLayer::DeliveryProgress,
                "quality_evidence",
            ),
            (
                StepProgressEventKind::RuntimeEvidenceObserved,
                "runtime evidence observed",
                progress.runtime_evidence_paths.as_slice(),
                StepProgressLayer::DeliveryProgress,
                "runtime_evidence",
            ),
            (
                StepProgressEventKind::DeploymentEvidenceObserved,
                "deployment evidence observed",
                progress.deployment_evidence_paths.as_slice(),
                StepProgressLayer::DeliveryProgress,
                "deployment_evidence",
            ),
            (
                StepProgressEventKind::ReviewEvidenceObserved,
                "review evidence observed",
                progress.review_evidence_paths.as_slice(),
                StepProgressLayer::DeliveryProgress,
                "review_evidence",
            ),
            (
                StepProgressEventKind::RiskEvidenceObserved,
                "risk evidence observed",
                progress.risk_evidence_paths.as_slice(),
                StepProgressLayer::WorkProgress,
                "risk_evidence",
            ),
        ];
        for (kind, label, paths, layer, semantic_tag) in evidence_groups {
            if paths.is_empty() {
                continue;
            }
            events.push(StepProgressEvent {
                kind,
                message: format!("{}: {}", label, paths.join(", ")),
                source: Some(StepProgressEventSource::Workspace),
                layer: Some(layer),
                semantic_tags: Self::semantic_tags(&[semantic_tag]),
                ai_annotation: None,
                paths: paths.to_vec(),
                checks: Vec::new(),
                score_delta: Some(paths.len().min(2) as i32),
                recorded_at: Some(now),
            });
        }
        if matches!(
            state,
            StepSupervisorState::Drifting | StepSupervisorState::Stalled
        ) || supervisor_guidance
            .map(|guidance| guidance.resume_hint.trim())
            .filter(|hint| !hint.is_empty())
            .is_some()
        {
            let mut message = format!("supervisor state -> {:?}", state).to_ascii_lowercase();
            if let Some(blocker) = current_blocker.map(str::trim).filter(|b| !b.is_empty()) {
                message.push_str(&format!("; blocker={}", blocker));
            }
            if let Some(observed) = supervisor_guidance
                .filter(|guidance| !guidance.observed_evidence.is_empty())
                .map(|guidance| Self::compact_list_for_prompt(&guidance.observed_evidence, 3, 72))
            {
                message.push_str(&format!("; observed={}", observed));
            }
            events.push(StepProgressEvent {
                kind: StepProgressEventKind::SupervisorIntervention,
                message,
                source: Some(
                    if supervisor_guidance
                        .map(|guidance| guidance.resume_hint.trim())
                        .filter(|hint| !hint.is_empty())
                        .is_some()
                    {
                        StepProgressEventSource::AiSupervisor
                    } else {
                        StepProgressEventSource::Supervisor
                    },
                ),
                layer: Some(StepProgressLayer::Recovery),
                semantic_tags: Self::normalize_unique_paths(
                    Self::semantic_tags(&["supervisor_intervention", "recovery"])
                        .into_iter()
                        .chain(
                            supervisor_guidance
                                .into_iter()
                                .flat_map(|guidance| guidance.semantic_tags.iter().cloned()),
                        ),
                ),
                ai_annotation: supervisor_guidance
                    .filter(|guidance| {
                        !(guidance.diagnosis.trim().is_empty()
                            && guidance.resume_hint.trim().is_empty()
                            && guidance.observed_evidence.is_empty())
                    })
                    .map(|guidance| {
                        let mut lines = vec![
                            format!("diagnosis: {}", guidance.diagnosis.trim()),
                            format!("resume_hint: {}", guidance.resume_hint.trim()),
                        ];
                        if !guidance.observed_evidence.is_empty() {
                            lines.push(format!(
                                "observed_evidence: {}",
                                Self::compact_list_for_prompt(&guidance.observed_evidence, 3, 72)
                            ));
                        }
                        lines.join("\n")
                    }),
                paths: Vec::new(),
                checks: Vec::new(),
                score_delta: None,
                recorded_at: Some(now),
            });
        }

        events
    }

    async fn update_step_supervision(
        &self,
        mission_id: &str,
        step: &mut MissionStep,
        step_index: u32,
        state: StepSupervisorState,
        progress: &StepProgressSnapshot,
        current_blocker: Option<&str>,
        supervisor_guidance: Option<&SupervisorGuidance>,
    ) {
        let now = mongodb::bson::DateTime::now();
        let last_activity_at = progress.has_activity().then_some(now);
        let last_progress_at = progress.has_progress().then_some(now);
        let increment_stall_count = matches!(
            state,
            StepSupervisorState::Drifting | StepSupervisorState::Stalled
        );
        let next_stall_count = if increment_stall_count {
            step.stall_count.saturating_add(1)
        } else {
            0
        };
        let supervisor_hint = supervisor_guidance.map(|guidance| guidance.resume_hint.as_str());
        let evidence_bundle =
            Self::merge_step_evidence_bundle(step.evidence_bundle.as_ref(), progress, None);
        let progress_events = Self::append_progress_events(
            &step.recent_progress_events,
            Self::build_progress_events(
                state.clone(),
                progress,
                current_blocker,
                supervisor_guidance,
            ),
        );
        if let Err(err) = self
            .agent_service
            .set_step_supervision(
                mission_id,
                step_index,
                state.clone(),
                last_activity_at,
                last_progress_at,
                Some(progress.progress_score()),
                current_blocker,
                supervisor_hint,
                increment_stall_count,
                Some(next_stall_count),
            )
            .await
        {
            tracing::warn!(
                "Failed to persist supervision state for mission {} step {}: {}",
                mission_id,
                step_index,
                err
            );
            return;
        }
        if let Err(err) = self
            .agent_service
            .set_step_observability(
                mission_id,
                step_index,
                &progress_events,
                evidence_bundle.as_ref(),
            )
            .await
        {
            tracing::warn!(
                "Failed to persist observability for mission {} step {}: {}",
                mission_id,
                step_index,
                err
            );
        } else {
            step.supervisor_state = Some(state.clone());
            step.last_activity_at = last_activity_at;
            step.last_progress_at = last_progress_at;
            step.progress_score = Some(progress.progress_score());
            step.current_blocker = current_blocker.map(str::to_string);
            step.last_supervisor_hint = supervisor_hint.map(str::to_string);
            step.stall_count = next_stall_count;
            step.recent_progress_events = progress_events;
            step.evidence_bundle = evidence_bundle;
        }

        let blocker = current_blocker
            .unwrap_or_default()
            .replace('"', r#"\""#)
            .replace('\n', " ");
        let hint = supervisor_hint
            .unwrap_or_default()
            .replace('"', r#"\""#)
            .replace('\n', " ");
        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::Status {
                    status: format!(
                        r#"{{"type":"step_supervision","step_index":{},"state":"{}","progress_score":{},"blocker":"{}","hint":"{}","artifacts":{},"required_hits":{},"quality_evidence":{},"runtime_evidence":{},"action":"{}"}}"#,
                        step_index,
                        match state {
                            StepSupervisorState::Healthy => "healthy",
                            StepSupervisorState::Busy => "busy",
                            StepSupervisorState::Drifting => "drifting",
                            StepSupervisorState::Stalled => "stalled",
                        },
                        progress.progress_score(),
                        blocker,
                        hint,
                        progress.artifact_paths.len(),
                        progress.required_artifact_paths.len(),
                        progress.quality_evidence_paths.len(),
                        progress.runtime_evidence_paths.len(),
                        match state {
                            StepSupervisorState::Healthy | StepSupervisorState::Busy => "continue",
                            StepSupervisorState::Drifting => "nudge",
                            StepSupervisorState::Stalled => "recover",
                        }
                    ),
                },
            )
            .await;
    }

    async fn create_isolated_step_session(
        &self,
        mission: &MissionDoc,
        agent_id: &str,
        mission_id: &str,
        session_max_turns: Option<i32>,
    ) -> Result<String> {
        let temp_session = self
            .agent_service
            .create_chat_session(
                &mission.team_id,
                agent_id,
                &mission.creator_id,
                mission.attached_document_ids.clone(),
                None,
                None,
                None,
                None,
                session_max_turns,
                None,
                None,
                false,
                false,
                None,
                Some("mission".to_string()),
                Some(mission_id.to_string()),
                Some(true),
            )
            .await
            .map_err(|e| anyhow!("Failed to create isolated step session: {}", e))?;
        Ok(temp_session.session_id)
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
        _session_id: &str,
        completed_steps: &[MissionStep],
        remaining_steps: &[MissionStep],
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<Option<Vec<MissionStep>>> {
        // Build the evaluation prompt
        let prompt = Self::build_replan_prompt(completed_steps, remaining_steps);
        let mission_doc = self
            .agent_service
            .get_mission(mission_id)
            .await
            .ok()
            .flatten()
            .ok_or_else(|| anyhow!("Mission not found"))?;
        let mission_step_max_retries = mission_doc.step_max_retries;
        let mission_step_timeout_seconds = mission_doc.step_timeout_seconds;
        let response = self
            .execute_replan_in_isolated_session(
                &mission_doc,
                agent_id,
                mission_id,
                &prompt,
                cancel_token,
                workspace_path,
            )
            .await?;

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
            step.supervisor_state = None;
            step.last_activity_at = None;
            step.last_progress_at = None;
            step.progress_score = None;
            step.current_blocker = None;
            step.last_supervisor_hint = None;
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

#[cfg(test)]
mod tests {
    use super::{MissionExecutor, StepFailureKind, StepProgressSnapshot, SupervisorGuidance};
    use crate::agent::mission_mongo::{
        MissionStep, RuntimeContract, RuntimeContractVerification, StepEvidenceBundle,
        StepProgressEvent, StepProgressEventKind, StepProgressEventSource, StepProgressLayer,
        StepStatus, StepSupervisorState,
    };
    use crate::agent::runtime;

    fn sample_step() -> MissionStep {
        MissionStep {
            index: 0,
            title: "Step 1".to_string(),
            description: "Do the thing".to_string(),
            status: StepStatus::Pending,
            is_checkpoint: false,
            approved_by: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            supervisor_state: None,
            last_activity_at: None,
            last_progress_at: None,
            progress_score: None,
            current_blocker: None,
            last_supervisor_hint: None,
            stall_count: 0,
            recent_progress_events: vec![],
            evidence_bundle: None,
            tokens_used: 0,
            output_summary: None,
            retry_count: 0,
            max_retries: 2,
            timeout_seconds: None,
            required_artifacts: vec!["reports/out.md".to_string()],
            completion_checks: vec!["exists:reports/out.md".to_string()],
            runtime_contract: None,
            contract_verification: None,
            use_subagent: false,
            tool_calls: vec![],
        }
    }

    fn sample_contract() -> runtime::MissionPreflightContract {
        runtime::MissionPreflightContract {
            required_artifacts: vec!["reports/out.md".to_string()],
            completion_checks: vec!["exists:reports/out.md".to_string()],
            no_artifact_reason: None,
        }
    }

    #[test]
    fn resolves_retry_contract_from_persisted_runtime_contract() {
        let step = sample_step();
        let persisted = sample_contract();

        let resolved = MissionExecutor::resolve_retry_preflight_contract(
            None,
            Some(&persisted),
            &step,
            Some("Step wrote files outside workspace: /opt/agime"),
            None,
        )
        .expect("persisted contract should be reused");

        assert_eq!(resolved.required_artifacts, persisted.required_artifacts);
        assert_eq!(resolved.completion_checks, persisted.completion_checks);
    }

    #[test]
    fn forces_fresh_preflight_when_contract_verification_failed() {
        let mut step = sample_step();
        step.runtime_contract = Some(RuntimeContract {
            required_artifacts: vec!["reports/out.md".to_string()],
            completion_checks: vec!["exists:reports/out.md".to_string()],
            no_artifact_reason: None,
            source: Some("mission_preflight__preflight".to_string()),
            captured_at: None,
        });
        step.contract_verification = Some(RuntimeContractVerification {
            tool_called: true,
            status: Some("fail".to_string()),
            gate_mode: Some("soft".to_string()),
            accepted: Some(false),
            reason: Some("`mission_preflight__verify_contract` returned fail".to_string()),
            checked_at: None,
        });

        let persisted = MissionExecutor::runtime_contract_doc_to_preflight(
            step.runtime_contract.as_ref().unwrap(),
        );
        let resolved = MissionExecutor::resolve_retry_preflight_contract(
            None,
            Some(&persisted),
            &step,
            Some("Step contract verification gate failed"),
            None,
        );

        assert!(resolved.is_none());
    }

    #[test]
    fn forces_fresh_preflight_when_operator_feedback_present() {
        let step = sample_step();
        let persisted = sample_contract();

        let resolved = MissionExecutor::resolve_retry_preflight_contract(
            None,
            Some(&persisted),
            &step,
            None,
            Some("Please change the output format to pdf"),
        );

        assert!(resolved.is_none());
    }

    #[test]
    fn build_step_prompt_uses_dynamic_preflight_retry_fields() {
        let mut step = sample_step();
        step.retry_count = 2;

        let prompt = MissionExecutor::build_step_prompt(
            "Ship it",
            0,
            &step,
            3,
            &[],
            Some("/workspace"),
            None,
            3,
            Some("Step wrote files outside workspace: /opt/agime"),
        );

        assert!(prompt.contains("\"attempt\": 3"));
        assert!(
            prompt.contains("\"last_error\": \"Step wrote files outside workspace: /opt/agime\"")
        );
    }

    #[test]
    fn build_step_prompt_adds_soft_quality_guidance_for_engineering_steps() {
        let mut step = sample_step();
        step.title = "Deploy service + runtime verification".to_string();
        step.description =
            "Deploy the API with pm2, verify /health, and capture concise deployment evidence."
                .to_string();

        let prompt = MissionExecutor::build_step_prompt(
            "Ship service",
            5,
            &step,
            7,
            &[],
            None,
            None,
            1,
            None,
        );

        assert!(prompt.contains("Delivery Quality Guidance"));
        assert!(prompt.contains("Recommended evidence note path"));
        assert!(prompt.contains("reports/final/quality/step-6-quality.md"));
        assert!(prompt.contains("Not a Hard Gate"));
    }

    #[test]
    fn build_step_prompt_skips_quality_guidance_for_generic_non_engineering_steps() {
        let mut step = sample_step();
        step.title = "Rename project codename".to_string();
        step.description = "Rename the project codename in the current note.".to_string();

        let prompt = MissionExecutor::build_step_prompt(
            "Rename note",
            0,
            &step,
            1,
            &[],
            None,
            None,
            1,
            None,
        );

        assert!(!prompt.contains("Delivery Quality Guidance"));
    }

    #[test]
    fn reuses_persisted_verify_contract_state_when_retry_has_no_new_verify_call() {
        let step = sample_step();

        let resolved = MissionExecutor::resolve_retry_verify_contract_state(
            false,
            None,
            Some((true, Some(true))),
            &step,
            Some("Step wrote files outside workspace: /opt/agime"),
            None,
        );

        assert_eq!(resolved, (true, Some(true)));
    }

    #[test]
    fn ignores_directory_like_external_output_candidates() {
        assert!(MissionExecutor::should_ignore_external_output_candidate(
            std::path::Path::new("/opt/agime"),
            false,
        ));
    }

    #[cfg(not(windows))]
    #[test]
    fn ignores_system_binary_external_output_candidates() {
        assert!(MissionExecutor::should_ignore_external_output_candidate(
            std::path::Path::new("/usr/local/bin/pm2"),
            true,
        ));
    }

    #[test]
    fn keeps_regular_external_files_as_recoverable_candidates() {
        assert!(!MissionExecutor::should_ignore_external_output_candidate(
            std::path::Path::new("/opt/agime/reports/out.md"),
            true,
        ));
    }

    #[test]
    fn ignores_missing_external_output_access_errors() {
        let err = std::io::Error::from(std::io::ErrorKind::NotFound);

        assert!(MissionExecutor::should_ignore_external_output_access_error(
            &err
        ));
    }

    #[test]
    fn keeps_permission_denied_external_output_access_errors_blocking() {
        let err = std::io::Error::from(std::io::ErrorKind::PermissionDenied);

        assert!(!MissionExecutor::should_ignore_external_output_access_error(&err));
    }

    #[test]
    fn explicit_step_timeout_is_not_raised_to_complex_floor() {
        let mut step = sample_step();
        step.timeout_seconds = Some(180);

        let resolved = MissionExecutor::resolve_step_timeout(&step, None);

        assert_eq!(resolved.as_secs(), 180);
    }

    #[test]
    fn implicit_complex_step_timeout_still_uses_complex_floor() {
        let step = sample_step();

        let resolved = MissionExecutor::resolve_step_timeout(&step, None);

        assert_eq!(resolved.as_secs(), 1200);
    }

    #[test]
    fn classifies_workspace_guard_retry_failures() {
        let kind = MissionExecutor::classify_retry_failure(
            "Step wrote files outside workspace: /opt/agime",
            &[],
        );
        assert_eq!(kind, StepFailureKind::WorkspaceGuard);
    }

    #[test]
    fn classifies_tool_parameter_retry_failures() {
        let kind = MissionExecutor::classify_retry_failure(
            "Mcp error: failed to deserialize parameters: unknown variant `Shell/Bash script`",
            &[],
        );
        assert_eq!(kind, StepFailureKind::ToolParameterSchema);
    }

    #[test]
    fn classifies_missing_parent_directory_retry_failures() {
        let kind = MissionExecutor::classify_retry_failure(
            "Platform tool call failed: McpError(ErrorData { code: ErrorCode(-32603), message: \"Failed to write file: No such file or directory (os error 2)\", data: None })",
            &[],
        );
        assert_eq!(kind, StepFailureKind::MissingParentDirectory);
    }

    #[test]
    fn classifies_missing_summary_retry_failures() {
        let kind = MissionExecutor::classify_retry_failure(
            "Step completion validation failed: empty assistant output summary",
            &[],
        );
        assert_eq!(kind, StepFailureKind::MissingSummary);
    }

    #[test]
    fn classifies_repeated_tool_denied_retry_failures() {
        let kind = MissionExecutor::classify_retry_failure(
            "Repeated tool call denied: developer__text_editor",
            &[],
        );
        assert_eq!(kind, StepFailureKind::RepeatedToolDenied);
    }

    #[test]
    fn classifies_security_tool_blocked_retry_failures() {
        let kind = MissionExecutor::classify_retry_failure(
            "Security: blocked tool 'developer__shell' (confidence=0.85): command substitution found",
            &[],
        );
        assert_eq!(kind, StepFailureKind::SecurityToolBlocked);
    }

    #[test]
    fn builds_workspace_retry_instruction() {
        let step = sample_step();
        let instruction = MissionExecutor::build_retry_turn_instruction(
            StepFailureKind::WorkspaceGuard,
            &step,
            &StepProgressSnapshot::default(),
            0,
            None,
            Some("/workspace/root"),
        )
        .expect("workspace guard should yield instruction");

        assert!(instruction.contains("workspace-relative paths"));
        assert!(instruction.contains("/workspace/root"));
    }

    #[test]
    fn builds_missing_parent_directory_retry_instruction() {
        let step = sample_step();
        let instruction = MissionExecutor::build_retry_turn_instruction(
            StepFailureKind::MissingParentDirectory,
            &step,
            &StepProgressSnapshot::default(),
            0,
            None,
            Some("/workspace/root"),
        )
        .expect("missing parent dir should yield instruction");

        assert!(instruction.contains("create the parent directory"));
        assert!(instruction.contains("/workspace/root"));
    }

    #[test]
    fn builds_missing_summary_retry_instruction() {
        let step = sample_step();
        let instruction = MissionExecutor::build_retry_turn_instruction(
            StepFailureKind::MissingSummary,
            &step,
            &StepProgressSnapshot::default(),
            0,
            None,
            None,
        )
        .expect("missing summary should yield instruction");
        assert!(instruction.contains("concise completion summary"));
    }

    #[test]
    fn content_heavy_timeout_retry_instruction_demands_incremental_persistence() {
        let mut step = sample_step();
        step.title = "Produce structured document and presentation deliverables".to_string();
        step.description =
            "Create the final document deliverable plus supporting presentation assets through incremental content generation.".to_string();
        step.required_artifacts = vec![
            "output/source-data.json".to_string(),
            "output/final-deliverable.html".to_string(),
        ];
        let progress = StepProgressSnapshot {
            artifact_count: 1,
            has_output_summary: true,
            ..Default::default()
        };

        let instruction = MissionExecutor::build_retry_turn_instruction(
            StepFailureKind::Timeout,
            &step,
            &progress,
            2,
            Some("developer__shell"),
            Some("/workspace/root"),
        )
        .expect("timeout retry instruction should exist");

        assert!(instruction.contains("Do not restart the whole step"));
        assert!(instruction.contains("Persist a smaller intermediate result first"));
        assert!(instruction.contains("source-data.json"));
    }

    #[test]
    fn runtime_steps_prefer_runtime_completion_checks() {
        let mut step = sample_step();
        step.title = "Deploy service and verify API health".to_string();
        step.description = "Run on port 3002 and verify /health plus /api/search".to_string();

        assert!(MissionExecutor::step_should_prefer_runtime_completion_checks(&step));
    }

    #[test]
    fn documentation_steps_do_not_force_runtime_completion_checks() {
        let mut step = sample_step();
        step.title = "Write release notes".to_string();
        step.description = "Document the migration plan and changelog".to_string();

        assert!(!MissionExecutor::step_should_prefer_runtime_completion_checks(&step));
    }

    #[test]
    fn mission_summary_fallback_includes_step_results() {
        let summary = MissionExecutor::render_mission_summary_fallback(
            "- Step 1: Build app [completed] -> shipped\n- Step 2: Verify API [completed] -> passed\n",
        );

        assert!(summary.contains("Mission execution completed."));
        assert!(summary.contains("Step 1: Build app"));
        assert!(summary.contains("Step 2: Verify API"));
    }

    #[test]
    fn mission_level_timeout_is_a_floor_for_steps() {
        let mut step = sample_step();
        step.timeout_seconds = Some(120);

        let resolved = MissionExecutor::resolve_step_timeout(&step, Some(1200));

        assert_eq!(resolved.as_secs(), 1200);
    }

    #[test]
    fn mission_level_retry_limit_is_a_floor_for_steps() {
        let resolved = MissionExecutor::resolve_effective_step_max_retries(2, Some(3));
        assert_eq!(resolved, 3);
    }

    #[test]
    fn classifies_supervisor_state_from_progress_signals() {
        let drifting = MissionExecutor::classify_supervisor_state(
            None,
            0,
            Some(StepFailureKind::Timeout),
            &StepProgressSnapshot {
                message_delta: 2,
                token_delta: 24,
                tool_call_count: 0,
                artifact_count: 0,
                required_artifact_hits: 0,
                has_output_summary: false,
                artifact_paths: vec![],
                required_artifact_paths: vec![],
                planning_evidence_paths: vec![],
                quality_evidence_paths: vec![],
                runtime_evidence_paths: vec![],
                deployment_evidence_paths: vec![],
                review_evidence_paths: vec![],
                risk_evidence_paths: vec![],
            },
            0,
            None,
        );
        assert_eq!(drifting, StepSupervisorState::Drifting);

        let healthy = MissionExecutor::classify_supervisor_state(
            None,
            0,
            Some(StepFailureKind::Timeout),
            &StepProgressSnapshot {
                message_delta: 4,
                token_delta: 120,
                tool_call_count: 2,
                artifact_count: 1,
                required_artifact_hits: 0,
                has_output_summary: false,
                artifact_paths: vec!["reports/out.md".to_string()],
                required_artifact_paths: vec![],
                planning_evidence_paths: vec![],
                quality_evidence_paths: vec![],
                runtime_evidence_paths: vec![],
                deployment_evidence_paths: vec![],
                review_evidence_paths: vec![],
                risk_evidence_paths: vec![],
            },
            0,
            None,
        );
        assert_eq!(healthy, StepSupervisorState::Healthy);

        let research_busy = MissionExecutor::classify_supervisor_state(
            None,
            0,
            Some(StepFailureKind::Timeout),
            &StepProgressSnapshot {
                message_delta: 3,
                token_delta: 420,
                tool_call_count: 2,
                artifact_count: 0,
                required_artifact_hits: 0,
                has_output_summary: false,
                artifact_paths: vec![],
                required_artifact_paths: vec![],
                planning_evidence_paths: vec![],
                quality_evidence_paths: vec![],
                runtime_evidence_paths: vec![],
                deployment_evidence_paths: vec![],
                review_evidence_paths: vec![],
                risk_evidence_paths: vec![],
            },
            0,
            None,
        );
        assert_eq!(research_busy, StepSupervisorState::Busy);

        let drifting_without_history = MissionExecutor::classify_supervisor_state(
            None,
            0,
            Some(StepFailureKind::Timeout),
            &StepProgressSnapshot::default(),
            0,
            None,
        );
        assert_eq!(drifting_without_history, StepSupervisorState::Drifting);
    }

    #[test]
    fn repeated_failed_tool_escalates_supervisor_state() {
        let drifting = MissionExecutor::classify_supervisor_state(
            None,
            0,
            Some(StepFailureKind::ToolExecution),
            &StepProgressSnapshot {
                message_delta: 1,
                token_delta: 12,
                ..Default::default()
            },
            2,
            Some("developer__shell"),
        );
        assert_eq!(drifting, StepSupervisorState::Drifting);
    }

    #[test]
    fn merges_evidence_bundle_without_losing_existing_paths() {
        let existing = StepEvidenceBundle {
            artifact_paths: vec!["reports/final/report-data.json".to_string()],
            planning_evidence_paths: vec!["reports/final/plan/outline.md".to_string()],
            planning_signals: vec!["outline_evidence".to_string()],
            quality_evidence_paths: vec!["reports/final/quality/build.log".to_string()],
            quality_signals: vec!["build_evidence".to_string()],
            runtime_evidence_paths: vec![],
            runtime_signals: vec![],
            deployment_evidence_paths: vec![],
            deployment_signals: vec![],
            review_evidence_paths: vec![],
            review_signals: vec![],
            risk_evidence_paths: vec!["reports/final/quality/known-issues.md".to_string()],
            risk_signals: vec!["known_issues_evidence".to_string()],
            required_artifact_paths: vec![],
            latest_summary: Some("existing".to_string()),
            updated_at: None,
        };
        let progress = StepProgressSnapshot {
            artifact_paths: vec!["reports/final/report.html".to_string()],
            planning_evidence_paths: vec!["reports/final/research/notes.md".to_string()],
            quality_evidence_paths: vec![],
            runtime_evidence_paths: vec!["reports/final/runtime/health.json".to_string()],
            deployment_evidence_paths: vec!["reports/final/deployment.md".to_string()],
            review_evidence_paths: vec![],
            risk_evidence_paths: vec!["reports/final/quality/gaps.md".to_string()],
            ..Default::default()
        };

        let merged = MissionExecutor::merge_step_evidence_bundle(
            Some(&existing),
            &progress,
            Some("latest summary"),
        )
        .expect("bundle should exist");

        assert_eq!(merged.artifact_paths.len(), 2);
        assert!(merged
            .artifact_paths
            .contains(&"reports/final/report-data.json".to_string()));
        assert!(merged
            .runtime_evidence_paths
            .contains(&"reports/final/runtime/health.json".to_string()));
        assert!(merged
            .planning_evidence_paths
            .contains(&"reports/final/plan/outline.md".to_string()));
        assert!(merged
            .planning_evidence_paths
            .contains(&"reports/final/research/notes.md".to_string()));
        assert!(merged
            .planning_signals
            .contains(&"outline_evidence".to_string()));
        assert!(merged
            .planning_signals
            .contains(&"research_evidence".to_string()));
        assert!(merged
            .risk_evidence_paths
            .contains(&"reports/final/quality/known-issues.md".to_string()));
        assert!(merged
            .risk_evidence_paths
            .contains(&"reports/final/quality/gaps.md".to_string()));
        assert!(merged
            .risk_signals
            .contains(&"known_issues_evidence".to_string()));
        assert!(merged.risk_signals.contains(&"gap_evidence".to_string()));
        assert!(merged
            .quality_signals
            .contains(&"build_evidence".to_string()));
        assert_eq!(merged.latest_summary.as_deref(), Some("latest summary"));
    }

    #[test]
    fn planning_evidence_counts_as_work_progress() {
        let research_busy = MissionExecutor::classify_supervisor_state(
            None,
            0,
            Some(StepFailureKind::Timeout),
            &StepProgressSnapshot {
                message_delta: 1,
                token_delta: 48,
                tool_call_count: 1,
                artifact_count: 0,
                required_artifact_hits: 0,
                has_output_summary: false,
                artifact_paths: vec![],
                required_artifact_paths: vec![],
                planning_evidence_paths: vec!["reports/final/research/notes.md".to_string()],
                quality_evidence_paths: vec![],
                runtime_evidence_paths: vec![],
                deployment_evidence_paths: vec![],
                review_evidence_paths: vec![],
                risk_evidence_paths: vec![],
            },
            0,
            None,
        );

        assert_eq!(research_busy, StepSupervisorState::Busy);
    }

    #[test]
    fn supervisor_decision_recovery_requires_no_activity_and_repeated_failures() {
        let decision = MissionExecutor::decide_supervisor_response(
            None,
            0,
            Some(StepFailureKind::Timeout),
            &StepProgressSnapshot::default(),
            2,
            Some("developer__shell"),
        );
        assert_eq!(decision.state, StepSupervisorState::Stalled);
        assert!(decision.should_generate_hint);
    }

    #[test]
    fn busy_supervisor_state_does_not_request_ai_hint() {
        let decision = MissionExecutor::decide_supervisor_response(
            None,
            0,
            Some(StepFailureKind::Timeout),
            &StepProgressSnapshot {
                message_delta: 2,
                token_delta: 320,
                tool_call_count: 2,
                planning_evidence_paths: vec!["reports/final/research/notes.md".to_string()],
                ..Default::default()
            },
            0,
            None,
        );

        assert_eq!(decision.state, StepSupervisorState::Busy);
        assert!(!decision.should_generate_hint);
    }

    #[test]
    fn drifting_supervisor_state_requests_ai_hint() {
        let decision = MissionExecutor::decide_supervisor_response(
            None,
            0,
            Some(StepFailureKind::Timeout),
            &StepProgressSnapshot {
                message_delta: 1,
                token_delta: 8,
                ..Default::default()
            },
            0,
            None,
        );

        assert_eq!(decision.state, StepSupervisorState::Drifting);
        assert!(decision.should_generate_hint);
    }

    #[test]
    fn busy_state_does_not_immediately_drift_on_first_weak_timeout_signal() {
        let decision = MissionExecutor::decide_supervisor_response(
            Some(&StepSupervisorState::Busy),
            0,
            Some(StepFailureKind::Timeout),
            &StepProgressSnapshot {
                message_delta: 1,
                token_delta: 16,
                tool_call_count: 0,
                ..Default::default()
            },
            0,
            None,
        );

        assert_eq!(decision.state, StepSupervisorState::Busy);
    }

    #[test]
    fn drifting_state_escalates_to_stalled_after_consecutive_no_activity_windows() {
        let decision = MissionExecutor::decide_supervisor_response(
            Some(&StepSupervisorState::Drifting),
            1,
            Some(StepFailureKind::Timeout),
            &StepProgressSnapshot::default(),
            0,
            None,
        );

        assert_eq!(decision.state, StepSupervisorState::Stalled);
    }

    #[test]
    fn detects_repeated_failed_tool_from_recent_calls() {
        let calls = vec![
            runtime::RetryPlaybookToolCall {
                name: "developer__shell".to_string(),
                success: false,
            },
            runtime::RetryPlaybookToolCall {
                name: "developer__text_editor".to_string(),
                success: true,
            },
            runtime::RetryPlaybookToolCall {
                name: "developer__shell".to_string(),
                success: false,
            },
        ];

        let detected = MissionExecutor::detect_repeated_failed_tool(&calls);

        assert_eq!(detected.as_deref(), Some("developer__shell"));
    }

    #[test]
    fn composes_retry_instruction_with_supervisor_hint() {
        let combined = MissionExecutor::compose_retry_turn_instruction(
            Some("Retry focus: reuse outputs.".to_string()),
            Some("先落一个最小可验证的中间成果，再继续扩展当前交付物。"),
        )
        .expect("combined instruction");
        assert!(combined.contains("Retry focus: reuse outputs."));
        assert!(combined.contains("Supervisor guidance: 先落一个最小可验证的中间成果"));
    }

    #[test]
    fn parses_supervisor_guidance_json() {
        let guidance = MissionExecutor::parse_supervisor_guidance_response(
            r#"{"diagnosis":"当前 step 过大，尚缺少可验证的中间成果","resume_hint":"继续当前 step，先保存一个最小可验证的中间结果，再扩展到下一层交付物。"}"#,
        )
        .expect("guidance");
        assert_eq!(guidance.diagnosis, "当前 step 过大，尚缺少可验证的中间成果");
        assert!(guidance.resume_hint.contains("最小可验证的中间结果"));
        assert!(guidance.semantic_tags.is_empty());
        assert!(guidance.observed_evidence.is_empty());
    }

    #[test]
    fn parses_supervisor_guidance_optional_semantic_fields() {
        let guidance = MissionExecutor::parse_supervisor_guidance_response(
            r#"{
                "diagnosis":"当前存在漂移风险，但已有持续工作迹象",
                "resume_hint":"继续当前 step，先保存一个可验证的中间结果，再决定是否扩展范围。",
                "semantic_tags":["Research","incremental delivery","recovery"],
                "observed_evidence":["已有 planning evidence","最近出现新的 work progress 事件"]
            }"#,
        )
        .expect("guidance");
        assert_eq!(
            guidance.semantic_tags,
            vec![
                "research".to_string(),
                "incremental_delivery".to_string(),
                "recovery".to_string()
            ]
        );
        assert_eq!(
            guidance.observed_evidence,
            vec![
                "已有 planning evidence".to_string(),
                "最近出现新的 work progress 事件".to_string()
            ]
        );
    }

    #[test]
    fn supervisor_intervention_event_carries_ai_semantics() {
        let progress = StepProgressSnapshot::default();
        let guidance = SupervisorGuidance {
            diagnosis: "当前已观察到研究与规划证据，但缺少新的可验证中间结果".to_string(),
            resume_hint: "继续当前 step，先落一个最小中间结果。".to_string(),
            semantic_tags: vec!["research".to_string(), "incremental_delivery".to_string()],
            observed_evidence: vec!["已有 planning evidence".to_string()],
        };

        let events = MissionExecutor::build_progress_events(
            StepSupervisorState::Drifting,
            &progress,
            Some("work progress slowed"),
            Some(&guidance),
        );
        let supervisor_event = events
            .into_iter()
            .find(|event| event.kind == StepProgressEventKind::SupervisorIntervention)
            .expect("supervisor event");

        assert!(supervisor_event
            .semantic_tags
            .contains(&"research".to_string()));
        assert!(supervisor_event
            .semantic_tags
            .contains(&"incremental_delivery".to_string()));
        let annotation = supervisor_event.ai_annotation.expect("annotation");
        assert!(annotation.contains("diagnosis:"));
        assert!(annotation.contains("observed_evidence:"));
    }

    #[test]
    fn compact_recent_progress_prioritizes_high_value_events() {
        let digest = MissionExecutor::compact_recent_progress_for_prompt(
            &[
                StepProgressEvent {
                    kind: StepProgressEventKind::ActivityObserved,
                    message: "step activity observed: messages_delta=3".to_string(),
                    source: Some(StepProgressEventSource::Executor),
                    layer: Some(StepProgressLayer::Activity),
                    semantic_tags: vec!["activity".to_string()],
                    ai_annotation: None,
                    paths: vec![],
                    checks: vec![],
                    score_delta: None,
                    recorded_at: None,
                },
                StepProgressEvent {
                    kind: StepProgressEventKind::WorkProgressObserved,
                    message: "work progress observed: tools=4, planning_evidence=1".to_string(),
                    source: Some(StepProgressEventSource::Executor),
                    layer: Some(StepProgressLayer::WorkProgress),
                    semantic_tags: vec!["work_progress".to_string()],
                    ai_annotation: None,
                    paths: vec![],
                    checks: vec![],
                    score_delta: None,
                    recorded_at: None,
                },
                StepProgressEvent {
                    kind: StepProgressEventKind::RequiredArtifactSatisfied,
                    message: "required artifacts available: reports/final/report-data.json"
                        .to_string(),
                    source: Some(StepProgressEventSource::Verifier),
                    layer: Some(StepProgressLayer::DeliveryProgress),
                    semantic_tags: vec!["contract_artifact".to_string()],
                    ai_annotation: None,
                    paths: vec![],
                    checks: vec![],
                    score_delta: None,
                    recorded_at: None,
                },
            ],
            4,
        );

        assert!(digest.contains("work progress observed"));
        assert!(digest.contains("required artifacts available"));
        assert!(!digest.contains("messages_delta=3"));
    }

    #[test]
    fn compact_evidence_prefers_signals_over_path_dumps() {
        let digest = MissionExecutor::compact_evidence_for_prompt(Some(&StepEvidenceBundle {
            artifact_paths: vec![
                "reports/final/long-mission-report.html".to_string(),
                "reports/final/long-mission-slides.html".to_string(),
            ],
            planning_signals: vec![
                "outline_evidence".to_string(),
                "research_evidence".to_string(),
            ],
            runtime_signals: vec!["health_check_evidence".to_string()],
            risk_signals: vec!["known_issues_evidence".to_string()],
            latest_summary: Some(
                "当前已有大纲与研究类证据，建议继续生成下一个可验证交付物。".to_string(),
            ),
            ..Default::default()
        }));

        assert!(digest.contains("planning: outline_evidence, research_evidence"));
        assert!(digest.contains("runtime: health_check_evidence"));
        assert!(digest.contains("risk: known_issues_evidence"));
        assert!(digest.contains("observed_summary: 当前已有大纲与研究类证据"));
        assert!(!digest.contains("reports/final/long-mission-slides.html"));
    }

    #[test]
    fn complex_runtime_goals_require_multi_step_plan() {
        let goal = "Build a fresh long-running Express app on port 3004 with /health, /api/search, a minimal UI, and deploy it with pm2";
        assert!(MissionExecutor::goal_requires_multi_step_plan(goal));
    }

    #[test]
    fn simple_goals_do_not_require_multi_step_plan() {
        let goal = "Write reports/summary.md with a concise environment summary";
        assert!(!MissionExecutor::goal_requires_multi_step_plan(goal));
    }

    #[test]
    fn runtime_goal_fallback_creates_multiple_steps() {
        let steps = MissionExecutor::fallback_steps_from_goal(
            "Build a long-running Express service with pm2 deployment, runtime verification, and a minimal web UI on port 3004",
            Some(3),
            Some(180),
        );

        assert!(steps.len() >= 5);
        assert!(steps[0].title.contains("Preflight"));
        assert!(steps
            .iter()
            .any(|step| step.title.contains("Deploy long-running service")));
    }

    #[test]
    fn complex_goal_single_step_plan_requires_expansion() {
        let steps = vec![MissionExecutor::fallback_step_from_goal(
            "Build and deploy a long-running service with APIs and UI",
            Some(2),
            Some(180),
        )];
        assert!(MissionExecutor::plan_requires_expansion(
            "Build and deploy a long-running service with APIs and UI",
            &steps
        ));
    }

    #[test]
    fn multi_artifact_runtime_goals_use_generic_bundle_fallback_steps() {
        let steps = MissionExecutor::fallback_steps_from_goal(
            "生成中文 HTML 调研报告、slides 演示稿、report-data.json，并通过 Express 在 3006 提供 /report 和 /slides 服务",
            Some(3),
            Some(240),
        );

        assert!(steps.len() >= 8);
        assert!(steps.iter().any(|step| step
            .title
            .contains("Generate primary deliverable incrementally")));
        assert!(steps
            .iter()
            .any(|step| step.title.contains("Generate companion deliverables")));
    }

    #[test]
    fn timeout_retry_timeout_inherits_previous_timeout_level() {
        let mut step = sample_step();
        step.error_message = Some("Step 3 timed out after 480s".to_string());

        let base = std::time::Duration::from_secs(240);
        let prior_level = MissionExecutor::infer_prior_timeout_retry_level(&step, base);
        let timeout = MissionExecutor::resolve_retry_attempt_timeout(base, prior_level, 0);

        assert_eq!(prior_level, 1);
        assert_eq!(timeout.as_secs(), 480);
    }
}
