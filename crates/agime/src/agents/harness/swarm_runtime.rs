use std::collections::HashSet;

use crate::agents::harness::task_runtime::{TaskKind, TaskRuntime, TaskRuntimeHost, TaskSpec};
use uuid::Uuid;

use super::delegation::{DelegationRuntimeState, SwarmOutcome, SwarmPlan};

#[derive(Debug, Clone, Default)]
pub struct SwarmWorkerSpec {
    pub task_id: String,
    pub target_artifact: String,
    pub write_scope: Vec<String>,
    pub result_contract: Vec<String>,
    pub validation_mode: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SwarmRuntimePlan {
    pub workers: Vec<SwarmWorkerSpec>,
    pub parallelism_budget: u32,
    pub validation_mode: bool,
}

fn target_within_scope(target: &str, scope: &str) -> bool {
    let trimmed_target = target.trim();
    let trimmed_scope = scope.trim();
    if trimmed_target.is_empty() || trimmed_scope.is_empty() {
        return false;
    }
    if trimmed_target.eq_ignore_ascii_case(trimmed_scope) {
        return true;
    }
    let normalized_scope = trimmed_scope.trim_end_matches('/');
    if normalized_scope.is_empty() {
        return false;
    }
    trimmed_target.to_ascii_lowercase().starts_with(&format!(
        "{}{}",
        normalized_scope.to_ascii_lowercase(),
        "/"
    ))
}

fn worker_write_scope(
    target: &str,
    delegation: &DelegationRuntimeState,
    swarm_plan: &SwarmPlan,
) -> Vec<String> {
    let base_scope = if swarm_plan.write_scope.is_empty() {
        &delegation.write_scope
    } else {
        &swarm_plan.write_scope
    };
    if base_scope.is_empty() {
        return vec![target.to_string()];
    }
    if base_scope
        .iter()
        .any(|scope| target_within_scope(target, scope))
    {
        return vec![target.to_string()];
    }
    base_scope.clone()
}

fn worker_result_contract(
    target: &str,
    delegation: &DelegationRuntimeState,
    swarm_plan: &SwarmPlan,
) -> Vec<String> {
    let base_contract = if swarm_plan.result_contract.is_empty() {
        &delegation.result_contract
    } else {
        &swarm_plan.result_contract
    };
    let scoped = base_contract
        .iter()
        .filter(|item| {
            target_within_scope(target, item) || item.trim().eq_ignore_ascii_case(target)
        })
        .cloned()
        .collect::<Vec<_>>();
    if !scoped.is_empty() {
        scoped
    } else if base_contract.is_empty() {
        vec![target.to_string()]
    } else {
        base_contract.clone()
    }
}

pub fn build_bounded_swarm_plan(
    session_id: &str,
    delegation: &DelegationRuntimeState,
    swarm_plan: &SwarmPlan,
    validation_mode: bool,
) -> SwarmRuntimePlan {
    let budget = swarm_plan
        .budget
        .parallelism_budget
        .unwrap_or(1)
        .clamp(1, 4);
    let mut workers = Vec::new();
    let mut seen = HashSet::new();

    for (idx, target) in swarm_plan.targets.iter().enumerate() {
        let trimmed = target.trim();
        if trimmed.is_empty() || !seen.insert(trimmed.to_ascii_lowercase()) {
            continue;
        }
        workers.push(SwarmWorkerSpec {
            task_id: format!("swarm_{}_{}_{}", session_id, idx, Uuid::new_v4().simple()),
            target_artifact: trimmed.to_string(),
            write_scope: worker_write_scope(trimmed, delegation, swarm_plan),
            result_contract: worker_result_contract(trimmed, delegation, swarm_plan),
            validation_mode,
        });
    }

    SwarmRuntimePlan {
        workers,
        parallelism_budget: budget,
        validation_mode,
    }
}

pub fn worker_task_spec(session_id: &str, depth: u32, worker: &SwarmWorkerSpec) -> TaskSpec {
    TaskSpec {
        task_id: worker.task_id.clone(),
        parent_session_id: session_id.to_string(),
        depth,
        kind: if worker.validation_mode {
            TaskKind::ValidationWorker
        } else {
            TaskKind::SwarmWorker
        },
        description: Some(format!(
            "bounded swarm worker for {}",
            worker.target_artifact
        )),
        write_scope: worker.write_scope.clone(),
        target_artifacts: vec![worker.target_artifact.clone()],
        result_contract: worker.result_contract.clone(),
        metadata: Default::default(),
    }
}

pub fn decide_bounded_swarm_outcome(
    required_targets: &[String],
    produced_targets: &[String],
    summary: Option<String>,
) -> SwarmOutcome {
    let accepted_targets = produced_targets
        .iter()
        .filter(|target| required_targets.contains(*target))
        .cloned()
        .collect::<Vec<_>>();

    let produced_delta = !accepted_targets.is_empty();
    SwarmOutcome {
        produced_delta,
        accepted_targets,
        summary,
        executed_workers: produced_targets.len(),
        downgrade_message: if !produced_delta {
            Some(
                "Bounded swarm produced no accepted target delta. Fall back to a single-worker path."
                    .to_string(),
            )
        } else {
            None
        },
    }
}

pub async fn bootstrap_validation_workers(
    runtime: &TaskRuntime,
    session_id: &str,
    depth: u32,
    plan: &SwarmRuntimePlan,
) -> anyhow::Result<Vec<String>> {
    let mut spawned = Vec::new();
    for worker in &plan.workers {
        let spec = worker_task_spec(session_id, depth, worker);
        let handle = runtime.spawn_task(spec).await?;
        spawned.push(handle.task_id);
    }
    Ok(spawned)
}
