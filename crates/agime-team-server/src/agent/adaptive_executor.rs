//! Adaptive Goal Execution (AGE) engine for mission execution.
//!
//! Implements goal-tree based execution with progress evaluation
//! and pivot protocol. Reuses runtime::execute_via_bridge and
//! MissionManager infrastructure.

use agime_team::MongoDb;
use anyhow::{anyhow, Result};

use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use super::mission_manager::MissionManager;
use super::mission_mongo::*;
use super::mission_monitor::consume_pending_monitor_intervention_instruction;
use super::mission_monitor::format_monitor_intervention_instruction;
use super::mission_monitor::normalize_monitor_action;
use super::mission_verifier;
use super::runtime;
use super::service_mongo::AgentService;
use super::task_manager::{StreamEvent, TaskManager};

const DEFAULT_GOAL_EXECUTION_TIMEOUT_SECS: u64 = 1200;
const DEFAULT_MIN_GOAL_EXECUTION_TIMEOUT_SECS: u64 = 300;
const MAX_GOAL_EXECUTION_TIMEOUT_SECS: u64 = 7200;
const DEFAULT_GOAL_TIMEOUT_CANCEL_GRACE_SECS: u64 = 20;
const MAX_GOAL_TIMEOUT_CANCEL_GRACE_SECS: u64 = 120;
const DEFAULT_GOAL_TIMEOUT_RETRY_LIMIT: u32 = 1;
const MAX_GOAL_RETRY_LIMIT: u32 = 8;
const DEFAULT_MISSION_PLANNING_TIMEOUT_SECS: u64 = 300;
const MAX_MISSION_PLANNING_TIMEOUT_SECS: u64 = 1800;
const DEFAULT_PLANNING_TIMEOUT_CANCEL_GRACE_SECS: u64 = 20;
const MAX_PLANNING_TIMEOUT_CANCEL_GRACE_SECS: u64 = 120;
const DEFAULT_GOAL_COMPLETION_CHECK_TIMEOUT_SECS: u64 = 30;
const MAX_GOAL_COMPLETION_CHECK_TIMEOUT_SECS: u64 = 300;
const MAX_GOAL_REQUIRED_ARTIFACTS: usize = 16;
const MAX_GOAL_COMPLETION_CHECKS: usize = 8;
const MAX_GOAL_COMPLETION_CHECK_CMD_LEN: usize = 1200;
const MAX_POST_GOAL_REVIEW_SUMMARY_CHARS: usize = 1600;
const MAX_COMPLETION_SALVAGE_LOOPS: u32 = 2;
const WAITING_EXTERNAL_COOLDOWN_SECS: i64 = 300;
const ACTIVITY_HEARTBEAT_INTERVAL_SECS: u64 = 15;
const RETRY_CONTEXT_TOOL_CALL_LIMIT: usize = 12;
const RETRY_CONTEXT_OUTPUT_LIMIT: usize = 1200;
const MISSION_PREFLIGHT_TOOL_NAME: &str = "mission_preflight__preflight";
const MISSION_VERIFY_CONTRACT_TOOL_NAME: &str = "mission_preflight__verify_contract";

enum GoalLoopResolution {
    Continue,
    StopForSynthesis,
}

#[derive(Debug, Clone)]
struct GoalSupervisorGuidance {
    diagnosis: String,
    resume_hint: String,
    status_assessment: Option<String>,
    recommended_action: Option<String>,
    semantic_tags: Vec<String>,
    observed_evidence: Vec<String>,
    persist_hint: Vec<String>,
    missing_core_deliverables: Vec<String>,
    confidence: Option<f64>,
    strategy_patch: Option<MissionStrategyPatch>,
    subagent_recommended: Option<bool>,
    parallelism_budget: Option<u32>,
}

#[derive(Debug, Clone)]
struct GoalMonitorInterventionPlan {
    intervention: MissionMonitorIntervention,
    instruction: Option<String>,
}

#[derive(Debug, Clone)]
struct GoalCompletionSalvagePlan {
    goals: Vec<GoalNode>,
    reason: Option<String>,
}

#[derive(Debug, Clone)]
struct GoalCompletionAssessorResult {
    decision: MissionCompletionDecision,
    reason: Option<String>,
    observed_evidence: Vec<String>,
    missing_core_deliverables: Vec<String>,
    salvage_plan: Option<GoalCompletionSalvagePlan>,
}

impl GoalCompletionAssessorResult {
    fn completion_assessment(&self) -> Option<MissionCompletionAssessment> {
        self.decision.to_assessment(
            self.reason.clone(),
            self.observed_evidence.clone(),
            self.missing_core_deliverables.clone(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GoalPlanReviewDecision {
    ContinueCurrentPlan,
    ContinueWithReplan,
    CompleteIfEvidenceSufficient,
    PartialHandoff,
    BlockedByEnvironment,
    BlockedByTooling,
    BlockedFail,
}

#[derive(Debug, Clone)]
struct GoalPlanReviewResult {
    decision: GoalPlanReviewDecision,
    selected_goal_id: Option<String>,
    reason: Option<String>,
    observed_evidence: Vec<String>,
    missing_core_deliverables: Vec<String>,
    salvage_plan: Option<GoalCompletionSalvagePlan>,
}

#[derive(Debug, Clone)]
enum NextGoalDirective {
    Execute(GoalNode),
    Continue,
    StopForSynthesis,
    Break,
}

struct AdaptiveSilentEventBroadcaster;

impl runtime::EventBroadcaster for AdaptiveSilentEventBroadcaster {
    fn broadcast(
        &self,
        _context_id: &str,
        _event: StreamEvent,
    ) -> impl std::future::Future<Output = ()> + Send {
        std::future::ready(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_replan_requires_explicit_strategy_action() {
        let continue_current = MissionStrategyState {
            action: Some("continue_current".to_string()),
            reason: None,
            missing_core_deliverables: Vec::new(),
            confidence: None,
            strategy_patch: None,
            subagent_recommended: None,
            parallelism_budget: None,
            updated_at: None,
        };
        let continue_with_replan = MissionStrategyState {
            action: Some("continue_with_replan".to_string()),
            ..continue_current.clone()
        };

        assert!(!AdaptiveExecutor::strategy_requests_bounded_replan(None));
        assert!(!AdaptiveExecutor::strategy_requests_bounded_replan(Some(
            &continue_current
        )));
        assert!(AdaptiveExecutor::strategy_requests_bounded_replan(Some(
            &continue_with_replan
        )));
    }

    #[test]
    fn supersede_open_goals_in_tree_keeps_only_bounded_repair_lane_open() {
        let mut goals = vec![
            GoalNode {
                goal_id: "g-2".to_string(),
                parent_id: None,
                title: "Primary goal".to_string(),
                description: "primary".to_string(),
                success_criteria: "done".to_string(),
                status: GoalStatus::Running,
                depth: 0,
                order: 1,
                exploration_budget: 3,
                attempts: vec![],
                output_summary: None,
                runtime_contract: None,
                contract_verification: None,
                pivot_reason: None,
                is_checkpoint: false,
                created_at: None,
                started_at: None,
                last_activity_at: None,
                last_progress_at: None,
                completed_at: None,
            },
            GoalNode {
                goal_id: "g-3".to_string(),
                parent_id: None,
                title: "Trailing goal".to_string(),
                description: "trailing".to_string(),
                success_criteria: "done".to_string(),
                status: GoalStatus::Pending,
                depth: 0,
                order: 2,
                exploration_budget: 3,
                attempts: vec![],
                output_summary: None,
                runtime_contract: None,
                contract_verification: None,
                pivot_reason: None,
                is_checkpoint: false,
                created_at: None,
                started_at: None,
                last_activity_at: None,
                last_progress_at: None,
                completed_at: None,
            },
            GoalNode {
                goal_id: "g-salvage-1".to_string(),
                parent_id: None,
                title: "Repair lane".to_string(),
                description: "bounded repair".to_string(),
                success_criteria: "done".to_string(),
                status: GoalStatus::Pending,
                depth: 0,
                order: 3,
                exploration_budget: 2,
                attempts: vec![],
                output_summary: None,
                runtime_contract: None,
                contract_verification: None,
                pivot_reason: Some("bounded_completion_repair".to_string()),
                is_checkpoint: false,
                created_at: None,
                started_at: None,
                last_activity_at: None,
                last_progress_at: None,
                completed_at: None,
            },
        ];

        let superseded = AdaptiveExecutor::supersede_open_goals_in_tree(
            &mut goals,
            &["g-salvage-1".to_string()],
            "replace remaining work with one bounded repair lane",
        );

        assert_eq!(superseded, 2);
        assert_eq!(goals[0].status, GoalStatus::Abandoned);
        assert_eq!(goals[1].status, GoalStatus::Abandoned);
        assert_eq!(goals[2].status, GoalStatus::Pending);
        assert!(goals[0]
            .output_summary
            .as_deref()
            .is_some_and(|text| text.contains("bounded adaptive repair")));
    }

    #[test]
    fn reuses_persisted_goal_preflight_contract_when_retry_has_no_new_preflight() {
        let goal = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Running,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: Some(RuntimeContractVerification {
                tool_called: true,
                status: Some("pass".to_string()),
                gate_mode: Some("soft".to_string()),
                accepted: Some(true),
                reason: None,
                checked_at: None,
            }),
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };
        let reusable = runtime::MissionPreflightContract {
            required_artifacts: vec!["reports/final/report.html".to_string()],
            completion_checks: vec!["exists:reports/final/report.html".to_string()],
            no_artifact_reason: None,
        };

        let resolved = AdaptiveExecutor::resolve_retry_goal_preflight_contract(
            None,
            Some(&reusable),
            &goal,
            Some("Goal timed out after 1200s"),
            None,
        )
        .expect("persisted contract should be reused");

        assert_eq!(resolved.required_artifacts, reusable.required_artifacts);
    }

    #[test]
    fn empty_goal_tree_is_not_usable() {
        assert!(!AdaptiveExecutor::goal_tree_is_usable(None));
        assert!(!AdaptiveExecutor::goal_tree_is_usable(Some(&[])));
    }

    #[test]
    fn completion_basis_requires_processed_goal_signal() {
        let pending_goal = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Pending,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };
        assert!(!AdaptiveExecutor::goal_tree_has_completion_basis(&[
            pending_goal.clone()
        ]));

        let mut completed_goal = pending_goal;
        completed_goal.status = GoalStatus::Completed;
        completed_goal.output_summary = Some("done".to_string());
        assert!(AdaptiveExecutor::goal_tree_has_completion_basis(&[
            completed_goal
        ]));
    }

    #[test]
    fn completed_goal_with_required_artifacts_but_no_outputs_is_not_material_delivery() {
        let _goal = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Draft contract".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Completed,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: Some("Outlined the remaining deliverables".to_string()),
            runtime_contract: Some(RuntimeContract {
                required_artifacts: vec!["deliverable/README.md".to_string()],
                completion_checks: vec!["exists:deliverable/README.md".to_string()],
                no_artifact_reason: None,
                source: None,
                captured_at: None,
            }),
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };

        let result = GoalCompletionAssessorResult {
            decision: MissionCompletionDecision::ContinueWithReplan,
            reason: Some("bounded repair goals are still needed".to_string()),
            observed_evidence: vec!["only a planning artifact exists".to_string()],
            missing_core_deliverables: vec!["final package".to_string()],
            salvage_plan: None,
        };

        assert!(result.completion_assessment().is_none());
        let blocked = MissionCompletionDecision::BlockedFail
            .to_assessment(
                Some(
                    "The remaining adaptive core deliverables still require another bounded repair loop."
                        .to_string(),
                ),
                result.observed_evidence.clone(),
                result.missing_core_deliverables.clone(),
            )
            .expect("blocked fail assessment should exist");
        assert_eq!(
            blocked.disposition,
            MissionCompletionDisposition::BlockedFail
        );
    }

    #[test]
    fn completion_assessor_continue_with_replan_parses_delta_goals() {
        let existing = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Existing".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Completed,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: Some("done".to_string()),
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };

        let response = r#"```json
        {
          "decision": "continue_with_replan",
          "reason": "还缺最终整合与交付索引",
          "delta_goals": [
            {
              "title": "补齐最终整合",
              "description": "复用现有证据生成最终整合文档",
              "success_criteria": "生成最终整合文档和交付索引"
            }
          ]
        }
        ```"#;

        let parsed = AdaptiveExecutor::parse_completion_salvage_response(&[existing], response)
            .expect("response should parse");

        assert_eq!(
            parsed.decision,
            MissionCompletionDecision::ContinueWithReplan
        );
        let plan = parsed.salvage_plan.expect("should request salvage");
        assert_eq!(plan.goals.len(), 1);
        assert_eq!(plan.goals[0].title, "补齐最终整合");
        assert!(plan.goals[0].goal_id.starts_with("g-salvage-"));
        assert_eq!(parsed.reason.as_deref(), Some("还缺最终整合与交付索引"));
    }

    #[test]
    fn completion_assessor_blocked_by_environment_produces_assessment() {
        let existing = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Existing".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Completed,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: Some("done".to_string()),
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };
        let response = r#"{
          "decision": "blocked_by_environment",
          "reason": "当前运行环境缺少显示服务，无法完成需要 GUI 的验证步骤",
          "observed_evidence": ["已完成源码生成", "运行环境无 display server"],
          "missing_core_deliverables": ["GUI 级验证证据"]
        }"#;

        let parsed = AdaptiveExecutor::parse_completion_salvage_response(&[existing], response)
            .expect("response should parse");
        let assessment = parsed
            .completion_assessment()
            .expect("blocked_by_environment should produce assessment");
        assert_eq!(
            assessment.disposition,
            MissionCompletionDisposition::BlockedByEnvironment
        );
        assert_eq!(assessment.missing_core_deliverables.len(), 1);
    }

    #[test]
    fn post_goal_plan_review_parses_continue_current_plan() {
        let existing = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Existing".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Completed,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: Some("done".to_string()),
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };
        let response = r#"{
          "decision": "continue_current_plan",
          "reason": "当前剩余 goal 仍与已发现证据一致",
          "observed_evidence": ["已完成环境探测", "未发现剩余计划冲突"],
          "missing_core_deliverables": []
        }"#;

        let parsed = AdaptiveExecutor::parse_post_goal_plan_review_response(&[existing], response)
            .expect("response should parse");

        assert_eq!(parsed.decision, GoalPlanReviewDecision::ContinueCurrentPlan);
        assert!(parsed.salvage_plan.is_none());
        assert_eq!(
            parsed.reason.as_deref(),
            Some("当前剩余 goal 仍与已发现证据一致")
        );
    }

    #[test]
    fn post_goal_plan_review_prompt_preserves_blocker_context_and_guard_rule() {
        let completed = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Capability probe".to_string(),
            description: "probe environment".to_string(),
            success_criteria: "detect whether GUI/browser automation is possible".to_string(),
            status: GoalStatus::Completed,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: Some("display unavailable; no visible window support; environment lacks GUI/browser capability and downstream GUI-only goals should not continue unchanged. ".repeat(20)),
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };
        let remaining = vec![GoalNode {
            goal_id: "g-2".to_string(),
            parent_id: None,
            title: "Open visible browser only if feasible".to_string(),
            description: "depends on GUI support".to_string(),
            success_criteria: "open visible browser".to_string(),
            status: GoalStatus::Pending,
            depth: 0,
            order: 1,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        }];

        let prompt = AdaptiveExecutor::build_post_goal_plan_review_prompt(
            "Need a visible browser verification or an explicit environment handoff.",
            &completed,
            &remaining,
        );

        assert!(prompt.contains("do not return `continue_current_plan`"));
        assert!(prompt.contains("only if feasible"));
        assert!(prompt.contains("display unavailable"));
    }

    #[test]
    fn next_goal_selection_response_parses_selected_goal() {
        let parsed = AdaptiveExecutor::parse_post_goal_plan_review_response(
            &[],
            r#"{
              "selected_goal_id": "g-5",
              "reason": "g-1 established that GUI capability is unavailable, so the environment handoff goal should run next",
              "observed_evidence": ["display unavailable", "fallback handoff goal exists"]
            }"#,
        )
        .expect("response should parse");

        assert_eq!(parsed.selected_goal_id.as_deref(), Some("g-5"));
        assert_eq!(
            parsed.reason.as_deref(),
            Some("g-1 established that GUI capability is unavailable, so the environment handoff goal should run next")
        );
        assert_eq!(parsed.observed_evidence.len(), 2);
    }

    #[test]
    fn completion_review_needed_when_complete_conflicts_with_pending_goal() {
        let goals = vec![
            GoalNode {
                goal_id: "g-1".to_string(),
                parent_id: None,
                title: "Done".to_string(),
                description: "desc".to_string(),
                success_criteria: "done".to_string(),
                status: GoalStatus::Completed,
                depth: 0,
                order: 0,
                exploration_budget: 3,
                attempts: vec![],
                output_summary: Some("done".to_string()),
                runtime_contract: None,
                contract_verification: None,
                pivot_reason: None,
                is_checkpoint: false,
                created_at: None,
                started_at: None,
                last_activity_at: None,
                last_progress_at: None,
                completed_at: None,
            },
            GoalNode {
                goal_id: "g-2".to_string(),
                parent_id: None,
                title: "Still pending".to_string(),
                description: "desc".to_string(),
                success_criteria: "done".to_string(),
                status: GoalStatus::Pending,
                depth: 0,
                order: 1,
                exploration_budget: 3,
                attempts: vec![],
                output_summary: None,
                runtime_contract: None,
                contract_verification: None,
                pivot_reason: None,
                is_checkpoint: false,
                created_at: None,
                started_at: None,
                last_activity_at: None,
                last_progress_at: None,
                completed_at: None,
            },
        ];
        let result = GoalCompletionAssessorResult {
            decision: MissionCompletionDecision::Complete,
            reason: Some("looks done".to_string()),
            observed_evidence: vec![],
            missing_core_deliverables: vec![],
            salvage_plan: None,
        };

        assert!(AdaptiveExecutor::completion_review_needed(&goals, &result));
    }

    #[test]
    fn contradictory_complete_with_unresolved_goals_becomes_replan() {
        let goals = vec![GoalNode {
            goal_id: "g-2".to_string(),
            parent_id: None,
            title: "Still pending".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Pending,
            depth: 0,
            order: 1,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        }];
        let result = GoalCompletionAssessorResult {
            decision: MissionCompletionDecision::Complete,
            reason: Some("looks done".to_string()),
            observed_evidence: vec![],
            missing_core_deliverables: vec![],
            salvage_plan: None,
        };

        let normalized =
            AdaptiveExecutor::normalize_contradictory_completion_result(&goals, result);
        assert_eq!(
            normalized.decision,
            MissionCompletionDecision::ContinueWithReplan
        );
        assert_eq!(normalized.missing_core_deliverables.len(), 1);
        assert_eq!(
            normalized
                .salvage_plan
                .as_ref()
                .expect("bounded repair loop should exist")
                .goals
                .len(),
            1
        );
    }

    #[test]
    fn provider_capacity_error_is_detected() {
        assert!(AdaptiveExecutor::goal_error_is_provider_capacity_block(
            "Rate limit exceeded: All credentials for model gpt-5.2 are cooling down"
        ));
        assert!(AdaptiveExecutor::goal_error_is_provider_capacity_block(
            "Rate limit exceeded: The usage limit has been reached"
        ));
        assert!(AdaptiveExecutor::goal_error_is_provider_capacity_block(
            "Authentication failed. Status: 401 Unauthorized. Response: Your authentication token has been invalidated."
        ));
        assert!(AdaptiveExecutor::goal_error_is_provider_capacity_block(
            "Request failed: Bad request (400): Your account does not have a valid coding plan subscription, or your subscription has expired"
        ));
        assert!(!AdaptiveExecutor::goal_error_is_provider_capacity_block(
            "Goal execution produced no tool calls after 3 attempts"
        ));
    }

    #[test]
    fn provider_capacity_error_does_not_fall_into_generic_soft_goal_signal() {
        let err =
            anyhow!("Rate limit exceeded: All credentials for model gpt-5.2 are cooling down");
        assert_eq!(AdaptiveExecutor::soft_goal_terminal_signal(&err), None);
    }

    #[test]
    fn reuses_persisted_goal_preflight_contract_after_missing_fresh_preflight_retry_error() {
        let goal = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Running,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: Some(RuntimeContractVerification {
                tool_called: true,
                status: Some("pass".to_string()),
                gate_mode: Some("soft".to_string()),
                accepted: Some(true),
                reason: None,
                checked_at: None,
            }),
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };
        let reusable = runtime::MissionPreflightContract {
            required_artifacts: vec!["reports/final/report.html".to_string()],
            completion_checks: vec!["exists:reports/final/report.html".to_string()],
            no_artifact_reason: None,
        };

        let resolved = AdaptiveExecutor::resolve_retry_goal_preflight_contract(
            None,
            Some(&reusable),
            &goal,
            Some(
                "Goal preflight validation failed: missing preflight contract payload: call mission_preflight__preflight",
            ),
            None,
        )
        .expect("persisted contract should still be reused after missing fresh preflight");

        assert_eq!(resolved.required_artifacts, reusable.required_artifacts);
    }

    #[test]
    fn build_goal_prompt_uses_dynamic_preflight_retry_fields() {
        let goal = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Pending,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };

        let prompt = AdaptiveExecutor::build_goal_prompt(
            &goal,
            &[],
            Some("/workspace"),
            None,
            3,
            Some("Goal timed out after 1200s"),
        );

        assert!(prompt.contains("\"attempt\": 3"));
        assert!(prompt.contains("\"last_error\": \"Goal timed out after 1200s\""));
    }

    #[test]
    fn build_goal_preflight_repair_prompt_requires_preflight_tool_first() {
        let goal = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Pending,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };

        let prompt = AdaptiveExecutor::build_goal_preflight_repair_prompt(
            &goal,
            Some("/workspace"),
            2,
            "Goal preflight validation failed: missing preflight contract payload",
        );

        assert!(prompt.contains("Your next response MUST be a tool call"));
        assert!(prompt.contains(MISSION_PREFLIGHT_TOOL_NAME));
        assert!(prompt.contains("\"attempt\": 2"));
    }

    #[test]
    fn build_goal_completion_repair_prompt_keeps_repair_generic() {
        let goal = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Pending,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };

        let prompt = AdaptiveExecutor::build_goal_completion_repair_prompt(
            &goal,
            Some("/workspace"),
            2,
            "Goal completion validation failed: required artifact not found: output/result.md",
        );

        assert!(prompt.contains(MISSION_PREFLIGHT_TOOL_NAME));
        assert!(prompt.contains("Do not restart the goal from scratch"));
        assert!(prompt.contains("Your next response MUST be a tool call"));
        assert!(prompt.contains("Do not call `mission_preflight__preflight` again unless you are actually correcting the contract itself."));
        assert!(!prompt.contains("HTML"));
        assert!(!prompt.contains("slides"));
    }

    #[test]
    fn build_goal_no_tool_recovery_prompt_is_generic_and_actionable() {
        let goal = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Pending,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };

        let prompt = AdaptiveExecutor::build_goal_no_tool_recovery_prompt(
            &goal,
            Some("/workspace"),
            2,
            "Goal execution produced no tool calls",
            true,
        );

        assert!(prompt.contains("concrete tool-backed recovery path"));
        assert!(prompt.contains("Reuse the current validated contract"));
        assert!(!prompt.contains("HTML"));
        assert!(!prompt.contains("slides"));
    }

    #[test]
    fn build_goal_supervisor_hint_prompt_is_generic_and_evidence_driven() {
        let goal = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Running,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };

        let prompt = AdaptiveExecutor::build_goal_supervisor_hint_prompt(
            "Mission goal",
            &goal,
            "- g-0 [completed]\n  title: prior evidence\n  success_criteria: done\n  summary: environment lacks visible GUI\n  attempts: 1\n  runtime_contract: false\n  verified: false\n  pivot_reason: (none)\n",
            Some("/workspace"),
            "Goal execution produced no tool calls",
            &[],
            None,
            None,
            2,
        );

        assert!(prompt.contains("Return JSON only"));
        assert!(prompt.contains("complete_if_evidence_sufficient"));
        assert!(prompt.contains("Mission goal"));
        assert!(prompt.contains("Current goal/evidence snapshot"));
        assert!(prompt.contains("Do not assume a specific deliverable type"));
        assert!(!prompt.contains("HTML"));
        assert!(!prompt.contains("slides"));
    }

    #[test]
    fn build_salvage_no_tool_replan_prompt_pushes_method_change() {
        let goal = GoalNode {
            goal_id: "g-salvage-1".to_string(),
            parent_id: None,
            title: "Repair final package".to_string(),
            description: "Fill the remaining core deliverables.".to_string(),
            success_criteria: "Produce the missing final package outputs.".to_string(),
            status: GoalStatus::Running,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: Some("bounded_completion_repair".to_string()),
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };

        let prompt = AdaptiveExecutor::build_salvage_no_tool_replan_prompt(
            "Mission goal",
            &goal,
            "- g-0 [completed]\n  summary: existing outputs are present\n",
            Some("/workspace"),
            "Goal execution produced no tool calls",
            None,
            2,
        );

        assert!(prompt.contains("replace the current salvage goal"));
        assert!(prompt.contains("continue_with_replan"));
        assert!(prompt.contains("Do not keep recommending a generic continue_current"));
    }

    #[test]
    fn parse_goal_supervisor_guidance_response_reads_optional_fields() {
        let guidance = AdaptiveExecutor::parse_goal_supervisor_guidance_response(
            r#"{
                "diagnosis": "The goal is active but the last retry made no concrete progress.",
                "status_assessment": "drifting",
                "recommended_action": "continue_with_hint",
                "resume_hint": "Reuse the current workspace and take one smallest concrete action.",
                "persist_hint": ["save an intermediate result"],
                "semantic_tags": ["recovery", "incremental_delivery"],
                "observed_evidence": ["recent retries without tool calls"]
            }"#,
        )
        .expect("guidance should parse");

        assert_eq!(guidance.status_assessment.as_deref(), Some("drifting"));
        assert_eq!(
            guidance.recommended_action.as_deref(),
            Some("continue_current")
        );
        assert!(guidance.semantic_tags.contains(&"recovery".to_string()));
        assert_eq!(guidance.observed_evidence.len(), 1);
        assert_eq!(guidance.persist_hint.len(), 1);
    }

    #[test]
    fn goal_retry_error_requires_completion_repair_detects_validation_failures() {
        assert!(
            AdaptiveExecutor::goal_retry_error_requires_completion_repair(Some(
                "Goal completion validation failed: required artifact not found: output/result.md",
            ))
        );
        assert!(
            AdaptiveExecutor::goal_retry_error_requires_completion_repair(Some(
                "Goal completion validation failed: completion check failed: `curl /health`",
            ))
        );
        assert!(
            !AdaptiveExecutor::goal_retry_error_requires_completion_repair(Some(
                "Goal preflight validation failed: missing preflight contract payload",
            ))
        );
    }

    #[test]
    fn goal_retry_error_is_no_tool_execution_detects_generic_no_tool_failures() {
        assert!(AdaptiveExecutor::goal_retry_error_is_no_tool_execution(
            Some("Goal execution produced no tool calls; switch to a concrete tool-backed recovery path")
        ));
        assert!(!AdaptiveExecutor::goal_retry_error_is_no_tool_execution(
            Some("Goal completion validation failed: required artifact not found: summary.md",)
        ));
    }

    #[test]
    fn passive_continue_action_includes_resume_current_step() {
        assert!(AdaptiveExecutor::is_goal_monitor_passive_continue_action(
            "continue_current"
        ));
        assert!(AdaptiveExecutor::is_goal_monitor_passive_continue_action(
            "resume_current_step"
        ));
        assert!(!AdaptiveExecutor::is_goal_monitor_passive_continue_action(
            "continue_with_replan"
        ));
    }

    #[test]
    fn repeated_no_tool_salvage_goal_requests_replan() {
        let goal = GoalNode {
            goal_id: "g-salvage-2".to_string(),
            parent_id: None,
            title: "Repair remaining outputs".to_string(),
            description: "Reuse current work and fill missing outputs.".to_string(),
            success_criteria: "Complete the remaining repair work.".to_string(),
            status: GoalStatus::Pending,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: Some("bounded_completion_repair".to_string()),
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };

        assert!(AdaptiveExecutor::should_replan_salvage_goal_after_no_tool(
            &goal,
            "Goal execution produced no tool calls; switch to a concrete tool-backed recovery path",
            2,
        ));
        assert!(!AdaptiveExecutor::should_replan_salvage_goal_after_no_tool(
            &goal,
            "Goal execution produced no tool calls; switch to a concrete tool-backed recovery path",
            1,
        ));
    }

    #[test]
    fn allocate_salvage_goal_ids_are_unique_within_same_batch() {
        let existing = vec![GoalNode {
            goal_id: "g-salvage-1".to_string(),
            parent_id: None,
            title: "existing".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Completed,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        }];

        let ids = AdaptiveExecutor::allocate_salvage_goal_ids(&existing, 3);
        assert_eq!(
            ids,
            vec![
                "g-salvage-2".to_string(),
                "g-salvage-3".to_string(),
                "g-salvage-4".to_string()
            ]
        );
    }

    #[test]
    fn reuses_persisted_goal_preflight_contract_for_completion_gap_retry() {
        let goal = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Running,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: Some(RuntimeContractVerification {
                tool_called: true,
                status: Some("pass".to_string()),
                gate_mode: Some("soft".to_string()),
                accepted: Some(true),
                reason: None,
                checked_at: None,
            }),
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };
        let reusable = runtime::MissionPreflightContract {
            required_artifacts: vec!["output/result.md".to_string()],
            completion_checks: vec!["exists:output/result.md".to_string()],
            no_artifact_reason: None,
        };

        let resolved = AdaptiveExecutor::resolve_retry_goal_preflight_contract(
            None,
            Some(&reusable),
            &goal,
            Some(
                "Goal completion validation failed: required artifact not found: output/result.md",
            ),
            None,
        )
        .expect("persisted contract should be reused for completion-gap retries");

        assert_eq!(resolved.required_artifacts, reusable.required_artifacts);
    }

    #[test]
    fn allows_persisted_goal_preflight_success_for_completion_gap_retries() {
        let goal = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Running,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: Some(RuntimeContractVerification {
                tool_called: true,
                status: Some("pass".to_string()),
                gate_mode: Some("soft".to_string()),
                accepted: Some(true),
                reason: None,
                checked_at: None,
            }),
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };
        let reusable = runtime::MissionPreflightContract {
            required_artifacts: vec!["summary.md".to_string()],
            completion_checks: vec!["exists:summary.md".to_string()],
            no_artifact_reason: None,
        };

        assert!(AdaptiveExecutor::allows_persisted_goal_preflight_success(
            Some(&reusable),
            &goal,
            Some("Goal completion validation failed: required artifact not found: summary.md"),
            None,
        ));
        assert!(AdaptiveExecutor::allows_persisted_goal_preflight_success(
            Some(&reusable),
            &goal,
            Some("Goal preflight validation failed: missing preflight contract payload"),
            None,
        ));
    }

    #[test]
    fn allows_existing_goal_contract_flow_for_completion_gap_repair() {
        let goal = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Running,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: Some(RuntimeContractVerification {
                tool_called: true,
                status: Some("pass".to_string()),
                gate_mode: Some("soft".to_string()),
                accepted: Some(true),
                reason: None,
                checked_at: None,
            }),
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };
        let fresh = runtime::MissionPreflightContract {
            required_artifacts: vec!["summary.md".to_string()],
            completion_checks: vec!["exists:summary.md".to_string()],
            no_artifact_reason: None,
        };

        assert!(AdaptiveExecutor::allows_existing_goal_contract_flow(
            Some(&fresh),
            &goal,
            Some("Goal completion validation failed: required artifact not found: summary.md"),
            None,
        ));
        assert!(AdaptiveExecutor::allows_existing_goal_contract_flow(
            Some(&fresh),
            &goal,
            Some("Goal preflight validation failed: missing preflight contract payload"),
            None,
        ));
    }

    #[test]
    fn procedural_preflight_gap_only_forces_fresh_when_no_reusable_contract_exists() {
        let goal = GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Running,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: Some(RuntimeContractVerification {
                tool_called: true,
                status: Some("pass".to_string()),
                gate_mode: Some("soft".to_string()),
                accepted: Some(true),
                reason: None,
                checked_at: None,
            }),
            pivot_reason: None,
            is_checkpoint: false,
            created_at: None,
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        };

        assert!(AdaptiveExecutor::should_force_fresh_goal_preflight(
            &goal,
            false,
            Some("Goal preflight validation failed: missing preflight contract payload"),
            None,
        ));
        assert!(!AdaptiveExecutor::should_force_fresh_goal_preflight(
            &goal,
            true,
            Some("Goal preflight validation failed: missing preflight contract payload"),
            None,
        ));
    }
}

/// AGE executor that orchestrates goal-tree based task execution.
pub struct AdaptiveExecutor {
    db: Arc<MongoDb>,
    mission_manager: Arc<MissionManager>,
    agent_service: Arc<AgentService>,
    internal_task_manager: Arc<TaskManager>,
    workspace_root: String,
}

struct HeartbeatGuard {
    cancel_token: CancellationToken,
}

impl HeartbeatGuard {
    fn new(cancel_token: CancellationToken) -> Self {
        Self { cancel_token }
    }
}

impl Drop for HeartbeatGuard {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

impl AdaptiveExecutor {
    fn spawn_goal_activity_heartbeat(
        agent_service: Arc<AgentService>,
        mission_id: String,
        goal_id: String,
        cancel_token: CancellationToken,
    ) {
        tokio::spawn(async move {
            let interval = Duration::from_secs(ACTIVITY_HEARTBEAT_INTERVAL_SECS);
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => break,
                    _ = tokio::time::sleep(interval) => {
                        if let Err(err) = agent_service.touch_goal_activity(&mission_id, &goal_id).await {
                            tracing::debug!(
                                "Failed to persist goal heartbeat for mission {} goal {}: {}",
                                mission_id,
                                goal_id,
                                err
                            );
                        }
                    }
                }
            }
        });
    }

    pub fn new(
        db: Arc<MongoDb>,
        mission_manager: Arc<MissionManager>,
        workspace_root: String,
    ) -> Self {
        let agent_service = Arc::new(AgentService::new(db.clone()));
        let internal_task_manager = Arc::new(TaskManager::new());
        Self {
            db,
            mission_manager,
            agent_service,
            internal_task_manager,
            workspace_root,
        }
    }

    fn mission_waiting_external_active(mission: &MissionDoc) -> bool {
        mission
            .waiting_external_until
            .as_ref()
            .is_some_and(|waiting_until| {
                waiting_until.timestamp_millis() > bson::DateTime::now().timestamp_millis()
            })
    }

    fn adaptive_done_status(mission: &MissionDoc) -> &'static str {
        match mission.status {
            MissionStatus::Paused => "paused",
            MissionStatus::Completed => "completed",
            MissionStatus::Cancelled => "cancelled",
            MissionStatus::Failed => "failed",
            MissionStatus::Running | MissionStatus::Planning | MissionStatus::Planned | MissionStatus::Draft
                if Self::mission_waiting_external_active(mission) =>
            {
                "waiting_external"
            }
            _ => "failed",
        }
    }

    fn goal_id_is_repair_lane(goal_id: &str) -> bool {
        let normalized = goal_id.to_ascii_lowercase();
        normalized.contains("salvage") || normalized.contains("repair")
    }

    async fn patch_goal_waiting_external_convergence_state(
        &self,
        mission_id: &str,
        goal_id: &str,
        blocker: &str,
    ) {
        let convergence_patch = MissionConvergencePatch {
            active_repair_lane_id: Some(
                Self::goal_id_is_repair_lane(goal_id).then_some(goal_id.to_string()),
            ),
            consecutive_no_tool_count: Some(0),
            last_blocker_fingerprint: Some(runtime::blocker_fingerprint(blocker)),
            waiting_external_until: Some(Some(Self::waiting_external_until_after_cooldown())),
        };
        if let Err(err) = self
            .agent_service
            .patch_mission_convergence_state(mission_id, &convergence_patch)
            .await
        {
            tracing::warn!(
                "Failed to persist adaptive waiting_external convergence state for mission {} goal {}: {}",
                mission_id,
                goal_id,
                err
            );
        }
    }

    async fn clear_expired_waiting_external_hold(&self, mission_id: &str, mission: &MissionDoc) {
        let convergence_patch = MissionConvergencePatch {
            active_repair_lane_id: None,
            consecutive_no_tool_count: None,
            last_blocker_fingerprint: None,
            waiting_external_until: Some(None),
        };
        if let Err(err) = self
            .agent_service
            .patch_mission_convergence_state(mission_id, &convergence_patch)
            .await
        {
            tracing::warn!(
                "Failed to clear expired adaptive waiting_external convergence state for mission {}: {}",
                mission_id,
                err
            );
        }

        if let Some(strategy) = mission.current_strategy.as_ref() {
            if strategy.action.as_deref() == Some("mark_waiting_external") {
                let mut resumed_strategy = strategy.clone();
                resumed_strategy.action = Some("continue_current".to_string());
                resumed_strategy.reason = Some(
                    strategy.reason.clone().unwrap_or_else(|| {
                        "External wait window expired; resume the current adaptive goal"
                            .to_string()
                    }),
                );
                resumed_strategy.updated_at = Some(bson::DateTime::now());
                if let Err(err) = self
                    .agent_service
                    .set_current_strategy(mission_id, Some(&resumed_strategy))
                    .await
                {
                    tracing::warn!(
                        "Failed to clear adaptive waiting_external strategy gate for mission {}: {}",
                        mission_id,
                        err
                    );
                }
            }
        }
    }

    /// Execute an adaptive mission.
    /// NOTE: Cleanup (Done broadcast + mission_manager.complete) is handled by
    /// the caller MissionExecutor::execute_mission, so we do NOT duplicate it here.
    pub async fn execute_adaptive(
        &self,
        mission_id: &str,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        // 1. Load mission
        let mission = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Mission not found"))?;

        let session_id;

        // Create workspace directory for this mission
        let workspace_path = runtime::create_workspace_dir(
            &self.workspace_root,
            &[
                (&mission.team_id, "team_id"),
                ("missions", "category"),
                (mission_id, "mission_id"),
            ],
        )?;
        self.agent_service
            .set_mission_workspace(mission_id, &workspace_path)
            .await
            .map_err(|e| anyhow!("Failed to set workspace: {}", e))?;

        if mission.status == MissionStatus::Draft {
            // ── Planning Phase: decompose goal into goal tree ──
            session_id = self
                .run_planning_phase(
                    mission_id,
                    &mission,
                    cancel_token.clone(),
                    Some(&workspace_path),
                )
                .await?;

            // Check approval policy: checkpoint/manual → pause for user confirmation
            if mission.approval_policy != ApprovalPolicy::Auto {
                self.agent_service
                    .update_mission_status(mission_id, &MissionStatus::Planned)
                    .await
                    .map_err(|e| anyhow!("Failed to update status: {}", e))?;

                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: r#"{"type":"mission_planned","mode":"adaptive"}"#.to_string(),
                        },
                    )
                    .await;

                // Return Ok — caller's cleanup will read actual status (Planned)
                return Ok(());
            }
        } else if mission.status == MissionStatus::Planned {
            // ── User confirmed the plan, skip planning ──
            session_id = mission
                .session_id
                .as_deref()
                .ok_or_else(|| anyhow!("Mission has no session"))?
                .to_string();
        } else {
            return Err(anyhow!(
                "Mission must be in Draft or Planned status to start"
            ));
        }

        // ── Execution Phase ──
        self.run_execution_phase(
            mission_id,
            &mission.agent_id,
            &session_id,
            cancel_token,
            Some(&workspace_path),
            None,
        )
        .await
    }

    /// Planning phase: create session, decompose goal, save goal tree.
    /// Returns the session_id on success.
    async fn run_planning_phase(
        &self,
        mission_id: &str,
        mission: &MissionDoc,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<String> {
        // Create dedicated AgentSession
        let session = self
            .agent_service
            .create_chat_session(
                &mission.team_id,
                &mission.agent_id,
                &mission.creator_id,
                mission.attached_document_ids.clone(),
                None,
                None,
                None,
                None,
                None,
                mission.step_timeout_seconds,
                None,
                false,
                false,
                None,
                Some("mission".to_string()),
                Some(mission_id.to_string()),
                Some(true),
            )
            .await
            .map_err(|e| anyhow!("Failed to create session: {}", e))?;

        let session_id = session.session_id.clone();
        self.agent_service
            .set_mission_session(mission_id, &session_id)
            .await
            .map_err(|e| anyhow!("Failed to set session: {}", e))?;

        self.agent_service
            .update_mission_status(mission_id, &MissionStatus::Planning)
            .await
            .map_err(|e| anyhow!("Failed to update status: {}", e))?;

        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::Status {
                    status: r#"{"type":"mission_planning","mode":"adaptive"}"#.to_string(),
                },
            )
            .await;

        let planning_timeout = Self::planning_timeout();
        let planning_cancel = CancellationToken::new();
        {
            let linked = planning_cancel.clone();
            let external = cancel_token.clone();
            tokio::spawn(async move {
                external.cancelled().await;
                linked.cancel();
            });
        }

        let goals = match tokio::time::timeout(
            planning_timeout,
            self.decompose_goal(
                mission_id,
                mission,
                &session_id,
                planning_cancel.clone(),
                workspace_path,
            ),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                planning_cancel.cancel();
                tokio::time::sleep(Self::planning_timeout_cancel_grace()).await;
                return Err(anyhow!(
                    "Adaptive mission planning timed out after {}s",
                    planning_timeout.as_secs()
                ));
            }
        };

        if goals.is_empty() {
            return Err(anyhow!("Agent generated empty goal tree"));
        }

        self.agent_service
            .save_goal_tree(mission_id, goals)
            .await
            .map_err(|e| anyhow!("Failed to save goal tree: {}", e))?;

        Ok(session_id)
    }

    /// Execution phase: run goal loop, check for pause, synthesize results.
    async fn run_execution_phase(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
        operator_hint: Option<&str>,
    ) -> Result<()> {
        self.agent_service
            .update_mission_status(mission_id, &MissionStatus::Running)
            .await
            .map_err(|e| anyhow!("Failed to update status: {}", e))?;

        self.execute_goal_loop(
            mission_id,
            agent_id,
            session_id,
            cancel_token.clone(),
            workspace_path,
            operator_hint,
        )
        .await?;

        self.synthesize_and_complete(
            mission_id,
            agent_id,
            session_id,
            cancel_token,
            workspace_path,
        )
        .await
    }

    /// Post-loop: skip synthesis if mission already reached a terminal/pause state,
    /// otherwise synthesize results and mark completed.
    async fn synthesize_and_complete(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<()> {
        let mut salvage_loops = 0u32;
        loop {
            let Some(mission) = self
                .agent_service
                .get_mission(mission_id)
                .await
                .ok()
                .flatten()
            else {
                return Err(anyhow!("Mission not found"));
            };
            if matches!(
                mission.status,
                MissionStatus::Paused
                    | MissionStatus::Cancelled
                    | MissionStatus::Failed
                    | MissionStatus::Completed
            ) {
                return Ok(());
            }
            if !Self::goal_tree_is_usable(mission.goal_tree.as_deref()) {
                return Err(anyhow!(
                    "Adaptive mission has no executable goal tree; planning must run before completion"
                ));
            }
            if mission.completion_assessment.is_none() {
                match self
                    .evaluate_completion_salvage(&mission, mission_id, agent_id, workspace_path)
                    .await
                {
                    Ok(result) if result.salvage_plan.is_some() => {
                        let plan = result
                            .salvage_plan
                            .clone()
                            .expect("salvage plan should exist when branch matches");
                        salvage_loops += 1;
                        if salvage_loops > MAX_COMPLETION_SALVAGE_LOOPS {
                            tracing::info!(
                                "Adaptive mission {} exceeded completion salvage review interval ({}); continuing with another bounded repair loop instead of forcing partial handoff",
                                mission_id,
                                MAX_COMPLETION_SALVAGE_LOOPS
                            );
                        }
                        let mut all_goals = mission.goal_tree.clone().unwrap_or_default();
                        all_goals.extend(plan.goals.clone());
                        self.agent_service
                            .save_goal_tree(mission_id, all_goals)
                            .await
                            .map_err(|e| {
                                anyhow!("Failed to persist adaptive completion salvage plan: {}", e)
                            })?;
                        self.mission_manager
                            .broadcast(
                                mission_id,
                                StreamEvent::Status {
                                    status: serde_json::json!({
                                        "type": "mission_completion_salvage_replanned",
                                        "new_goal_count": plan.goals.len(),
                                        "reason": plan.reason,
                                    })
                                    .to_string(),
                                },
                            )
                            .await;
                        self.execute_goal_loop(
                            mission_id,
                            agent_id,
                            session_id,
                            cancel_token.clone(),
                            workspace_path,
                            None,
                        )
                        .await?;
                        continue;
                    }
                    Ok(result) => {
                        if let Some(assessment) = result.completion_assessment() {
                            if let Err(err) = self
                                .agent_service
                                .set_mission_completion_assessment(mission_id, &assessment)
                                .await
                            {
                                tracing::warn!(
                                    "Failed to persist adaptive completion assessment for mission {}: {}",
                                    mission_id,
                                    err
                                );
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            "Adaptive mission {} completion assessor failed, keeping best-effort finish path: {}",
                            mission_id,
                            err
                        );
                    }
                }
            }

            self.synthesize_results(
                mission_id,
                agent_id,
                session_id,
                cancel_token.clone(),
                workspace_path,
            )
            .await?;

            if let Err(err) = self
                .agent_service
                .clear_mission_current_goal(mission_id)
                .await
            {
                tracing::warn!(
                    "Failed to clear current goal before marking adaptive mission {} completed: {}",
                    mission_id,
                    err
                );
            }

            if let Err(e) = self
                .agent_service
                .update_mission_status(mission_id, &MissionStatus::Completed)
                .await
            {
                tracing::warn!("Failed to mark mission {} completed: {}", mission_id, e);
            }

            return Ok(());
        }
    }

    /// Decompose mission goal into a goal tree via LLM.
    async fn decompose_goal(
        &self,
        mission_id: &str,
        mission: &MissionDoc,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<Vec<GoalNode>> {
        let context_section = mission
            .context
            .as_deref()
            .map(|c| format!("\n## Additional Context\n{}", c))
            .unwrap_or_default();

        let prompt = format!(
            r#"You are decomposing a mission goal. Analyze the following goal and create a tree of 2-8 sub-goals.

## Goal
{}
{}

## Output Format
Output a JSON array wrapped in ```json code block. Each goal:
[{{"goal_id": "g-1", "parent_id": null, "title": "...", "description": "...", "success_criteria": "How to verify this goal is complete", "is_checkpoint": false, "order": 0}}]

Rules:
- goal_id format: "g-1", "g-2", "g-1-1" (sub-goals use parent ID prefix)
- parent_id is null for top-level goals
- success_criteria must be concrete and verifiable
- Set is_checkpoint: true for steps requiring human review
- Each goal should be an independently executable unit of work"#,
            mission.goal, context_section
        );

        if let Err(err) = self
            .execute_via_bridge(
                &mission.agent_id,
                session_id,
                mission_id,
                &prompt,
                cancel_token,
                workspace_path,
                None, // no mission_context during planning
            )
            .await
        {
            if runtime::is_waiting_external_provider_message(&err.to_string()) {
                tracing::warn!(
                    "Mission {} adaptive planning hit external/provider block ({}); using fallback goal tree so monitor/runtime can continue",
                    mission_id,
                    err
                );
                return Ok(vec![self.fallback_goal_from_mission(mission)]);
            }
            return Err(err);
        }

        // Parse goal tree from session messages
        let session = self
            .agent_service
            .get_session(session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Session not found"))?;

        let text = match runtime::extract_last_assistant_text(&session.messages_json) {
            Some(text) => text,
            None => {
                tracing::warn!(
                    "Mission {} adaptive planning has no assistant response, using fallback goal",
                    mission.mission_id
                );
                return Ok(vec![self.fallback_goal_from_mission(mission)]);
            }
        };

        let json_str = runtime::extract_json_block(&text);
        match self.parse_goal_tree_json(&json_str) {
            Ok(goals) if !goals.is_empty() => Ok(goals),
            Ok(_) => {
                tracing::warn!(
                    "Mission {} adaptive planning produced empty goal tree, using fallback goal",
                    mission.mission_id
                );
                Ok(vec![self.fallback_goal_from_mission(mission)])
            }
            Err(e) => {
                tracing::warn!(
                    "Mission {} adaptive planning JSON parse failed: {}. Using fallback goal",
                    mission.mission_id,
                    e
                );
                Ok(vec![self.fallback_goal_from_mission(mission)])
            }
        }
    }

    /// Parse goal tree JSON into GoalNode entries.
    fn parse_goal_tree_json(&self, json_str: &str) -> Result<Vec<GoalNode>> {
        #[derive(serde::Deserialize)]
        struct RawGoal {
            goal_id: String,
            parent_id: Option<String>,
            title: String,
            description: String,
            success_criteria: String,
            #[serde(default)]
            is_checkpoint: bool,
            #[serde(default)]
            order: u32,
        }

        fn parse_raw_goals_value(
            value: serde_json::Value,
        ) -> Result<Vec<RawGoal>, serde_json::Error> {
            if value.is_array() {
                return serde_json::from_value(value);
            }
            if let Some(arr) = value
                .get("goals")
                .or_else(|| value.get("goal_tree"))
                .or_else(|| value.get("steps"))
                .and_then(|v| v.as_array())
            {
                return serde_json::from_value(serde_json::Value::Array(arr.clone()));
            }
            serde_json::from_value(value)
        }

        let normalized = runtime::normalize_loose_json(json_str);
        let candidates: [&str; 2] = [json_str, &normalized];
        let mut raw: Option<Vec<RawGoal>> = None;
        let mut last_err = None;
        for candidate in candidates {
            match serde_json::from_str::<serde_json::Value>(candidate)
                .and_then(parse_raw_goals_value)
            {
                Ok(goals) => {
                    raw = Some(goals);
                    break;
                }
                Err(e) => {
                    last_err = Some(e.to_string());
                }
            }
        }

        let raw = raw.ok_or_else(|| {
            anyhow!(
                "Failed to parse goal tree JSON: {}",
                last_err.unwrap_or_else(|| "unknown error".to_string())
            )
        })?;

        let goals = raw
            .into_iter()
            .map(|r| {
                let depth = if r.parent_id.is_none() {
                    0
                } else {
                    let dashes = r.goal_id.matches('-').count() as u32;
                    dashes.saturating_sub(1)
                };
                GoalNode {
                    goal_id: r.goal_id,
                    parent_id: r.parent_id,
                    title: r.title,
                    description: r.description,
                    success_criteria: r.success_criteria,
                    status: GoalStatus::Pending,
                    depth,
                    order: r.order,
                    exploration_budget: 3,
                    attempts: vec![],
                    output_summary: None,
                    runtime_contract: None,
                    contract_verification: None,
                    pivot_reason: None,
                    is_checkpoint: r.is_checkpoint,
                    created_at: Some(bson::DateTime::now()),
                    started_at: None,
                    last_activity_at: None,
                    last_progress_at: None,
                    completed_at: None,
                }
            })
            .collect();

        Ok(goals)
    }

    fn fallback_goal_from_mission(&self, mission: &MissionDoc) -> GoalNode {
        GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "执行核心目标".to_string(),
            description: mission.goal.clone(),
            success_criteria: "给出可验证的最终结果或明确失败原因".to_string(),
            status: GoalStatus::Pending,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: Some(bson::DateTime::now()),
            started_at: None,
            last_activity_at: None,
            last_progress_at: None,
            completed_at: None,
        }
    }

    fn goal_tree_is_usable(goals: Option<&[GoalNode]>) -> bool {
        matches!(goals, Some(goals) if !goals.is_empty())
    }

    fn goal_tree_has_completion_basis(goals: &[GoalNode]) -> bool {
        goals.iter().any(|goal| {
            matches!(goal.status, GoalStatus::Completed | GoalStatus::Abandoned)
                || goal
                    .output_summary
                    .as_deref()
                    .is_some_and(|summary| !summary.trim().is_empty())
                || goal.runtime_contract.is_some()
                || !goal.attempts.is_empty()
                || goal
                    .pivot_reason
                    .as_deref()
                    .is_some_and(|reason| !reason.trim().is_empty())
        })
    }

    fn build_goal_evidence_digest(goals: &[GoalNode]) -> String {
        let mut digest = String::new();
        for goal in goals {
            let summary = goal
                .output_summary
                .as_deref()
                .filter(|text| !text.trim().is_empty())
                .unwrap_or("(no summary recorded)");
            let summary = if summary.chars().count() > 320 {
                let truncated: String = summary.chars().take(317).collect();
                format!("{}...", truncated)
            } else {
                summary.to_string()
            };
            digest.push_str(&format!(
                "- {} [{}]\n  title: {}\n  success_criteria: {}\n  summary: {}\n  attempts: {}\n  runtime_contract: {}\n  verified: {}\n  pivot_reason: {}\n",
                goal.goal_id,
                match goal.status {
                    GoalStatus::Pending => "pending",
                    GoalStatus::Running => "running",
                    GoalStatus::Completed => "completed",
                    GoalStatus::Failed => "failed",
                    GoalStatus::Pivoting => "pivoting",
                    GoalStatus::Abandoned => "abandoned",
                    GoalStatus::AwaitingApproval => "awaiting_approval",
                },
                goal.title,
                goal.success_criteria,
                summary,
                goal.attempts.len(),
                goal.runtime_contract.is_some(),
                goal.contract_verification
                    .as_ref()
                    .and_then(|v| v.accepted)
                    .unwrap_or(false),
                goal.pivot_reason.as_deref().unwrap_or("(none)")
            ));
        }
        if digest.trim().is_empty() {
            "- (none)\n".to_string()
        } else {
            digest
        }
    }

    fn goal_matches_runtime_snapshot(goal: &GoalNode, current_goal: Option<&str>) -> bool {
        current_goal
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.contains(&goal.goal_id) || value.contains(&goal.title))
            .unwrap_or(false)
    }

    fn collect_goal_monitor_missing_core_deliverables(
        mission: &MissionDoc,
        goal: &GoalNode,
        reusable_contract: Option<&runtime::MissionPreflightContract>,
    ) -> Vec<String> {
        let mut missing = Vec::new();
        if let Some(snapshot) = mission.latest_stuck_phase_snapshot.as_ref().filter(|snapshot| {
            Self::goal_matches_runtime_snapshot(goal, snapshot.current_goal.as_deref())
        }) {
            missing.extend(snapshot.missing_core_deliverables.iter().cloned());
        }
        if missing.is_empty() {
            if let Some(strategy) = mission.current_strategy.as_ref() {
                missing.extend(strategy.missing_core_deliverables.iter().cloned());
            }
        }
        if missing.is_empty() {
            if let Some(contract) = reusable_contract {
                missing.extend(contract.required_artifacts.iter().cloned());
            }
        }
        if missing.is_empty() {
            if let Some(contract) = goal.runtime_contract.as_ref() {
                missing.extend(contract.required_artifacts.iter().cloned());
            }
        }
        let mut deduped = Vec::new();
        for item in missing {
            let trimmed = item.trim();
            if trimmed.is_empty() {
                continue;
            }
            if deduped.iter().any(|existing: &String| existing == trimmed) {
                continue;
            }
            deduped.push(trimmed.to_string());
            if deduped.len() >= 6 {
                break;
            }
        }
        deduped
    }

    fn build_goal_monitor_strategy_patch(
        mission: &MissionDoc,
        _goal: &GoalNode,
        reason_for_change: &str,
        new_goal_shape: &str,
        expected_gain: &str,
    ) -> MissionStrategyPatch {
        let previous_strategy_summary = mission.current_strategy.as_ref().and_then(|strategy| {
            let action = strategy.action.as_deref().unwrap_or("none").trim();
            let reason = strategy.reason.as_deref().unwrap_or("").trim();
            if action.is_empty() && reason.is_empty() {
                None
            } else if reason.is_empty() {
                Some(action.to_string())
            } else {
                Some(format!("{action}: {reason}"))
            }
        });
        MissionStrategyPatch {
            previous_strategy_summary,
            reason_for_change: Some(Self::compact_goal_prompt_text(reason_for_change, 220)),
            new_goal_shape: Some(Self::compact_goal_prompt_text(new_goal_shape, 220)),
            preserved_user_intent: Some(Self::compact_goal_prompt_text(&mission.goal, 220)),
            expected_gain: Some(Self::compact_goal_prompt_text(expected_gain, 220)),
            applied_at: Some(bson::DateTime::now()),
        }
    }

    fn build_generic_goal_supervisor_guidance(
        mission: &MissionDoc,
        goal: &GoalNode,
        failure_message: &str,
        reusable_contract: Option<&runtime::MissionPreflightContract>,
        attempt: u32,
    ) -> GoalSupervisorGuidance {
        let missing_core_deliverables =
            Self::collect_goal_monitor_missing_core_deliverables(mission, goal, reusable_contract);
        let worker_state = mission.latest_worker_state.as_ref().filter(|state| {
            Self::goal_matches_runtime_snapshot(goal, state.current_goal.as_deref())
        });
        let stuck_snapshot = mission.latest_stuck_phase_snapshot.as_ref().filter(|snapshot| {
            Self::goal_matches_runtime_snapshot(goal, snapshot.current_goal.as_deref())
        });
        let has_existing_assets = worker_state
            .map(|state| !state.core_assets_now.is_empty() || !state.assets_delta.is_empty())
            .unwrap_or(false)
            || goal
                .output_summary
                .as_deref()
                .map(str::trim)
                .is_some_and(|summary| !summary.is_empty());
        let repeated_no_tool =
            Self::goal_retry_error_is_no_tool_execution(Some(failure_message)) && attempt >= 2;
        let repair_deliverables = has_existing_assets && !missing_core_deliverables.is_empty();
        let needs_contract_repair = Self::goal_error_requires_contract_repair(failure_message)
            || Self::goal_error_is_procedural_preflight_gap(failure_message);
        let waiting_external = Self::goal_error_is_provider_capacity_block(failure_message);
        let salvage_like_goal = goal.goal_id.starts_with("g-salvage-")
            || goal.title.to_ascii_lowercase().contains("salvage")
            || goal.description.to_ascii_lowercase().contains("repair");
        let recommended_action = if waiting_external {
            "mark_waiting_external"
        } else if needs_contract_repair {
            "repair_contract"
        } else if repair_deliverables {
            "repair_deliverables"
        } else if repeated_no_tool || salvage_like_goal || !missing_core_deliverables.is_empty() {
            "continue_with_replan"
        } else {
            "continue_current"
        };
        let (diagnosis, resume_hint, expected_gain, semantic_tags, confidence) =
            match recommended_action {
                "mark_waiting_external" => (
                    "Upstream capacity or another external dependency is temporarily unavailable, so the worker should preserve current results and resume after the blocker clears.",
                    "Keep the current workspace and results, avoid replaying the same path, and retry only after the external dependency or provider capacity has recovered.",
                    "Preserve progress while waiting for the external blocker to clear.",
                    vec![
                        "waiting_external".to_string(),
                        "external_blocker".to_string(),
                        "preserve_progress".to_string(),
                    ],
                    Some(0.82),
                ),
                "repair_contract" => (
                    "The current contract or preflight assumptions are blocking execution, so the next loop should repair the contract instead of replaying the same attempt path.",
                    "Reuse the current workspace outputs, rewrite only the contract or goal framing that is blocking progress, and continue from that repaired contract.",
                    "Realign the execution contract with the actual task while preserving user intent.",
                    vec![
                        "recovery".to_string(),
                        "repair_contract".to_string(),
                        "goal_reframing".to_string(),
                    ],
                    Some(0.74),
                ),
                "repair_deliverables" => (
                    "The worker already has usable partial outputs, but core deliverables are still missing and the loop should narrow to filling only those gaps.",
                    "Reuse the current workspace outputs, repair only the missing core deliverables, and save each repaired result immediately instead of replaying the full attempt.",
                    "Convert existing partial results into a directly reusable core delivery set.",
                    vec![
                        "recovery".to_string(),
                        "repair_deliverables".to_string(),
                        "incremental_delivery".to_string(),
                    ],
                    Some(0.72),
                ),
                "continue_with_replan" => (
                    "The current retry path is no longer producing new execution evidence, so the goal should change method instead of replaying the same loop.",
                    "Reuse the current workspace and replace the current path with 1-2 tighter repair actions that target the core missing result.",
                    "Switch to a bounded repair plan that avoids the exhausted retry pattern.",
                    vec![
                        "recovery".to_string(),
                        "repair_replan".to_string(),
                        "no_tool_retry".to_string(),
                    ],
                    Some(0.68),
                ),
                _ => (
                    "The recent retry ended without a concrete tool action, but the current plan can still continue if the worker resumes from the smallest verifiable next step.",
                    "Continue from the current workspace, take one smallest concrete tool-backed next step, and persist one intermediate result before expanding scope.",
                    "Resume progress without restarting the broader goal.",
                    vec![
                        "recovery".to_string(),
                        "incremental_delivery".to_string(),
                        "continue_current".to_string(),
                    ],
                    Some(0.55),
                ),
            };
        let mut observed_evidence = vec!["recent retry ended without tool calls".to_string()];
        if has_existing_assets {
            observed_evidence.push("usable workspace outputs already exist".to_string());
        }
        if waiting_external {
            observed_evidence.push("external or provider capacity is temporarily unavailable".to_string());
        }
        if needs_contract_repair {
            observed_evidence
                .push("current contract or preflight assumptions are blocking execution".to_string());
        }
        if stuck_snapshot
            .and_then(|snapshot| snapshot.current_blocker.as_deref())
            .is_some()
        {
            observed_evidence.push("worker has already recorded an explicit blocker".to_string());
        }
        let subagent_recommended = if recommended_action != "continue_current"
            && missing_core_deliverables.len() >= 2
        {
            Some(true)
        } else {
            None
        };
        let parallelism_budget = subagent_recommended.map(|_| {
            if missing_core_deliverables.len() >= 3 {
                3
            } else {
                2
            }
        });
        let strategy_patch = if recommended_action == "continue_current" {
            None
        } else {
            Some(Self::build_goal_monitor_strategy_patch(
                mission,
                goal,
                diagnosis,
                resume_hint,
                expected_gain,
            ))
        };
        let mut persist_hint = Vec::new();
        if recommended_action == "repair_deliverables" {
            persist_hint.push("save each repaired core deliverable as soon as it is regenerated".to_string());
        } else if recommended_action == "repair_contract" {
            persist_hint.push("save the repaired contract or goal framing before resuming execution".to_string());
        } else if recommended_action == "mark_waiting_external" {
            persist_hint.push("preserve the strongest current outputs and wait for the external blocker to clear".to_string());
        } else {
            persist_hint.push(
                "save one intermediate result or evidence item before broadening scope".to_string(),
            );
        }

        GoalSupervisorGuidance {
            diagnosis: diagnosis.to_string(),
            resume_hint: resume_hint.to_string(),
            status_assessment: Some("drifting".to_string()),
            recommended_action: Some(recommended_action.to_string()),
            semantic_tags,
            observed_evidence,
            persist_hint,
            missing_core_deliverables,
            confidence,
            strategy_patch,
            subagent_recommended,
            parallelism_budget,
        }
    }

    fn build_unresolved_goal_digest(goals: &[GoalNode]) -> String {
        let unresolved = goals
            .iter()
            .filter(|goal| !matches!(goal.status, GoalStatus::Completed))
            .collect::<Vec<_>>();
        if unresolved.is_empty() {
            return "- (none)\n".to_string();
        }

        let mut digest = String::new();
        for goal in unresolved.iter().take(6) {
            let summary = goal
                .output_summary
                .as_deref()
                .filter(|text| !text.trim().is_empty())
                .unwrap_or("(no summary recorded)");
            let summary = if summary.chars().count() > 220 {
                let truncated: String = summary.chars().take(217).collect();
                format!("{}...", truncated)
            } else {
                summary.to_string()
            };
            digest.push_str(&format!(
                "- {} [{}]\n  title: {}\n  summary: {}\n  pivot_reason: {}\n",
                goal.goal_id,
                match goal.status {
                    GoalStatus::Pending => "pending",
                    GoalStatus::Running => "running",
                    GoalStatus::Completed => "completed",
                    GoalStatus::Failed => "failed",
                    GoalStatus::Pivoting => "pivoting",
                    GoalStatus::Abandoned => "abandoned",
                    GoalStatus::AwaitingApproval => "awaiting_approval",
                },
                goal.title,
                summary,
                goal.pivot_reason.as_deref().unwrap_or("(none)")
            ));
        }
        if unresolved.len() > 6 {
            digest.push_str(&format!(
                "- ... {} more unresolved goals omitted\n",
                unresolved.len() - 6
            ));
        }
        digest
    }

    fn completion_review_needed(goals: &[GoalNode], result: &GoalCompletionAssessorResult) -> bool {
        if result.decision != MissionCompletionDecision::Complete {
            return false;
        }

        let has_nonterminal_goals = goals.iter().any(|goal| {
            matches!(
                goal.status,
                GoalStatus::Pending | GoalStatus::Running | GoalStatus::Failed
            )
        });
        let has_abandoned_goals = goals
            .iter()
            .any(|goal| matches!(goal.status, GoalStatus::Abandoned));

        has_nonterminal_goals || has_abandoned_goals || !result.missing_core_deliverables.is_empty()
    }

    fn completion_fallback_needed(
        goals: &[GoalNode],
        result: &GoalCompletionAssessorResult,
    ) -> bool {
        if result.decision != MissionCompletionDecision::Complete {
            return false;
        }

        goals.iter().any(|goal| {
            matches!(
                goal.status,
                GoalStatus::Pending | GoalStatus::Running | GoalStatus::Failed
            )
        }) || !result.missing_core_deliverables.is_empty()
    }

    fn build_completion_review_prompt(
        mission_goal: &str,
        goals: &[GoalNode],
        initial: &GoalCompletionAssessorResult,
    ) -> String {
        let goal_digest = Self::build_goal_evidence_digest(goals);
        let unresolved_digest = Self::build_unresolved_goal_digest(goals);
        let initial_reason = initial.reason.as_deref().unwrap_or("(none)");
        format!(
            "You are reviewing a potentially contradictory `complete` decision for an adaptive mission.\n\n\
Mission goal:\n{}\n\n\
Current goal digest:\n{}\n\
Unresolved / non-complete goals:\n{}\n\
Initial assessment:\n\
- decision: complete\n\
- reason: {}\n\
- observed_evidence: {:?}\n\
- missing_core_deliverables: {:?}\n\n\
Task:\n\
- Reassess whether the mission should truly end now.\n\
- Return `complete` only if the unresolved goals are genuinely non-core, superseded, or otherwise not needed for the requested end-user outcome.\n\
- If important work is still missing but can be finished in 1-3 bounded delta goals, return `continue_with_replan` and provide `delta_goals`.\n\
- If useful partial delivery exists but the missing work is not worth another autonomous loop, return `partial_handoff`.\n\
- Treat `partial_handoff` as valid only when the already delivered outputs are directly reusable by the end user in their current state.\n\
- A scaffold, draft, placeholder, outline, contract, carrier file, or partially populated shell created mainly to enable later filling does not qualify as useful partial delivery unless the mission explicitly asked for that scaffold or draft itself.\n\
- If the unresolved goals still contain the main substance of the requested outcome, do not collapse to `partial_handoff`; prefer `continue_with_replan`, `blocked_by_environment`, `blocked_by_tooling`, or `blocked_fail`.\n\
- If the missing work depends on missing runtime capabilities or environment access, return `blocked_by_environment`.\n\
- If the missing work is mainly blocked by failing tools or unstable source-access paths, return `blocked_by_tooling`.\n\
- Use evidence-based, low-commitment reasoning.\n\n\
Return JSON only:\n\
{{\n\
  \"decision\": \"complete\" | \"continue_with_replan\" | \"partial_handoff\" | \"blocked_by_environment\" | \"blocked_by_tooling\" | \"blocked_fail\",\n\
  \"reason\": \"short explanation\",\n\
  \"observed_evidence\": [\"...\"],\n\
  \"missing_core_deliverables\": [\"...\"],\n\
  \"delta_goals\": [\n\
    {{\n\
      \"title\": \"...\",\n\
      \"description\": \"...\",\n\
      \"success_criteria\": \"...\",\n\
      \"is_checkpoint\": false\n\
    }}\n\
  ]\n\
}}\n\
If no bounded salvage loop is appropriate, return an empty array for `delta_goals`.",
            mission_goal,
            goal_digest,
            unresolved_digest,
            initial_reason,
            initial.observed_evidence,
            initial.missing_core_deliverables
        )
    }

    fn build_post_goal_plan_review_prompt(
        mission_goal: &str,
        completed_goal: &GoalNode,
        remaining_goals: &[GoalNode],
    ) -> String {
        let completed_summary = completed_goal
            .output_summary
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or("(no summary recorded)");
        let completed_summary =
            if completed_summary.chars().count() > MAX_POST_GOAL_REVIEW_SUMMARY_CHARS {
                let truncated: String = completed_summary
                    .chars()
                    .take(MAX_POST_GOAL_REVIEW_SUMMARY_CHARS.saturating_sub(3))
                    .collect();
                format!("{}...", truncated)
            } else {
                completed_summary.to_string()
            };
        let remaining_digest = Self::build_goal_evidence_digest(remaining_goals);
        format!(
            "You are the plan review monitor for an adaptive long-running mission.\n\n\
Mission goal:\n{}\n\n\
Recently completed goal:\n- goal_id: {}\n- title: {}\n- success_criteria: {}\n- summary: {}\n\n\
Remaining non-terminal goals:\n{}\n\
Task:\n\
- Decide whether the remaining plan should continue unchanged, be replaced with a bounded repair plan, or end as a partial/blocking handoff.\n\
- Prefer `continue_current_plan` when the remaining goals still fit the current evidence and environment.\n\
- If the completed goal establishes that a prerequisite capability, environment, or access path is unavailable, and any remaining goal still depends on that prerequisite, do not return `continue_current_plan`.\n\
- Treat explicit goal guards such as \"only if feasible\", \"only when supported\", or environment-specific prerequisites as real plan constraints that must be honored.\n\
- Use `continue_with_replan` only when the remaining work should be replaced with 1-3 bounded delta goals.\n\
- Use `complete_if_evidence_sufficient` only when the requested end-user outcome is already materially delivered and the remaining goals are non-core or superseded.\n\
- Use `partial_handoff` when useful delivery exists but the remaining work is not worth another autonomous loop.\n\
- Use `blocked_by_environment` when the remaining work depends on runtime capabilities or environment access that are clearly unavailable now.\n\
- Use `blocked_by_tooling` when the remaining work is mainly blocked by failing tools or unstable source-access paths.\n\
- Do not assume a specific deliverable type unless it is supported by the goal summaries or evidence.\n\
- Keep the reasoning evidence-driven and low-commitment.\n\n\
Return JSON only:\n\
{{\n\
  \"decision\": \"continue_current_plan\" | \"continue_with_replan\" | \"complete_if_evidence_sufficient\" | \"partial_handoff\" | \"blocked_by_environment\" | \"blocked_by_tooling\" | \"blocked_fail\",\n\
  \"reason\": \"short explanation\",\n\
  \"observed_evidence\": [\"...\"],\n\
  \"missing_core_deliverables\": [\"...\"],\n\
  \"delta_goals\": [\n\
    {{\n\
      \"title\": \"...\",\n\
      \"description\": \"...\",\n\
      \"success_criteria\": \"...\",\n\
      \"is_checkpoint\": false\n\
    }}\n\
  ]\n\
}}\n\
If `continue_current_plan`, `complete_if_evidence_sufficient`, `partial_handoff`, or a blocked disposition is enough, return an empty `delta_goals` array.",
            mission_goal,
            completed_goal.goal_id,
            completed_goal.title,
            completed_goal.success_criteria,
            completed_summary,
            remaining_digest
        )
    }

    fn parse_post_goal_plan_review_response(
        goals: &[GoalNode],
        response: &str,
    ) -> Result<GoalPlanReviewResult> {
        #[derive(serde::Deserialize)]
        struct DeltaGoal {
            title: String,
            description: String,
            success_criteria: String,
            #[serde(default)]
            is_checkpoint: bool,
        }

        let value = runtime::parse_first_json_value(response)
            .or_else(|_| runtime::parse_first_json_value(&runtime::extract_json_block(response)))
            .map_err(|err| anyhow!("Failed to parse adaptive post-goal review JSON: {}", err))?;
        let decision = match value
            .get("decision")
            .and_then(|v| v.as_str())
            .unwrap_or("continue_current_plan")
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '-'], "_")
            .as_str()
        {
            "continue_with_replan" | "replan_remaining_goals" => {
                GoalPlanReviewDecision::ContinueWithReplan
            }
            "complete_if_evidence_sufficient" | "complete" => {
                GoalPlanReviewDecision::CompleteIfEvidenceSufficient
            }
            "partial_handoff" => GoalPlanReviewDecision::PartialHandoff,
            "blocked_by_environment" => GoalPlanReviewDecision::BlockedByEnvironment,
            "blocked_by_tooling" => GoalPlanReviewDecision::BlockedByTooling,
            "blocked_fail" => GoalPlanReviewDecision::BlockedFail,
            _ => GoalPlanReviewDecision::ContinueCurrentPlan,
        };
        let reason = value
            .get("reason")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let selected_goal_id = value
            .get("selected_goal_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let observed_evidence = value
            .get("observed_evidence")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(|item| item.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let missing_core_deliverables = value
            .get("missing_core_deliverables")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(|item| item.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let salvage_plan = if matches!(decision, GoalPlanReviewDecision::ContinueWithReplan) {
            let raw_goals = value
                .get("delta_goals")
                .and_then(|v| serde_json::from_value::<Vec<DeltaGoal>>(v.clone()).ok())
                .unwrap_or_default();
            if raw_goals.is_empty() {
                None
            } else {
                let start_order = goals.iter().map(|goal| goal.order).max().unwrap_or(0);
                let salvage_ids = Self::allocate_salvage_goal_ids(goals, raw_goals.len());
                let built_goals = raw_goals
                    .into_iter()
                    .zip(salvage_ids.into_iter())
                    .enumerate()
                    .map(|(index, (raw, goal_id))| GoalNode {
                        goal_id,
                        parent_id: None,
                        title: raw.title,
                        description: raw.description,
                        success_criteria: raw.success_criteria,
                        status: GoalStatus::Pending,
                        depth: 0,
                        order: start_order + index as u32 + 1,
                        exploration_budget: 3,
                        attempts: vec![],
                        output_summary: None,
                        runtime_contract: None,
                        contract_verification: None,
                        pivot_reason: None,
                        is_checkpoint: raw.is_checkpoint,
                        created_at: Some(bson::DateTime::now()),
                        started_at: None,
                        last_activity_at: None,
                        last_progress_at: None,
                        completed_at: None,
                    })
                    .collect::<Vec<_>>();
                Some(GoalCompletionSalvagePlan {
                    goals: built_goals,
                    reason: reason.clone(),
                })
            }
        } else {
            None
        };

        Ok(GoalPlanReviewResult {
            decision,
            selected_goal_id,
            reason,
            observed_evidence,
            missing_core_deliverables,
            salvage_plan,
        })
    }

    fn normalize_contradictory_completion_result(
        goals: &[GoalNode],
        mut result: GoalCompletionAssessorResult,
    ) -> GoalCompletionAssessorResult {
        if result.decision != MissionCompletionDecision::Complete {
            return result;
        }

        let unresolved_titles = goals
            .iter()
            .filter(|goal| {
                matches!(
                    goal.status,
                    GoalStatus::Pending | GoalStatus::Running | GoalStatus::Failed
                )
            })
            .map(|goal| goal.title.clone())
            .take(3)
            .collect::<Vec<_>>();

        if result.missing_core_deliverables.is_empty() && unresolved_titles.is_empty() {
            return result;
        }

        let bounded_repair_goals = Self::bounded_completion_repair_goals(goals);
        if !bounded_repair_goals.is_empty() {
            result.decision = MissionCompletionDecision::ContinueWithReplan;
            result.salvage_plan = Some(GoalCompletionSalvagePlan {
                goals: bounded_repair_goals.clone(),
                reason: result.reason.clone(),
            });
            if result.reason.is_none() {
                result.reason = Some(format!(
                    "A prior completion decision still left {} unresolved adaptive goal(s); continue with a bounded repair loop instead of closing the mission.",
                    bounded_repair_goals.len()
                ));
            }
            if result.missing_core_deliverables.is_empty() {
                result.missing_core_deliverables = unresolved_titles;
            }
            if result.observed_evidence.is_empty() {
                result.observed_evidence.push(
                    "A prior completion decision conflicted with unresolved adaptive goals."
                        .to_string(),
                );
            }
            return result;
        }

        result.decision = MissionCompletionDecision::PartialHandoff;
        result.salvage_plan = None;
        if result.reason.is_none() {
            result.reason = Some(
                "Useful partial delivery exists, but unresolved core work remains; treating the outcome as partial handoff instead of complete."
                    .to_string(),
            );
        }
        if result.missing_core_deliverables.is_empty() {
            result.missing_core_deliverables = unresolved_titles;
        }
        if result.observed_evidence.is_empty() {
            result.observed_evidence.push(
                "A prior completion decision conflicted with unresolved adaptive goals."
                    .to_string(),
            );
        }
        result
    }

    fn build_completion_assessor_prompt(mission_goal: &str, goals: &[GoalNode]) -> String {
        let goal_digest = Self::build_goal_evidence_digest(goals);
        format!(
            "You are the completion assessor for an adaptive long-running mission.\n\n\
Mission goal:\n{}\n\n\
Goal digest:\n{}\n\
Decide whether the mission is already sufficiently complete, or whether a single bounded salvage loop should fill the most important missing deliverables.\n\n\
Rules:\n\
- Prefer `complete` only when the mission's requested end-user outcome is materially delivered, not merely diagnosed.\n\
- Use `continue_with_replan` only when the remaining work is clearly bounded and can be completed in 1-3 delta goals.\n\
- Use `partial_handoff` when useful partial delivery exists but the remaining gaps are not worth another autonomous loop.\n\
- Treat `partial_handoff` as valid only when the already delivered outputs are directly reusable by the end user in their current state.\n\
- A scaffold, draft, placeholder, outline, contract, carrier file, or partially populated shell created mainly to enable later filling does not qualify as useful partial delivery unless the mission explicitly asked for that scaffold or draft itself.\n\
- If the main substance of the requested outcome still sits inside pending or unresolved goals, do not collapse to `partial_handoff`; prefer `continue_with_replan`, `blocked_by_environment`, `blocked_by_tooling`, or `blocked_fail`.\n\
- Use `blocked_by_environment` when the remaining gaps require capabilities or environment access the current runtime clearly does not have.\n\
- Use `blocked_by_tooling` when the remaining gaps are primarily caused by failing or unavailable tools / source-access paths.\n\
- A blocker note, preflight memo, risk note, or partial handoff document by itself does not count as `complete` unless the mission goal was only to produce that diagnosis.\n\
- If pending or undelivered goals remain because the requested work cannot be executed in this runtime, prefer `blocked_by_environment`, `blocked_by_tooling`, or `partial_handoff` over `complete`.\n\
- Do not restart previously completed goals.\n\
- Focus on core missing deliverables or synthesis gaps, not minor byproducts.\n\
- Use low-commitment, evidence-based reasoning.\n\n\
Return JSON only:\n\
{{\n\
  \"decision\": \"complete\" | \"continue_with_replan\" | \"partial_handoff\" | \"blocked_by_environment\" | \"blocked_by_tooling\" | \"blocked_fail\",\n\
  \"reason\": \"short explanation\",\n\
  \"observed_evidence\": [\"...\"],\n\
  \"missing_core_deliverables\": [\"...\"],\n\
  \"delta_goals\": [\n\
    {{\n\
      \"title\": \"...\",\n\
      \"description\": \"...\",\n\
      \"success_criteria\": \"...\",\n\
      \"is_checkpoint\": false\n\
    }}\n\
  ]\n\
}}\n\
If no salvage loop is needed, return an empty array for `delta_goals`.",
            mission_goal, goal_digest
        )
    }

    fn allocate_salvage_goal_ids(existing: &[GoalNode], count: usize) -> Vec<String> {
        let mut existing_ids = existing
            .iter()
            .map(|goal| goal.goal_id.clone())
            .collect::<std::collections::HashSet<_>>();
        let mut ids = Vec::with_capacity(count);
        let mut counter = 1usize;
        while ids.len() < count {
            let candidate = format!("g-salvage-{}", counter);
            if existing_ids.insert(candidate.clone()) {
                ids.push(candidate);
            }
            counter += 1;
        }
        ids
    }

    fn collect_executable_goals<'a>(goals: &'a [GoalNode]) -> Vec<&'a GoalNode> {
        let parent_ids_with_pending: Vec<&str> = goals
            .iter()
            .filter(|g| {
                matches!(
                    g.status,
                    GoalStatus::Pending
                        | GoalStatus::Pivoting
                        | GoalStatus::Running
                        | GoalStatus::AwaitingApproval
                )
            })
            .filter_map(|g| g.parent_id.as_deref())
            .collect();

        let mut candidates: Vec<&GoalNode> = goals
            .iter()
            .filter(|g| g.status == GoalStatus::Pending || g.status == GoalStatus::Pivoting)
            .filter(|g| !parent_ids_with_pending.contains(&g.goal_id.as_str()))
            .collect();

        candidates.sort_by(|a, b| b.depth.cmp(&a.depth).then(a.order.cmp(&b.order)));
        candidates
    }

    fn strategy_requests_bounded_replan(
        strategy: Option<&MissionStrategyState>,
    ) -> bool {
        strategy
            .and_then(|strategy| strategy.action.as_deref())
            .is_some_and(|action| action == "continue_with_replan")
    }

    fn build_remaining_plan_action_prompt(mission_goal: &str, goals: &[GoalNode]) -> String {
        let goal_digest = Self::build_goal_evidence_digest(goals);
        let candidates = Self::collect_executable_goals(goals);
        let mut candidate_digest = String::new();
        for goal in &candidates {
            candidate_digest.push_str(&format!(
                "- {} [{}]\n  title: {}\n  description: {}\n  success_criteria: {}\n",
                goal.goal_id,
                match goal.status {
                    GoalStatus::Pending => "pending",
                    GoalStatus::Pivoting => "pivoting",
                    GoalStatus::Running => "running",
                    GoalStatus::AwaitingApproval => "awaiting_approval",
                    GoalStatus::Completed => "completed",
                    GoalStatus::Failed => "failed",
                    GoalStatus::Abandoned => "abandoned",
                },
                goal.title,
                goal.description,
                goal.success_criteria,
            ));
        }
        if candidate_digest.trim().is_empty() {
            candidate_digest.push_str("- (none)\n");
        }

        format!(
            "You are the orchestration monitor for an adaptive long-running mission.\n\n\
Mission goal:\n{}\n\n\
Current goal/evidence snapshot:\n{}\n\
Executable candidate goals:\n{}\n\
Task:\n\
- Decide the best next orchestration action for the remaining plan.\n\
- Use `continue_current_plan` when one candidate goal is still the best next move. When you choose it, set `selected_goal_id` to that candidate.\n\
- Prefer goals whose prerequisites still fit the current evidence and environment.\n\
- If earlier completed goals established that a prerequisite capability, environment, or access path is unavailable, do not choose downstream goals that still depend on it.\n\
- Respect explicit guards such as \"only if feasible\", \"if supported\", or fallback/handoff goals intended for blocked environments.\n\
- Use `continue_with_replan` only when the remaining work should be replaced with 1-3 bounded delta goals.\n\
- Use `complete_if_evidence_sufficient` only when the requested end-user outcome is already materially delivered.\n\
- Use `partial_handoff`, `blocked_by_environment`, or `blocked_by_tooling` when another autonomous loop is no longer the right move.\n\
- Keep the reasoning evidence-based and low-commitment.\n\n\
Return JSON only:\n\
{{\n\
  \"decision\": \"continue_current_plan\" | \"continue_with_replan\" | \"complete_if_evidence_sufficient\" | \"partial_handoff\" | \"blocked_by_environment\" | \"blocked_by_tooling\" | \"blocked_fail\",\n\
  \"selected_goal_id\": \"candidate goal id\" | null,\n\
  \"reason\": \"short explanation\",\n\
  \"observed_evidence\": [\"...\"],\n\
  \"missing_core_deliverables\": [\"...\"],\n\
  \"delta_goals\": [\n\
    {{\n\
      \"title\": \"...\",\n\
      \"description\": \"...\",\n\
      \"success_criteria\": \"...\",\n\
      \"is_checkpoint\": false\n\
    }}\n\
  ]\n\
}}",
            mission_goal, goal_digest, candidate_digest
        )
    }

    fn parse_completion_salvage_response(
        goals: &[GoalNode],
        response: &str,
    ) -> Result<GoalCompletionAssessorResult> {
        #[derive(serde::Deserialize)]
        struct DeltaGoal {
            title: String,
            description: String,
            success_criteria: String,
            #[serde(default)]
            is_checkpoint: bool,
        }

        let value = runtime::parse_first_json_value(response)
            .or_else(|_| runtime::parse_first_json_value(&runtime::extract_json_block(response)))
            .map_err(|err| anyhow!("Failed to parse adaptive completion assessor JSON: {}", err))?;
        let decision = MissionCompletionDecision::from_assessor_decision(
            value
                .get("decision")
                .and_then(|v| v.as_str())
                .unwrap_or("complete"),
        );
        let reason = value
            .get("reason")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let observed_evidence = value
            .get("observed_evidence")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(|item| item.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let missing_core_deliverables = value
            .get("missing_core_deliverables")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(|item| item.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let salvage_plan = if decision == MissionCompletionDecision::ContinueWithReplan {
            let raw_goals_value = value
                .get("delta_goals")
                .or_else(|| value.get("goals"))
                .cloned()
                .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
            let raw_goals: Vec<DeltaGoal> = serde_json::from_value(raw_goals_value)?;
            if raw_goals.is_empty() {
                return Err(anyhow!(
                    "Adaptive completion assessor requested continue_with_replan without delta goals"
                ));
            }
            let start_order = goals.iter().map(|goal| goal.order).max().unwrap_or(0);
            let salvage_ids = Self::allocate_salvage_goal_ids(goals, raw_goals.len());
            let built_goals = raw_goals
                .into_iter()
                .zip(salvage_ids.into_iter())
                .enumerate()
                .map(|(index, (raw, goal_id))| GoalNode {
                    goal_id,
                    parent_id: None,
                    title: raw.title,
                    description: raw.description,
                    success_criteria: raw.success_criteria,
                    status: GoalStatus::Pending,
                    depth: 0,
                    order: start_order + index as u32 + 1,
                    exploration_budget: 3,
                    attempts: vec![],
                    output_summary: None,
                    runtime_contract: None,
                    contract_verification: None,
                    pivot_reason: None,
                    is_checkpoint: raw.is_checkpoint,
                    created_at: Some(bson::DateTime::now()),
                    started_at: None,
                    last_activity_at: None,
                    last_progress_at: None,
                    completed_at: None,
                })
                .collect::<Vec<_>>();
            Some(GoalCompletionSalvagePlan {
                goals: built_goals,
                reason: reason.clone(),
            })
        } else {
            None
        };
        Ok(GoalCompletionAssessorResult {
            decision,
            reason,
            observed_evidence,
            missing_core_deliverables,
            salvage_plan,
        })
    }

    async fn evaluate_completion_salvage(
        &self,
        mission: &MissionDoc,
        mission_id: &str,
        agent_id: &str,
        workspace_path: Option<&str>,
    ) -> Result<GoalCompletionAssessorResult> {
        if let Err(err) = runtime::reconcile_mission_artifacts(&self.agent_service, mission).await {
            tracing::warn!(
                "Failed to reconcile workspace artifacts before adaptive completion assessment for mission {}: {}",
                mission_id,
                err
            );
        }

        let goals = mission.goal_tree.as_deref().unwrap_or(&[]);
        let prompt = Self::build_completion_assessor_prompt(&mission.goal, goals);
        let response = self
            .execute_goal_monitor_in_isolated_session(
                mission,
                agent_id,
                mission_id,
                &prompt,
                workspace_path,
            )
            .await?;
        let mut result = Self::parse_completion_salvage_response(goals, &response)?;

        if Self::completion_review_needed(goals, &result) {
            let review_prompt = Self::build_completion_review_prompt(&mission.goal, goals, &result);
            match self
                .execute_goal_monitor_in_isolated_session(
                    mission,
                    agent_id,
                    mission_id,
                    &review_prompt,
                    workspace_path,
                )
                .await
                .and_then(|review_response| {
                    Self::parse_completion_salvage_response(goals, &review_response)
                }) {
                Ok(reviewed) => {
                    result = reviewed;
                }
                Err(err) => {
                    tracing::warn!(
                        "Adaptive mission {} completion review failed; keeping initial assessment: {}",
                        mission_id,
                        err
                    );
                }
            }
        }

        if Self::completion_fallback_needed(goals, &result) {
            result = Self::normalize_contradictory_completion_result(goals, result);
        }

        Ok(result)
    }

    async fn maybe_resolve_goal_gap(
        &self,
        mission_id: &str,
        agent_id: &str,
        goal: &GoalNode,
        workspace_path: Option<&str>,
        trigger_reason: &str,
    ) -> Result<Option<GoalLoopResolution>> {
        let Some(mission) = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
        else {
            return Ok(None);
        };
        if !Self::goal_tree_is_usable(mission.goal_tree.as_deref()) {
            return Ok(None);
        }

        let result = match self
            .evaluate_completion_salvage(&mission, mission_id, agent_id, workspace_path)
            .await
        {
            Ok(result) => result,
            Err(err) => {
                tracing::warn!(
                    "Adaptive mission {} failed to classify goal {} gap after trigger '{}': {}",
                    mission_id,
                    goal.goal_id,
                    trigger_reason,
                    err
                );
                return Ok(None);
            }
        };

        match result.decision {
            MissionCompletionDecision::ContinueWithReplan => {
                let Some(plan) = result.salvage_plan.clone() else {
                    return Ok(None);
                };
                let mut all_goals = mission.goal_tree.clone().unwrap_or_default();
                let plan_goals = plan.goals.clone();
                let mut preserved_goal_ids = Vec::new();
                let active_repair_lane_id = if Self::goal_is_salvage_like(goal) {
                    Some(goal.goal_id.clone())
                } else {
                    plan_goals.first().map(|item| item.goal_id.clone())
                };
                if Self::goal_is_salvage_like(goal) && !plan.goals.is_empty() {
                    let mut replacement_goals = plan.goals.clone();
                    let mut reused_lane = replacement_goals.remove(0);
                    reused_lane.goal_id = goal.goal_id.clone();
                    reused_lane.parent_id = goal.parent_id.clone();
                    reused_lane.depth = goal.depth;
                    reused_lane.order = goal.order;
                    reused_lane.pivot_reason = Some("bounded_completion_repair".to_string());
                    reused_lane.created_at = Some(bson::DateTime::now());
                    reused_lane.started_at = None;
                    reused_lane.last_activity_at = None;
                    reused_lane.last_progress_at = None;
                    reused_lane.completed_at = None;

                    let max_existing_order = all_goals.iter().map(|item| item.order).max().unwrap_or(0);
                    let mut inserted_reused_lane = false;
                    all_goals = all_goals
                        .into_iter()
                        .filter_map(|existing| {
                            if existing.goal_id == goal.goal_id {
                                inserted_reused_lane = true;
                                return Some(reused_lane.clone());
                            }
                            if Self::goal_is_stale_pending_salvage(&existing) {
                                return None;
                            }
                            Some(existing)
                        })
                        .collect();
                    if !inserted_reused_lane {
                        all_goals.push(reused_lane);
                    }
                    preserved_goal_ids.push(goal.goal_id.clone());

                    let extra_goals = replacement_goals
                        .into_iter()
                        .take(2)
                        .enumerate()
                        .map(|(index, mut extra_goal)| {
                            extra_goal.order = max_existing_order + index as u32 + 1;
                            extra_goal
                        })
                        .collect::<Vec<_>>();
                    preserved_goal_ids.extend(extra_goals.iter().map(|goal| goal.goal_id.clone()));
                    all_goals.extend(extra_goals);
                } else {
                    all_goals = all_goals
                        .into_iter()
                        .filter(|existing| !Self::goal_is_stale_pending_salvage(existing))
                        .collect();
                    let new_goals = plan.goals.clone().into_iter().take(3).collect::<Vec<_>>();
                    preserved_goal_ids.extend(new_goals.iter().map(|goal| goal.goal_id.clone()));
                    all_goals.extend(new_goals);
                }
                let supersede_reason = result.reason.clone().unwrap_or_else(|| {
                    "Remaining work was replaced with a bounded adaptive repair lane"
                        .to_string()
                });
                let superseded = Self::supersede_open_goals_in_tree(
                    &mut all_goals,
                    &preserved_goal_ids,
                    &supersede_reason,
                );
                self.agent_service
                    .save_goal_tree(mission_id, all_goals)
                    .await
                    .map_err(|e| anyhow!("Failed to persist adaptive repair plan: {}", e))?;
                let convergence_patch = MissionConvergencePatch {
                    active_repair_lane_id: Some(active_repair_lane_id),
                    consecutive_no_tool_count: Some(0),
                    last_blocker_fingerprint: Some(runtime::blocker_fingerprint(trigger_reason)),
                    waiting_external_until: Some(None),
                };
                if let Err(err) = self
                    .agent_service
                    .patch_mission_convergence_state(mission_id, &convergence_patch)
                    .await
                {
                    tracing::warn!(
                        "Failed to persist repair lane convergence state for mission {}: {}",
                        mission_id,
                        err
                    );
                }

                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: serde_json::json!({
                                "type": "goal_gap_replanned",
                                "goal_id": goal.goal_id,
                                "new_goal_count": preserved_goal_ids.len(),
                                "superseded_goal_count": superseded,
                                "reused_repair_lane": Self::goal_is_salvage_like(goal),
                                "reason": plan.reason,
                                "observed_evidence": result.observed_evidence,
                                "missing_core_deliverables": result.missing_core_deliverables,
                            })
                            .to_string(),
                        },
                    )
                    .await;
                Ok(Some(GoalLoopResolution::Continue))
            }
            MissionCompletionDecision::Complete
            | MissionCompletionDecision::CompletedWithMinorGaps => {
                if let Some(assessment) = result.completion_assessment() {
                    if let Err(err) = self
                        .agent_service
                        .set_mission_completion_assessment(mission_id, &assessment)
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist adaptive completion assessment for mission {}: {}",
                            mission_id,
                            err
                        );
                    }
                }
                if let Err(err) = self
                    .agent_service
                    .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::Completed)
                    .await
                {
                    tracing::warn!(
                        "Failed to mark goal {} completed after semantic completion assessment: {}",
                        goal.goal_id,
                        err
                    );
                }
                if let Some(reason) = result.reason.as_deref() {
                    if let Err(err) = self
                        .agent_service
                        .set_goal_output_summary(mission_id, &goal.goal_id, reason)
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist semantic completion summary for goal {}: {}",
                            goal.goal_id,
                            err
                        );
                    }
                }
                self.record_goal_worker_state(
                    mission_id,
                    goal,
                    goal.attempts.len().max(1) as u32,
                    goal.runtime_contract
                        .as_ref()
                        .map(|contract| contract.required_artifacts.clone())
                        .unwrap_or_default(),
                    None,
                    Some("semantic completion review accepted the current goal result"),
                    None,
                    result.observed_evidence.clone(),
                    result
                        .reason
                        .as_deref()
                        .map(|text| vec![Self::compact_goal_prompt_text(text, 220)])
                        .unwrap_or_default(),
                    None,
                )
                .await;
                let convergence_patch = MissionConvergencePatch {
                    active_repair_lane_id: Some(None),
                    consecutive_no_tool_count: Some(0),
                    last_blocker_fingerprint: Some(None),
                    waiting_external_until: Some(None),
                };
                if let Err(err) = self
                    .agent_service
                    .patch_mission_convergence_state(mission_id, &convergence_patch)
                    .await
                {
                    tracing::warn!(
                        "Failed to clear convergence state after semantic completion for mission {} goal {}: {}",
                        mission_id,
                        goal.goal_id,
                        err
                    );
                }
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: serde_json::json!({
                                "type": "goal_gap_completed",
                                "goal_id": goal.goal_id,
                                "decision": match result.decision {
                                    MissionCompletionDecision::CompletedWithMinorGaps => "completed_with_minor_gaps",
                                    _ => "complete",
                                },
                                "reason": result.reason,
                                "observed_evidence": result.observed_evidence,
                                "missing_core_deliverables": result.missing_core_deliverables,
                            })
                            .to_string(),
                        },
                    )
                    .await;
                if let Err(err) = self
                    .agent_service
                    .clear_mission_current_goal(mission_id)
                    .await
                {
                    tracing::warn!(
                        "Failed to clear current goal before semantic completion synthesis for mission {}: {}",
                        mission_id,
                        err
                    );
                }
                Ok(Some(GoalLoopResolution::StopForSynthesis))
            }
            MissionCompletionDecision::WaitingExternal => {
                if let Some(assessment) = result.completion_assessment() {
                    if let Err(err) = self
                        .agent_service
                        .set_mission_completion_assessment(mission_id, &assessment)
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist waiting-external assessment for mission {}: {}",
                            mission_id,
                            err
                        );
                    }
                }
                let wait_reason = result.reason.clone().unwrap_or_else(|| {
                    format!(
                        "Adaptive goal is waiting on an external dependency after semantic gap assessment: {}",
                        trigger_reason
                    )
                });
                let waiting_external_until = Self::waiting_external_until_after_cooldown();
                let intervention = MissionMonitorIntervention {
                    action: "mark_waiting_external".to_string(),
                    feedback: Some(wait_reason.clone()),
                    semantic_tags: vec![
                        "waiting_external".to_string(),
                        "semantic_gap".to_string(),
                    ],
                    observed_evidence: result.observed_evidence.clone(),
                    missing_core_deliverables: result.missing_core_deliverables.clone(),
                    confidence: None,
                    strategy_patch: None,
                    subagent_recommended: None,
                    parallelism_budget: None,
                    requested_at: Some(mongodb::bson::DateTime::now()),
                    applied_at: None,
                };
                self.record_goal_stuck_snapshot(
                    mission_id,
                    goal,
                    result.observed_evidence.clone(),
                    &wait_reason,
                    goal.attempts
                        .iter()
                        .map(|attempt| attempt.approach.clone())
                        .collect(),
                    Some("wait for the external dependency, then resume with the preserved workspace"),
                    result.missing_core_deliverables.clone(),
                )
                .await;
                let convergence_patch = MissionConvergencePatch {
                    active_repair_lane_id: Some(if Self::goal_is_salvage_like(goal) {
                        Some(goal.goal_id.clone())
                    } else {
                        None
                    }),
                    consecutive_no_tool_count: Some(0),
                    last_blocker_fingerprint: Some(runtime::blocker_fingerprint(&wait_reason)),
                    waiting_external_until: Some(Some(waiting_external_until)),
                };
                if let Err(err) = self
                    .agent_service
                    .patch_mission_convergence_state(mission_id, &convergence_patch)
                    .await
                {
                    tracing::warn!(
                        "Failed to persist waiting_external convergence state for mission {} goal {}: {}",
                        mission_id,
                        goal.goal_id,
                        err
                    );
                }
                self.persist_goal_monitor_intervention(mission_id, &goal.goal_id, &intervention)
                    .await;
                if let Err(err) = self
                    .agent_service
                    .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::Pending)
                    .await
                {
                    tracing::warn!(
                        "Failed to reset goal {} to pending after waiting-external assessment: {}",
                        goal.goal_id,
                        err
                    );
                }
                if let Err(err) = self
                    .agent_service
                    .clear_mission_current_goal(mission_id)
                    .await
                {
                    tracing::warn!(
                        "Failed to clear current goal before waiting-external continue for mission {}: {}",
                        mission_id,
                        err
                    );
                }
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: serde_json::json!({
                                "type": "goal_gap_waiting_external",
                                "goal_id": goal.goal_id,
                                "reason": result.reason,
                                "observed_evidence": result.observed_evidence,
                                "missing_core_deliverables": result.missing_core_deliverables,
                            })
                            .to_string(),
                        },
                    )
                    .await;
                Ok(Some(GoalLoopResolution::Continue))
            }
            MissionCompletionDecision::PartialHandoff
            | MissionCompletionDecision::BlockedByEnvironment
            | MissionCompletionDecision::BlockedByTooling
            | MissionCompletionDecision::BlockedFail => {
                if let Some(assessment) = result.completion_assessment() {
                    if let Err(err) = self
                        .agent_service
                        .set_mission_completion_assessment(mission_id, &assessment)
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist adaptive terminal assessment for mission {}: {}",
                            mission_id,
                            err
                        );
                    }
                }
                let abandon_reason = result.reason.clone().unwrap_or_else(|| {
                    format!(
                        "Adaptive goal stopped after semantic gap assessment: {}",
                        trigger_reason
                    )
                });
                self.record_goal_stuck_snapshot(
                    mission_id,
                    goal,
                    result.observed_evidence.clone(),
                    &abandon_reason,
                    goal.attempts
                        .iter()
                        .map(|attempt| attempt.approach.clone())
                        .collect(),
                    Some("handoff the current result or change environment/tooling before resuming"),
                    result.missing_core_deliverables.clone(),
                )
                .await;
                let convergence_patch = MissionConvergencePatch {
                    active_repair_lane_id: Some(None),
                    consecutive_no_tool_count: Some(0),
                    last_blocker_fingerprint: Some(runtime::blocker_fingerprint(&abandon_reason)),
                    waiting_external_until: Some(None),
                };
                if let Err(err) = self
                    .agent_service
                    .patch_mission_convergence_state(mission_id, &convergence_patch)
                    .await
                {
                    tracing::warn!(
                        "Failed to persist terminal convergence state for mission {} goal {}: {}",
                        mission_id,
                        goal.goal_id,
                        err
                    );
                }
                if let Err(err) = self
                    .agent_service
                    .abandon_goal_atomic(mission_id, &goal.goal_id, &abandon_reason)
                    .await
                {
                    tracing::warn!(
                        "Failed to abandon goal {} after semantic gap assessment: {}",
                        goal.goal_id,
                        err
                    );
                }
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: serde_json::json!({
                                "type": "goal_gap_terminal",
                                "goal_id": goal.goal_id,
                                "decision": match result.decision {
                                    MissionCompletionDecision::PartialHandoff => "partial_handoff",
                                    MissionCompletionDecision::BlockedByEnvironment => "blocked_by_environment",
                                    MissionCompletionDecision::BlockedByTooling => "blocked_by_tooling",
                                    MissionCompletionDecision::BlockedFail => "blocked_fail",
                                    MissionCompletionDecision::WaitingExternal => "waiting_external",
                                    MissionCompletionDecision::CompletedWithMinorGaps => "completed_with_minor_gaps",
                                    _ => "complete",
                                },
                                "reason": result.reason,
                                "observed_evidence": result.observed_evidence,
                                "missing_core_deliverables": result.missing_core_deliverables,
                            })
                            .to_string(),
                        },
                    )
                    .await;
                if let Err(err) = self
                    .agent_service
                    .clear_mission_current_goal(mission_id)
                    .await
                {
                    tracing::warn!(
                        "Failed to clear current goal before terminal semantic synthesis for mission {}: {}",
                        mission_id,
                        err
                    );
                }
                Ok(Some(GoalLoopResolution::StopForSynthesis))
            }
        }
    }

    async fn supersede_open_goals(
        &self,
        mission_id: &str,
        preserve_goal_id: Option<&str>,
        reason: &str,
    ) -> Result<usize> {
        let Some(mission) = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
        else {
            return Ok(0);
        };

        let mut superseded = 0usize;
        for open_goal in mission
            .goal_tree
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter(|open_goal| {
                preserve_goal_id != Some(open_goal.goal_id.as_str())
                    && matches!(
                        open_goal.status,
                        GoalStatus::Pending
                            | GoalStatus::Running
                            | GoalStatus::Failed
                            | GoalStatus::Pivoting
                            | GoalStatus::AwaitingApproval
                    )
            })
        {
            if let Err(err) = self
                .agent_service
                .abandon_goal_atomic(mission_id, &open_goal.goal_id, reason)
                .await
            {
                tracing::warn!(
                    "Failed to supersede goal {} for mission {}: {}",
                    open_goal.goal_id,
                    mission_id,
                    err
                );
                continue;
            }
            superseded += 1;
        }

        Ok(superseded)
    }

    async fn maybe_review_remaining_plan_after_goal_completion(
        &self,
        mission_id: &str,
        agent_id: &str,
        completed_goal: &GoalNode,
        workspace_path: Option<&str>,
    ) -> Result<Option<GoalLoopResolution>> {
        let Some(mission) = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
        else {
            return Ok(None);
        };
        if !Self::strategy_requests_bounded_replan(mission.current_strategy.as_ref()) {
            return Ok(None);
        }
        let goals = mission.goal_tree.as_deref().unwrap_or(&[]);
        let completed_goal = goals
            .iter()
            .find(|goal| goal.goal_id == completed_goal.goal_id)
            .cloned()
            .unwrap_or_else(|| completed_goal.clone());
        let remaining_goals = goals
            .iter()
            .filter(|goal| {
                goal.goal_id != completed_goal.goal_id
                    && matches!(
                        goal.status,
                        GoalStatus::Pending
                            | GoalStatus::Running
                            | GoalStatus::Failed
                            | GoalStatus::Pivoting
                            | GoalStatus::AwaitingApproval
                    )
            })
            .cloned()
            .collect::<Vec<_>>();
        if remaining_goals.is_empty() {
            return Ok(None);
        }

        let prompt = Self::build_post_goal_plan_review_prompt(
            &mission.goal,
            &completed_goal,
            &remaining_goals,
        );
        let result = match self
            .execute_post_goal_plan_review_with_repair(
                &mission,
                agent_id,
                mission_id,
                workspace_path,
                &prompt,
            )
            .await
        {
            Ok(result) => result,
            Err(err) => {
                tracing::warn!(
                    "Adaptive mission {} produced invalid post-goal plan review after {}: {}",
                    mission_id,
                    completed_goal.goal_id,
                    err
                );
                return Ok(None);
            }
        };

        match result.decision {
            GoalPlanReviewDecision::ContinueCurrentPlan => Ok(None),
            GoalPlanReviewDecision::ContinueWithReplan => {
                let Some(plan) = result.salvage_plan.clone() else {
                    return Ok(None);
                };
                let mut all_goals = mission.goal_tree.clone().unwrap_or_default();
                let new_goals = plan.goals.clone();
                let mut preserved_goal_ids = vec![completed_goal.goal_id.clone()];
                preserved_goal_ids.extend(new_goals.iter().map(|goal| goal.goal_id.clone()));
                let supersede_reason = result.reason.clone().unwrap_or_else(|| {
                    format!(
                        "Remaining goals were superseded after {} by a bounded adaptive repair plan",
                        completed_goal.goal_id
                    )
                });
                all_goals.extend(new_goals.clone());
                let superseded = Self::supersede_open_goals_in_tree(
                    &mut all_goals,
                    &preserved_goal_ids,
                    &supersede_reason,
                );
                self.agent_service
                    .save_goal_tree(mission_id, all_goals)
                    .await
                    .map_err(|e| {
                        anyhow!("Failed to persist post-goal adaptive repair plan: {}", e)
                    })?;
                let convergence_patch = MissionConvergencePatch {
                    active_repair_lane_id: Some(new_goals.first().map(|goal| goal.goal_id.clone())),
                    consecutive_no_tool_count: Some(0),
                    last_blocker_fingerprint: Some(runtime::blocker_fingerprint(&supersede_reason)),
                    waiting_external_until: Some(None),
                };
                if let Err(err) = self
                    .agent_service
                    .patch_mission_convergence_state(mission_id, &convergence_patch)
                    .await
                {
                    tracing::warn!(
                        "Failed to persist post-goal repair convergence state for mission {}: {}",
                        mission_id,
                        err
                    );
                }
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: serde_json::json!({
                                "type": "goal_plan_replanned",
                                "goal_id": completed_goal.goal_id,
                                "new_goal_count": plan.goals.len(),
                                "superseded_goal_count": superseded,
                                "reason": plan.reason,
                                "observed_evidence": result.observed_evidence,
                                "missing_core_deliverables": result.missing_core_deliverables,
                            })
                            .to_string(),
                        },
                    )
                    .await;
                Ok(Some(GoalLoopResolution::Continue))
            }
            GoalPlanReviewDecision::CompleteIfEvidenceSufficient
            | GoalPlanReviewDecision::PartialHandoff
            | GoalPlanReviewDecision::BlockedByEnvironment
            | GoalPlanReviewDecision::BlockedByTooling
            | GoalPlanReviewDecision::BlockedFail => {
                let assessment = match result.decision {
                    GoalPlanReviewDecision::CompleteIfEvidenceSufficient => {
                        MissionCompletionDecision::Complete.to_assessment(
                            result.reason.clone(),
                            result.observed_evidence.clone(),
                            result.missing_core_deliverables.clone(),
                        )
                    }
                    GoalPlanReviewDecision::PartialHandoff => {
                        MissionCompletionDecision::PartialHandoff.to_assessment(
                            result.reason.clone(),
                            result.observed_evidence.clone(),
                            result.missing_core_deliverables.clone(),
                        )
                    }
                    GoalPlanReviewDecision::BlockedByEnvironment => {
                        MissionCompletionDecision::BlockedByEnvironment.to_assessment(
                            result.reason.clone(),
                            result.observed_evidence.clone(),
                            result.missing_core_deliverables.clone(),
                        )
                    }
                    GoalPlanReviewDecision::BlockedByTooling => {
                        MissionCompletionDecision::BlockedByTooling.to_assessment(
                            result.reason.clone(),
                            result.observed_evidence.clone(),
                            result.missing_core_deliverables.clone(),
                        )
                    }
                    GoalPlanReviewDecision::BlockedFail => MissionCompletionDecision::BlockedFail
                        .to_assessment(
                            result.reason.clone(),
                            result.observed_evidence.clone(),
                            result.missing_core_deliverables.clone(),
                        ),
                    GoalPlanReviewDecision::ContinueCurrentPlan
                    | GoalPlanReviewDecision::ContinueWithReplan => None,
                };
                if let Some(assessment) = assessment.as_ref() {
                    if let Err(err) = self
                        .agent_service
                        .set_mission_completion_assessment(mission_id, assessment)
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist post-goal plan assessment for mission {}: {}",
                            mission_id,
                            err
                        );
                    }
                }
                let supersede_reason = result.reason.clone().unwrap_or_else(|| {
                    format!(
                        "Remaining goals were closed after {} by post-goal semantic review",
                        completed_goal.goal_id
                    )
                });
                let superseded = self
                    .supersede_open_goals(
                        mission_id,
                        Some(&completed_goal.goal_id),
                        &supersede_reason,
                    )
                    .await?;
                if let Err(err) = self
                    .agent_service
                    .clear_mission_current_goal(mission_id)
                    .await
                {
                    tracing::warn!(
                        "Failed to clear current goal before post-goal synthesis for mission {}: {}",
                        mission_id,
                        err
                    );
                }
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: serde_json::json!({
                                "type": "goal_plan_closed",
                                "goal_id": completed_goal.goal_id,
                                "decision": match result.decision {
                                    GoalPlanReviewDecision::CompleteIfEvidenceSufficient => "complete_if_evidence_sufficient",
                                    GoalPlanReviewDecision::PartialHandoff => "partial_handoff",
                                    GoalPlanReviewDecision::BlockedByEnvironment => "blocked_by_environment",
                                    GoalPlanReviewDecision::BlockedByTooling => "blocked_by_tooling",
                                    GoalPlanReviewDecision::BlockedFail => "blocked_fail",
                                    _ => "continue_current_plan",
                                },
                                "superseded_goal_count": superseded,
                                "reason": result.reason,
                                "observed_evidence": result.observed_evidence,
                                "missing_core_deliverables": result.missing_core_deliverables,
                            })
                            .to_string(),
                        },
                    )
                    .await;
                Ok(Some(GoalLoopResolution::StopForSynthesis))
            }
        }
    }

    /// Core execution loop — iterates over goal tree using state machine pattern.
    async fn execute_goal_loop(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
        operator_hint: Option<&str>,
    ) -> Result<()> {
        loop {
            // 1. Reload goal tree from DB
            let mission = self
                .agent_service
                .get_mission(mission_id)
                .await
                .map_err(|e| anyhow!("DB error: {}", e))?
                .ok_or_else(|| anyhow!("Mission not found"))?;

            if let Some(waiting_until) = mission.waiting_external_until {
                if let Some(delay) = Self::waiting_external_remaining_delay(waiting_until) {
                    tracing::info!(
                        "Mission {} is parked in waiting_external for {:?} before the next adaptive decision round",
                        mission_id,
                        delay
                    );
                    tokio::select! {
                        _ = cancel_token.cancelled() => return Ok(()),
                        _ = tokio::time::sleep(delay) => {}
                    }
                    self.clear_expired_waiting_external_hold(mission_id, &mission)
                        .await;
                    continue;
                }
                self.clear_expired_waiting_external_hold(mission_id, &mission)
                    .await;
            }

            let goals = mission.goal_tree.as_deref().unwrap_or(&[]);

            // 2. Let the monitor/assessor decide the next orchestration action.
            let next = self
                .decide_next_goal_action(&mission, agent_id, mission_id, workspace_path)
                .await?;
            let goal = match next {
                NextGoalDirective::Execute(g) => g,
                NextGoalDirective::Continue => continue,
                NextGoalDirective::StopForSynthesis => return Ok(()),
                NextGoalDirective::Break => break,
            };

            // 3. Check cancellation — return Ok so outer cleanup reads actual DB status
            if cancel_token.is_cancelled() {
                // Only set Cancelled if not already Paused (pause route sets Paused before cancelling token)
                if let Ok(Some(m)) = self.agent_service.get_mission(mission_id).await {
                    if m.status != MissionStatus::Paused {
                        if let Err(e) = self
                            .agent_service
                            .update_mission_status(mission_id, &MissionStatus::Cancelled)
                            .await
                        {
                            tracing::warn!(
                                "Failed to mark mission {} cancelled during adaptive loop: {}",
                                mission_id,
                                e
                            );
                        }
                    }
                }
                return Ok(());
            }

            // 5. Check approval policy for goals.
            // A goal approved via route marks current_goal_id to bypass re-pause.
            let goal_approved = mission.current_goal_id.as_deref() == Some(goal.goal_id.as_str())
                && goal.status == GoalStatus::Pending;
            let needs_approval = match mission.approval_policy {
                ApprovalPolicy::Auto => false,
                ApprovalPolicy::Checkpoint => goal.is_checkpoint,
                ApprovalPolicy::Manual => true,
            };
            if needs_approval && goal.status == GoalStatus::Pending && !goal_approved {
                if let Err(e) = self
                    .agent_service
                    .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::AwaitingApproval)
                    .await
                {
                    tracing::warn!(
                        "Failed to set goal {} awaiting_approval: {}",
                        goal.goal_id,
                        e
                    );
                }
                if let Err(e) = self
                    .agent_service
                    .update_mission_status(mission_id, &MissionStatus::Paused)
                    .await
                {
                    tracing::warn!("Failed to pause mission {}: {}", mission_id, e);
                }
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: format!(
                                r#"{{"type":"mission_paused","goal_id":"{}","reason":"{}"}}"#,
                                goal.goal_id,
                                if mission.approval_policy == ApprovalPolicy::Manual {
                                    "manual"
                                } else {
                                    "checkpoint"
                                }
                            ),
                        },
                    )
                    .await;
                return Ok(());
            }

            // 6. Collect completed goals for context
            let completed_goals: Vec<&GoalNode> = goals
                .iter()
                .filter(|g| g.status == GoalStatus::Completed)
                .collect();
            let goal_step_index = completed_goals.len() as u32;

            let policy_str = match mission.approval_policy {
                ApprovalPolicy::Auto => "auto",
                ApprovalPolicy::Checkpoint => "checkpoint",
                ApprovalPolicy::Manual => "manual",
            };

            // 7. Execute goal
            let merged_operator_hint = operator_hint.map(str::to_string);

            let workspace_before = match workspace_path {
                Some(wp) => runtime::snapshot_workspace_files(wp).ok(),
                None => None,
            };
            let goal_contract = match self
                .run_single_goal(
                    mission_id,
                    agent_id,
                    session_id,
                    &goal,
                    &completed_goals,
                    cancel_token.clone(),
                    workspace_path,
                    policy_str,
                    completed_goals.len() + 1,
                    goals.len(),
                    mission.step_timeout_seconds,
                    mission.step_max_retries,
                    merged_operator_hint.as_deref(),
                )
                .await
            {
                Ok(contract) => contract,
                Err(err) => {
                    if Self::goal_error_is_provider_capacity_block(&err.to_string()) {
                        tracing::warn!(
                            "Mission {} goal {} hit upstream provider capacity block; deferring goal instead of failing: {}",
                            mission_id,
                            goal.goal_id,
                            err
                        );
                        if let Some(resolution) = self
                            .defer_goal_for_provider_capacity(
                                mission_id,
                                &goal,
                                &err,
                                &cancel_token,
                            )
                            .await?
                        {
                            match resolution {
                                GoalLoopResolution::Continue => continue,
                                GoalLoopResolution::StopForSynthesis => return Ok(()),
                            }
                        }
                    }
                    if let Some(signal) = Self::soft_goal_terminal_signal(&err) {
                        tracing::warn!(
                            "Mission {} goal {} ended with soft terminal error; switching to monitor-driven resolution: {}",
                            mission_id,
                            goal.goal_id,
                            err
                        );
                        self.record_soft_goal_attempt(mission_id, &goal, &signal, &err)
                            .await;
                        if matches!(signal, ProgressSignal::Blocked)
                            || goal.attempts.len() as u32 + 1 >= goal.exploration_budget
                        {
                            if let Some(resolution) = self
                                .maybe_resolve_goal_gap(
                                    mission_id,
                                    agent_id,
                                    &goal,
                                    workspace_path,
                                    &err.to_string(),
                                )
                                .await?
                            {
                                match resolution {
                                    GoalLoopResolution::Continue => continue,
                                    GoalLoopResolution::StopForSynthesis => return Ok(()),
                                }
                            }
                        }
                        match signal {
                            ProgressSignal::Advancing => {}
                            ProgressSignal::Stalled => {
                                let attempt_count = goal.attempts.len() as u32 + 1;
                                if attempt_count >= goal.exploration_budget {
                                    tracing::info!(
                                        "Mission {} goal {} exhausted its soft-terminal exploration budget without a monitor-directed replan; resetting to pending for the joint-drive loop",
                                        mission_id,
                                        goal.goal_id
                                    );
                                    if let Err(e) = self
                                        .agent_service
                                        .update_goal_status(
                                            mission_id,
                                            &goal.goal_id,
                                            &GoalStatus::Pending,
                                        )
                                        .await
                                    {
                                        tracing::warn!(
                                            "Failed to reset goal {} to pending after soft terminal budget exhaustion: {}",
                                            goal.goal_id,
                                            e
                                        );
                                    }
                                } else if let Err(e) = self
                                    .agent_service
                                    .update_goal_status(
                                        mission_id,
                                        &goal.goal_id,
                                        &GoalStatus::Pending,
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        "Failed to reset goal {} to pending after soft error: {}",
                                        goal.goal_id,
                                        e
                                    );
                                }
                            }
                            ProgressSignal::Blocked => {
                                tracing::info!(
                                    "Mission {} goal {} hit a soft-terminal blocked signal without a monitor-directed resolution; resetting to pending for the joint-drive loop",
                                    mission_id,
                                    goal.goal_id
                                );
                                if let Err(e) = self
                                    .agent_service
                                    .update_goal_status(
                                        mission_id,
                                        &goal.goal_id,
                                        &GoalStatus::Pending,
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        "Failed to reset blocked goal {} to pending after soft error: {}",
                                        goal.goal_id,
                                        e
                                    );
                                }
                            }
                        }
                        continue;
                    }
                    return Err(err);
                }
            };

            // Pause/cancel can happen while goal is executing.
            // If so, stop the loop without evaluating progress.
            if let Ok(Some(current)) = self.agent_service.get_mission(mission_id).await {
                if matches!(
                    current.status,
                    MissionStatus::Paused | MissionStatus::Cancelled
                ) {
                    return Ok(());
                }
            }

            // 8. Evaluate progress
            let signal = self
                .evaluate_goal(
                    mission_id,
                    agent_id,
                    session_id,
                    &goal,
                    cancel_token.clone(),
                    workspace_path,
                )
                .await?;

            // 8.1 Update the last attempt's signal with actual evaluation result
            if let Err(e) = self
                .agent_service
                .update_last_attempt_signal(mission_id, &goal.goal_id, &signal)
                .await
            {
                tracing::warn!(
                    "Failed to update attempt signal for goal {}: {}",
                    goal.goal_id,
                    e
                );
            }

            // 9. Handle signal
            match signal {
                ProgressSignal::Advancing => {
                    self.complete_goal(
                        mission_id,
                        &goal,
                        goal_step_index,
                        &goal_contract,
                        workspace_path,
                        workspace_before.as_ref(),
                    )
                    .await?;
                    if let Some(resolution) = self
                        .maybe_review_remaining_plan_after_goal_completion(
                            mission_id,
                            agent_id,
                            &goal,
                            workspace_path,
                        )
                        .await?
                    {
                        match resolution {
                            GoalLoopResolution::Continue => continue,
                            GoalLoopResolution::StopForSynthesis => return Ok(()),
                        }
                    }
                }
                ProgressSignal::Stalled => {
                    let attempt_count = goal.attempts.len() as u32 + 1;
                    if let Some(resolution) = self
                        .maybe_apply_goal_monitor_guidance(
                            mission_id,
                            agent_id,
                            session_id,
                            &goal,
                            workspace_path,
                            "Goal execution ended without strong completion evidence; decide whether to continue with a narrower next action, repair the remaining delivery, or end with a partial handoff.",
                            Some(&goal_contract),
                            attempt_count,
                        )
                        .await?
                    {
                        match resolution {
                            GoalLoopResolution::Continue => continue,
                            GoalLoopResolution::StopForSynthesis => return Ok(()),
                        }
                    }
                    // Check exploration budget
                    if attempt_count >= goal.exploration_budget {
                        if let Some(resolution) = self
                            .maybe_resolve_goal_gap(
                                mission_id,
                                agent_id,
                                &goal,
                                workspace_path,
                                "Goal stalled after bounded retries; decide whether to repair the remaining delivery, hand off partial work, or classify an environment/tooling blocker.",
                            )
                            .await?
                        {
                            match resolution {
                                GoalLoopResolution::Continue => continue,
                                GoalLoopResolution::StopForSynthesis => return Ok(()),
                            }
                        }
                        tracing::info!(
                            "Mission {} goal {} stalled after bounded retries without a monitor-directed replan; resetting to pending for the joint-drive loop",
                            mission_id,
                            goal.goal_id
                        );
                        if let Err(e) = self
                            .agent_service
                            .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::Pending)
                            .await
                        {
                            tracing::warn!(
                                "Failed to reset goal {} to pending after bounded stalled retries: {}",
                                goal.goal_id,
                                e
                            );
                        }
                    } else {
                        // Reset to Pending so find_next_goal picks it up again
                        if let Err(e) = self
                            .agent_service
                            .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::Pending)
                            .await
                        {
                            tracing::warn!(
                                "Failed to reset goal {} to pending: {}",
                                goal.goal_id,
                                e
                            );
                        }
                    }
                }
                ProgressSignal::Blocked => {
                    let attempt_count = goal.attempts.len() as u32 + 1;
                    if let Some(resolution) = self
                        .maybe_apply_goal_monitor_guidance(
                            mission_id,
                            agent_id,
                            session_id,
                            &goal,
                            workspace_path,
                            "Goal is blocked on the current path; decide whether to replan around the blocker, classify an environment or tooling constraint, or hand off the remaining gap.",
                            Some(&goal_contract),
                            attempt_count,
                        )
                        .await?
                    {
                        match resolution {
                            GoalLoopResolution::Continue => continue,
                            GoalLoopResolution::StopForSynthesis => return Ok(()),
                        }
                    }
                    if let Some(resolution) = self
                        .maybe_resolve_goal_gap(
                            mission_id,
                            agent_id,
                            &goal,
                            workspace_path,
                            "Goal is blocked on the current path; decide whether a bounded repair loop is still worthwhile or whether this should become a partial, environment, or tooling handoff.",
                        )
                        .await?
                    {
                        match resolution {
                            GoalLoopResolution::Continue => continue,
                            GoalLoopResolution::StopForSynthesis => return Ok(()),
                        }
                    }
                    tracing::info!(
                        "Mission {} goal {} remains blocked without a monitor-directed repair outcome; resetting to pending for the joint-drive loop",
                        mission_id,
                        goal.goal_id
                    );
                    if let Err(e) = self
                        .agent_service
                        .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::Pending)
                        .await
                    {
                        tracing::warn!(
                            "Failed to reset blocked goal {} to pending: {}",
                            goal.goal_id,
                            e
                        );
                    }
                }
            }
        }

        Ok(())
    }

    fn bounded_completion_repair_goals(goals: &[GoalNode]) -> Vec<GoalNode> {
        let salvage_ids = Self::allocate_salvage_goal_ids(goals, 3);
        goals.iter()
            .filter(|goal| {
                matches!(
                    goal.status,
                    GoalStatus::Pending | GoalStatus::Running | GoalStatus::Failed
                )
            })
            .take(3)
            .zip(salvage_ids.into_iter())
            .enumerate()
            .map(|(ordinal, (goal, salvage_id))| GoalNode {
                goal_id: salvage_id,
                parent_id: None,
                title: format!("Repair: {}", goal.title),
                description: format!(
                    "Reuse the current workspace and already collected evidence to finish the remaining core outcome from goal '{}'. Do not re-explore solved paths. Original description: {}",
                    goal.title, goal.description
                ),
                success_criteria: goal.success_criteria.clone(),
                status: GoalStatus::Pending,
                depth: 0,
                order: ordinal as u32,
                exploration_budget: goal.exploration_budget.min(2).max(1),
                attempts: Vec::new(),
                output_summary: None,
                runtime_contract: None,
                contract_verification: None,
                pivot_reason: Some("bounded_completion_repair".to_string()),
                is_checkpoint: goal.is_checkpoint,
                created_at: Some(bson::DateTime::now()),
                started_at: None,
                last_activity_at: None,
                last_progress_at: None,
                completed_at: None,
            })
            .collect()
    }

    fn count_existing_salvage_goals(goals: &[GoalNode]) -> u32 {
        goals
            .iter()
            .filter(|goal| goal.goal_id.starts_with("g-salvage-"))
            .count() as u32
    }

    fn goal_is_open(goal: &GoalNode) -> bool {
        matches!(
            goal.status,
            GoalStatus::Pending
                | GoalStatus::Running
                | GoalStatus::Failed
                | GoalStatus::Pivoting
                | GoalStatus::AwaitingApproval
        )
    }

    fn goal_is_salvage_like(goal: &GoalNode) -> bool {
        goal.goal_id.starts_with("g-salvage-")
            || goal.pivot_reason.as_deref() == Some("bounded_completion_repair")
            || goal.title.to_ascii_lowercase().contains("repair")
            || goal.description.to_ascii_lowercase().contains("bounded repair")
    }

    fn supersede_open_goals_in_tree(
        goals: &mut [GoalNode],
        preserved_goal_ids: &[String],
        reason: &str,
    ) -> usize {
        let now = bson::DateTime::now();
        let mut superseded = 0usize;

        for goal in goals.iter_mut() {
            if preserved_goal_ids
                .iter()
                .any(|preserved| preserved == &goal.goal_id)
                || !Self::goal_is_open(goal)
            {
                continue;
            }

            goal.status = GoalStatus::Abandoned;
            if goal
                .output_summary
                .as_deref()
                .map(str::trim)
                .is_none_or(|summary| summary.is_empty())
            {
                goal.output_summary =
                    Some(format!("Superseded by bounded adaptive repair: {reason}"));
            }
            if goal.pivot_reason.is_none() {
                goal.pivot_reason = Some("superseded_by_bounded_repair".to_string());
            }
            goal.completed_at = Some(now);
            goal.last_activity_at = Some(now);
            goal.last_progress_at = Some(now);
            superseded += 1;
        }

        superseded
    }

    fn goal_is_stale_pending_salvage(goal: &GoalNode) -> bool {
        Self::goal_is_salvage_like(goal)
            && matches!(
                goal.status,
                GoalStatus::Pending | GoalStatus::Running | GoalStatus::Failed
            )
            && goal.attempts.is_empty()
            && goal
                .output_summary
                .as_deref()
                .map(str::trim)
                .is_none_or(|summary| summary.is_empty())
    }

    /// Bridge to TaskExecutor (same pattern as MissionExecutor).
    #[allow(clippy::too_many_arguments)]
    async fn execute_via_bridge(
        &self,
        agent_id: &str,
        session_id: &str,
        mission_id: &str,
        user_message: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
        mission_context: Option<serde_json::Value>,
    ) -> Result<()> {
        runtime::execute_via_bridge(
            &self.db,
            &self.agent_service,
            &self.internal_task_manager,
            &self.mission_manager,
            mission_id,
            agent_id,
            session_id,
            user_message,
            cancel_token,
            workspace_path,
            Some(mission_id),
            None,
            mission_context,
            Some(self.mission_manager.clone()),
        )
        .await
    }

    async fn decide_next_goal_action(
        &self,
        mission: &MissionDoc,
        agent_id: &str,
        mission_id: &str,
        workspace_path: Option<&str>,
    ) -> Result<NextGoalDirective> {
        let goals = mission.goal_tree.as_deref().unwrap_or(&[]);
        let has_open_goals = goals.iter().any(|goal| {
            matches!(
                goal.status,
                GoalStatus::Pending
                    | GoalStatus::Pivoting
                    | GoalStatus::Running
                    | GoalStatus::Failed
                    | GoalStatus::AwaitingApproval
            )
        });
        if !has_open_goals {
            return Ok(NextGoalDirective::Break);
        }

        let candidates = Self::collect_executable_goals(goals);
        if candidates.is_empty() {
            return Ok(NextGoalDirective::Break);
        }
        if let Some(active_repair_lane_id) = mission.active_repair_lane_id.as_deref() {
            if let Some(active_repair_goal) = candidates
                .iter()
                .find(|goal| goal.goal_id == active_repair_lane_id)
            {
                return Ok(NextGoalDirective::Execute((**active_repair_goal).clone()));
            }
        }
        if candidates.len() == 1
            || !Self::strategy_requests_bounded_replan(mission.current_strategy.as_ref())
        {
            return Ok(NextGoalDirective::Execute(candidates[0].clone()));
        }

        let prompt = Self::build_remaining_plan_action_prompt(&mission.goal, goals);
        let decision = match self
            .execute_post_goal_plan_review_with_repair(
                mission,
                agent_id,
                mission_id,
                workspace_path,
                &prompt,
            )
            .await
        {
            Ok(selection) => selection,
            Err(err) => {
                if runtime::is_waiting_external_provider_message(&err.to_string()) {
                    tracing::debug!(
                        "Adaptive mission {} skipped next-action review because upstream provider is unavailable: {}",
                        mission_id,
                        err
                    );
                } else {
                    tracing::warn!(
                        "Adaptive mission {} produced invalid next-action review: {}",
                        mission_id,
                        err
                    );
                }
                return Ok(NextGoalDirective::Execute(candidates[0].clone()));
            }
        };

        match decision.decision {
            GoalPlanReviewDecision::ContinueCurrentPlan => {
                let selected_goal = decision
                    .selected_goal_id
                    .as_deref()
                    .and_then(|goal_id| {
                        candidates
                            .iter()
                            .find(|goal| goal.goal_id == goal_id)
                            .map(|goal| (*goal).clone())
                    })
                    .unwrap_or_else(|| (*candidates[0]).clone());

                tracing::info!(
                    "Adaptive mission {} selected next goal {} via orchestration review",
                    mission_id,
                    selected_goal.goal_id
                );

                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: serde_json::json!({
                                "type": "next_goal_selected",
                                "goal_id": selected_goal.goal_id,
                                "reason": decision.reason,
                                "observed_evidence": decision.observed_evidence,
                            })
                            .to_string(),
                        },
                    )
                    .await;

                Ok(NextGoalDirective::Execute(selected_goal))
            }
            GoalPlanReviewDecision::ContinueWithReplan => {
                let Some(plan) = decision.salvage_plan.clone() else {
                    tracing::warn!(
                        "Adaptive mission {} requested continue_with_replan without delta goals; falling back to first candidate",
                        mission_id
                    );
                    return Ok(NextGoalDirective::Execute(candidates[0].clone()));
                };

                let mut all_goals = mission.goal_tree.clone().unwrap_or_default();
                let new_goals = plan.goals.clone();
                let preserved_goal_ids = new_goals
                    .iter()
                    .map(|goal| goal.goal_id.clone())
                    .collect::<Vec<_>>();
                let supersede_reason = plan
                    .reason
                    .clone()
                    .or_else(|| decision.reason.clone())
                    .unwrap_or_else(|| {
                        "Remaining goals were superseded by an adaptive orchestration replan"
                            .to_string()
                    });
                all_goals.extend(new_goals.clone());
                let superseded = Self::supersede_open_goals_in_tree(
                    &mut all_goals,
                    &preserved_goal_ids,
                    &supersede_reason,
                );
                self.agent_service
                    .save_goal_tree(mission_id, all_goals)
                    .await
                    .map_err(|e| {
                        anyhow!(
                            "Failed to persist adaptive replan before goal selection: {}",
                            e
                        )
                    })?;
                let convergence_patch = MissionConvergencePatch {
                    active_repair_lane_id: Some(new_goals.first().map(|goal| goal.goal_id.clone())),
                    consecutive_no_tool_count: Some(0),
                    last_blocker_fingerprint: Some(runtime::blocker_fingerprint(&supersede_reason)),
                    waiting_external_until: Some(None),
                };
                if let Err(err) = self
                    .agent_service
                    .patch_mission_convergence_state(mission_id, &convergence_patch)
                    .await
                {
                    tracing::warn!(
                        "Failed to persist next-goal repair convergence state for mission {}: {}",
                        mission_id,
                        err
                    );
                }

                tracing::info!(
                    "Adaptive mission {} replaced remaining plan with {} delta goals via orchestration review",
                    mission_id,
                    plan.goals.len()
                );

                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: serde_json::json!({
                                "type": "goal_plan_replanned",
                                "goal_id": serde_json::Value::Null,
                                "new_goal_count": plan.goals.len(),
                                "superseded_goal_count": superseded,
                                "reason": plan.reason.or(decision.reason),
                                "observed_evidence": decision.observed_evidence,
                                "missing_core_deliverables": decision.missing_core_deliverables,
                            })
                            .to_string(),
                        },
                    )
                    .await;

                Ok(NextGoalDirective::Continue)
            }
            GoalPlanReviewDecision::CompleteIfEvidenceSufficient
            | GoalPlanReviewDecision::PartialHandoff
            | GoalPlanReviewDecision::BlockedByEnvironment
            | GoalPlanReviewDecision::BlockedByTooling
            | GoalPlanReviewDecision::BlockedFail => {
                let assessment = match decision.decision {
                    GoalPlanReviewDecision::CompleteIfEvidenceSufficient => {
                        MissionCompletionDecision::Complete.to_assessment(
                            decision.reason.clone(),
                            decision.observed_evidence.clone(),
                            decision.missing_core_deliverables.clone(),
                        )
                    }
                    GoalPlanReviewDecision::PartialHandoff => {
                        MissionCompletionDecision::PartialHandoff.to_assessment(
                            decision.reason.clone(),
                            decision.observed_evidence.clone(),
                            decision.missing_core_deliverables.clone(),
                        )
                    }
                    GoalPlanReviewDecision::BlockedByEnvironment => {
                        MissionCompletionDecision::BlockedByEnvironment.to_assessment(
                            decision.reason.clone(),
                            decision.observed_evidence.clone(),
                            decision.missing_core_deliverables.clone(),
                        )
                    }
                    GoalPlanReviewDecision::BlockedByTooling => {
                        MissionCompletionDecision::BlockedByTooling.to_assessment(
                            decision.reason.clone(),
                            decision.observed_evidence.clone(),
                            decision.missing_core_deliverables.clone(),
                        )
                    }
                    GoalPlanReviewDecision::BlockedFail => MissionCompletionDecision::BlockedFail
                        .to_assessment(
                            decision.reason.clone(),
                            decision.observed_evidence.clone(),
                            decision.missing_core_deliverables.clone(),
                        ),
                    GoalPlanReviewDecision::ContinueCurrentPlan
                    | GoalPlanReviewDecision::ContinueWithReplan => None,
                };
                if let Some(assessment) = assessment.as_ref() {
                    if let Err(err) = self
                        .agent_service
                        .set_mission_completion_assessment(mission_id, assessment)
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist next-action completion assessment for mission {}: {}",
                            mission_id,
                            err
                        );
                    }
                }

                let supersede_reason = decision.reason.clone().unwrap_or_else(|| {
                    "Remaining goals were closed by adaptive orchestration review".to_string()
                });
                let superseded = self
                    .supersede_open_goals(mission_id, None, &supersede_reason)
                    .await?;
                if let Err(err) = self
                    .agent_service
                    .clear_mission_current_goal(mission_id)
                    .await
                {
                    tracing::warn!(
                        "Failed to clear current goal before adaptive synthesis for mission {}: {}",
                        mission_id,
                        err
                    );
                }
                tracing::info!(
                    "Adaptive mission {} closed remaining plan via orchestration review with decision {:?}",
                    mission_id,
                    decision.decision
                );
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: serde_json::json!({
                                "type": "goal_plan_closed",
                                "goal_id": serde_json::Value::Null,
                                "decision": match decision.decision {
                                    GoalPlanReviewDecision::CompleteIfEvidenceSufficient => "complete_if_evidence_sufficient",
                                    GoalPlanReviewDecision::PartialHandoff => "partial_handoff",
                                    GoalPlanReviewDecision::BlockedByEnvironment => "blocked_by_environment",
                                    GoalPlanReviewDecision::BlockedByTooling => "blocked_by_tooling",
                                    GoalPlanReviewDecision::BlockedFail => "blocked_fail",
                                    _ => "continue_current_plan",
                                },
                                "superseded_goal_count": superseded,
                                "reason": decision.reason,
                                "observed_evidence": decision.observed_evidence,
                                "missing_core_deliverables": decision.missing_core_deliverables,
                            })
                            .to_string(),
                        },
                    )
                    .await;
                Ok(NextGoalDirective::StopForSynthesis)
            }
        }
    }

    /// Execute a single goal via bridge.
    #[allow(clippy::too_many_arguments)]
    async fn run_single_goal(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        goal: &GoalNode,
        completed_goals: &[&GoalNode],
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
        approval_policy: &str,
        current_step: usize,
        total_steps: usize,
        mission_step_timeout_seconds: Option<u64>,
        mission_step_max_retries: Option<u32>,
        operator_hint: Option<&str>,
    ) -> Result<runtime::MissionPreflightContract> {
        let heartbeat_token = CancellationToken::new();
        let _heartbeat_guard = HeartbeatGuard::new(heartbeat_token.clone());
        Self::spawn_goal_activity_heartbeat(
            self.agent_service.clone(),
            mission_id.to_string(),
            goal.goal_id.clone(),
            heartbeat_token,
        );
        let tokens_before = self.get_session_total_tokens(session_id).await;
        let messages_before = self
            .agent_service
            .get_session(session_id)
            .await
            .ok()
            .flatten()
            .map(|s| runtime::count_session_messages(&s.messages_json))
            .unwrap_or(0);

        // Mark as Running
        if let Err(e) = self
            .agent_service
            .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::Running)
            .await
        {
            tracing::warn!("Failed to set goal {} running: {}", goal.goal_id, e);
        }
        if let Err(e) = self
            .agent_service
            .advance_mission_goal(mission_id, &goal.goal_id)
            .await
        {
            tracing::warn!("Failed to advance mission goal to {}: {}", goal.goal_id, e);
        }

        // Broadcast GoalStart
        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::GoalStart {
                    goal_id: goal.goal_id.clone(),
                    title: goal.title.clone(),
                    depth: goal.depth,
                },
            )
            .await;

        let mission_snapshot = self.agent_service.get_mission(mission_id).await.ok().flatten();

        // Execute via bridge with mission context + retry/timeout protection
        let mc_json = serde_json::json!({
            "goal": goal.title,
            "approval_policy": approval_policy,
            "total_steps": total_steps,
            "current_step": current_step,
            "current_strategy": mission_snapshot.as_ref().and_then(|m| m.current_strategy.clone()),
            "latest_worker_state": mission_snapshot.as_ref().and_then(|m| m.latest_worker_state.clone()),
            "latest_stuck_phase_snapshot": mission_snapshot.as_ref().and_then(|m| m.latest_stuck_phase_snapshot.clone()),
        });

        let max_retries = Self::resolve_goal_max_retries(mission_step_max_retries);
        let goal_timeout = Self::resolve_goal_timeout(mission_step_timeout_seconds);
        let timeout_retry_limit = Self::goal_timeout_retry_limit().min(max_retries);
        let timeout_cancel_grace = Self::goal_timeout_cancel_grace();
        let mut timeout_retries_used: u32 = 0;
        let mut last_err: Option<anyhow::Error> = None;
        let mut reusable_contract = goal
            .runtime_contract
            .as_ref()
            .map(Self::runtime_contract_doc_to_preflight);
        let mut reusable_verify_state =
            Self::persisted_goal_verify_contract_state(goal.contract_verification.as_ref());
        let mut queued_goal_monitor_intervention: Option<String> = None;

        for attempt in 0..=max_retries {
            let attempt_blocker = last_err.as_ref().map(|err| err.to_string());
            let attempt_assets = reusable_contract
                .as_ref()
                .map(|contract| contract.required_artifacts.clone())
                .or_else(|| {
                    goal.runtime_contract
                        .as_ref()
                        .map(|contract| contract.required_artifacts.clone())
                })
                .unwrap_or_default();
            self.record_goal_worker_state(
                mission_id,
                goal,
                attempt + 1,
                attempt_assets,
                attempt_blocker.as_deref(),
                Some(if attempt == 0 {
                    "goal execution in progress"
                } else {
                    "goal retry in progress"
                }),
                Some(goal.success_criteria.as_str()),
                vec![format!("retry_attempt:{}", attempt + 1)],
                Vec::new(),
                None,
            )
            .await;
            let raw_prompt = if attempt == 0 {
                Self::build_goal_prompt(
                    goal,
                    completed_goals,
                    workspace_path,
                    operator_hint,
                    1,
                    None,
                )
            } else {
                let prev_err = last_err
                    .as_ref()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "unknown error".to_string());
                if Self::goal_retry_error_is_no_tool_execution(Some(&prev_err)) {
                    Self::build_goal_no_tool_recovery_prompt(
                        goal,
                        workspace_path,
                        attempt + 1,
                        &prev_err,
                        reusable_contract.is_some(),
                    )
                } else if reusable_contract.is_none()
                    && Self::goal_retry_error_is_missing_fresh_preflight(Some(&prev_err))
                {
                    Self::build_goal_preflight_repair_prompt(
                        goal,
                        workspace_path,
                        attempt + 1,
                        &prev_err,
                    )
                } else if Self::goal_retry_error_requires_completion_repair(Some(&prev_err))
                    || (reusable_contract.is_some()
                        && Self::goal_error_is_procedural_preflight_gap(&prev_err))
                {
                    Self::build_goal_completion_repair_prompt(
                        goal,
                        workspace_path,
                        attempt + 1,
                        &prev_err,
                    )
                } else {
                    let goal_prompt = Self::build_goal_prompt(
                        goal,
                        completed_goals,
                        workspace_path,
                        operator_hint,
                        attempt + 1,
                        Some(&prev_err),
                    );
                    let (recent_tool_calls, previous_output) =
                        match self.agent_service.get_session(session_id).await {
                            Ok(Some(sess)) => (
                                runtime::recent_tool_calls_for_retry(
                                    &sess.messages_json,
                                    RETRY_CONTEXT_TOOL_CALL_LIMIT,
                                ),
                                runtime::latest_assistant_output_for_retry(
                                    &sess.messages_json,
                                    RETRY_CONTEXT_OUTPUT_LIMIT,
                                ),
                            ),
                            Ok(None) => (Vec::new(), None),
                            Err(err) => {
                                tracing::debug!(
                                    "Failed to load session {} for goal retry context: {}",
                                    session_id,
                                    err
                                );
                                (Vec::new(), None)
                            }
                        };
                    let playbook = runtime::render_retry_playbook(&runtime::RetryPlaybookContext {
                        mode_label: "goal".to_string(),
                        unit_title: goal.title.clone(),
                        attempt_number: attempt + 1,
                        max_attempts: max_retries + 1,
                        failure_message: prev_err,
                        workspace_path: workspace_path.map(|s| s.to_string()),
                        previous_output,
                        recent_tool_calls,
                    });
                    format!("{}\n\n{}", goal_prompt, playbook)
                }
            };
            let persisted_monitor_intervention = consume_pending_monitor_intervention_instruction(
                &self.agent_service,
                &self.mission_manager,
                mission_id,
            )
            .await;
            let pending_monitor_intervention = match (
                queued_goal_monitor_intervention.take(),
                persisted_monitor_intervention,
            ) {
                (Some(local), Some(persisted)) => Some(format!("{}\n{}", persisted, local)),
                (Some(local), None) => Some(local),
                (None, Some(persisted)) => Some(persisted),
                (None, None) => None,
            };
            let prompt = Self::append_monitor_intervention_to_prompt(
                raw_prompt,
                pending_monitor_intervention.as_deref(),
            );

            if attempt > 0 {
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: format!(
                                r#"{{"type":"goal_retry","goal_id":"{}","attempt":{}}}"#,
                                goal.goal_id, attempt
                            ),
                        },
                    )
                    .await;

                // 2s, 4s, 8s, 16s, 16s...
                let delay = Duration::from_secs(2u64.saturating_pow(attempt.min(4)));
                tokio::time::sleep(delay).await;
            }

            let attempt_cancel = cancel_token.child_token();
            let exec_fut = self.execute_via_bridge(
                agent_id,
                session_id,
                mission_id,
                &prompt,
                attempt_cancel.clone(),
                workspace_path,
                Some(mc_json.clone()),
            );
            tokio::pin!(exec_fut);

            let attempt_result = match tokio::time::timeout(goal_timeout, &mut exec_fut).await {
                Ok(res) => res,
                Err(_) => {
                    attempt_cancel.cancel();
                    match tokio::time::timeout(timeout_cancel_grace, &mut exec_fut).await {
                        Ok(Ok(_)) => {
                            tracing::warn!(
                                "Mission {} goal {} exceeded {}s timeout but completed during {}s cancel grace",
                                mission_id,
                                goal.goal_id,
                                goal_timeout.as_secs(),
                                timeout_cancel_grace.as_secs()
                            );
                        }
                        Ok(Err(err)) => {
                            tracing::debug!(
                                "Mission {} goal {} stopped after timeout cancellation: {}",
                                mission_id,
                                goal.goal_id,
                                err
                            );
                        }
                        Err(_) => {
                            tracing::warn!(
                                "Mission {} goal {} did not stop within {}s cancel grace after timeout",
                                mission_id,
                                goal.goal_id,
                                timeout_cancel_grace.as_secs()
                            );
                        }
                    }

                    Err(anyhow!(
                        "Goal {} timed out after {}s",
                        goal.goal_id,
                        goal_timeout.as_secs()
                    ))
                }
            };

            match attempt_result {
                Ok(_) => {
                    let mut goal_tool_calls: Vec<ToolCallRecord> = Vec::new();
                    let mut preflight_contract: Option<runtime::MissionPreflightContract> = None;
                    let mut verify_contract_status: Option<bool> = None;
                    if let Ok(Some(sess)) = self.agent_service.get_session(session_id).await {
                        preflight_contract = runtime::extract_latest_preflight_contract_since(
                            &sess.messages_json,
                            messages_before,
                            MISSION_PREFLIGHT_TOOL_NAME,
                        );
                        verify_contract_status =
                            runtime::extract_latest_verify_contract_status_since(
                                &sess.messages_json,
                                messages_before,
                                MISSION_VERIFY_CONTRACT_TOOL_NAME,
                            );
                        goal_tool_calls = mission_verifier::from_tool_tuples(
                            runtime::extract_tool_calls_since(&sess.messages_json, messages_before),
                        );
                    }
                    if goal_tool_calls.is_empty() {
                        let attempted_methods = goal
                            .attempts
                            .iter()
                            .map(|attempt| attempt.approach.clone())
                            .chain(goal.pivot_reason.iter().cloned())
                            .collect::<Vec<_>>();
                        let no_tool_fingerprint =
                            runtime::blocker_fingerprint("Goal execution produced no tool calls");
                        let next_no_tool_count = match self.agent_service.get_mission(mission_id).await {
                            Ok(Some(mission_state))
                                if mission_state.last_blocker_fingerprint == no_tool_fingerprint =>
                            {
                                mission_state.consecutive_no_tool_count.saturating_add(1)
                            }
                            _ => 1,
                        };
                        let convergence_patch = MissionConvergencePatch {
                            active_repair_lane_id: Some(if Self::goal_is_salvage_like(goal) {
                                Some(goal.goal_id.clone())
                            } else {
                                None
                            }),
                            consecutive_no_tool_count: Some(next_no_tool_count),
                            last_blocker_fingerprint: Some(no_tool_fingerprint.clone()),
                            waiting_external_until: Some(None),
                        };
                        if let Err(err) = self
                            .agent_service
                            .patch_mission_convergence_state(mission_id, &convergence_patch)
                            .await
                        {
                            tracing::warn!(
                                "Failed to persist no-tool convergence state for mission {} goal {}: {}",
                                mission_id,
                                goal.goal_id,
                                err
                            );
                        }
                        let missing_core_deliverables = reusable_contract
                            .as_ref()
                            .map(|contract| contract.required_artifacts.clone())
                            .or_else(|| {
                                goal.runtime_contract
                                    .as_ref()
                                    .map(|contract| contract.required_artifacts.clone())
                            })
                            .unwrap_or_default();
                        self.record_goal_stuck_snapshot(
                            mission_id,
                            goal,
                            Vec::new(),
                            "Goal execution produced no tool calls",
                            attempted_methods,
                            Some("switch to a concrete tool-backed or repair-oriented next action"),
                            missing_core_deliverables,
                        )
                        .await;
                        self.mission_manager
                            .broadcast(
                                mission_id,
                                StreamEvent::Status {
                                    status: format!(
                                        r#"{{"type":"goal_no_tool_execution","goal_id":"{}","attempt":{},"reason":"no_tool_calls"}}"#,
                                        goal.goal_id,
                                        attempt + 1,
                                    ),
                                },
                            )
                            .await;

                        if attempt < max_retries {
                            tracing::warn!(
                                "Goal {} attempt {} produced no tool calls (will retry with direct-action recovery)",
                                goal.goal_id,
                                attempt + 1
                            );
                            if let Some(plan) = self
                                .build_goal_monitor_intervention(
                                    agent_id,
                                    mission_id,
                                    session_id,
                                    goal,
                                    workspace_path,
                                    "Goal execution produced no tool calls; recover with concrete next action",
                                    reusable_contract.as_ref(),
                                    attempt + 1,
                                )
                                .await
                            {
                                let action = normalize_monitor_action(&plan.intervention.action)
                                    .unwrap_or_else(|| "continue_current".to_string());
                                if next_no_tool_count >= 2
                                    && Self::is_goal_monitor_passive_continue_action(&action)
                                {
                                    if let Some(resolution) = self
                                        .maybe_resolve_goal_gap(
                                            mission_id,
                                            agent_id,
                                            goal,
                                            workspace_path,
                                            "repeated no-tool execution requires bounded repair replan",
                                        )
                                        .await?
                                    {
                                        match resolution {
                                            GoalLoopResolution::Continue => continue,
                                            GoalLoopResolution::StopForSynthesis => {
                                                return Ok(reusable_contract
                                                    .clone()
                                                    .or_else(|| {
                                                        goal.runtime_contract
                                                            .as_ref()
                                                            .map(Self::runtime_contract_doc_to_preflight)
                                                    })
                                                    .unwrap_or_default())
                                            }
                                        }
                                    }
                                }
                                match action.as_str() {
                                    "complete_if_evidence_sufficient" => {
                                        if let Some(resolution) = self
                                            .apply_goal_semantic_completion_intervention(
                                                mission_id,
                                                agent_id,
                                                goal,
                                                workspace_path,
                                                "Goal execution produced no tool calls; monitor concluded the existing evidence is already sufficient",
                                                &plan.intervention,
                                            )
                                            .await?
                                        {
                                            match resolution {
                                                GoalLoopResolution::Continue => continue,
                                                GoalLoopResolution::StopForSynthesis => {
                                                    return Ok(reusable_contract
                                                        .clone()
                                                        .or_else(|| {
                                                            goal.runtime_contract
                                                                .as_ref()
                                                                .map(Self::runtime_contract_doc_to_preflight)
                                                        })
                                                        .unwrap_or_default())
                                                }
                                            }
                                        }
                                    }
                                    "continue_with_replan"
                                    | "repair_deliverables"
                                    | "repair_contract"
                                    | "replan_remaining_goals"
                                    | "split_current_step"
                                    | "partial_handoff"
                                    | "blocked_by_environment"
                                    | "blocked_by_tooling"
                                    | "blocked_fail" => {
                                        self.record_goal_monitor_intervention_applied(
                                            mission_id,
                                            &goal.goal_id,
                                            &plan.intervention,
                                        )
                                        .await;
                                        let feedback = plan
                                            .intervention
                                            .feedback
                                            .as_deref()
                                            .map(str::trim)
                                            .filter(|text| !text.is_empty())
                                            .unwrap_or("monitor requested a repair-oriented recovery");
                                        return Err(anyhow!(
                                            "Goal execution produced no tool calls; monitor escalated with action {}. {}",
                                            action,
                                            feedback
                                        ));
                                    }
                                    _ => {
                                        queued_goal_monitor_intervention = self
                                            .persist_goal_monitor_intervention(
                                                mission_id,
                                                &goal.goal_id,
                                                &plan.intervention,
                                            )
                                            .await
                                            .or(plan.instruction);
                                    }
                                }
                            }
                            let soft_no_tool_error = anyhow!(
                                "Goal execution produced no tool calls; switch to a concrete tool-backed recovery path"
                            );
                            self.record_soft_goal_attempt(
                                mission_id,
                                goal,
                                &ProgressSignal::Blocked,
                                &soft_no_tool_error,
                            )
                            .await;
                            last_err = Some(soft_no_tool_error);
                            continue;
                        }
                        tracing::warn!(
                            "Goal {} produced no tool calls after {} attempts; escalating to adaptive repair flow",
                            goal.goal_id,
                            attempt + 1
                        );
                        return Err(anyhow!(
                            "Goal execution produced no tool calls after {} attempts; escalate through repair flow",
                            attempt + 1
                        ));
                    }
                    let allows_persisted_preflight_success =
                        Self::allows_persisted_goal_preflight_success(
                            reusable_contract.as_ref(),
                            goal,
                            last_err.as_ref().map(|e| e.to_string()).as_deref(),
                            operator_hint,
                        );
                    let effective_contract_candidate = Self::resolve_retry_goal_preflight_contract(
                        preflight_contract,
                        reusable_contract.as_ref(),
                        goal,
                        last_err.as_ref().map(|e| e.to_string()).as_deref(),
                        operator_hint,
                    );
                    let allows_existing_contract_flow = Self::allows_existing_goal_contract_flow(
                        effective_contract_candidate.as_ref(),
                        goal,
                        last_err.as_ref().map(|e| e.to_string()).as_deref(),
                        operator_hint,
                    );
                    let effective_contract = match mission_verifier::resolve_effective_contract(
                        effective_contract_candidate,
                        MISSION_PREFLIGHT_TOOL_NAME,
                        mission_verifier::VerifierLimits {
                            max_required_artifacts: MAX_GOAL_REQUIRED_ARTIFACTS,
                            max_completion_checks: MAX_GOAL_COMPLETION_CHECKS,
                            max_completion_check_cmd_len: MAX_GOAL_COMPLETION_CHECK_CMD_LEN,
                        },
                    ) {
                        Ok(contract) => contract,
                        Err(check_err) => {
                            self.mission_manager
                                    .broadcast(
                                        mission_id,
                                        StreamEvent::Status {
                                            status: format!(
                                                r#"{{"type":"goal_validation_failed","goal_id":"{}","attempt":{},"reason":"{}"}}"#,
                                                goal.goal_id,
                                                attempt + 1,
                                                check_err
                                                    .to_string()
                                                    .replace('"', r#"\""#)
                                                    .replace('\n', " ")
                                            ),
                                        },
                                    )
                                    .await;

                            if attempt < max_retries {
                                tracing::warn!(
                                        "Goal {} attempt {} failed preflight validation (will retry): {}",
                                        goal.goal_id,
                                        attempt + 1,
                                        check_err
                                    );
                                last_err = Some(anyhow!(
                                    "Goal preflight validation failed: {}",
                                    check_err
                                ));
                                continue;
                            }
                            tracing::warn!(
                                "Goal {} preflight validation still missing after retries; downgrading to best-effort contract: {}",
                                goal.goal_id,
                                check_err
                            );
                            reusable_contract
                                .clone()
                                .unwrap_or(runtime::MissionPreflightContract {
                                required_artifacts: Vec::new(),
                                completion_checks: Vec::new(),
                                no_artifact_reason: Some(
                                    "best-effort goal execution without strict preflight contract"
                                        .to_string(),
                                ),
                            })
                        }
                    };
                    if let Err(e) = self
                        .agent_service
                        .set_goal_runtime_contract(
                            mission_id,
                            &goal.goal_id,
                            &Self::to_runtime_contract_doc(&effective_contract),
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist runtime contract for mission {} goal {}: {}",
                            mission_id,
                            goal.goal_id,
                            e
                        );
                    }
                    reusable_contract = Some(effective_contract.clone());

                    // Extract summary and validate declared contract against workspace.
                    let summary = self.extract_step_summary(session_id).await;
                    if let Err(check_err) = mission_verifier::validate_contract_outputs(
                        &effective_contract,
                        workspace_path,
                        summary.as_deref(),
                        &goal_tool_calls,
                        0,
                        MISSION_PREFLIGHT_TOOL_NAME,
                        allows_persisted_preflight_success || allows_existing_contract_flow,
                        mission_verifier::CompletionCheckMode::AllowShell {
                            timeout: Self::goal_completion_check_timeout(),
                        },
                        false,
                    )
                    .await
                    {
                        self.mission_manager
                            .broadcast(
                                mission_id,
                                StreamEvent::Status {
                                    status: format!(
                                        r#"{{"type":"goal_validation_failed","goal_id":"{}","attempt":{},"reason":"{}"}}"#,
                                        goal.goal_id,
                                        attempt + 1,
                                        check_err
                                            .to_string()
                                            .replace('"', r#"\""#)
                                            .replace('\n', " ")
                                    ),
                                },
                            )
                            .await;

                        if attempt < max_retries {
                            tracing::warn!(
                                "Goal {} attempt {} failed completion validation (will retry): {}",
                                goal.goal_id,
                                attempt + 1,
                                check_err
                            );
                            last_err =
                                Some(anyhow!("Goal completion validation failed: {}", check_err));
                            continue;
                        }
                        tracing::warn!(
                            "Goal {} completion validation did not fully pass after retries; accepting best-effort result: {}",
                            goal.goal_id,
                            check_err
                        );
                    }

                    let gate_mode = runtime::contract_verify_gate_mode();
                    let fresh_verify_tool_called = mission_verifier::has_verify_contract_tool_call(
                        &goal_tool_calls,
                        MISSION_VERIFY_CONTRACT_TOOL_NAME,
                    );
                    let (verify_tool_called, verify_contract_status) =
                        Self::resolve_retry_goal_verify_contract_state(
                            fresh_verify_tool_called,
                            verify_contract_status,
                            reusable_verify_state,
                            goal,
                            last_err.as_ref().map(|e| e.to_string()).as_deref(),
                            operator_hint,
                        );
                    let verify_status_label = mission_verifier::verify_contract_status_label(
                        verify_tool_called,
                        verify_contract_status,
                    );
                    let gate_error = mission_verifier::enforce_verify_contract_gate(
                        gate_mode,
                        verify_tool_called,
                        verify_contract_status,
                        MISSION_VERIFY_CONTRACT_TOOL_NAME,
                    )
                    .err();
                    let gate_reason = gate_error
                        .as_ref()
                        .map(|e| e.to_string())
                        .unwrap_or_default();
                    if let Err(e) = self
                        .agent_service
                        .set_goal_contract_verification(
                            mission_id,
                            &goal.goal_id,
                            &RuntimeContractVerification {
                                tool_called: verify_tool_called,
                                status: Some(verify_status_label.to_string()),
                                gate_mode: Some(
                                    runtime::contract_verify_gate_mode_label(gate_mode).to_string(),
                                ),
                                accepted: Some(gate_error.is_none()),
                                reason: if gate_reason.trim().is_empty() {
                                    None
                                } else {
                                    Some(gate_reason.clone())
                                },
                                checked_at: Some(mongodb::bson::DateTime::now()),
                            },
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist contract verification for mission {} goal {}: {}",
                            mission_id,
                            goal.goal_id,
                            e
                        );
                    }
                    reusable_verify_state = if gate_error.is_none()
                        && (verify_tool_called || verify_contract_status.is_some())
                    {
                        Some((verify_tool_called, verify_contract_status))
                    } else {
                        reusable_verify_state
                    };
                    self.mission_manager
                        .broadcast(
                            mission_id,
                            StreamEvent::Status {
                                status: format!(
                                    r#"{{"type":"goal_contract_verification","goal_id":"{}","attempt":{},"gate":"{}","tool_called":{},"verify_status":"{}","accepted":{},"reason":"{}"}}"#,
                                    goal.goal_id,
                                    attempt + 1,
                                    runtime::contract_verify_gate_mode_label(gate_mode),
                                    verify_tool_called,
                                    verify_status_label,
                                    gate_error.is_none(),
                                    gate_reason.replace('"', r#"\""#).replace('\n', " ")
                                ),
                            },
                        )
                        .await;
                    if let Some(gate_err) = gate_error {
                        if attempt < max_retries {
                            tracing::warn!(
                                "Goal {} attempt {} failed contract verify gate (will retry): {}",
                                goal.goal_id,
                                attempt + 1,
                                gate_err
                            );
                            last_err = Some(anyhow!(
                                "Goal contract verification gate failed: {}",
                                gate_err
                            ));
                            continue;
                        }
                        tracing::warn!(
                            "Goal {} contract verification gate still failing after retries; accepting best-effort goal completion: {}",
                            goal.goal_id,
                            gate_err
                        );
                    }

                    let tokens_after = self.get_session_total_tokens(session_id).await;
                    let tokens_used = (tokens_after - tokens_before).max(0);

                    // Record attempt
                    let goal_attempt_record = AttemptRecord {
                        attempt_number: goal.attempts.len() as u32 + 1,
                        approach: goal
                            .pivot_reason
                            .clone()
                            .unwrap_or_else(|| "initial".to_string()),
                        signal: ProgressSignal::Advancing, // will be updated by evaluate
                        learnings: summary.clone().unwrap_or_default(),
                        tokens_used,
                        started_at: Some(bson::DateTime::now()),
                        completed_at: Some(bson::DateTime::now()),
                    };

                    if let Err(e) = self
                        .agent_service
                        .push_goal_attempt(mission_id, &goal.goal_id, &goal_attempt_record)
                        .await
                    {
                        tracing::warn!("Failed to push attempt for goal {}: {}", goal.goal_id, e);
                    }

                    if let Some(ref s) = summary {
                        if let Err(e) = self
                            .agent_service
                            .set_goal_output_summary(mission_id, &goal.goal_id, s)
                            .await
                        {
                            tracing::warn!(
                                "Failed to set output summary for goal {}: {}",
                                goal.goal_id,
                                e
                            );
                        }
                    }

                    if let Err(e) = self
                        .agent_service
                        .add_mission_tokens(mission_id, tokens_used)
                        .await
                    {
                        tracing::warn!(
                            "Failed to add mission {} tokens after goal {}: {}",
                            mission_id,
                            goal.goal_id,
                            e
                        );
                    }
                    self.record_goal_worker_state(
                        mission_id,
                        goal,
                        attempt + 1,
                        effective_contract.required_artifacts.clone(),
                        None,
                        Some("goal attempt produced a usable result"),
                        None,
                        goal_tool_calls
                            .iter()
                            .take(6)
                            .map(|call| {
                                format!(
                                    "tool:{}:{}",
                                    call.name,
                                    if call.success { "ok" } else { "failed" }
                                )
                            })
                            .collect(),
                        summary
                            .as_deref()
                            .map(|text| vec![Self::compact_goal_prompt_text(text, 220)])
                            .unwrap_or_default(),
                        None,
                    )
                    .await;
                    return Ok(effective_contract);
                }
                Err(e) => {
                    if cancel_token.is_cancelled() {
                        if let Ok(Some(current)) = self.agent_service.get_mission(mission_id).await
                        {
                            if matches!(
                                current.status,
                                MissionStatus::Paused | MissionStatus::Cancelled
                            ) {
                                if let Err(err) = self
                                    .agent_service
                                    .update_goal_status(
                                        mission_id,
                                        &goal.goal_id,
                                        &GoalStatus::Pending,
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        "Failed to reset goal {} to pending for mission {} after cancel: {}",
                                        goal.goal_id,
                                        mission_id,
                                        err
                                    );
                                }
                                return Ok(runtime::MissionPreflightContract {
                                    required_artifacts: Vec::new(),
                                    completion_checks: Vec::new(),
                                    no_artifact_reason: Some(
                                        "mission paused_or_cancelled".to_string(),
                                    ),
                                });
                            }
                        }
                    }

                    let is_timeout = Self::is_timeout_error(&e);
                    let is_retryable = runtime::is_retryable_error(&e);
                    let can_retry_timeout =
                        !is_timeout || timeout_retries_used < timeout_retry_limit;
                    if is_retryable && can_retry_timeout && attempt < max_retries {
                        if is_timeout {
                            timeout_retries_used = timeout_retries_used.saturating_add(1);
                        }
                        tracing::warn!(
                            "Goal {} attempt {} failed (retryable, timeout={}, timeout_retries={}/{}): {}",
                            goal.goal_id,
                            attempt + 1,
                            is_timeout,
                            timeout_retries_used,
                            timeout_retry_limit,
                            e
                        );
                        last_err = Some(e);
                        continue;
                    }

                    let tokens_after = self.get_session_total_tokens(session_id).await;
                    let tokens_used = (tokens_after - tokens_before).max(0);
                    if let Err(err) = self
                        .agent_service
                        .add_mission_tokens(mission_id, tokens_used)
                        .await
                    {
                        tracing::warn!(
                            "Failed to add mission {} tokens after failed goal {}: {}",
                            mission_id,
                            goal.goal_id,
                            err
                        );
                    }
                    let attempted_methods = goal
                        .attempts
                        .iter()
                        .map(|attempt| attempt.approach.clone())
                        .chain(goal.pivot_reason.iter().cloned())
                        .collect::<Vec<_>>();
                    self.record_goal_stuck_snapshot(
                        mission_id,
                        goal,
                        Vec::new(),
                        &e.to_string(),
                        attempted_methods,
                        Some("let monitor choose a repair, replan, waiting, or handoff mode"),
                        reusable_contract
                            .as_ref()
                            .map(|contract| contract.required_artifacts.clone())
                            .unwrap_or_default(),
                    )
                    .await;
                    return Err(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow!("Goal failed after retries")))
    }

    async fn get_session_total_tokens(&self, session_id: &str) -> i32 {
        self.agent_service
            .get_session(session_id)
            .await
            .ok()
            .flatten()
            .and_then(|s| s.total_tokens)
            .unwrap_or(0)
    }

    fn env_u64(name: &str) -> Option<u64> {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
    }

    fn env_u32(name: &str) -> Option<u32> {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v > 0)
    }

    fn planning_timeout() -> Duration {
        let secs = Self::env_u64("TEAM_MISSION_PLANNING_TIMEOUT_SECS")
            .unwrap_or(DEFAULT_MISSION_PLANNING_TIMEOUT_SECS)
            .min(MAX_MISSION_PLANNING_TIMEOUT_SECS);
        Duration::from_secs(secs)
    }

    fn planning_timeout_cancel_grace() -> Duration {
        let secs = Self::env_u64("TEAM_MISSION_PLANNING_CANCEL_GRACE_SECS")
            .unwrap_or(DEFAULT_PLANNING_TIMEOUT_CANCEL_GRACE_SECS)
            .min(MAX_PLANNING_TIMEOUT_CANCEL_GRACE_SECS);
        Duration::from_secs(secs)
    }

    fn goal_completion_check_timeout() -> Duration {
        let secs = Self::env_u64("TEAM_MISSION_GOAL_COMPLETION_CHECK_TIMEOUT_SECS")
            .unwrap_or(DEFAULT_GOAL_COMPLETION_CHECK_TIMEOUT_SECS)
            .min(MAX_GOAL_COMPLETION_CHECK_TIMEOUT_SECS);
        Duration::from_secs(secs.max(5))
    }

    fn clamp_goal_timeout_secs(timeout_secs: u64) -> u64 {
        timeout_secs.clamp(1, MAX_GOAL_EXECUTION_TIMEOUT_SECS)
    }

    fn resolve_min_goal_timeout_secs() -> u64 {
        Self::env_u64("TEAM_MISSION_MIN_GOAL_TIMEOUT_SECS")
            .unwrap_or(DEFAULT_MIN_GOAL_EXECUTION_TIMEOUT_SECS)
            .clamp(1, MAX_GOAL_EXECUTION_TIMEOUT_SECS)
    }

    fn resolve_goal_timeout(mission_step_timeout_seconds: Option<u64>) -> Duration {
        let configured_secs = mission_step_timeout_seconds
            .or_else(|| Self::env_u64("TEAM_MISSION_STEP_TIMEOUT_SECS"))
            .unwrap_or(DEFAULT_GOAL_EXECUTION_TIMEOUT_SECS);
        let clamped_secs = Self::clamp_goal_timeout_secs(configured_secs);
        let min_goal_secs = Self::resolve_min_goal_timeout_secs();
        Duration::from_secs(clamped_secs.max(min_goal_secs))
    }

    fn resolve_goal_max_retries(mission_step_max_retries: Option<u32>) -> u32 {
        mission_step_max_retries
            .or_else(|| Self::env_u32("TEAM_MISSION_DEFAULT_RETRIES"))
            .unwrap_or(2)
            .min(MAX_GOAL_RETRY_LIMIT)
    }

    fn goal_timeout_cancel_grace() -> Duration {
        let secs = Self::env_u64("TEAM_MISSION_TIMEOUT_CANCEL_GRACE_SECS")
            .unwrap_or(DEFAULT_GOAL_TIMEOUT_CANCEL_GRACE_SECS)
            .min(MAX_GOAL_TIMEOUT_CANCEL_GRACE_SECS);
        Duration::from_secs(secs)
    }

    fn goal_timeout_retry_limit() -> u32 {
        Self::env_u32("TEAM_MISSION_TIMEOUT_RETRY_LIMIT")
            .unwrap_or(DEFAULT_GOAL_TIMEOUT_RETRY_LIMIT)
            .min(MAX_GOAL_RETRY_LIMIT)
    }

    fn is_timeout_error(e: &anyhow::Error) -> bool {
        let msg = e.to_string().to_ascii_lowercase();
        msg.contains("timed out") || msg.contains("timeout")
    }

    /// Build prompt for executing a single goal.
    fn build_goal_prompt(
        goal: &GoalNode,
        completed_goals: &[&GoalNode],
        workspace_path: Option<&str>,
        operator_hint: Option<&str>,
        preflight_attempt: u32,
        preflight_last_error: Option<&str>,
    ) -> String {
        let mut prompt = format!(
            "## Goal: {}\n{}\n\n## Success Criteria\n{}\n",
            goal.title, goal.description, goal.success_criteria
        );

        if !completed_goals.is_empty() {
            prompt.push_str("\n## Completed Related Goals\n");
            for cg in completed_goals {
                let full = cg.output_summary.as_deref().unwrap_or("(no summary)");
                let summary = if full.chars().count() > 300 {
                    let truncated: String = full.chars().take(297).collect();
                    format!("{}...", truncated)
                } else {
                    full.to_string()
                };
                prompt.push_str(&format!(
                    "- Goal {}: {} → {}\n",
                    cg.goal_id, cg.title, summary
                ));
            }
        }

        if !goal.attempts.is_empty() {
            prompt.push_str("\n## Previous Attempts\n");
            for a in &goal.attempts {
                prompt.push_str(&format!(
                    "- Attempt {} ({}): {}\n",
                    a.attempt_number, a.approach, a.learnings
                ));
            }
        }

        if let Some(hint) = operator_hint.map(str::trim).filter(|h| !h.is_empty()) {
            prompt.push_str("\n## Operator Guidance (Highest Priority)\n");
            prompt.push_str(hint);
            prompt.push('\n');
        }

        prompt.push_str("\n## Mandatory Preflight Gate (Must Run First)\n");
        prompt.push_str(&format!(
            "- Before any other tool call, you MUST call `{}`.\n",
            MISSION_PREFLIGHT_TOOL_NAME
        ));
        prompt.push_str("- If preflight is skipped, this goal attempt will be retried.\n");
        prompt.push_str("- In preflight, you MUST declare a contract: `required_artifacts` and/or `completion_checks`; for non-file outcomes, provide `no_artifact_reason`.\n");
        let preflight_goal_title = Self::escape_json_for_prompt(&goal.title);
        let preflight_goal_desc = Self::escape_json_for_prompt(&goal.description);
        let preflight_workspace = Self::escape_json_for_prompt(workspace_path.unwrap_or_default());
        let preflight_last_error =
            Self::escape_json_for_prompt(preflight_last_error.unwrap_or_default());
        prompt.push_str("```json\n");
        prompt.push_str("{\n");
        prompt.push_str(&format!(
            "  \"step_title\": \"{}\",\n",
            preflight_goal_title
        ));
        prompt.push_str(&format!("  \"step_goal\": \"{}\",\n", preflight_goal_desc));
        prompt.push_str(&format!(
            "  \"workspace_path\": \"{}\",\n",
            preflight_workspace
        ));
        prompt.push_str("  \"required_artifacts\": [],\n");
        prompt.push_str("  \"completion_checks\": [],\n");
        prompt.push_str("  \"no_artifact_reason\": \"\",\n");
        prompt.push_str(&format!("  \"attempt\": {},\n", preflight_attempt.max(1)));
        prompt.push_str(&format!("  \"last_error\": \"{}\"\n", preflight_last_error));
        prompt.push_str("}\n");
        prompt.push_str("```\n");
        prompt.push_str("- Optional but recommended: call `mission_preflight__workspace_overview` to inspect current workspace before execution.\n");
        prompt.push_str("- Before final completion response, call `mission_preflight__verify_contract` with your final contract to self-verify outputs.\n");

        prompt.push_str("\nExecute this goal. Focus on meeting the success criteria.");
        prompt
    }

    fn build_goal_preflight_repair_prompt(
        goal: &GoalNode,
        workspace_path: Option<&str>,
        preflight_attempt: u32,
        last_error: &str,
    ) -> String {
        let title = Self::escape_json_for_prompt(&goal.title);
        let description = Self::escape_json_for_prompt(&goal.description);
        let success = Self::escape_json_for_prompt(&goal.success_criteria);
        let workspace = Self::escape_json_for_prompt(workspace_path.unwrap_or_default());
        let last_error = Self::escape_json_for_prompt(last_error);

        format!(
            r#"The previous goal attempt failed because it did not produce a valid mission preflight contract.

Your next response MUST be a tool call to `{tool}` before any prose, summary, or other tool call.
Do not explain. Do not summarize. Repair the preflight contract first.

## Goal
- title: {title}
- description: {description}
- success_criteria: {success}
- workspace_path: {workspace}

## Required repair
- Call `{tool}` immediately.
- Declare `required_artifacts` and/or `completion_checks`.
- If the goal intentionally has no file artifacts, set `required_artifacts: []` and provide `no_artifact_reason`.
- Use the actual deliverables for this goal instead of placeholders.

## Retry context
```json
{{
  "attempt": {attempt},
  "last_error": "{last_error}"
}}
```"#,
            tool = MISSION_PREFLIGHT_TOOL_NAME,
            title = title,
            description = description,
            success = success,
            workspace = workspace,
            attempt = preflight_attempt.max(1),
            last_error = last_error,
        )
    }

    fn build_goal_completion_repair_prompt(
        goal: &GoalNode,
        workspace_path: Option<&str>,
        preflight_attempt: u32,
        last_error: &str,
    ) -> String {
        let title = Self::escape_json_for_prompt(&goal.title);
        let description = Self::escape_json_for_prompt(&goal.description);
        let success = Self::escape_json_for_prompt(&goal.success_criteria);
        let workspace = Self::escape_json_for_prompt(workspace_path.unwrap_or_default());
        let last_error = Self::escape_json_for_prompt(last_error);

        format!(
            r#"The previous goal attempt produced useful work, but completion validation still failed.

Focus on reconciling only the missing validation gap. Do not restart the goal from scratch.
Your next response MUST be a tool call that directly repairs the missing output or verification gap.
Do not spend a turn restating the plan, re-summarizing the goal, or re-running the same unchanged contract.

## Goal
- title: {title}
- description: {description}
- success_criteria: {success}
- workspace_path: {workspace}

## Repair intent
- Reuse the existing workspace outputs whenever possible.
- Prefer the smallest repair that satisfies validation.
- If a declared artifact/check is missing, create only the missing evidence or deliverable.
- If the previous contract over-declared deliverables or the contract itself is inaccurate, call `{tool}` first with a corrected contract before continuing.
- If the existing contract is still correct, you may keep using it and only repair the missing output/evidence gap.
- Do not call `{tool}` again unless you are actually correcting the contract itself.
- Preserve existing successful outputs instead of regenerating the entire goal.

## Retry context
```json
{{
  "attempt": {attempt},
  "last_error": "{last_error}"
}}
```"#,
            tool = MISSION_PREFLIGHT_TOOL_NAME,
            title = title,
            description = description,
            success = success,
            workspace = workspace,
            attempt = preflight_attempt.max(1),
            last_error = last_error,
        )
    }

    fn build_goal_no_tool_recovery_prompt(
        goal: &GoalNode,
        workspace_path: Option<&str>,
        attempt: u32,
        last_error: &str,
        has_reusable_contract: bool,
    ) -> String {
        let title = Self::escape_json_for_prompt(&goal.title);
        let description = Self::escape_json_for_prompt(&goal.description);
        let success = Self::escape_json_for_prompt(&goal.success_criteria);
        let workspace = Self::escape_json_for_prompt(workspace_path.unwrap_or_default());
        let last_error = Self::escape_json_for_prompt(last_error);

        let next_action_guidance = if has_reusable_contract {
            format!(
                "- Reuse the current validated contract unless it is clearly wrong.\n- Your next response should immediately take a concrete tool-backed recovery path instead of prose-only reflection.\n- Prefer the smallest useful action or short sequence of actions that advances the current goal: create a missing deliverable, update a missing section, save intermediate evidence, run a missing verification, inspect the exact workspace input you need, or repair the contract.\n- If the contract itself is wrong, correct it by calling `{}` first; otherwise do not spend a turn restating the plan.",
                MISSION_PREFLIGHT_TOOL_NAME
            )
        } else {
            format!(
                "- Your next response should immediately take a concrete tool-backed recovery path instead of prose-only reflection.\n- Because this goal still lacks a usable contract, call `{}` first and declare the minimum real deliverables/checks needed for this goal.\n- After that, continue with the smallest concrete action or short sequence of actions that produces or verifies progress.",
                MISSION_PREFLIGHT_TOOL_NAME
            )
        };

        format!(
            r#"The previous goal attempt ended without any tool call, so the goal did not make verifiable progress.

## Goal
- title: {title}
- description: {description}
- success_criteria: {success}
- workspace_path: {workspace}

## Recovery requirement
{next_action_guidance}

## Retry context
```json
{{
  "attempt": {attempt},
  "last_error": "{last_error}"
}}
```"#,
            title = title,
            description = description,
            success = success,
            workspace = workspace,
            next_action_guidance = next_action_guidance,
            attempt = attempt.max(1),
            last_error = last_error,
        )
    }

    fn compact_goal_prompt_text(text: &str, max_chars: usize) -> String {
        let trimmed = text.trim();
        if trimmed.chars().count() <= max_chars {
            return trimmed.to_string();
        }
        let truncated: String = trimmed.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }

    fn compact_goal_prompt_list(
        items: &[String],
        max_items: usize,
        max_item_chars: usize,
    ) -> String {
        if items.is_empty() {
            return "none".to_string();
        }
        items
            .iter()
            .take(max_items)
            .map(|item| Self::compact_goal_prompt_text(item, max_item_chars))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn compact_goal_attempts_for_prompt(goal: &GoalNode) -> String {
        if goal.attempts.is_empty() {
            return "none".to_string();
        }
        goal.attempts
            .iter()
            .rev()
            .take(3)
            .rev()
            .map(|attempt| {
                format!(
                    "- attempt {} [{}]: {}",
                    attempt.attempt_number,
                    match attempt.signal {
                        ProgressSignal::Advancing => "advancing",
                        ProgressSignal::Stalled => "stalled",
                        ProgressSignal::Blocked => "blocked",
                    },
                    Self::compact_goal_prompt_text(&attempt.learnings, 180)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn compact_goal_contract_for_prompt(
        contract: Option<&runtime::MissionPreflightContract>,
    ) -> String {
        let Some(contract) = contract else {
            return "none".to_string();
        };
        let required_artifacts =
            Self::compact_goal_prompt_list(&contract.required_artifacts, 4, 96);
        let completion_checks = Self::compact_goal_prompt_list(&contract.completion_checks, 3, 120);
        let no_artifact_reason = contract
            .no_artifact_reason
            .as_deref()
            .map(|text| Self::compact_goal_prompt_text(text, 180))
            .unwrap_or_else(|| "none".to_string());
        format!(
            "required_artifacts: {}\ncompletion_checks: {}\nno_artifact_reason: {}",
            required_artifacts, completion_checks, no_artifact_reason
        )
    }

    fn compact_retry_tool_calls_for_prompt(
        recent_tool_calls: &[runtime::RetryPlaybookToolCall],
    ) -> String {
        if recent_tool_calls.is_empty() {
            return "- none".to_string();
        }
        recent_tool_calls
            .iter()
            .take(RETRY_CONTEXT_TOOL_CALL_LIMIT)
            .map(|call| {
                format!(
                    "- {} ({})",
                    call.name,
                    if call.success { "success" } else { "failed" }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn build_goal_supervisor_hint_prompt(
        mission_goal: &str,
        goal: &GoalNode,
        goal_evidence_snapshot: &str,
        workspace_path: Option<&str>,
        failure_message: &str,
        recent_tool_calls: &[runtime::RetryPlaybookToolCall],
        previous_output: Option<&str>,
        reusable_contract: Option<&runtime::MissionPreflightContract>,
        attempt: u32,
    ) -> String {
        let previous_output = previous_output
            .map(|text| Self::compact_goal_prompt_text(text, RETRY_CONTEXT_OUTPUT_LIMIT))
            .unwrap_or_else(|| "none".to_string());
        format!(
            "You are the monitor agent for a long-running adaptive mission goal.\n\
Return JSON only.\n\
- diagnosis: one concise sentence explaining the current blocker or drift.\n\
- status_assessment (optional): a low-commitment assessment such as busy, drifting, stalled, waiting_external, or evidence_sufficient.\n\
- recommended_action (optional): one of continue_current, repair_deliverables, repair_contract, continue_with_replan, extend_lease, resume_current_step, split_current_step, replan_remaining_goals, mark_waiting_external, complete_if_evidence_sufficient, partial_handoff, blocked_by_environment, blocked_by_tooling.\n\
- resume_hint: concrete next-step guidance that continues from existing work, narrows scope, and asks for immediate intermediate persistence when useful.\n\
- persist_hint (optional): 1-3 concise suggestions for intermediate outputs or evidence that should be saved next.\n\
- semantic_tags (optional): 1-4 broad, task-agnostic tags such as research, planning, implementation, verification, recovery, narrowing_scope, incremental_delivery, evidence_gap.\n\
- observed_evidence (optional): 1-3 brief observations grounded in the current evidence or progress signals.\n\
- missing_core_deliverables (optional): 1-6 concrete missing core deliverables that still block the end-user outcome.\n\
- confidence (optional): number between 0 and 1.\n\
- strategy_patch (optional): object with previous_strategy_summary, reason_for_change, new_goal_shape, preserved_user_intent, expected_gain, applied_at.\n\
- subagent_recommended (optional): boolean.\n\
- parallelism_budget (optional): integer 1-3 when subagent help is worthwhile.\n\
Do not assume a specific deliverable type unless it is explicitly supported by the goal or evidence.\n\
Keep the language evidence-driven and low-commitment.\n\
Prefer continuing from existing outputs instead of restarting.\n\
If the current evidence already appears sufficient, recommend complete_if_evidence_sufficient instead of inventing new work.\n\
If prior completed goals already establish that a required capability or environment path is unavailable, prefer blocked_by_environment, partial_handoff, or continue_with_replan over another vague continue_current.\n\
If the current goal can be satisfied by recording blocking evidence itself, prefer a bounded repair or handoff over repeated retries on the same unavailable path.\n\n\
Mission goal:\n{}\n\n\
Goal:\n- title: {}\n- description: {}\n- success_criteria: {}\n- workspace_path: {}\n\n\
Current goal/evidence snapshot:\n{}\n\n\
Retry context:\n- attempt: {}\n- last_error: {}\n\n\
Reusable contract:\n{}\n\n\
Recent attempts:\n{}\n\n\
Recent tool calls:\n{}\n\n\
Latest assistant output:\n{}\n",
            mission_goal,
            goal.title,
            goal.description,
            goal.success_criteria,
            workspace_path.unwrap_or("unknown"),
            goal_evidence_snapshot,
            attempt.max(1),
            Self::compact_goal_prompt_text(failure_message, 240),
            Self::compact_goal_contract_for_prompt(reusable_contract),
            Self::compact_goal_attempts_for_prompt(goal),
            Self::compact_retry_tool_calls_for_prompt(recent_tool_calls),
            previous_output
        )
    }

    fn build_goal_supervisor_guidance_repair_prompt(
        goal: &GoalNode,
        previous_response: &str,
    ) -> String {
        format!(
            "Your previous monitor reply for an adaptive mission goal was not valid JSON.\n\
Re-emit the guidance as valid JSON only.\n\
Keep the same meaning if possible, but fix the schema and make the action explicit.\n\
Use low-commitment, evidence-driven wording.\n\
\n\
Goal:\n- title: {}\n- description: {}\n- success_criteria: {}\n\n\
Return JSON with exactly these fields:\n\
{{\n\
  \"diagnosis\": \"one concise sentence\",\n\
  \"status_assessment\": \"busy|drifting|stalled|waiting_external|evidence_sufficient\" | null,\n\
  \"recommended_action\": \"continue_current|repair_deliverables|repair_contract|continue_with_replan|extend_lease|resume_current_step|split_current_step|replan_remaining_goals|mark_waiting_external|complete_if_evidence_sufficient|partial_handoff|blocked_by_environment|blocked_by_tooling\" | null,\n\
  \"resume_hint\": \"concrete next-step guidance\",\n\
  \"persist_hint\": [\"optional short item\"],\n\
  \"semantic_tags\": [\"optional_tag\"],\n\
  \"observed_evidence\": [\"optional observation\"],\n\
  \"missing_core_deliverables\": [\"optional deliverable\"],\n\
  \"confidence\": 0.5,\n\
  \"strategy_patch\": {{\n\
    \"previous_strategy_summary\": \"optional summary\",\n\
    \"reason_for_change\": \"optional reason\",\n\
    \"new_goal_shape\": \"optional goal shape\",\n\
    \"preserved_user_intent\": \"optional preserved intent\",\n\
    \"expected_gain\": \"optional gain\",\n\
    \"applied_at\": \"optional timestamp\"\n\
  }} | null,\n\
  \"subagent_recommended\": true | false | null,\n\
  \"parallelism_budget\": 1 | 2 | 3 | null\n\
}}\n\n\
Previous invalid response:\n{}",
            goal.title,
            goal.description,
            goal.success_criteria,
            Self::compact_goal_prompt_text(previous_response, 1200)
        )
    }

    fn build_salvage_no_tool_replan_prompt(
        mission_goal: &str,
        goal: &GoalNode,
        goal_evidence_snapshot: &str,
        workspace_path: Option<&str>,
        failure_message: &str,
        reusable_contract: Option<&runtime::MissionPreflightContract>,
        attempt: u32,
    ) -> String {
        let workspace = workspace_path.unwrap_or("(unknown)");
        let contract_summary = reusable_contract
            .map(|contract| {
                let required = if contract.required_artifacts.is_empty() {
                    "required_artifacts=0".to_string()
                } else {
                    format!(
                        "required_artifacts={} [{}]",
                        contract.required_artifacts.len(),
                        Self::compact_goal_prompt_list(&contract.required_artifacts, 3, 48)
                    )
                };
                let checks = if contract.completion_checks.is_empty() {
                    "completion_checks=0".to_string()
                } else {
                    format!(
                        "completion_checks={} [{}]",
                        contract.completion_checks.len(),
                        Self::compact_goal_prompt_list(&contract.completion_checks, 3, 48)
                    )
                };
                let no_artifact = contract
                    .no_artifact_reason
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(|text| format!("no_artifact_reason={text}"))
                    .unwrap_or_else(|| "no_artifact_reason=(none)".to_string());
                format!("{required}; {checks}; {no_artifact}")
            })
            .unwrap_or_else(|| "(none)".to_string());
        format!(
            "You are the monitor agent for an adaptive mission.\n\
The current goal is already a bounded repair/salvage goal, but repeated retries still ended without any tool call.\n\
Do not keep recommending a generic continue_current unless you can point to a genuinely new, concrete tool path that differs from the exhausted attempt pattern.\n\
\n\
Task:\n\
- Reassess whether the worker should change method now.\n\
- Prefer `continue_with_replan` when you should replace the current salvage goal with 1-2 tighter repair goals that reuse existing workspace outputs.\n\
- Prefer `blocked_by_environment` when the missing work depends on runtime capabilities that are not available.\n\
- Prefer `blocked_by_tooling` when the missing work is mainly blocked by failing tools or unstable source access.\n\
- Prefer `partial_handoff` only when directly reusable partial delivery already exists and another autonomous loop is not worth it.\n\
- Avoid restarting completed work.\n\
- Keep the response evidence-driven and low-commitment.\n\
\n\
Mission goal:\n{mission_goal}\n\
\n\
Current salvage goal:\n\
- title: {title}\n\
- description: {description}\n\
- success_criteria: {criteria}\n\
- workspace_path: {workspace}\n\
- attempt: {attempt}\n\
- last_error: {failure}\n\
\n\
Current goal/evidence snapshot:\n{snapshot}\n\
\n\
Reusable contract:\n{contract}\n\
\n\
Return JSON only:\n\
{{\n\
  \"diagnosis\": \"one concise sentence\",\n\
  \"status_assessment\": \"drifting|stalled|waiting_external|evidence_sufficient\" | null,\n\
  \"recommended_action\": \"continue_with_replan|repair_deliverables|repair_contract|blocked_by_environment|blocked_by_tooling|partial_handoff|continue_current|mark_waiting_external\" | null,\n\
  \"resume_hint\": \"concrete next-step guidance\",\n\
  \"persist_hint\": [\"optional short item\"],\n\
  \"semantic_tags\": [\"optional_tag\"],\n\
  \"observed_evidence\": [\"optional observation\"],\n\
  \"missing_core_deliverables\": [\"optional deliverable\"],\n\
  \"confidence\": 0.5,\n\
  \"strategy_patch\": {{\n\
    \"previous_strategy_summary\": \"optional summary\",\n\
    \"reason_for_change\": \"optional reason\",\n\
    \"new_goal_shape\": \"optional goal shape\",\n\
    \"preserved_user_intent\": \"optional preserved intent\",\n\
    \"expected_gain\": \"optional gain\",\n\
    \"applied_at\": \"optional timestamp\"\n\
  }} | null,\n\
  \"subagent_recommended\": true | false | null,\n\
  \"parallelism_budget\": 1 | 2 | 3 | null\n\
}}\n",
            mission_goal = mission_goal,
            title = goal.title,
            description = goal.description,
            criteria = goal.success_criteria,
            workspace = workspace,
            attempt = attempt + 1,
            failure = failure_message,
            snapshot = goal_evidence_snapshot,
            contract = contract_summary,
        )
    }

    fn build_post_goal_plan_review_repair_prompt(previous_response: &str) -> String {
        format!(
            "Your previous adaptive plan review reply was not valid JSON.\n\
Re-emit the decision as valid JSON only.\n\
Keep the same meaning if possible, but make the decision explicit and keep the reasoning low-commitment.\n\
\n\
Return JSON with exactly these fields:\n\
{{\n\
  \"decision\": \"continue_current_plan|continue_with_replan|complete_if_evidence_sufficient|partial_handoff|blocked_by_environment|blocked_by_tooling|blocked_fail\",\n\
  \"selected_goal_id\": \"optional remaining goal id when continuing the current plan\",\n\
  \"reason\": \"short explanation\",\n\
  \"observed_evidence\": [\"optional observation\"],\n\
  \"missing_core_deliverables\": [\"optional missing item\"],\n\
  \"delta_goals\": [\n\
    {{\n\
      \"title\": \"...\",\n\
      \"description\": \"...\",\n\
      \"success_criteria\": \"...\",\n\
      \"is_checkpoint\": false\n\
    }}\n\
  ]\n\
}}\n\
Use an empty `delta_goals` array unless the decision is `continue_with_replan`.\n\
Omit `selected_goal_id` unless a specific remaining goal should run next.\n\
\n\
Previous invalid response:\n{}",
            Self::compact_goal_prompt_text(previous_response, 1200)
        )
    }

    fn parse_goal_supervisor_guidance_response(
        assistant_text: &str,
    ) -> Option<GoalSupervisorGuidance> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum StringListOrString {
            List(Vec<String>),
            Single(String),
        }

        impl StringListOrString {
            fn into_vec(self) -> Vec<String> {
                match self {
                    Self::List(items) => items,
                    Self::Single(item) => vec![item],
                }
            }
        }

        #[derive(serde::Deserialize)]
        struct GuidancePayload {
            diagnosis: Option<String>,
            status_assessment: Option<String>,
            recommended_action: Option<String>,
            resume_hint: Option<String>,
            persist_hint: Option<StringListOrString>,
            semantic_tags: Option<StringListOrString>,
            observed_evidence: Option<StringListOrString>,
            missing_core_deliverables: Option<StringListOrString>,
            confidence: Option<f64>,
            strategy_patch: Option<MissionStrategyPatch>,
            subagent_recommended: Option<bool>,
            parallelism_budget: Option<u32>,
        }

        let json_str = runtime::extract_json_block(assistant_text);
        let normalized = runtime::normalize_loose_json(&json_str);
        let raw_normalized = runtime::normalize_loose_json(assistant_text);
        let payload = serde_json::from_str::<GuidancePayload>(assistant_text)
            .or_else(|_| serde_json::from_str::<GuidancePayload>(&json_str))
            .or_else(|_| serde_json::from_str::<GuidancePayload>(&normalized))
            .or_else(|_| serde_json::from_str::<GuidancePayload>(&raw_normalized))
            .ok()?;
        let diagnosis = payload.diagnosis?.trim().to_string();
        let resume_hint = payload.resume_hint?.trim().to_string();
        if diagnosis.is_empty() || resume_hint.is_empty() {
            return None;
        }

        let status_assessment = payload
            .status_assessment
            .map(|value| {
                value
                    .trim()
                    .to_ascii_lowercase()
                    .replace(char::is_whitespace, "_")
            })
            .filter(|value| !value.is_empty());
        let recommended_action = payload
            .recommended_action
            .and_then(|value| normalize_monitor_action(&value));
        let persist_hint = payload
            .persist_hint
            .map(StringListOrString::into_vec)
            .unwrap_or_default()
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .take(3)
            .collect::<Vec<_>>();
        let semantic_tags = payload
            .semantic_tags
            .map(StringListOrString::into_vec)
            .unwrap_or_default()
            .into_iter()
            .map(|tag| {
                tag.trim()
                    .to_ascii_lowercase()
                    .replace(char::is_whitespace, "_")
            })
            .filter(|tag| !tag.is_empty())
            .take(4)
            .collect::<Vec<_>>();
        let observed_evidence = payload
            .observed_evidence
            .map(StringListOrString::into_vec)
            .unwrap_or_default()
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .take(3)
            .collect::<Vec<_>>();
        let missing_core_deliverables = payload
            .missing_core_deliverables
            .map(StringListOrString::into_vec)
            .unwrap_or_default()
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .take(6)
            .collect::<Vec<_>>();
        let confidence = payload.confidence.map(|value| value.clamp(0.0, 1.0));
        let strategy_patch = payload.strategy_patch.map(|mut patch| {
            if patch.applied_at.is_none() {
                patch.applied_at = Some(bson::DateTime::now());
            }
            patch
        });
        let parallelism_budget = payload.parallelism_budget.map(|value| value.clamp(1, 3));

        Some(GoalSupervisorGuidance {
            diagnosis,
            resume_hint,
            status_assessment,
            recommended_action,
            semantic_tags,
            observed_evidence,
            persist_hint,
            missing_core_deliverables,
            confidence,
            strategy_patch,
            subagent_recommended: payload.subagent_recommended,
            parallelism_budget,
        })
    }

    async fn execute_goal_monitor_in_isolated_session(
        &self,
        mission: &MissionDoc,
        agent_id: &str,
        mission_id: &str,
        prompt: &str,
        workspace_path: Option<&str>,
    ) -> Result<String> {
        let temp_session = self
            .agent_service
            .create_chat_session(
                &mission.team_id,
                agent_id,
                &mission.creator_id,
                mission.attached_document_ids.clone(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                false,
                false,
                None,
                Some("system".to_string()),
                Some(mission_id.to_string()),
                Some(true),
            )
            .await
            .map_err(|e| anyhow!("Failed to create isolated goal monitor session: {}", e))?;
        let temp_session_id = temp_session.session_id.clone();
        let silent_broadcaster = Arc::new(AdaptiveSilentEventBroadcaster);

        let exec_result = runtime::execute_via_bridge(
            &self.db,
            &self.agent_service,
            &self.internal_task_manager,
            &silent_broadcaster,
            &temp_session_id,
            agent_id,
            &temp_session_id,
            prompt,
            CancellationToken::new(),
            workspace_path,
            None,
            None,
            None,
            None,
        )
        .await;

        let response = self
            .agent_service
            .get_session(&temp_session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .and_then(|session| runtime::extract_last_assistant_text(&session.messages_json))
            .unwrap_or_default();

        if let Err(err) = self
            .agent_service
            .delete_session_if_idle(&temp_session_id)
            .await
        {
            tracing::warn!(
                "Failed to delete isolated goal monitor session {} for mission {}: {}",
                temp_session_id,
                mission_id,
                err
            );
        }

        exec_result?;
        if response.trim().is_empty() {
            return Err(anyhow!(
                "Mission {} goal monitor returned empty assistant output",
                mission_id
            ));
        }
        Ok(response)
    }

    async fn execute_post_goal_plan_review_with_repair(
        &self,
        mission: &MissionDoc,
        agent_id: &str,
        mission_id: &str,
        workspace_path: Option<&str>,
        prompt: &str,
    ) -> Result<GoalPlanReviewResult> {
        let response = self
            .execute_goal_monitor_in_isolated_session(
                mission,
                agent_id,
                mission_id,
                prompt,
                workspace_path,
            )
            .await?;

        match Self::parse_post_goal_plan_review_response(
            mission.goal_tree.as_deref().unwrap_or(&[]),
            &response,
        ) {
            Ok(result) => Ok(result),
            Err(initial_err) => {
                let repair_prompt = Self::build_post_goal_plan_review_repair_prompt(&response);
                let repaired = self
                    .execute_goal_monitor_in_isolated_session(
                        mission,
                        agent_id,
                        mission_id,
                        &repair_prompt,
                        workspace_path,
                    )
                    .await
                    .map_err(|repair_err| {
                        anyhow!(
                            "Failed to repair adaptive plan review JSON after initial parse error ({}): {}",
                            initial_err,
                            repair_err
                        )
                    })?;
                Self::parse_post_goal_plan_review_response(
                    mission.goal_tree.as_deref().unwrap_or(&[]),
                    &repaired,
                )
                .map_err(|repair_parse_err| {
                    anyhow!(
                        "Failed to parse adaptive plan review JSON after repair attempt (initial: {}; repaired: {})",
                        initial_err,
                        repair_parse_err
                    )
                })
            }
        }
    }

    async fn build_goal_monitor_intervention(
        &self,
        agent_id: &str,
        mission_id: &str,
        session_id: &str,
        goal: &GoalNode,
        workspace_path: Option<&str>,
        failure_message: &str,
        reusable_contract: Option<&runtime::MissionPreflightContract>,
        attempt: u32,
    ) -> Option<GoalMonitorInterventionPlan> {
        let mission = match self.agent_service.get_mission(mission_id).await {
            Ok(Some(mission)) => mission,
            Ok(None) => return None,
            Err(err) => {
                tracing::debug!(
                    "Failed to load mission {} for adaptive goal monitor intervention: {}",
                    mission_id,
                    err
                );
                return None;
            }
        };
        match self.agent_service.get_mission(mission_id).await {
            Ok(Some(existing)) if existing.pending_monitor_intervention.is_some() => return None,
            Ok(_) => {}
            Err(err) => {
                tracing::debug!(
                    "Failed to inspect pending monitor intervention for mission {} goal {}: {}",
                    mission_id,
                    goal.goal_id,
                    err
                );
                return None;
            }
        }

        let (recent_tool_calls, previous_output) =
            match self.agent_service.get_session(session_id).await {
                Ok(Some(sess)) => (
                    runtime::recent_tool_calls_for_retry(
                        &sess.messages_json,
                        RETRY_CONTEXT_TOOL_CALL_LIMIT,
                    ),
                    runtime::latest_assistant_output_for_retry(
                        &sess.messages_json,
                        RETRY_CONTEXT_OUTPUT_LIMIT,
                    ),
                ),
                Ok(None) => (Vec::new(), None),
                Err(err) => {
                    tracing::debug!(
                        "Failed to load session {} for goal monitor intervention: {}",
                        session_id,
                        err
                    );
                    (Vec::new(), None)
                }
            };

        let goal_evidence_snapshot =
            Self::build_goal_evidence_digest(mission.goal_tree.as_deref().unwrap_or(&[]));
        let prompt = Self::build_goal_supervisor_hint_prompt(
            &mission.goal,
            goal,
            &goal_evidence_snapshot,
            workspace_path,
            failure_message,
            &recent_tool_calls,
            previous_output.as_deref(),
            reusable_contract,
            attempt,
        );
        let response = match self
            .execute_goal_monitor_in_isolated_session(
                &mission,
                agent_id,
                mission_id,
                &prompt,
                workspace_path,
            )
            .await
        {
            Ok(response) => response,
            Err(err) => {
                tracing::debug!(
                    "Failed to generate adaptive goal monitor guidance for mission {} goal {}: {}; falling back to generic monitor hint",
                    mission_id,
                    goal.goal_id,
                    err
                );
                String::new()
            }
        };
        let mut guidance = if let Some(guidance) =
            Self::parse_goal_supervisor_guidance_response(&response)
        {
            guidance
        } else {
            let repaired = if !response.trim().is_empty() {
                let repair_prompt =
                    Self::build_goal_supervisor_guidance_repair_prompt(goal, &response);
                match self
                    .execute_goal_monitor_in_isolated_session(
                        &mission,
                        agent_id,
                        mission_id,
                        &repair_prompt,
                        workspace_path,
                    )
                    .await
                {
                    Ok(repair_response) => {
                        Self::parse_goal_supervisor_guidance_response(&repair_response)
                    }
                    Err(err) => {
                        tracing::warn!(
                            "Adaptive goal monitor guidance repair failed for mission {} goal {}: {}",
                            mission_id,
                            goal.goal_id,
                            err
                        );
                        None
                    }
                }
            } else {
                None
            };

            if let Some(guidance) = repaired {
                guidance
            } else {
                tracing::warn!(
                    "Adaptive goal monitor guidance fell back to generic recovery for mission {} goal {}",
                    mission_id,
                    goal.goal_id
                );
                Self::build_generic_goal_supervisor_guidance(
                    &mission,
                    goal,
                    failure_message,
                    reusable_contract,
                    attempt,
                )
            }
        };

        if Self::should_replan_salvage_goal_after_no_tool(goal, failure_message, attempt)
            && Self::is_goal_monitor_passive_continue_action(
                guidance
                    .recommended_action
                    .as_deref()
                    .unwrap_or("continue_current"),
            )
        {
            let salvage_prompt = Self::build_salvage_no_tool_replan_prompt(
                &mission.goal,
                goal,
                &goal_evidence_snapshot,
                workspace_path,
                failure_message,
                reusable_contract,
                attempt,
            );
            let salvage_guidance = match self
                .execute_goal_monitor_in_isolated_session(
                    &mission,
                    agent_id,
                    mission_id,
                    &salvage_prompt,
                    workspace_path,
                )
                .await
            {
                Ok(salvage_response) => {
                    let parsed = Self::parse_goal_supervisor_guidance_response(&salvage_response);
                    if parsed.is_none() && !salvage_response.trim().is_empty() {
                        let repair_prompt = Self::build_goal_supervisor_guidance_repair_prompt(
                            goal,
                            &salvage_response,
                        );
                        match self
                            .execute_goal_monitor_in_isolated_session(
                                &mission,
                                agent_id,
                                mission_id,
                                &repair_prompt,
                                workspace_path,
                            )
                            .await
                        {
                            Ok(repair_response) => {
                                Self::parse_goal_supervisor_guidance_response(&repair_response)
                            }
                            Err(err) => {
                                tracing::warn!(
                                    "Adaptive salvage monitor repair failed for mission {} goal {}: {}",
                                    mission_id,
                                    goal.goal_id,
                                    err
                                );
                                None
                            }
                        }
                    } else {
                        parsed
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        "Adaptive salvage no-tool reassessment failed for mission {} goal {}: {}",
                        mission_id,
                        goal.goal_id,
                        err
                    );
                    None
                }
            };

            if let Some(replanned) = salvage_guidance.filter(|candidate| {
                !Self::is_goal_monitor_passive_continue_action(
                    candidate
                        .recommended_action
                        .as_deref()
                        .unwrap_or("continue_current"),
                )
            }) {
                guidance = replanned;
            } else {
                tracing::info!(
                    "Adaptive mission {} goal {} forced salvage replan fallback after repeated no-tool retries",
                    mission_id,
                    goal.goal_id
                );
                guidance.diagnosis = "Repeated retries on the current bounded repair goal still produced no executable action, so the method should change instead of replaying the same loop.".to_string();
                guidance.resume_hint = "Reuse the current workspace outputs, replace the current salvage step with 1-2 tighter repair actions, and avoid replaying the same no-tool attempt pattern.".to_string();
                guidance.status_assessment = Some("drifting".to_string());
                guidance.recommended_action = Some("continue_with_replan".to_string());
                if !guidance
                    .semantic_tags
                    .iter()
                    .any(|tag| tag == "repair_replan")
                {
                    guidance.semantic_tags.push("repair_replan".to_string());
                }
                if !guidance
                    .semantic_tags
                    .iter()
                    .any(|tag| tag == "salvage_loop")
                {
                    guidance.semantic_tags.push("salvage_loop".to_string());
                }
                if !guidance
                    .semantic_tags
                    .iter()
                    .any(|tag| tag == "no_tool_retry")
                {
                    guidance.semantic_tags.push("no_tool_retry".to_string());
                }
                if !guidance
                    .observed_evidence
                    .iter()
                    .any(|item| item.contains("repeated salvage retries ended without tool calls"))
                {
                    guidance
                        .observed_evidence
                        .push("repeated salvage retries ended without tool calls".to_string());
                }
                if guidance.persist_hint.is_empty() {
                    guidance.persist_hint.push(
                        "save the strongest existing evidence before re-planning the remaining repair work"
                            .to_string(),
                    );
                }
            }
        }

        tracing::info!(
            "Adaptive mission {} goal {} monitor guidance chose action {}",
            mission_id,
            goal.goal_id,
            guidance
                .recommended_action
                .as_deref()
                .unwrap_or("continue_current")
        );

        let mut feedback_lines = vec![
            format!("Diagnosis: {}", guidance.diagnosis.trim()),
            format!("Next: {}", guidance.resume_hint.trim()),
        ];
        if !guidance.persist_hint.is_empty() {
            feedback_lines.push(format!(
                "Persist next: {}",
                Self::compact_goal_prompt_list(&guidance.persist_hint, 3, 96)
            ));
        }
        if let Some(status) = guidance.status_assessment.as_deref() {
            feedback_lines.push(format!("Assessment: {}", status));
        }

        let intervention = MissionMonitorIntervention {
            action: guidance
                .recommended_action
                .clone()
                .unwrap_or_else(|| "continue_current".to_string()),
            feedback: Some(feedback_lines.join(" ")),
            semantic_tags: guidance.semantic_tags.clone(),
            observed_evidence: guidance.observed_evidence.clone(),
            missing_core_deliverables: guidance.missing_core_deliverables.clone(),
            confidence: guidance.confidence,
            strategy_patch: guidance.strategy_patch.clone(),
            subagent_recommended: guidance.subagent_recommended,
            parallelism_budget: guidance.parallelism_budget,
            requested_at: Some(mongodb::bson::DateTime::now()),
            applied_at: None,
        };
        let instruction = format_monitor_intervention_instruction(&intervention);
        Some(GoalMonitorInterventionPlan {
            intervention,
            instruction,
        })
    }

    async fn persist_goal_monitor_intervention(
        &self,
        mission_id: &str,
        goal_id: &str,
        intervention: &MissionMonitorIntervention,
    ) -> Option<String> {
        let normalized_action = normalize_monitor_action(&intervention.action)
            .unwrap_or_else(|| intervention.action.clone());
        let strategy = MissionStrategyState {
            action: Some(normalized_action.clone()),
            reason: intervention.feedback.clone(),
            missing_core_deliverables: intervention.missing_core_deliverables.clone(),
            confidence: intervention.confidence,
            strategy_patch: intervention.strategy_patch.clone(),
            subagent_recommended: intervention.subagent_recommended,
            parallelism_budget: intervention.parallelism_budget,
            updated_at: Some(bson::DateTime::now()),
        };
        if let Err(err) = self
            .agent_service
            .set_current_strategy(mission_id, Some(&strategy))
            .await
        {
            tracing::warn!(
                "Failed to persist adaptive current strategy for mission {} goal {}: {}",
                mission_id,
                goal_id,
                err
            );
        }
        if normalized_action == "mark_waiting_external" {
            let blocker = intervention
                .feedback
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .unwrap_or("Adaptive goal is waiting on an external dependency");
            self.patch_goal_waiting_external_convergence_state(mission_id, goal_id, blocker)
                .await;
        }
        if let Err(err) = self
            .agent_service
            .set_pending_monitor_intervention(mission_id, &intervention)
            .await
        {
            tracing::warn!(
                "Failed to persist adaptive goal monitor intervention for mission {} goal {}: {}",
                mission_id,
                goal_id,
                err
            );
            return format_monitor_intervention_instruction(intervention);
        }
        tracing::info!(
            "Queued adaptive goal monitor intervention for mission {} goal {} action {}",
            mission_id,
            goal_id,
            intervention.action
        );

        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::Status {
                    status: serde_json::json!({
                        "type": "goal_monitor_intervention_queued",
                        "goal_id": goal_id,
                        "action": intervention.action.clone(),
                        "semantic_tags": intervention.semantic_tags.clone(),
                        "observed_evidence": intervention.observed_evidence.clone(),
                    })
                    .to_string(),
                },
            )
            .await;
        format_monitor_intervention_instruction(intervention)
    }

    async fn record_goal_monitor_intervention_applied(
        &self,
        mission_id: &str,
        goal_id: &str,
        intervention: &MissionMonitorIntervention,
    ) {
        let strategy = MissionStrategyState {
            action: Some(
                normalize_monitor_action(&intervention.action)
                    .unwrap_or_else(|| intervention.action.clone()),
            ),
            reason: intervention.feedback.clone(),
            missing_core_deliverables: intervention.missing_core_deliverables.clone(),
            confidence: intervention.confidence,
            strategy_patch: intervention.strategy_patch.clone(),
            subagent_recommended: intervention.subagent_recommended,
            parallelism_budget: intervention.parallelism_budget,
            updated_at: Some(bson::DateTime::now()),
        };
        if let Err(err) = self
            .agent_service
            .set_current_strategy(mission_id, Some(&strategy))
            .await
        {
            tracing::warn!(
                "Failed to persist applied adaptive strategy for mission {} goal {}: {}",
                mission_id,
                goal_id,
                err
            );
        }
        if let Err(err) = self
            .agent_service
            .record_monitor_intervention_applied(mission_id, intervention)
            .await
        {
            tracing::warn!(
                "Failed to record applied adaptive goal monitor intervention for mission {} goal {}: {}",
                mission_id,
                goal_id,
                err
            );
            return;
        }
        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::Status {
                    status: serde_json::json!({
                        "type": "goal_monitor_intervention_applied",
                        "goal_id": goal_id,
                        "action": intervention.action.clone(),
                        "semantic_tags": intervention.semantic_tags.clone(),
                        "observed_evidence": intervention.observed_evidence.clone(),
                    })
                    .to_string(),
                },
            )
            .await;
    }

    async fn record_goal_worker_state(
        &self,
        mission_id: &str,
        goal: &GoalNode,
        attempt_number: u32,
        core_assets_now: Vec<String>,
        blocker: Option<&str>,
        method_summary: Option<&str>,
        next_step_candidate: Option<&str>,
        capability_signals: Vec<String>,
        subtask_results_summary: Vec<String>,
        parallelism_used: Option<u32>,
    ) {
        let worker_state = WorkerCompactState {
            current_goal: Some(format!("Goal {}: {}", goal.goal_id, goal.title)),
            core_assets_now: core_assets_now.clone(),
            assets_delta: core_assets_now.iter().take(4).cloned().collect(),
            current_blocker: blocker.map(|text| Self::compact_goal_prompt_text(text, 220)),
            method_summary: Some(
                method_summary
                    .map(|text| Self::compact_goal_prompt_text(text, 220))
                    .unwrap_or_else(|| format!("goal attempt {} in progress", attempt_number)),
            ),
            next_step_candidate: next_step_candidate
                .map(|text| Self::compact_goal_prompt_text(text, 220)),
            capability_signals: capability_signals
                .into_iter()
                .take(6)
                .map(|text| Self::compact_goal_prompt_text(&text, 120))
                .collect(),
            subtask_plan: Vec::new(),
            subtask_results_summary: subtask_results_summary
                .into_iter()
                .take(4)
                .map(|text| Self::compact_goal_prompt_text(&text, 220))
                .collect(),
            merge_risk: parallelism_used
                .filter(|count| *count > 1)
                .map(|count| format!("parallel merge pending across {} subtask result(s)", count)),
            parallelism_used,
            recorded_at: Some(bson::DateTime::now()),
        };
        if let Err(err) = self
            .agent_service
            .set_latest_worker_state(mission_id, Some(&worker_state))
            .await
        {
            tracing::warn!(
                "Failed to persist goal worker state for mission {} goal {}: {}",
                mission_id,
                goal.goal_id,
                err
            );
        }
        if let Err(err) = self
            .agent_service
            .set_latest_stuck_phase_snapshot(mission_id, None)
            .await
        {
            tracing::warn!(
                "Failed to clear stale adaptive stuck snapshot for mission {} goal {}: {}",
                mission_id,
                goal.goal_id,
                err
            );
        }
        let patch = MissionConvergencePatch {
            active_repair_lane_id: Some(if Self::goal_is_salvage_like(goal) {
                Some(goal.goal_id.clone())
            } else {
                None
            }),
            consecutive_no_tool_count: Some(0),
            last_blocker_fingerprint: Some(
                blocker
                    .and_then(runtime::blocker_fingerprint)
                    .map(|fingerprint| fingerprint.to_string()),
            ),
            waiting_external_until: Some(None),
        };
        if let Err(err) = self
            .agent_service
            .patch_mission_convergence_state(mission_id, &patch)
            .await
        {
            tracing::warn!(
                "Failed to patch adaptive convergence state for mission {} goal {}: {}",
                mission_id,
                goal.goal_id,
                err
            );
        }
    }

    async fn record_goal_stuck_snapshot(
        &self,
        mission_id: &str,
        goal: &GoalNode,
        completed_results: Vec<String>,
        blocker: &str,
        attempted_methods: Vec<String>,
        recommended_next_method: Option<&str>,
        missing_core_deliverables: Vec<String>,
    ) {
        let mut snapshot_results = completed_results;
        if snapshot_results.is_empty() {
            if let Some(summary) = goal.output_summary.as_deref() {
                snapshot_results.push(Self::compact_goal_prompt_text(summary, 220));
            }
        }
        let snapshot = MissionStuckPhaseSnapshot {
            current_goal: Some(format!("Goal {}: {}", goal.goal_id, goal.title)),
            completed_results: snapshot_results
                .into_iter()
                .take(8)
                .map(|text| Self::compact_goal_prompt_text(&text, 220))
                .collect(),
            missing_core_deliverables: if missing_core_deliverables.is_empty() {
                goal.runtime_contract
                    .as_ref()
                    .map(|contract| contract.required_artifacts.clone())
                    .unwrap_or_default()
            } else {
                missing_core_deliverables
            },
            current_blocker: Some(Self::compact_goal_prompt_text(blocker, 240)),
            attempted_methods: attempted_methods
                .into_iter()
                .take(6)
                .map(|text| Self::compact_goal_prompt_text(&text, 180))
                .collect(),
            recommended_next_method: recommended_next_method
                .map(|text| Self::compact_goal_prompt_text(text, 180)),
            recorded_at: Some(bson::DateTime::now()),
        };
        if let Err(err) = self
            .agent_service
            .set_latest_stuck_phase_snapshot(mission_id, Some(&snapshot))
            .await
        {
            tracing::warn!(
                "Failed to persist adaptive stuck snapshot for mission {} goal {}: {}",
                mission_id,
                goal.goal_id,
                err
            );
        }
        let patch = MissionConvergencePatch {
            active_repair_lane_id: Some(if Self::goal_is_salvage_like(goal) {
                Some(goal.goal_id.clone())
            } else {
                None
            }),
            consecutive_no_tool_count: None,
            last_blocker_fingerprint: Some(runtime::blocker_fingerprint(blocker)),
            waiting_external_until: None,
        };
        if let Err(err) = self
            .agent_service
            .patch_mission_convergence_state(mission_id, &patch)
            .await
        {
            tracing::warn!(
                "Failed to persist adaptive blocker fingerprint for mission {} goal {}: {}",
                mission_id,
                goal.goal_id,
                err
            );
        }
    }

    async fn maybe_apply_goal_monitor_guidance(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        goal: &GoalNode,
        workspace_path: Option<&str>,
        failure_message: &str,
        reusable_contract: Option<&runtime::MissionPreflightContract>,
        attempt: u32,
    ) -> Result<Option<GoalLoopResolution>> {
        let Some(plan) = self
            .build_goal_monitor_intervention(
                agent_id,
                mission_id,
                session_id,
                goal,
                workspace_path,
                failure_message,
                reusable_contract,
                attempt,
            )
            .await
        else {
            return Ok(None);
        };

        let action = normalize_monitor_action(&plan.intervention.action)
            .unwrap_or_else(|| "continue_current".to_string());

        match action.as_str() {
            "complete_if_evidence_sufficient" => {
                return self
                    .apply_goal_semantic_completion_intervention(
                        mission_id,
                        agent_id,
                        goal,
                        workspace_path,
                        failure_message,
                        &plan.intervention,
                    )
                    .await;
            }
            "continue_with_replan"
            | "repair_deliverables"
            | "repair_contract"
            | "replan_remaining_goals"
            | "split_current_step"
            | "partial_handoff"
            | "blocked_by_environment"
            | "blocked_by_tooling"
            | "blocked_fail" => {
                self.record_goal_monitor_intervention_applied(
                    mission_id,
                    &goal.goal_id,
                    &plan.intervention,
                )
                .await;
                let trigger_reason = plan
                    .intervention
                    .feedback
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(|feedback| format!("{} {}", failure_message, feedback))
                    .unwrap_or_else(|| failure_message.to_string());
                return self
                    .maybe_resolve_goal_gap(
                        mission_id,
                        agent_id,
                        goal,
                        workspace_path,
                        &trigger_reason,
                    )
                    .await;
            }
            "continue_current" | "resume_current_step" | "mark_waiting_external" => {
                self.persist_goal_monitor_intervention(
                    mission_id,
                    &goal.goal_id,
                    &plan.intervention,
                )
                .await;
                if let Err(err) = self
                    .agent_service
                    .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::Pending)
                    .await
                {
                    tracing::warn!(
                        "Failed to reset goal {} to pending after monitor guidance for mission {}: {}",
                        goal.goal_id,
                        mission_id,
                        err
                    );
                    return Ok(None);
                }
                if action == "mark_waiting_external" {
                    if let Err(err) = self
                        .agent_service
                        .clear_mission_current_goal(mission_id)
                        .await
                    {
                        tracing::warn!(
                            "Failed to clear current goal while parking mission {} goal {} in waiting_external: {}",
                            mission_id,
                            goal.goal_id,
                            err
                        );
                    }
                    self.mission_manager
                        .broadcast(
                            mission_id,
                            StreamEvent::Status {
                                status: serde_json::json!({
                                    "type": "goal_waiting_external",
                                    "goal_id": goal.goal_id,
                                    "feedback": plan.intervention.feedback,
                                })
                                .to_string(),
                            },
                        )
                        .await;
                }
                return Ok(Some(GoalLoopResolution::Continue));
            }
            _ => {
                self.persist_goal_monitor_intervention(
                    mission_id,
                    &goal.goal_id,
                    &plan.intervention,
                )
                .await;
                if let Some(instruction) = plan.instruction.as_deref() {
                    tracing::debug!(
                        "Adaptive goal {} monitor guidance kept as advisory only: {}",
                        goal.goal_id,
                        instruction
                    );
                }
            }
        }

        Ok(None)
    }

    async fn apply_goal_semantic_completion_intervention(
        &self,
        mission_id: &str,
        agent_id: &str,
        goal: &GoalNode,
        workspace_path: Option<&str>,
        failure_message: &str,
        intervention: &MissionMonitorIntervention,
    ) -> Result<Option<GoalLoopResolution>> {
        self.record_goal_monitor_intervention_applied(mission_id, &goal.goal_id, intervention)
            .await;
        let semantic_summary = intervention
            .feedback
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .unwrap_or(failure_message)
            .to_string();
        if let Err(err) = self
            .agent_service
            .set_goal_output_summary(mission_id, &goal.goal_id, &semantic_summary)
            .await
        {
            tracing::warn!(
                "Failed to persist semantic completion summary for mission {} goal {}: {}",
                mission_id,
                goal.goal_id,
                err
            );
        }
        if let Err(err) = self
            .agent_service
            .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::Completed)
            .await
        {
            tracing::warn!(
                "Failed to mark mission {} goal {} completed from monitor guidance: {}",
                mission_id,
                goal.goal_id,
                err
            );
            return Ok(None);
        }
        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::Status {
                    status: serde_json::json!({
                        "type": "goal_monitor_semantic_complete",
                        "goal_id": goal.goal_id,
                        "reason": semantic_summary,
                        "observed_evidence": intervention.observed_evidence,
                        "semantic_tags": intervention.semantic_tags,
                    })
                    .to_string(),
                },
            )
            .await;
        let updated_goal = self
            .agent_service
            .get_mission(mission_id)
            .await
            .ok()
            .flatten()
            .and_then(|mission| {
                mission
                    .goal_tree
                    .unwrap_or_default()
                    .into_iter()
                    .find(|candidate| candidate.goal_id == goal.goal_id)
            })
            .unwrap_or_else(|| {
                let mut completed_goal = goal.clone();
                completed_goal.status = GoalStatus::Completed;
                completed_goal.output_summary = Some(semantic_summary.clone());
                completed_goal
            });
        if let Some(resolution) = self
            .maybe_review_remaining_plan_after_goal_completion(
                mission_id,
                agent_id,
                &updated_goal,
                workspace_path,
            )
            .await?
        {
            return Ok(Some(resolution));
        }
        let remaining_open_goals = self
            .agent_service
            .get_mission(mission_id)
            .await
            .ok()
            .flatten()
            .map(|mission| {
                mission
                    .goal_tree
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|candidate| candidate.goal_id != goal.goal_id)
                    .filter(Self::goal_is_open)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let next_repair_lane_id = remaining_open_goals
            .iter()
            .find(|candidate| Self::goal_is_salvage_like(candidate))
            .map(|candidate| candidate.goal_id.clone());
        let convergence_patch = MissionConvergencePatch {
            active_repair_lane_id: Some(next_repair_lane_id),
            consecutive_no_tool_count: Some(0),
            last_blocker_fingerprint: Some(None),
            waiting_external_until: Some(None),
        };
        if let Err(err) = self
            .agent_service
            .patch_mission_convergence_state(mission_id, &convergence_patch)
            .await
        {
            tracing::warn!(
                "Failed to clear/update convergence state after semantic monitor completion for mission {} goal {}: {}",
                mission_id,
                goal.goal_id,
                err
            );
        }
        if let Err(err) = self
            .agent_service
            .clear_mission_current_goal(mission_id)
            .await
        {
            tracing::warn!(
                "Failed to clear current goal after semantic monitor completion for mission {} goal {}: {}",
                mission_id,
                goal.goal_id,
                err
            );
        }
        Ok(Some(if remaining_open_goals.is_empty() {
            GoalLoopResolution::StopForSynthesis
        } else {
            GoalLoopResolution::Continue
        }))
    }

    fn to_runtime_contract_doc(contract: &runtime::MissionPreflightContract) -> RuntimeContract {
        RuntimeContract {
            required_artifacts: contract.required_artifacts.clone(),
            completion_checks: contract.completion_checks.clone(),
            no_artifact_reason: contract.no_artifact_reason.clone(),
            source: Some(MISSION_PREFLIGHT_TOOL_NAME.to_string()),
            captured_at: Some(mongodb::bson::DateTime::now()),
        }
    }

    fn runtime_contract_doc_to_preflight(
        contract: &RuntimeContract,
    ) -> runtime::MissionPreflightContract {
        runtime::MissionPreflightContract {
            required_artifacts: contract.required_artifacts.clone(),
            completion_checks: contract.completion_checks.clone(),
            no_artifact_reason: contract.no_artifact_reason.clone(),
        }
    }

    fn goal_error_is_procedural_preflight_gap(error: &str) -> bool {
        let lower = error.to_ascii_lowercase();
        [
            "missing preflight contract payload",
            "mandatory preflight missing",
            "mandatory preflight order violation",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
    }

    fn goal_error_requires_contract_repair(error: &str) -> bool {
        let lower = error.to_ascii_lowercase();
        [
            "invalid preflight contract payload",
            "goal contract verification gate failed",
            "contract verification",
            "non-file output",
            "no_artifact_reason",
            "invalid required artifact path",
            "invalid completion check path",
            "unsupported completion check",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
    }

    fn goal_error_indicates_completion_gap(error: &str) -> bool {
        let lower = error.to_ascii_lowercase();
        [
            "goal completion validation failed",
            "required artifact not found",
            "completion check failed",
            "empty assistant output summary",
            "assistant reported file output",
            "no new workspace artifact was detected",
            "mandatory preflight order violation",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
    }

    fn should_force_fresh_goal_preflight(
        goal: &GoalNode,
        has_reusable_contract: bool,
        last_error: Option<&str>,
        operator_hint: Option<&str>,
    ) -> bool {
        if operator_hint
            .map(str::trim)
            .is_some_and(|hint| !hint.is_empty())
        {
            return true;
        }
        if let Some(error) = last_error {
            if Self::goal_error_is_procedural_preflight_gap(error) {
                if !has_reusable_contract {
                    return true;
                }
            } else if Self::goal_error_requires_contract_repair(error) {
                return true;
            }
        }
        if let Some(verification) = goal.contract_verification.as_ref() {
            let status_failed = verification.status.as_deref().is_some_and(|status| {
                matches!(
                    status.trim().to_ascii_lowercase().as_str(),
                    "fail" | "error"
                )
            });
            let reason_requires_contract_repair = verification
                .reason
                .as_deref()
                .is_some_and(Self::goal_error_requires_contract_repair);
            let reason_is_procedural_preflight_gap = verification
                .reason
                .as_deref()
                .is_some_and(Self::goal_error_is_procedural_preflight_gap);

            if reason_requires_contract_repair {
                return true;
            }
            if reason_is_procedural_preflight_gap && !has_reusable_contract {
                return true;
            }
            if (verification.accepted == Some(false) || status_failed)
                && !(has_reusable_contract && reason_is_procedural_preflight_gap)
            {
                return true;
            }
        }
        false
    }

    fn goal_retry_error_is_missing_fresh_preflight(last_error: Option<&str>) -> bool {
        let Some(last_error) = last_error else {
            return false;
        };
        let lower = last_error.to_ascii_lowercase();
        lower.contains("missing preflight contract payload")
            || lower.contains("mandatory preflight missing")
    }

    fn goal_retry_error_requires_completion_repair(last_error: Option<&str>) -> bool {
        let Some(last_error) = last_error else {
            return false;
        };
        let lower = last_error.to_ascii_lowercase();
        lower.contains("goal completion validation failed")
            || lower.contains("required artifact not found")
            || lower.contains("completion check failed")
            || lower.contains("empty assistant output summary")
            || lower.contains("assistant reported file output")
            || lower.contains("no new workspace artifact was detected")
    }

    fn goal_retry_error_is_no_tool_execution(last_error: Option<&str>) -> bool {
        let Some(last_error) = last_error else {
            return false;
        };
        let lower = last_error.to_ascii_lowercase();
        lower.contains("produced no tool calls")
            || lower.contains("produced no actionable tool execution")
            || lower.contains("ended without any tool call")
    }

    fn is_goal_monitor_passive_continue_action(action: &str) -> bool {
        matches!(
            normalize_monitor_action(action).as_deref(),
            Some("continue_current") | Some("resume_current_step")
        )
    }

    fn should_replan_salvage_goal_after_no_tool(
        goal: &GoalNode,
        failure_message: &str,
        attempt: u32,
    ) -> bool {
        attempt >= 2
            && Self::goal_retry_error_is_no_tool_execution(Some(failure_message))
            && (goal.goal_id.starts_with("g-salvage-")
                || goal.pivot_reason.as_deref() == Some("bounded_completion_repair"))
    }

    fn goal_retry_error_allows_persisted_contract_reuse(last_error: Option<&str>) -> bool {
        last_error.is_some_and(|error| {
            Self::goal_retry_error_is_missing_fresh_preflight(Some(error))
                || Self::goal_error_indicates_completion_gap(error)
        })
    }

    fn allows_persisted_goal_preflight_success(
        reusable_contract: Option<&runtime::MissionPreflightContract>,
        goal: &GoalNode,
        last_error: Option<&str>,
        operator_hint: Option<&str>,
    ) -> bool {
        reusable_contract.is_some()
            && !Self::should_force_fresh_goal_preflight(
                goal,
                reusable_contract.is_some(),
                last_error,
                operator_hint,
            )
    }

    fn allows_existing_goal_contract_flow(
        effective_contract: Option<&runtime::MissionPreflightContract>,
        goal: &GoalNode,
        last_error: Option<&str>,
        operator_hint: Option<&str>,
    ) -> bool {
        effective_contract.is_some()
            && Self::goal_retry_error_allows_persisted_contract_reuse(last_error)
            && !Self::should_force_fresh_goal_preflight(
                goal,
                effective_contract.is_some(),
                last_error,
                operator_hint,
            )
    }

    fn resolve_retry_goal_preflight_contract(
        fresh_contract: Option<runtime::MissionPreflightContract>,
        reusable_contract: Option<&runtime::MissionPreflightContract>,
        goal: &GoalNode,
        last_error: Option<&str>,
        operator_hint: Option<&str>,
    ) -> Option<runtime::MissionPreflightContract> {
        if fresh_contract.is_some() {
            return fresh_contract;
        }
        if reusable_contract.is_some()
            && Self::goal_retry_error_allows_persisted_contract_reuse(last_error)
        {
            return reusable_contract.cloned();
        }
        if Self::should_force_fresh_goal_preflight(
            goal,
            reusable_contract.is_some(),
            last_error,
            operator_hint,
        ) {
            return None;
        }
        reusable_contract.cloned()
    }

    fn parse_verify_status_label(status: Option<&str>) -> Option<bool> {
        match status.map(str::trim).map(|s| s.to_ascii_lowercase()) {
            Some(status) if status == "pass" || status == "ok" => Some(true),
            Some(status) if status == "fail" || status == "error" => Some(false),
            _ => None,
        }
    }

    fn persisted_goal_verify_contract_state(
        verification: Option<&RuntimeContractVerification>,
    ) -> Option<(bool, Option<bool>)> {
        let verification = verification?;
        if verification.accepted != Some(true) {
            return None;
        }
        let status = Self::parse_verify_status_label(verification.status.as_deref());
        if !verification.tool_called && status.is_none() {
            return None;
        }
        Some((verification.tool_called || status.is_some(), status))
    }

    fn resolve_retry_goal_verify_contract_state(
        fresh_tool_called: bool,
        fresh_status: Option<bool>,
        reusable_state: Option<(bool, Option<bool>)>,
        goal: &GoalNode,
        last_error: Option<&str>,
        operator_hint: Option<&str>,
    ) -> (bool, Option<bool>) {
        if fresh_tool_called || fresh_status.is_some() {
            return (fresh_tool_called || fresh_status.is_some(), fresh_status);
        }
        if reusable_state.is_some()
            && Self::goal_retry_error_allows_persisted_contract_reuse(last_error)
        {
            return reusable_state.unwrap_or((false, None));
        }
        if Self::should_force_fresh_goal_preflight(
            goal,
            reusable_state.is_some(),
            last_error,
            operator_hint,
        ) {
            return (false, None);
        }
        reusable_state.unwrap_or((false, None))
    }

    fn escape_json_for_prompt(input: &str) -> String {
        input
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "")
    }

    fn append_monitor_intervention_to_prompt(
        prompt: String,
        monitor_intervention: Option<&str>,
    ) -> String {
        let Some(monitor_intervention) = monitor_intervention
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return prompt;
        };
        format!(
            "{}\n\n## Pending Monitor Intervention\n{}\n",
            prompt, monitor_intervention
        )
    }

    /// Extract the full output text from the last assistant message.
    /// Saved as-is for debugging; truncated only when injected into prompts.
    async fn extract_step_summary(&self, session_id: &str) -> Option<String> {
        let session = match self.agent_service.get_session(session_id).await {
            Ok(s) => s?,
            Err(e) => {
                tracing::warn!("Failed to get session {} for summary: {}", session_id, e);
                return None;
            }
        };
        runtime::extract_last_assistant_text(&session.messages_json).filter(|t| !t.is_empty())
    }

    /// Evaluate whether a goal has been achieved.
    async fn evaluate_goal(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        goal: &GoalNode,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<ProgressSignal> {
        let prompt = format!(
            r#"Evaluate whether the goal you just executed has been achieved.

## Goal: {}
## Success Criteria: {}

Assess:
- advancing: Success criteria met or substantial progress made
- stalled: Some progress but criteria not met, may need a different approach
- blocked: Encountered insurmountable obstacle, current direction is not viable

Output JSON only: {{"signal": "advancing|stalled|blocked", "reasoning": "...", "learnings": "..."}}"#,
            goal.title, goal.success_criteria
        );

        self.execute_via_bridge(
            agent_id,
            session_id,
            mission_id,
            &prompt,
            cancel_token,
            workspace_path,
            None,
        )
        .await?;

        // Parse response
        let session = self
            .agent_service
            .get_session(session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Session not found"))?;

        let text = runtime::extract_last_assistant_text(&session.messages_json).unwrap_or_default();
        let json_str = runtime::extract_json_block(&text);

        // Parse signal
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
            let signal_str = val
                .get("signal")
                .and_then(|s| s.as_str())
                .unwrap_or("stalled");
            match signal_str {
                "advancing" => Ok(ProgressSignal::Advancing),
                "blocked" => Ok(ProgressSignal::Blocked),
                _ => Ok(ProgressSignal::Stalled),
            }
        } else {
            // Default to stalled if parse fails — safer than assuming success
            Ok(ProgressSignal::Stalled)
        }
    }

    /// Mark a goal as completed.
    async fn complete_goal(
        &self,
        mission_id: &str,
        goal: &GoalNode,
        step_index: u32,
        contract: &runtime::MissionPreflightContract,
        workspace_path: Option<&str>,
        before: Option<&runtime::WorkspaceSnapshot>,
    ) -> Result<()> {
        if let Err(e) = self
            .agent_service
            .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::Completed)
            .await
        {
            tracing::warn!("Failed to complete goal {}: {}", goal.goal_id, e);
        }

        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::GoalComplete {
                    goal_id: goal.goal_id.clone(),
                    signal: "advancing".to_string(),
                },
            )
            .await;

        if let Some(wp) = workspace_path {
            if let Err(e) = self
                .register_goal_artifacts(
                    mission_id,
                    goal,
                    step_index,
                    &contract.required_artifacts,
                    wp,
                    before,
                )
                .await
            {
                tracing::warn!(
                    "Artifact scan failed for mission {} goal {}: {}",
                    mission_id,
                    goal.goal_id,
                    e
                );
            }
        }

        self.record_goal_worker_state(
            mission_id,
            goal,
            goal.attempts.len().max(1) as u32,
            contract.required_artifacts.clone(),
            None,
            Some("goal completed with usable result"),
            None,
            Vec::new(),
            goal.output_summary
                .as_deref()
                .map(|text| vec![Self::compact_goal_prompt_text(text, 220)])
                .unwrap_or_default(),
            None,
        )
        .await;

        Ok(())
    }

    async fn register_goal_artifacts(
        &self,
        mission_id: &str,
        _goal: &GoalNode,
        step_index: u32,
        required_artifacts: &[String],
        workspace_path: &str,
        before: Option<&runtime::WorkspaceSnapshot>,
    ) -> Result<()> {
        runtime::save_scanned_artifacts(
            &self.agent_service,
            mission_id,
            step_index,
            workspace_path,
            before,
            Some(required_artifacts),
        )
        .await?;
        runtime::save_required_artifacts(
            &self.agent_service,
            mission_id,
            step_index,
            workspace_path,
            required_artifacts,
        )
        .await
    }

    fn soft_goal_terminal_signal(error: &anyhow::Error) -> Option<ProgressSignal> {
        let message = error.to_string();
        if Self::goal_retry_error_is_no_tool_execution(Some(&message))
            || Self::goal_error_is_procedural_preflight_gap(&message)
            || message
                .to_ascii_lowercase()
                .contains("goal preflight validation failed")
            || message
                .to_ascii_lowercase()
                .contains("goal contract verification gate failed")
        {
            return Some(ProgressSignal::Blocked);
        }
        if Self::goal_error_indicates_completion_gap(&message) {
            return Some(ProgressSignal::Stalled);
        }
        None
    }

    fn goal_error_is_provider_capacity_block(message: &str) -> bool {
        runtime::is_waiting_external_provider_message(message)
    }

    fn waiting_external_until_after_cooldown() -> bson::DateTime {
        bson::DateTime::from_millis(
            bson::DateTime::now().timestamp_millis() + WAITING_EXTERNAL_COOLDOWN_SECS * 1000,
        )
    }

    fn waiting_external_remaining_delay(
        waiting_external_until: bson::DateTime,
    ) -> Option<Duration> {
        let remaining_ms = waiting_external_until.timestamp_millis() - bson::DateTime::now().timestamp_millis();
        if remaining_ms <= 0 {
            None
        } else {
            Some(Duration::from_millis(remaining_ms as u64))
        }
    }

    fn provider_capacity_retry_delay(error: &anyhow::Error) -> Duration {
        let _ = error;
        Duration::from_secs(WAITING_EXTERNAL_COOLDOWN_SECS as u64)
    }

    async fn record_soft_goal_attempt(
        &self,
        mission_id: &str,
        goal: &GoalNode,
        signal: &ProgressSignal,
        error: &anyhow::Error,
    ) {
        let attempt = AttemptRecord {
            attempt_number: goal.attempts.len() as u32 + 1,
            approach: goal
                .pivot_reason
                .clone()
                .unwrap_or_else(|| "soft_recovery".to_string()),
            signal: signal.clone(),
            learnings: error.to_string(),
            tokens_used: 0,
            started_at: Some(bson::DateTime::now()),
            completed_at: Some(bson::DateTime::now()),
        };

        if let Err(e) = self
            .agent_service
            .push_goal_attempt(mission_id, &goal.goal_id, &attempt)
            .await
        {
            tracing::warn!(
                "Failed to push soft recovery attempt for goal {}: {}",
                goal.goal_id,
                e
            );
        }

        if let Err(e) = self
            .agent_service
            .update_last_attempt_signal(mission_id, &goal.goal_id, signal)
            .await
        {
            tracing::warn!(
                "Failed to update soft recovery signal for goal {}: {}",
                goal.goal_id,
                e
            );
        }
    }

    async fn defer_goal_for_provider_capacity(
        &self,
        mission_id: &str,
        goal: &GoalNode,
        error: &anyhow::Error,
        cancel_token: &CancellationToken,
    ) -> Result<Option<GoalLoopResolution>> {
        let feedback =
            "Upstream model/provider capacity is temporarily unavailable; preserve current outputs and retry after cooldown."
                .to_string();
        let observed = Self::compact_goal_prompt_text(&error.to_string(), 240);
        let blocker_fingerprint = runtime::blocker_fingerprint(&error.to_string());
        let mut delay = Self::provider_capacity_retry_delay(error);
        let mut should_enqueue_intervention = true;

        if let Ok(Some(mission_state)) = self.agent_service.get_mission(mission_id).await {
            if mission_state.last_blocker_fingerprint == blocker_fingerprint {
                if let Some(waiting_until) = mission_state.waiting_external_until {
                    if let Some(remaining) =
                        Self::waiting_external_remaining_delay(waiting_until)
                    {
                        delay = remaining;
                        should_enqueue_intervention = false;
                    }
                }
            }
        }

        let waiting_external_until = Self::waiting_external_until_after_cooldown();
        let convergence_patch = MissionConvergencePatch {
            active_repair_lane_id: Some(if Self::goal_is_salvage_like(goal) {
                Some(goal.goal_id.clone())
            } else {
                None
            }),
            consecutive_no_tool_count: Some(0),
            last_blocker_fingerprint: Some(blocker_fingerprint.clone()),
            waiting_external_until: Some(Some(waiting_external_until)),
        };
        if let Err(err) = self
            .agent_service
            .patch_mission_convergence_state(mission_id, &convergence_patch)
            .await
        {
            tracing::warn!(
                "Failed to persist provider wait convergence state for mission {} goal {}: {}",
                mission_id,
                goal.goal_id,
                err
            );
        }

        if should_enqueue_intervention {
            let intervention = MissionMonitorIntervention {
                action: "mark_waiting_external".to_string(),
                feedback: Some(feedback.clone()),
                semantic_tags: vec![
                    "waiting_external".to_string(),
                    "provider_capacity".to_string(),
                    "retry_later".to_string(),
                ],
                observed_evidence: vec![observed.clone()],
                missing_core_deliverables: Vec::new(),
                confidence: None,
                strategy_patch: None,
                subagent_recommended: None,
                parallelism_budget: None,
                requested_at: Some(bson::DateTime::now()),
                applied_at: None,
            };
            self.record_goal_stuck_snapshot(
                mission_id,
                goal,
                goal.output_summary
                    .as_deref()
                    .map(|text| vec![Self::compact_goal_prompt_text(text, 220)])
                    .unwrap_or_default(),
                &feedback,
                goal.attempts
                    .iter()
                    .map(|attempt| attempt.approach.clone())
                    .collect(),
                Some("wait for provider capacity and resume with the preserved strategy"),
                goal.runtime_contract
                    .as_ref()
                    .map(|contract| contract.required_artifacts.clone())
                    .unwrap_or_default(),
            )
            .await;
            self.persist_goal_monitor_intervention(mission_id, &goal.goal_id, &intervention)
                .await;
            self.mission_manager
                .broadcast(
                    mission_id,
                    StreamEvent::Status {
                        status: serde_json::json!({
                            "type": "goal_waiting_provider_capacity",
                            "goal_id": goal.goal_id,
                            "feedback": feedback,
                            "observed_evidence": [observed],
                            "retry_after_seconds": delay.as_secs(),
                        })
                        .to_string(),
                    },
                )
                .await;
        } else {
            tracing::debug!(
                "Mission {} goal {} remains in waiting_external cooldown for {:?}; skipping duplicate provider intervention",
                mission_id,
                goal.goal_id,
                delay
            );
        }
        if let Err(err) = self
            .agent_service
            .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::Pending)
            .await
        {
            tracing::warn!(
                "Failed to reset goal {} to pending after provider capacity block in mission {}: {}",
                goal.goal_id,
                mission_id,
                err
            );
        }
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = cancel_token.cancelled() => {}
        }
        Ok(Some(GoalLoopResolution::Continue))
    }

    /// Synthesize final results from all completed/abandoned goals.
    async fn synthesize_results(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<()> {
        let mission = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Mission not found"))?;

        if let Err(err) = runtime::reconcile_mission_artifacts(&self.agent_service, &mission).await {
            tracing::warn!(
                "Failed to reconcile workspace artifacts before adaptive synthesis for mission {}: {}",
                mission_id,
                err
            );
        }

        let goals = mission.goal_tree.as_deref().unwrap_or(&[]);
        if goals.is_empty() {
            return Err(anyhow!(
                "Adaptive mission has no goals to synthesize; refusing empty completion"
            ));
        }
        if !Self::goal_tree_has_completion_basis(goals) {
            return Err(anyhow!(
                "Adaptive mission has no processed goals or evidence; refusing empty completion"
            ));
        }

        let mut summary_parts = String::new();
        for g in goals {
            let status_label = match g.status {
                GoalStatus::Completed => "completed",
                GoalStatus::Abandoned => "abandoned",
                _ => "other",
            };
            let output = g.output_summary.as_deref().unwrap_or("(no output)");
            let truncated_output = if output.chars().count() > 500 {
                let t: String = output.chars().take(497).collect();
                format!("{}...", t)
            } else {
                output.to_string()
            };
            summary_parts.push_str(&format!(
                "- {} [{}]: {}\n",
                g.title, status_label, truncated_output
            ));
        }

        let prompt = format!(
            "All goals have been processed. Please synthesize the final results.\n\n\
             ## Goal Execution Results\n{}\n\n\
             Provide a concise final summary including key achievements and any incomplete parts.",
            summary_parts
        );

        // Best-effort synthesis; failure is non-fatal
        let synthesis_ok = if let Err(e) = self
            .execute_via_bridge(
                agent_id,
                session_id,
                mission_id,
                &prompt,
                cancel_token,
                workspace_path,
                None,
            )
            .await
        {
            tracing::warn!("Mission {} synthesis failed: {}", mission_id, e);
            false
        } else {
            true
        };

        if synthesis_ok {
            if let Some(summary) = self.extract_step_summary(session_id).await {
                if let Err(e) = self
                    .agent_service
                    .set_mission_final_summary(mission_id, &summary)
                    .await
                {
                    tracing::warn!("Failed to save mission {} final summary: {}", mission_id, e);
                }
            }
        }

        Ok(())
    }

    /// Resume a paused adaptive mission (with guaranteed cleanup).
    /// NOTE: resume_mission in MissionExecutor has no cleanup wrapper,
    /// so we must handle Done broadcast + mission_manager.complete here.
    pub async fn resume_adaptive(
        &self,
        mission_id: &str,
        cancel_token: CancellationToken,
        resume_feedback: Option<String>,
    ) -> Result<()> {
        let result = self
            .resume_adaptive_inner(mission_id, cancel_token, resume_feedback)
            .await;

        // Read actual mission status from DB to determine the correct Done event
        // (handles re-pause at checkpoint, completed, cancelled, etc.)
        match &result {
            Ok(_) => {
                let done_status = match self.agent_service.get_mission(mission_id).await {
                    Ok(Some(m)) => {
                        let done_status = Self::adaptive_done_status(&m);
                        if done_status == "failed"
                            && matches!(
                                m.status,
                                MissionStatus::Running
                                    | MissionStatus::Planning
                                    | MissionStatus::Planned
                                    | MissionStatus::Draft
                            )
                        {
                            tracing::error!(
                                "Adaptive mission {} resume returned Ok while mission status remained {:?} without an active waiting_external hold",
                                mission_id,
                                m.status
                            );
                        }
                        done_status
                    }
                    Ok(None) => {
                        tracing::warn!("Mission {} not found during cleanup", mission_id);
                        "completed"
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to read mission {} status during cleanup: {}",
                            mission_id,
                            e
                        );
                        "completed"
                    }
                };

                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Done {
                            status: done_status.to_string(),
                            error: None,
                        },
                    )
                    .await;
            }
            Err(e) => {
                let mut done_status = "failed";
                let mut done_error = Some(e.to_string());
                let mut should_persist_failure = true;

                if let Ok(Some(mission)) = self.agent_service.get_mission(mission_id).await {
                    match mission.status {
                        MissionStatus::Paused => {
                            done_status = "paused";
                            done_error = None;
                            should_persist_failure = false;
                        }
                        MissionStatus::Cancelled => {
                            done_status = "cancelled";
                            done_error = None;
                            should_persist_failure = false;
                        }
                        MissionStatus::Running
                        | MissionStatus::Planning
                        | MissionStatus::Planned
                        | MissionStatus::Draft
                            if Self::mission_waiting_external_active(&mission) =>
                        {
                            done_status = "waiting_external";
                            done_error = None;
                            should_persist_failure = false;
                        }
                        _ => {}
                    }
                }

                if should_persist_failure {
                    self.persist_failure_state(mission_id, &e.to_string()).await;
                }
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Done {
                            status: done_status.to_string(),
                            error: done_error,
                        },
                    )
                    .await;
            }
        }

        self.mission_manager.complete(mission_id).await;
        result
    }

    async fn persist_failure_state(&self, mission_id: &str, error_message: &str) {
        if let Err(e) = self
            .agent_service
            .update_mission_status(mission_id, &MissionStatus::Failed)
            .await
        {
            tracing::warn!(
                "Failed to mark mission {} as failed during adaptive cleanup: {}",
                mission_id,
                e
            );
        }

        if let Err(e) = self
            .agent_service
            .set_mission_error(mission_id, error_message)
            .await
        {
            tracing::warn!(
                "Failed to persist mission {} error message during adaptive cleanup: {}",
                mission_id,
                e
            );
        }
    }

    async fn resume_adaptive_inner(
        &self,
        mission_id: &str,
        cancel_token: CancellationToken,
        resume_feedback: Option<String>,
    ) -> Result<()> {
        let mission = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Mission not found"))?;

        if !matches!(
            mission.status,
            MissionStatus::Paused | MissionStatus::Failed
        ) {
            return Err(anyhow!("Mission is not paused/failed"));
        }

        // Read workspace_path from mission doc (set during initial execution)
        let workspace_path = mission.workspace_path.clone();
        let mut session_id = runtime::ensure_mission_session(
            &self.agent_service,
            mission_id,
            &mission,
            None,
            mission.step_timeout_seconds,
            workspace_path.as_deref(),
        )
        .await?;

        if mission.status == MissionStatus::Failed {
            if let Err(e) = self.agent_service.clear_mission_error(mission_id).await {
                tracing::warn!(
                    "Failed to clear mission {} error before adaptive resume: {}",
                    mission_id,
                    e
                );
            }
        }
        if !Self::goal_tree_is_usable(mission.goal_tree.as_deref()) {
            tracing::warn!(
                "Mission {} resume found empty goal tree; rebuilding adaptive plan before execution",
                mission_id
            );
            session_id = self
                .run_planning_phase(
                    mission_id,
                    &mission,
                    cancel_token.clone(),
                    workspace_path.as_deref(),
                )
                .await?;
        }
        if let Some(goals) = mission.goal_tree.as_ref() {
            for goal in goals {
                let should_reset = if mission.status == MissionStatus::Failed {
                    matches!(goal.status, GoalStatus::Failed | GoalStatus::Running)
                } else {
                    // Mission paused: clean up stale running goal left by interrupted pause flow.
                    matches!(goal.status, GoalStatus::Running)
                };
                if !should_reset {
                    continue;
                }
                if let Err(e) = self
                    .agent_service
                    .reset_goal_for_retry(mission_id, &goal.goal_id)
                    .await
                {
                    tracing::warn!(
                        "Failed to reset mission {} goal {} for retry: {}",
                        mission_id,
                        goal.goal_id,
                        e
                    );
                }
            }
        }

        // Update status to Running
        if let Err(e) = self
            .agent_service
            .update_mission_status(mission_id, &MissionStatus::Running)
            .await
        {
            tracing::warn!(
                "Failed to set mission {} running on resume: {}",
                mission_id,
                e
            );
        }

        // Resume goal loop (skips completed/abandoned goals automatically)
        self.execute_goal_loop(
            mission_id,
            &mission.agent_id,
            &session_id,
            cancel_token.clone(),
            workspace_path.as_deref(),
            resume_feedback.as_deref(),
        )
        .await?;

        // Check terminal/pause states — don't synthesize in these cases.
        let current = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?;
        if let Some(m) = current.as_ref() {
            if matches!(
                m.status,
                MissionStatus::Paused
                    | MissionStatus::Cancelled
                    | MissionStatus::Failed
                    | MissionStatus::Completed
            ) {
                return Ok(());
            }
        }

        self.synthesize_and_complete(
            mission_id,
            &mission.agent_id,
            &session_id,
            cancel_token,
            workspace_path.as_deref(),
        )
        .await
    }
}
