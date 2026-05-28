//! Minimal long-running task observation route (Milestone C).
//!
//! The desktop's `DesktopTaskRuntime` wraps the core
//! [`agime::agents::harness::TaskRuntime`], which is harness's *internal*
//! sub-agent / swarm-worker / validation-worker dispatcher — not a
//! user-level "submit a prompt and let it run for hours" task manager.
//! The team-server's user-level `TaskManager` (with Mongo persistence
//! and user-visible task UUIDs) is a separate construct.
//!
//! This route therefore only exposes the minimum observability the
//! desktop UI can use today: a query that tells the chat composer
//! whether the current session has any sub-agent / swarm worker
//! task in flight, so the "send" button can be disabled mid-turn.
//!
//! When the desktop later grows a true user-facing long-running task
//! manager (with SQLite persistence + attach SSE), the additional
//! endpoints (`POST /tasks`, `GET /tasks`, `GET /tasks/:id`,
//! `GET /tasks/:id/stream`, `POST /tasks/:id/cancel`) will land here.
//!
//! NOTE: this route is intentionally *not* registered in `openapi.rs`.
//! `scripts/check-openapi-schema.sh` compares the generated schema
//! against the committed `ui/desktop/openapi.json`, which is built
//! without the `desktop_harness_host` feature. Adding feature-gated
//! paths would cause CI schema drift. The desktop client should call
//! this route via plain `fetch` until the feature becomes default.

use crate::state::AppState;
use axum::{
    extract::{Query, State},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/tasks/active", get(active_tasks))
        .with_state(state)
}
