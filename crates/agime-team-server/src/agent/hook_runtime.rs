use serde_json::Value;

use super::harness_core::{
    HookEventKind, HookSpec, RunCheckpoint, RunCheckpointKind, RunLease, RunMemory, RunStatus,
};
use super::mission_mongo::MissionDoc;
use super::runtime;
use super::service_mongo::AgentService;

fn normalized_scope(scope: &[String]) -> Vec<String> {
    scope
        .iter()
        .filter_map(|item| runtime::normalize_relative_workspace_path(item))
        .collect()
}

fn normalized_editor_path(args: &Value) -> Option<String> {
    args.get("path")
        .and_then(Value::as_str)
        .and_then(runtime::normalize_relative_workspace_path)
}

pub fn builtin_hook_specs() -> Vec<HookSpec> {
    vec![
        HookSpec {
            hook_id: "builtin:pre_tool_use".to_string(),
            event: HookEventKind::PreToolUse,
            description: Some("Enforce deterministic tool policies such as bounded subagent write scopes.".to_string()),
            blocking: true,
            write_scope: Vec::new(),
            required_tools: vec!["subagent".to_string()],
            enabled: true,
        },
        HookSpec {
            hook_id: "builtin:subagent_stop".to_string(),
            event: HookEventKind::SubagentStop,
            description: Some("Refresh mission overlay and attempt settle after subagent completion.".to_string()),
            blocking: false,
            write_scope: Vec::new(),
            required_tools: vec!["subagent".to_string()],
            enabled: true,
        },
        HookSpec {
            hook_id: "builtin:run_settle".to_string(),
            event: HookEventKind::RunSettle,
            description: Some("Persist durable settle checkpoint once a mission completes.".to_string()),
            blocking: false,
            write_scope: Vec::new(),
            required_tools: Vec::new(),
            enabled: true,
        },
    ]
}

pub fn apply_pre_tool_use_hooks(
    tool_name: &str,
    args: &Value,
    allowed_write_scope: Option<&[String]>,
) -> Result<Value, String> {
    let allowed_scope = allowed_write_scope.map(normalized_scope).unwrap_or_default();
    if !allowed_scope.is_empty() && tool_name == "developer__text_editor" {
        let Some(path) = normalized_editor_path(args) else {
            return Err(
                "developer__text_editor writes must use a workspace-relative `path` that stays inside the current task write scope"
                    .to_string(),
            );
        };
        if !allowed_scope
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&path))
        {
            return Err(format!(
                "developer__text_editor attempted to write `{}` outside the current task write scope ({})",
                path,
                allowed_scope.join(", ")
            ));
        }
    }
    if tool_name != "subagent" {
        return Ok(args.clone());
    }
    let Some(parent_scope) = allowed_write_scope else {
        return Ok(args.clone());
    };
    let requested_scope = args
        .get("write_scope")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let constrained = runtime::constrain_subagent_write_scope(parent_scope, &requested_scope);
    let mut adjusted = args.clone();
    adjusted["write_scope"] = Value::Array(
        constrained
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>(),
    );
    Ok(adjusted)
}

pub async fn run_subagent_stop_hooks(
    agent_service: &AgentService,
    mission_id: Option<&str>,
) -> Result<(), mongodb::error::Error> {
    let Some(mission_id) = mission_id else {
        return Ok(());
    };
    agent_service
        .refresh_delivery_manifest_from_artifacts(mission_id)
        .await?;
    agent_service.refresh_progress_memory(mission_id).await?;
    let _ = agent_service.settle_mission_result_if_ready(mission_id).await?;
    Ok(())
}

pub async fn run_settle_hooks(
    agent_service: &AgentService,
    mission: &MissionDoc,
) -> Result<(), mongodb::error::Error> {
    let Some(run_id) = mission.current_run_id.as_deref() else {
        return Ok(());
    };
    let run_state = agent_service.get_run_state(run_id).await?;
    let current_node_id = run_state
        .as_ref()
        .and_then(|state| state.current_node_id.clone())
        .or_else(|| {
            mission
                .current_goal_id
                .as_ref()
                .map(|goal_id| format!("goal:{}", goal_id))
        })
        .or_else(|| mission.current_step.map(|index| format!("step:{}", index)));
    let memory = run_state
        .as_ref()
        .and_then(|state| state.memory.clone())
        .or_else(|| mission.progress_memory.as_ref().map(RunMemory::from));
    let lease = run_state
        .as_ref()
        .and_then(|state| state.lease.clone())
        .or_else(|| mission.execution_lease.as_ref().map(RunLease::from));
    agent_service
        .save_run_checkpoint(&RunCheckpoint {
            id: None,
            run_id: run_id.to_string(),
            mission_id: Some(mission.mission_id.clone()),
            task_graph_id: Some(format!("mission:{}:{}", mission.mission_id, run_id)),
            current_node_id,
            checkpoint_kind: RunCheckpointKind::SettleComplete,
            status: RunStatus::Completed,
            lease,
            memory,
            last_turn_outcome: None,
            created_at: Some(mongodb::bson::DateTime::now()),
        })
        .await?;
    Ok(())
}
