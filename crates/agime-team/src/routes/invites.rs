//! Invite routes for team invitation management

use crate::error::TeamError;
use crate::models::{AcceptInviteResponse, CreateInviteRequest, MemberRole, TeamInvite, ValidateInviteResponse};
use crate::services::{InviteService, MemberService};
use crate::AuthenticatedUserId;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Extension, Json, Router,
};
use serde::Deserialize;
use sqlx::SqlitePool;
use std::sync::Arc;

/// State for invite routes
#[derive(Clone)]
pub struct InviteRoutesState {
    pub pool: Arc<SqlitePool>,
    pub user_id: String,
    pub base_url: String,
}

/// Helper function to get user ID from Extension or fallback to state
fn get_invite_user_id(auth_user: Option<&AuthenticatedUserId>, state: &InviteRoutesState) -> String {
    auth_user
        .map(|u| u.0.clone())
        .unwrap_or_else(|| state.user_id.clone())
}

/// Configure invite routes
pub fn configure(pool: Arc<SqlitePool>, user_id: String, base_url: String) -> Router {
    let state = InviteRoutesState {
        pool,
        user_id,
        base_url,
    };

    Router::new()
        // Public routes (no team ID needed)
        .route("/invites/{code}", get(validate_invite))
        .route("/invites/{code}/accept", post(accept_invite))
        // Team-specific routes
        .route("/teams/{team_id}/invites", get(list_invites))
        .route("/teams/{team_id}/invites", post(create_invite))
        .route("/teams/{team_id}/invites/{code}", delete(delete_invite))
        .with_state(state)
}

/// Validate an invite code (public - no auth needed for viewing)
async fn validate_invite(
    State(state): State<InviteRoutesState>,
    Path(code): Path<String>,
) -> Result<impl IntoResponse, TeamError> {
    let response = InviteService::validate_invite(&state.pool, &code).await?;
    Ok(Json(response))
}

/// Accept an invite and join the team
#[derive(Debug, Deserialize)]
pub struct AcceptInviteRequest {
    /// Display name for the new member
    pub display_name: Option<String>,
}

async fn accept_invite(
    State(state): State<InviteRoutesState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(code): Path<String>,
    Json(request): Json<AcceptInviteRequest>,
) -> Result<impl IntoResponse, TeamError> {
    let display_name = request.display_name.unwrap_or_else(|| "New Member".to_string());
    let user_id = get_invite_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    let response = InviteService::accept_invite(
        &state.pool,
        &code,
        &user_id,
        &display_name,
    )
    .await?;

    if response.success {
        Ok((StatusCode::OK, Json(response)))
    } else {
        Ok((StatusCode::BAD_REQUEST, Json(response)))
    }
}

/// List all invites for a team
async fn list_invites(
    State(state): State<InviteRoutesState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(team_id): Path<String>,
) -> Result<impl IntoResponse, TeamError> {
    let user_id = get_invite_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    // Verify caller has permission (must be Owner or Admin)
    let member_service = MemberService::new();
    let caller = member_service.get_member_by_user(&state.pool, &team_id, &user_id).await?;
    if !matches!(caller.role, MemberRole::Owner | MemberRole::Admin) {
        return Err(TeamError::PermissionDenied {
            action: "list invites".to_string(),
        });
    }

    let invites = InviteService::list_invites(&state.pool, &team_id).await?;

    // Filter to only valid invites for the response
    let valid_invites: Vec<_> = invites.into_iter().filter(|i| i.is_valid()).collect();

    Ok(Json(serde_json::json!({
        "invites": valid_invites,
        "total": valid_invites.len()
    })))
}

/// Create a new invite for a team
async fn create_invite(
    State(state): State<InviteRoutesState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(team_id): Path<String>,
    Json(request): Json<CreateInviteRequest>,
) -> Result<impl IntoResponse, TeamError> {
    let user_id = get_invite_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    // Verify caller has permission (must be Owner or Admin)
    let member_service = MemberService::new();
    let caller = member_service.get_member_by_user(&state.pool, &team_id, &user_id).await?;
    if !matches!(caller.role, MemberRole::Owner | MemberRole::Admin) {
        return Err(TeamError::PermissionDenied {
            action: "create invite".to_string(),
        });
    }

    let response = InviteService::create_invite(
        &state.pool,
        &team_id,
        &user_id,
        request,
        &state.base_url,
    )
    .await?;

    Ok((StatusCode::CREATED, Json(response)))
}

/// Delete (revoke) an invite
async fn delete_invite(
    State(state): State<InviteRoutesState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path((team_id, code)): Path<(String, String)>,
) -> Result<impl IntoResponse, TeamError> {
    let user_id = get_invite_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    // Verify caller has permission (must be Owner or Admin)
    let member_service = MemberService::new();
    let caller = member_service.get_member_by_user(&state.pool, &team_id, &user_id).await?;
    if !matches!(caller.role, MemberRole::Owner | MemberRole::Admin) {
        return Err(TeamError::PermissionDenied {
            action: "delete invite".to_string(),
        });
    }

    let deleted = InviteService::delete_invite(&state.pool, &code, &team_id).await?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(TeamError::InviteNotFound("Invite not found".to_string()))
    }
}
