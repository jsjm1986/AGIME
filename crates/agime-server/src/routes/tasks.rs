//! User-level long-running task routes (Milestone C).
//!
//! These routes expose the desktop's *user-level* long-running task manager
//! ([`crate::host_task_manager::DesktopTaskManager`]) — submit a prompt, let
//! it run a detached background turn through the full `run_harness_host` loop,
//! list/inspect/cancel it, and attach to a live SSE event stream.
//!
//! This is distinct from the harness-internal sub-agent dispatcher exposed by
//! [`GET /tasks/active`], which only reports whether a chat turn currently has
//! sub-agent / swarm worker tasks in flight (so the composer can disable the
//! send button mid-turn).
//!
//! Endpoints:
//! - `POST   /tasks`            — submit a task, returns `{ task_id }`
//! - `GET    /tasks`            — list tasks (optionally `?session_id=`)
//! - `GET    /tasks/:id`        — fetch a task snapshot
//! - `POST   /tasks/:id/cancel` — cancel a running task
//! - `GET    /tasks/:id/stream` — attach to the live SSE event stream,
//!                                replaying from `?last_event_id=` if given
//! - `GET    /tasks/active`     — (pre-existing) sub-agent activity probe
//!
//! NOTE: these routes are intentionally *not* registered in `openapi.rs`.
//! `scripts/check-openapi-schema.sh` compares the generated schema against the
//! committed `ui/desktop/openapi.json`, which is built **without** the
//! `desktop_harness_host` feature. Adding feature-gated paths would cause CI
//! schema drift. The desktop client should call these routes via plain
//! `fetch` / `EventSource` until the feature becomes the schema default.

use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{self, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::{
    convert::Infallible,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ActiveQuery {
    pub parent_session_id: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ActiveTasksResponse {
    pub parent_session_id: String,
    pub has_active: bool,
}

#[utoipa::path(
    get,
    path = "/tasks/active",
    params(ActiveQuery),
    responses(
        (status = 200, description = "Returns whether the given parent session has any sub-agent or worker task currently active.", body = ActiveTasksResponse),
    ),
    tag = "tasks"
)]
pub async fn active_tasks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ActiveQuery>,
) -> impl IntoResponse {
    let runtime = state.task_runtime().await;
    let has_active = runtime.has_active_tasks(&query.parent_session_id);
    Json(ActiveTasksResponse {
        parent_session_id: query.parent_session_id,
        has_active,
    })
}

#[derive(Debug, Deserialize)]
pub struct SubmitTaskRequest {
    pub session_id: String,
    /// The user's prompt. Sent as a single user text message to the harness.
    pub prompt: String,
}

#[derive(Debug, Serialize)]
pub struct SubmitTaskResponse {
    pub task_id: String,
}

/// Submit a new long-running task. Returns immediately with the generated
/// `task_id`; the work runs in a detached background turn.
pub async fn submit_task(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SubmitTaskRequest>,
) -> Result<Json<SubmitTaskResponse>, StatusCode> {
    use agime::conversation::message::Message;

    if request.prompt.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let manager = state.task_manager().await;
    let user_message = Message::user().with_text(&request.prompt);
    let task_id = manager
        .submit(state.clone(), request.session_id, user_message)
        .await
        .map_err(|e| {
            tracing::error!("failed to submit task: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(SubmitTaskResponse { task_id }))
}

#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    pub session_id: Option<String>,
}

/// List tasks, optionally filtered to a single session, newest first.
pub async fn list_tasks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListTasksQuery>,
) -> impl IntoResponse {
    let manager = state.task_manager().await;
    let tasks = manager.list(query.session_id.as_deref()).await;
    Json(tasks)
}

/// Fetch the current snapshot for a single task.
pub async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let manager = state.task_manager().await;
    match manager.get(&task_id).await {
        Some(snapshot) => Ok(Json(snapshot)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// Cancel a running task. 404 if the task does not exist; 200 if cancelled or
/// already terminal.
pub async fn cancel_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> StatusCode {
    let manager = state.task_manager().await;
    match manager.cancel(&task_id).await {
        Ok(()) => StatusCode::OK,
        Err(_) => StatusCode::NOT_FOUND,
    }
}

#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    /// Resume after this event sequence number. Events with `seq <= this` are
    /// not replayed.
    pub last_event_id: Option<u64>,
}

/// SSE attach to a task's live event stream. Replays any buffered events newer
/// than `last_event_id`, then forwards live events until the task reaches a
/// terminal state (the terminal event is the last frame sent).
pub async fn stream_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    Query(query): Query<StreamQuery>,
) -> Result<TaskSseResponse, StatusCode> {
    use crate::host_task_manager::{UserTaskEvent, UserTaskEventPayload};

    let manager = state.task_manager().await;
    let Some((backlog, mut receiver, _status)) =
        manager.subscribe(&task_id, query.last_event_id).await
    else {
        return Err(StatusCode::NOT_FOUND);
    };

    let (tx, rx) = mpsc::channel::<String>(256);

    fn is_terminal_payload(payload: &UserTaskEventPayload) -> bool {
        matches!(
            payload,
            UserTaskEventPayload::Completed
                | UserTaskEventPayload::Failed { .. }
                | UserTaskEventPayload::Cancelled
        )
    }

    async fn send_event(tx: &mpsc::Sender<String>, event: &UserTaskEvent) -> bool {
        let json = serde_json::to_string(event)
            .unwrap_or_else(|e| format!(r#"{{"kind":"failed","error":"serialize: {}"}}"#, e));
        // SSE `id:` lets the browser EventSource resume via Last-Event-ID.
        let frame = format!("id: {}\ndata: {}\n\n", event.seq, json);
        tx.send(frame).await.is_ok()
    }

    drop(tokio::spawn(async move {
        // 1. Drain the replay backlog first.
        for event in backlog {
            let terminal = is_terminal_payload(&event.payload);
            if !send_event(&tx, &event).await {
                return;
            }
            if terminal {
                return;
            }
        }

        // 2. Forward live events until terminal or the channel closes. If the
        // task was already terminal at subscribe time, the terminal event was
        // either in the backlog above (and we already returned) or will arrive
        // here before the sender drops and closes the channel.
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let terminal = is_terminal_payload(&event.payload);
                    if !send_event(&tx, &event).await {
                        return;
                    }
                    if terminal {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Client fell behind the channel buffer. Tell it to
                    // re-fetch the snapshot, then keep going.
                    let _ = tx.send("data: {\"kind\":\"lagged\"}\n\n".to_string()).await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    }));

    Ok(TaskSseResponse {
        rx: ReceiverStream::new(rx),
    })
}

/// Minimal `text/event-stream` response wrapper. Mirrors the `SseResponse`
/// shape in `routes/reply.rs`.
pub struct TaskSseResponse {
    rx: ReceiverStream<String>,
}

impl Stream for TaskSseResponse {
    type Item = Result<Bytes, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.rx)
            .poll_next(cx)
            .map(|opt| opt.map(|s| Ok(Bytes::from(s))))
    }
}

impl IntoResponse for TaskSseResponse {
    fn into_response(self) -> axum::response::Response {
        let body = axum::body::Body::from_stream(self);
        http::Response::builder()
            .header("Content-Type", "text/event-stream")
            .header("Cache-Control", "no-cache")
            .header("Connection", "keep-alive")
            .body(body)
            .unwrap()
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/tasks/active", get(active_tasks))
        .route("/tasks", post(submit_task).get(list_tasks))
        .route("/tasks/{id}", get(get_task))
        .route("/tasks/{id}/cancel", post(cancel_task))
        .route("/tasks/{id}/stream", get(stream_task))
        .with_state(state)
}
