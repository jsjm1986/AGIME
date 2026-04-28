use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::Value;

use super::agent_task_v4_runner::AgentTaskV4Runner;
use super::chat_channels::ChatChannelService;
use super::service_mongo::{AgentService, ExecutionSlotAcquireOutcome};
use super::task_manager::{StreamEvent, TaskManager};
use agime_team::models::AgentTask;
use agime_team::MongoDb;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskAdmissionOutcome {
    Started,
    Queued,
}

pub async fn admit_or_queue_task(
    db: &Arc<MongoDb>,
    agent_service: &Arc<AgentService>,
    task_manager: &Arc<TaskManager>,
    workspace_root: &str,
    task_id: &str,
) -> Result<TaskAdmissionOutcome> {
    let task = agent_service
        .get_task(task_id)
        .await?
        .ok_or_else(|| anyhow!("Task {} not found", task_id))?;

    match agent_service
        .try_acquire_execution_slot(&task.agent_id)
        .await?
    {
        ExecutionSlotAcquireOutcome::Acquired => {
            spawn_task_runner(
                db.clone(),
                agent_service.clone(),
                task_manager.clone(),
                workspace_root.to_string(),
                task,
            );
            Ok(TaskAdmissionOutcome::Started)
        }
        ExecutionSlotAcquireOutcome::Saturated => {
            let task = agent_service
                .mark_task_queued(task_id)
                .await?
                .ok_or_else(|| anyhow!("Task {} cannot be queued", task_id))?;
            apply_task_surface_state(db, agent_service, &task, "queued").await;
            task_manager
                .broadcast(
                    task_id,
                    StreamEvent::Status {
                        status: "queued".to_string(),
                    },
                )
                .await;
            Ok(TaskAdmissionOutcome::Queued)
        }
    }
}

pub async fn start_next_queued_tasks_for_agent(
    db: &Arc<MongoDb>,
    agent_service: &Arc<AgentService>,
    task_manager: &Arc<TaskManager>,
    workspace_root: &str,
    agent_id: &str,
) -> Result<()> {
    loop {
        match agent_service.try_acquire_execution_slot(agent_id).await? {
            ExecutionSlotAcquireOutcome::Saturated => break,
            ExecutionSlotAcquireOutcome::Acquired => {}
        }

        let Some(task) = agent_service
            .claim_next_queued_task_for_agent(agent_id)
            .await?
        else {
            agent_service.release_execution_slot(agent_id).await?;
            break;
        };

        spawn_task_runner(
            db.clone(),
            agent_service.clone(),
            task_manager.clone(),
            workspace_root.to_string(),
            task,
        );
    }

    Ok(())
}

fn spawn_task_runner(
    db: Arc<MongoDb>,
    agent_service: Arc<AgentService>,
    task_manager: Arc<TaskManager>,
    workspace_root: String,
    task: AgentTask,
) {
    let task_id = task.id.clone();
    let agent_id = task.agent_id.clone();
    tokio::spawn(async move {
        let (cancel_token, _) = task_manager.register(&task_id).await;
        apply_task_surface_state(&db, &agent_service, &task, "running").await;
        let runner = AgentTaskV4Runner::new(
            db.clone(),
            agent_service.clone(),
            task_manager.clone(),
            workspace_root.clone(),
        );
        if let Err(error) = runner.execute_task(&task_id, cancel_token).await {
            tracing::error!("Task execution failed: {}", error);
            match agent_service.fail_task(&task_id, &error.to_string()).await {
                Ok(None) => tracing::warn!(
                    "fail_task: no update for task {} (already terminal?)",
                    task_id
                ),
                Err(db_error) => {
                    tracing::error!("Failed to update task status to failed: {}", db_error)
                }
                _ => {}
            }
            task_manager
                .broadcast(
                    &task_id,
                    StreamEvent::Done {
                        status: "failed".to_string(),
                        error: Some(error.to_string()),
                    },
                )
                .await;
        }

        task_manager.complete(&task_id).await;
        if let Err(error) = agent_service.release_execution_slot(&agent_id).await {
            tracing::warn!(
                "Failed to release execution slot for agent {} after task {}: {}",
                agent_id,
                task_id,
                error
            );
        }
        if let Err(error) =
            start_next_queued_tasks_for_agent(
                &db,
                &agent_service,
                &task_manager,
                &workspace_root,
                &agent_id,
            )
            .await
        {
            tracing::warn!(
                "Failed to dispatch queued work for agent {} after task {}: {}",
                agent_id,
                task_id,
                error
            );
        }
    });
}

async fn apply_task_surface_state(
    db: &Arc<MongoDb>,
    agent_service: &Arc<AgentService>,
    task: &AgentTask,
    status: &str,
) {
    let Some(session_id) = task
        .content
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };

    if let Err(error) = agent_service
        .set_session_execution_state(session_id, status, true)
        .await
    {
        tracing::debug!(
            "Failed to apply session execution state {} for {}: {}",
            status,
            session_id,
            error
        );
    }

    let Some(channel_id) = task
        .content
        .get("channel_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    let run_scope_id = task
        .content
        .get("run_scope_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("__channel_root__");
    let channel_service = ChatChannelService::new(db.clone());
    if let Err(error) = channel_service
        .set_run_state(channel_id, run_scope_id, status, true)
        .await
    {
        tracing::debug!(
            "Failed to apply channel execution state {} for {}:{}: {}",
            status,
            channel_id,
            run_scope_id,
            error
        );
    }
}
