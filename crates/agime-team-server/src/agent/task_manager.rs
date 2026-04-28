//! Task manager for tracking background task execution
//!
//! Provides tracking, cancellation, and real-time streaming support for running tasks.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::{broadcast, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Stream event for real-time task updates
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    /// Status changed
    Status { status: String },
    /// Text content (streaming)
    Text { content: String },
    /// Thinking/reasoning content (extended thinking from models like Claude)
    Thinking { content: String },
    /// Tool call started
    ToolCall { name: String, id: String },
    /// Tool result
    ToolResult {
        id: String,
        success: bool,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },
    /// Bounded worker started
    WorkerStarted {
        task_id: String,
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        logical_worker_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt_index: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_task_id: Option<String>,
    },
    /// Bounded worker progress
    WorkerProgress {
        task_id: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        percent: Option<u8>,
    },
    WorkerFollowup {
        task_id: String,
        kind: String,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        logical_worker_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt_index: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_task_id: Option<String>,
    },
    WorkerIdle {
        task_id: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        logical_worker_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt_id: Option<String>,
    },
    /// Bounded worker completed or failed
    WorkerFinished {
        task_id: String,
        kind: String,
        status: String,
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        produced_delta: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        logical_worker_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt_index: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_task_id: Option<String>,
    },
    PermissionRequested {
        task_id: String,
        tool_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worker_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        logical_worker_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt_id: Option<String>,
    },
    PermissionResolved {
        task_id: String,
        tool_name: String,
        decision: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worker_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        logical_worker_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt_id: Option<String>,
    },
    PermissionTimedOut {
        task_id: String,
        tool_name: String,
        timeout_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worker_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        logical_worker_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt_id: Option<String>,
    },
    /// Workspace files likely changed after a tool execution
    WorkspaceChanged { tool_name: String },
    /// Turn progress
    Turn { current: usize, max: usize },
    /// Context compaction occurred
    Compaction {
        strategy: String,
        before_tokens: usize,
        after_tokens: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Session ID notification
    SessionId { session_id: String },
    /// Task completed
    Done {
        status: String,
        error: Option<String>,
    },
}

impl StreamEvent {
    /// SSE event type string for this event variant.
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::Status { .. } => "status",
            Self::Text { .. } => "text",
            Self::Thinking { .. } => "thinking",
            Self::ToolCall { .. } => "toolcall",
            Self::ToolResult { .. } => "toolresult",
            Self::WorkerStarted { .. } => "worker_started",
            Self::WorkerProgress { .. } => "worker_progress",
            Self::WorkerFollowup { .. } => "worker_followup",
            Self::WorkerIdle { .. } => "worker_idle",
            Self::WorkerFinished { .. } => "worker_finished",
            Self::PermissionRequested { .. } => "permission_requested",
            Self::PermissionResolved { .. } => "permission_resolved",
            Self::PermissionTimedOut { .. } => "permission_timed_out",
            Self::WorkspaceChanged { .. } => "workspace_changed",
            Self::Turn { .. } => "turn",
            Self::Compaction { .. } => "compaction",
            Self::SessionId { .. } => "session_id",
            Self::Done { .. } => "done",
        }
    }

    /// Whether this event signals the end of the stream.
    pub fn is_done(&self) -> bool {
        matches!(self, Self::Done { .. })
    }
}

/// Running task info
pub struct RunningTask {
    #[allow(dead_code)]
    pub task_id: String,
    pub cancel_token: CancellationToken,
    pub started_at: std::time::Instant,
    pub stream_tx: broadcast::Sender<StreamEvent>,
}

/// Task manager for tracking running tasks
pub struct TaskManager {
    tasks: RwLock<HashMap<String, RunningTask>>,
}

impl TaskManager {
    /// Create a new task manager
    pub fn new() -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new running task, returns (CancellationToken, broadcast::Sender)
    pub async fn register(
        &self,
        task_id: &str,
    ) -> (CancellationToken, broadcast::Sender<StreamEvent>) {
        {
            let tasks = self.tasks.read().await;
            if let Some(existing) = tasks.get(task_id) {
                return (existing.cancel_token.clone(), existing.stream_tx.clone());
            }
        }

        let token = CancellationToken::new();
        // Create broadcast channel with buffer for 512 events
        let (tx, _) = broadcast::channel(512);
        let task = RunningTask {
            task_id: task_id.to_string(),
            cancel_token: token.clone(),
            started_at: std::time::Instant::now(),
            stream_tx: tx.clone(),
        };

        let mut tasks = self.tasks.write().await;
        tasks.insert(task_id.to_string(), task);
        info!("Task registered: {}", task_id);

        (token, tx)
    }

    /// Subscribe to task stream events
    pub async fn subscribe(&self, task_id: &str) -> Option<broadcast::Receiver<StreamEvent>> {
        let tasks = self.tasks.read().await;
        tasks.get(task_id).map(|t| t.stream_tx.subscribe())
    }

    /// Broadcast an event to task subscribers
    pub async fn broadcast(&self, task_id: &str, event: StreamEvent) {
        let tasks = self.tasks.read().await;
        if let Some(task) = tasks.get(task_id) {
            if let Err(e) = task.stream_tx.send(event) {
                tracing::debug!(
                    "broadcast to task {} dropped because there are no active receivers ({})",
                    task_id,
                    e
                );
            }
        }
    }

    /// Mark task as completed and remove from tracking
    pub async fn complete(&self, task_id: &str) {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.remove(task_id) {
            let duration = task.started_at.elapsed();
            info!("Task completed: {} (duration: {:?})", task_id, duration);
        }
    }

    /// Cancel a running task.
    ///
    /// Keep the task registered until the runner emits its terminal `Done` event.
    /// Removing it here makes the final event invisible to `/tasks/{id}/stream`.
    pub async fn cancel(&self, task_id: &str) -> bool {
        let tasks = self.tasks.read().await;
        if let Some(task) = tasks.get(task_id) {
            task.cancel_token.cancel();
            warn!("Task cancellation requested: {}", task_id);
            true
        } else {
            false
        }
    }

    /// Check if a task is running
    #[allow(dead_code)]
    pub async fn is_running(&self, task_id: &str) -> bool {
        let tasks = self.tasks.read().await;
        tasks.contains_key(task_id)
    }

    /// Get count of running tasks
    #[allow(dead_code)]
    pub async fn running_count(&self) -> usize {
        let tasks = self.tasks.read().await;
        tasks.len()
    }

    /// Clean up stale tasks (running for more than max_duration)
    pub async fn cleanup_stale(&self, max_duration: std::time::Duration) {
        let mut tasks = self.tasks.write().await;
        let now = std::time::Instant::now();

        let stale: Vec<String> = tasks
            .iter()
            .filter(|(_, t)| now.duration_since(t.started_at) > max_duration)
            .map(|(id, _)| id.clone())
            .collect();

        for id in stale {
            if let Some(task) = tasks.remove(&id) {
                task.cancel_token.cancel();
                warn!("Stale task cancelled: {}", id);
            }
        }
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a shared task manager
pub fn create_task_manager() -> Arc<TaskManager> {
    static GLOBAL_TASK_MANAGER: OnceLock<Arc<TaskManager>> = OnceLock::new();
    GLOBAL_TASK_MANAGER
        .get_or_init(|| Arc::new(TaskManager::new()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::{StreamEvent, TaskManager};

    #[test]
    fn worker_followup_and_permission_timeout_event_types_are_stable() {
        assert_eq!(
            StreamEvent::WorkerFollowup {
                task_id: "task-1".to_string(),
                kind: "correction".to_string(),
                reason: "validation failed".to_string(),
                logical_worker_id: Some("worker-a".to_string()),
                attempt_id: Some("attempt-1".to_string()),
                attempt_index: Some(1),
                previous_task_id: Some("task-0".to_string()),
            }
            .event_type(),
            "worker_followup"
        );
        assert_eq!(
            StreamEvent::PermissionTimedOut {
                task_id: "task-1".to_string(),
                tool_name: "write_file".to_string(),
                timeout_ms: 60000,
                worker_name: Some("worker-1".to_string()),
                logical_worker_id: Some("worker-a".to_string()),
                attempt_id: Some("attempt-1".to_string()),
            }
            .event_type(),
            "permission_timed_out"
        );
    }

    #[tokio::test]
    async fn cancel_keeps_stream_registered_until_complete() {
        let manager = TaskManager::new();
        let (token, _) = manager.register("task-1").await;
        let receiver = manager.subscribe("task-1").await;

        assert!(manager.cancel("task-1").await);
        assert!(token.is_cancelled());
        assert!(manager.is_running("task-1").await);
        assert!(receiver.is_some());

        manager
            .broadcast(
                "task-1",
                StreamEvent::Done {
                    status: "cancelled".to_string(),
                    error: None,
                },
            )
            .await;
        manager.complete("task-1").await;
        assert!(!manager.is_running("task-1").await);
    }
}
