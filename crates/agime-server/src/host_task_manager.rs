//! Desktop harness host — user-level long-running task manager.
//!
//! This is **distinct** from [`crate::host_task_runtime::DesktopTaskRuntime`],
//! which wraps the core [`agime::agents::harness::TaskRuntime`] — that runtime
//! is harness's *internal* sub-agent / swarm-worker / validation-worker
//! dispatcher, scoped to a single chat turn. The core runtime has no concept
//! of "submit a prompt and let it run in the background for minutes/hours";
//! its `spawn_task` is the harness asking itself to fan out a worker.
//!
//! The desktop's *user-level* long-running task is a higher-level construct:
//! the user submits a prompt against a session, the server detaches a
//! background turn that drives the full `run_harness_host` loop, and the UI
//! attaches to a live event stream (or polls a snapshot) and can cancel it.
//! That is what this module provides.
//!
//! The team-server backs the equivalent `TaskManager` with Mongo persistence
//! and user-visible task UUIDs. The desktop keeps everything in-process:
//! - `Arc<Mutex<HashMap<task_id, UserTaskRecord>>>` is the task table.
//! - Each task owns a `CancellationToken` and a `broadcast::Sender` event bus.
//! - A bounded ring buffer per task retains the last N events so an attaching
//!   client can replay missed events via `last_event_id`.
//!
//! When the desktop later wants durability (resume across process restarts),
//! a SQLite-backed implementation plugs in behind the same `submit` / `list`
//! / `get` / `cancel` / `subscribe` surface without changing the routes.
//!
//! All callers are gated by the `desktop_harness_host` cargo feature.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;

use agime::agents::{AgentEvent, SessionConfig};
use agime::conversation::message::Message;
use anyhow::{anyhow, Result};
use chrono::Utc;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Mutex};
use tokio_util::sync::CancellationToken;

use crate::state::AppState;

/// Capacity of each task's live event broadcast channel. A slow attaching
/// client that lags beyond this many events sees a `Lagged` notification and
/// should re-fetch the snapshot; this is fine because the snapshot carries the
/// terminal status.
const TASK_EVENT_CHANNEL_CAPACITY: usize = 256;

/// How many recent events to retain per task for `last_event_id` replay. A
/// client that reconnects within this window catches up without missing
/// frames; beyond it, it should re-fetch the snapshot.
const TASK_EVENT_REPLAY_BUFFER: usize = 512;

/// Lifecycle status of a user-level long-running task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserTaskStatus {
    /// Submitted, background turn not yet started.
    Pending,
    /// Background turn is actively running.
    Running,
    /// Background turn finished without error.
    Completed,
    /// Background turn ended in an error.
    Failed,
    /// Cancelled by the client before completion.
    Cancelled,
}

impl UserTaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            UserTaskStatus::Completed | UserTaskStatus::Failed | UserTaskStatus::Cancelled
        )
    }
}

/// A point-in-time view of a task, returned by `get` / `list` and persisted in
/// the in-memory table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserTaskSnapshot {
    pub task_id: String,
    pub session_id: String,
    pub status: UserTaskStatus,
    /// First ~200 chars of the submitted prompt, for list display.
    pub prompt_preview: String,
    /// Monotonic event sequence number of the last event emitted so far.
    pub last_event_id: u64,
    /// Terminal error string when `status == Failed`.
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
}

/// An event on a task's live stream. `seq` is monotonic per task and is what a
/// client passes back as `last_event_id` to resume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserTaskEvent {
    pub task_id: String,
    pub seq: u64,
    pub payload: UserTaskEventPayload,
}

/// The body of a [`UserTaskEvent`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UserTaskEventPayload {
    /// Background turn transitioned to `Running`.
    Started,
    /// A chat message produced during the turn.
    Message { message: serde_json::Value },
    /// The conversation history was replaced (e.g. after compaction).
    ConversationReplaced { conversation: serde_json::Value },
    /// The active model/mode changed mid-turn.
    ModelChange { model: String, mode: String },
    /// An MCP server notification observed during the turn.
    Notification {
        request_id: String,
        message: serde_json::Value,
    },
    /// A harness control envelope (the 26-type protocol) observed during the
    /// turn, carried opaquely.
    Control { envelope: serde_json::Value },
    /// Terminal: the turn completed successfully.
    Completed,
    /// Terminal: the turn failed.
    Failed { error: String },
    /// Terminal: the task was cancelled.
    Cancelled,
}

struct UserTaskRecord {
    snapshot: UserTaskSnapshot,
    cancel_token: CancellationToken,
    event_tx: broadcast::Sender<UserTaskEvent>,
    /// Retained recent events for `last_event_id` replay on attach.
    replay: VecDeque<UserTaskEvent>,
    /// Next sequence number to assign.
    next_seq: u64,
}

/// Process-wide manager for user-level long-running tasks.
#[derive(Clone)]
pub struct DesktopTaskManager {
    inner: Arc<Mutex<HashMap<String, UserTaskRecord>>>,
}

impl DesktopTaskManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Submit a new long-running task: detach a background turn that drives the
    /// full harness host loop for `session_id` with `user_message`. Returns the
    /// generated `task_id` immediately; the caller can then attach via
    /// [`Self::subscribe`] or poll via [`Self::get`].
    pub async fn submit(
        &self,
        state: Arc<AppState>,
        session_id: String,
        user_message: Message,
    ) -> Result<String> {
        let task_id = format!("task-{}", uuid::Uuid::new_v4());
        let now = Utc::now().timestamp();
        let cancel_token = CancellationToken::new();
        let (event_tx, _) = broadcast::channel(TASK_EVENT_CHANNEL_CAPACITY);
        let prompt_preview = preview_of(&user_message);

        let snapshot = UserTaskSnapshot {
            task_id: task_id.clone(),
            session_id: session_id.clone(),
            status: UserTaskStatus::Pending,
            prompt_preview,
            last_event_id: 0,
            error: None,
            created_at: now,
            updated_at: now,
            finished_at: None,
        };

        {
            let mut map = self.inner.lock().await;
            map.insert(
                task_id.clone(),
                UserTaskRecord {
                    snapshot,
                    cancel_token: cancel_token.clone(),
                    event_tx,
                    replay: VecDeque::new(),
                    next_seq: 0,
                },
            );
        }

        let manager = self.clone();
        let spawn_session_id = session_id.clone();
        let spawn_task_id = task_id.clone();
        drop(tokio::spawn(async move {
            manager
                .run_task(
                    state,
                    spawn_task_id,
                    spawn_session_id,
                    user_message,
                    cancel_token,
                )
                .await;
        }));

        Ok(task_id)
    }

    /// Background driver for a single task. Drives `run_harness_host` (via
    /// [`crate::desktop_harness_host::DesktopHarnessHost`]) and republishes its
    /// `AgentEvent`s + `HarnessControlEnvelope`s as [`UserTaskEvent`]s.
    async fn run_task(
        &self,
        state: Arc<AppState>,
        task_id: String,
        session_id: String,
        user_message: Message,
        cancel_token: CancellationToken,
    ) {
        self.transition(&task_id, UserTaskStatus::Running, None)
            .await;
        self.emit(&task_id, UserTaskEventPayload::Started).await;

        let agent = match state.get_agent(session_id.clone()).await {
            Ok(agent) => agent,
            Err(e) => {
                let error = format!("failed to get session agent: {}", e);
                self.finish_failed(&task_id, error).await;
                return;
            }
        };

        let session_config = SessionConfig {
            id: session_id.clone(),
            schedule_id: None,
            max_turns: None,
            retry_config: None,
        };

        let host = crate::desktop_harness_host::DesktopHarnessHost::new(state.clone());
        let (control_tx, mut control_rx) =
            tokio::sync::mpsc::channel::<agime::agents::HarnessControlEnvelope>(64);

        // Forward control envelopes onto the task event stream until the
        // channel closes (host finished) or the task is cancelled.
        let control_manager = self.clone();
        let control_task_id = task_id.clone();
        let control_cancel = cancel_token.clone();
        let control_forwarder = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = control_cancel.cancelled() => break,
                    maybe = control_rx.recv() => match maybe {
                        Some(envelope) => {
                            let value = serde_json::to_value(&envelope)
                                .unwrap_or(serde_json::Value::Null);
                            control_manager
                                .emit(&control_task_id, UserTaskEventPayload::Control { envelope: value })
                                .await;
                        }
                        None => break,
                    },
                }
            }
        });

        let stream_result = host
            .execute_chat_host_with_mirror(
                agent.clone(),
                user_message,
                session_config,
                Some(cancel_token.clone()),
                control_tx,
            )
            .await;

        let mut stream = match stream_result {
            Ok(stream) => stream,
            Err(e) => {
                control_forwarder.abort();
                self.finish_failed(&task_id, format!("harness host failed to start: {}", e))
                    .await;
                return;
            }
        };

        let mut turn_error: Option<String> = None;
        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    break;
                }
                next = stream.next() => match next {
                    Some(Ok(event)) => {
                        if let Some(payload) = agent_event_payload(&event) {
                            self.emit(&task_id, payload).await;
                        }
                    }
                    Some(Err(e)) => {
                        turn_error = Some(format!("{}", e));
                        break;
                    }
                    None => break,
                },
            }
        }

        control_forwarder.abort();

        if cancel_token.is_cancelled() {
            self.finish_cancelled(&task_id).await;
        } else if let Some(error) = turn_error {
            self.finish_failed(&task_id, error).await;
        } else {
            self.finish_completed(&task_id).await;
        }
    }

    /// List all known tasks, optionally filtered to a single session, newest
    /// first.
    pub async fn list(&self, session_id: Option<&str>) -> Vec<UserTaskSnapshot> {
        let map = self.inner.lock().await;
        let mut out: Vec<UserTaskSnapshot> = map
            .values()
            .filter(|record| {
                session_id
                    .map(|sid| record.snapshot.session_id == sid)
                    .unwrap_or(true)
            })
            .map(|record| record.snapshot.clone())
            .collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out
    }

    /// Fetch the current snapshot for a task.
    pub async fn get(&self, task_id: &str) -> Option<UserTaskSnapshot> {
        let map = self.inner.lock().await;
        map.get(task_id).map(|record| record.snapshot.clone())
    }

    /// Cancel a running task. Returns an error if the task does not exist.
    /// Cancelling an already-terminal task is a no-op success.
    pub async fn cancel(&self, task_id: &str) -> Result<()> {
        let map = self.inner.lock().await;
        let record = map
            .get(task_id)
            .ok_or_else(|| anyhow!("task {} not found", task_id))?;
        if !record.snapshot.status.is_terminal() {
            record.cancel_token.cancel();
        }
        Ok(())
    }

    /// Subscribe to a task's live event stream. Returns the events the client
    /// missed (those with `seq > after_seq`, from the replay buffer) plus a
    /// live receiver for subsequent events. Returns `None` if the task does
    /// not exist.
    pub async fn subscribe(
        &self,
        task_id: &str,
        after_seq: Option<u64>,
    ) -> Option<(
        Vec<UserTaskEvent>,
        broadcast::Receiver<UserTaskEvent>,
        UserTaskStatus,
    )> {
        let map = self.inner.lock().await;
        let record = map.get(task_id)?;
        let receiver = record.event_tx.subscribe();
        let backlog: Vec<UserTaskEvent> = match after_seq {
            Some(after) => record
                .replay
                .iter()
                .filter(|event| event.seq > after)
                .cloned()
                .collect(),
            None => record.replay.iter().cloned().collect(),
        };
        Some((backlog, receiver, record.snapshot.status))
    }

    /// Emit an event for a task: assign the next seq, push into the replay
    /// buffer, update the snapshot's `last_event_id`, and broadcast. Dropped
    /// silently if the task no longer exists.
    async fn emit(&self, task_id: &str, payload: UserTaskEventPayload) {
        let mut map = self.inner.lock().await;
        let Some(record) = map.get_mut(task_id) else {
            return;
        };
        let seq = record.next_seq;
        record.next_seq += 1;
        let event = UserTaskEvent {
            task_id: task_id.to_string(),
            seq,
            payload,
        };
        record.snapshot.last_event_id = seq;
        record.snapshot.updated_at = Utc::now().timestamp();
        record.replay.push_back(event.clone());
        while record.replay.len() > TASK_EVENT_REPLAY_BUFFER {
            record.replay.pop_front();
        }
        // A send error only means no live subscribers; the replay buffer still
        // retains the event for a later attach.
        let _ = record.event_tx.send(event);
    }

    /// Update a task's lifecycle status without emitting an event.
    async fn transition(&self, task_id: &str, status: UserTaskStatus, error: Option<String>) {
        let mut map = self.inner.lock().await;
        let Some(record) = map.get_mut(task_id) else {
            return;
        };
        record.snapshot.status = status;
        record.snapshot.updated_at = Utc::now().timestamp();
        if let Some(error) = error {
            record.snapshot.error = Some(error);
        }
        if status.is_terminal() {
            record.snapshot.finished_at = Some(record.snapshot.updated_at);
        }
    }

    async fn finish_completed(&self, task_id: &str) {
        self.transition(task_id, UserTaskStatus::Completed, None)
            .await;
        self.emit(task_id, UserTaskEventPayload::Completed).await;
    }

    async fn finish_failed(&self, task_id: &str, error: String) {
        self.transition(task_id, UserTaskStatus::Failed, Some(error.clone()))
            .await;
        self.emit(task_id, UserTaskEventPayload::Failed { error })
            .await;
    }

    async fn finish_cancelled(&self, task_id: &str) {
        self.transition(task_id, UserTaskStatus::Cancelled, None)
            .await;
        self.emit(task_id, UserTaskEventPayload::Cancelled).await;
    }
}

impl Default for DesktopTaskManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Map a core [`AgentEvent`] (which is not itself `Serialize`) to a
/// serializable [`UserTaskEventPayload`], mirroring the variant handling in
/// `routes/reply.rs`. `ToolTransportRequest` is intentionally dropped — the
/// desktop `/reply` stream drops it too.
fn agent_event_payload(event: &AgentEvent) -> Option<UserTaskEventPayload> {
    match event {
        AgentEvent::Message(message) => Some(UserTaskEventPayload::Message {
            message: serde_json::to_value(message).unwrap_or(serde_json::Value::Null),
        }),
        AgentEvent::HistoryReplaced(conversation) => {
            Some(UserTaskEventPayload::ConversationReplaced {
                conversation: serde_json::to_value(conversation).unwrap_or(serde_json::Value::Null),
            })
        }
        AgentEvent::ModelChange { model, mode } => Some(UserTaskEventPayload::ModelChange {
            model: model.clone(),
            mode: mode.clone(),
        }),
        AgentEvent::McpNotification((request_id, notification)) => {
            Some(UserTaskEventPayload::Notification {
                request_id: request_id.clone(),
                message: serde_json::to_value(notification).unwrap_or(serde_json::Value::Null),
            })
        }
        AgentEvent::ToolTransportRequest(_) => None,
    }
}

fn preview_of(message: &Message) -> String {
    use agime::conversation::message::MessageContent;
    let mut buf = String::new();
    for content in &message.content {
        if let MessageContent::Text(text) = content {
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(&text.text);
        }
    }
    buf.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(task_id: &str, seq: u64) -> UserTaskEvent {
        UserTaskEvent {
            task_id: task_id.to_string(),
            seq,
            payload: UserTaskEventPayload::Started,
        }
    }

    #[test]
    fn status_terminal_classification() {
        assert!(!UserTaskStatus::Pending.is_terminal());
        assert!(!UserTaskStatus::Running.is_terminal());
        assert!(UserTaskStatus::Completed.is_terminal());
        assert!(UserTaskStatus::Failed.is_terminal());
        assert!(UserTaskStatus::Cancelled.is_terminal());
    }

    #[tokio::test]
    async fn emit_assigns_monotonic_seq_and_updates_snapshot() {
        let manager = DesktopTaskManager::new();
        let (tx, _) = broadcast::channel(16);
        {
            let mut map = manager.inner.lock().await;
            map.insert(
                "t1".to_string(),
                UserTaskRecord {
                    snapshot: UserTaskSnapshot {
                        task_id: "t1".to_string(),
                        session_id: "s1".to_string(),
                        status: UserTaskStatus::Running,
                        prompt_preview: "hi".to_string(),
                        last_event_id: 0,
                        error: None,
                        created_at: 0,
                        updated_at: 0,
                        finished_at: None,
                    },
                    cancel_token: CancellationToken::new(),
                    event_tx: tx,
                    replay: VecDeque::new(),
                    next_seq: 0,
                },
            );
        }

        manager.emit("t1", UserTaskEventPayload::Started).await;
        manager.emit("t1", UserTaskEventPayload::Completed).await;

        let snapshot = manager.get("t1").await.unwrap();
        assert_eq!(snapshot.last_event_id, 1);

        let (backlog, _rx, _status) = manager.subscribe("t1", None).await.unwrap();
        assert_eq!(backlog.len(), 2);
        assert_eq!(backlog[0].seq, 0);
        assert_eq!(backlog[1].seq, 1);
    }

    #[tokio::test]
    async fn subscribe_replays_only_after_seq() {
        let manager = DesktopTaskManager::new();
        let (tx, _) = broadcast::channel(16);
        {
            let mut map = manager.inner.lock().await;
            let mut replay = VecDeque::new();
            replay.push_back(make_event("t2", 0));
            replay.push_back(make_event("t2", 1));
            replay.push_back(make_event("t2", 2));
            map.insert(
                "t2".to_string(),
                UserTaskRecord {
                    snapshot: UserTaskSnapshot {
                        task_id: "t2".to_string(),
                        session_id: "s1".to_string(),
                        status: UserTaskStatus::Running,
                        prompt_preview: String::new(),
                        last_event_id: 2,
                        error: None,
                        created_at: 0,
                        updated_at: 0,
                        finished_at: None,
                    },
                    cancel_token: CancellationToken::new(),
                    event_tx: tx,
                    replay,
                    next_seq: 3,
                },
            );
        }

        let (backlog, _rx, _status) = manager.subscribe("t2", Some(0)).await.unwrap();
        assert_eq!(backlog.len(), 2);
        assert_eq!(backlog[0].seq, 1);
        assert_eq!(backlog[1].seq, 2);
    }

    #[tokio::test]
    async fn list_filters_by_session_and_sorts_newest_first() {
        let manager = DesktopTaskManager::new();
        {
            let mut map = manager.inner.lock().await;
            for (id, session, created) in [("a", "s1", 100), ("b", "s1", 200), ("c", "s2", 150)] {
                let (tx, _) = broadcast::channel(4);
                map.insert(
                    id.to_string(),
                    UserTaskRecord {
                        snapshot: UserTaskSnapshot {
                            task_id: id.to_string(),
                            session_id: session.to_string(),
                            status: UserTaskStatus::Running,
                            prompt_preview: String::new(),
                            last_event_id: 0,
                            error: None,
                            created_at: created,
                            updated_at: created,
                            finished_at: None,
                        },
                        cancel_token: CancellationToken::new(),
                        event_tx: tx,
                        replay: VecDeque::new(),
                        next_seq: 0,
                    },
                );
            }
        }

        let s1 = manager.list(Some("s1")).await;
        assert_eq!(s1.len(), 2);
        assert_eq!(s1[0].task_id, "b");
        assert_eq!(s1[1].task_id, "a");

        let all = manager.list(None).await;
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn cancel_unknown_task_errors() {
        let manager = DesktopTaskManager::new();
        assert!(manager.cancel("nope").await.is_err());
    }
}
