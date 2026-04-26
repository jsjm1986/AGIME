use crate::config::AgimeMode;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use super::completion::CompletionSurfacePolicy;
use super::delegation::DelegationRuntimeState;
use super::result_budget::ToolResultBudget;
use super::signals::{CoordinatorSignalStore, SharedCoordinatorSignalStore};
use super::state::{CoordinatorExecutionMode, HarnessMode, ProviderTurnMode};
use super::task_runtime::TaskRuntime;
use super::transition::{shared_transition_trace, QueryTrackingState, SharedTransitionTrace};
use super::CoordinatorRole;

#[derive(Debug, Clone, Default)]
pub struct HarnessPermissionContext {
    pub mode: HarnessMode,
    pub write_scope: Vec<String>,
    pub resource_scope: Vec<String>,
    pub allow_child_tasks: bool,
    pub require_explicit_scope: bool,
}

#[derive(Debug, Clone, Default)]
pub struct HarnessWorkerRuntimeContext {
    pub swarm_run_id: Option<String>,
    pub worker_name: Option<String>,
    pub leader_name: Option<String>,
    pub logical_worker_id: Option<String>,
    pub coordinator_role: CoordinatorRole,
    pub mailbox_dir: Option<PathBuf>,
    pub permission_dir: Option<PathBuf>,
    pub scratchpad_dir: Option<PathBuf>,
    pub enable_permission_bridge: bool,
    pub allow_worker_messaging: bool,
    pub peer_worker_addresses: Vec<String>,
    pub peer_worker_catalog: Vec<String>,
    pub validation_mode: bool,
}

#[derive(Debug, Clone)]
pub struct HarnessContext {
    pub session_id: String,
    pub working_dir: PathBuf,
    pub agime_mode: AgimeMode,
    pub max_turns: u32,
    pub cancel_token: Option<CancellationToken>,
    pub mode: HarnessMode,
    pub coordinator_execution_mode: CoordinatorExecutionMode,
    pub provider_turn_mode: ProviderTurnMode,
    pub completion_surface_policy: CompletionSurfacePolicy,
    pub delegation: DelegationRuntimeState,
    pub write_scope: Vec<String>,
    pub target_artifacts: Vec<String>,
    pub result_contract: Vec<String>,
    pub validation_mode: bool,
    pub server_local_tool_names: Vec<String>,
    pub required_tool_prefixes: Vec<String>,
    pub task_budget_tokens: Option<u32>,
    pub task_runtime: Arc<TaskRuntime>,
    pub tool_result_budget: ToolResultBudget,
    pub coordinator_signals: SharedCoordinatorSignalStore,
    pub permission_context: HarnessPermissionContext,
    pub transition_trace: SharedTransitionTrace,
    pub query_tracking: QueryTrackingState,
    pub worker_runtime: Option<HarnessWorkerRuntimeContext>,
}

impl HarnessContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: String,
        working_dir: PathBuf,
        agime_mode: AgimeMode,
        max_turns: u32,
        cancel_token: Option<CancellationToken>,
        mode: HarnessMode,
        coordinator_execution_mode: CoordinatorExecutionMode,
        provider_turn_mode: ProviderTurnMode,
        completion_surface_policy: CompletionSurfacePolicy,
        delegation: DelegationRuntimeState,
        write_scope: Vec<String>,
        target_artifacts: Vec<String>,
        result_contract: Vec<String>,
        validation_mode: bool,
        server_local_tool_names: Vec<String>,
        required_tool_prefixes: Vec<String>,
        task_budget_tokens: Option<u32>,
        task_runtime: Arc<TaskRuntime>,
        worker_runtime: Option<HarnessWorkerRuntimeContext>,
    ) -> Self {
        let allow_child_tasks = delegation.can_delegate_subagent();
        Self {
            session_id,
            working_dir,
            agime_mode,
            max_turns,
            cancel_token,
            mode,
            coordinator_execution_mode,
            provider_turn_mode,
            completion_surface_policy,
            delegation: delegation.clone(),
            write_scope,
            target_artifacts,
            result_contract,
            validation_mode,
            server_local_tool_names,
            required_tool_prefixes,
            task_budget_tokens,
            task_runtime,
            tool_result_budget: ToolResultBudget::default(),
            coordinator_signals: CoordinatorSignalStore::shared(),
            permission_context: HarnessPermissionContext {
                mode,
                write_scope: delegation.write_scope.clone(),
                resource_scope: Vec::new(),
                allow_child_tasks,
                require_explicit_scope: false,
            },
            transition_trace: shared_transition_trace(),
            query_tracking: QueryTrackingState::default(),
            worker_runtime,
        }
    }
}
