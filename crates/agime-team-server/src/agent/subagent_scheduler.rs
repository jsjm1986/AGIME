use std::collections::HashSet;

use super::runtime;

#[derive(Debug, Clone, Default)]
pub struct SubagentBootstrapRequest {
    pub goal: Option<String>,
    pub context: Option<String>,
    pub locked_target: Option<String>,
    pub missing_artifacts: Vec<String>,
    pub node_target_artifacts: Vec<String>,
    pub node_result_contract: Vec<String>,
    pub parent_write_scope: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SubagentBootstrapCall {
    pub target_artifact: Option<String>,
    pub spec_name: String,
    pub instructions: String,
    pub write_scope: Vec<String>,
}

pub fn build_subagent_bootstrap_call(req: &SubagentBootstrapRequest) -> SubagentBootstrapCall {
    let target_artifact = select_target(req);
    let write_scope = target_artifact
        .clone()
        .map(|value| vec![value])
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| req.parent_write_scope.clone());
    let target_hint = target_artifact
        .as_deref()
        .or_else(|| req.missing_artifacts.first().map(String::as_str))
        .unwrap_or("the current bounded deliverable");
    let mut lines = Vec::new();
    if let Some(goal) = req.goal.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        lines.push(format!("Mission goal: {}", goal));
    }
    if let Some(context) = req
        .context
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Context: {}", context));
    }
    lines.push(format!(
        "Act as one bounded helper subagent. Your job is to make direct progress on {}.",
        target_hint
    ));
    if !req.missing_artifacts.is_empty() {
        lines.push(format!(
            "Remaining required deliverables: {}",
            req.missing_artifacts.join(", ")
        ));
    }
    if !req.node_result_contract.is_empty() {
        lines.push(format!(
            "Current node result contract: {}",
            req.node_result_contract.join(", ")
        ));
    }
    lines.push(
        "Create or materially update one bounded deliverable, then return a concise final summary. Do not fan out more helpers."
            .to_string(),
    );
    lines.push(
        "Prefer direct file creation or update over broad analysis. If you cannot complete the bounded deliverable, return one concrete blocker."
            .to_string(),
    );

    SubagentBootstrapCall {
        target_artifact,
        spec_name: "general-worker".to_string(),
        instructions: lines.join("\n"),
        write_scope,
    }
}

pub fn build_subagent_downgrade_message(
    locked_target: Option<&str>,
    missing_artifacts: &[String],
    err_text: &str,
) -> String {
    let target_hint = locked_target
        .and_then(runtime::normalize_relative_workspace_path)
        .or_else(|| {
            missing_artifacts
                .iter()
                .find_map(|value| runtime::normalize_relative_workspace_path(value))
        })
        .unwrap_or_else(|| "the current bounded deliverable".to_string());
    format!(
        "Automatic helper subagent failed in this run: {}. Do not call `subagent` again in this run. Continue on the main worker path and directly complete {}.",
        err_text, target_hint
    )
}

fn select_target(req: &SubagentBootstrapRequest) -> Option<String> {
    let mut ordered = Vec::new();
    let mut seen = HashSet::new();
    for candidate in req
        .locked_target
        .iter()
        .chain(req.node_target_artifacts.iter())
        .chain(req.node_result_contract.iter())
        .chain(req.missing_artifacts.iter())
    {
        let Some(normalized) = runtime::normalize_relative_workspace_path(candidate) else {
            continue;
        };
        let key = normalized.to_ascii_lowercase();
        if seen.insert(key) {
            ordered.push(normalized);
        }
    }
    ordered.into_iter().next()
}
