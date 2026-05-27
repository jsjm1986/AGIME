//! Execution-admission control (team-server side).
//!
//! Wraps the generic [`agime_runtime::execution_admission`] flow with adapters
//! around `AgentService`, `ChatChannelService`, the Mongo task collection and
//! `TaskManager` so existing call sites keep their `(&Arc<MongoDb>,
//! &Arc<AgentService>, &Arc<TaskManager>, &str, &str)` signatures unchanged.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;

use agime_runtime::admission::{
    ExecutionSlotAcquireOutcome as RtExecutionSlotAcquireOutcome, ExecutionSlotProvider,
};
use agime_runtime::execution_admission::{
    self, QueuedTaskBroadcaster, QueuedTaskRef, TaskQueueRepository, TaskRuntimeSpawner,
    TaskSurfacePropagator,
};

use super::agent_task_v4_runner::AgentTaskV4Runner;
use super::chat_channels::ChatChannelService;
use super::service_mongo::{AgentService, ExecutionSlotAcquireOutcome};
use super::task_manager::{StreamEvent, TaskManager};
use agime_team::models::AgentTask;
use agime_team::MongoDb;

pub use agime_runtime::execution_admission::TaskAdmissionOutcome;

struct AgentServiceSlotAdapter<'a> {
    agent_service: &'a AgentService,
}

#[async_trait]
impl<'a> ExecutionSlotProvider for AgentServiceSlotAdapter<'a> {
    async fn try_acquire_execution_slot(
        &self,
        agent_id: &str,
    ) -> Result<RtExecutionSlotAcquireOutcome> {
        match self
            .agent_service
            .try_acquire_execution_slot(agent_id)
            .await
            .map_err(|error| anyhow!("{}", error))?
        {
            ExecutionSlotAcquireOutcome::Acquired => Ok(RtExecutionSlotAcquireOutcome::Acquired),
            ExecutionSlotAcquireOutcome::Saturated => Ok(RtExecutionSlotAcquireOutcome::Saturated),
        }
    }

    async fn release_execution_slot(&self, agent_id: &str) -> Result<()> {
        self.agent_service
            .release_execution_slot(agent_id)
            .await
            .map_err(|error| anyhow!("{}", error))
    }
}

struct AgentServiceQueueAdapter<'a> {
    agent_service: &'a AgentService,
}

fn task_to_ref(task: &AgentTask) -> QueuedTaskRef {
    QueuedTaskRef {
        task_id: task.id.clone(),
        agent_id: task.agent_id.clone(),
    }
}

#[async_trait]
impl<'a> TaskQueueRepository for AgentServiceQueueAdapter<'a> {
    async fn get_task_ref(&self, task_id: &str) -> Result<Option<QueuedTaskRef>> {
        Ok(self
            .agent_service
            .get_task(task_id)
            .await?
            .as_ref()
            .map(task_to_ref))
    }

    async fn mark_task_queued(&self, task_id: &str) -> Result<Option<QueuedTaskRef>> {
        Ok(self
            .agent_service
            .mark_task_queued(task_id)
            .await?
            .as_ref()
            .map(task_to_ref))
    }

    async fn claim_next_queued_task_for_agent(
        &self,
        agent_id: &str,
    ) -> Result<Option<QueuedTaskRef>> {
        Ok(self
            .agent_service
            .claim_next_queued_task_for_agent(agent_id)
            .await?
            .as_ref()
            .map(task_to_ref))
    }

    async fn list_agents_with_queued_tasks(&self) -> Result<Vec<String>> {
        Ok(self
            .agent_service
            .list_agents_with_queued_tasks()
            .await
            .map_err(|error| anyhow!("{}", error))?)
    }
}

struct SurfacePropagatorAdapter<'a> {
    db: &'a Arc<MongoDb>,
    agent_service: &'a Arc<AgentService>,
}

impl<'a> SurfacePropagatorAdapter<'a> {
    async fn propagate(&self, task_id: &str, status: &str) {
        let Ok(Some(task)) = self.agent_service.get_task(task_id).await else {
            return;
        };
        apply_task_surface_state(self.db, self.agent_service, &task, status).await;
    }
}

#[async_trait]
impl<'a> TaskSurfacePropagator for SurfacePropagatorAdapter<'a> {
    async fn apply_task_surface_state(&self, task_id: &str, status: &str) {
        self.propagate(task_id, status).await;
    }
}

struct QueuedBroadcasterAdapter<'a> {
    task_manager: &'a TaskManager,
}

#[async_trait]
impl<'a> QueuedTaskBroadcaster for QueuedBroadcasterAdapter<'a> {
    async fn broadcast_queued(&self, task_id: &str) {
        self.task_manager
            .broadcast(
                task_id,
                StreamEvent::Status {
                    status: "queued".to_string(),
                },
            )
            .await;
    }
}

struct AgentTaskSpawnerAdapter {
    db: Arc<MongoDb>,
    agent_service: Arc<AgentService>,
    task_manager: Arc<TaskManager>,
    workspace_root: String,
}

#[async_trait]
impl TaskRuntimeSpawner for AgentTaskSpawnerAdapter {
    async fn spawn_task_runner(&self, task: QueuedTaskRef) {
        let Ok(Some(full_task)) = self.agent_service.get_task(&task.task_id).await else {
            tracing::warn!(
                "spawn_task_runner: task {} disappeared before spawn",
                task.task_id
            );
            return;
        };
        spawn_task_runner(
            self.db.clone(),
            self.agent_service.clone(),
            self.task_manager.clone(),
            self.workspace_root.clone(),
            full_task,
        );
    }
}

pub async fn admit_or_queue_task(
    db: &Arc<MongoDb>,
    agent_service: &Arc<AgentService>,
    task_manager: &Arc<TaskManager>,
    workspace_root: &str,
    task_id: &str,
) -> Result<TaskAdmissionOutcome> {
    let slots = AgentServiceSlotAdapter { agent_service };
    let queue = AgentServiceQueueAdapter { agent_service };
    let propagator = SurfacePropagatorAdapter { db, agent_service };
    let broadcaster = QueuedBroadcasterAdapter { task_manager };
    let spawner = AgentTaskSpawnerAdapter {
        db: db.clone(),
        agent_service: agent_service.clone(),
        task_manager: task_manager.clone(),
        workspace_root: workspace_root.to_string(),
    };
    execution_admission::admit_or_queue_task(
        &slots,
        &queue,
        &propagator,
        &broadcaster,
        &spawner,
        task_id,
    )
    .await
}

pub async fn start_next_queued_tasks_for_agent(
    db: &Arc<MongoDb>,
    agent_service: &Arc<AgentService>,
    task_manager: &Arc<TaskManager>,
    workspace_root: &str,
    agent_id: &str,
) -> Result<()> {
    let slots = AgentServiceSlotAdapter { agent_service };
    let queue = AgentServiceQueueAdapter { agent_service };
    let spawner = AgentTaskSpawnerAdapter {
        db: db.clone(),
        agent_service: agent_service.clone(),
        task_manager: task_manager.clone(),
        workspace_root: workspace_root.to_string(),
    };
    execution_admission::start_next_queued_tasks_for_agent(&slots, &queue, &spawner, agent_id).await
}

pub async fn resume_queued_tasks(
    db: &Arc<MongoDb>,
    agent_service: &Arc<AgentService>,
    task_manager: &Arc<TaskManager>,
    workspace_root: &str,
) -> Result<usize> {
    let slots = AgentServiceSlotAdapter { agent_service };
    let queue = AgentServiceQueueAdapter { agent_service };
    let spawner = AgentTaskSpawnerAdapter {
        db: db.clone(),
        agent_service: agent_service.clone(),
        task_manager: task_manager.clone(),
        workspace_root: workspace_root.to_string(),
    };
    execution_admission::resume_queued_tasks(&slots, &queue, &spawner).await
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
        let cancel_token_for_result = cancel_token.clone();
        apply_task_surface_state(&db, &agent_service, &task, "running").await;
        let runner = AgentTaskV4Runner::new(
            db.clone(),
            agent_service.clone(),
            task_manager.clone(),
            workspace_root.clone(),
        );
        if let Err(error) = runner.execute_task(&task_id, cancel_token).await {
            tracing::error!("Task execution failed: {}", error);
            if cancel_token_for_result.is_cancelled() {
                task_manager
                    .broadcast(
                        &task_id,
                        StreamEvent::Done {
                            status: "cancelled".to_string(),
                            error: None,
                        },
                    )
                    .await;
            } else {
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
        if let Err(error) = start_next_queued_tasks_for_agent(
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
