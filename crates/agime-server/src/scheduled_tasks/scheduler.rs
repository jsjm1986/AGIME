//! Background scheduler for scheduled tasks.
//!
//! Runs a tick loop that claims due tasks via lease-based concurrency
//! and spawns async task runs. Self-evaluation loop retries on score < 75.

use crate::scheduled_tasks::models::{
    infer_task_profile_from_prompt, ScheduledTaskDoc, ScheduledTaskExecutionContract,
    ScheduledTaskKind, ScheduledTaskPayloadKind, ScheduledTaskProfile,
    ScheduledTaskRunDoc, ScheduledTaskRunOutcomeReason, ScheduledTaskRunStatus,
    ScheduledTaskScheduleSpecKind, ScheduledTaskSelfEvaluation,
    ScheduledTaskSelfEvaluationGrade,
};
use crate::scheduled_tasks::service::{ScheduledTaskService, TaskRunResult};
use anyhow::Result;
use chrono::{DateTime, Utc};
use croner::Cron;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Schedule computation
// ---------------------------------------------------------------------------

/// Compute the next fire time for a task given its schedule spec.
pub fn compute_next_fire_at(
    task: &ScheduledTaskDoc,
    timezone: &str,
) -> Option<DateTime<Utc>> {
    let now = Utc::now();
    match task.task_kind {
        ScheduledTaskKind::OneShot => task.one_shot_at.filter(|t| *t > now),
        ScheduledTaskKind::Cron => {
            let cron_expr = match task.cron_expression.as_deref() {
                Some(e) => e,
                None => return None,
            };
            let cron = match Cron::new(cron_expr) {
                Ok(c) => c,
                Err(_) => return None,
            };
            cron.get_next_run_time_from(
                &now.to_rfc3339(),
                croner::Timezone::TimezoneString(timezone.to_string()),
            )
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc))
        }
    }
}

// ---------------------------------------------------------------------------
// Self-evaluation
// ---------------------------------------------------------------------------

fn grade_from_score(score: i32) -> ScheduledTaskSelfEvaluationGrade {
    match score {
        90..=100 => ScheduledTaskSelfEvaluationGrade::Excellent,
        75..=89 => ScheduledTaskSelfEvaluationGrade::Good,
        60..=74 => ScheduledTaskSelfEvaluationGrade::Acceptable,
        40..=59 => ScheduledTaskSelfEvaluationGrade::Weak,
        _ => ScheduledTaskSelfEvaluationGrade::Failed,
    }
}

// ---------------------------------------------------------------------------
// Run completion classification
// ---------------------------------------------------------------------------

fn classify_run_completion(
    status: ScheduledTaskRunStatus,
    self_eval: Option<&ScheduledTaskSelfEvaluation>,
    error: Option<&str>,
) -> ScheduledTaskRunOutcomeReason {
    match status {
        ScheduledTaskRunStatus::Completed => {
            if error.is_some() {
                ScheduledTaskRunOutcomeReason::CompletedWithWarnings
            } else if self_eval
                .map(|e| e.grade == ScheduledTaskSelfEvaluationGrade::Excellent)
                .unwrap_or(false)
            {
                ScheduledTaskRunOutcomeReason::Completed
            } else if self_eval
                .map(|e| matches!(e.grade,
                    ScheduledTaskSelfEvaluationGrade::Good
                    | ScheduledTaskSelfEvaluationGrade::Acceptable))
                .unwrap_or(false)
            {
                ScheduledTaskRunOutcomeReason::Completed
            } else {
                ScheduledTaskRunOutcomeReason::CompletedWithWarnings
            }
        }
        ScheduledTaskRunStatus::Failed => {
            if error.map(|e| e.contains("contract")).unwrap_or(false) {
                ScheduledTaskRunOutcomeReason::FailedContractViolation
            } else if error.map(|e| e.contains("capability")).unwrap_or(false) {
                ScheduledTaskRunOutcomeReason::BlockedCapabilityPolicy
            } else {
                ScheduledTaskRunOutcomeReason::FailedNoFinalAnswer
            }
        }
        ScheduledTaskRunStatus::Cancelled => ScheduledTaskRunOutcomeReason::Cancelled,
        _ => ScheduledTaskRunOutcomeReason::Completed,
    }
}

// ---------------------------------------------------------------------------
// Scheduler loop
// ---------------------------------------------------------------------------

const DEFAULT_LEASE_SECS: i64 = 120;
const DEFAULT_TICK_INTERVAL_SECS: u64 = 5;
const DEFAULT_GRACE_SECS: i64 = 5;
const SELF_EVALUATION_THRESHOLD: i32 = 75;
const MAX_IMPROVEMENT_LOOPS: i32 = 3;

/// Spawn the background scheduler loop.
/// Returns a `JoinHandle` so the caller can manage lifecycle.
pub fn spawn_scheduler_loop(
    service: Arc<ScheduledTaskService>,
    timezone: String,
) -> tokio::task::JoinHandle<()> {
    let lease_owner = format!("scheduler-{}", Uuid::new_v4());
    tokio::spawn(async move {
        let tick_interval = Duration::from_secs(DEFAULT_TICK_INTERVAL_SECS);
        loop {
            tokio::time::sleep(tick_interval).await;
            let due = match service.claim_due_tasks(&lease_owner, DEFAULT_LEASE_SECS) {
                Ok(tasks) => tasks,
                Err(e) => {
                    eprintln!("scheduler: claim_due_tasks error: {}", e);
                    continue;
                }
            };
            for task in due {
                let svc = service.clone();
                let tz = timezone.clone();
                tokio::spawn(async move {
                    if let Err(e) = run_task(svc, task, tz).await {
                        eprintln!("scheduler: run_task error: {}", e);
                    }
                });
            }
        }
    })
}

/// Run a single task: execute, self-evaluate, retry if needed, record.
pub async fn run_task(
    service: Arc<ScheduledTaskService>,
    task: ScheduledTaskDoc,
    timezone: String,
) -> Result<()> {
    let task_id = task.task_id.clone();
    let agent_id = task.agent_id.clone();
    let prompt = task.prompt.clone();
    let exec_contract = task.execution_contract.clone();

    // Create run record
    let run_id = Uuid::new_v4().to_string();
    let run = ScheduledTaskRunDoc {
        run_id: run_id.clone(),
        task_id: task_id.clone(),
        runtime_session_id: None,
        fire_message_id: None,
        status: ScheduledTaskRunStatus::Running,
        outcome_reason: None,
        warning_count: 0,
        summary: None,
        error: None,
        self_evaluation: None,
        initial_self_evaluation: None,
        improvement_loop_applied: false,
        improvement_loop_count: 0,
        trigger_source: "scheduler".to_string(),
        started_at: Utc::now(),
        finished_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let run = service.create_run(run)?;

    // Execute task (placeholder — desktop runtime integration point)
    let result = execute_task_run(&agent_id, &prompt, &exec_contract).await;

    // Determine outcome
    let (status, error, summary, self_eval, warnings) = match result {
        Ok(result) => {
            let mut warnings = 0;
            let self_eval = result.self_evaluation.clone();
            let score = self_eval.as_ref().map(|e| e.score).unwrap_or(100);
            if score < SELF_EVALUATION_THRESHOLD {
                warnings = 1;
            }
            (
                ScheduledTaskRunStatus::Completed,
                result.error,
                result.summary,
                self_eval,
                warnings,
            )
        }
        Err(e) => {
            let err_msg = e.to_string();
            (ScheduledTaskRunStatus::Failed, Some(err_msg), None, None, 0)
        }
    };

    // Self-evaluation retry loop
    let mut final_status = status;
    let mut final_summary = summary;
    let mut final_error = error;
    let mut final_self_eval = self_eval;
    let mut improvement_count = 0;
    let mut run_doc = run;

    while final_self_eval
        .as_ref()
        .map(|e| e.score < SELF_EVALUATION_THRESHOLD)
        .unwrap_or(false)
        && improvement_count < MAX_IMPROVEMENT_LOOPS
    {
        improvement_count += 1;
        run_doc.improvement_loop_applied = true;
        run_doc.improvement_loop_count = improvement_count;

        // Retry execution with feedback
        let retry_result = retry_with_feedback(
            &agent_id,
            &prompt,
            &exec_contract,
            final_self_eval.as_ref(),
        )
        .await;

        match retry_result {
            Ok(retry_outcome) => {
                if let Some(ref eval) = retry_outcome.self_evaluation {
                    if eval.score >= SELF_EVALUATION_THRESHOLD {
                        final_status = ScheduledTaskRunStatus::Completed;
                        final_summary = retry_outcome.summary;
                        final_error = retry_outcome.error;
                        final_self_eval = retry_outcome.self_evaluation;
                        break;
                    }
                }
                final_summary = retry_outcome.summary;
                final_error = retry_outcome.error;
                final_self_eval = retry_outcome.self_evaluation;
            }
            Err(e) => {
                final_status = ScheduledTaskRunStatus::Failed;
                final_error = Some(e.to_string());
                break;
            }
        }
    }

    // Update run record
    let outcome_reason = Some(classify_run_completion(
        final_status,
        final_self_eval.as_ref(),
        final_error.as_deref(),
    ));
    let mut final_run = run_doc;
    final_run.status = final_status;
    final_run.summary = final_summary.clone();
    final_run.error = final_error.clone();
    final_run.self_evaluation = final_self_eval.clone();
    final_run.warning_count = warnings;
    final_run.outcome_reason = outcome_reason;
    final_run.finished_at = Some(Utc::now());
    final_run.updated_at = Utc::now();

    service
        .complete_run(&run_id, final_run.status, final_run.outcome_reason)
        .ok();

    // Compute next fire time and record
    let next_fire = compute_next_fire_at(&task, &timezone);
    service.record_task_run(&task_id, run_id, next_fire, false)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Execution (desktop runtime integration point)
// ---------------------------------------------------------------------------

struct TaskExecutionOutcome {
    summary: Option<String>,
    error: Option<String>,
    self_evaluation: Option<ScheduledTaskSelfEvaluation>,
}

/// Execute the task — desktop runtime integration point.
/// In the full implementation this creates a runtime session and runs the LLM turn.
async fn execute_task_run(
    _agent_id: &str,
    _prompt: &str,
    _contract: &ScheduledTaskExecutionContract,
) -> Result<TaskExecutionOutcome> {
    // TODO: integrate with desktop runtime session creation
    // This would:
    // 1. Create a runtime session via the desktop harness
    // 2. Submit the prompt with execution contract constraints
    // 3. Collect the response and self-evaluation
    Ok(TaskExecutionOutcome {
        summary: Some("Task executed (placeholder)".to_string()),
        error: None,
        self_evaluation: Some(ScheduledTaskSelfEvaluation {
            score: 85,
            grade: ScheduledTaskSelfEvaluationGrade::Good,
            goal_completion: 80,
            result_quality: 85,
            evidence_quality: 80,
            execution_stability: 90,
            contract_compliance: 90,
            summary: "Placeholder execution completed successfully".to_string(),
            completed_steps: vec![],
            failed_steps: vec![],
            risks: vec![],
            confidence: 0.85,
        }),
    })
}

/// Retry with self-evaluation feedback.
async fn retry_with_feedback(
    _agent_id: &str,
    _prompt: &str,
    _contract: &ScheduledTaskExecutionContract,
    _prior_eval: Option<&ScheduledTaskSelfEvaluation>,
) -> Result<TaskExecutionOutcome> {
    // TODO: integrate with desktop runtime for retry execution
    Ok(TaskExecutionOutcome {
        summary: Some("Retry executed (placeholder)".to_string()),
        error: None,
        self_evaluation: Some(ScheduledTaskSelfEvaluation {
            score: 80,
            grade: ScheduledTaskSelfEvaluationGrade::Good,
            goal_completion: 75,
            result_quality: 80,
            evidence_quality: 75,
            execution_stability: 85,
            contract_compliance: 85,
            summary: "Retry completed".to_string(),
            completed_steps: vec![],
            failed_steps: vec![],
            risks: vec![],
            confidence: 0.80,
        }),
    })
}

// ---------------------------------------------------------------------------
// Manual run trigger
// ---------------------------------------------------------------------------

/// Trigger a task run immediately (bypass scheduler).
pub async fn trigger_run_now(
    service: Arc<ScheduledTaskService>,
    task: ScheduledTaskDoc,
    timezone: String,
) -> Result<ScheduledTaskRunDoc> {
    let task_id = task.task_id.clone();
    let run_id = Uuid::new_v4().to_string();
    let run = ScheduledTaskRunDoc {
        run_id: run_id.clone(),
        task_id: task_id.clone(),
        runtime_session_id: None,
        fire_message_id: None,
        status: ScheduledTaskRunStatus::Running,
        outcome_reason: None,
        warning_count: 0,
        summary: None,
        error: None,
        self_evaluation: None,
        initial_self_evaluation: None,
        improvement_loop_applied: false,
        improvement_loop_count: 0,
        trigger_source: "manual".to_string(),
        started_at: Utc::now(),
        finished_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let run = service.create_run(run)?;

    let result = execute_task_run(&task.agent_id, &task.prompt, &task.execution_contract).await;

    let (status, error, summary, self_eval) = match result {
        Ok(r) => (
            ScheduledTaskRunStatus::Completed,
            r.error,
            r.summary,
            r.self_evaluation,
        ),
        Err(e) => (ScheduledTaskRunStatus::Failed, Some(e.to_string()), None, None),
    };

    service.complete_run(
        &run_id,
        status,
        Some(classify_run_completion(
            status,
            self_eval.as_ref(),
            error.as_deref(),
        )),
    )?;

    let mut final_run = run;
    final_run.status = status;
    final_run.summary = summary;
    final_run.error = error;
    final_run.self_evaluation = self_eval;
    final_run.finished_at = Some(Utc::now());
    final_run.updated_at = Utc::now();

    Ok(final_run)
}