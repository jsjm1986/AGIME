use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::models::mongo::{ExternalUserStatus, PaginatedResponse};
use crate::routes::mongo::teams::{can_manage_team, AppState};
use crate::services::mongo::{ExternalUserService, TeamService};
use crate::AuthenticatedUserId;

#[derive(Debug, Deserialize)]
struct ListExternalUsersQuery {
    #[serde(default = "default_page")]
    page: u32,
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    search: Option<String>,
    #[serde(default)]
    status: Option<ExternalUserStatus>,
}

#[derive(Debug, Deserialize)]
struct ListExternalUserEventsQuery {
    #[serde(default)]
    external_user_id: Option<String>,
    #[serde(default)]
    event_type: Option<String>,
    #[serde(default = "default_page")]
    page: u32,
    #[serde(default = "default_limit")]
    limit: u32,
}

#[derive(Debug, Deserialize)]
struct ResetPasswordRequest {
    new_password: String,
}

fn default_page() -> u32 {
    1
}

fn default_limit() -> u32 {
    50
}

pub fn external_user_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/teams/{team_id}/external-users", get(list_external_users))
        .route(
            "/teams/{team_id}/external-users/events",
            get(list_external_user_events),
        )
        .route(
            "/teams/{team_id}/external-users/{external_user_id}",
            get(get_external_user_detail),
        )
        .route(
            "/teams/{team_id}/external-users/{external_user_id}/disable",
            post(disable_external_user),
        )
        .route(
            "/teams/{team_id}/external-users/{external_user_id}/enable",
            post(enable_external_user),
        )
        .route(
            "/teams/{team_id}/external-users/{external_user_id}/reset-password",
            post(reset_external_user_password),
        )
}

async fn ensure_team_manager(
    state: &Arc<AppState>,
    team_id: &str,
    user_id: &str,
) -> Result<(), (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".into()))?;
    if !can_manage_team(&team, user_id) {
        return Err((StatusCode::FORBIDDEN, "Forbidden".into()));
    }
    Ok(())
}

async fn list_external_users(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(query): Query<ListExternalUsersQuery>,
) -> Result<Json<PaginatedResponse<crate::models::mongo::ExternalUserSummary>>, (StatusCode, String)>
{
    ensure_team_manager(&state, &team_id, user.as_str()).await?;
    let service = ExternalUserService::new(state.db.clone());
    let result = service
        .list_users(
            &team_id,
            query.page,
            query.limit,
            query.search.as_deref(),
            query.status,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(result))
}

async fn get_external_user_detail(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, external_user_id)): Path<(String, String)>,
) -> Result<Json<crate::models::mongo::ExternalUserDetail>, (StatusCode, String)> {
    ensure_team_manager(&state, &team_id, user.as_str()).await?;
    let service = ExternalUserService::new(state.db.clone());
    let detail = service
        .get_user_detail(&team_id, &external_user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "User not found".into()))?;
    Ok(Json(detail))
}

async fn list_external_user_events(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(query): Query<ListExternalUserEventsQuery>,
) -> Result<
    Json<PaginatedResponse<crate::models::mongo::ExternalUserEventResponse>>,
    (StatusCode, String),
> {
    ensure_team_manager(&state, &team_id, user.as_str()).await?;
    let service = ExternalUserService::new(state.db.clone());
    let events = service
        .list_events(
            &team_id,
            query.external_user_id.as_deref(),
            query.event_type.as_deref(),
            query.page,
            query.limit,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(events))
}

async fn disable_external_user(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, external_user_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    ensure_team_manager(&state, &team_id, user.as_str()).await?;
    let service = ExternalUserService::new(state.db.clone());
    service
        .set_user_status(
            &team_id,
            &external_user_id,
            ExternalUserStatus::Disabled,
            user.as_str(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn enable_external_user(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, external_user_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    ensure_team_manager(&state, &team_id, user.as_str()).await?;
    let service = ExternalUserService::new(state.db.clone());
    service
        .set_user_status(
            &team_id,
            &external_user_id,
            ExternalUserStatus::Active,
            user.as_str(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reset_external_user_password(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, external_user_id)): Path<(String, String)>,
    Json(body): Json<ResetPasswordRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    ensure_team_manager(&state, &team_id, user.as_str()).await?;
    let service = ExternalUserService::new(state.db.clone());
    service
        .reset_password(
            &team_id,
            &external_user_id,
            &body.new_password,
            user.as_str(),
        )
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}
