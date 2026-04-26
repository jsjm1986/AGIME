use crate::conversation::message::{Message, MessageContent, ToolRequest};
use crate::conversation::Conversation;
use crate::session::session_manager::SessionType;
use rmcp::model::CallToolRequestParams;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplicitDelegationIntent {
    Subagent,
    Swarm,
}

#[derive(Debug, Clone)]
pub struct DelegationBootstrapPlan {
    pub request: ToolRequest,
    pub transition_reason: String,
    pub inline_notice: Option<String>,
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
    let normalized = path
        .trim()
        .replace('\\', "/")
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\''
                    | '`'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '。'
                    | '！'
                    | '？'
                    | ':'
                    | '.'
                    | ','
                    | ';'
                    | '!'
                    | '?'
            )
        })
        .trim()
        .to_string();
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
        .or_else(|| req.missing_artifacts.first().map(String::as_str))
        .unwrap_or("the current bounded deliverable");
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
    lines.push(
        "You are a worker, not the leader. Do not chat, do not write meta commentary, and do not recurse into more helpers unless the protocol explicitly allows it."
            .to_string(),
    );
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
        "Create or materially update one bounded deliverable, then return a concise final summary. Your final message is mandatory and must clearly state what you changed, what result you produced, or the single concrete blocker."
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
        err_text, target_hint
    )
}

fn latest_user_text(conversation: &Conversation) -> String {
    conversation
        .messages()
        .iter()
        .rev()
        .find(|message| message.role == rmcp::model::Role::User)
        .map(|message| {
            message
                .content
                .iter()
                .filter_map(|content| match content {
                    MessageContent::Text(text) => Some(text.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string()
        })
        .unwrap_or_default()
}

fn latest_assistant_text(message: &Message) -> String {
    message
        .content
        .iter()
        .filter_map(|content| match content {
            MessageContent::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn normalized_stable_targets(values: &[String]) -> Vec<String> {
    let mut stable = Vec::new();
    for value in values {
        if let Some(normalized) = normalize_hint_path(value) {
            let stable_shape = normalized.contains(':')
                || normalized.contains('/')
                || normalized.contains('.')
                || normalized.contains('_')
                || normalized.contains('-');
            if stable_shape && !stable.iter().any(|existing| existing == &normalized) {
                stable.push(normalized);
            }
        }
    }
    stable
}

fn extract_stable_targets_from_request(text: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for token in text
        .split(|ch: char| {
            ch.is_whitespace() || matches!(ch, ',' | ';' | '，' | '；' | '(' | ')' | '（' | '）')
        })
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let trimmed = token.trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\''
                    | '`'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '。'
                    | '！'
                    | '？'
                    | ':'
                    | '.'
                    | ','
                    | ';'
                    | '!'
                    | '?'
            )
        });
        if trimmed.is_empty() {
            continue;
        }
        candidates.push(trimmed.to_string());
    }
    normalized_stable_targets(&candidates)
}

fn explicit_swarm_keywords(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("swarm")
        || lowered.contains("parallel")
        || lowered.contains("multiple workers")
        || lowered.contains("multiple worker")
        || lowered.contains("multi-agent")
        || text.contains("并行")
        || text.contains("多代理")
        || text.contains("多个 worker")
        || text.contains("多 worker")
}

fn explicit_subagent_keywords(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("subagent")
        || lowered.contains("one worker")
        || lowered.contains("single worker")
        || text.contains("子代理")
}

fn internal_delegation_target(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized.starts_with("channel:")
        || normalized.starts_with("thread:")
        || normalized.starts_with("session:")
        || normalized.starts_with("mailbox:")
}

pub fn detect_explicit_delegation_intent(
    conversation: &Conversation,
    delegation: &DelegationRuntimeState,
) -> Option<ExplicitDelegationIntent> {
    if delegation.current_depth > 0 {
        return None;
    }
    let request = latest_user_text(conversation);
    if request.is_empty() {
        return None;
    }
    if explicit_swarm_keywords(&request) {
        return Some(ExplicitDelegationIntent::Swarm);
    }
    if explicit_subagent_keywords(&request) && delegation.can_delegate_subagent() {
        return Some(ExplicitDelegationIntent::Subagent);
    }
    None
}

fn build_subagent_bootstrap_request(
    conversation: &Conversation,
    response: &Message,
    delegation: &DelegationRuntimeState,
    inline_notice: Option<String>,
    transition_reason: impl Into<String>,
) -> Option<DelegationBootstrapPlan> {
    let latest_request = latest_user_text(conversation);
    let latest_hint = latest_assistant_text(response);
    let bootstrap = build_subagent_bootstrap_call(&SubagentBootstrapRequest {
        goal: (!latest_request.trim().is_empty()).then_some(latest_request),
        context: (!latest_hint.trim().is_empty()).then_some(latest_hint),
        locked_target: delegation.target_artifacts.first().cloned(),
        missing_artifacts: normalized_stable_targets(&delegation.result_contract),
        node_target_artifacts: delegation.target_artifacts.clone(),
        node_result_contract: delegation.result_contract.clone(),
        parent_write_scope: delegation.write_scope.clone(),
    })?;
    let arguments = json!({
        "instructions": bootstrap.instructions,
        "summary": true,
    });
    Some(DelegationBootstrapPlan {
        request: ToolRequest {
            id: format!("auto_subagent_{}", Uuid::new_v4().simple()),
            tool_call: Ok(CallToolRequestParams {
                name: "subagent".into(),
                arguments: arguments.as_object().cloned(),
                meta: None,
                task: None,
            }),
            thought_signature: None,
        },
        transition_reason: transition_reason.into(),
        inline_notice,
    })
}

pub fn maybe_build_explicit_delegation_bootstrap_request(
    conversation: &Conversation,
    response: &Message,
    delegation: &mut DelegationRuntimeState,
) -> Option<DelegationBootstrapPlan> {
    match detect_explicit_delegation_intent(conversation, delegation)? {
        ExplicitDelegationIntent::Subagent => build_subagent_bootstrap_request(
            conversation,
            response,
            delegation,
            Some(
                "Explicit delegation request detected. Injected one bounded subagent bootstrap for this turn."
                    .to_string(),
            ),
            "explicit_subagent_bootstrap",
        ),
        ExplicitDelegationIntent::Swarm => {
            let latest_request = latest_user_text(conversation);
            let request_targets = extract_stable_targets_from_request(&latest_request)
                .into_iter()
                .filter(|item| !internal_delegation_target(item))
                .collect::<Vec<_>>();
            let stable_targets = if request_targets.len() >= 2 {
                request_targets
            } else {
                let mut merged = request_targets;
                for item in normalized_stable_targets(&delegation.target_artifacts)
                    .into_iter()
                    .chain(normalized_stable_targets(&delegation.result_contract).into_iter())
                {
                    if internal_delegation_target(&item)
                        || merged.iter().any(|existing| existing == &item)
                    {
                        continue;
                    }
                    merged.push(item);
                }
                merged
            };
            if stable_targets.len() >= 2 {
                let latest_hint = latest_assistant_text(response);
                let write_scope = delegation.write_scope.clone();
                let result_contract = delegation.result_contract.clone();
                let instructions = [
                    Some(
                        "Execute this turn as a bounded swarm. Spawn at least two workers and keep each worker scoped to one target."
                            .to_string(),
                    ),
                    (!latest_request.trim().is_empty())
                        .then_some(format!("Latest user request:\n{}", latest_request)),
                    (!latest_hint.trim().is_empty())
                        .then_some(format!("Leader planning hint:\n{}", latest_hint)),
                    Some(format!("Bounded targets: {}", stable_targets.join(", "))),
                    Some(format!(
                        "Result contract: {}",
                        if result_contract.is_empty() {
                            "none".to_string()
                        } else {
                            result_contract.join(", ")
                        }
                    )),
                    Some(format!(
                        "Write scope: {}",
                        if write_scope.is_empty() {
                            "none".to_string()
                        } else {
                            write_scope.join(", ")
                        }
                    )),
                    Some(
                        "Workers must act as execution workers, not planners, and each worker must return a concise final summary to the leader."
                            .to_string(),
                    ),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n\n");
                let arguments = json!({
                    "instructions": instructions,
                    "targets": stable_targets,
                    "write_scope": write_scope,
                    "result_contract": result_contract,
                    "summary": true,
                });
                Some(DelegationBootstrapPlan {
                    request: ToolRequest {
                        id: format!("auto_swarm_{}", Uuid::new_v4().simple()),
                        tool_call: Ok(CallToolRequestParams {
                            name: "swarm".into(),
                            arguments: arguments.as_object().cloned(),
                            meta: None,
                            task: None,
                        }),
                        thought_signature: None,
                    },
                    transition_reason: "explicit_swarm_bootstrap".to_string(),
                    inline_notice: Some(
                        "Explicit swarm request detected. Injected a bounded swarm bootstrap for this turn."
                            .to_string(),
                    ),
                })
            } else {
                let downgrade = build_swarm_downgrade_message(
                    delegation.target_artifacts.first().map(String::as_str),
                    &delegation.result_contract,
                    "Could not derive at least two stable worker targets from the current request",
                );
                delegation.note_swarm_fallback(
                    "Could not derive at least two stable worker targets from the current request",
                );
                build_subagent_bootstrap_request(
                    conversation,
                    response,
                    delegation,
                    Some(downgrade),
                    "explicit_swarm_fallback_to_subagent_bootstrap",
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_subagent_bootstrap_call, detect_explicit_delegation_intent,
        extract_stable_targets_from_request, maybe_build_explicit_delegation_bootstrap_request,
        normalized_stable_targets, DelegationMode, DelegationRuntimeState,
        ExplicitDelegationIntent, SubagentBootstrapRequest,
    };
    use crate::conversation::message::Message;
    use crate::conversation::Conversation;
    use crate::session::SessionType;

    #[test]
    fn bootstrap_call_allows_generic_bounded_target_when_contract_is_empty() {
        let call = build_subagent_bootstrap_call(&SubagentBootstrapRequest {
            goal: Some("Inspect the environment".to_string()),
            context: None,
            locked_target: None,
            missing_artifacts: Vec::new(),
            node_target_artifacts: Vec::new(),
            node_result_contract: Vec::new(),
            parent_write_scope: Vec::new(),
        })
        .expect("bootstrap call");
        assert!(call
            .instructions
            .contains("the current bounded deliverable"));
    }

    #[test]
    fn detects_explicit_swarm_request_from_latest_user_message() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("Use swarm with multiple workers to inspect the repo.")
        ]);
        let delegation = DelegationRuntimeState::new(
            DelegationMode::Subagent,
            0,
            1,
            vec!["src".to_string()],
            vec!["src/a.ts".to_string(), "src/b.ts".to_string()],
            vec!["src/a.ts".to_string(), "src/b.ts".to_string()],
        );
        assert_eq!(
            detect_explicit_delegation_intent(&conversation, &delegation),
            Some(ExplicitDelegationIntent::Swarm)
        );
    }

    #[test]
    fn explicit_swarm_bootstrap_falls_back_to_subagent_when_targets_are_not_stable() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("Use swarm to inspect the current repository.")
        ]);
        let response = Message::assistant().with_text("I should inspect the repo in parallel.");
        let mut delegation = DelegationRuntimeState::new(
            DelegationMode::Swarm,
            0,
            1,
            Vec::new(),
            vec!["draft alpha".to_string()],
            vec!["notes".to_string()],
        );
        let plan = maybe_build_explicit_delegation_bootstrap_request(
            &conversation,
            &response,
            &mut delegation,
        )
        .expect("bootstrap plan");
        assert_eq!(
            plan.request
                .tool_call
                .as_ref()
                .expect("tool call")
                .name
                .as_ref(),
            "subagent"
        );
        assert!(
            plan.inline_notice
                .as_deref()
                .unwrap_or_default()
                .contains("Automatic swarm execution fell back")
                || plan
                    .inline_notice
                    .as_deref()
                    .unwrap_or_default()
                    .contains("Recursive delegation failed")
        );
    }

    #[test]
    fn explicit_swarm_bootstrap_uses_stable_targets_from_request_when_runtime_contract_is_empty() {
        let conversation = Conversation::new_unvalidated(vec![Message::user()
            .with_text("Use swarm to inspect src/game.ts and public/logo.svg in parallel.")]);
        let response = Message::assistant().with_text("I'll inspect both assets.");
        let mut delegation = DelegationRuntimeState::new(
            DelegationMode::Swarm,
            0,
            1,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let plan = maybe_build_explicit_delegation_bootstrap_request(
            &conversation,
            &response,
            &mut delegation,
        )
        .expect("bootstrap plan");
        let tool_call = plan.request.tool_call.as_ref().expect("tool call");
        assert_eq!(tool_call.name.as_ref(), "swarm");
        let args = tool_call.arguments.as_ref().expect("arguments");
        let targets = args
            .get("targets")
            .and_then(|value| value.as_array())
            .expect("targets array");
        assert!(targets
            .iter()
            .any(|value| value.as_str() == Some("src/game.ts")));
        assert!(targets
            .iter()
            .any(|value| value.as_str() == Some("public/logo.svg")));
    }

    #[test]
    fn explicit_swarm_bootstrap_prefers_request_targets_over_internal_channel_artifacts() {
        let conversation = Conversation::new_unvalidated(vec![Message::user().with_text(
            "Use swarm to inspect README.md and docs/. Return a concise final summary.",
        )]);
        let response = Message::assistant().with_text("I'll inspect both targets.");
        let mut delegation = DelegationRuntimeState::new(
            DelegationMode::Subagent,
            0,
            1,
            Vec::new(),
            vec!["channel:abc".to_string()],
            vec!["channel:abc".to_string()],
        );
        let plan = maybe_build_explicit_delegation_bootstrap_request(
            &conversation,
            &response,
            &mut delegation,
        )
        .expect("bootstrap plan");
        let tool_call = plan.request.tool_call.as_ref().expect("tool call");
        assert_eq!(tool_call.name.as_ref(), "swarm");
        let targets = tool_call
            .arguments
            .as_ref()
            .and_then(|value| value.get("targets"))
            .and_then(|value| value.as_array())
            .expect("targets array")
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(targets, vec!["README.md", "docs/"]);
    }

    #[test]
    fn extract_stable_targets_from_request_ignores_sentence_tail_summary_token() {
        let targets = extract_stable_targets_from_request(
            "Use swarm to inspect README.md and docs/. Return a concise final summary.",
        );
        assert!(targets.iter().any(|value| value == "README.md"));
        assert!(targets.iter().any(|value| value == "docs/"));
        assert!(!targets.iter().any(|value| value == "summary."));
        assert!(!targets.iter().any(|value| value == "summary"));
    }

    #[test]
    fn explicit_delegation_intent_is_disabled_for_child_workers() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("Use swarm to inspect README.md and docs/.")
        ]);
        let mut delegation = DelegationRuntimeState::for_session_type(SessionType::SubAgent);
        delegation.mode = DelegationMode::Swarm;
        assert_eq!(
            detect_explicit_delegation_intent(&conversation, &delegation),
            None
        );
    }

    #[test]
    fn digital_avatar_language_does_not_count_as_explicit_subagent_intent() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("请帮我创建一个新的数字分身，并配置它的能力。")
        ]);
        let delegation = DelegationRuntimeState::new(
            DelegationMode::Subagent,
            0,
            1,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(
            detect_explicit_delegation_intent(&conversation, &delegation),
            None
        );
    }

    #[test]
    fn normalize_hint_path_strips_sentence_punctuation_from_runtime_targets() {
        let normalized = normalized_stable_targets(&[
            "README.md".to_string(),
            "docs/.".to_string(),
            "summary.".to_string(),
        ]);
        assert!(normalized.iter().any(|value| value == "README.md"));
        assert!(normalized.iter().any(|value| value == "docs/"));
        assert!(!normalized.iter().any(|value| value == "docs/."));
        assert!(!normalized.iter().any(|value| value == "summary."));
    }
}
