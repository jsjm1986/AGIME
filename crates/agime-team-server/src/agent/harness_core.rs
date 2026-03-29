use serde::{Deserialize, Serialize};

use super::mission_mongo::{
    MissionActionPacket, MissionExecutionLease, MissionProgressMemory,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HarnessTurnMode {
    Plan,
    Execute,
    Repair,
    Blocked,
    Complete,
    #[default]
    Conversation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HarnessSwarmMode {
    #[default]
    Disabled,
    Gather,
    Fill,
    Draft,
    Validate,
    RecursiveOrchestrate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HarnessDelegationMode {
    #[default]
    Disabled,
    Subagent,
    Swarm,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Planning,
    Executing,
    Repairing,
    Blocked,
    WaitingExternal,
    Paused,
    Completed,
    Failed,
    Cancelled,
    #[default]
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubagentPermissionMode {
    #[default]
    Inherit,
    ReadOnly,
    ScopedWrite,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubagentSpawnPolicy {
    #[default]
    OnDemand,
    AutoIfStalled,
    Required,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HookEventKind {
    SessionStart,
    PreTurn,
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    SubagentStart,
    SubagentStop,
    PreCompact,
    RunResume,
    RunSettle,
    #[default]
    Conversation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunCheckpointKind {
    NodeStart,
    NodeSuccess,
    RepairStart,
    SubagentFanOut,
    SubagentFanIn,
    SettleComplete,
    #[default]
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskNode {
    pub task_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub mode: HarnessTurnMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_artifacts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_artifacts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_mode: Option<HarnessDelegationMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism_budget: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swarm_mode: Option<HarnessSwarmMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swarm_budget: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub write_scope: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub result_contract: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskEdge {
    pub from_node_id: String,
    pub to_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskGraph {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<bson::oid::ObjectId>,
    pub task_graph_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub root_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_node_id: Option<String>,
    #[serde(default)]
    pub nodes: Vec<TaskNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<TaskEdge>,
    #[serde(default)]
    pub graph_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActionPacket {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_artifacts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_artifacts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_delta: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub success_proof: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failure_escalation: Vec<String>,
}

impl ActionPacket {
    pub fn locked_target(&self) -> Option<&str> {
        self.target_artifacts
            .iter()
            .map(String::as_str)
            .find(|value| !value.trim().is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunMemory {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub done: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failed_attempt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_best_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<bson::DateTime>,
}

impl RunMemory {
    pub fn first_missing(&self) -> Option<&str> {
        self.missing
            .iter()
            .map(String::as_str)
            .find(|value| !value.trim().is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunLease {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_progress_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TurnOutcome {
    #[serde(default)]
    pub mode: HarnessTurnMode,
    #[serde(default)]
    pub produced_file_delta: bool,
    #[serde(default)]
    pub produced_evidence_delta: bool,
    #[serde(default)]
    pub produced_blocker_delta: bool,
    #[serde(default)]
    pub tool_calls: usize,
    #[serde(default)]
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectMemory {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assumptions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferences: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArtifactMemory {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known_artifacts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunState {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<bson::oid::ObjectId>,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_graph_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_node_id: Option<String>,
    #[serde(default)]
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<RunLease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<RunMemory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_memory: Option<ProjectMemory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_memory: Option<ArtifactMemory>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_subagents: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_outcome: Option<TurnOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunJournal {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<bson::oid::ObjectId>,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<String>,
    pub task_node_id: String,
    #[serde(default)]
    pub mode: HarnessTurnMode,
    #[serde(default)]
    pub tool_calls: usize,
    #[serde(default)]
    pub produced_file_delta: bool,
    #[serde(default)]
    pub produced_evidence_delta: bool,
    #[serde(default)]
    pub produced_blocker_delta: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunCheckpoint {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<bson::oid::ObjectId>,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_graph_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_node_id: Option<String>,
    #[serde(default)]
    pub checkpoint_kind: RunCheckpointKind,
    #[serde(default)]
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<RunLease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<RunMemory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_outcome: Option<TurnOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentSpec {
    pub name: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(default)]
    pub permission_mode: SubagentPermissionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub write_scope: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub result_schema: Vec<String>,
    #[serde(default)]
    pub spawn_policy: SubagentSpawnPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentRun {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<bson::oid::ObjectId>,
    pub subagent_run_id: String,
    pub parent_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_task_node_id: Option<String>,
    pub spec_name: String,
    #[serde(default)]
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub write_scope: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<bson::DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HookSpec {
    pub hook_id: String,
    pub event: HookEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub write_scope: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_tools: Vec<String>,
    #[serde(default)]
    pub enabled: bool,
}

impl From<&MissionActionPacket> for ActionPacket {
    fn from(value: &MissionActionPacket) -> Self {
        Self {
            target_artifacts: value.target_files.clone(),
            input_artifacts: value.input_files.clone(),
            required_tools: value.required_tool_use.clone(),
            expected_delta: value.expected_artifact_delta.clone(),
            success_proof: value.success_proof.clone(),
            failure_escalation: value.failure_escalation.clone(),
        }
    }
}

impl From<&MissionProgressMemory> for RunMemory {
    fn from(value: &MissionProgressMemory) -> Self {
        Self {
            done: value.done.clone(),
            missing: value.missing.clone(),
            blocked_by: value.blocked_by.clone(),
            last_failed_attempt: value.last_failed_attempt.clone(),
            next_best_action: value.next_best_action.clone(),
            confidence: value.confidence,
            updated_at: value.updated_at,
        }
    }
}

impl From<&MissionExecutionLease> for RunLease {
    fn from(value: &MissionExecutionLease) -> Self {
        Self {
            holder_kind: value.holder_kind.clone(),
            run_id: value.run_id.clone(),
            session_id: value.session_id.clone(),
            last_progress_at: value.last_heartbeat_at,
            expires_at: value.expires_at,
        }
    }
}
