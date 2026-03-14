use std::sync::Arc;

use agime_team::services::mongo::TeamService;
use agime_team::AuthenticatedUserId;
use axum::{
    extract::{Extension, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;

use crate::agent::skill_registry_tools::SkillRegistryToolsProvider;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RegistrySearchQuery {
    #[serde(rename = "teamId")]
    pub team_id: String,
    pub query: String,
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct RegistryPopularQuery {
    #[serde(rename = "teamId")]
    pub team_id: String,
    pub mode: Option<String>,
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct RegistryUpdatesQuery {
    #[serde(rename = "teamId")]
    pub team_id: String,
    #[serde(rename = "importedSkillId")]
    pub imported_skill_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegistryPreviewRequest {
    #[serde(rename = "teamId")]
    pub team_id: String,
    pub source: String,
    #[serde(rename = "skillId")]
    pub skill_id: String,
    #[serde(rename = "sourceRef")]
    pub source_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegistryImportRequest {
    #[serde(rename = "teamId")]
    pub team_id: String,
    pub source: String,
    #[serde(rename = "skillId")]
    pub skill_id: String,
    #[serde(rename = "sourceRef")]
    pub source_ref: Option<String>,
    pub visibility: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegistryUpgradeRequest {
    #[serde(rename = "teamId")]
    pub team_id: String,
    #[serde(rename = "importedSkillId")]
    pub imported_skill_id: String,
    #[serde(default)]
    pub force: bool,
}

pub fn skill_registry_router(state: Arc<AppState>) -> Router {
    let search_state = state.clone();
    let popular_state = state.clone();
    let imported_state = state.clone();
    let preview_state = state.clone();
    let import_state = state.clone();
    let updates_state = state.clone();
    let upgrade_state = state;
    Router::new()
        .route(
            "/popular",
            get(move |Extension(user), Query(query)| {
                list_popular_registry_skills(popular_state.clone(), user, query)
            }),
        )
        .route(
            "/search",
            get(move |Extension(user), Query(query)| {
                search_registry(search_state.clone(), user, query)
            }),
        )
        .route(
            "/imported",
            get(move |Extension(user), Query(query)| {
                list_imported_registry_skills(imported_state.clone(), user, query)
            }),
        )
        .route(
            "/preview",
            post(move |Extension(user), Json(req)| {
                preview_registry_skill(preview_state.clone(), user, req)
            }),
        )
        .route(
            "/import",
            post(move |Extension(user), Json(req)| {
                import_registry_skill(import_state.clone(), user, req)
            }),
        )
        .route(
            "/updates",
            get(move |Extension(user), Query(query)| {
                check_registry_updates(updates_state.clone(), user, query)
            }),
        )
        .route(
            "/upgrade",
            post(move |Extension(user), Json(req)| {
                upgrade_registry_skill(upgrade_state.clone(), user, req)
            }),
        )
}

fn json_error(status: StatusCode, message: impl ToString) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": message.to_string()
        })),
    )
        .into_response()
}

fn is_team_member(team: &agime_team::models::mongo::Team, user_id: &str) -> bool {
    team.members.iter().any(|member| member.user_id == user_id)
}

fn can_manage_team(team: &agime_team::models::mongo::Team, user_id: &str) -> bool {
    team.members.iter().any(|member| {
        member.user_id == user_id
            && matches!(member.role.to_ascii_lowercase().as_str(), "owner" | "admin")
    })
}

async fn load_team(
    state: &Arc<AppState>,
    team_id: &str,
) -> Result<agime_team::models::mongo::Team, Response> {
    let db = state.require_mongodb()?;
    TeamService::new((*db).clone())
        .get(team_id)
        .await
        .map_err(|err| json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "Team not found"))
}

async fn require_team_member(
    state: &Arc<AppState>,
    team_id: &str,
    user_id: &str,
) -> Result<(), Response> {
    let team = load_team(state, team_id).await?;
    if !is_team_member(&team, user_id) {
        return Err(json_error(
            StatusCode::FORBIDDEN,
            "Only team members can access skill registry",
        ));
    }
    Ok(())
}

async fn require_team_manager(
    state: &Arc<AppState>,
    team_id: &str,
    user_id: &str,
) -> Result<(), Response> {
    let team = load_team(state, team_id).await?;
    if !can_manage_team(&team, user_id) {
        return Err(json_error(
            StatusCode::FORBIDDEN,
            "Only team admin or owner can upgrade imported skills",
        ));
    }
    Ok(())
}

fn registry_provider(
    state: &Arc<AppState>,
    team_id: &str,
    actor_id: &str,
) -> Result<SkillRegistryToolsProvider, Response> {
    let db = state.require_mongodb()?;
    Ok(SkillRegistryToolsProvider::new(
        db,
        team_id.to_string(),
        actor_id.to_string(),
    ))
}

async fn search_registry(
    state: Arc<AppState>,
    user: AuthenticatedUserId,
    query: RegistrySearchQuery,
) -> Result<Json<serde_json::Value>, Response> {
    require_team_member(&state, &query.team_id, &user.0).await?;
    let provider = registry_provider(&state, &query.team_id, &user.0)?;
    let result = provider
        .search_registry(&query.query, query.limit)
        .await
        .map_err(|err| json_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    Ok(Json(result))
}

async fn list_popular_registry_skills(
    state: Arc<AppState>,
    user: AuthenticatedUserId,
    query: RegistryPopularQuery,
) -> Result<Json<serde_json::Value>, Response> {
    require_team_member(&state, &query.team_id, &user.0).await?;
    let provider = registry_provider(&state, &query.team_id, &user.0)?;
    let result = provider
        .list_popular_registry_skills(query.mode.as_deref(), query.limit)
        .await
        .map_err(|err| json_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    Ok(Json(result))
}

async fn preview_registry_skill(
    state: Arc<AppState>,
    user: AuthenticatedUserId,
    req: RegistryPreviewRequest,
) -> Result<Json<serde_json::Value>, Response> {
    require_team_member(&state, &req.team_id, &user.0).await?;
    let provider = registry_provider(&state, &req.team_id, &user.0)?;
    let result = provider
        .preview_registry_skill(&req.source, &req.skill_id, req.source_ref.as_deref())
        .await
        .map_err(|err| json_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    Ok(Json(result))
}

async fn list_imported_registry_skills(
    state: Arc<AppState>,
    user: AuthenticatedUserId,
    query: RegistryUpdatesQuery,
) -> Result<Json<serde_json::Value>, Response> {
    require_team_member(&state, &query.team_id, &user.0).await?;
    let provider = registry_provider(&state, &query.team_id, &user.0)?;
    let result = provider
        .list_imported_registry_skills()
        .await
        .map_err(|err| json_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    Ok(Json(result))
}

async fn import_registry_skill(
    state: Arc<AppState>,
    user: AuthenticatedUserId,
    req: RegistryImportRequest,
) -> Result<Json<serde_json::Value>, Response> {
    require_team_member(&state, &req.team_id, &user.0).await?;
    let provider = registry_provider(&state, &req.team_id, &user.0)?;
    let result = provider
        .import_registry_skill(
            &req.source,
            &req.skill_id,
            req.source_ref.as_deref(),
            req.visibility.as_deref(),
        )
        .await
        .map_err(|err| json_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    Ok(Json(result))
}

async fn check_registry_updates(
    state: Arc<AppState>,
    user: AuthenticatedUserId,
    query: RegistryUpdatesQuery,
) -> Result<Json<serde_json::Value>, Response> {
    require_team_member(&state, &query.team_id, &user.0).await?;
    let provider = registry_provider(&state, &query.team_id, &user.0)?;
    let result = provider
        .check_registry_updates(query.imported_skill_id.as_deref())
        .await
        .map_err(|err| json_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    Ok(Json(result))
}

async fn upgrade_registry_skill(
    state: Arc<AppState>,
    user: AuthenticatedUserId,
    req: RegistryUpgradeRequest,
) -> Result<Json<serde_json::Value>, Response> {
    require_team_manager(&state, &req.team_id, &user.0).await?;
    let provider = registry_provider(&state, &req.team_id, &user.0)?;
    let result = provider
        .upgrade_registry_skill(&req.imported_skill_id, req.force)
        .await
        .map_err(|err| json_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    Ok(Json(result))
}
