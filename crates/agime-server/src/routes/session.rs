use crate::routes::errors::ErrorResponse;
use crate::routes::recipe_utils::{apply_recipe_to_agent, build_recipe_with_parameter_values};
use crate::state::AppState;
use agime::recipe::Recipe;
use agime::session::session_manager::{
    CfpmToolGateEventRecord, MemoryCandidate, MemoryFact, MemoryFactDraft, MemoryFactPatch,
    MemorySnapshotRecord, SessionInsights,
};
use agime::session::{Session, SessionManager};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{
    extract::Path,
    http::StatusCode,
    routing::{delete, get, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// Query parameters for listing sessions with pagination
#[derive(Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsQuery {
    /// Maximum number of sessions to return (default: 50, max: 200)
    pub limit: Option<i64>,
    /// Cursor for pagination - return sessions updated before this timestamp (ISO 8601 format)
    pub before: Option<DateTime<Utc>>,
    /// Filter to only return favorited sessions
    pub favorites_only: Option<bool>,
    /// Filter by tags (comma-separated list)
    pub tags: Option<String>,
    /// Filter by working directory (exact match)
    pub working_dir: Option<String>,
    /// Filter sessions updated after this date (ISO 8601 format)
    pub date_from: Option<DateTime<Utc>>,
    /// Filter sessions updated before this date (ISO 8601 format)
    pub date_to: Option<DateTime<Utc>>,
    /// Filter by specific dates (comma-separated YYYY-MM-DD dates, e.g. "2024-01-03,2024-01-05")
    pub dates: Option<String>,
    /// Timezone offset in minutes (from JS getTimezoneOffset(), e.g., -480 for UTC+8)
    pub timezone_offset: Option<i32>,
    /// Sort field: updated_at (default), created_at, message_count, total_tokens
    pub sort_by: Option<String>,
    /// Sort order: desc (default), asc
    pub sort_order: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedSessionListResponse {
    /// List of session information objects
    pub sessions: Vec<Session>,
    /// Whether there are more sessions available
    pub has_more: bool,
    /// Cursor for the next page (updated_at of the last session)
    pub next_cursor: Option<String>,
    /// Total count of sessions matching the filter criteria
    pub total_count: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionListResponse {
    /// List of available session information objects
    sessions: Vec<Session>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSessionNameRequest {
    /// Updated name for the session (max 200 characters)
    name: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSessionUserRecipeValuesRequest {
    /// Recipe parameter values entered by the user
    user_recipe_values: HashMap<String, String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UpdateSessionUserRecipeValuesResponse {
    recipe: Recipe,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportSessionRequest {
    json: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSessionMetadataRequest {
    /// Whether the session is marked as favorite
    #[serde(skip_serializing_if = "Option::is_none")]
    is_favorite: Option<bool>,
    /// Tags assigned to the session
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadataResponse {
    /// Whether the session is marked as favorite
    is_favorite: bool,
    /// Tags assigned to the session
    tags: Vec<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AllTagsResponse {
    /// All unique tags across all sessions
    tags: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum EditType {
    Fork,
    Edit,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EditMessageRequest {
    timestamp: i64,
    #[serde(default = "default_edit_type")]
    edit_type: EditType,
}

fn default_edit_type() -> EditType {
    EditType::Fork
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EditMessageResponse {
    session_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateMemoryFactRequest {
    category: String,
    content: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    pinned: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RenameMemoryPathRequest {
    from_path: String,
    to_path: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RenameMemoryPathResponse {
    updated_count: u64,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RollbackMemorySnapshotRequest {
    snapshot_id: i64,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListMemoryCandidatesQuery {
    decision: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListMemoryToolGatesQuery {
    limit: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RollbackMemorySnapshotResponse {
    restored_count: u64,
}

const MAX_NAME_LENGTH: usize = 200;

#[utoipa::path(
    get,
    path = "/sessions",
    params(ListSessionsQuery),
    responses(
        (status = 200, description = "List of available sessions retrieved successfully", body = PaginatedSessionListResponse),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn list_sessions(
    Query(query): Query<ListSessionsQuery>,
) -> Result<Json<PaginatedSessionListResponse>, StatusCode> {
    let limit = query.limit.unwrap_or(50).min(200).max(1);
    let favorites_only = query.favorites_only.unwrap_or(false);
    let tags: Option<Vec<String>> = query.tags.map(|t| {
        t.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });
    let sort_by = query.sort_by.unwrap_or_else(|| "updated_at".to_string());
    let sort_order = query.sort_order.unwrap_or_else(|| "desc".to_string());

    // Parse discrete dates if provided (format: YYYY-MM-DD)
    let dates: Option<Vec<String>> = query
        .dates
        .map(|d| {
            d.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty());

    // Query limit + 1 to accurately determine if there are more records
    let (mut sessions, total_count) = SessionManager::list_sessions_paginated(
        limit + 1,
        query.before,
        favorites_only,
        tags,
        query.working_dir,
        query.date_from,
        query.date_to,
        dates,
        query.timezone_offset,
        sort_by,
        sort_order,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Check if we got more than requested limit
    let has_more = sessions.len() as i64 > limit;
    if has_more {
        sessions.pop(); // Remove the extra record used for has_more detection
    }

    let next_cursor = if has_more {
        sessions.last().map(|s| s.updated_at.to_rfc3339())
    } else {
        None
    };

    Ok(Json(PaginatedSessionListResponse {
        sessions,
        has_more,
        next_cursor,
        total_count,
    }))
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}",
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Session history retrieved successfully", body = Session),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn get_session(Path(session_id): Path<String>) -> Result<Json<Session>, StatusCode> {
    let session = SessionManager::get_session(&session_id, true)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(session))
}
#[utoipa::path(
    get,
    path = "/sessions/insights",
    responses(
        (status = 200, description = "Session insights retrieved successfully", body = SessionInsights),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn get_session_insights() -> Result<Json<SessionInsights>, StatusCode> {
    let insights = SessionManager::get_insights()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(insights))
}

#[utoipa::path(
    put,
    path = "/sessions/{session_id}/name",
    request_body = UpdateSessionNameRequest,
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Session name updated successfully"),
        (status = 400, description = "Bad request - Name too long (max 200 characters)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn update_session_name(
    Path(session_id): Path<String>,
    Json(request): Json<UpdateSessionNameRequest>,
) -> Result<StatusCode, StatusCode> {
    let name = request.name.trim();
    if name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if name.len() > MAX_NAME_LENGTH {
        return Err(StatusCode::BAD_REQUEST);
    }

    SessionManager::update_session(&session_id)
        .user_provided_name(name.to_string())
        .apply()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

#[utoipa::path(
    put,
    path = "/sessions/{session_id}/user_recipe_values",
    request_body = UpdateSessionUserRecipeValuesRequest,
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Session user recipe values updated successfully", body = UpdateSessionUserRecipeValuesResponse),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
// Update session user recipe parameter values
async fn update_session_user_recipe_values(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<UpdateSessionUserRecipeValuesRequest>,
) -> Result<Json<UpdateSessionUserRecipeValuesResponse>, ErrorResponse> {
    SessionManager::update_session(&session_id)
        .user_recipe_values(Some(request.user_recipe_values))
        .apply()
        .await
        .map_err(|err| ErrorResponse {
            message: err.to_string(),
            status: StatusCode::INTERNAL_SERVER_ERROR,
        })?;

    let session = SessionManager::get_session(&session_id, false)
        .await
        .map_err(|err| ErrorResponse {
            message: err.to_string(),
            status: StatusCode::INTERNAL_SERVER_ERROR,
        })?;
    let recipe = session.recipe.ok_or_else(|| ErrorResponse {
        message: "Recipe not found".to_string(),
        status: StatusCode::NOT_FOUND,
    })?;

    let user_recipe_values = session.user_recipe_values.unwrap_or_default();
    match build_recipe_with_parameter_values(&recipe, user_recipe_values).await {
        Ok(Some(recipe)) => {
            let agent = state
                .get_agent_for_route(session_id.clone())
                .await
                .map_err(|status| ErrorResponse {
                    message: format!("Failed to get agent: {}", status),
                    status,
                })?;
            if let Some(prompt) = apply_recipe_to_agent(&agent, &recipe, false).await {
                agent.extend_system_prompt(prompt).await;
            }
            Ok(Json(UpdateSessionUserRecipeValuesResponse { recipe }))
        }
        Ok(None) => Err(ErrorResponse {
            message: "Missing required parameters".to_string(),
            status: StatusCode::BAD_REQUEST,
        }),
        Err(e) => Err(ErrorResponse {
            message: e.to_string(),
            status: StatusCode::INTERNAL_SERVER_ERROR,
        }),
    }
}

#[utoipa::path(
    delete,
    path = "/sessions/{session_id}",
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Session deleted successfully"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn delete_session(Path(session_id): Path<String>) -> Result<StatusCode, StatusCode> {
    SessionManager::delete_session(&session_id)
        .await
        .map_err(|e| {
            if e.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })?;

    Ok(StatusCode::OK)
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/export",
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Session exported successfully", body = String),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn export_session(Path(session_id): Path<String>) -> Result<Json<String>, StatusCode> {
    let exported = SessionManager::export_session(&session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(exported))
}

#[utoipa::path(
    post,
    path = "/sessions/import",
    request_body = ImportSessionRequest,
    responses(
        (status = 200, description = "Session imported successfully", body = Session),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 400, description = "Bad request - Invalid JSON"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn import_session(
    Json(request): Json<ImportSessionRequest>,
) -> Result<Json<Session>, StatusCode> {
    let session = SessionManager::import_session(&request.json)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    Ok(Json(session))
}

#[utoipa::path(
    post,
    path = "/sessions/{session_id}/edit_message",
    request_body = EditMessageRequest,
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Session prepared for editing - frontend should submit the edited message", body = EditMessageResponse),
        (status = 400, description = "Bad request - Invalid message timestamp"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session or message not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn edit_message(
    Path(session_id): Path<String>,
    Json(request): Json<EditMessageRequest>,
) -> Result<Json<EditMessageResponse>, StatusCode> {
    match request.edit_type {
        EditType::Fork => {
            let new_session = SessionManager::copy_session(&session_id, "(edited)".to_string())
                .await
                .map_err(|e| {
                    tracing::error!("Failed to copy session: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;

            SessionManager::truncate_conversation(&new_session.id, request.timestamp)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to truncate conversation: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;

            Ok(Json(EditMessageResponse {
                session_id: new_session.id,
            }))
        }
        EditType::Edit => {
            SessionManager::truncate_conversation(&session_id, request.timestamp)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to truncate conversation: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;

            Ok(Json(EditMessageResponse {
                session_id: session_id.clone(),
            }))
        }
    }
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/memory/facts",
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Memory facts retrieved successfully", body = Vec<MemoryFact>),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn get_memory_facts(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<MemoryFact>>, StatusCode> {
    let facts = SessionManager::list_memory_facts(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(facts))
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/memory/candidates",
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session"),
        ListMemoryCandidatesQuery
    ),
    responses(
        (status = 200, description = "Memory candidates retrieved successfully", body = Vec<MemoryCandidate>),
        (status = 400, description = "Bad request - Invalid query"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn get_memory_candidates(
    Path(session_id): Path<String>,
    Query(query): Query<ListMemoryCandidatesQuery>,
) -> Result<Json<Vec<MemoryCandidate>>, StatusCode> {
    let decision = query
        .decision
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());

    if let Some(ref value) = decision {
        if value != "accepted" && value != "rejected" {
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    let limit = query.limit.map(|value| value.clamp(1, 500));
    let candidates =
        SessionManager::list_memory_candidates(&session_id, decision.as_deref(), limit)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(candidates))
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/memory/tool-gates",
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session"),
        ListMemoryToolGatesQuery
    ),
    responses(
        (status = 200, description = "CFPM tool gate events retrieved successfully", body = Vec<CfpmToolGateEventRecord>),
        (status = 400, description = "Bad request - Invalid query"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn get_memory_tool_gates(
    Path(session_id): Path<String>,
    Query(query): Query<ListMemoryToolGatesQuery>,
) -> Result<Json<Vec<CfpmToolGateEventRecord>>, StatusCode> {
    let limit = query.limit.map(|value| value.clamp(1, 200));
    let events = SessionManager::list_recent_cfpm_tool_gate_events(&session_id, limit)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(events))
}

#[utoipa::path(
    post,
    path = "/sessions/{session_id}/memory/facts",
    request_body = CreateMemoryFactRequest,
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Memory fact created successfully", body = MemoryFact),
        (status = 400, description = "Bad request - Invalid memory fact payload"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn create_memory_fact(
    Path(session_id): Path<String>,
    Json(request): Json<CreateMemoryFactRequest>,
) -> Result<Json<MemoryFact>, StatusCode> {
    let draft = MemoryFactDraft {
        category: request.category,
        content: request.content,
        source: request.source.unwrap_or_else(|| "user".to_string()),
        pinned: request.pinned.unwrap_or(false),
        confidence: None,
        evidence_count: None,
        last_validated_at: None,
        validation_command: None,
    };

    SessionManager::create_memory_fact(&session_id, draft)
        .await
        .map(Json)
        .map_err(|err| {
            let lower = err.to_string().to_ascii_lowercase();
            if lower.contains("not found") {
                StatusCode::NOT_FOUND
            } else if lower.contains("cannot be empty") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })
}

#[utoipa::path(
    patch,
    path = "/sessions/{session_id}/memory/facts/{fact_id}",
    request_body = MemoryFactPatch,
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session"),
        ("fact_id" = String, Path, description = "Unique identifier for the memory fact")
    ),
    responses(
        (status = 200, description = "Memory fact updated successfully", body = MemoryFact),
        (status = 400, description = "Bad request - Invalid memory fact payload"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session or memory fact not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn update_memory_fact(
    Path((session_id, fact_id)): Path<(String, String)>,
    Json(patch): Json<MemoryFactPatch>,
) -> Result<Json<MemoryFact>, StatusCode> {
    SessionManager::update_memory_fact(&session_id, &fact_id, patch)
        .await
        .map(Json)
        .map_err(|err| {
            let lower = err.to_string().to_ascii_lowercase();
            if lower.contains("not found") {
                StatusCode::NOT_FOUND
            } else if lower.contains("cannot be empty") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })
}

#[utoipa::path(
    post,
    path = "/sessions/{session_id}/memory/path-rename",
    request_body = RenameMemoryPathRequest,
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Memory path rename completed", body = RenameMemoryPathResponse),
        (status = 400, description = "Bad request - Invalid rename payload"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn rename_memory_paths(
    Path(session_id): Path<String>,
    Json(request): Json<RenameMemoryPathRequest>,
) -> Result<Json<RenameMemoryPathResponse>, StatusCode> {
    SessionManager::rename_memory_paths(&session_id, &request.from_path, &request.to_path)
        .await
        .map(|updated_count| Json(RenameMemoryPathResponse { updated_count }))
        .map_err(|err| {
            let lower = err.to_string().to_ascii_lowercase();
            if lower.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/memory/snapshots",
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Memory snapshots retrieved successfully", body = Vec<MemorySnapshotRecord>),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn get_memory_snapshots(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<MemorySnapshotRecord>>, StatusCode> {
    let snapshots = SessionManager::list_memory_snapshots(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(snapshots))
}

#[utoipa::path(
    post,
    path = "/sessions/{session_id}/memory/rollback",
    request_body = RollbackMemorySnapshotRequest,
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Memory snapshot rollback completed", body = RollbackMemorySnapshotResponse),
        (status = 400, description = "Bad request - Invalid rollback payload"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session or snapshot not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn rollback_memory_snapshot(
    Path(session_id): Path<String>,
    Json(request): Json<RollbackMemorySnapshotRequest>,
) -> impl IntoResponse {
    match SessionManager::rollback_memory_snapshot(&session_id, request.snapshot_id).await {
        Ok(restored_count) => Ok(Json(RollbackMemorySnapshotResponse { restored_count })),
        Err(err) => {
            let lower = err.to_string().to_ascii_lowercase();
            if lower.contains("not found") {
                Err(StatusCode::NOT_FOUND)
            } else {
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }
}

// Constants for metadata keys in extension_data
const FAVORITES_KEY: &str = "favorites.v0";
const TAGS_KEY: &str = "tags.v0";

#[utoipa::path(
    put,
    path = "/sessions/{session_id}/metadata",
    request_body = UpdateSessionMetadataRequest,
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Session metadata updated successfully", body = SessionMetadataResponse),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn update_session_metadata(
    Path(session_id): Path<String>,
    Json(request): Json<UpdateSessionMetadataRequest>,
) -> Result<Json<SessionMetadataResponse>, StatusCode> {
    // Get the current session to retrieve existing extension_data
    let session = SessionManager::get_session(&session_id, false)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let mut extension_data = session.extension_data;

    // Update favorites if provided
    if let Some(is_favorite) = request.is_favorite {
        extension_data
            .extension_states
            .insert(FAVORITES_KEY.to_string(), serde_json::json!(is_favorite));
    }

    // Update tags if provided
    if let Some(tags) = request.tags {
        extension_data
            .extension_states
            .insert(TAGS_KEY.to_string(), serde_json::json!(tags));
    }

    // Save the updated extension_data
    SessionManager::update_session(&session_id)
        .extension_data(extension_data.clone())
        .apply()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Extract current values for response
    let is_favorite = extension_data
        .extension_states
        .get(FAVORITES_KEY)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let tags = extension_data
        .extension_states
        .get(TAGS_KEY)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(Json(SessionMetadataResponse { is_favorite, tags }))
}

#[utoipa::path(
    get,
    path = "/sessions/tags",
    responses(
        (status = 200, description = "All unique tags retrieved successfully", body = AllTagsResponse),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn get_all_tags() -> Result<Json<AllTagsResponse>, StatusCode> {
    let sessions = SessionManager::list_sessions()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut all_tags = std::collections::HashSet::new();

    for session in sessions {
        if let Some(tags_value) = session.extension_data.extension_states.get(TAGS_KEY) {
            if let Some(tags_array) = tags_value.as_array() {
                for tag in tags_array {
                    if let Some(tag_str) = tag.as_str() {
                        all_tags.insert(tag_str.to_string());
                    }
                }
            }
        }
    }

    let mut tags: Vec<String> = all_tags.into_iter().collect();
    tags.sort();

    Ok(Json(AllTagsResponse { tags }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        // Static routes first (to avoid matching as {session_id})
        .route("/sessions", get(list_sessions))
        .route("/sessions/import", post(import_session))
        .route("/sessions/insights", get(get_session_insights))
        .route("/sessions/tags", get(get_all_tags))
        // Dynamic routes after static ones
        .route("/sessions/{session_id}", get(get_session))
        .route("/sessions/{session_id}", delete(delete_session))
        .route("/sessions/{session_id}/export", get(export_session))
        .route("/sessions/{session_id}/name", put(update_session_name))
        .route(
            "/sessions/{session_id}/user_recipe_values",
            put(update_session_user_recipe_values),
        )
        .route("/sessions/{session_id}/memory/facts", get(get_memory_facts))
        .route(
            "/sessions/{session_id}/memory/candidates",
            get(get_memory_candidates),
        )
        .route(
            "/sessions/{session_id}/memory/tool-gates",
            get(get_memory_tool_gates),
        )
        .route(
            "/sessions/{session_id}/memory/facts",
            post(create_memory_fact),
        )
        .route(
            "/sessions/{session_id}/memory/facts/{fact_id}",
            axum::routing::patch(update_memory_fact),
        )
        .route(
            "/sessions/{session_id}/memory/path-rename",
            post(rename_memory_paths),
        )
        .route(
            "/sessions/{session_id}/memory/snapshots",
            get(get_memory_snapshots),
        )
        .route(
            "/sessions/{session_id}/memory/rollback",
            post(rollback_memory_snapshot),
        )
        .route(
            "/sessions/{session_id}/metadata",
            put(update_session_metadata),
        )
        .route("/sessions/{session_id}/edit_message", post(edit_message))
        .with_state(state)
}
