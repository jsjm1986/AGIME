//! MongoDB routes - Team API

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId},
    options::FindOptions,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc};

use crate::db::{collections, MongoDb};
use crate::models::mongo::{
    CreateTeamRequest, DocumentAnalysisTrigger, DocumentOrigin, DocumentStatus, SmartLogTrigger,
    Team, TeamDetailResponse, TeamSettings, TeamSummary,
};
use crate::models::BuiltinExtension;
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
    pub fn parse(s: &str) -> Self {
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
        .map(|m| TeamRole::parse(&m.role))
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticIndexResponse {
    pub team_id: String,
    pub version: String,
    pub generated_at: String,
    pub entities: Vec<SemanticIndexEntity>,
    pub builtin_catalog: Vec<SemanticBuiltinCatalogItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticIndexEntity {
    pub id: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub scope: String,
    pub team_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portal_id: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticBuiltinCatalogItem {
    pub id: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub name: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub scope: String,
    pub is_platform: bool,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct SemanticPortalRow {
    #[serde(rename = "_id")]
    id: ObjectId,
    slug: String,
    name: String,
    status: crate::models::mongo::PortalStatus,
    #[serde(default)]
    domain: Option<crate::models::mongo::PortalDomain>,
    #[serde(default)]
    coding_agent_id: Option<String>,
    #[serde(default)]
    service_agent_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct SemanticAgentRow {
    agent_id: String,
    team_id: String,
    name: String,
    #[serde(default)]
    agent_domain: Option<String>,
    #[serde(default)]
    agent_role: Option<String>,
    #[serde(default)]
    owner_manager_agent_id: Option<String>,
    #[serde(default)]
    enabled_extensions: Vec<SemanticAgentBuiltinExtensionRow>,
    #[serde(default)]
    custom_extensions: Vec<SemanticAgentCustomExtensionRow>,
    status: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    updated_at: DateTime<Utc>,
}

fn semantic_extension_enabled_default() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct SemanticAgentBuiltinExtensionRow {
    extension: BuiltinExtension,
    #[serde(default = "semantic_extension_enabled_default")]
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct SemanticAgentCustomExtensionRow {
    name: String,
    #[serde(default = "semantic_extension_enabled_default")]
    enabled: bool,
    #[serde(default, rename = "type")]
    ext_type: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    source_extension_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SemanticDocumentRow {
    #[serde(rename = "_id")]
    id: ObjectId,
    name: String,
    #[serde(default)]
    display_name: Option<String>,
    status: DocumentStatus,
    folder_path: String,
    mime_type: String,
    file_size: i64,
    origin: DocumentOrigin,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct SemanticFolderRow {
    #[serde(rename = "_id")]
    id: ObjectId,
    name: String,
    full_path: String,
    #[serde(default)]
    is_system: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct SemanticSkillRow {
    #[serde(rename = "_id")]
    id: ObjectId,
    name: String,
    #[serde(default)]
    description: Option<String>,
    visibility: String,
    protection_level: String,
    version: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct SemanticExtensionRow {
    #[serde(rename = "_id")]
    id: ObjectId,
    name: String,
    #[serde(default)]
    description: Option<String>,
    extension_type: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct SemanticGovernanceStateRow {
    portal_id: String,
    state: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    updated_at: DateTime<Utc>,
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
        .route("/teams/{id}/semantic-index", get(get_semantic_index))
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

fn portal_projection() -> FindOptions {
    FindOptions::builder()
        .projection(doc! {
            "_id": 1,
            "slug": 1,
            "name": 1,
            "status": 1,
            "domain": 1,
            "coding_agent_id": 1,
            "service_agent_id": 1,
            "updated_at": 1
        })
        .build()
}

fn document_projection() -> FindOptions {
    FindOptions::builder()
        .projection(doc! {
            "_id": 1,
            "name": 1,
            "display_name": 1,
            "status": 1,
            "folder_path": 1,
            "mime_type": 1,
            "file_size": 1,
            "origin": 1,
            "updated_at": 1
        })
        .build()
}

fn folder_projection() -> FindOptions {
    FindOptions::builder()
        .projection(doc! {
            "_id": 1,
            "name": 1,
            "full_path": 1,
            "is_system": 1,
            "updated_at": 1
        })
        .build()
}

fn skill_projection() -> FindOptions {
    FindOptions::builder()
        .projection(doc! {
            "_id": 1,
            "name": 1,
            "description": 1,
            "visibility": 1,
            "protection_level": 1,
            "version": 1,
            "updated_at": 1
        })
        .build()
}

fn extension_projection() -> FindOptions {
    FindOptions::builder()
        .projection(doc! {
            "_id": 1,
            "name": 1,
            "description": 1,
            "extension_type": 1,
            "source": 1,
            "updated_at": 1
        })
        .build()
}

fn governance_projection() -> FindOptions {
    FindOptions::builder()
        .projection(doc! {
            "portal_id": 1,
            "state": 1,
            "updated_at": 1
        })
        .build()
}

fn make_semantic_entity(
    id: String,
    entity_type: &str,
    name: String,
    display_name: Option<String>,
    aliases: Vec<String>,
    status: Option<String>,
    scope: &str,
    team_id: &str,
    portal_id: Option<String>,
    metadata: serde_json::Value,
) -> SemanticIndexEntity {
    SemanticIndexEntity {
        id,
        entity_type: entity_type.to_string(),
        name,
        display_name,
        aliases,
        status,
        scope: scope.to_string(),
        team_id: team_id.to_string(),
        portal_id,
        metadata,
    }
}

fn parse_governance_request_entities(
    team_id: &str,
    portal_lookup: &std::collections::HashMap<String, (&str, &str)>,
    row: &SemanticGovernanceStateRow,
) -> Vec<SemanticIndexEntity> {
    let mut entities = Vec::new();

    let groups = [
        ("capability_requests", "capabilityRequests", "capability"),
        ("gap_proposals", "gapProposals", "proposal"),
        ("optimization_tickets", "optimizationTickets", "ticket"),
    ];

    for (snake_key, camel_key, kind) in groups {
        let items = row
            .state
            .get(snake_key)
            .or_else(|| row.state.get(camel_key))
            .and_then(serde_json::Value::as_array);
        let Some(items) = items else {
            continue;
        };

        for item in items {
            let Some(id) = item.get("id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let title = item
                .get("title")
                .and_then(serde_json::Value::as_str)
                .or_else(|| item.get("name").and_then(serde_json::Value::as_str))
                .unwrap_or(id);
            let status = item
                .get("status")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let risk_level = item
                .get("risk")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let (portal_name, portal_slug) = portal_lookup
                .get(&row.portal_id)
                .map(|(name, slug)| (Some((*name).to_string()), Some((*slug).to_string())))
                .unwrap_or((None, None));

            entities.push(make_semantic_entity(
                id.to_string(),
                "governance_request",
                title.to_string(),
                None,
                Vec::new(),
                status,
                "portal",
                team_id,
                Some(row.portal_id.clone()),
                serde_json::json!({
                    "kind": kind,
                    "riskLevel": risk_level,
                    "portalName": portal_name,
                    "portalSlug": portal_slug,
                }),
            ));
        }
    }

    entities
}

fn builtin_display_name(builtin: BuiltinExtension) -> &'static str {
    match builtin {
        BuiltinExtension::Skills => "Skills",
        BuiltinExtension::SkillRegistry => "Skill Registry",
        BuiltinExtension::Todo => "Todo",
        BuiltinExtension::ExtensionManager => "Extension Manager",
        BuiltinExtension::Team => "Team",
        BuiltinExtension::ChatRecall => "Chat Recall",
        BuiltinExtension::DocumentTools => "Document Tools",
        BuiltinExtension::Developer => "Developer",
        BuiltinExtension::Memory => "Memory",
        BuiltinExtension::ComputerController => "Computer Controller",
        BuiltinExtension::AutoVisualiser => "Auto Visualiser",
        BuiltinExtension::Tutorial => "Tutorial",
    }
}

fn push_alias_variants(aliases: &mut Vec<String>, raw: &str) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }

    aliases.push(trimmed.to_string());

    let normalized_input = trimmed.replace(['_', '-'], " ");
    let normalized = normalized_input.split_whitespace().collect::<Vec<_>>();
    if normalized.is_empty() {
        return;
    }

    let words = normalized;
    let spaced = words.join(" ");
    let lower_spaced = spaced.to_ascii_lowercase();
    let upper_spaced = spaced.to_ascii_uppercase();
    let collapsed = words.join("");
    let lower_collapsed = collapsed.to_ascii_lowercase();
    let upper_collapsed = collapsed.to_ascii_uppercase();
    let snake = words
        .iter()
        .map(|word| word.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("_");
    let upper_snake = snake.to_ascii_uppercase();
    let kebab = words
        .iter()
        .map(|word| word.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("-");
    let upper_kebab = kebab.to_ascii_uppercase();

    aliases.push(spaced);
    aliases.push(lower_spaced);
    aliases.push(upper_spaced);
    aliases.push(collapsed.clone());
    aliases.push(lower_collapsed);
    aliases.push(upper_collapsed);
    aliases.push(snake);
    aliases.push(upper_snake);
    aliases.push(kebab);
    aliases.push(upper_kebab);
}

fn builtin_special_aliases(builtin: BuiltinExtension) -> &'static [&'static str] {
    match builtin {
        BuiltinExtension::Skills => &["Team Skills", "TEAM SKILLS", "Team Skill Tools"],
        BuiltinExtension::SkillRegistry => &["Skills Registry", "SKILLS REGISTRY"],
        BuiltinExtension::ExtensionManager => &[
            "MCP Manager",
            "MCP Extension Manager",
            "Extension Manager MCP",
        ],
        BuiltinExtension::DocumentTools => &["Doc Tools", "DOC TOOLS", "DocumentTools"],
        BuiltinExtension::ChatRecall => &["Chat Recall Memory"],
        BuiltinExtension::ComputerController => {
            &["ComputerControl", "COMPUTERCONTROL", "Computer Control"]
        }
        BuiltinExtension::AutoVisualiser => {
            &["AutoVisualizer", "AUTOVISUALIZER", "Auto Visualizer"]
        }
        _ => &[],
    }
}

fn builtin_aliases(builtin: BuiltinExtension) -> Vec<String> {
    let mut aliases = Vec::new();
    push_alias_variants(&mut aliases, builtin_display_name(builtin));
    push_alias_variants(&mut aliases, builtin.name());
    if let Some(mcp_name) = builtin.mcp_name() {
        push_alias_variants(&mut aliases, mcp_name);
    }
    for alias in builtin_special_aliases(builtin) {
        push_alias_variants(&mut aliases, alias);
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

fn builtin_catalog() -> Vec<SemanticBuiltinCatalogItem> {
    BuiltinExtension::all()
        .into_iter()
        .map(|builtin| SemanticBuiltinCatalogItem {
            id: builtin.name().to_string(),
            entity_type: "extension".to_string(),
            name: builtin.name().to_string(),
            display_name: builtin_display_name(builtin).to_string(),
            aliases: builtin_aliases(builtin),
            description: Some(builtin.description().to_string()),
            scope: "builtin".to_string(),
            is_platform: builtin.is_platform(),
            metadata: serde_json::json!({
                "builtin": true,
                "mcpName": builtin.mcp_name(),
            }),
        })
        .collect()
}

async fn get_semantic_index(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
) -> Result<Json<SemanticIndexResponse>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Forbidden".to_string()));
    }

    let team_object_id = ObjectId::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid team id".to_string()))?;

    let portals = state
        .db
        .collection::<SemanticPortalRow>(collections::PORTALS)
        .find(
            doc! { "team_id": team_object_id, "is_deleted": false },
            portal_projection(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let agents = state
        .db
        .collection::<SemanticAgentRow>(collections::TEAM_AGENTS)
        .find(
            doc! { "team_id": &id },
            FindOptions::builder()
                .projection(doc! {
                    "_id": 1,
                    "agent_id": 1,
                    "team_id": 1,
                    "name": 1,
                    "agent_domain": 1,
                    "agent_role": 1,
                    "owner_manager_agent_id": 1,
                    "enabled_extensions": 1,
                    "custom_extensions": 1,
                    "status": 1,
                    "updated_at": 1
                })
                .build(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let documents = state
        .db
        .collection::<SemanticDocumentRow>(collections::DOCUMENTS)
        .find(
            doc! { "team_id": team_object_id, "is_deleted": false },
            document_projection(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let folders = state
        .db
        .collection::<SemanticFolderRow>(collections::FOLDERS)
        .find(
            doc! { "team_id": team_object_id, "is_deleted": false },
            folder_projection(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let skills = state
        .db
        .collection::<SemanticSkillRow>(collections::SKILLS)
        .find(
            doc! { "team_id": team_object_id, "is_deleted": false },
            skill_projection(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let extensions = state
        .db
        .collection::<SemanticExtensionRow>(collections::EXTENSIONS)
        .find(
            doc! { "team_id": team_object_id, "is_deleted": false },
            extension_projection(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let governance_states = state
        .db
        .collection::<SemanticGovernanceStateRow>(collections::AVATAR_GOVERNANCE_STATES)
        .find(doc! { "team_id": &id }, governance_projection())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let portal_lookup = portals
        .iter()
        .map(|portal| {
            (
                portal.id.to_hex(),
                (portal.name.as_str(), portal.slug.as_str()),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();

    let mut entities = Vec::new();
    let mut updated_candidates = Vec::new();
    let mut agent_extension_usage = std::collections::HashMap::<
        String,
        (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Vec<String>,
        ),
    >::new();

    for portal in &portals {
        updated_candidates.push(portal.updated_at);
        entities.push(make_semantic_entity(
            portal.id.to_hex(),
            "portal",
            portal.name.clone(),
            None,
            vec![portal.slug.clone(), format!("/p/{}", portal.slug)],
            Some(match portal.status {
                crate::models::mongo::PortalStatus::Draft => "draft".to_string(),
                crate::models::mongo::PortalStatus::Published => "published".to_string(),
                crate::models::mongo::PortalStatus::Archived => "archived".to_string(),
            }),
            "team",
            &id,
            Some(portal.id.to_hex()),
            serde_json::json!({
                "slug": portal.slug,
                "domain": portal.domain,
                "managerAgentId": portal.coding_agent_id,
                "serviceAgentId": portal.service_agent_id,
            }),
        ));
    }

    for agent in &agents {
        updated_candidates.push(agent.updated_at);
        let mut aliases = Vec::new();
        if let Some(role) = &agent.agent_role {
            aliases.push(role.clone());
        }
        if let Some(domain) = &agent.agent_domain {
            aliases.push(domain.clone());
        }
        let enabled_builtin_extensions = agent
            .enabled_extensions
            .iter()
            .filter(|extension| extension.enabled)
            .map(|extension| extension.extension.name().to_string())
            .collect::<Vec<_>>();
        let enabled_custom_extensions = agent
            .custom_extensions
            .iter()
            .filter(|extension| extension.enabled)
            .map(|extension| extension.name.clone())
            .collect::<Vec<_>>();

        for extension in agent
            .custom_extensions
            .iter()
            .filter(|extension| extension.enabled)
        {
            let key = extension.name.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            let entry = agent_extension_usage.entry(key).or_insert_with(|| {
                (
                    extension.name.clone(),
                    extension.ext_type.clone(),
                    extension.source.clone(),
                    extension.source_extension_id.clone(),
                    Vec::new(),
                )
            });
            if !entry.4.iter().any(|agent_name| agent_name == &agent.name) {
                entry.4.push(agent.name.clone());
            }
        }

        entities.push(make_semantic_entity(
            agent.agent_id.clone(),
            "agent",
            agent.name.clone(),
            None,
            aliases,
            Some(agent.status.clone()),
            "team",
            &agent.team_id,
            None,
            serde_json::json!({
                "domain": agent.agent_domain,
                "role": agent.agent_role,
                "ownerManagerAgentId": agent.owner_manager_agent_id,
                "enabledBuiltinExtensions": enabled_builtin_extensions,
                "enabledCustomExtensions": enabled_custom_extensions,
            }),
        ));
    }

    for document in &documents {
        updated_candidates.push(document.updated_at);
        entities.push(make_semantic_entity(
            document.id.to_hex(),
            "document",
            document.name.clone(),
            document.display_name.clone(),
            Vec::new(),
            Some(match document.status {
                DocumentStatus::Active => "active".to_string(),
                DocumentStatus::Draft => "draft".to_string(),
                DocumentStatus::Accepted => "accepted".to_string(),
                DocumentStatus::Archived => "archived".to_string(),
                DocumentStatus::Superseded => "superseded".to_string(),
            }),
            "team",
            &id,
            None,
            serde_json::json!({
                "folderPath": document.folder_path,
                "mimeType": document.mime_type,
                "fileSize": document.file_size,
                "origin": match document.origin {
                    DocumentOrigin::Human => "human",
                    DocumentOrigin::Agent => "agent",
                },
            }),
        ));
    }

    for folder in &folders {
        updated_candidates.push(folder.updated_at);
        entities.push(make_semantic_entity(
            folder.id.to_hex(),
            "folder",
            folder.name.clone(),
            Some(folder.full_path.clone()),
            vec![folder.full_path.clone()],
            None,
            "team",
            &id,
            None,
            serde_json::json!({
                "fullPath": folder.full_path,
                "isSystem": folder.is_system,
            }),
        ));
    }

    for skill in &skills {
        updated_candidates.push(skill.updated_at);
        entities.push(make_semantic_entity(
            skill.id.to_hex(),
            "skill",
            skill.name.clone(),
            None,
            Vec::new(),
            None,
            "team",
            &id,
            None,
            serde_json::json!({
                "description": skill.description,
                "visibility": skill.visibility,
                "protectionLevel": skill.protection_level,
                "version": skill.version,
            }),
        ));
    }

    for extension in &extensions {
        updated_candidates.push(extension.updated_at);
        entities.push(make_semantic_entity(
            extension.id.to_hex(),
            "extension",
            extension.name.clone(),
            None,
            Vec::new(),
            None,
            "team",
            &id,
            None,
            serde_json::json!({
                "description": extension.description,
                "extensionType": extension.extension_type,
                "source": extension.source,
            }),
        ));
    }

    let known_extension_names = entities
        .iter()
        .filter(|entity| entity.entity_type == "extension")
        .map(|entity| entity.name.trim().to_lowercase())
        .collect::<std::collections::HashSet<_>>();

    for (normalized_name, (name, ext_type, source, source_extension_id, agent_names)) in
        agent_extension_usage
    {
        if known_extension_names.contains(&normalized_name) {
            continue;
        }
        entities.push(make_semantic_entity(
            format!("agent-extension:{}:{}", id, normalized_name),
            "extension",
            name.clone(),
            None,
            source_extension_id.into_iter().collect(),
            Some("enabled".to_string()),
            "agent",
            &id,
            None,
            serde_json::json!({
                "extensionType": ext_type,
                "source": source,
                "agents": agent_names,
                "derivedFromAgentConfig": true,
            }),
        ));
    }

    for state_row in &governance_states {
        updated_candidates.push(state_row.updated_at);
        entities.extend(parse_governance_request_entities(
            &id,
            &portal_lookup,
            state_row,
        ));
    }

    let latest_updated_at = updated_candidates
        .into_iter()
        .max()
        .unwrap_or_else(Utc::now);
    let generated_at = Utc::now();
    let version = format!(
        "{}:{}:{}:{}:{}:{}:{}:{}",
        id,
        latest_updated_at.timestamp_millis(),
        portals.len(),
        agents.len(),
        documents.len(),
        folders.len(),
        skills.len(),
        entities
            .iter()
            .filter(|item| item.entity_type == "governance_request")
            .count()
    );

    Ok(Json(SemanticIndexResponse {
        team_id: id,
        version,
        generated_at: generated_at.to_rfc3339(),
        entities,
        builtin_catalog: builtin_catalog(),
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
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[derive(Debug, Deserialize)]
struct TeamSettingsAgentRow {
    agent_id: String,
    #[serde(default)]
    agent_domain: Option<String>,
    #[serde(default)]
    agent_role: Option<String>,
    #[serde(default)]
    owner_manager_agent_id: Option<String>,
}

fn is_general_settings_agent(agent: &TeamSettingsAgentRow) -> bool {
    if agent
        .owner_manager_agent_id
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        return false;
    }

    if matches!(
        agent.agent_domain.as_deref(),
        Some("digital_avatar") | Some("ecosystem_portal")
    ) {
        return false;
    }

    !matches!(
        agent.agent_role.as_deref(),
        Some("manager") | Some("service")
    )
}

async fn load_team_settings_agent_sets(
    db: &MongoDb,
    team_id: &str,
) -> Result<(HashSet<String>, HashSet<String>), mongodb::error::Error> {
    let coll = db.collection::<TeamSettingsAgentRow>(collections::TEAM_AGENTS);
    let options = FindOptions::builder()
        .projection(doc! {
            "agent_id": 1,
            "agent_domain": 1,
            "agent_role": 1,
            "owner_manager_agent_id": 1,
        })
        .build();
    let rows: Vec<TeamSettingsAgentRow> = coll
        .find(doc! { "team_id": team_id }, options)
        .await?
        .try_collect()
        .await?;

    let mut valid_agent_ids = HashSet::new();
    let mut valid_general_agent_ids = HashSet::new();
    for row in rows {
        let agent_id = row.agent_id.trim().to_string();
        if agent_id.is_empty() {
            continue;
        }
        if is_general_settings_agent(&row) {
            valid_general_agent_ids.insert(agent_id.clone());
        }
        valid_agent_ids.insert(agent_id);
    }

    Ok((valid_agent_ids, valid_general_agent_ids))
}

fn sanitize_team_settings_agent_ids(
    settings: &mut TeamSettings,
    valid_agent_ids: &HashSet<String>,
    valid_general_agent_ids: &HashSet<String>,
) -> bool {
    let mut changed = false;

    if settings
        .document_analysis
        .agent_id
        .as_ref()
        .map(|id| !valid_agent_ids.contains(id))
        .unwrap_or(false)
    {
        settings.document_analysis.agent_id = None;
        changed = true;
    }

    if settings
        .ai_describe
        .agent_id
        .as_ref()
        .map(|id| !valid_agent_ids.contains(id))
        .unwrap_or(false)
    {
        settings.ai_describe.agent_id = None;
        changed = true;
    }

    if settings
        .general_agent
        .default_agent_id
        .as_ref()
        .map(|id| !valid_general_agent_ids.contains(id))
        .unwrap_or(false)
    {
        settings.general_agent.default_agent_id = None;
        changed = true;
    }

    changed
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
    #[serde(rename = "aiDescribe")]
    pub ai_describe: AiDescribeSettingsResponse,
    #[serde(rename = "generalAgent")]
    pub general_agent: GeneralAgentSettingsResponse,
    #[serde(rename = "chatAssistant")]
    pub chat_assistant: ChatAssistantSettingsResponse,
    #[serde(rename = "shellSecurity")]
    pub shell_security: ShellSecuritySettingsResponse,
    #[serde(rename = "avatarGovernance")]
    pub avatar_governance: AvatarGovernanceSettingsResponse,
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

#[derive(Debug, Serialize)]
pub struct AiDescribeSettingsResponse {
    #[serde(rename = "agentId", skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GeneralAgentSettingsResponse {
    #[serde(rename = "defaultAgentId", skip_serializing_if = "Option::is_none")]
    pub default_agent_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatAssistantSettingsResponse {
    #[serde(rename = "companyName", skip_serializing_if = "Option::is_none")]
    pub company_name: Option<String>,
    #[serde(rename = "departmentName", skip_serializing_if = "Option::is_none")]
    pub department_name: Option<String>,
    #[serde(rename = "teamName", skip_serializing_if = "Option::is_none")]
    pub team_name: Option<String>,
    #[serde(rename = "teamSummary", skip_serializing_if = "Option::is_none")]
    pub team_summary: Option<String>,
    #[serde(rename = "businessContext", skip_serializing_if = "Option::is_none")]
    pub business_context: Option<String>,
    #[serde(rename = "toneHint", skip_serializing_if = "Option::is_none")]
    pub tone_hint: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AvatarGovernanceSettingsResponse {
    #[serde(rename = "autoProposalTriggerCount")]
    pub auto_proposal_trigger_count: i64,
    #[serde(rename = "managerApprovalMode")]
    pub manager_approval_mode: String,
    #[serde(rename = "optimizationMode")]
    pub optimization_mode: String,
    #[serde(rename = "lowRiskAction")]
    pub low_risk_action: String,
    #[serde(rename = "mediumRiskAction")]
    pub medium_risk_action: String,
    #[serde(rename = "highRiskAction")]
    pub high_risk_action: String,
    #[serde(rename = "autoCreateCapabilityRequests")]
    pub auto_create_capability_requests: bool,
    #[serde(rename = "autoCreateOptimizationTickets")]
    pub auto_create_optimization_tickets: bool,
    #[serde(rename = "requireHumanForPublish")]
    pub require_human_for_publish: bool,
}

#[derive(Debug, Serialize)]
pub struct ShellSecuritySettingsResponse {
    pub mode: String,
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
            ai_describe: AiDescribeSettingsResponse {
                agent_id: s.ai_describe.agent_id,
            },
            general_agent: GeneralAgentSettingsResponse {
                default_agent_id: s.general_agent.default_agent_id,
            },
            chat_assistant: ChatAssistantSettingsResponse {
                company_name: s.chat_assistant.company_name,
                department_name: s.chat_assistant.department_name,
                team_name: s.chat_assistant.team_name,
                team_summary: s.chat_assistant.team_summary,
                business_context: s.chat_assistant.business_context,
                tone_hint: s.chat_assistant.tone_hint,
            },
            shell_security: ShellSecuritySettingsResponse {
                mode: match s.shell_security.mode {
                    crate::models::mongo::ShellSecurityMode::Off => "off".to_string(),
                    crate::models::mongo::ShellSecurityMode::Warn => "warn".to_string(),
                    crate::models::mongo::ShellSecurityMode::Block => "block".to_string(),
                },
            },
            avatar_governance: AvatarGovernanceSettingsResponse {
                auto_proposal_trigger_count: s.avatar_governance.auto_proposal_trigger_count,
                manager_approval_mode: s.avatar_governance.manager_approval_mode,
                optimization_mode: s.avatar_governance.optimization_mode,
                low_risk_action: s.avatar_governance.low_risk_action,
                medium_risk_action: s.avatar_governance.medium_risk_action,
                high_risk_action: s.avatar_governance.high_risk_action,
                auto_create_capability_requests: s
                    .avatar_governance
                    .auto_create_capability_requests,
                auto_create_optimization_tickets: s
                    .avatar_governance
                    .auto_create_optimization_tickets,
                require_human_for_publish: s.avatar_governance.require_human_for_publish,
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct UpdateTeamSettingsRequest {
    #[serde(rename = "documentAnalysis")]
    document_analysis: Option<UpdateDocAnalysisRequest>,
    #[serde(rename = "aiDescribe")]
    ai_describe: Option<UpdateAiDescribeSettingsRequest>,
    #[serde(rename = "generalAgent")]
    general_agent: Option<UpdateGeneralAgentSettingsRequest>,
    #[serde(rename = "chatAssistant")]
    chat_assistant: Option<UpdateChatAssistantSettingsRequest>,
    #[serde(rename = "shellSecurity")]
    shell_security: Option<UpdateShellSecuritySettingsRequest>,
    #[serde(rename = "avatarGovernance")]
    avatar_governance: Option<UpdateAvatarGovernanceSettingsRequest>,
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

#[derive(Debug, Deserialize)]
struct UpdateAiDescribeSettingsRequest {
    #[serde(rename = "agentId")]
    agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateGeneralAgentSettingsRequest {
    #[serde(rename = "defaultAgentId")]
    default_agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateChatAssistantSettingsRequest {
    #[serde(rename = "companyName")]
    company_name: Option<String>,
    #[serde(rename = "departmentName")]
    department_name: Option<String>,
    #[serde(rename = "teamName")]
    team_name: Option<String>,
    #[serde(rename = "teamSummary")]
    team_summary: Option<String>,
    #[serde(rename = "businessContext")]
    business_context: Option<String>,
    #[serde(rename = "toneHint")]
    tone_hint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateAvatarGovernanceSettingsRequest {
    #[serde(rename = "autoProposalTriggerCount")]
    auto_proposal_trigger_count: Option<i64>,
    #[serde(rename = "managerApprovalMode")]
    manager_approval_mode: Option<String>,
    #[serde(rename = "optimizationMode")]
    optimization_mode: Option<String>,
    #[serde(rename = "lowRiskAction")]
    low_risk_action: Option<String>,
    #[serde(rename = "mediumRiskAction")]
    medium_risk_action: Option<String>,
    #[serde(rename = "highRiskAction")]
    high_risk_action: Option<String>,
    #[serde(rename = "autoCreateCapabilityRequests")]
    auto_create_capability_requests: Option<bool>,
    #[serde(rename = "autoCreateOptimizationTickets")]
    auto_create_optimization_tickets: Option<bool>,
    #[serde(rename = "requireHumanForPublish")]
    require_human_for_publish: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UpdateShellSecuritySettingsRequest {
    mode: Option<String>,
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

    let mut settings = team.settings;
    let (valid_agent_ids, valid_general_agent_ids) =
        load_team_settings_agent_sets(state.db.as_ref(), &id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if sanitize_team_settings_agent_ids(&mut settings, &valid_agent_ids, &valid_general_agent_ids) {
        service
            .update_settings(&id, settings.clone())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(Json(TeamSettingsResponse::from(settings)))
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
        return Err((
            StatusCode::FORBIDDEN,
            "Only admin/owner can update settings".to_string(),
        ));
    }

    let requested_document_agent_id = req
        .document_analysis
        .as_ref()
        .and_then(|settings| settings.agent_id.clone())
        .and_then(non_empty);
    let requested_ai_describe_agent_id = req
        .ai_describe
        .as_ref()
        .and_then(|settings| settings.agent_id.clone())
        .and_then(non_empty);
    let requested_general_agent_id = req
        .general_agent
        .as_ref()
        .and_then(|settings| settings.default_agent_id.clone())
        .and_then(non_empty);
    let (valid_agent_ids, valid_general_agent_ids) =
        load_team_settings_agent_sets(state.db.as_ref(), &id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(agent_id) = requested_document_agent_id.as_ref() {
        if !valid_agent_ids.contains(agent_id) {
            return Err((
                StatusCode::BAD_REQUEST,
                "Document analysis agent does not exist in this team".to_string(),
            ));
        }
    }

    if let Some(agent_id) = requested_ai_describe_agent_id.as_ref() {
        if !valid_agent_ids.contains(agent_id) {
            return Err((
                StatusCode::BAD_REQUEST,
                "AI describe agent does not exist in this team".to_string(),
            ));
        }
    }

    if let Some(agent_id) = requested_general_agent_id.as_ref() {
        if !valid_general_agent_ids.contains(agent_id) {
            return Err((
                StatusCode::BAD_REQUEST,
                "Default general agent must be an existing general team agent".to_string(),
            ));
        }
    }

    let mut settings = team.settings;

    // Merge document_analysis fields
    if let Some(da) = req.document_analysis {
        let d = &mut settings.document_analysis;
        if let Some(v) = da.enabled {
            d.enabled = v;
        }
        if let Some(v) = da.api_url {
            d.api_url = non_empty(v);
        }
        if let Some(v) = da.api_key {
            d.api_key = non_empty(v);
        }
        if let Some(v) = da.model {
            d.model = non_empty(v);
        }
        if let Some(v) = da.api_format {
            d.api_format = non_empty(v);
        }
        if let Some(v) = da.agent_id {
            d.agent_id = non_empty(v);
        }
        if let Some(v) = da.min_file_size {
            d.min_file_size = v;
        }
        if let Some(v) = da.max_file_size {
            d.max_file_size = v;
        }
        if let Some(v) = da.skip_mime_prefixes {
            d.skip_mime_prefixes = v;
        }
    }

    if let Some(ai_describe) = req.ai_describe {
        settings.ai_describe.agent_id = ai_describe.agent_id.and_then(non_empty);
    }

    if let Some(general_agent) = req.general_agent {
        settings.general_agent.default_agent_id =
            general_agent.default_agent_id.and_then(non_empty);
    }

    if let Some(chat_assistant) = req.chat_assistant {
        settings.chat_assistant.company_name =
            chat_assistant.company_name.and_then(non_empty);
        settings.chat_assistant.department_name =
            chat_assistant.department_name.and_then(non_empty);
        settings.chat_assistant.team_name =
            chat_assistant.team_name.and_then(non_empty);
        settings.chat_assistant.team_summary =
            chat_assistant.team_summary.and_then(non_empty);
        settings.chat_assistant.business_context =
            chat_assistant.business_context.and_then(non_empty);
        settings.chat_assistant.tone_hint =
            chat_assistant.tone_hint.and_then(non_empty);
    }

    if let Some(avatar) = req.avatar_governance {
        let a = &mut settings.avatar_governance;
        if let Some(v) = avatar.auto_proposal_trigger_count {
            a.auto_proposal_trigger_count = v.clamp(1, 10);
        }
        if let Some(v) = avatar.manager_approval_mode {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                a.manager_approval_mode = trimmed.to_string();
            }
        }
        if let Some(v) = avatar.optimization_mode {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                a.optimization_mode = trimmed.to_string();
            }
        }
        if let Some(v) = avatar.low_risk_action {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                a.low_risk_action = trimmed.to_string();
            }
        }
        if let Some(v) = avatar.medium_risk_action {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                a.medium_risk_action = trimmed.to_string();
            }
        }
        if let Some(v) = avatar.high_risk_action {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                a.high_risk_action = trimmed.to_string();
            }
        }
        if let Some(v) = avatar.auto_create_capability_requests {
            a.auto_create_capability_requests = v;
        }
        if let Some(v) = avatar.auto_create_optimization_tickets {
            a.auto_create_optimization_tickets = v;
        }
        if let Some(v) = avatar.require_human_for_publish {
            a.require_human_for_publish = v;
        }
    }

    if let Some(shell) = req.shell_security {
        if let Some(v) = shell.mode {
            match v.trim().to_ascii_lowercase().as_str() {
                "off" => {
                    settings.shell_security.mode = crate::models::mongo::ShellSecurityMode::Off
                }
                "warn" => {
                    settings.shell_security.mode = crate::models::mongo::ShellSecurityMode::Warn
                }
                "block" => {
                    settings.shell_security.mode = crate::models::mongo::ShellSecurityMode::Block
                }
                _ => {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "Invalid shellSecurity.mode; expected off, warn, or block".to_string(),
                    ));
                }
            }
        }
    }

    sanitize_team_settings_agent_ids(&mut settings, &valid_agent_ids, &valid_general_agent_ids);

    let updated = service
        .update_settings(&id, settings)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(TeamSettingsResponse::from(updated.settings)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::mongo::{ShellSecurityMode, TeamSettings};

    #[test]
    fn team_settings_response_serializes_shell_security() {
        let mut settings = TeamSettings::default();
        settings.shell_security.mode = ShellSecurityMode::Warn;
        settings.ai_describe.agent_id = Some("agent-123".to_string());
        settings.general_agent.default_agent_id = Some("agent-456".to_string());

        let value = serde_json::to_value(TeamSettingsResponse::from(settings)).unwrap();

        assert_eq!(value["shellSecurity"]["mode"], "warn");
        assert_eq!(value["aiDescribe"]["agentId"], "agent-123");
        assert_eq!(value["generalAgent"]["defaultAgentId"], "agent-456");
    }
}
