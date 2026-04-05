use std::env;

use serde::{Deserialize, Serialize};

pub const AGIME_ENABLE_NATIVE_SWARM_TOOL_ENV: &str = "AGIME_ENABLE_NATIVE_SWARM_TOOL";
pub const AGIME_ENABLE_SWARM_PLANNER_AUTO_ENV: &str = "AGIME_ENABLE_SWARM_PLANNER_AUTO";
pub const AGIME_ENABLE_LEADER_PERMISSION_BRIDGE_ENV: &str = "AGIME_ENABLE_LEADER_PERMISSION_BRIDGE";
pub const AGIME_ENABLE_SWARM_SCRATCHPAD_ENV: &str = "AGIME_ENABLE_SWARM_SCRATCHPAD";

fn env_truthy(key: &str) -> bool {
    env::var(key)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub fn native_swarm_tool_enabled() -> bool {
    env_truthy(AGIME_ENABLE_NATIVE_SWARM_TOOL_ENV)
}

pub fn planner_auto_swarm_enabled() -> bool {
    env_truthy(AGIME_ENABLE_SWARM_PLANNER_AUTO_ENV)
}

pub fn leader_permission_bridge_enabled() -> bool {
    env_truthy(AGIME_ENABLE_LEADER_PERMISSION_BRIDGE_ENV)
}

pub fn swarm_scratchpad_enabled() -> bool {
    env_truthy(AGIME_ENABLE_SWARM_SCRATCHPAD_ENV)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoordinatorRole {
    #[default]
    Leader,
    Worker,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerIdentity {
    pub run_id: String,
    pub worker_id: String,
    pub worker_name: String,
    pub role: CoordinatorRole,
    pub target_artifact: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SwarmExecutionRequest {
    pub instructions: String,
    pub targets: Vec<String>,
    pub write_scope: Vec<String>,
    pub result_contract: Vec<String>,
    pub parallelism_budget: Option<u32>,
    pub validation_mode: bool,
    pub summary_only: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SwarmExecutionResult {
    pub run_id: String,
    pub worker_summaries: Vec<String>,
    pub validation_summaries: Vec<String>,
    pub produced_targets: Vec<String>,
    pub downgraded: bool,
    pub downgrade_message: Option<String>,
    pub scratchpad_path: Option<String>,
    pub mailbox_path: Option<String>,
}

pub fn sanitize_worker_name(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | ' ' => '_',
            _ => ch,
        })
        .collect::<String>();
    let trimmed = sanitized.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "worker".to_string()
    } else {
        trimmed.chars().take(48).collect()
    }
}

pub fn worker_name_for_target(target: &str, idx: usize) -> String {
    format!("worker_{}_{}", idx + 1, sanitize_worker_name(target))
}
