//! Teams HTTP routes

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::TeamError;
use crate::models::{
    Team, CreateTeamRequest, ListTeamsQuery,
};
use crate::services::TeamService;
use crate::services::MemberService;
use crate::AuthenticatedUserId;
use super::get_user_id;

/// Team state for routes
#[derive(Clone)]
pub struct TeamState {
    pub pool: Arc<SqlitePool>,
    pub user_id: String, // TODO: Get from auth middleware
    /// Base path for installing resources (skills, recipes, extensions)
    pub base_path: PathBuf,
}

/// Query params for listing teams
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTeamsParams {
    pub page: Option<u32>,
    pub limit: Option<u32>,
}

/// Create team request (API)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTeamApiRequest {
    pub name: String,
    pub description: Option<String>,
    pub repository_url: Option<String>,
}

/// Update team request (API)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTeamApiRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub repository_url: Option<String>,
}

/// Team response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub repository_url: Option<String>,
    pub owner_id: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Team summary response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamSummaryResponse {
    pub team: TeamResponse,
    pub members_count: u32,
    pub skills_count: u32,
    pub recipes_count: u32,
    pub extensions_count: u32,
    /// The current user's ID making the request
    pub current_user_id: String,
}

/// Paginated teams response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamsListResponse {
    pub teams: Vec<TeamResponse>,
    pub total: u64,
    pub page: u32,
    pub limit: u32,
}

impl From<Team> for TeamResponse {
    fn from(team: Team) -> Self {
        Self {
            id: team.id,
            name: team.name,
            description: team.description,
            repository_url: team.repository_url,
            owner_id: team.owner_id,
            created_at: team.created_at.to_rfc3339(),
            updated_at: team.updated_at.to_rfc3339(),
        }
    }
}

// Note: We don't implement From<TeamSummary> for TeamSummaryResponse because
// TeamSummaryResponse requires current_user_id which is only available from
// the request context (state.user_id), not from the TeamSummary model.

/// Configure team routes
pub fn routes(state: TeamState) -> Router {
    Router::new()
        .route("/teams", post(create_team).get(list_teams))
        .route("/teams/{id}", get(get_team).put(update_team).delete(delete_team))
        .with_state(state)
}

/// Create a new team
async fn create_team(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Json(req): Json<CreateTeamApiRequest>,
) -> Result<(StatusCode, Json<TeamResponse>), TeamError> {
    let service = TeamService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    let request = CreateTeamRequest {
        name: req.name,
        description: req.description,
        repository_url: req.repository_url,
        settings: None,
    };

    let team = service.create_team(&state.pool, request, &user_id).await?;

    Ok((StatusCode::CREATED, Json(TeamResponse::from(team))))
}

/// List teams for current user
async fn list_teams(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Query(params): Query<ListTeamsParams>,
) -> Result<Json<TeamsListResponse>, TeamError> {
    let service = TeamService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    let query = ListTeamsQuery {
        page: params.page.unwrap_or(1),
        limit: params.limit.unwrap_or(20).min(100),
        search: None,
        owner_id: None,
    };

    let result = service.list_teams(&state.pool, query, &user_id).await?;

    let response = TeamsListResponse {
        teams: result.items.into_iter().map(TeamResponse::from).collect(),
        total: result.total,
        page: result.page,
        limit: result.limit,
    };

    Ok(Json(response))
}

/// Get team by ID with summary
async fn get_team(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(team_id): Path<String>,
) -> Result<Json<TeamSummaryResponse>, TeamError> {
    let service = TeamService::new();
    let member_service = MemberService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    // Verify caller is a member of this team
    member_service.get_member_by_user(&state.pool, &team_id, &user_id).await?;

    let summary = service.get_team_summary(&state.pool, &team_id).await?;

    Ok(Json(TeamSummaryResponse {
        team: TeamResponse::from(summary.team),
        members_count: summary.members_count,
        skills_count: summary.skills_count,
        recipes_count: summary.recipes_count,
        extensions_count: summary.extensions_count,
        current_user_id: user_id,
    }))
}

/// Update a team
async fn update_team(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(team_id): Path<String>,
    Json(req): Json<UpdateTeamApiRequest>,
) -> Result<Json<TeamResponse>, TeamError> {
    let service = TeamService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    // First verify the team exists and user has permission
    let team = service.get_team(&state.pool, &team_id).await?;

    // Check ownership (only owner can update)
    if team.owner_id != user_id {
        return Err(TeamError::PermissionDenied {
            action: "update team".to_string(),
        });
    }

    // Build update request
    let update_request = crate::models::UpdateTeamRequest {
        name: req.name,
        description: req.description,
        repository_url: req.repository_url,
        settings: None,
    };

    // Perform update
    let updated_team = service.update_team(&state.pool, &team_id, update_request).await?;

    Ok(Json(TeamResponse::from(updated_team)))
}

/// Delete a team (soft delete)
async fn delete_team(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(team_id): Path<String>,
) -> Result<StatusCode, TeamError> {
    let service = TeamService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    // First verify the team exists and user has permission
    let team = service.get_team(&state.pool, &team_id).await?;

    // Check ownership (only owner can delete)
    if team.owner_id != user_id {
        return Err(TeamError::PermissionDenied {
            action: "delete team".to_string(),
        });
    }

    service.delete_team(&state.pool, &team_id).await?;

    Ok(StatusCode::NO_CONTENT)
}
