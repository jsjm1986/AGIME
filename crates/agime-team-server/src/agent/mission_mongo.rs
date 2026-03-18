//! MongoDB document types for Mission Track (Phase 2)
//!
//! Missions are goal-driven, multi-step autonomous tasks with:
//! - Agent-generated execution plans (2-10 steps)
//! - Approval policies: Auto, Checkpoint, Manual
//! - Structured artifacts (code, documents, configs)
//! - Token budget control
//! - Real-time streaming of step execution

use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

// ─── Enums ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MissionStatus {
    Draft,
    Planning,
    Planned,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MissionCompletionDisposition {
    Complete,
    CompletedWithMinorGaps,
    PartialHandoff,
    BlockedByEnvironment,
    BlockedByTooling,
    WaitingExternal,
    BlockedFail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissionCompletionDecision {
    Complete,
    CompletedWithMinorGaps,
    ContinueWithReplan,
    PartialHandoff,
    BlockedByEnvironment,
    BlockedByTooling,
    WaitingExternal,
    BlockedFail,
}

impl MissionCompletionDecision {
    pub fn from_assessor_decision(raw: &str) -> Self {
        let normalized = raw.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        match normalized.as_str() {
            "complete_if_sufficient" => Self::Complete,
            "completed_with_minor_gaps" => Self::CompletedWithMinorGaps,
            "continue_with_replan" => Self::ContinueWithReplan,
            "partial_handoff" => Self::PartialHandoff,
            "blocked_by_environment" => Self::BlockedByEnvironment,
            "blocked_by_tooling" => Self::BlockedByTooling,
            "waiting_external" | "mark_waiting_external" => Self::WaitingExternal,
            "blocked_fail" => Self::BlockedFail,
            _ => Self::Complete,
        }
    }

    pub fn to_assessment(
        self,
        reason: Option<String>,
        observed_evidence: Vec<String>,
        missing_core_deliverables: Vec<String>,
    ) -> Option<MissionCompletionAssessment> {
        let disposition = match self {
            Self::Complete => MissionCompletionDisposition::Complete,
            Self::CompletedWithMinorGaps => MissionCompletionDisposition::CompletedWithMinorGaps,
            Self::PartialHandoff => MissionCompletionDisposition::PartialHandoff,
            Self::BlockedByEnvironment => MissionCompletionDisposition::BlockedByEnvironment,
            Self::BlockedByTooling => MissionCompletionDisposition::BlockedByTooling,
            Self::WaitingExternal => MissionCompletionDisposition::WaitingExternal,
            Self::BlockedFail => MissionCompletionDisposition::BlockedFail,
            Self::ContinueWithReplan => return None,
        };

        Some(MissionCompletionAssessment {
            disposition,
            reason,
            observed_evidence,
            missing_core_deliverables,
            recorded_at: Some(bson::DateTime::now()),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    AwaitingApproval,
    Running,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StepSupervisorState {
    Healthy,
    Busy,
    Drifting,
    Stalled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StepProgressEventKind {
    StepStarted,
    ActivityObserved,
    WorkProgressObserved,
    SummaryObserved,
    ArtifactObserved,
    RequiredArtifactSatisfied,
    PlanningEvidenceObserved,
    QualityEvidenceObserved,
    RuntimeEvidenceObserved,
    DeploymentEvidenceObserved,
    ReviewEvidenceObserved,
    RiskEvidenceObserved,
    RuntimeContractCaptured,
    ContractVerified,
    RetryScheduled,
    SupervisorIntervention,
    StepCompleted,
    StepFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StepProgressEventSource {
    Executor,
    Workspace,
    Verifier,
    Supervisor,
    AiSupervisor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StepProgressLayer {
    Activity,
    WorkProgress,
    DeliveryProgress,
    Recovery,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    #[default]
    Auto,
    Checkpoint,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MissionRouteMode {
    #[default]
    Auto,
    Mission,
    Direct,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    Code,
    Document,
    Config,
    Image,
    Data,
    Other,
}

// ─── AGE Types (Adaptive Goal Execution) ─────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    #[default]
    Sequential,
    Adaptive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionProfile {
    #[default]
    Auto,
    Fast,
    Full,
}

const AUTO_FAST_GOAL_MAX_CHARS: usize = 320;
const AUTO_FAST_CONTEXT_MAX_CHARS: usize = 1200;
const AUTO_FAST_MAX_ATTACHED_DOCS: usize = 2;

fn legacy_auto_fast_heuristic(mission: &MissionDoc) -> bool {
    if mission.execution_mode == ExecutionMode::Adaptive {
        return false;
    }
    if mission.approval_policy != ApprovalPolicy::Auto {
        return false;
    }
    if mission.step_timeout_seconds.is_some() || mission.step_max_retries.is_some() {
        return false;
    }

    let goal_len = mission.goal.chars().count();
    let ctx_len = mission
        .context
        .as_deref()
        .map(|s| s.chars().count())
        .unwrap_or(0);
    let attached_count = mission.attached_document_ids.len();

    goal_len <= AUTO_FAST_GOAL_MAX_CHARS
        && ctx_len <= AUTO_FAST_CONTEXT_MAX_CHARS
        && attached_count <= AUTO_FAST_MAX_ATTACHED_DOCS
}

/// Resolve `auto` profile strategy.
///
/// `TEAM_MISSION_AUTO_PROFILE`:
/// - `full` (default): always use full planning/execution for reliability.
/// - `fast`: force fast profile in sequential mode.
/// - `legacy_fast_heuristic`: use legacy size-based heuristic.
pub fn classify_auto_execution_profile(mission: &MissionDoc) -> ExecutionProfile {
    if mission.execution_mode == ExecutionMode::Adaptive {
        return ExecutionProfile::Full;
    }

    let strategy = std::env::var("TEAM_MISSION_AUTO_PROFILE")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "full".to_string());

    match strategy.as_str() {
        "fast" => ExecutionProfile::Fast,
        "legacy_fast_heuristic" => {
            if legacy_auto_fast_heuristic(mission) {
                ExecutionProfile::Fast
            } else {
                ExecutionProfile::Full
            }
        }
        _ => ExecutionProfile::Full,
    }
}

/// Resolve the effective execution profile used at runtime.
pub fn resolve_execution_profile(mission: &MissionDoc) -> ExecutionProfile {
    match mission.execution_profile {
        ExecutionProfile::Auto => classify_auto_execution_profile(mission),
        ExecutionProfile::Fast => {
            if mission.execution_mode == ExecutionMode::Adaptive {
                ExecutionProfile::Full
            } else {
                ExecutionProfile::Fast
            }
        }
        ExecutionProfile::Full => ExecutionProfile::Full,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressSignal {
    Advancing,
    Stalled,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    #[default]
    Pending,
    Running,
    AwaitingApproval,
    Completed,
    Pivoting,
    Abandoned,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptRecord {
    pub attempt_number: u32,
    pub approach: String,
    pub signal: ProgressSignal,
    pub learnings: String,
    pub tokens_used: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeContract {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_artifacts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completion_checks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_artifact_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeContractVerification {
    #[serde(default)]
    pub tool_called: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalNode {
    pub goal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub title: String,
    pub description: String,
    pub success_criteria: String,
    #[serde(default)]
    pub status: GoalStatus,
    #[serde(default)]
    pub depth: u32,
    #[serde(default)]
    pub order: u32,
    #[serde(default = "default_exploration_budget")]
    pub exploration_budget: u32,
    #[serde(default)]
    pub attempts: Vec<AttemptRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_contract: Option<RuntimeContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_verification: Option<RuntimeContractVerification>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot_reason: Option<String>,
    #[serde(default)]
    pub is_checkpoint: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_progress_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<bson::DateTime>,
}

fn default_exploration_budget() -> u32 {
    3
}

// ─── Mission Step (embedded in MissionDoc) ───────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub name: String,
    #[serde(default)]
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepProgressEvent {
    pub kind: StepProgressEventKind,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<StepProgressEventSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<StepProgressLayer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_annotation: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_delta: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StepEvidenceBundle {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_artifact_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planning_evidence_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planning_signals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quality_evidence_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quality_signals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_evidence_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_signals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deployment_evidence_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deployment_signals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_evidence_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_signals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risk_evidence_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risk_signals: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionStep {
    pub index: u32,
    pub title: String,
    pub description: String,
    pub status: StepStatus,
    #[serde(default)]
    pub is_checkpoint: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor_state: Option<StepSupervisorState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_progress_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_score: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_blocker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_supervisor_hint: Option<String>,
    #[serde(default)]
    pub stall_count: u32,
    /// Recent structured progress events retained for long-running supervision.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_progress_events: Vec<StepProgressEvent>,
    /// Aggregated evidence bundle derived from artifacts and verification signals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_bundle: Option<StepEvidenceBundle>,
    #[serde(default)]
    pub tokens_used: i32,
    /// Structured output summary extracted after step completion.
    /// Injected into subsequent step prompts to avoid context bloat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_summary: Option<String>,
    /// Number of times this step has been retried.
    #[serde(default)]
    pub retry_count: u32,
    /// Maximum retries allowed for transient failures (default 2).
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Optional per-step timeout override in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    /// Relative file paths that must exist in workspace before this step can complete.
    /// Example: ["reports/final_plan.md", "data/market.csv"]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_artifacts: Vec<String>,
    /// Optional shell checks executed in workspace after step completion.
    /// All checks must exit with status 0.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completion_checks: Vec<String>,
    /// Runtime contract captured from mandatory mission_preflight call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_contract: Option<RuntimeContract>,
    /// Runtime verification result from mission_preflight__verify_contract gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_verification: Option<RuntimeContractVerification>,
    /// Whether this step should prefer delegated execution via subagent tool.
    #[serde(default)]
    pub use_subagent: bool,
    /// Tool calls made during this step's execution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallRecord>,
}

// ─── Mission Document ────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub mission_id: String,
    pub team_id: String,
    pub agent_id: String,
    pub creator_id: String,
    pub goal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub status: MissionStatus,
    #[serde(default)]
    pub approval_policy: ApprovalPolicy,
    #[serde(default)]
    pub steps: Vec<MissionStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_step: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_chat_session_id: Option<String>,
    #[serde(default)]
    pub token_budget: i64,
    #[serde(default)]
    pub total_tokens_used: i64,
    #[serde(default)]
    pub priority: i32,
    /// Optional mission-level default step timeout (seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_timeout_seconds: Option<u64>,
    /// Optional mission-level default retries for generated steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_max_retries: Option<u32>,
    /// Plan version, incremented on each re-plan.
    #[serde(default = "default_plan_version")]
    pub plan_version: u32,
    // ─── AGE fields ───
    #[serde(default)]
    pub execution_mode: ExecutionMode,
    #[serde(default)]
    pub execution_profile: ExecutionProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_tree: Option<Vec<GoalNode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_goal_id: Option<String>,
    #[serde(default)]
    pub total_pivots: u32,
    #[serde(default)]
    pub total_abandoned: u32,
    // ─── end AGE fields ───
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Final mission-level summary synthesized after all steps/goals complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_assessment: Option<MissionCompletionAssessment>,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<bson::DateTime>,

    // === Phase 2: Document attachment ===
    #[serde(default)]
    pub attached_document_ids: Vec<String>,

    // === Workspace isolation ===
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    /// Current execution run identifier.
    /// Regenerated on each start/resume to isolate runtime event streams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_monitor_intervention: Option<MissionMonitorIntervention>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_applied_monitor_intervention: Option<MissionMonitorIntervention>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_strategy: Option<MissionStrategyState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_worker_state: Option<WorkerCompactState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_stuck_phase_snapshot: Option<MissionStuckPhaseSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_repair_lane_id: Option<String>,
    #[serde(default)]
    pub consecutive_no_tool_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_blocker_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_external_until: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionCompletionAssessment {
    pub disposition: MissionCompletionDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_core_deliverables: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MissionStrategyPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_strategy_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_for_change: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_goal_shape: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved_user_intent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_gain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MissionStrategyState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_core_deliverables: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_patch: Option<MissionStrategyPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_recommended: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism_budget: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkerCompactState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_goal: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub core_assets_now: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets_delta: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_blocker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_step_candidate: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_signals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtask_plan: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtask_results_summary: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_risk: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism_used: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MissionStuckPhaseSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_goal: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completed_results: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_core_deliverables: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_blocker: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempted_methods: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_next_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Default)]
pub struct MissionConvergencePatch {
    pub active_repair_lane_id: Option<Option<String>>,
    pub consecutive_no_tool_count: Option<u32>,
    pub last_blocker_fingerprint: Option<Option<String>>,
    pub waiting_external_until: Option<Option<bson::DateTime>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionArtifactDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub artifact_id: String,
    pub mission_id: String,
    pub step_index: u32,
    pub name: String,
    pub artifact_type: ArtifactType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub size: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_document_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_document_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<bson::DateTime>,
    pub created_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionEventDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub mission_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub event_id: i64,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: bson::DateTime,
}

// ─── Request / Response Types ────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateMissionRequest {
    pub agent_id: String,
    pub goal: String,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub route_mode: Option<MissionRouteMode>,
    #[serde(default)]
    pub approval_policy: Option<ApprovalPolicy>,
    #[serde(default)]
    pub token_budget: Option<i64>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub step_timeout_seconds: Option<u64>,
    #[serde(default)]
    pub step_max_retries: Option<u32>,
    #[serde(default)]
    pub source_chat_session_id: Option<String>,
    #[serde(default)]
    pub execution_mode: Option<ExecutionMode>,
    #[serde(default)]
    pub execution_profile: Option<ExecutionProfile>,
    #[serde(default)]
    pub attached_document_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MissionListItem {
    pub mission_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub goal: String,
    pub status: MissionStatus,
    pub approval_policy: ApprovalPolicy,
    pub step_count: usize,
    pub completed_steps: usize,
    pub current_step: Option<u32>,
    pub total_tokens_used: i64,
    pub created_at: String,
    pub updated_at: String,
    // AGE fields
    pub execution_mode: ExecutionMode,
    pub execution_profile: ExecutionProfile,
    pub resolved_execution_profile: ExecutionProfile,
    pub goal_count: usize,
    pub completed_goals: usize,
    pub pivots: u32,
    pub attached_doc_count: usize,
}

#[derive(Debug, Deserialize)]
pub struct ListMissionsQuery {
    pub team_id: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_plan_version() -> u32 {
    1
}
fn default_max_retries() -> u32 {
    2
}
fn default_page() -> u32 {
    1
}
fn default_limit() -> u32 {
    20
}

#[derive(Debug, Deserialize)]
pub struct StepActionRequest {
    #[serde(default)]
    pub feedback: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GoalActionRequest {
    #[serde(default)]
    pub feedback: Option<String>,
    #[serde(default)]
    pub alternative_approach: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MonitorActionRequest {
    pub action: String,
    #[serde(default)]
    pub feedback: Option<String>,
    #[serde(default)]
    pub semantic_tags: Vec<String>,
    #[serde(default)]
    pub observed_evidence: Vec<String>,
    #[serde(default)]
    pub missing_core_deliverables: Vec<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub strategy_patch: Option<MissionStrategyPatch>,
    #[serde(default)]
    pub subagent_recommended: Option<bool>,
    #[serde(default)]
    pub parallelism_budget: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionMonitorIntervention {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_core_deliverables: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_patch: Option<MissionStrategyPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_recommended: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism_budget: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_at: Option<bson::DateTime>,
}

#[derive(Debug, Serialize)]
pub struct MonitorInterventionSnapshot {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_core_deliverables: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_patch: Option<MissionStrategyPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_recommended: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism_budget: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorAssessmentSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_assessment: Option<String>,
    #[serde(default)]
    pub evidence_sufficient: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_summary: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MonitorStepSnapshot {
    pub index: u32,
    pub title: String,
    pub description: String,
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor_state: Option<StepSupervisorState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_progress_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_score: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_blocker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_supervisor_hint: Option<String>,
    #[serde(default)]
    pub stall_count: u32,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub output_summary_present: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_artifacts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completion_checks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_progress_events: Vec<StepProgressEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_bundle: Option<StepEvidenceBundle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment: Option<MonitorAssessmentSnapshot>,
}

#[derive(Debug, Serialize)]
pub struct MonitorGoalSnapshot {
    pub goal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub title: String,
    pub description: String,
    pub success_criteria: String,
    pub status: GoalStatus,
    #[serde(default)]
    pub attempt_count: usize,
    #[serde(default)]
    pub output_summary_present: bool,
    #[serde(default)]
    pub has_runtime_contract: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_verified: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_progress_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment: Option<MonitorAssessmentSnapshot>,
}

#[derive(Debug, Serialize)]
pub struct MissionMonitorSnapshot {
    pub mission_id: String,
    pub status: MissionStatus,
    pub execution_mode: ExecutionMode,
    pub execution_profile: ExecutionProfile,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_step: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_assessment: Option<MissionCompletionAssessment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_strategy: Option<MissionStrategyState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_worker_state: Option<WorkerCompactState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_stuck_phase_snapshot: Option<MissionStuckPhaseSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_repair_lane_id: Option<String>,
    #[serde(default)]
    pub consecutive_no_tool_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_blocker_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_external_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_intervention: Option<MonitorInterventionSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_applied_intervention: Option<MonitorInterventionSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_last_activity_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_last_progress_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_contract: Option<MonitorContractSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<MonitorAssetSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<MonitorStepSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<MonitorGoalSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorContractSnapshot {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_artifacts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completion_checks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_artifact_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorAssetRecord {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    pub artifact_type: ArtifactType,
    #[serde(default)]
    pub step_index: u32,
    #[serde(default)]
    pub size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorAssetSnapshot {
    #[serde(default)]
    pub total_assets: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub core_assets_now: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_assets: Vec<MonitorAssetRecord>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFromChatRequest {
    pub agent_id: String,
    pub goal: String,
    pub chat_session_id: String,
    #[serde(default)]
    pub approval_policy: Option<ApprovalPolicy>,
    #[serde(default)]
    pub token_budget: Option<i64>,
}
