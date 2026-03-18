use std::collections::BTreeSet;
use std::sync::Arc;

use agime_team::MongoDb;
use anyhow::{anyhow, Result};

use super::mission_executor::MissionExecutor;
use super::mission_manager::MissionManager;
use super::mission_mongo::{
    GoalNode, MissionArtifactDoc, MissionDoc, MissionMonitorIntervention, MissionMonitorSnapshot,
    MissionStatus, MissionStep, MonitorActionRequest, MonitorAssessmentSnapshot,
    MonitorAssetRecord, MonitorAssetSnapshot, MonitorContractSnapshot, MonitorGoalSnapshot,
    MonitorInterventionSnapshot, MonitorStepSnapshot, StepEvidenceBundle, StepStatus,
    StepSupervisorState,
};
use super::service_mongo::AgentService;
use super::task_manager::StreamEvent;
use super::runtime;

const WAITING_EXTERNAL_COOLDOWN_SECS: i64 = 300;

#[derive(Debug, Clone)]
pub struct MonitorActionOutcome {
    pub status: String,
    pub action: String,
    pub applied: bool,
}

pub fn bson_time_to_rfc3339(value: Option<bson::DateTime>) -> Option<String> {
    value.map(|ts| ts.to_chrono().to_rfc3339())
}

fn collect_intervention_signals(
    pending: Option<&MissionMonitorIntervention>,
    applied: Option<&MissionMonitorIntervention>,
) -> BTreeSet<String> {
    let mut signals = BTreeSet::new();
    for intervention in [pending, applied].into_iter().flatten() {
        signals.insert(intervention.action.trim().to_ascii_lowercase());
        for tag in &intervention.semantic_tags {
            let normalized = tag.trim().to_ascii_lowercase().replace([' ', '-'], "_");
            if !normalized.is_empty() {
                signals.insert(normalized);
            }
        }
    }
    signals
}

fn bundle_has_completion_evidence(bundle: Option<&StepEvidenceBundle>) -> bool {
    let Some(bundle) = bundle else {
        return false;
    };
    !bundle.artifact_paths.is_empty()
        || !bundle.required_artifact_paths.is_empty()
        || !bundle.planning_evidence_paths.is_empty()
        || !bundle.quality_evidence_paths.is_empty()
        || !bundle.runtime_evidence_paths.is_empty()
        || !bundle.deployment_evidence_paths.is_empty()
        || !bundle.review_evidence_paths.is_empty()
        || !bundle.risk_evidence_paths.is_empty()
        || bundle
            .latest_summary
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
}

fn summarize_quality(
    bundle: Option<&StepEvidenceBundle>,
    completion_checks: &[String],
) -> Option<String> {
    let Some(bundle) = bundle else {
        return (!completion_checks.is_empty())
            .then_some("completion checks declared but no quality evidence recorded".to_string());
    };

    if !bundle.quality_evidence_paths.is_empty() || !bundle.review_evidence_paths.is_empty() {
        return Some(format!(
            "quality evidence present (quality={}, review={})",
            bundle.quality_evidence_paths.len(),
            bundle.review_evidence_paths.len()
        ));
    }

    (!completion_checks.is_empty())
        .then_some("completion checks declared but quality evidence remains partial".to_string())
}

fn summarize_risk(
    bundle: Option<&StepEvidenceBundle>,
    current_blocker: Option<&str>,
    contract_verified: Option<bool>,
) -> Option<String> {
    if let Some(blocker) = current_blocker
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(blocker.to_string());
    }

    let Some(bundle) = bundle else {
        return (contract_verified == Some(false))
            .then_some("contract verification remains weak".to_string());
    };

    if !bundle.risk_evidence_paths.is_empty() {
        return Some(format!(
            "risk evidence recorded ({})",
            bundle.risk_evidence_paths.len()
        ));
    }

    (contract_verified == Some(false)).then_some("contract verification remains weak".to_string())
}

fn derive_step_status_assessment(
    step: &MissionStep,
    evidence_sufficient: bool,
    intervention_signals: &BTreeSet<String>,
) -> Option<String> {
    if intervention_signals.contains("mark_waiting_external")
        || intervention_signals.contains("waiting_external")
    {
        return Some("waiting_external".to_string());
    }
    if intervention_signals.contains("waiting_runtime") {
        return Some("waiting_runtime".to_string());
    }
    if step
        .current_blocker
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
        && !evidence_sufficient
    {
        return Some("blocked".to_string());
    }
    if evidence_sufficient {
        return Some("evidence_sufficient".to_string());
    }
    match step.supervisor_state.as_ref() {
        Some(StepSupervisorState::Healthy) => Some("healthy".to_string()),
        Some(StepSupervisorState::Busy) => Some("busy".to_string()),
        Some(StepSupervisorState::Drifting) => Some("drifting".to_string()),
        Some(StepSupervisorState::Stalled) => Some("stalled".to_string()),
        None => None,
    }
}

pub fn assess_step_snapshot(
    step: &MissionStep,
    pending: Option<&MissionMonitorIntervention>,
    applied: Option<&MissionMonitorIntervention>,
) -> MonitorAssessmentSnapshot {
    let intervention_signals = collect_intervention_signals(pending, applied);
    let mut observed_evidence = Vec::new();
    let mut missing_evidence = Vec::new();

    if step
        .output_summary
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
    {
        observed_evidence.push("output_summary_present".to_string());
    }

    if let Some(bundle) = step.evidence_bundle.as_ref() {
        if !bundle.artifact_paths.is_empty() || !bundle.required_artifact_paths.is_empty() {
            observed_evidence.push("artifact_evidence_present".to_string());
        }
        if !bundle.planning_evidence_paths.is_empty() {
            observed_evidence.push("planning_evidence_present".to_string());
        }
        if !bundle.quality_evidence_paths.is_empty() || !bundle.review_evidence_paths.is_empty() {
            observed_evidence.push("quality_evidence_present".to_string());
        }
        if !bundle.runtime_evidence_paths.is_empty() {
            observed_evidence.push("runtime_evidence_present".to_string());
        }
        if !bundle.deployment_evidence_paths.is_empty() {
            observed_evidence.push("deployment_evidence_present".to_string());
        }
        if !bundle.risk_evidence_paths.is_empty() {
            observed_evidence.push("risk_evidence_present".to_string());
        }
    }

    if step
        .contract_verification
        .as_ref()
        .and_then(|verification| verification.accepted)
        == Some(true)
    {
        observed_evidence.push("contract_verified".to_string());
    }

    if !step.required_artifacts.is_empty()
        && step
            .evidence_bundle
            .as_ref()
            .map(|bundle| bundle.required_artifact_paths.is_empty())
            .unwrap_or(true)
    {
        missing_evidence.push("declared artifacts still lack recorded evidence".to_string());
    }
    if !step.completion_checks.is_empty()
        && step
            .evidence_bundle
            .as_ref()
            .map(|bundle| {
                bundle.quality_evidence_paths.is_empty()
                    && bundle.runtime_evidence_paths.is_empty()
                    && bundle.deployment_evidence_paths.is_empty()
            })
            .unwrap_or(true)
    {
        missing_evidence
            .push("declared completion checks are not yet backed by evidence".to_string());
    }

    let has_summary = step
        .output_summary
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
        || step
            .evidence_bundle
            .as_ref()
            .and_then(|bundle| bundle.latest_summary.as_deref())
            .is_some_and(|text| !text.trim().is_empty());
    let evidence_sufficient =
        bundle_has_completion_evidence(step.evidence_bundle.as_ref()) || has_summary;

    if !evidence_sufficient && observed_evidence.is_empty() {
        missing_evidence.push("no summary or evidence recorded yet".to_string());
    }

    MonitorAssessmentSnapshot {
        status_assessment: derive_step_status_assessment(
            step,
            evidence_sufficient,
            &intervention_signals,
        ),
        evidence_sufficient,
        observed_evidence,
        missing_evidence,
        quality_summary: summarize_quality(step.evidence_bundle.as_ref(), &step.completion_checks),
        risk_summary: summarize_risk(
            step.evidence_bundle.as_ref(),
            step.current_blocker.as_deref(),
            step.contract_verification
                .as_ref()
                .and_then(|verification| verification.accepted),
        ),
    }
}

pub fn assess_goal_snapshot(
    goal: &GoalNode,
    pending: Option<&MissionMonitorIntervention>,
    applied: Option<&MissionMonitorIntervention>,
) -> MonitorAssessmentSnapshot {
    let intervention_signals = collect_intervention_signals(pending, applied);
    let mut observed_evidence = Vec::new();
    let mut missing_evidence = Vec::new();

    if goal
        .output_summary
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
    {
        observed_evidence.push("goal_output_summary_present".to_string());
    }
    if goal.runtime_contract.is_some() {
        observed_evidence.push("goal_runtime_contract_present".to_string());
    }
    if goal
        .contract_verification
        .as_ref()
        .and_then(|verification| verification.accepted)
        == Some(true)
    {
        observed_evidence.push("goal_contract_verified".to_string());
    }
    if !goal.attempts.is_empty() {
        observed_evidence.push("goal_attempt_history_present".to_string());
    }

    let evidence_sufficient = goal
        .output_summary
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
        || (goal.runtime_contract.is_some()
            && goal
                .contract_verification
                .as_ref()
                .and_then(|verification| verification.accepted)
                == Some(true));

    if !evidence_sufficient && observed_evidence.is_empty() {
        missing_evidence.push("goal evidence remains thin".to_string());
    }

    let status_assessment = if intervention_signals.contains("mark_waiting_external")
        || intervention_signals.contains("waiting_external")
    {
        Some("waiting_external".to_string())
    } else if goal
        .pivot_reason
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
        && !evidence_sufficient
    {
        Some("blocked".to_string())
    } else if evidence_sufficient {
        Some("evidence_sufficient".to_string())
    } else {
        None
    };

    MonitorAssessmentSnapshot {
        status_assessment,
        evidence_sufficient,
        observed_evidence,
        missing_evidence,
        quality_summary: None,
        risk_summary: goal
            .pivot_reason
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string),
    }
}

fn build_contract_snapshot(
    step: Option<&MissionStep>,
    goal: Option<&GoalNode>,
) -> Option<MonitorContractSnapshot> {
    if let Some(step) = step {
        let verified = step.contract_verification.as_ref().and_then(|v| v.accepted);
        if let Some(contract) = step.runtime_contract.as_ref() {
            return Some(MonitorContractSnapshot {
                required_artifacts: contract.required_artifacts.clone(),
                completion_checks: contract.completion_checks.clone(),
                no_artifact_reason: contract.no_artifact_reason.clone(),
                verified,
            });
        }
        if !step.required_artifacts.is_empty()
            || !step.completion_checks.is_empty()
            || verified.is_some()
        {
            return Some(MonitorContractSnapshot {
                required_artifacts: step.required_artifacts.clone(),
                completion_checks: step.completion_checks.clone(),
                no_artifact_reason: None,
                verified,
            });
        }
    }

    goal.and_then(|goal| {
        let verified = goal.contract_verification.as_ref().and_then(|v| v.accepted);
        goal.runtime_contract.as_ref().map(|contract| MonitorContractSnapshot {
            required_artifacts: contract.required_artifacts.clone(),
            completion_checks: contract.completion_checks.clone(),
            no_artifact_reason: contract.no_artifact_reason.clone(),
            verified,
        })
    })
}

fn build_asset_snapshot(artifacts: &[MissionArtifactDoc]) -> Option<MonitorAssetSnapshot> {
    if artifacts.is_empty() {
        return None;
    }

    let core_assets_now = artifacts
        .iter()
        .rev()
        .filter_map(|artifact| artifact.file_path.clone().or_else(|| {
            (!artifact.name.trim().is_empty()).then_some(artifact.name.clone())
        }))
        .take(8)
        .collect::<Vec<_>>();

    let recent_assets = artifacts
        .iter()
        .rev()
        .take(8)
        .map(|artifact| MonitorAssetRecord {
            name: artifact.name.clone(),
            file_path: artifact.file_path.clone(),
            artifact_type: artifact.artifact_type.clone(),
            step_index: artifact.step_index,
            size: artifact.size,
        })
        .collect::<Vec<_>>();

    Some(MonitorAssetSnapshot {
        total_assets: artifacts.len(),
        core_assets_now,
        recent_assets,
    })
}

pub fn build_monitor_snapshot(
    mission: &MissionDoc,
    artifacts: &[MissionArtifactDoc],
    is_active: bool,
) -> MissionMonitorSnapshot {
    let to_snapshot = |intervention: &MissionMonitorIntervention| MonitorInterventionSnapshot {
        action: intervention.action.clone(),
        feedback: intervention.feedback.clone(),
        semantic_tags: intervention.semantic_tags.clone(),
        observed_evidence: intervention.observed_evidence.clone(),
        missing_core_deliverables: intervention.missing_core_deliverables.clone(),
        confidence: intervention.confidence,
        strategy_patch: intervention.strategy_patch.clone(),
        subagent_recommended: intervention.subagent_recommended,
        parallelism_budget: intervention.parallelism_budget,
        requested_at: bson_time_to_rfc3339(intervention.requested_at),
        applied_at: bson_time_to_rfc3339(intervention.applied_at),
    };
    let current_step = infer_current_step(mission);
    let step_ref = current_step.and_then(|index| mission.steps.get(index as usize));
    let step = step_ref.map(|step| MonitorStepSnapshot {
        index: step.index,
        title: step.title.clone(),
        description: step.description.clone(),
        status: step.status.clone(),
        supervisor_state: step.supervisor_state.clone(),
        last_activity_at: bson_time_to_rfc3339(step.last_activity_at),
        last_progress_at: bson_time_to_rfc3339(step.last_progress_at),
        progress_score: step.progress_score,
        current_blocker: step.current_blocker.clone(),
        last_supervisor_hint: step.last_supervisor_hint.clone(),
        stall_count: step.stall_count,
        retry_count: step.retry_count,
        output_summary_present: step
            .output_summary
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty()),
        required_artifacts: step.required_artifacts.clone(),
        completion_checks: step.completion_checks.clone(),
        recent_progress_events: step.recent_progress_events.clone(),
        evidence_bundle: step.evidence_bundle.clone(),
        assessment: Some(assess_step_snapshot(
            step,
            mission.pending_monitor_intervention.as_ref(),
            mission.last_applied_monitor_intervention.as_ref(),
        )),
    });
    let goal_ref = mission.current_goal_id.as_ref().and_then(|goal_id| {
        mission
            .goal_tree
            .as_ref()
            .and_then(|goals| goals.iter().find(|goal| goal.goal_id == *goal_id))
    });
    let goal = goal_ref.map(|goal| MonitorGoalSnapshot {
        goal_id: goal.goal_id.clone(),
        parent_id: goal.parent_id.clone(),
        title: goal.title.clone(),
        description: goal.description.clone(),
        success_criteria: goal.success_criteria.clone(),
        status: goal.status.clone(),
        attempt_count: goal.attempts.len(),
        output_summary_present: goal
            .output_summary
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty()),
        has_runtime_contract: goal.runtime_contract.is_some(),
        contract_verified: goal.contract_verification.as_ref().and_then(|v| v.accepted),
        pivot_reason: goal.pivot_reason.clone(),
        last_activity_at: bson_time_to_rfc3339(goal.last_activity_at),
        last_progress_at: bson_time_to_rfc3339(goal.last_progress_at),
        assessment: Some(assess_goal_snapshot(
            goal,
            mission.pending_monitor_intervention.as_ref(),
            mission.last_applied_monitor_intervention.as_ref(),
        )),
    });
    let goal_last_activity_at = goal.as_ref().and_then(|snapshot| snapshot.last_activity_at.clone());
    let goal_last_progress_at = goal.as_ref().and_then(|snapshot| snapshot.last_progress_at.clone());
    let current_contract = build_contract_snapshot(step_ref, goal_ref);
    let assets = build_asset_snapshot(artifacts);

    MissionMonitorSnapshot {
        mission_id: mission.mission_id.clone(),
        status: mission.status.clone(),
        execution_mode: mission.execution_mode.clone(),
        execution_profile: mission.execution_profile.clone(),
        is_active,
        current_run_id: mission.current_run_id.clone(),
        current_step,
        current_goal_id: mission.current_goal_id.clone(),
        error_message: mission.error_message.clone(),
        completion_assessment: mission.completion_assessment.clone(),
        current_strategy: mission.current_strategy.clone(),
        latest_worker_state: mission.latest_worker_state.clone(),
        latest_stuck_phase_snapshot: mission.latest_stuck_phase_snapshot.clone(),
        active_repair_lane_id: mission.active_repair_lane_id.clone(),
        consecutive_no_tool_count: mission.consecutive_no_tool_count,
        last_blocker_fingerprint: mission.last_blocker_fingerprint.clone(),
        waiting_external_until: bson_time_to_rfc3339(mission.waiting_external_until),
        context: mission.context.clone(),
        pending_intervention: mission
            .pending_monitor_intervention
            .as_ref()
            .map(to_snapshot),
        last_applied_intervention: mission
            .last_applied_monitor_intervention
            .as_ref()
            .map(to_snapshot),
        goal_last_activity_at,
        goal_last_progress_at,
        current_contract,
        assets,
        step,
        goal,
    }
}

fn waiting_external_until_after_cooldown() -> bson::DateTime {
    bson::DateTime::from_millis(
        bson::DateTime::now().timestamp_millis() + WAITING_EXTERNAL_COOLDOWN_SECS * 1000,
    )
}

fn waiting_external_repair_lane_id(mission: &MissionDoc) -> Option<String> {
    mission
        .current_step
        .map(|step_index| format!("step-{}", step_index))
        .or_else(|| {
            mission.current_goal_id.as_ref().and_then(|goal_id| {
                let normalized = goal_id.to_ascii_lowercase();
                (normalized.contains("salvage") || normalized.contains("repair"))
                    .then_some(goal_id.clone())
            })
        })
}

fn infer_current_step(mission: &MissionDoc) -> Option<u32> {
    mission
        .current_step
        .or_else(|| infer_current_step_from_worker_state(mission))
        .or_else(|| {
            mission
                .steps
                .iter()
                .find(|step| {
                    matches!(
                        step.status,
                        StepStatus::Running
                            | StepStatus::AwaitingApproval
                            | StepStatus::Pending
                    )
                })
                .map(|step| step.index as u32)
        })
}

fn infer_current_step_from_worker_state(mission: &MissionDoc) -> Option<u32> {
    let label = mission
        .latest_worker_state
        .as_ref()
        .and_then(|state| state.current_goal.as_deref())?;
    let digits = label
        .trim()
        .strip_prefix("Step ")?
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    let step_number = digits.parse::<u32>().ok()?;
    step_number.checked_sub(1)
}

pub fn normalize_monitor_action(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    match normalized.as_str() {
        "continue_current" => Some("continue_current".to_string()),
        "continue_with_hint" => Some("continue_current".to_string()),
        "continue_with_replan" => Some("continue_with_replan".to_string()),
        "repair_deliverables" => Some("repair_deliverables".to_string()),
        "repair_contract" => Some("repair_contract".to_string()),
        "extend_lease"
        | "resume_current_step"
        | "split_current_step"
        | "replan_remaining_goals"
        | "mark_waiting_external"
        | "complete_if_evidence_sufficient"
        | "completed_with_minor_gaps"
        | "partial_handoff"
        | "blocked_by_environment"
        | "blocked_by_tooling"
        | "blocked_fail" => Some(normalized),
        _ => None,
    }
}

pub fn build_monitor_feedback(action: &str, body: &MonitorActionRequest) -> Option<String> {
    build_monitor_feedback_parts(
        action,
        body.feedback.as_deref(),
        &body.observed_evidence,
        &body.semantic_tags,
        &body.missing_core_deliverables,
        body.confidence,
        body.strategy_patch.as_ref(),
        body.subagent_recommended,
        body.parallelism_budget,
    )
}

pub fn build_monitor_feedback_parts(
    action: &str,
    feedback: Option<&str>,
    observed_evidence: &[String],
    semantic_tags: &[String],
    missing_core_deliverables: &[String],
    confidence: Option<f64>,
    strategy_patch: Option<&super::mission_mongo::MissionStrategyPatch>,
    subagent_recommended: Option<bool>,
    parallelism_budget: Option<u32>,
) -> Option<String> {
    let mut lines = vec![format!("Monitor requested action: {}", action)];
    if let Some(feedback) = feedback.map(str::trim).filter(|s| !s.is_empty()) {
        lines.push(format!("Monitor feedback: {}", feedback));
    }
    if !observed_evidence.is_empty() {
        lines.push(format!(
            "Observed evidence: {}",
            observed_evidence.join(", ")
        ));
    }
    if !semantic_tags.is_empty() {
        lines.push(format!("Semantic tags: {}", semantic_tags.join(", ")));
    }
    if !missing_core_deliverables.is_empty() {
        lines.push(format!(
            "Missing core deliverables: {}",
            missing_core_deliverables.join(", ")
        ));
    }
    if let Some(confidence) = confidence {
        lines.push(format!("Monitor confidence: {:.2}", confidence));
    }
    if let Some(strategy_patch) = strategy_patch {
        if let Some(reason) = strategy_patch
            .reason_for_change
            .as_deref()
            .filter(|text| !text.trim().is_empty())
        {
            lines.push(format!("Strategy change reason: {}", reason));
        }
        if let Some(new_goal_shape) = strategy_patch
            .new_goal_shape
            .as_deref()
            .filter(|text| !text.trim().is_empty())
        {
            lines.push(format!("Reframed goal shape: {}", new_goal_shape));
        }
    }
    if let Some(subagent_recommended) = subagent_recommended {
        lines.push(format!(
            "Subagent recommended: {}",
            if subagent_recommended { "yes" } else { "no" }
        ));
    }
    if let Some(parallelism_budget) = parallelism_budget {
        lines.push(format!("Parallelism budget: {}", parallelism_budget));
    }
    Some(lines.join("\n"))
}

pub fn format_monitor_intervention_instruction(
    intervention: &MissionMonitorIntervention,
) -> Option<String> {
    build_monitor_feedback_parts(
        &intervention.action,
        intervention.feedback.as_deref(),
        &intervention.observed_evidence,
        &intervention.semantic_tags,
        &intervention.missing_core_deliverables,
        intervention.confidence,
        intervention.strategy_patch.as_ref(),
        intervention.subagent_recommended,
        intervention.parallelism_budget,
    )
}

pub async fn consume_pending_monitor_intervention_instruction(
    service: &AgentService,
    mission_manager: &Arc<MissionManager>,
    mission_id: &str,
) -> Option<String> {
    let mission = match service.get_mission(mission_id).await {
        Ok(Some(mission)) => mission,
        Ok(None) => return None,
        Err(err) => {
            tracing::warn!(
                "Failed to load mission {} while consuming monitor intervention: {}",
                mission_id,
                err
            );
            return None;
        }
    };
    let intervention = mission.pending_monitor_intervention?;
    let instruction = format_monitor_intervention_instruction(&intervention)?;
    match service
        .mark_pending_monitor_intervention_applied(mission_id, &intervention)
        .await
    {
        Ok(true) => {
            mission_manager
                .broadcast(
                    mission_id,
                    StreamEvent::Status {
                        status: serde_json::json!({
                            "type": "monitor_intervention_applied",
                            "action": intervention.action,
                            "semantic_tags": intervention.semantic_tags,
                            "observed_evidence": intervention.observed_evidence,
                        })
                        .to_string(),
                    },
                )
                .await;
            Some(instruction)
        }
        Ok(false) => None,
        Err(err) => {
            tracing::warn!(
                "Failed to mark monitor intervention applied for mission {}: {}",
                mission_id,
                err
            );
            None
        }
    }
}

pub async fn execute_monitor_action(
    service: &AgentService,
    db: &Arc<MongoDb>,
    mission_manager: &Arc<MissionManager>,
    workspace_root: &str,
    mission: &MissionDoc,
    action: String,
    feedback: Option<String>,
    observed_evidence: Vec<String>,
    semantic_tags: Vec<String>,
    missing_core_deliverables: Vec<String>,
    confidence: Option<f64>,
    strategy_patch: Option<super::mission_mongo::MissionStrategyPatch>,
    subagent_recommended: Option<bool>,
    parallelism_budget: Option<u32>,
) -> Result<MonitorActionOutcome> {
    let pending_intervention = MissionMonitorIntervention {
        action: action.clone(),
        feedback: feedback.clone(),
        semantic_tags: semantic_tags.clone(),
        observed_evidence: observed_evidence.clone(),
        missing_core_deliverables: missing_core_deliverables.clone(),
        confidence,
        strategy_patch: strategy_patch.clone(),
        subagent_recommended,
        parallelism_budget,
        requested_at: Some(bson::DateTime::now()),
        applied_at: None,
    };
    let current_strategy = super::mission_mongo::MissionStrategyState {
        action: Some(action.clone()),
        reason: feedback.clone(),
        missing_core_deliverables: missing_core_deliverables.clone(),
        confidence,
        strategy_patch: strategy_patch.clone(),
        subagent_recommended,
        parallelism_budget,
        updated_at: Some(bson::DateTime::now()),
    };
    mission_manager
        .broadcast(
            &mission.mission_id,
            StreamEvent::Status {
                status: serde_json::json!({
                    "type": "monitor_action_requested",
                    "action": action.clone(),
                    "observed_evidence": observed_evidence,
                    "semantic_tags": semantic_tags,
                    "missing_core_deliverables": missing_core_deliverables,
                    "confidence": confidence,
                    "subagent_recommended": subagent_recommended,
                    "parallelism_budget": parallelism_budget,
                })
                .to_string(),
            },
        )
        .await;

    service
        .set_current_strategy(&mission.mission_id, Some(&current_strategy))
        .await
        .map_err(|e| {
            anyhow!(
                "Failed to persist current strategy for mission {}: {}",
                mission.mission_id,
                e
            )
        })?;

    if action == "mark_waiting_external" {
        let blocker = feedback
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .or_else(|| {
                mission
                    .current_strategy
                    .as_ref()
                    .and_then(|strategy| strategy.reason.as_deref())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
            })
            .unwrap_or("Mission is waiting on an external dependency");
        let convergence_patch = super::mission_mongo::MissionConvergencePatch {
            active_repair_lane_id: Some(waiting_external_repair_lane_id(mission)),
            consecutive_no_tool_count: Some(0),
            last_blocker_fingerprint: Some(runtime::blocker_fingerprint(blocker)),
            waiting_external_until: Some(Some(waiting_external_until_after_cooldown())),
        };
        service
            .patch_mission_convergence_state(&mission.mission_id, &convergence_patch)
            .await
            .map_err(|e| {
                anyhow!(
                    "Failed to persist waiting_external convergence state for mission {}: {}",
                    mission.mission_id,
                    e
                )
            })?;
        if mission.current_goal_id.is_some() {
            if let Err(err) = service.clear_mission_current_goal(&mission.mission_id).await {
                tracing::warn!(
                    "Failed to clear current goal while parking mission {} in waiting_external: {}",
                    mission.mission_id,
                    err
                );
            }
        }
    }

    let executable_resume_action = matches!(
        action.as_str(),
        "continue_current"
            | "continue_with_replan"
            | "repair_deliverables"
            | "repair_contract"
            | "resume_current_step"
            | "split_current_step"
            | "replan_remaining_goals"
            | "complete_if_evidence_sufficient"
    );

    if executable_resume_action
        && matches!(
            mission.status,
            MissionStatus::Paused | MissionStatus::Failed
        )
    {
        let registration = mission_manager
            .register_with_grace(&mission.mission_id)
            .await
            .ok_or_else(|| anyhow!("Mission is already active"))?;
        let run_id = registration.run_id.clone();
        let cancel_token = registration.cancel_token;
        if let Err(e) = service
            .set_mission_current_run(&mission.mission_id, &run_id)
            .await
        {
            mission_manager.complete(&mission.mission_id).await;
            return Err(anyhow!(
                "Failed to set current run for monitor action {} on mission {}: {}",
                action,
                mission.mission_id,
                e
            ));
        }

        let executor = MissionExecutor::new(
            db.clone(),
            mission_manager.clone(),
            workspace_root.to_string(),
        );
        let mission_id = mission.mission_id.clone();
        let action_for_task = action.clone();
        tokio::spawn(async move {
            if let Err(e) = executor
                .resume_mission(&mission_id, cancel_token, feedback)
                .await
            {
                tracing::error!(
                    "Mission monitor action resume failed: {} action={} err={}",
                    mission_id,
                    action_for_task,
                    e
                );
            }
        });

        return Ok(MonitorActionOutcome {
            status: "action_resuming".to_string(),
            action,
            applied: true,
        });
    }

    service
        .set_pending_monitor_intervention(&mission.mission_id, &pending_intervention)
        .await
        .map_err(|e| {
            anyhow!(
                "Failed to persist pending monitor intervention for mission {}: {}",
                mission.mission_id,
                e
            )
        })?;

    Ok(MonitorActionOutcome {
        status: "action_recorded".to_string(),
        action,
        applied: false,
    })
}

#[cfg(test)]
mod tests {
    use super::{assess_goal_snapshot, assess_step_snapshot, normalize_monitor_action};
    use crate::agent::mission_mongo::{
        GoalNode, GoalStatus, MissionMonitorIntervention, MissionStep, StepEvidenceBundle,
        StepStatus, StepSupervisorState,
    };

    fn sample_step() -> MissionStep {
        MissionStep {
            index: 0,
            title: "step".to_string(),
            description: "desc".to_string(),
            status: StepStatus::Running,
            is_checkpoint: false,
            approved_by: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            supervisor_state: Some(StepSupervisorState::Busy),
            last_activity_at: None,
            last_progress_at: None,
            progress_score: Some(3),
            current_blocker: None,
            last_supervisor_hint: None,
            stall_count: 0,
            recent_progress_events: Vec::new(),
            evidence_bundle: None,
            tokens_used: 0,
            output_summary: None,
            retry_count: 0,
            max_retries: 2,
            timeout_seconds: None,
            required_artifacts: Vec::new(),
            completion_checks: Vec::new(),
            runtime_contract: None,
            contract_verification: None,
            use_subagent: false,
            tool_calls: Vec::new(),
        }
    }

    fn sample_goal() -> GoalNode {
        GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "goal".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Running,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: Vec::new(),
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
        }
    }

    #[test]
    fn step_assessment_marks_waiting_external_from_intervention() {
        let step = sample_step();
        let pending = MissionMonitorIntervention {
            action: "mark_waiting_external".to_string(),
            feedback: Some("waiting for remote service".to_string()),
            semantic_tags: vec!["waiting_external".to_string()],
            observed_evidence: vec!["service still starting".to_string()],
            missing_core_deliverables: Vec::new(),
            confidence: None,
            strategy_patch: None,
            subagent_recommended: None,
            parallelism_budget: None,
            requested_at: None,
            applied_at: None,
        };

        let assessment = assess_step_snapshot(&step, Some(&pending), None);

        assert_eq!(
            assessment.status_assessment.as_deref(),
            Some("waiting_external")
        );
        assert!(!assessment.evidence_sufficient);
    }

    #[test]
    fn step_assessment_detects_evidence_sufficient_from_bundle() {
        let mut step = sample_step();
        step.output_summary = Some("已有交付摘要".to_string());
        step.evidence_bundle = Some(StepEvidenceBundle {
            artifact_paths: vec!["deliverable/index.md".to_string()],
            latest_summary: Some("deliverable ready".to_string()),
            ..Default::default()
        });

        let assessment = assess_step_snapshot(&step, None, None);

        assert!(assessment.evidence_sufficient);
        assert_eq!(
            assessment.status_assessment.as_deref(),
            Some("evidence_sufficient")
        );
        assert!(assessment
            .observed_evidence
            .contains(&"artifact_evidence_present".to_string()));
    }

    #[test]
    fn goal_assessment_marks_blocked_from_pivot_reason() {
        let mut goal = sample_goal();
        goal.pivot_reason = Some("upstream dependency unavailable".to_string());

        let assessment = assess_goal_snapshot(&goal, None, None);

        assert_eq!(assessment.status_assessment.as_deref(), Some("blocked"));
        assert_eq!(
            assessment.risk_summary.as_deref(),
            Some("upstream dependency unavailable")
        );
    }

    #[test]
    fn normalize_monitor_action_accepts_terminal_monitor_actions() {
        assert_eq!(
            normalize_monitor_action("blocked_by_environment").as_deref(),
            Some("blocked_by_environment")
        );
        assert_eq!(
            normalize_monitor_action("partial_handoff").as_deref(),
            Some("partial_handoff")
        );
    }
}
