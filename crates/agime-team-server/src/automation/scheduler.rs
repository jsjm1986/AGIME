use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use croner::Cron;

use agime_team::MongoDb;

use crate::agent::ChatManager;

use super::models::{AutomationScheduleDoc, ScheduleMode, ScheduleStatus};
use super::runner::AutomationRunner;
use super::service::AutomationService;

pub fn compute_next_run_at(
    schedule: &AutomationScheduleDoc,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>> {
    match schedule.mode {
        ScheduleMode::Schedule => {
            let expr = schedule
                .cron_expression
                .as_deref()
                .ok_or_else(|| anyhow!("Missing cron expression"))?;
            let cron = Cron::new(expr)
                .parse()
                .map_err(|e| anyhow!("Invalid cron expression: {}", e))?;
            let next = cron
                .find_next_occurrence(&now, false)
                .map_err(|e| anyhow!("Failed to calculate next occurrence: {}", e))?;
            Ok(Some(next.with_timezone(&Utc)))
        }
        ScheduleMode::Monitor => {
            let interval = schedule.poll_interval_seconds.unwrap_or(300).max(30);
            Ok(Some(now + chrono::Duration::seconds(interval)))
        }
    }
}

pub fn spawn_scheduler(db: Arc<MongoDb>, chat_manager: Arc<ChatManager>, workspace_root: String) {
    tokio::spawn(async move {
        tracing::info!("automation scheduler: started");
        let service = AutomationService::new(db.clone());
        let runner = AutomationRunner::new(db.clone(), chat_manager.clone(), workspace_root);
        let tick = Duration::from_secs(30);
        loop {
            tokio::time::sleep(tick).await;
            let now = Utc::now();
            let due = match service.claim_due_schedules(now).await {
                Ok(items) => items,
                Err(error) => {
                    tracing::error!(
                        "automation scheduler: failed to claim due schedules: {}",
                        error
                    );
                    continue;
                }
            };
            tracing::info!(
                "automation scheduler: tick at {} claimed {} due schedule(s)",
                now.to_rfc3339(),
                due.len()
            );
            for schedule in due {
                if schedule.status != ScheduleStatus::Active {
                    continue;
                }
                match service
                    .get_module(&schedule.team_id, &schedule.module_id)
                    .await
                {
                    Ok(Some(module)) => {
                        match runner
                            .start_module_run(
                                &schedule.team_id,
                                &module,
                                &schedule.created_by,
                                super::models::RunMode::from(schedule.mode.clone()),
                                Some(schedule.schedule_id.clone()),
                                Some(schedule.created_by.clone()),
                            )
                            .await
                        {
                            Ok(run) => {
                                let _ = service
                                    .mark_schedule_run(
                                        &schedule.team_id,
                                        &schedule.schedule_id,
                                        &run.run_id,
                                        schedule.last_run_at,
                                        schedule.next_run_at,
                                    )
                                    .await;
                            }
                            Err(error) => {
                                tracing::error!(
                                    "automation scheduler: failed to start run for schedule {}: {}",
                                    schedule.schedule_id,
                                    error
                                );
                            }
                        }
                    }
                    Ok(None) => {
                        tracing::warn!(
                            "automation scheduler: module {} missing for schedule {}",
                            schedule.module_id,
                            schedule.schedule_id
                        );
                    }
                    Err(error) => {
                        tracing::error!(
                            "automation scheduler: failed to load module {}: {}",
                            schedule.module_id,
                            error
                        );
                    }
                }
            }
        }
    });
}
