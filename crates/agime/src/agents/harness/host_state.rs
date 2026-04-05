use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::agents::extension::ExtensionConfig;
use crate::session::extension_data::ExtensionState;
use crate::session::SessionManager;

use super::completion::ExecuteCompletionOutcome;
use super::delegation::DelegationMode;
use super::signals::CoordinatorSignalSummary;
use super::state::{CoordinatorExecutionMode, ProviderTurnMode};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HarnessHostSessionState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_board_session_id: Option<String>,
    pub delegation_mode: DelegationMode,
    #[serde(default)]
    pub coordinator_execution_mode: CoordinatorExecutionMode,
    #[serde(default)]
    pub provider_turn_mode: ProviderTurnMode,
    pub write_scope: Vec<String>,
    pub target_artifacts: Vec<String>,
    pub result_contract: Vec<String>,
    #[serde(default)]
    pub server_local_tool_names: Vec<String>,
    #[serde(default)]
    pub required_tool_prefixes: Vec<String>,
    pub parallelism_budget: Option<u32>,
    pub swarm_budget: Option<u32>,
    pub validation_mode: bool,
    #[serde(default)]
    pub worker_extensions: Vec<ExtensionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_notification_summary: Option<CoordinatorSignalSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_signal_summary: Option<CoordinatorSignalSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_completion_outcome: Option<ExecuteCompletionOutcome>,
}

impl ExtensionState for HarnessHostSessionState {
    const EXTENSION_NAME: &'static str = "harness_host";
    const VERSION: &'static str = "v0";
}

pub async fn load_host_session_state(session_id: &str) -> Result<Option<HarnessHostSessionState>> {
    let session = SessionManager::get_session(session_id, false).await?;
    Ok(HarnessHostSessionState::from_extension_data(
        &session.extension_data,
    ))
}

pub async fn save_host_session_state(
    session_id: &str,
    state: &HarnessHostSessionState,
) -> Result<()> {
    SessionManager::set_extension_state(session_id, state).await
}

pub async fn update_host_signal_summary(
    session_id: &str,
    summary: CoordinatorSignalSummary,
) -> Result<()> {
    if let Some(mut state) = load_host_session_state(session_id).await? {
        state.last_signal_summary = Some(summary);
        save_host_session_state(session_id, &state).await?;
    }
    Ok(())
}

pub async fn update_host_notification_summary(
    session_id: &str,
    summary: Option<CoordinatorSignalSummary>,
) -> Result<()> {
    if let Some(mut state) = load_host_session_state(session_id).await? {
        state.last_notification_summary = summary;
        save_host_session_state(session_id, &state).await?;
    }
    Ok(())
}

pub async fn update_host_completion_outcome(
    session_id: &str,
    outcome: ExecuteCompletionOutcome,
) -> Result<()> {
    if let Some(mut state) = load_host_session_state(session_id).await? {
        state.last_completion_outcome = Some(outcome);
        save_host_session_state(session_id, &state).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{
        SessionManager, SessionType, TaskItem, TaskScope, TaskStatus, TasksStateV2,
    };
    use std::collections::HashMap;
    use uuid::Uuid;

    #[tokio::test]
    async fn save_host_session_state_preserves_existing_tasks_state() {
        let session_id = format!("host-state-tasks-{}", Uuid::new_v4());
        SessionManager::create_session_with_id(
            session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "host-state-test".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create session");

        let mut tasks = TasksStateV2::new(TaskScope::Leader, session_id.clone());
        tasks.items.push(TaskItem {
            id: "1".to_string(),
            subject: "projected worker task".to_string(),
            description: "ensure tasks survive host-state merge".to_string(),
            active_form: "projecting worker task".to_string(),
            owner: Some("worker-a".to_string()),
            status: TaskStatus::Completed,
            blocks: Vec::new(),
            blocked_by: Vec::new(),
            metadata: HashMap::new(),
        });
        SessionManager::set_extension_state(&session_id, &tasks)
            .await
            .expect("save tasks");

        save_host_session_state(
            &session_id,
            &HarnessHostSessionState {
                task_board_session_id: Some(session_id.clone()),
                delegation_mode: DelegationMode::Swarm,
                coordinator_execution_mode: CoordinatorExecutionMode::ExplicitSwarm,
                provider_turn_mode: ProviderTurnMode::Aggregated,
                write_scope: Vec::new(),
                target_artifacts: Vec::new(),
                result_contract: Vec::new(),
                server_local_tool_names: Vec::new(),
                required_tool_prefixes: Vec::new(),
                parallelism_budget: Some(2),
                swarm_budget: Some(2),
                validation_mode: false,
                worker_extensions: Vec::new(),
                last_notification_summary: None,
                last_signal_summary: None,
                last_completion_outcome: None,
            },
        )
        .await
        .expect("save host state");

        let session = SessionManager::get_session(&session_id, false)
            .await
            .expect("load session");
        let restored = TasksStateV2::from_extension_data(&session.extension_data)
            .expect("tasks state should remain present");
        assert_eq!(restored.items.len(), 1);
        assert_eq!(restored.items[0].subject, "projected worker task");
    }
}
