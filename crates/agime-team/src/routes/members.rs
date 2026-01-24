//! Members HTTP routes

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::error::TeamError;
use crate::models::{
    TeamMember, MemberRole, AddMemberRequest, ListMembersQuery,
};
use crate::services::{MemberService, CleanupService};
use crate::routes::teams::TeamState;
use crate::AuthenticatedUserId;
use super::get_user_id;

/// Query params for listing members
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMembersParams {
    pub page: Option<u32>,
    pub limit: Option<u32>,
}

/// Add member request (API)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddMemberApiRequest {
    pub user_id: String,
    pub display_name: String,
    pub role: Option<String>,
    pub endpoint_url: Option<String>,
}

/// Update member request (API)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMemberApiRequest {
    pub role: Option<String>,
    pub display_name: Option<String>,
}

/// Member response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberResponse {
    pub id: String,
    pub team_id: String,
    pub user_id: String,
    pub display_name: String,
    pub endpoint_url: Option<String>,
    pub role: String,
    pub status: String,
    pub joined_at: String,
}

/// Paginated members response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MembersListResponse {
    pub members: Vec<MemberResponse>,
    pub total: u64,
    pub page: u32,
    pub limit: u32,
}

/// Query params for cleanup count
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupCountQuery {
    pub user_id: String,
}

/// Cleanup count response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupCountResponse {
    pub count: usize,
}

/// Remove member result response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveMemberApiResponse {
    pub member_id: String,
    pub team_id: String,
    pub user_id: String,
    pub cleaned_count: usize,
    pub failures: usize,
}

impl From<TeamMember> for MemberResponse {
    fn from(member: TeamMember) -> Self {
        Self {
            id: member.id,
            team_id: member.team_id,
            user_id: member.user_id,
            display_name: member.display_name,
            endpoint_url: member.endpoint_url,
            role: member.role.to_string(),
            status: member.status.to_string(),
            joined_at: member.joined_at.to_rfc3339(),
        }
    }
}

/// Configure member routes
pub fn routes(state: TeamState) -> Router {
    Router::new()
        .route("/teams/{team_id}/members", post(add_member).get(list_members))
        .route("/teams/{team_id}/members/cleanup-count", get(get_cleanup_count))
        .route("/members/{member_id}", put(update_member).delete(remove_member))
        .route("/teams/{team_id}/leave", post(leave_team))
        .with_state(state)
}

/// Add a member to a team
async fn add_member(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(team_id): Path<String>,
    Json(req): Json<AddMemberApiRequest>,
) -> Result<(StatusCode, Json<MemberResponse>), TeamError> {
    let service = MemberService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    // Verify caller has permission to add members (must be Owner or Admin)
    let caller = service.get_member_by_user(&state.pool, &team_id, &user_id).await?;
    if !matches!(caller.role, MemberRole::Owner | MemberRole::Admin) {
        return Err(TeamError::PermissionDenied {
            action: "add member".to_string(),
        });
    }

    let role = req.role
        .as_ref()
        .and_then(|r| r.parse().ok())
        .unwrap_or(MemberRole::Member);

    // Cannot add Owner role through API
    if role == MemberRole::Owner {
        return Err(TeamError::PermissionDenied {
            action: "add owner role".to_string(),
        });
    }

    // Admin cannot add another Admin
    if caller.role == MemberRole::Admin && role == MemberRole::Admin {
        return Err(TeamError::PermissionDenied {
            action: "add admin (only owner can)".to_string(),
        });
    }

    let request = AddMemberRequest {
        user_id: req.user_id,
        display_name: req.display_name,
        role: Some(role),
        endpoint_url: req.endpoint_url,
    };

    let member = service.add_member(&state.pool, &team_id, request).await?;

    Ok((StatusCode::CREATED, Json(MemberResponse::from(member))))
}

/// List members of a team
async fn list_members(
    State(state): State<TeamState>,
    Path(team_id): Path<String>,
    Query(params): Query<ListMembersParams>,
) -> Result<Json<MembersListResponse>, TeamError> {
    let service = MemberService::new();

    let query = ListMembersQuery {
        page: params.page.unwrap_or(1),
        limit: params.limit.unwrap_or(50).min(100),
        status: None,
        role: None,
    };

    let result = service.list_members(&state.pool, &team_id, query).await?;

    let response = MembersListResponse {
        members: result.items.into_iter().map(MemberResponse::from).collect(),
        total: result.total,
        page: result.page,
        limit: result.limit,
    };

    Ok(Json(response))
}

/// Update a member
async fn update_member(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(member_id): Path<String>,
    Json(req): Json<UpdateMemberApiRequest>,
) -> Result<Json<MemberResponse>, TeamError> {
    let service = MemberService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    // Get the member to update
    let member = service.get_member(&state.pool, &member_id).await?;

    // Cannot update owner's role
    if member.role == MemberRole::Owner && req.role.is_some() {
        return Err(TeamError::PermissionDenied {
            action: "change owner role".to_string(),
        });
    }

    // Verify caller has permission (must be Owner or Admin, or updating own display_name)
    let caller = service.get_member_by_user(&state.pool, &member.team_id, &user_id).await?;
    let is_self_update = caller.id == member.id;

    // For role changes, require Owner or Admin
    if req.role.is_some() {
        if !matches!(caller.role, MemberRole::Owner | MemberRole::Admin) {
            return Err(TeamError::PermissionDenied {
                action: "update member role".to_string(),
            });
        }

        // Parse the new role
        let new_role: MemberRole = req.role.as_ref()
            .and_then(|r| r.parse().ok())
            .unwrap_or(MemberRole::Member);

        // Admin cannot promote to Admin or Owner
        if caller.role == MemberRole::Admin && matches!(new_role, MemberRole::Admin | MemberRole::Owner) {
            return Err(TeamError::PermissionDenied {
                action: "promote to admin/owner (only owner can)".to_string(),
            });
        }
    }

    // For display_name changes, allow self-update or require Owner/Admin
    if req.display_name.is_some() && !is_self_update {
        if !matches!(caller.role, MemberRole::Owner | MemberRole::Admin) {
            return Err(TeamError::PermissionDenied {
                action: "update other member's display name".to_string(),
            });
        }
    }

    // Parse role if provided
    let role = req.role.as_ref().and_then(|r| r.parse().ok());

    // Perform the update
    let updated_member = service.update_member(
        &state.pool,
        &member_id,
        role,
        req.display_name,
    ).await?;

    Ok(Json(MemberResponse::from(updated_member)))
}

/// Remove a member from a team
async fn remove_member(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(member_id): Path<String>,
) -> Result<Json<RemoveMemberApiResponse>, TeamError> {
    let service = MemberService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    // Get member to check permissions
    let member = service.get_member(&state.pool, &member_id).await?;

    // Cannot remove owner
    if member.role == MemberRole::Owner {
        return Err(TeamError::CannotRemoveOwner);
    }

    // Verify caller has permission to remove members (must be Owner or Admin)
    let caller = service.get_member_by_user(&state.pool, &member.team_id, &user_id).await?;
    if !matches!(caller.role, MemberRole::Owner | MemberRole::Admin) {
        return Err(TeamError::PermissionDenied {
            action: "remove member".to_string(),
        });
    }

    // Admin cannot remove another Admin
    if caller.role == MemberRole::Admin && member.role == MemberRole::Admin {
        return Err(TeamError::PermissionDenied {
            action: "remove admin (only owner can)".to_string(),
        });
    }

    // Use the cleanup removal method
    let result = service.remove_member_with_cleanup(
        &state.pool,
        &member_id,
        &state.base_path,
        &user_id,
    ).await?;

    Ok(Json(RemoveMemberApiResponse {
        member_id: result.member_id,
        team_id: result.team_id,
        user_id: result.user_id,
        cleaned_count: result.cleanup_result.cleaned_count,
        failures: result.cleanup_result.failures.len(),
    }))
}

/// Leave a team
async fn leave_team(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(team_id): Path<String>,
) -> Result<Json<RemoveMemberApiResponse>, TeamError> {
    let service = MemberService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    // Get member record for current user
    let member = service.get_member_by_user(&state.pool, &team_id, &user_id).await?;

    // Owner cannot leave
    if member.role == MemberRole::Owner {
        return Err(TeamError::OwnerCannotLeave);
    }

    // Use the cleanup removal method
    let result = service.remove_member_with_cleanup(
        &state.pool,
        &member.id,
        &state.base_path,
        &user_id,
    ).await?;

    Ok(Json(RemoveMemberApiResponse {
        member_id: result.member_id,
        team_id: result.team_id,
        user_id: result.user_id,
        cleaned_count: result.cleanup_result.cleaned_count,
        failures: result.cleanup_result.failures.len(),
    }))
}

/// Get cleanup count for a user in a team
/// Returns the number of resources that would be cleaned up if the user is removed
async fn get_cleanup_count(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(team_id): Path<String>,
    Query(query): Query<CleanupCountQuery>,
) -> Result<Json<CleanupCountResponse>, TeamError> {
    let member_service = MemberService::new();
    let cleanup_service = CleanupService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    // SEC-2 FIX: Verify caller has permission to view cleanup counts
    // Must be Owner or Admin to view cleanup info for other users
    let caller = member_service.get_member_by_user(&state.pool, &team_id, &user_id).await?;

    // Allow viewing own cleanup count, or require Owner/Admin for others
    if query.user_id != user_id {
        if !matches!(caller.role, MemberRole::Owner | MemberRole::Admin) {
            return Err(TeamError::PermissionDenied {
                action: "view cleanup count for other users".to_string(),
            });
        }
    }

    let count = cleanup_service
        .get_cleanup_count(&state.pool, &team_id, &query.user_id)
        .await?;

    Ok(Json(CleanupCountResponse { count }))
}
