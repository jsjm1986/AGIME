//! MongoDB routes - User Groups API
//! Provides endpoints for team user group management

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

use super::teams::{can_manage_team, is_team_member, AppState};
use crate::models::mongo::user_group_mongo::*;
use crate::models::PaginatedResponse;
use crate::services::mongo::{TeamService, UserGroupService};
use crate::AuthenticatedUserId;

#[derive(Debug, Deserialize)]
pub struct ListGroupsQuery {
    #[serde(default = "crate::models::default_page")]
    pub page: u32,
    #[serde(default = "crate::models::default_limit")]
    pub limit: u32,
}

pub fn user_group_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/teams/{team_id}/groups", get(list_groups))
        .route("/teams/{team_id}/groups", post(create_group))
        .route("/teams/{team_id}/groups/{group_id}", get(get_group))
        .route("/teams/{team_id}/groups/{group_id}", put(update_group))
        .route("/teams/{team_id}/groups/{group_id}", delete(delete_group))
        .route(
            "/teams/{team_id}/groups/{group_id}/members",
            put(update_members),
        )
}

/// List user groups for a team
async fn list_groups(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<ListGroupsQuery>,
) -> Result<Json<PaginatedResponse<UserGroupSummary>>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Not a team member".to_string()));
    }

    let svc = UserGroupService::new((*state.db).clone());
    let (items, total) = svc
        .list(&team_id, q.page, q.limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(PaginatedResponse::new(items, total, q.page, q.limit)))
}

/// Create a user group
async fn create_group(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Json(req): Json<CreateUserGroupRequest>,
) -> Result<Json<UserGroupDetail>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only admin/owner can create groups".to_string(),
        ));
    }

    let svc = UserGroupService::new((*state.db).clone());
    let group = svc
        .create(&team_id, &user.0, req)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(Json(group))
}

/// Get a user group by ID
async fn get_group(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, group_id)): Path<(String, String)>,
) -> Result<Json<UserGroupDetail>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Not a team member".to_string()));
    }

    let svc = UserGroupService::new((*state.db).clone());
    let group = svc
        .get(&team_id, &group_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Group not found".to_string()))?;

    Ok(Json(group))
}

/// Update a user group
async fn update_group(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, group_id)): Path<(String, String)>,
    Json(req): Json<UpdateUserGroupRequest>,
) -> Result<Json<UserGroupDetail>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only admin/owner can update groups".to_string(),
        ));
    }

    let svc = UserGroupService::new((*state.db).clone());
    let group = svc
        .update(&team_id, &group_id, req)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Group not found".to_string()))?;

    Ok(Json(group))
}

/// Delete a user group (soft delete)
async fn delete_group(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, group_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only admin/owner can delete groups".to_string(),
        ));
    }

    let svc = UserGroupService::new((*state.db).clone());
    let deleted = svc
        .delete(&team_id, &group_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "Group not found".to_string()))
    }
}

/// Update group members (add/remove)
async fn update_members(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, group_id)): Path<(String, String)>,
    Json(req): Json<UpdateGroupMembersRequest>,
) -> Result<Json<UserGroupDetail>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only admin/owner can manage group members".to_string(),
        ));
    }

    let svc = UserGroupService::new((*state.db).clone());
    let group = svc
        .update_members(&team_id, &group_id, req)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Group not found".to_string()))?;

    Ok(Json(group))
}
