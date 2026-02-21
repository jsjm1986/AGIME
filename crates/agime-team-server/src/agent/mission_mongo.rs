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
pub enum ApprovalPolicy {
    Auto,
    Checkpoint,
    Manual,
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        Self::Auto
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Sequential,
    Adaptive,
}

impl Default for ExecutionMode {
    fn default() -> Self {
        Self::Sequential
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressSignal {
    Advancing,
    Stalled,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Pending,
    Running,
    AwaitingApproval,
    Completed,
    Pivoting,
    Abandoned,
    Failed,
}

impl Default for GoalStatus {
    fn default() -> Self {
        Self::Pending
    }
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
    pub pivot_reason: Option<String>,
    #[serde(default)]
    pub is_checkpoint: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<bson::DateTime>,
}

fn default_exploration_budget() -> u32 {
    3
}

// ─── Mission Step (embedded in MissionDoc) ───────────────

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
    /// Plan version, incremented on each re-plan.
    #[serde(default = "default_plan_version")]
    pub plan_version: u32,
    // ─── AGE fields ───
    #[serde(default)]
    pub execution_mode: ExecutionMode,
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
    pub approval_policy: Option<ApprovalPolicy>,
    #[serde(default)]
    pub token_budget: Option<i64>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub source_chat_session_id: Option<String>,
    #[serde(default)]
    pub execution_mode: Option<ExecutionMode>,
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
    pub goal_count: usize,
    pub completed_goals: usize,
    pub pivots: u32,
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
pub struct CreateFromChatRequest {
    pub agent_id: String,
    pub goal: String,
    pub chat_session_id: String,
    #[serde(default)]
    pub approval_policy: Option<ApprovalPolicy>,
    #[serde(default)]
    pub token_budget: Option<i64>,
}
