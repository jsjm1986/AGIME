use crate::conversation::message::{Message, MessageContent, ToolRequest};
use crate::conversation::Conversation;
use crate::prompt_template::render_global_file;
use rmcp::model::CallToolRequestParams;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use super::coordinator::planner_auto_swarm_enabled;
use super::delegation::{DelegationRuntimeState, SwarmBudget, SwarmPlan};
use super::state::{CoordinatorExecutionMode, HarnessMode};
use super::swarm_tool::SWARM_TOOL_NAME;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannerUpgradeReason {
    StructuredTargets,
    StructuredResultContract,
}

#[derive(Debug, Clone, Default)]
pub struct PlannerUpgradeDecision {
    pub plan: Option<SwarmPlan>,
    pub prompt_addendum: Option<String>,
    pub reason: Option<PlannerUpgradeReason>,
}

#[derive(Serialize)]
struct SwarmUpgradePromptContext {
    targets: String,
    result_contract: String,
    user_hint: String,
}

fn normalize_target_candidate(value: &str) -> Option<String> {
    let normalized = value.trim().replace('\\', "/");
    if normalized.is_empty() || normalized.contains('\n') || normalized.contains('\r') {
        return None;
    }
    let stable_shape = normalized.contains(':')
        || normalized.contains('/')
        || normalized.contains('.')
        || normalized.contains('_')
        || normalized.contains('-');
    if !stable_shape && normalized.split_whitespace().count() > 1 {
        return None;
    }
    Some(normalized)
}

fn dedupe_stable_targets(values: &[String]) -> Vec<String> {
    let mut stable = Vec::new();
    for value in values {
        if let Some(normalized) = normalize_target_candidate(value) {
            if !stable.iter().any(|existing| existing == &normalized) {
                stable.push(normalized);
            }
        }
    }
    stable
}

fn conversation_already_contains_swarm_request(conversation: &Conversation) -> bool {
    conversation.messages().iter().any(|message| {
        message.content.iter().any(|content| {
            content
                .as_tool_request()
                .and_then(|request| request.tool_call.as_ref().ok())
                .map(|tool_call| tool_call.name.as_ref() == SWARM_TOOL_NAME)
                .unwrap_or(false)
        })
    })
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

pub fn build_auto_swarm_instructions(
    conversation: &Conversation,
    response: &Message,
    plan: &SwarmPlan,
) -> String {
    let user_text = latest_user_text(conversation);
    let assistant_hint = latest_assistant_text(response);
    let mut sections = vec![
        "Automatically upgraded this turn into a bounded swarm execution.".to_string(),
        "Primary objective: execute the latest user request and produce the requested bounded deliverables in parallel."
            .to_string(),
    ];

    if !user_text.is_empty() {
        sections.push(format!("Latest user request:\n{}", user_text));
    }

    if !assistant_hint.is_empty() {
        sections.push(format!(
            "Leader planning hint for this swarm run:\n{}",
            assistant_hint
        ));
    }

    sections.push(format!(
        "Bounded targets: {}",
        if plan.targets.is_empty() {
            "none".to_string()
        } else {
            plan.targets.join(", ")
        }
    ));
    sections.push(format!(
        "Result contract: {}",
        if plan.result_contract.is_empty() {
            "none".to_string()
        } else {
            plan.result_contract.join(", ")
        }
    ));
    sections.push(format!(
        "Write scope: {}",
        if plan.write_scope.is_empty() {
            "none".to_string()
        } else {
            plan.write_scope.join(", ")
        }
    ));
    sections.push(
        "Workers must remain bounded to their target, use summary-only communication, and return concise final summaries to the leader."
            .to_string(),
    );

    sections.join("\n\n")
}

pub fn build_auto_swarm_tool_request(
    conversation: &Conversation,
    response: &Message,
    decision: &PlannerUpgradeDecision,
) -> Option<ToolRequest> {
    let plan = decision.plan.as_ref()?;
    if plan.targets.len() < 2 {
        return None;
    }

    let request_id = format!("auto_swarm_{}", Uuid::new_v4().simple());
    let arguments = json!({
        "instructions": build_auto_swarm_instructions(conversation, response, plan),
        "targets": plan.targets,
        "write_scope": plan.write_scope,
        "result_contract": plan.result_contract,
        "parallelism_budget": plan.budget.parallelism_budget,
        "validation_mode": plan.validation_mode,
        "summary": true,
    });

    Some(ToolRequest {
        id: request_id,
        tool_call: Ok(CallToolRequestParams {
            name: SWARM_TOOL_NAME.into(),
            arguments: arguments.as_object().cloned(),
            meta: None,
            task: None,
        }),
        thought_signature: None,
    })
}

pub fn maybe_plan_swarm_upgrade(
    mode: HarnessMode,
    execution_mode: CoordinatorExecutionMode,
    conversation: &Conversation,
    delegation: &DelegationRuntimeState,
) -> PlannerUpgradeDecision {
    if !planner_auto_swarm_enabled() {
        return PlannerUpgradeDecision::default();
    }
    if execution_mode != CoordinatorExecutionMode::AutoSwarm {
        return PlannerUpgradeDecision::default();
    }
    if !matches!(mode, HarnessMode::Conversation | HarnessMode::Execute) {
        return PlannerUpgradeDecision::default();
    }
    if !delegation.can_delegate_swarm() {
        return PlannerUpgradeDecision::default();
    }
    if delegation.swarm_calls_this_run > 0 {
        return PlannerUpgradeDecision::default();
    }
    if delegation.downgrade_message.is_some() {
        return PlannerUpgradeDecision::default();
    }
    if conversation_already_contains_swarm_request(conversation) {
        return PlannerUpgradeDecision::default();
    }

    let (targets, reason) = {
        let structured_targets = dedupe_stable_targets(&delegation.target_artifacts);
        if structured_targets.len() >= 2 {
            (
                structured_targets,
                Some(PlannerUpgradeReason::StructuredTargets),
            )
        } else {
            let structured_contract = dedupe_stable_targets(&delegation.result_contract);
            if structured_contract.len() >= 2 {
                (
                    structured_contract,
                    Some(PlannerUpgradeReason::StructuredResultContract),
                )
            } else {
                (Vec::new(), None)
            }
        }
    };

    if targets.len() < 2 {
        return PlannerUpgradeDecision::default();
    }

    let user_hint = conversation
        .messages()
        .last()
        .map(|message| {
            message
                .content
                .iter()
                .filter_map(|content| match content {
                    crate::conversation::message::MessageContent::Text(text) => {
                        Some(text.text.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    PlannerUpgradeDecision {
        plan: Some(SwarmPlan {
            budget: SwarmBudget {
                parallelism_budget: Some(targets.len().min(4) as u32),
            },
            targets: targets.clone(),
            write_scope: delegation.write_scope.clone(),
            result_contract: delegation.result_contract.clone(),
            validation_mode: !delegation.result_contract.is_empty(),
        }),
        reason,
        prompt_addendum: Some(
            render_global_file(
                "swarm_upgrade_addendum.md",
                &SwarmUpgradePromptContext {
                    targets: targets.join(", "),
                    result_contract: if delegation.result_contract.is_empty() {
                        "none".to_string()
                    } else {
                        delegation.result_contract.join(", ")
                    },
                    user_hint: user_hint.trim().to_string(),
                },
            )
            .unwrap_or_else(|_| {
                format!(
                    "This turn has multiple bounded targets ({}) and may benefit from a `swarm` tool call. Prefer `swarm` over repeated single-worker delegation when the user intent is to parallelize concrete deliverables. Current user hint: {}",
                    targets.join(", "),
                    user_hint.trim()
                )
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::Message;

    #[test]
    fn planner_upgrade_stays_idle_without_targets() {
        std::env::remove_var(super::super::coordinator::AGIME_ENABLE_SWARM_PLANNER_AUTO_ENV);
        let decision = maybe_plan_swarm_upgrade(
            HarnessMode::Conversation,
            CoordinatorExecutionMode::AutoSwarm,
            &Conversation::new_unvalidated(vec![Message::user().with_text("hello")]),
            &DelegationRuntimeState::default(),
        );
        assert!(decision.plan.is_none());
        assert!(decision.prompt_addendum.is_none());
    }

    #[test]
    fn planner_upgrade_requires_stable_targets() {
        std::env::set_var(
            super::super::coordinator::AGIME_ENABLE_SWARM_PLANNER_AUTO_ENV,
            "true",
        );
        let mut delegation = DelegationRuntimeState::new(
            super::super::delegation::DelegationMode::Swarm,
            0,
            1,
            vec!["docs".to_string()],
            vec!["draft alpha".to_string(), "docs/out.md".to_string()],
            vec!["docs/out.md".to_string(), "docs/summary.md".to_string()],
        );
        delegation.swarm_disabled_for_run = false;
        let decision = maybe_plan_swarm_upgrade(
            HarnessMode::Conversation,
            CoordinatorExecutionMode::AutoSwarm,
            &Conversation::new_unvalidated(vec![Message::user().with_text("split work")]),
            &delegation,
        );
        assert_eq!(
            decision.reason,
            Some(PlannerUpgradeReason::StructuredResultContract)
        );
        assert_eq!(
            decision.plan.expect("plan").targets,
            vec!["docs/out.md".to_string(), "docs/summary.md".to_string()]
        );
        std::env::remove_var(super::super::coordinator::AGIME_ENABLE_SWARM_PLANNER_AUTO_ENV);
    }

    #[test]
    fn planner_upgrade_respects_sticky_downgrade() {
        std::env::set_var(
            super::super::coordinator::AGIME_ENABLE_SWARM_PLANNER_AUTO_ENV,
            "true",
        );
        let mut delegation = DelegationRuntimeState::new(
            super::super::delegation::DelegationMode::Swarm,
            0,
            1,
            vec!["docs".to_string()],
            vec!["docs/a.md".to_string(), "docs/b.md".to_string()],
            vec!["docs/a.md".to_string(), "docs/b.md".to_string()],
        );
        delegation.note_swarm_fallback("previous worker produced no delta");
        let decision = maybe_plan_swarm_upgrade(
            HarnessMode::Conversation,
            CoordinatorExecutionMode::AutoSwarm,
            &Conversation::new_unvalidated(vec![Message::user().with_text("split work")]),
            &delegation,
        );
        assert!(decision.plan.is_none());
        std::env::remove_var(super::super::coordinator::AGIME_ENABLE_SWARM_PLANNER_AUTO_ENV);
    }

    #[test]
    fn planner_upgrade_does_not_reenter_after_swarm_call_in_same_run() {
        std::env::set_var(
            super::super::coordinator::AGIME_ENABLE_SWARM_PLANNER_AUTO_ENV,
            "true",
        );
        let mut delegation = DelegationRuntimeState::new(
            super::super::delegation::DelegationMode::Swarm,
            0,
            1,
            vec!["README.md".to_string()],
            vec!["README.md".to_string(), "docs/".to_string()],
            vec!["README.md".to_string(), "docs/".to_string()],
        );
        delegation.swarm_calls_this_run = 1;
        let decision = maybe_plan_swarm_upgrade(
            HarnessMode::Conversation,
            CoordinatorExecutionMode::AutoSwarm,
            &Conversation::new_unvalidated(vec![Message::user().with_text("split work")]),
            &delegation,
        );
        assert!(decision.plan.is_none());
        assert!(decision.prompt_addendum.is_none());
        std::env::remove_var(super::super::coordinator::AGIME_ENABLE_SWARM_PLANNER_AUTO_ENV);
    }

    #[test]
    fn build_auto_swarm_tool_request_uses_swarm_tool_name() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("Produce docs/a.md and docs/b.md")
        ]);
        let response = Message::assistant().with_text("I'll parallelize the deliverables.");
        let decision = PlannerUpgradeDecision {
            plan: Some(SwarmPlan {
                budget: SwarmBudget {
                    parallelism_budget: Some(2),
                },
                targets: vec!["docs/a.md".to_string(), "docs/b.md".to_string()],
                write_scope: vec!["docs".to_string()],
                result_contract: vec!["docs/a.md".to_string(), "docs/b.md".to_string()],
                validation_mode: true,
            }),
            prompt_addendum: None,
            reason: Some(PlannerUpgradeReason::StructuredTargets),
        };

        let request =
            build_auto_swarm_tool_request(&conversation, &response, &decision).expect("request");
        let tool_call = request.tool_call.expect("tool call");
        assert_eq!(tool_call.name.as_ref(), SWARM_TOOL_NAME);
        let arguments = tool_call.arguments.expect("arguments");
        let targets = arguments
            .get("targets")
            .and_then(serde_json::Value::as_array)
            .expect("targets");
        assert_eq!(targets.len(), 2);
    }
}
