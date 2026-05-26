//! Generic execution-admission control.
//!
//! Migrated from `agime-team-server::agent::execution_admission`. The runtime
//! exposes the admit/queue/resume control flow as generic functions over four
//! small traits:
//!
//! * [`ExecutionSlotProvider`] — atomic slot acquire/release (already in
//!   [`crate::admission`]). Reused here.
//! * [`TaskQueueRepository`] — persistent queue inspection/transition.
//! * [`TaskSurfacePropagator`] — fan out task status to session/channel
//!   surfaces (chat session execution state, channel run-state, etc.).
//! * [`TaskRuntimeSpawner`] — spawn the actual task runner (kept on the
//!   team-server side until batch 7 moves the runner itself).
//!
//! Team-server wraps these with adapters around `AgentService`, the Mongo
//! task collection and `ChatChannelService`. Desktop server can implement
//! them with an in-memory queue.

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use crate::admission::{ExecutionSlotAcquireOutcome, ExecutionSlotProvider};

/// Outcome of admitting a single task into execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskAdmissionOutcome {
    Started,
    Queued,
}

/// Minimal task identity needed to spawn a runner: the task itself plus the
/// agent that owns its execution slot.
///
/// We deliberately do not surface the full `AgentTask` document in the
/// runtime crate — runners on each side know how to look up their own
/// concrete task record from `task_id`.
#[derive(Debug, Clone)]
pub struct QueuedTaskRef {
    pub task_id: String,
    pub agent_id: String,
}

/// Persistent queue operations the admission flow needs.
#[async_trait]
pub trait TaskQueueRepository: Send + Sync {
    async fn get_task_ref(&self, task_id: &str) -> Result<Option<QueuedTaskRef>>;

    async fn mark_task_queued(&self, task_id: &str) -> Result<Option<QueuedTaskRef>>;

    async fn claim_next_queued_task_for_agent(
        &self,
        agent_id: &str,
    ) -> Result<Option<QueuedTaskRef>>;

    async fn list_agents_with_queued_tasks(&self) -> Result<Vec<String>>;
}

/// Fan task status changes out to session/channel surfaces.
#[async_trait]
pub trait TaskSurfacePropagator: Send + Sync {
    async fn apply_task_surface_state(&self, task_id: &str, status: &str);
}

/// Spawn the concrete runner for a claimed task.
///
/// Returning is non-blocking — the implementation is expected to do its own
/// `tokio::spawn` and immediately yield.
#[async_trait]
pub trait TaskRuntimeSpawner: Send + Sync {
    async fn spawn_task_runner(&self, task: QueuedTaskRef);
}

/// Broadcast a queued-status event for `task_id`.
#[async_trait]
pub trait QueuedTaskBroadcaster: Send + Sync {
    async fn broadcast_queued(&self, task_id: &str);
}

/// Try to admit a task into execution; queue it if all execution slots for
/// its agent are saturated.
pub async fn admit_or_queue_task<S, Q, P, B, R>(
    slots: &S,
    queue: &Q,
    propagator: &P,
    broadcaster: &B,
    spawner: &R,
    task_id: &str,
) -> Result<TaskAdmissionOutcome>
where
    S: ExecutionSlotProvider + ?Sized,
    Q: TaskQueueRepository + ?Sized,
    P: TaskSurfacePropagator + ?Sized,
    B: QueuedTaskBroadcaster + ?Sized,
    R: TaskRuntimeSpawner + ?Sized,
{
    let task = queue
        .get_task_ref(task_id)
        .await?
        .ok_or_else(|| anyhow!("Task {} not found", task_id))?;

    match slots.try_acquire_execution_slot(&task.agent_id).await? {
        ExecutionSlotAcquireOutcome::Acquired => {
            spawner.spawn_task_runner(task).await;
            Ok(TaskAdmissionOutcome::Started)
        }
        ExecutionSlotAcquireOutcome::Saturated => {
            let task = queue
                .mark_task_queued(task_id)
                .await?
                .ok_or_else(|| anyhow!("Task {} cannot be queued", task_id))?;
            propagator
                .apply_task_surface_state(&task.task_id, "queued")
                .await;
            broadcaster.broadcast_queued(&task.task_id).await;
            Ok(TaskAdmissionOutcome::Queued)
        }
    }
}

/// Drain queued tasks for `agent_id` until either the queue empties or the
/// agent's execution slots saturate.
pub async fn start_next_queued_tasks_for_agent<S, Q, R>(
    slots: &S,
    queue: &Q,
    spawner: &R,
    agent_id: &str,
) -> Result<()>
where
    S: ExecutionSlotProvider + ?Sized,
    Q: TaskQueueRepository + ?Sized,
    R: TaskRuntimeSpawner + ?Sized,
{
    loop {
        match slots.try_acquire_execution_slot(agent_id).await? {
            ExecutionSlotAcquireOutcome::Saturated => break,
            ExecutionSlotAcquireOutcome::Acquired => {}
        }

        let Some(task) = queue.claim_next_queued_task_for_agent(agent_id).await? else {
            slots.release_execution_slot(agent_id).await?;
            break;
        };

        spawner.spawn_task_runner(task).await;
    }

    Ok(())
}

/// Resume queued work for every agent that has at least one queued task.
///
/// Returns the number of agents whose queues were inspected. Per-agent
/// failures are logged and swallowed so a single bad agent cannot block
/// startup recovery.
pub async fn resume_queued_tasks<S, Q, R>(slots: &S, queue: &Q, spawner: &R) -> Result<usize>
where
    S: ExecutionSlotProvider + ?Sized,
    Q: TaskQueueRepository + ?Sized,
    R: TaskRuntimeSpawner + ?Sized,
{
    let agent_ids = queue.list_agents_with_queued_tasks().await?;
    let count = agent_ids.len();
    for agent_id in agent_ids {
        if let Err(error) =
            start_next_queued_tasks_for_agent(slots, queue, spawner, &agent_id).await
        {
            tracing::warn!(
                "Failed to resume queued AgentTask work for agent {}: {}",
                agent_id,
                error
            );
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use tokio::sync::Mutex as AsyncMutex;

    struct FakeSlots {
        capacity: AtomicUsize,
        in_use: AsyncMutex<std::collections::HashMap<String, usize>>,
    }

    impl FakeSlots {
        fn with_capacity(capacity: usize) -> Self {
            Self {
                capacity: AtomicUsize::new(capacity),
                in_use: AsyncMutex::new(std::collections::HashMap::new()),
            }
        }

        async fn force_acquire(&self, agent_id: &str) {
            let mut map = self.in_use.lock().await;
            *map.entry(agent_id.to_string()).or_insert(0) += 1;
        }
    }

    #[async_trait]
    impl ExecutionSlotProvider for FakeSlots {
        async fn try_acquire_execution_slot(
            &self,
            agent_id: &str,
        ) -> Result<ExecutionSlotAcquireOutcome> {
            let cap = self.capacity.load(Ordering::SeqCst);
            let mut map = self.in_use.lock().await;
            let entry = map.entry(agent_id.to_string()).or_insert(0);
            if *entry >= cap {
                Ok(ExecutionSlotAcquireOutcome::Saturated)
            } else {
                *entry += 1;
                Ok(ExecutionSlotAcquireOutcome::Acquired)
            }
        }

        async fn release_execution_slot(&self, agent_id: &str) -> Result<()> {
            let mut map = self.in_use.lock().await;
            if let Some(entry) = map.get_mut(agent_id) {
                if *entry > 0 {
                    *entry -= 1;
                }
            }
            Ok(())
        }
    }

    struct FakeQueue {
        tasks: AsyncMutex<Vec<QueuedTaskRef>>,
        queued: AsyncMutex<Vec<QueuedTaskRef>>,
    }

    impl FakeQueue {
        fn new(initial: Vec<QueuedTaskRef>) -> Self {
            Self {
                tasks: AsyncMutex::new(initial),
                queued: AsyncMutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl TaskQueueRepository for FakeQueue {
        async fn get_task_ref(&self, task_id: &str) -> Result<Option<QueuedTaskRef>> {
            let tasks = self.tasks.lock().await;
            Ok(tasks.iter().find(|t| t.task_id == task_id).cloned())
        }

        async fn mark_task_queued(&self, task_id: &str) -> Result<Option<QueuedTaskRef>> {
            let tasks = self.tasks.lock().await;
            let Some(task) = tasks.iter().find(|t| t.task_id == task_id).cloned() else {
                return Ok(None);
            };
            drop(tasks);
            self.queued.lock().await.push(task.clone());
            Ok(Some(task))
        }

        async fn claim_next_queued_task_for_agent(
            &self,
            agent_id: &str,
        ) -> Result<Option<QueuedTaskRef>> {
            let mut queued = self.queued.lock().await;
            if let Some(idx) = queued.iter().position(|t| t.agent_id == agent_id) {
                Ok(Some(queued.remove(idx)))
            } else {
                Ok(None)
            }
        }

        async fn list_agents_with_queued_tasks(&self) -> Result<Vec<String>> {
            let queued = self.queued.lock().await;
            let mut ids: Vec<String> = queued.iter().map(|t| t.agent_id.clone()).collect();
            ids.sort();
            ids.dedup();
            Ok(ids)
        }
    }

    #[derive(Default)]
    struct RecordingPropagator {
        events: Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl TaskSurfacePropagator for RecordingPropagator {
        async fn apply_task_surface_state(&self, task_id: &str, status: &str) {
            self.events
                .lock()
                .unwrap()
                .push((task_id.to_string(), status.to_string()));
        }
    }

    #[derive(Default)]
    struct RecordingBroadcaster {
        broadcasts: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl QueuedTaskBroadcaster for RecordingBroadcaster {
        async fn broadcast_queued(&self, task_id: &str) {
            self.broadcasts.lock().unwrap().push(task_id.to_string());
        }
    }

    #[derive(Default)]
    struct RecordingSpawner {
        spawned: Mutex<Vec<QueuedTaskRef>>,
    }

    #[async_trait]
    impl TaskRuntimeSpawner for RecordingSpawner {
        async fn spawn_task_runner(&self, task: QueuedTaskRef) {
            self.spawned.lock().unwrap().push(task);
        }
    }

    fn task(id: &str, agent: &str) -> QueuedTaskRef {
        QueuedTaskRef {
            task_id: id.to_string(),
            agent_id: agent.to_string(),
        }
    }

    #[tokio::test]
    async fn admit_starts_when_slot_available() {
        let slots = FakeSlots::with_capacity(1);
        let queue = FakeQueue::new(vec![task("t1", "a1")]);
        let propagator = RecordingPropagator::default();
        let broadcaster = RecordingBroadcaster::default();
        let spawner = RecordingSpawner::default();

        let outcome = admit_or_queue_task(&slots, &queue, &propagator, &broadcaster, &spawner, "t1")
            .await
            .unwrap();

        assert_eq!(outcome, TaskAdmissionOutcome::Started);
        assert_eq!(spawner.spawned.lock().unwrap().len(), 1);
        assert!(propagator.events.lock().unwrap().is_empty());
        assert!(broadcaster.broadcasts.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn admit_queues_when_slot_saturated() {
        let slots = FakeSlots::with_capacity(1);
        slots.force_acquire("a1").await;
        let queue = FakeQueue::new(vec![task("t1", "a1")]);
        let propagator = RecordingPropagator::default();
        let broadcaster = RecordingBroadcaster::default();
        let spawner = RecordingSpawner::default();

        let outcome = admit_or_queue_task(&slots, &queue, &propagator, &broadcaster, &spawner, "t1")
            .await
            .unwrap();

        assert_eq!(outcome, TaskAdmissionOutcome::Queued);
        assert!(spawner.spawned.lock().unwrap().is_empty());
        assert_eq!(
            propagator.events.lock().unwrap().clone(),
            vec![("t1".to_string(), "queued".to_string())]
        );
        assert_eq!(
            broadcaster.broadcasts.lock().unwrap().clone(),
            vec!["t1".to_string()]
        );
    }

    #[tokio::test]
    async fn resume_drains_queued_tasks_per_agent() {
        let slots = FakeSlots::with_capacity(2);
        let queue = FakeQueue::new(vec![]);
        queue
            .queued
            .lock()
            .await
            .extend([task("t1", "a1"), task("t2", "a1"), task("t3", "a2")]);
        let spawner = RecordingSpawner::default();

        let agents = resume_queued_tasks(&slots, &queue, &spawner).await.unwrap();
        assert_eq!(agents, 2);
        assert_eq!(spawner.spawned.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn start_next_releases_slot_when_queue_empty() {
        let slots = FakeSlots::with_capacity(1);
        let queue = FakeQueue::new(vec![]);
        let spawner = RecordingSpawner::default();

        start_next_queued_tasks_for_agent(&slots, &queue, &spawner, "a1")
            .await
            .unwrap();

        let in_use_total: usize = slots.in_use.lock().await.values().sum();
        assert_eq!(in_use_total, 0);
        assert!(spawner.spawned.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn admit_or_queue_task_concurrent_only_one_winner_per_slot() {
        // Single slot; both admit calls race. Exactly one must Start, the
        // other must Queue.
        let slots = std::sync::Arc::new(FakeSlots::with_capacity(1));
        let queue = std::sync::Arc::new(FakeQueue::new(vec![
            task("t1", "shared"),
            task("t2", "shared"),
        ]));
        let propagator = std::sync::Arc::new(RecordingPropagator::default());
        let broadcaster = std::sync::Arc::new(RecordingBroadcaster::default());
        let spawner = std::sync::Arc::new(RecordingSpawner::default());

        let s1 = slots.clone();
        let q1 = queue.clone();
        let p1 = propagator.clone();
        let b1 = broadcaster.clone();
        let r1 = spawner.clone();
        let s2 = slots.clone();
        let q2 = queue.clone();
        let p2 = propagator.clone();
        let b2 = broadcaster.clone();
        let r2 = spawner.clone();

        let h1 = tokio::spawn(async move {
            admit_or_queue_task(&*s1, &*q1, &*p1, &*b1, &*r1, "t1")
                .await
                .unwrap()
        });
        let h2 = tokio::spawn(async move {
            admit_or_queue_task(&*s2, &*q2, &*p2, &*b2, &*r2, "t2")
                .await
                .unwrap()
        });

        let (o1, o2) = tokio::join!(h1, h2);
        let outcomes = [o1.unwrap(), o2.unwrap()];

        let started = outcomes
            .iter()
            .filter(|o| **o == TaskAdmissionOutcome::Started)
            .count();
        let queued = outcomes
            .iter()
            .filter(|o| **o == TaskAdmissionOutcome::Queued)
            .count();
        assert_eq!(started, 1);
        assert_eq!(queued, 1);
        assert_eq!(spawner.spawned.lock().unwrap().len(), 1);
        assert_eq!(broadcaster.broadcasts.lock().unwrap().len(), 1);
    }
}
