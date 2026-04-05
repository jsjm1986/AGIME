use crate::agents::swarm_tool::SWARM_TOOL_NAME;
use crate::conversation::message::ToolRequest;

use super::tools::RuntimeToolMeta;
use super::{
    delegation::DelegationRuntimeState,
    state::{CoordinatorExecutionMode, HarnessMode},
};

#[derive(Debug, Clone)]
pub struct HarnessPolicy {
    pub mode: HarnessMode,
    pub max_subagent_calls_per_turn: usize,
}

impl HarnessPolicy {
    pub fn new(mode: HarnessMode) -> Self {
        Self {
            mode,
            max_subagent_calls_per_turn: 1,
        }
    }
}

#[derive(Debug, Clone)]
pub enum PolicyDecision {
    Allow,
    Deny { reason: String },
}

pub fn format_runtime_policy_denial(tool_name: &str, reason: &str) -> String {
    format!("Harness policy denied `{}`: {}", tool_name, reason)
}

pub fn apply_runtime_policy(
    policy: &HarnessPolicy,
    coordinator_execution_mode: CoordinatorExecutionMode,
    delegation: &DelegationRuntimeState,
    meta: &RuntimeToolMeta,
    _request: &ToolRequest,
    subagent_calls_so_far: usize,
) -> PolicyDecision {
    if policy.mode == HarnessMode::Plan && meta.is_subagent {
        return PolicyDecision::Deny {
            reason: "subagent delegation is disabled in explicit plan mode".to_string(),
        };
    }

    if policy.mode == HarnessMode::Plan
        && !matches!(
            meta.execution_mode,
            super::tools::ToolExecutionMode::ConcurrentReadOnly
        )
    {
        return PolicyDecision::Deny {
            reason:
                "mutating tools are disabled in explicit plan mode; switch to /execute to run them"
                    .to_string(),
        };
    }

    if policy.mode == HarnessMode::Blocked
        && !matches!(
            meta.execution_mode,
            super::tools::ToolExecutionMode::ConcurrentReadOnly
        )
    {
        return PolicyDecision::Deny {
            reason:
                "blocked mode only permits read-only investigation until the blocker is resolved"
                    .to_string(),
        };
    }

    if policy.mode == HarnessMode::Complete && !meta.is_final_output {
        return PolicyDecision::Deny {
            reason: "only final output collection is allowed in complete mode".to_string(),
        };
    }

    if meta.is_subagent && !delegation.can_delegate_subagent() {
        return PolicyDecision::Deny {
            reason: format!(
                "subagent delegation depth {} reached max depth {}",
                delegation.current_depth, delegation.max_depth
            ),
        };
    }

    if meta.is_subagent && subagent_calls_so_far >= policy.max_subagent_calls_per_turn {
        return PolicyDecision::Deny {
            reason: "only one bounded subagent call is allowed per turn".to_string(),
        };
    }

    if meta.name == SWARM_TOOL_NAME
        && coordinator_execution_mode == CoordinatorExecutionMode::AutoSwarm
        && delegation.swarm_calls_this_run >= 1
    {
        return PolicyDecision::Deny {
            reason: "automatic swarm execution is already active for this run; continue on the current worker results instead of launching another swarm".to_string(),
        };
    }

    PolicyDecision::Allow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::harness::result_budget::ToolResultBudgetBucket;
    use crate::agents::harness::tools::{ConcurrencyPolicy, InterruptBehavior, ToolExecutionMode};

    fn request(id: &str, tool_name: &str) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(rmcp::model::CallToolRequestParams {
                name: tool_name.to_string().into(),
                arguments: Some(rmcp::object!({})),
                meta: None,
                task: None,
            }),
            thought_signature: None,
        }
    }

    fn meta(
        name: &str,
        execution_mode: ToolExecutionMode,
        is_subagent: bool,
        is_final_output: bool,
    ) -> RuntimeToolMeta {
        RuntimeToolMeta {
            name: name.to_string(),
            transport: crate::agents::harness::ToolTransportKind::WorkerLocal,
            execution_mode,
            concurrency_policy: ConcurrencyPolicy::Static(execution_mode),
            interrupt_behavior: InterruptBehavior::Block,
            requires_permission: true,
            supports_notifications: true,
            supports_task_progress: false,
            supports_streaming_result: false,
            supports_child_tasks: is_subagent,
            requires_explicit_scope: false,
            max_result_chars: Some(1024),
            result_budget_bucket: ToolResultBudgetBucket::Standard,
            is_frontend: false,
            is_subagent,
            is_final_output,
        }
    }

    #[test]
    fn plan_mode_denies_mutating_tools() {
        let decision = apply_runtime_policy(
            &HarnessPolicy::new(HarnessMode::Plan),
            CoordinatorExecutionMode::SingleWorker,
            &DelegationRuntimeState::default(),
            &meta("write", ToolExecutionMode::SerialMutating, false, false),
            &request("1", "write"),
            0,
        );
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }

    #[test]
    fn complete_mode_only_allows_final_output() {
        let deny = apply_runtime_policy(
            &HarnessPolicy::new(HarnessMode::Complete),
            CoordinatorExecutionMode::SingleWorker,
            &DelegationRuntimeState::default(),
            &meta(
                "search",
                ToolExecutionMode::ConcurrentReadOnly,
                false,
                false,
            ),
            &request("1", "search"),
            0,
        );
        let allow = apply_runtime_policy(
            &HarnessPolicy::new(HarnessMode::Complete),
            CoordinatorExecutionMode::SingleWorker,
            &DelegationRuntimeState::default(),
            &meta(
                "recipe__final_output",
                ToolExecutionMode::StatefulSerial,
                false,
                true,
            ),
            &request("2", "recipe__final_output"),
            0,
        );
        assert!(matches!(deny, PolicyDecision::Deny { .. }));
        assert!(matches!(allow, PolicyDecision::Allow));
    }

    #[test]
    fn blocked_mode_denies_mutating_tools() {
        let decision = apply_runtime_policy(
            &HarnessPolicy::new(HarnessMode::Blocked),
            CoordinatorExecutionMode::SingleWorker,
            &DelegationRuntimeState::default(),
            &meta("write", ToolExecutionMode::SerialMutating, false, false),
            &request("1", "write"),
            0,
        );
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }

    #[test]
    fn delegation_depth_denies_subagent() {
        let decision = apply_runtime_policy(
            &HarnessPolicy::new(HarnessMode::Execute),
            CoordinatorExecutionMode::SingleWorker,
            &DelegationRuntimeState {
                current_depth: 1,
                max_depth: 1,
                ..DelegationRuntimeState::default()
            },
            &meta("subagent", ToolExecutionMode::SerialMutating, true, false),
            &request("1", "subagent"),
            0,
        );
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }

    #[test]
    fn second_subagent_call_in_same_turn_is_denied() {
        let decision = apply_runtime_policy(
            &HarnessPolicy::new(HarnessMode::Execute),
            CoordinatorExecutionMode::SingleWorker,
            &DelegationRuntimeState::default(),
            &meta("subagent", ToolExecutionMode::SerialMutating, true, false),
            &request("2", "subagent"),
            1,
        );
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }

    #[test]
    fn auto_swarm_mode_denies_second_swarm_call_in_same_run() {
        let decision = apply_runtime_policy(
            &HarnessPolicy::new(HarnessMode::Conversation),
            CoordinatorExecutionMode::AutoSwarm,
            &DelegationRuntimeState {
                mode: super::super::delegation::DelegationMode::Swarm,
                swarm_calls_this_run: 1,
                ..DelegationRuntimeState::default()
            },
            &meta("swarm", ToolExecutionMode::SerialMutating, false, false),
            &request("1", "swarm"),
            0,
        );
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }
}
