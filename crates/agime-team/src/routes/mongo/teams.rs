//! MongoDB routes - Team API

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::db::MongoDb;
use crate::models::mongo::{
    CreateTeamRequest, DocumentAnalysisTrigger, SmartLogTrigger, Team,
    TeamDetailResponse, TeamSummary,
};
use crate::services::mongo::{ExtensionService, RecipeService, SkillService, TeamService};
use crate::AuthenticatedUserId;

/// User role in a team
#[derive(Debug, Clone, PartialEq)]
pub enum TeamRole {
    Owner,
    Admin,
    Member,
}

impl TeamRole {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "owner" => TeamRole::Owner,
            "admin" => TeamRole::Admin,
            _ => TeamRole::Member,
        }
    }

    pub fn can_manage(&self) -> bool {
        matches!(self, TeamRole::Owner | TeamRole::Admin)
    }

    pub fn is_owner(&self) -> bool {
        matches!(self, TeamRole::Owner)
    }
}

/// Get user's role in a team
pub fn get_user_role(team: &Team, user_id: &str) -> Option<TeamRole> {
    team.members
        .iter()
        .find(|m| m.user_id == user_id)
        .map(|m| TeamRole::from_str(&m.role))
}

/// Check if user is a member of the team
pub fn is_team_member(team: &Team, user_id: &str) -> bool {
    team.members.iter().any(|m| m.user_id == user_id)
}

/// Check if user can manage the team (admin or owner)
pub fn can_manage_team(team: &Team, user_id: &str) -> bool {
    get_user_role(team, user_id)
        .map(|r| r.can_manage())
        .unwrap_or(false)
}

/// Check if user is the owner of the team
pub fn is_team_owner(team: &Team, user_id: &str) -> bool {
    get_user_role(team, user_id)
        .map(|r| r.is_owner())
        .unwrap_or(false)
}

/// App state for MongoDB routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<MongoDb>,
    pub smart_log_trigger: Option<Arc<dyn SmartLogTrigger>>,
    pub doc_analysis_trigger: Option<Arc<dyn DocumentAnalysisTrigger>>,
    /// Base URL for generating public portal URLs (e.g. "http://192.168.1.100:8080")
    pub portal_base_url: String,
    /// Whether portal_base_url comes from explicit BASE_URL configuration.
    pub portal_base_url_configured: bool,
    /// Optional testing URL base (typically IP:port) for published portals.
    pub portal_test_base_url: Option<String>,
    /// Workspace root for file-based portal projects
    pub workspace_root: String,
}

/// Pagination query parameters
#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub page: Option<usize>,
    pub limit: Option<usize>,
    pub search: Option<String>,
}

/// Teams list response matching frontend expectations
#[derive(Debug, Serialize)]
pub struct TeamsResponse {
    pub teams: Vec<TeamSummary>,
    pub total: usize,
    pub page: usize,
    pub limit: usize,
}

/// Team response wrapper
#[derive(Debug, Serialize)]
pub struct TeamResponse {
    pub team: TeamSummary,
}

/// Members response
#[derive(Debug, Serialize)]
pub struct MembersResponse {
    pub members: Vec<MemberInfo>,
}

/// Member info for API response
#[derive(Debug, Serialize)]
pub struct MemberInfo {
    pub id: String,
    #[serde(rename = "teamId")]
    pub team_id: String,
    #[serde(rename = "userId")]
    pub user_id: String,
    pub email: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "endpointUrl")]
    pub endpoint_url: Option<String>,
    pub role: String,
    pub status: String,
    pub permissions: MemberPermissionsInfo,
    #[serde(rename = "joinedAt")]
    pub joined_at: String,
}

/// Member permissions for API response
#[derive(Debug, Serialize)]
pub struct MemberPermissionsInfo {
    #[serde(rename = "canShare")]
    pub can_share: bool,
    #[serde(rename = "canInstall")]
    pub can_install: bool,
    #[serde(rename = "canDeleteOwn")]
    pub can_delete_own: bool,
}

/// Update team request
#[derive(Debug, Deserialize)]
pub struct UpdateTeamRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "repositoryUrl")]
    pub repository_url: Option<String>,
}

/// Add member request
#[derive(Debug, Deserialize)]
pub struct AddMemberRequest {
    pub email: String,
    pub role: String,
}

/// Update member request
#[derive(Debug, Deserialize)]
pub struct UpdateMemberRequest {
    pub role: String,
}

/// Create invite request
#[derive(Debug, Deserialize)]
pub struct CreateInviteRequest {
    pub role: String,
    pub expires_in_days: Option<i64>,
    pub max_uses: Option<i32>,
}

/// Invite info for API response
#[derive(Debug, Serialize)]
pub struct InviteInfo {
    pub id: String,
    #[serde(rename = "teamId")]
    pub team_id: String,
    pub role: String,
    #[serde(rename = "createdBy")]
    pub created_by: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: Option<String>,
    #[serde(rename = "maxUses")]
    pub max_uses: Option<i32>,
    #[serde(rename = "usedCount")]
    pub used_count: i32,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

/// Invites response
#[derive(Debug, Serialize)]
pub struct InvitesResponse {
    pub invites: Vec<InviteInfo>,
}

/// Validate invite response
#[derive(Debug, Serialize)]
pub struct ValidateInviteResponse {
    pub valid: bool,
    #[serde(rename = "teamId")]
    pub team_id: Option<String>,
    #[serde(rename = "teamName")]
    pub team_name: Option<String>,
    pub role: Option<String>,
    #[serde(rename = "expiresAt")]
    pub expires_at: Option<String>,
    pub error: Option<String>,
}

/// Accept invite request
#[derive(Debug, Deserialize)]
pub struct AcceptInviteRequest {
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
}

/// Accept invite response
#[derive(Debug, Serialize)]
pub struct AcceptInviteResponse {
    pub success: bool,
    #[serde(rename = "teamId")]
    pub team_id: Option<String>,
    #[serde(rename = "teamName")]
    pub team_name: Option<String>,
    pub error: Option<String>,
}

pub fn team_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/teams", get(list_teams).post(create_team))
        .route(
            "/teams/{id}",
            get(get_team).put(update_team).delete(delete_team),
        )
        .route("/teams/{id}/members", get(get_members).post(add_member))
        .route("/members/{id}", put(update_member).delete(remove_member))
        .route("/teams/{id}/invites", get(get_invites).post(create_invite))
        .route("/teams/{team_id}/invites/{code}", delete(revoke_invite))
        .route(
            "/teams/{id}/settings",
            get(get_team_settings).put(update_team_settings),
        )
        // Public invite routes (no auth required for validation)
        .route("/invites/{code}", get(validate_invite))
        .route("/invites/{code}/accept", post(accept_invite))
}

async fn list_teams(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Query(query): Query<PaginationQuery>,
) -> Result<Json<TeamsResponse>, (StatusCode, String)> {
    let service = TeamService::new((*state.db).clone());
    let all_teams = service
        .list_for_user(&user.0)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Apply search filter
    let filtered_teams: Vec<TeamSummary> = if let Some(search) = &query.search {
        let search_lower = search.to_lowercase();
        all_teams
            .into_iter()
            .filter(|t| t.name.to_lowercase().contains(&search_lower))
            .collect()
    } else {
        all_teams
    };

    let total = filtered_teams.len();
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(100).min(1000);

    // Apply pagination
    let start = (page - 1) * limit;
    let teams: Vec<TeamSummary> = filtered_teams.into_iter().skip(start).take(limit).collect();

    Ok(Json(TeamsResponse {
        teams,
        total,
        page,
        limit,
    }))
}

async fn create_team(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Json(req): Json<CreateTeamRequest>,
) -> Result<Json<TeamResponse>, (StatusCode, String)> {
    let service = TeamService::new((*state.db).clone());
    service
        .create(&user.0, req)
        .await
        .map(|t| {
            Json(TeamResponse {
                team: TeamSummary::from(t),
            })
        })
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn get_team(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
) -> Result<Json<TeamDetailResponse>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let skill_service = SkillService::new((*state.db).clone());
    let recipe_service = RecipeService::new((*state.db).clone());
    let extension_service = ExtensionService::new((*state.db).clone());

    let team = team_service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Check if user is a team member
    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view team details".to_string(),
        ));
    }

    let members_count = team.members.len();

    // Get current user's role
    let current_user_role = team
        .members
        .iter()
        .find(|m| m.user_id == user.0)
        .map(|m| m.role.clone())
        .unwrap_or_else(|| "member".to_string());

    // Get resource counts (page=1, limit=1 just to get total count)
    let skills_count = skill_service
        .list(&id, Some(1), Some(1), None, None)
        .await
        .map(|r| r.total as usize)
        .unwrap_or(0);
    let recipes_count = recipe_service
        .list(&id, Some(1), Some(1), None, None)
        .await
        .map(|r| r.total as usize)
        .unwrap_or(0);
    let extensions_count = extension_service
        .list(&id, Some(1), Some(1), None, None)
        .await
        .map(|r| r.total as usize)
        .unwrap_or(0);

    Ok(Json(TeamDetailResponse {
        team: TeamSummary::from(team),
        members_count,
        skills_count,
        recipes_count,
        extensions_count,
        current_user_id: user.0,
        current_user_role,
    }))
}

async fn delete_team(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let service = TeamService::new((*state.db).clone());

    // Get team and check ownership
    let team = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Only owner can delete team
    if !is_team_owner(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team owner can delete the team".to_string(),
        ));
    }

    service
        .delete(&id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn update_team(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTeamRequest>,
) -> Result<Json<TeamResponse>, (StatusCode, String)> {
    let service = TeamService::new((*state.db).clone());

    // Get team and check permission
    let team = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Only admin/owner can update team
    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team admin or owner can update the team".to_string(),
        ));
    }

    service
        .update(&id, req.name, req.description, req.repository_url)
        .await
        .map(|t| {
            Json(TeamResponse {
                team: TeamSummary::from(t),
            })
        })
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn get_members(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
) -> Result<Json<MembersResponse>, (StatusCode, String)> {
    let service = TeamService::new((*state.db).clone());
    let team = service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Check if user is a team member
    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view member list".to_string(),
        ));
    }

    let members: Vec<MemberInfo> = team
        .members
        .into_iter()
        .map(|m| MemberInfo {
            id: format!("{}_{}", team_id, m.user_id),
            team_id: team_id.clone(),
            user_id: m.user_id.clone(),
            email: m.email,
            display_name: m.display_name,
            endpoint_url: None,
            role: m.role,
            status: m.status,
            permissions: MemberPermissionsInfo {
                can_share: m.permissions.can_share,
                can_install: m.permissions.can_install,
                can_delete_own: m.permissions.can_delete_own,
            },
            joined_at: m.joined_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(MembersResponse { members }))
}

async fn add_member(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Json(req): Json<AddMemberRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let service = TeamService::new((*state.db).clone());

    // Get team and check permission
    let team = service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Only admin/owner can add members
    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team admin or owner can add members".to_string(),
        ));
    }

    service
        .add_member(&team_id, &req.email, &req.email, &req.role)
        .await
        .map(|_| Json(serde_json::json!({ "success": true })))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn update_member(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(member_id): Path<String>,
    Json(req): Json<UpdateMemberRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // member_id format: {team_id}_{user_id}
    let parts: Vec<&str> = member_id.splitn(2, '_').collect();
    if parts.len() != 2 {
        return Err((StatusCode::BAD_REQUEST, "Invalid member ID".to_string()));
    }
    let team_id = parts[0];
    let target_user_id = parts[1];

    let service = TeamService::new((*state.db).clone());

    // Get team and check permission
    let team = service
        .get(team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Only admin/owner can update member roles
    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team admin or owner can update member roles".to_string(),
        ));
    }

    // Cannot change owner's role
    if is_team_owner(&team, target_user_id) && req.role.to_lowercase() != "owner" {
        return Err((
            StatusCode::FORBIDDEN,
            "Cannot change owner's role".to_string(),
        ));
    }

    service
        .update_member_role(team_id, target_user_id, &req.role)
        .await
        .map(|_| Json(serde_json::json!({ "success": true })))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn remove_member(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(member_id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    // member_id format: {team_id}_{user_id}
    let parts: Vec<&str> = member_id.splitn(2, '_').collect();
    if parts.len() != 2 {
        return Err((StatusCode::BAD_REQUEST, "Invalid member ID".to_string()));
    }
    let team_id = parts[0];
    let target_user_id = parts[1];

    let service = TeamService::new((*state.db).clone());

    // Get team and check permission
    let team = service
        .get(team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Only admin/owner can remove members (or user can remove themselves)
    let is_self_removal = user.0 == target_user_id;
    if !is_self_removal && !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team admin or owner can remove members".to_string(),
        ));
    }

    // Cannot remove the owner
    if is_team_owner(&team, target_user_id) {
        return Err((
            StatusCode::FORBIDDEN,
            "Cannot remove team owner".to_string(),
        ));
    }

    service
        .remove_member(team_id, target_user_id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn get_invites(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
) -> Result<Json<InvitesResponse>, (StatusCode, String)> {
    let service = TeamService::new((*state.db).clone());

    // Get team and check membership
    let team = service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Only team members can view invites
    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view invites".to_string(),
        ));
    }

    let invites = service
        .list_invites(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let invites: Vec<InviteInfo> = invites
        .into_iter()
        .map(|i| InviteInfo {
            id: i.code.clone(),
            team_id: team_id.clone(),
            role: i.role,
            created_by: i.created_by,
            expires_at: i.expires_at.map(|dt| dt.to_rfc3339()),
            max_uses: i.max_uses,
            used_count: i.used_count,
            created_at: i.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(InvitesResponse { invites }))
}

async fn create_invite(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Json(req): Json<CreateInviteRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let service = TeamService::new((*state.db).clone());

    // Get team and check permission
    let team = service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Only admin/owner can create invites
    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team admin or owner can create invites".to_string(),
        ));
    }

    let invite = service
        .create_invite(
            &team_id,
            &user.0,
            &req.role,
            req.expires_in_days,
            req.max_uses,
        )
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "code": invite.code,
        "url": format!("/join/{}", invite.code),
        "expiresAt": invite.expires_at.map(|dt| dt.to_rfc3339()),
        "maxUses": invite.max_uses,
        "usedCount": invite.used_count
    })))
}

async fn revoke_invite(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, code)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let service = TeamService::new((*state.db).clone());

    // Get team and check permission
    let team = service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Only admin/owner can revoke invites
    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team admin or owner can revoke invites".to_string(),
        ));
    }

    service
        .revoke_invite(&team_id, &code)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

/// Validate an invite code (public - no auth required)
async fn validate_invite(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> Result<Json<ValidateInviteResponse>, (StatusCode, String)> {
    let service = TeamService::new((*state.db).clone());

    match service.get_invite_by_code(&code).await {
        Ok(Some(invite)) => {
            // Check if expired
            if let Some(expires_at) = invite.expires_at {
                if expires_at < chrono::Utc::now() {
                    return Ok(Json(ValidateInviteResponse {
                        valid: false,
                        team_id: None,
                        team_name: None,
                        role: None,
                        expires_at: None,
                        error: Some("Invite has expired".to_string()),
                    }));
                }
            }

            // Check max uses
            if let Some(max) = invite.max_uses {
                if invite.used_count >= max {
                    return Ok(Json(ValidateInviteResponse {
                        valid: false,
                        team_id: None,
                        team_name: None,
                        role: None,
                        expires_at: None,
                        error: Some("Invite has reached maximum uses".to_string()),
                    }));
                }
            }

            // Get team name
            let team_name = service
                .get(&invite.team_id.to_hex())
                .await
                .ok()
                .flatten()
                .map(|t| t.name);

            Ok(Json(ValidateInviteResponse {
                valid: true,
                team_id: Some(invite.team_id.to_hex()),
                team_name,
                role: Some(invite.role),
                expires_at: invite.expires_at.map(|dt| dt.to_rfc3339()),
                error: None,
            }))
        }
        Ok(None) => Ok(Json(ValidateInviteResponse {
            valid: false,
            team_id: None,
            team_name: None,
            role: None,
            expires_at: None,
            error: Some("Invalid invite code".to_string()),
        })),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// Accept an invite and join the team
async fn accept_invite(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(code): Path<String>,
    Json(req): Json<AcceptInviteRequest>,
) -> Result<Json<AcceptInviteResponse>, (StatusCode, String)> {
    let service = TeamService::new((*state.db).clone());
    let display_name = req.display_name.unwrap_or_else(|| "New Member".to_string());

    match service.accept_invite(&code, &user.0, &display_name).await {
        Ok(result) => Ok(Json(result)),
        Err(e) => Ok(Json(AcceptInviteResponse {
            success: false,
            team_id: None,
            team_name: None,
            error: Some(e.to_string()),
        })),
    }
}

// ── Team Settings ──

/// Convert an empty string to None, non-empty to Some.
fn non_empty(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

#[derive(Debug, Serialize)]
pub struct TeamSettingsResponse {
    #[serde(rename = "requireExtensionReview")]
    pub require_extension_review: bool,
    #[serde(rename = "membersCanInvite")]
    pub members_can_invite: bool,
    #[serde(rename = "defaultVisibility")]
    pub default_visibility: String,
    #[serde(rename = "documentAnalysis")]
    pub document_analysis: DocumentAnalysisSettingsResponse,
}

#[derive(Debug, Serialize)]
pub struct DocumentAnalysisSettingsResponse {
    pub enabled: bool,
    #[serde(rename = "apiUrl", skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(rename = "apiKeySet")]
    pub api_key_set: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(rename = "apiFormat", skip_serializing_if = "Option::is_none")]
    pub api_format: Option<String>,
    #[serde(rename = "agentId", skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(rename = "minFileSize")]
    pub min_file_size: i64,
    #[serde(rename = "maxFileSize", skip_serializing_if = "Option::is_none")]
    pub max_file_size: Option<i64>,
    #[serde(rename = "skipMimePrefixes")]
    pub skip_mime_prefixes: Vec<String>,
}

impl From<crate::models::mongo::TeamSettings> for TeamSettingsResponse {
    fn from(s: crate::models::mongo::TeamSettings) -> Self {
        Self {
            require_extension_review: s.require_extension_review,
            members_can_invite: s.members_can_invite,
            default_visibility: s.default_visibility,
            document_analysis: DocumentAnalysisSettingsResponse {
                enabled: s.document_analysis.enabled,
                api_url: s.document_analysis.api_url,
                api_key_set: s.document_analysis.api_key.is_some(),
                model: s.document_analysis.model,
                api_format: s.document_analysis.api_format,
                agent_id: s.document_analysis.agent_id,
                min_file_size: s.document_analysis.min_file_size,
                max_file_size: s.document_analysis.max_file_size,
                skip_mime_prefixes: s.document_analysis.skip_mime_prefixes,
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct UpdateTeamSettingsRequest {
    #[serde(rename = "documentAnalysis")]
    document_analysis: Option<UpdateDocAnalysisRequest>,
}

#[derive(Debug, Deserialize)]
struct UpdateDocAnalysisRequest {
    enabled: Option<bool>,
    #[serde(rename = "apiUrl")]
    api_url: Option<String>,
    #[serde(rename = "apiKey")]
    api_key: Option<String>,
    model: Option<String>,
    #[serde(rename = "apiFormat")]
    api_format: Option<String>,
    #[serde(rename = "agentId")]
    agent_id: Option<String>,
    #[serde(rename = "minFileSize")]
    min_file_size: Option<i64>,
    #[serde(rename = "maxFileSize")]
    max_file_size: Option<Option<i64>>,
    #[serde(rename = "skipMimePrefixes")]
    skip_mime_prefixes: Option<Vec<String>>,
}

async fn get_team_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
) -> Result<Json<TeamSettingsResponse>, (StatusCode, String)> {
    let service = TeamService::new((*state.db).clone());
    let team = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Forbidden".to_string()));
    }

    Ok(Json(TeamSettingsResponse::from(team.settings)))
}

async fn update_team_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTeamSettingsRequest>,
) -> Result<Json<TeamSettingsResponse>, (StatusCode, String)> {
    let service = TeamService::new((*state.db).clone());
    let team = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !can_manage_team(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Only admin/owner can update settings".to_string()));
    }

    let mut settings = team.settings;

    // Merge document_analysis fields
    if let Some(da) = req.document_analysis {
        let d = &mut settings.document_analysis;
        if let Some(v) = da.enabled { d.enabled = v; }
        if let Some(v) = da.api_url { d.api_url = non_empty(v); }
        if let Some(v) = da.api_key { d.api_key = non_empty(v); }
        if let Some(v) = da.model { d.model = non_empty(v); }
        if let Some(v) = da.api_format { d.api_format = non_empty(v); }
        if let Some(v) = da.agent_id { d.agent_id = non_empty(v); }
        if let Some(v) = da.min_file_size { d.min_file_size = v; }
        if let Some(v) = da.max_file_size { d.max_file_size = v; }
        if let Some(v) = da.skip_mime_prefixes { d.skip_mime_prefixes = v; }
    }

    let updated = service
        .update_settings(&id, settings)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(TeamSettingsResponse::from(updated.settings)))
}
