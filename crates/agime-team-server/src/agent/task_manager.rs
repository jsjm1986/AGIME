//! Task manager for tracking background task execution
//!
//! Provides tracking, cancellation, and real-time streaming support for running tasks.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
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
    /// Workspace files likely changed after a tool execution
    WorkspaceChanged { tool_name: String },
    /// Turn progress
    Turn { current: usize, max: usize },
    /// Context compaction occurred
    Compaction {
        strategy: String,
        before_tokens: usize,
        after_tokens: usize,
    },
    /// Session ID notification
    SessionId { session_id: String },
    /// Task completed
    Done {
        status: String,
        error: Option<String>,
    },
    // ─── AGE Events ───
    /// Goal execution started
    GoalStart {
        goal_id: String,
        title: String,
        depth: u32,
    },
    /// Goal execution completed
    GoalComplete { goal_id: String, signal: String },
    /// Goal pivoted to new approach
    Pivot {
        goal_id: String,
        from_approach: String,
        to_approach: String,
        learnings: String,
    },
    /// Goal abandoned
    GoalAbandoned { goal_id: String, reason: String },
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
            Self::WorkspaceChanged { .. } => "workspace_changed",
            Self::Turn { .. } => "turn",
            Self::Compaction { .. } => "compaction",
            Self::SessionId { .. } => "session_id",
            Self::Done { .. } => "done",
            Self::GoalStart { .. } => "goal_start",
            Self::GoalComplete { .. } => "goal_complete",
            Self::Pivot { .. } => "pivot",
            Self::GoalAbandoned { .. } => "goal_abandoned",
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
                // Only warn if there are active receivers (lagged = buffer overflow)
                // SendError means no receivers, which is normal
                warn!("broadcast to task {}: no active receivers ({})", task_id, e);
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

    /// Cancel a running task and remove it from tracking
    pub async fn cancel(&self, task_id: &str) -> bool {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.remove(task_id) {
            task.cancel_token.cancel();
            warn!("Task cancelled and removed: {}", task_id);
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
#[allow(dead_code)]
pub fn create_task_manager() -> Arc<TaskManager> {
    Arc::new(TaskManager::new())
}
