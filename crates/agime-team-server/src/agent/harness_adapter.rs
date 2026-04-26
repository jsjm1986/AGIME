use std::sync::Arc;

use agime::agents::harness::{
    ChildTaskRequest, CompletionSurfacePolicy, DelegationMode, DelegationRuntimeState,
    HarnessContext, HarnessMode, ProviderTurnMode, SwarmBudget, SwarmPlan, TaskKind, TaskRuntime,
    TaskSpec,
};
use agime_team::models::AgentTask;
use agime_team::MongoDb;

use super::harness_core::{HarnessDelegationMode, HarnessTurnMode, TaskNode};

pub struct ServerHarnessAdapter {
    pub db: Option<Arc<MongoDb>>,
    pub task_runtime: Arc<TaskRuntime>,
}

#[derive(Clone, Debug, Default)]
pub struct ServerDelegationContext {
    pub delegation_mode: HarnessDelegationMode,
    pub depth: u32,
    pub max_depth: u32,
    pub write_scope: Vec<String>,
    pub target_artifacts: Vec<String>,
    pub result_contract: Vec<String>,
    pub parallelism_budget: Option<u32>,
    pub swarm_budget: Option<u32>,
    pub validation_mode: bool,
    pub parent_run_id: Option<String>,
    pub parent_task_node_id: Option<String>,
    pub task_graph_id: Option<String>,
    pub spec_name: String,
}

#[derive(Clone, Debug)]
pub struct ServerChildTaskOptions {
    pub kind: TaskKind,
    pub task_id: Option<String>,
    pub spec_name: Option<String>,
    pub target_artifacts: Vec<String>,
    pub result_contract: Vec<String>,
    pub delegation_mode: HarnessDelegationMode,
    pub parallelism_budget: Option<u32>,
    pub swarm_budget: Option<u32>,
    pub validation_mode: bool,
}

impl ServerHarnessAdapter {
    pub fn new(db: Arc<MongoDb>) -> Self {
        Self {
            db: Some(db),
            task_runtime: TaskRuntime::shared(),
        }
    }

    pub fn map_turn_mode(mode: HarnessTurnMode) -> HarnessMode {
        match mode {
            HarnessTurnMode::Plan => HarnessMode::Plan,
            HarnessTurnMode::Execute => HarnessMode::Execute,
            HarnessTurnMode::Repair => HarnessMode::Repair,
            HarnessTurnMode::Blocked => HarnessMode::Blocked,
            HarnessTurnMode::Complete => HarnessMode::Complete,
            HarnessTurnMode::Conversation => HarnessMode::Conversation,
        }
    }

    pub fn provider_turn_mode_for(mode: HarnessTurnMode) -> ProviderTurnMode {
        match mode {
            HarnessTurnMode::Execute => ProviderTurnMode::Aggregated,
            HarnessTurnMode::Plan
            | HarnessTurnMode::Repair
            | HarnessTurnMode::Blocked
            | HarnessTurnMode::Complete
            | HarnessTurnMode::Conversation => ProviderTurnMode::Streaming,
        }
    }

    pub fn completion_surface_policy_for(mode: HarnessTurnMode) -> CompletionSurfacePolicy {
        match mode {
            HarnessTurnMode::Plan | HarnessTurnMode::Conversation => {
                CompletionSurfacePolicy::Conversation
            }
            HarnessTurnMode::Execute
            | HarnessTurnMode::Repair
            | HarnessTurnMode::Blocked
            | HarnessTurnMode::Complete => CompletionSurfacePolicy::Execute,
        }
    }

    pub fn map_delegation_mode(mode: Option<HarnessDelegationMode>) -> DelegationMode {
        match mode.unwrap_or_default() {
            HarnessDelegationMode::Disabled => DelegationMode::Disabled,
            HarnessDelegationMode::Subagent => DelegationMode::Subagent,
            HarnessDelegationMode::Swarm => DelegationMode::Swarm,
        }
    }

    pub fn build_task_spec_from_node(
        &self,
        run_id: &str,
        session_id: &str,
        depth: u32,
        node: &TaskNode,
        kind: TaskKind,
        description: Option<String>,
    ) -> TaskSpec {
        TaskSpec {
            task_id: format!("{}:{}", run_id, node.task_node_id),
            parent_session_id: session_id.to_string(),
            depth,
            kind,
            description: description.or_else(|| node.title.clone()),
            write_scope: node.write_scope.clone(),
            target_artifacts: node.target_artifacts.clone(),
            result_contract: node.result_contract.clone(),
            metadata: Default::default(),
        }
    }

    pub fn build_swarm_plan_from_node(&self, node: &TaskNode) -> Option<SwarmPlan> {
        self.build_swarm_plan(
            node.delegation_mode.clone(),
            node.parallelism_budget,
            node.swarm_budget,
            node.target_artifacts.clone(),
            node.write_scope.clone(),
            node.result_contract.clone(),
            false,
        )
    }

    pub fn build_swarm_plan(
        &self,
        delegation_mode: Option<HarnessDelegationMode>,
        parallelism_budget: Option<u32>,
        swarm_budget: Option<u32>,
        targets: Vec<String>,
        write_scope: Vec<String>,
        result_contract: Vec<String>,
        validation_mode: bool,
    ) -> Option<SwarmPlan> {
        if !matches!(delegation_mode, Some(HarnessDelegationMode::Swarm)) {
            return None;
        }
        if targets.is_empty() {
            return None;
        }
        Some(SwarmPlan {
            budget: SwarmBudget {
                parallelism_budget: parallelism_budget.or(swarm_budget),
            },
            targets,
            write_scope,
            result_contract,
            validation_mode,
        })
    }

    pub fn build_context(
        &self,
        session_id: String,
        working_dir: std::path::PathBuf,
        mode: HarnessTurnMode,
        delegation_mode: Option<HarnessDelegationMode>,
        max_turns: u32,
        write_scope: Vec<String>,
        target_artifacts: Vec<String>,
        result_contract: Vec<String>,
    ) -> HarnessContext {
        let delegation = DelegationRuntimeState::new(
            Self::map_delegation_mode(delegation_mode),
            0,
            agime::agents::harness::bounded_subagent_depth_from_env(),
            write_scope.clone(),
            target_artifacts.clone(),
            result_contract.clone(),
        );
        let harness_mode = Self::map_turn_mode(mode.clone());
        let provider_turn_mode = Self::provider_turn_mode_for(mode.clone());
        let completion_surface_policy = Self::completion_surface_policy_for(mode);
        let context = HarnessContext::new(
            session_id,
            working_dir,
            agime::config::AgimeMode::Auto,
            max_turns,
            None,
            harness_mode,
            agime::agents::CoordinatorExecutionMode::SingleWorker,
            provider_turn_mode,
            completion_surface_policy,
            delegation,
            write_scope,
            target_artifacts,
            result_contract,
            false,
            Vec::new(),
            Vec::new(),
            None,
            self.task_runtime.clone(),
            None,
        );
        context
    }

    fn task_string_field(task: &AgentTask, key: &str) -> Option<String> {
        task.content
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(|value| value.to_string())
    }

    fn task_u32_field(task: &AgentTask, key: &str) -> Option<u32> {
        task.content
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .map(|value| value.min(u32::MAX as u64) as u32)
    }

    fn task_bool_field(task: &AgentTask, key: &str) -> Option<bool> {
        task.content.get(key).and_then(serde_json::Value::as_bool)
    }

    fn task_string_vec_field(task: &AgentTask, key: &str) -> Vec<String> {
        task.content
            .get(key)
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn task_delegation_mode_field(task: &AgentTask, key: &str) -> Option<HarnessDelegationMode> {
        match task.content.get(key).and_then(serde_json::Value::as_str) {
            Some(value) if value.eq_ignore_ascii_case("swarm") => {
                Some(HarnessDelegationMode::Swarm)
            }
            Some(value) if value.eq_ignore_ascii_case("subagent") => {
                Some(HarnessDelegationMode::Subagent)
            }
            Some(value) if value.eq_ignore_ascii_case("disabled") => {
                Some(HarnessDelegationMode::Disabled)
            }
            _ => None,
        }
    }

    pub fn parse_subagent_runtime(&self, task: &AgentTask) -> Option<ServerDelegationContext> {
        let task_role = Self::task_string_field(task, "task_role")
            .or_else(|| Self::task_string_field(task, "content.task_role"))
            .unwrap_or_default();
        let explicit_depth = Self::task_u32_field(task, "subagent_depth");
        let explicit_max_depth = Self::task_u32_field(task, "subagent_max_depth");
        let explicit_write_scope = Self::task_string_vec_field(task, "subagent_write_scope");
        let top_level_write_scope = Self::task_string_vec_field(task, "write_scope");
        let target_artifacts = {
            let values = Self::task_string_vec_field(task, "target_artifacts");
            if values.is_empty() {
                Self::task_string_vec_field(task, "subagent_target_artifacts")
            } else {
                values
            }
        };
        let result_contract = {
            let values = Self::task_string_vec_field(task, "result_contract");
            if values.is_empty() {
                Self::task_string_vec_field(task, "subagent_result_contract")
            } else {
                values
            }
        };
        let delegation_mode = Self::task_delegation_mode_field(task, "delegation_mode")
            .or_else(|| Self::task_delegation_mode_field(task, "subagent_delegation_mode"));
        let parallelism_budget = Self::task_u32_field(task, "parallelism_budget")
            .or_else(|| Self::task_u32_field(task, "subagent_parallelism_budget"));
        let swarm_budget = Self::task_u32_field(task, "swarm_budget")
            .or_else(|| Self::task_u32_field(task, "subagent_swarm_budget"));
        let validation_mode = Self::task_bool_field(task, "validation_mode")
            .or_else(|| Self::task_bool_field(task, "subagent_validation_mode"))
            .unwrap_or(task_role == "validation_worker");
        let explicit_parent_run = Self::task_string_field(task, "subagent_parent_run_id");
        let explicit_parent_task_node =
            Self::task_string_field(task, "subagent_parent_task_node_id");
        let explicit_spec_name = Self::task_string_field(task, "subagent_spec_name");
        let task_graph_id = Self::task_string_field(task, "task_graph_id")
            .or_else(|| Self::task_string_field(task, "subagent_task_graph_id"));
        if task_role == "subagent_worker"
            || task_role == "validation_worker"
            || explicit_depth.is_some()
            || explicit_parent_run.is_some()
            || explicit_parent_task_node.is_some()
        {
            return Some(ServerDelegationContext {
                delegation_mode: delegation_mode.unwrap_or(HarnessDelegationMode::Subagent),
                depth: explicit_depth.unwrap_or(1),
                max_depth: explicit_max_depth.unwrap_or(2).max(1),
                write_scope: if explicit_write_scope.is_empty() {
                    top_level_write_scope
                } else {
                    explicit_write_scope
                },
                target_artifacts,
                result_contract,
                parallelism_budget,
                swarm_budget,
                validation_mode,
                parent_run_id: explicit_parent_run,
                parent_task_node_id: explicit_parent_task_node
                    .or_else(|| Self::task_string_field(task, "task_node_id")),
                task_graph_id,
                spec_name: explicit_spec_name
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "general-worker".to_string()),
            });
        }
        None
    }

    pub fn build_child_task_request(
        &self,
        args: &serde_json::Value,
        runtime: &ServerDelegationContext,
        options: Option<&ServerChildTaskOptions>,
    ) -> Result<ChildTaskRequest, String> {
        let instructions = args
            .get("instructions")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let subrecipe = args
            .get("subrecipe")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let summary_only = args
            .get("summary")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let validation_requested = args
            .get("validation")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let requested_extensions = args
            .get("extensions")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
            })
            .filter(|items| !items.is_empty());
        let requested_write_scope = args
            .get("write_scope")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let instructions = match (instructions, subrecipe.clone()) {
            (Some(text), Some(recipe)) => format!("Subagent role `{}`.\n\n{}", recipe, text),
            (Some(text), None) => text,
            (None, Some(recipe)) => format!(
                "Execute the bounded subagent role `{}` and return the result.",
                recipe
            ),
            (None, None) => {
                return Err(
                    "Error: subagent call requires `instructions` or `subrecipe`".to_string(),
                );
            }
        };

        Ok(ChildTaskRequest {
            instructions,
            spec_name: options
                .and_then(|value| value.spec_name.clone())
                .or(subrecipe)
                .or_else(|| Some(runtime.spec_name.clone())),
            summary_only,
            validation_mode: options.map(|value| value.validation_mode).unwrap_or(false)
                || validation_requested
                || runtime.validation_mode,
            requested_extensions,
            requested_write_scope,
            delegation_mode: Self::map_delegation_mode(Some(
                options
                    .map(|value| value.delegation_mode.clone())
                    .unwrap_or(runtime.delegation_mode.clone()),
            )),
            target_artifacts: options
                .map(|value| value.target_artifacts.clone())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| runtime.target_artifacts.clone()),
            result_contract: options
                .map(|value| value.result_contract.clone())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| runtime.result_contract.clone()),
            parallelism_budget: options
                .and_then(|value| value.parallelism_budget)
                .or(runtime.parallelism_budget),
            swarm_budget: options
                .and_then(|value| value.swarm_budget)
                .or(runtime.swarm_budget),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agime_team::models::{AgentTask, TaskStatus, TaskType};
    use chrono::Utc;

    #[test]
    fn builds_swarm_plan_from_swarm_node() {
        let adapter = ServerHarnessAdapter {
            db: None,
            task_runtime: TaskRuntime::shared(),
        };
        let node = TaskNode {
            task_node_id: "node-1".to_string(),
            delegation_mode: Some(HarnessDelegationMode::Swarm),
            parallelism_budget: Some(2),
            target_artifacts: vec!["src/a.rs".to_string()],
            write_scope: vec!["src/".to_string()],
            result_contract: vec!["src/a.rs".to_string()],
            ..TaskNode::default()
        };
        let plan = adapter.build_swarm_plan_from_node(&node).expect("plan");
        assert_eq!(plan.targets, vec!["src/a.rs".to_string()]);
    }

    #[test]
    fn builds_validation_swarm_plan_when_targets_exist() {
        let adapter = ServerHarnessAdapter {
            db: None,
            task_runtime: TaskRuntime::shared(),
        };
        let plan = adapter
            .build_swarm_plan(
                Some(HarnessDelegationMode::Swarm),
                Some(2),
                None,
                vec!["docs/report.md".to_string()],
                vec!["docs/".to_string()],
                vec!["docs/report.md".to_string()],
                true,
            )
            .expect("plan");
        assert!(plan.validation_mode);
        assert_eq!(plan.targets, vec!["docs/report.md".to_string()]);
    }

    #[test]
    fn parses_subagent_runtime_from_task_payload() {
        let adapter = ServerHarnessAdapter {
            db: None,
            task_runtime: TaskRuntime::shared(),
        };
        let task = AgentTask {
            id: "task-1".to_string(),
            team_id: "team-1".to_string(),
            agent_id: "agent-1".to_string(),
            submitter_id: "user-1".to_string(),
            approver_id: None,
            task_type: TaskType::Chat,
            content: serde_json::json!({
                "task_role": "subagent_worker",
                "delegation_mode": "swarm",
                "subagent_depth": 1,
                "subagent_max_depth": 3,
                "write_scope": ["src/"],
                "target_artifacts": ["src/a.rs"],
                "result_contract": ["src/a.rs"],
                "parallelism_budget": 2,
                "task_graph_id": "graph-1"
            }),
            status: TaskStatus::Pending,
            priority: 0,
            submitted_at: Utc::now(),
            approved_at: None,
            started_at: None,
            completed_at: None,
            error_message: None,
        };
        let runtime = adapter.parse_subagent_runtime(&task).expect("runtime");
        assert_eq!(runtime.delegation_mode, HarnessDelegationMode::Swarm);
        assert_eq!(runtime.parallelism_budget, Some(2));
        assert_eq!(runtime.task_graph_id.as_deref(), Some("graph-1"));
    }

    #[test]
    fn builds_child_task_request_from_tool_args() {
        let adapter = ServerHarnessAdapter {
            db: None,
            task_runtime: TaskRuntime::shared(),
        };
        let runtime = ServerDelegationContext {
            delegation_mode: HarnessDelegationMode::Swarm,
            spec_name: "general-worker".to_string(),
            target_artifacts: vec!["src/out.md".to_string()],
            result_contract: vec!["src/out.md".to_string()],
            parallelism_budget: Some(2),
            ..Default::default()
        };
        let args = serde_json::json!({
            "instructions": "write the file",
            "summary": true,
            "write_scope": ["src/"],
            "extensions": ["developer"]
        });
        let request = adapter
            .build_child_task_request(&args, &runtime, None)
            .expect("request");
        assert_eq!(request.target_artifacts, vec!["src/out.md".to_string()]);
        assert_eq!(request.parallelism_budget, Some(2));
        assert_eq!(request.requested_write_scope, vec!["src/".to_string()]);
    }
}
