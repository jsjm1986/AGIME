use std::collections::HashSet;

use super::runtime;

#[derive(Debug, Clone, Default)]
pub struct SwarmBootstrapRequest {
    pub goal: Option<String>,
    pub context: Option<String>,
    pub locked_target: Option<String>,
    pub missing_artifacts: Vec<String>,
    pub node_target_artifacts: Vec<String>,
    pub node_result_contract: Vec<String>,
    pub parallelism_budget: Option<u32>,
    pub swarm_budget: Option<u32>,
    pub parent_write_scope: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SwarmBootstrapCall {
    pub target_artifact: Option<String>,
    pub spec_name: String,
    pub instructions: String,
    pub write_scope: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SwarmBootstrapExecution {
    pub target_artifact: Option<String>,
    pub success: bool,
    pub summary: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SwarmBootstrapDecision {
    pub produced_file_delta: bool,
    pub accepted_targets: Vec<String>,
    pub summary_text: Option<String>,
    pub disable_swarm_for_run: bool,
    pub downgrade_message: Option<String>,
}

pub fn build_recursive_swarm_downgrade_message(
    locked_target: Option<&str>,
    missing_artifacts: &[String],
    err_text: &str,
) -> String {
    let target_hint = locked_target
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| missing_artifacts.first().map(String::as_str))
        .unwrap_or("the next concrete missing deliverable");
    format!(
        "Recursive delegation failed in this run: {}. Do not call `subagent` again in this run. Continue with a single-worker path and directly create or materially update {}.",
        err_text,
        target_hint
    )
}

fn bootstrap_spec_for_target(target: &str) -> &'static str {
    let lower = target.to_ascii_lowercase();
    if lower.ends_with(".csv") || lower.ends_with(".json") {
        "fill"
    } else if lower.ends_with(".md") || lower.ends_with(".html") || lower.ends_with(".txt") {
        "artifact-draft"
    } else {
        "general-worker"
    }
}

fn bootstrap_instructions(req: &SwarmBootstrapRequest, target: Option<&str>) -> String {
    let mut lines = Vec::new();
    if let Some(goal) = req.goal.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        lines.push(format!("Mission goal: {}", goal));
    }
    if let Some(context) = req.context.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        lines.push(format!("Mission context: {}", context));
    }
    if let Some(target) = target.map(str::trim).filter(|v| !v.is_empty()) {
        lines.push(format!(
            "Bounded objective: create or materially update `{}` in this run.",
            target
        ));
    }
    if !req.parent_write_scope.is_empty() {
        lines.push(format!(
            "Write only inside this scope: {}",
            req.parent_write_scope.join(", ")
        ));
    }
    lines.push(
        "Use tool-backed execution. Prefer producing the real target artifact now over writing analysis-only notes."
            .to_string(),
    );
    lines.push(
        "If the full deliverable cannot be completed in one pass, still leave behind the strongest reusable partial artifact for the same target file."
            .to_string(),
    );
    lines.join("\n")
}

fn bootstrap_targets(req: &SwarmBootstrapRequest, max_targets: usize) -> Vec<String> {
    let mut ordered = Vec::new();
    let mut seen = HashSet::new();
    fn push_target(ordered: &mut Vec<String>, seen: &mut HashSet<String>, value: &str) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return;
        }
        let Some(normalized) = runtime::normalize_relative_workspace_path(trimmed) else {
            return;
        };
        let key = normalized.to_ascii_lowercase();
        if seen.insert(key) {
            ordered.push(normalized);
        }
    }

    if let Some(target) = req.locked_target.as_deref() {
        push_target(&mut ordered, &mut seen, target);
    }
    for item in &req.missing_artifacts {
        push_target(&mut ordered, &mut seen, item);
        if ordered.len() >= max_targets {
            return ordered;
        }
    }
    for item in req
        .node_target_artifacts
        .iter()
        .chain(req.node_result_contract.iter())
    {
        push_target(&mut ordered, &mut seen, item);
        if ordered.len() >= max_targets {
            return ordered;
        }
    }
    ordered
}

pub fn build_swarm_bootstrap_calls(req: &SwarmBootstrapRequest) -> Vec<SwarmBootstrapCall> {
    let max_targets = req
        .parallelism_budget
        .or(req.swarm_budget)
        .map(|value| value.clamp(1, 3) as usize)
        .unwrap_or(1);
    let targets = bootstrap_targets(req, max_targets);
    if targets.is_empty() {
        return vec![SwarmBootstrapCall {
            target_artifact: None,
            spec_name: "general-worker".to_string(),
            instructions: bootstrap_instructions(req, req.locked_target.as_deref()),
            write_scope: req.parent_write_scope.clone(),
        }];
    }
    let mut seen = HashSet::new();
    targets
        .into_iter()
        .filter(|target| seen.insert(target.to_ascii_lowercase()))
        .map(|target| SwarmBootstrapCall {
            target_artifact: Some(target.clone()),
            spec_name: bootstrap_spec_for_target(&target).to_string(),
            instructions: bootstrap_instructions(req, Some(target.as_str())),
            write_scope: vec![target],
        })
        .collect()
}

pub fn decide_swarm_bootstrap_outcome(
    req: &SwarmBootstrapRequest,
    executions: &[SwarmBootstrapExecution],
    changed_targets: &[String],
) -> SwarmBootstrapDecision {
    let accepted_targets = changed_targets
        .iter()
        .filter_map(|target| runtime::normalize_relative_workspace_path(target))
        .collect::<Vec<_>>();
    let produced_file_delta = !accepted_targets.is_empty();
    let summary_parts = executions
        .iter()
        .filter_map(|execution| execution.summary.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let summary_text = if summary_parts.is_empty() {
        None
    } else {
        Some(summary_parts.join("\n\n---\n\n"))
    };
    if produced_file_delta {
        return SwarmBootstrapDecision {
            produced_file_delta,
            accepted_targets,
            summary_text,
            disable_swarm_for_run: false,
            downgrade_message: None,
        };
    }

    let first_error = executions
        .iter()
        .filter_map(|execution| execution.error.as_deref())
        .map(str::trim)
        .find(|text| !text.is_empty())
        .map(str::to_string);
    let success_count = executions.iter().filter(|execution| execution.success).count();
    let target_hint = req
        .locked_target
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| req.missing_artifacts.first().map(String::as_str))
        .unwrap_or("the next concrete missing deliverable");
    let downgrade_message = if let Some(err_text) = first_error {
        Some(format!(
            "Automatic swarm bootstrap failed: {}. Recursive delegation is now disabled for this run. Continue with a single-worker execution path and directly work on {}.",
            err_text, target_hint
        ))
    } else if success_count == 0 {
        Some(format!(
            "Automatic swarm bootstrap produced no usable subagent result. Recursive delegation is now disabled for this run. Continue with a single-worker execution path and directly work on {}.",
            target_hint
        ))
    } else {
        None
    };

    SwarmBootstrapDecision {
        produced_file_delta: false,
        accepted_targets,
        summary_text,
        disable_swarm_for_run: downgrade_message.is_some(),
        downgrade_message,
    }
}
