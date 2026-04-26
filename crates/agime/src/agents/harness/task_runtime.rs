use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Subagent,
    SwarmWorker,
    ValidationWorker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorkerAttemptIdentity {
    pub logical_worker_id: String,
    pub attempt_id: String,
    pub attempt_index: u32,
    pub followup_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_task_id: Option<String>,
}

impl WorkerAttemptIdentity {
    const LOGICAL_WORKER_ID_KEY: &'static str = "logical_worker_id";
    const ATTEMPT_ID_KEY: &'static str = "attempt_id";
    const ATTEMPT_INDEX_KEY: &'static str = "attempt_index";
    const FOLLOWUP_KIND_KEY: &'static str = "followup_kind";
    const PREVIOUS_TASK_ID_KEY: &'static str = "previous_task_id";

    pub fn fresh(logical_worker_id: impl Into<String>, attempt_id: impl Into<String>) -> Self {
        Self {
            logical_worker_id: logical_worker_id.into(),
            attempt_id: attempt_id.into(),
            attempt_index: 0,
            followup_kind: "fresh".to_string(),
            previous_task_id: None,
        }
    }

    pub fn followup(
        logical_worker_id: impl Into<String>,
        attempt_id: impl Into<String>,
        attempt_index: u32,
        followup_kind: impl Into<String>,
        previous_task_id: impl Into<String>,
    ) -> Self {
        Self {
            logical_worker_id: logical_worker_id.into(),
            attempt_id: attempt_id.into(),
            attempt_index,
            followup_kind: followup_kind.into(),
            previous_task_id: Some(previous_task_id.into()),
        }
    }

    pub fn write_to_metadata(&self, metadata: &mut HashMap<String, String>) {
        metadata.insert(
            Self::LOGICAL_WORKER_ID_KEY.to_string(),
            self.logical_worker_id.clone(),
        );
        metadata.insert(Self::ATTEMPT_ID_KEY.to_string(), self.attempt_id.clone());
        metadata.insert(
            Self::ATTEMPT_INDEX_KEY.to_string(),
            self.attempt_index.to_string(),
        );
        metadata.insert(
            Self::FOLLOWUP_KIND_KEY.to_string(),
            self.followup_kind.clone(),
        );
        if let Some(previous_task_id) = &self.previous_task_id {
            metadata.insert(
                Self::PREVIOUS_TASK_ID_KEY.to_string(),
                previous_task_id.clone(),
            );
        }
    }

    pub fn from_metadata(metadata: &HashMap<String, String>) -> Option<Self> {
        let logical_worker_id = metadata.get(Self::LOGICAL_WORKER_ID_KEY)?.clone();
        let attempt_id = metadata.get(Self::ATTEMPT_ID_KEY)?.clone();
        let attempt_index = metadata
            .get(Self::ATTEMPT_INDEX_KEY)
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let followup_kind = metadata
            .get(Self::FOLLOWUP_KIND_KEY)
            .cloned()
            .unwrap_or_else(|| "fresh".to_string());
        Some(Self {
            logical_worker_id,
            attempt_id,
            attempt_index,
            followup_kind,
            previous_task_id: metadata.get(Self::PREVIOUS_TASK_ID_KEY).cloned(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    pub task_id: String,
    pub parent_session_id: String,
    pub depth: u32,
    pub kind: TaskKind,
    pub description: Option<String>,
    pub write_scope: Vec<String>,
    pub target_artifacts: Vec<String>,
    pub result_contract: Vec<String>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub task_id: String,
    pub parent_session_id: String,
    pub depth: u32,
    pub kind: TaskKind,
    pub status: TaskStatus,
    pub description: Option<String>,
    pub write_scope: Vec<String>,
    pub target_artifacts: Vec<String>,
    pub result_contract: Vec<String>,
    pub summary: Option<String>,
    pub produced_delta: bool,
    pub accepted_targets: Vec<String>,
    pub metadata: HashMap<String, String>,
    pub started_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResultEnvelope {
    pub task_id: String,
    pub kind: TaskKind,
    pub status: TaskStatus,
    pub summary: String,
    pub accepted_targets: Vec<String>,
    pub produced_delta: bool,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskRuntimeEvent {
    Started(TaskSnapshot),
    Progress {
        task_id: String,
        message: String,
        percent: Option<u8>,
    },
    FollowupRequested {
        task_id: String,
        kind: String,
        reason: String,
    },
    Idle {
        task_id: String,
        message: String,
    },
    PermissionRequested {
        task_id: String,
        worker_name: Option<String>,
        tool_name: String,
    },
    PermissionResolved {
        task_id: String,
        worker_name: Option<String>,
        tool_name: String,
        decision: String,
        source: Option<String>,
    },
    PermissionTimedOut {
        task_id: String,
        worker_name: Option<String>,
        tool_name: String,
        timeout_ms: u64,
    },
    Completed(TaskResultEnvelope),
    Failed(TaskResultEnvelope),
    Cancelled {
        task_id: String,
    },
}

#[derive(Debug, Clone)]
pub struct TaskHandle {
    pub task_id: String,
    pub cancel_token: CancellationToken,
}

#[async_trait]
pub trait TaskRuntimeHost: Send + Sync {
    async fn spawn_task(&self, spec: TaskSpec) -> Result<TaskHandle>;
    async fn record_progress(
        &self,
        task_id: &str,
        message: impl Into<String> + Send,
        percent: Option<u8>,
    ) -> Result<()>;
    async fn record_followup_requested(
        &self,
        task_id: &str,
        kind: impl Into<String> + Send,
        reason: impl Into<String> + Send,
    ) -> Result<()>;
    async fn record_idle(&self, task_id: &str, message: impl Into<String> + Send) -> Result<()>;
    async fn record_permission_requested(
        &self,
        task_id: &str,
        worker_name: Option<String>,
        tool_name: impl Into<String> + Send,
    ) -> Result<()>;
    async fn record_permission_resolved(
        &self,
        task_id: &str,
        worker_name: Option<String>,
        tool_name: impl Into<String> + Send,
        decision: impl Into<String> + Send,
        source: Option<String>,
    ) -> Result<()>;
    async fn record_permission_timed_out(
        &self,
        task_id: &str,
        worker_name: Option<String>,
        tool_name: impl Into<String> + Send,
        timeout_ms: u64,
    ) -> Result<()>;
    async fn complete(&self, result: TaskResultEnvelope) -> Result<()>;
    async fn fail(&self, result: TaskResultEnvelope) -> Result<()>;
    async fn cancel(&self, task_id: &str) -> Result<()>;
    async fn snapshot(&self, task_id: &str) -> Option<TaskSnapshot>;
    fn subscribe(&self, task_id: &str) -> Option<broadcast::Receiver<TaskRuntimeEvent>>;
    fn subscribe_all(&self) -> broadcast::Receiver<TaskRuntimeEvent>;
}

#[derive(Debug, Clone)]
pub struct TaskRuntime {
    tasks: Arc<DashMap<String, TaskSnapshot>>,
    event_buses: Arc<DashMap<String, broadcast::Sender<TaskRuntimeEvent>>>,
    cancel_tokens: Arc<DashMap<String, CancellationToken>>,
    global_event_bus: broadcast::Sender<TaskRuntimeEvent>,
}

impl TaskRuntime {
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn has_active_tasks_for_parent_session(&self, parent_session_id: &str) -> bool {
        self.tasks.iter().any(|entry| {
            let snapshot = entry.value();
            snapshot.parent_session_id == parent_session_id
                && matches!(snapshot.status, TaskStatus::Running | TaskStatus::Pending)
        })
    }

    fn emit(&self, task_id: &str, event: TaskRuntimeEvent) {
        if let Some(bus) = self.event_buses.get(task_id) {
            let _ = bus.send(event.clone());
        }
        let _ = self.global_event_bus.send(event);
    }

    fn update_terminal(
        &self,
        task_id: &str,
        status: TaskStatus,
        summary: Option<String>,
        produced_delta: bool,
        accepted_targets: Vec<String>,
    ) -> Result<()> {
        let Some(mut entry) = self.tasks.get_mut(task_id) else {
            return Err(anyhow!("task {} not found", task_id));
        };
        entry.status = status;
        entry.summary = summary;
        entry.produced_delta = produced_delta;
        entry.accepted_targets = accepted_targets;
        entry.updated_at = Utc::now().timestamp();
        entry.finished_at = Some(entry.updated_at);
        Ok(())
    }
}

impl Default for TaskRuntime {
    fn default() -> Self {
        let (global_event_bus, _) = broadcast::channel(256);
        Self {
            tasks: Arc::new(DashMap::new()),
            event_buses: Arc::new(DashMap::new()),
            cancel_tokens: Arc::new(DashMap::new()),
            global_event_bus,
        }
    }
}

static SESSION_TASK_RUNTIMES: Lazy<DashMap<String, Arc<TaskRuntime>>> = Lazy::new(DashMap::new);

pub fn register_task_runtime(session_id: &str, runtime: Arc<TaskRuntime>) {
    SESSION_TASK_RUNTIMES.insert(session_id.to_string(), runtime);
}

pub fn task_runtime_for_session(session_id: &str) -> Option<Arc<TaskRuntime>> {
    SESSION_TASK_RUNTIMES
        .get(session_id)
        .map(|runtime| runtime.clone())
}

pub fn unregister_task_runtime(session_id: &str) {
    SESSION_TASK_RUNTIMES.remove(session_id);
}

#[async_trait]
impl TaskRuntimeHost for TaskRuntime {
    async fn spawn_task(&self, spec: TaskSpec) -> Result<TaskHandle> {
        if self.tasks.contains_key(&spec.task_id) {
            return Err(anyhow!("task {} already exists", spec.task_id));
        }

        let now = Utc::now().timestamp();
        let snapshot = TaskSnapshot {
            task_id: spec.task_id.clone(),
            parent_session_id: spec.parent_session_id.clone(),
            depth: spec.depth,
            kind: spec.kind,
            status: TaskStatus::Running,
            description: spec.description.clone(),
            write_scope: spec.write_scope.clone(),
            target_artifacts: spec.target_artifacts.clone(),
            result_contract: spec.result_contract.clone(),
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: spec.metadata.clone(),
            started_at: now,
            updated_at: now,
            finished_at: None,
        };
        let (tx, _) = broadcast::channel(64);
        self.tasks.insert(spec.task_id.clone(), snapshot.clone());
        self.event_buses.insert(spec.task_id.clone(), tx);
        let cancel_token = CancellationToken::new();
        self.cancel_tokens
            .insert(spec.task_id.clone(), cancel_token.clone());
        self.emit(&spec.task_id, TaskRuntimeEvent::Started(snapshot));
        Ok(TaskHandle {
            task_id: spec.task_id,
            cancel_token,
        })
    }

    async fn record_progress(
        &self,
        task_id: &str,
        message: impl Into<String> + Send,
        percent: Option<u8>,
    ) -> Result<()> {
        let Some(mut entry) = self.tasks.get_mut(task_id) else {
            return Err(anyhow!("task {} not found", task_id));
        };
        entry.updated_at = Utc::now().timestamp();
        self.emit(
            task_id,
            TaskRuntimeEvent::Progress {
                task_id: task_id.to_string(),
                message: message.into(),
                percent,
            },
        );
        Ok(())
    }

    async fn record_followup_requested(
        &self,
        task_id: &str,
        kind: impl Into<String> + Send,
        reason: impl Into<String> + Send,
    ) -> Result<()> {
        let Some(mut entry) = self.tasks.get_mut(task_id) else {
            return Err(anyhow!("task {} not found", task_id));
        };
        entry.updated_at = Utc::now().timestamp();
        self.emit(
            task_id,
            TaskRuntimeEvent::FollowupRequested {
                task_id: task_id.to_string(),
                kind: kind.into(),
                reason: reason.into(),
            },
        );
        Ok(())
    }

    async fn record_idle(&self, task_id: &str, message: impl Into<String> + Send) -> Result<()> {
        let Some(mut entry) = self.tasks.get_mut(task_id) else {
            return Err(anyhow!("task {} not found", task_id));
        };
        entry.updated_at = Utc::now().timestamp();
        self.emit(
            task_id,
            TaskRuntimeEvent::Idle {
                task_id: task_id.to_string(),
                message: message.into(),
            },
        );
        Ok(())
    }

    async fn record_permission_requested(
        &self,
        task_id: &str,
        worker_name: Option<String>,
        tool_name: impl Into<String> + Send,
    ) -> Result<()> {
        let Some(mut entry) = self.tasks.get_mut(task_id) else {
            return Err(anyhow!("task {} not found", task_id));
        };
        entry.updated_at = Utc::now().timestamp();
        self.emit(
            task_id,
            TaskRuntimeEvent::PermissionRequested {
                task_id: task_id.to_string(),
                worker_name,
                tool_name: tool_name.into(),
            },
        );
        Ok(())
    }

    async fn record_permission_timed_out(
        &self,
        task_id: &str,
        worker_name: Option<String>,
        tool_name: impl Into<String> + Send,
        timeout_ms: u64,
    ) -> Result<()> {
        let Some(mut entry) = self.tasks.get_mut(task_id) else {
            return Err(anyhow!("task {} not found", task_id));
        };
        entry.updated_at = Utc::now().timestamp();
        self.emit(
            task_id,
            TaskRuntimeEvent::PermissionTimedOut {
                task_id: task_id.to_string(),
                worker_name,
                tool_name: tool_name.into(),
                timeout_ms,
            },
        );
        Ok(())
    }

    async fn record_permission_resolved(
        &self,
        task_id: &str,
        worker_name: Option<String>,
        tool_name: impl Into<String> + Send,
        decision: impl Into<String> + Send,
        source: Option<String>,
    ) -> Result<()> {
        let Some(mut entry) = self.tasks.get_mut(task_id) else {
            return Err(anyhow!("task {} not found", task_id));
        };
        entry.updated_at = Utc::now().timestamp();
        self.emit(
            task_id,
            TaskRuntimeEvent::PermissionResolved {
                task_id: task_id.to_string(),
                worker_name,
                tool_name: tool_name.into(),
                decision: decision.into(),
                source,
            },
        );
        Ok(())
    }

    async fn complete(&self, result: TaskResultEnvelope) -> Result<()> {
        let task_id = result.task_id.clone();
        if self
            .tasks
            .get(&task_id)
            .is_some_and(|entry| entry.status == TaskStatus::Cancelled)
        {
            return Ok(());
        }
        self.update_terminal(
            &task_id,
            TaskStatus::Completed,
            Some(result.summary.clone()),
            result.produced_delta,
            result.accepted_targets.clone(),
        )?;
        self.emit(&task_id, TaskRuntimeEvent::Completed(result));
        Ok(())
    }

    async fn fail(&self, result: TaskResultEnvelope) -> Result<()> {
        let task_id = result.task_id.clone();
        if self
            .tasks
            .get(&task_id)
            .is_some_and(|entry| entry.status == TaskStatus::Cancelled)
        {
            return Ok(());
        }
        self.update_terminal(
            &task_id,
            TaskStatus::Failed,
            Some(result.summary.clone()),
            result.produced_delta,
            result.accepted_targets.clone(),
        )?;
        self.emit(&task_id, TaskRuntimeEvent::Failed(result));
        Ok(())
    }

    async fn cancel(&self, task_id: &str) -> Result<()> {
        let Some(cancel_token) = self.cancel_tokens.get(task_id) else {
            return Err(anyhow!("task {} not found", task_id));
        };
        cancel_token.cancel();
        self.update_terminal(task_id, TaskStatus::Cancelled, None, false, Vec::new())?;
        self.emit(
            task_id,
            TaskRuntimeEvent::Cancelled {
                task_id: task_id.to_string(),
            },
        );
        Ok(())
    }

    async fn snapshot(&self, task_id: &str) -> Option<TaskSnapshot> {
        self.tasks.get(task_id).map(|entry| entry.clone())
    }

    fn subscribe(&self, task_id: &str) -> Option<broadcast::Receiver<TaskRuntimeEvent>> {
        self.event_buses.get(task_id).map(|bus| bus.subscribe())
    }

    fn subscribe_all(&self) -> broadcast::Receiver<TaskRuntimeEvent> {
        self.global_event_bus.subscribe()
    }
}
