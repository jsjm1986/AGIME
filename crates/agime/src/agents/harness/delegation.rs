use crate::session::session_manager::SessionType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DelegationMode {
    #[default]
    Disabled,
    Subagent,
    Swarm,
}

#[derive(Debug, Clone, Default)]
pub struct SwarmBudget {
    pub parallelism_budget: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct SwarmPlan {
    pub budget: SwarmBudget,
    pub targets: Vec<String>,
    pub write_scope: Vec<String>,
    pub result_contract: Vec<String>,
    pub validation_mode: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SwarmOutcome {
    pub produced_delta: bool,
    pub accepted_targets: Vec<String>,
    pub summary: Option<String>,
    pub executed_workers: usize,
    pub downgrade_message: Option<String>,
}

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

#[derive(Debug, Clone, Default)]
pub struct DelegationRuntimeState {
    pub mode: DelegationMode,
    pub current_depth: u32,
    pub max_depth: u32,
    pub write_scope: Vec<String>,
    pub target_artifacts: Vec<String>,
    pub result_contract: Vec<String>,
    pub subagent_calls_this_turn: usize,
    pub swarm_calls_this_run: usize,
    pub downgrade_message: Option<String>,
    pub swarm_disabled_for_run: bool,
}

impl DelegationRuntimeState {
    pub fn new(
        mode: DelegationMode,
        current_depth: u32,
        max_depth: u32,
        write_scope: Vec<String>,
        target_artifacts: Vec<String>,
        result_contract: Vec<String>,
    ) -> Self {
        Self {
            mode,
            current_depth,
            max_depth,
            write_scope,
            target_artifacts,
            result_contract,
            subagent_calls_this_turn: 0,
            swarm_calls_this_run: 0,
            downgrade_message: None,
            swarm_disabled_for_run: false,
        }
    }

    pub fn for_session_type(session_type: SessionType) -> Self {
        let current_depth = if matches!(session_type, SessionType::SubAgent) {
            1
        } else {
            0
        };
        Self::new(
            DelegationMode::Subagent,
            current_depth,
            bounded_subagent_depth_from_env(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    pub fn reset_turn(&mut self) {
        self.subagent_calls_this_turn = 0;
        self.downgrade_message = None;
    }

    pub fn can_delegate_subagent(&self) -> bool {
        self.mode != DelegationMode::Disabled && self.current_depth < self.max_depth
    }

    pub fn note_subagent_call(&mut self) {
        self.subagent_calls_this_turn = self.subagent_calls_this_turn.saturating_add(1);
    }

    pub fn note_swarm_call(&mut self) {
        self.swarm_calls_this_run = self.swarm_calls_this_run.saturating_add(1);
    }

    pub fn note_subagent_failure(&mut self, reason: impl Into<String>) {
        let target_hint = self
            .target_artifacts
            .first()
            .cloned()
            .or_else(|| self.result_contract.first().cloned())
            .unwrap_or_else(|| "the current bounded deliverable".to_string());
        self.downgrade_message = Some(format!(
            "Automatic helper subagent failed in this run: {}. Do not call `subagent` again in this run. Continue on the main worker path and directly complete {}.",
            reason.into(),
            target_hint
        ));
    }

    pub fn can_delegate_swarm(&self) -> bool {
        self.mode == DelegationMode::Swarm && !self.swarm_disabled_for_run
    }

    pub fn note_swarm_fallback(&mut self, reason: impl Into<String>) {
        self.swarm_disabled_for_run = true;
        self.downgrade_message = Some(format!(
            "Automatic swarm execution fell back to single-worker mode: {}. Do not fan out more workers in this run.",
            reason.into()
        ));
    }
}

pub fn bounded_subagent_depth_from_env() -> u32 {
    std::env::var("AGIME_HARNESS_MAX_SUBAGENT_DEPTH")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1)
}

fn normalize_hint_path(path: &str) -> Option<String> {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn first_target_hint(
    locked_target: Option<&str>,
    missing_artifacts: &[String],
    fallback: &str,
) -> String {
    locked_target
        .and_then(normalize_hint_path)
        .or_else(|| {
            missing_artifacts
                .iter()
                .find_map(|value| normalize_hint_path(value))
        })
        .unwrap_or_else(|| fallback.to_string())
}

fn select_bootstrap_target(req: &SubagentBootstrapRequest) -> Option<String> {
    for candidate in req
        .locked_target
        .iter()
        .chain(req.node_target_artifacts.iter())
        .chain(req.node_result_contract.iter())
        .chain(req.missing_artifacts.iter())
    {
        if let Some(normalized) = normalize_hint_path(candidate) {
            return Some(normalized);
        }
    }
    None
}

pub fn build_subagent_bootstrap_call(
    req: &SubagentBootstrapRequest,
) -> Option<SubagentBootstrapCall> {
    let target_artifact = select_bootstrap_target(req);
    let target_hint = target_artifact
        .as_deref()
        .or_else(|| req.missing_artifacts.first().map(String::as_str))?;
    let write_scope = target_artifact
        .clone()
        .map(|value| vec![value])
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| req.parent_write_scope.clone());
    let mut lines = Vec::new();
    if let Some(goal) = req
        .goal
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Task goal: {}", goal));
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

    Some(SubagentBootstrapCall {
        target_artifact,
        spec_name: "general-worker".to_string(),
        instructions: lines.join("\n"),
        write_scope,
    })
}

pub fn build_subagent_downgrade_message(
    locked_target: Option<&str>,
    missing_artifacts: &[String],
    err_text: &str,
) -> String {
    let target_hint = first_target_hint(
        locked_target,
        missing_artifacts,
        "the current bounded deliverable",
    );
    format!(
        "Automatic helper subagent failed in this run: {}. Do not call `subagent` again in this run. Continue on the main worker path and directly complete {}.",
        err_text, target_hint
    )
}

pub fn build_swarm_downgrade_message(
    locked_target: Option<&str>,
    missing_artifacts: &[String],
    err_text: &str,
) -> String {
    let target_hint = first_target_hint(
        locked_target,
        missing_artifacts,
        "the next concrete missing deliverable",
    );
    format!(
        "Recursive delegation failed in this run: {}. Do not call `subagent` again in this run. Continue with a single-worker path and directly create or materially update {}.",
        err_text,
        target_hint
    )
}
