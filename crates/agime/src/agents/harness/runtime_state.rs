use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::session::extension_data::ExtensionState;
use crate::session::SessionManager;

use super::CoordinatorRole;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HarnessWorkerRuntimeState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_board_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swarm_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leader_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_worker_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_task_id: Option<String>,
    #[serde(default)]
    pub coordinator_role: CoordinatorRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailbox_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scratchpad_dir: Option<PathBuf>,
    #[serde(default)]
    pub enable_permission_bridge: bool,
    #[serde(default)]
    pub allow_worker_messaging: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peer_worker_addresses: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peer_worker_catalog: Vec<String>,
    #[serde(default)]
    pub validation_mode: bool,
}

impl HarnessWorkerRuntimeState {
    pub fn is_configured(&self) -> bool {
        self.swarm_run_id.is_some()
            || self.task_board_session_id.is_some()
            || self.worker_name.is_some()
            || self.leader_name.is_some()
            || self.logical_worker_id.is_some()
            || self.attempt_id.is_some()
            || self.attempt_index.is_some()
            || self.previous_task_id.is_some()
            || self.mailbox_dir.is_some()
            || self.permission_dir.is_some()
            || self.scratchpad_dir.is_some()
            || self.enable_permission_bridge
            || self.allow_worker_messaging
            || !self.peer_worker_addresses.is_empty()
            || !self.peer_worker_catalog.is_empty()
            || self.validation_mode
            || self.coordinator_role != CoordinatorRole::Leader
    }
}

impl ExtensionState for HarnessWorkerRuntimeState {
    const EXTENSION_NAME: &'static str = "harness_worker_runtime";
    const VERSION: &'static str = "v0";
}

pub async fn load_worker_runtime_state(
    session_id: &str,
) -> Result<Option<HarnessWorkerRuntimeState>> {
    let session = SessionManager::get_session(session_id, false).await?;
    Ok(HarnessWorkerRuntimeState::from_extension_data(
        &session.extension_data,
    ))
}

pub async fn save_worker_runtime_state(
    session_id: &str,
    state: &HarnessWorkerRuntimeState,
) -> Result<()> {
    SessionManager::set_extension_state(session_id, state).await
}
