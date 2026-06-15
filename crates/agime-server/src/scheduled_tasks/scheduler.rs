//! Background scheduler for scheduled tasks.
//!
//! Runs a tick loop that claims due tasks via lease-based concurrency
//! and spawns async task runs.

use crate::scheduled_tasks::models::{
    ScheduledTaskDoc, ScheduledTaskKind, ScheduledTaskRunDoc, ScheduledTaskRunOutcomeReason,
    ScheduledTaskRunStatus,
};
use crate::scheduled_tasks::service::ScheduledTaskService;
use crate::state::AppState;
use agime::agents::{AgentEvent, SessionConfig};
use agime::conversation::message::Message;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use croner::Cron;
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Schedule computation
// ---------------------------------------------------------------------------

/// Compute the next fire time for a task given its schedule spec.
pub fn compute_next_fire_at(task: &ScheduledTaskDoc, timezone: &str) -> Option<DateTime<Utc>> {
    let now = Utc::now();
    match task.task_kind {
        ScheduledTaskKind::OneShot => task.one_shot_at.filter(|t| *t > now),
        ScheduledTaskKind::Cron => {
            let cron_expr = match task.cron_expression.as_deref() {
                Some(e) => e,
                None => return None,
            };
            let tz: chrono_tz::Tz = match timezone.parse() {
                Ok(tz) => tz,
                Err(_) => return None,
            };
            let cron = match Cron::new(cron_expr).parse() {
                Ok(c) => c,
                Err(_) => return None,
            };
            let localized_now = now.with_timezone(&tz);
            cron.find_next_occurrence(&localized_now, false)
                .ok()
                .map(|next| next.with_timezone(&Utc))
        }
    }
}

// ---------------------------------------------------------------------------
// Run completion classification
// ---------------------------------------------------------------------------

fn classify_run_completion(
    status: ScheduledTaskRunStatus,
    error: Option<&str>,
) -> ScheduledTaskRunOutcomeReason {
    match status {
        ScheduledTaskRunStatus::Completed => {
            if error.is_some() {
                ScheduledTaskRunOutcomeReason::CompletedWithWarnings
            } else {
                ScheduledTaskRunOutcomeReason::Completed
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

const DEFAULT_LEASE_SECS: i64 = 600;
const DEFAULT_TICK_INTERVAL_SECS: u64 = 5;

/// Spawn the background scheduler loop.
/// Returns a `JoinHandle` so the caller can manage lifecycle.
pub fn spawn_scheduler_loop(
    state: Arc<AppState>,
    service: Arc<ScheduledTaskService>,
    timezone: String,
) -> tokio::task::JoinHandle<()> {
    let lease_owner = format!("scheduler-{}", Uuid::new_v4());
    tokio::spawn(async move {
        let tick_interval = Duration::from_secs(DEFAULT_TICK_INTERVAL_SECS);
        loop {
            tokio::time::sleep(tick_interval).await;
            // Reconcile orphaned runs first: a run left Running by a crash or a
            // failed finish_run would otherwise block its task from re-claiming.
            if let Err(e) = service.reclaim_stale_runs() {
                eprintln!("scheduler: reclaim_stale_runs error: {}", e);
            }
            let due = match service.claim_due_tasks(&lease_owner, DEFAULT_LEASE_SECS) {
                Ok(tasks) => tasks,
                Err(e) => {
                    eprintln!("scheduler: claim_due_tasks error: {}", e);
                    continue;
                }
            };
            for task in due {
                let st = state.clone();
                let svc = service.clone();
                let tz = timezone.clone();
                let owner = lease_owner.clone();
                tokio::spawn(async move {
                    if let Err(e) = run_task(st, svc, task, tz, owner).await {
                        eprintln!("scheduler: run_task error: {}", e);
                    }
                });
            }
        }
    })
}

/// Run a single task: execute the LLM turn, record the run, advance the
/// schedule (and terminate one-shot tasks so they don't re-fire).
pub async fn run_task(
    state: Arc<AppState>,
    service: Arc<ScheduledTaskService>,
    task: ScheduledTaskDoc,
    timezone: String,
    lease_owner: String,
) -> Result<()> {
    let task_id = task.task_id.clone();
    let agent_id = task.agent_id.clone();
    let prompt = task.prompt.clone();

    // Create run record
    let run_id = Uuid::new_v4().to_string();
    let session_id = format!("scheduled-task::{}::{}", task_id, run_id);
    let run = ScheduledTaskRunDoc {
        run_id: run_id.clone(),
        task_id: task_id.clone(),
        runtime_session_id: Some(session_id.clone()),
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
    service.create_run(run)?;

    // Execute the real LLM turn through the desktop harness.
    let result = execute_task_run(&state, &agent_id, &prompt, &session_id).await;

    let (status, error, summary) = match result {
        Ok(outcome) => (
            ScheduledTaskRunStatus::Completed,
            outcome.error,
            outcome.summary,
        ),
        Err(e) => (ScheduledTaskRunStatus::Failed, Some(e.to_string()), None),
    };

    let outcome_reason = Some(classify_run_completion(status, error.as_deref()));

    // Persist the run terminal state. If this fails the run is still recorded
    // as Running on disk; do NOT advance the schedule, or the task's
    // next_fire_at/lease would move forward while the run record stays
    // inconsistent. The stale-run reclaimer transitions the orphaned Running
    // record to Failed, and the task is re-claimed on a later tick.
    if let Err(e) = service.finish_run(&run_id, status, outcome_reason, summary, error) {
        eprintln!("scheduler: finish_run error for {}: {}", run_id, e);
        return Err(anyhow!("finish_run failed for {}: {}", run_id, e));
    }

    // Advance the schedule. One-shot tasks terminate; cron tasks roll forward.
    match task.task_kind {
        ScheduledTaskKind::OneShot => {
            service.complete_task(&task_id, &run_id, &lease_owner)?;
        }
        ScheduledTaskKind::Cron => {
            // Prefer the task's own timezone; fall back to the scheduler default.
            let tz = if task.timezone.trim().is_empty() {
                timezone.as_str()
            } else {
                task.timezone.as_str()
            };
            let next_fire = compute_next_fire_at(&task, tz);
            if next_fire.is_none() {
                // Cron/timezone failed to parse — pause the task instead of
                // leaving a stale past next_fire_at that re-fires every tick.
                eprintln!(
                    "scheduler: task {} has no computable next fire; pausing",
                    task_id
                );
                service.pause_stalled_task(&task_id, &run_id, &lease_owner)?;
            } else {
                // `missed` is always false: missed-fire detection (comparing the
                // expected vs. actual fire time after downtime/lease backlog) is
                // not wired up yet, so the counter stays at zero.
                service.record_task_run(&task_id, run_id, next_fire, false, &lease_owner)?;
            }
        }
    }

    Ok(())
}
// ---------------------------------------------------------------------------
// Execution (desktop runtime integration)
// ---------------------------------------------------------------------------

struct TaskExecutionOutcome {
    summary: Option<String>,
    error: Option<String>,
}

/// Execute the task by running a real LLM turn through the desktop harness.
///
/// Mirrors [`crate::host_task_manager`]'s background-turn pattern: a dedicated
/// agent keyed by the run's logical session id (so it doesn't contend with the
/// foreground `/reply` session agent), driven by `execute_chat_host_with_mirror`
/// to completion, accumulating the assistant text as the run summary.
async fn execute_task_run(
    state: &Arc<AppState>,
    _agent_id: &str,
    prompt: &str,
    session_id: &str,
) -> Result<TaskExecutionOutcome> {
    let agent = state
        .get_agent(session_id.to_string())
        .await
        .map_err(|e| anyhow!("failed to get agent for scheduled task: {}", e))?;

    let session_config = SessionConfig {
        id: session_id.to_string(),
        schedule_id: None,
        max_turns: None,
        retry_config: None,
    };

    let host = crate::desktop_harness_host::DesktopHarnessHost::new(state.clone());
    let (control_tx, mut control_rx) =
        tokio::sync::mpsc::channel::<agime::agents::HarnessControlEnvelope>(64);
    // Drain control envelopes so the bounded channel never back-pressures the
    // harness; scheduled tasks have no interactive control consumer.
    let control_drain = tokio::spawn(async move { while control_rx.recv().await.is_some() {} });

    let user_message = Message::user().with_text(prompt);

    let stream_result = host
        .execute_chat_host_with_mirror(
            agent.clone(),
            user_message,
            session_config,
            None,
            control_tx,
        )
        .await;

    let mut stream = match stream_result {
        Ok(stream) => stream,
        Err(e) => {
            control_drain.abort();
            let _ = state.agent_manager.remove_session(session_id).await;
            return Err(anyhow!("harness host failed to start: {}", e));
        }
    };

    let mut last_assistant_text: Option<String> = None;
    let mut turn_error: Option<String> = None;
    while let Some(next) = stream.next().await {
        match next {
            Ok(AgentEvent::Message(message)) => {
                if message.role == rmcp::model::Role::Assistant {
                    let text = message.as_concat_text();
                    if !text.trim().is_empty() {
                        last_assistant_text = Some(text);
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                turn_error = Some(e.to_string());
                break;
            }
        }
    }

    control_drain.abort();
    let _ = state.agent_manager.remove_session(session_id).await;

    if let Some(err) = turn_error {
        return Err(anyhow!(err));
    }

    Ok(TaskExecutionOutcome {
        summary: last_assistant_text.map(|t| t.chars().take(2000).collect()),
        error: None,
    })
}

// ---------------------------------------------------------------------------
// Manual run trigger
// ---------------------------------------------------------------------------

/// Trigger a task run immediately (bypass scheduler). Does not touch the
/// task's schedule/lease — manual runs are independent of the fire cycle.
pub async fn trigger_run_now(
    state: Arc<AppState>,
    service: Arc<ScheduledTaskService>,
    task: ScheduledTaskDoc,
) -> Result<ScheduledTaskRunDoc> {
    let task_id = task.task_id.clone();
    let run_id = Uuid::new_v4().to_string();
    let session_id = format!("scheduled-task::{}::{}", task_id, run_id);
    let run = ScheduledTaskRunDoc {
        run_id: run_id.clone(),
        task_id: task_id.clone(),
        runtime_session_id: Some(session_id.clone()),
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

    let result = execute_task_run(&state, &task.agent_id, &task.prompt, &session_id).await;

    let (status, error, summary) = match result {
        Ok(r) => (ScheduledTaskRunStatus::Completed, r.error, r.summary),
        Err(e) => (ScheduledTaskRunStatus::Failed, Some(e.to_string()), None),
    };

    let outcome_reason = Some(classify_run_completion(status, error.as_deref()));
    service.finish_run(
        &run_id,
        status,
        outcome_reason,
        summary.clone(),
        error.clone(),
    )?;

    let mut final_run = run;
    final_run.status = status;
    final_run.summary = summary;
    final_run.error = error;
    final_run.outcome_reason = outcome_reason;
    final_run.finished_at = Some(Utc::now());
    final_run.updated_at = Utc::now();

    Ok(final_run)
}
