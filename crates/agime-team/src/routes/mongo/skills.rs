//! MongoDB routes - Skills API

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::teams::{can_manage_team, is_team_member, AppState};
use super::InstallResponse;
use crate::models::mongo::SmartLogContext;
use crate::routes::skills::generate_access_token;
use crate::models::mongo::SkillStorageType;
use crate::services::mongo::{SkillService, TeamService};
use crate::services::mongo::skill_service_mongo::generate_skill_md_for_inline;
use crate::AuthenticatedUserId;

#[derive(Debug, Deserialize)]
pub struct SkillQuery {
    #[serde(rename = "teamId")]
    pub team_id: Option<String>,
    pub page: Option<u64>,
    pub limit: Option<u64>,
    pub search: Option<String>,
    pub sort: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SkillsResponse {
    pub skills: Vec<SkillInfo>,
    pub total: u64,
    pub page: u64,
    pub limit: u64,
    #[serde(rename = "totalPages")]
    pub total_pages: u64,
}

/// Skill info matching frontend SharedSkill interface
#[derive(Debug, Serialize)]
pub struct SkillInfo {
    pub id: String,
    #[serde(rename = "teamId")]
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    pub content: Option<String>,
    #[serde(rename = "storageType")]
    pub storage_type: String,
    #[serde(rename = "authorId")]
    pub author_id: String,
    pub version: String,
    pub visibility: String,
    #[serde(rename = "protectionLevel")]
    pub protection_level: String,
    pub tags: Vec<String>,
    #[serde(rename = "useCount")]
    pub use_count: i32,
    #[serde(rename = "aiDescription", skip_serializing_if = "Option::is_none")]
    pub ai_description: Option<String>,
    #[serde(rename = "aiDescriptionLang", skip_serializing_if = "Option::is_none")]
    pub ai_description_lang: Option<String>,
    #[serde(rename = "aiDescribedAt", skip_serializing_if = "Option::is_none")]
    pub ai_described_at: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct SkillFileRequest {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateSkillRequest {
    #[serde(rename = "teamId")]
    pub team_id: String,
    pub name: String,
    pub content: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub visibility: Option<String>,
    /// Full SKILL.md text (triggers package mode when combined with files)
    #[serde(rename = "skillMd")]
    pub skill_md: Option<String>,
    /// Supporting files for package mode
    pub files: Option<Vec<SkillFileRequest>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSkillRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct VerifyAccessRequest {
    #[serde(rename = "userId")]
    pub _user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BackfillRequest {
    #[serde(rename = "teamId")]
    pub team_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyAccessResponse {
    pub authorized: bool,
    pub token: Option<String>,
    pub expires_at: Option<String>,
    pub protection_level: String,
    pub allows_local_install: bool,
    pub error: Option<String>,
}

pub fn skill_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/skills", get(list_skills).post(create_skill))
        .route("/skills/backfill-md", post(backfill_skill_md))
        .route(
            "/skills/{id}",
            get(get_skill).put(update_skill).delete(delete_skill),
        )
        .route("/skills/{id}/install", post(install_skill))
        .route("/skills/{id}/uninstall", delete(uninstall_skill))
        .route("/skills/{id}/verify-access", post(verify_skill_access))
}

fn allows_local_install_str(level: &str) -> bool {
    matches!(
        level.trim().to_ascii_lowercase().as_str(),
        "public" | "team_installable" | "none"
    )
}

fn skill_to_json(skill: crate::models::mongo::Skill) -> serde_json::Value {
    let storage_type = match skill.storage_type {
        crate::models::mongo::SkillStorageType::Inline => "inline",
        crate::models::mongo::SkillStorageType::Package => "package",
    };

    serde_json::json!({
        "id": skill.id.map(|id| id.to_hex()).unwrap_or_default(),
        "teamId": skill.team_id.to_hex(),
        "name": skill.name,
        "description": skill.description,
        "content": skill.content,
        "storageType": storage_type,
        "skillMd": skill.skill_md,
        "files": skill.files,
        "manifest": skill.manifest,
        "packageUrl": skill.package_url,
        "packageHash": skill.package_hash,
        "packageSize": skill.package_size,
        "metadata": skill.metadata,
        "authorId": skill.created_by,
        "version": skill.version,
        "previousVersionId": skill.previous_version_id,
        "visibility": skill.visibility,
        "protectionLevel": skill.protection_level,
        "dependencies": skill.dependencies,
        "tags": skill.tags,
        "useCount": skill.use_count,
        "aiDescription": skill.ai_description,
        "aiDescriptionLang": skill.ai_description_lang,
        "aiDescribedAt": skill.ai_described_at.map(|dt| dt.to_rfc3339()),
        "createdAt": skill.created_at.to_rfc3339(),
        "updatedAt": skill.updated_at.to_rfc3339()
    })
}

async fn list_skills(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Query(query): Query<SkillQuery>,
) -> Result<Json<SkillsResponse>, (StatusCode, String)> {
    let team_id = query
        .team_id
        .ok_or((StatusCode::BAD_REQUEST, "teamId required".to_string()))?;

    // Check team membership
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view skills".to_string(),
        ));
    }

    let service = SkillService::new((*state.db).clone());
    let result = service
        .list(
            &team_id,
            query.page,
            query.limit,
            query.search.as_deref(),
            query.sort.as_deref(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let skills: Vec<SkillInfo> = result
        .items
        .into_iter()
        .map(|s| SkillInfo {
            id: s.id,
            team_id: s.team_id,
            name: s.name,
            description: s.description,
            content: None,
            storage_type: s.storage_type,
            author_id: s.author_id,
            version: s.version,
            visibility: s.visibility,
            protection_level: s.protection_level,
            tags: s.tags,
            use_count: s.use_count,
            ai_description: s.ai_description,
            ai_description_lang: s.ai_description_lang,
            ai_described_at: s.ai_described_at.map(|dt| dt.to_rfc3339()),
            created_at: s.created_at.to_rfc3339(),
            updated_at: s.updated_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(SkillsResponse {
        skills,
        total: result.total,
        page: result.page,
        limit: result.limit,
        total_pages: result.total_pages,
    }))
}

async fn create_skill(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Json(req): Json<CreateSkillRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Check team membership
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&req.team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can create skills".to_string(),
        ));
    }

    let service = SkillService::new((*state.db).clone());
    let skill = match (req.skill_md, req.files) {
        // Package mode: both skill_md and files provided
        (Some(skill_md), Some(files)) => {
            service
                .create_package(
                    &req.team_id,
                    &user.0,
                    &req.name,
                    req.description.clone(),
                    skill_md,
                    files
                        .into_iter()
                        .map(|f| crate::models::mongo::SkillFile {
                            path: f.path,
                            content: f.content,
                        })
                        .collect(),
                    req.content.unwrap_or_default(),
                    req.tags,
                    req.visibility,
                )
                .await
                .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
        }
        // Inline mode: neither skill_md nor files
        (None, None) => {
            service
                .create(
                    &req.team_id,
                    &user.0,
                    &req.name,
                    &req.content.unwrap_or_default(),
                    req.description.clone(),
                    req.tags,
                    req.visibility,
                )
                .await
                .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
        }
        // Partial package: reject
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Both skillMd and files must be provided together for package mode".to_string(),
            ));
        }
    };

    // Smart Log trigger
    if let Some(trigger) = &state.smart_log_trigger {
        trigger.trigger(SmartLogContext::new(
            req.team_id.clone(),
            user.0.clone(),
            "create",
            "skill",
            skill.id.map(|id| id.to_hex()).unwrap_or_default(),
            skill.name.clone(),
            Some(format!(
                "名称: {}\n描述: {}\n内容: {}",
                skill.name,
                req.description.unwrap_or_default(),
                skill.content.as_deref().unwrap_or("")
            )),
        ));
    }

    Ok(Json(skill_to_json(skill)))
}

async fn delete_skill(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let service = SkillService::new((*state.db).clone());

    // Get skill to find team_id
    let skill = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Skill not found".to_string()))?;

    // Check team permission
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&skill.team_id.to_hex())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Only admin/owner can delete skills
    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team admin or owner can delete skills".to_string(),
        ));
    }

    // Smart Log trigger (before delete)
    if let Some(trigger) = &state.smart_log_trigger {
        trigger.trigger(SmartLogContext::new(
            skill.team_id.to_hex(),
            user.0.clone(),
            "delete",
            "skill",
            skill.id.map(|id| id.to_hex()).unwrap_or_default(),
            skill.name.clone(),
            None,
        ));
    }

    service
        .delete(&id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn get_skill(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let service = SkillService::new((*state.db).clone());
    let mut skill = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Skill not found".to_string()))?;

    // Check team membership
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&skill.team_id.to_hex())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view skills".to_string(),
        ));
    }

    // Lazy-generate skill_md for inline skills that are missing it
    if skill.skill_md.is_none() && matches!(skill.storage_type, SkillStorageType::Inline) {
        let generated = generate_skill_md_for_inline(
            &skill.name,
            skill.description.as_deref(),
            skill.content.as_deref().unwrap_or(""),
            &skill.version,
        );
        skill.skill_md = Some(generated);

        // Fire-and-forget: persist to MongoDB
        let db = (*state.db).clone();
        let skill_id = id.clone();
        let skill_md_clone = skill.skill_md.clone();
        tokio::spawn(async move {
            let svc = SkillService::new(db);
            if let Err(e) = svc.update_skill_md(&skill_id, skill_md_clone.as_deref().unwrap_or("")).await {
                tracing::warn!("Failed to persist lazy-generated skill_md for {}: {}", skill_id, e);
            }
        });
    }

    Ok(Json(skill_to_json(skill)))
}

async fn update_skill(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSkillRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let service = SkillService::new((*state.db).clone());

    // Get skill to find team_id
    let skill = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Skill not found".to_string()))?;

    // Check team permission
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&skill.team_id.to_hex())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Only admin/owner can update skills
    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team admin or owner can update skills".to_string(),
        ));
    }

    let skill = service
        .update(&id, req.name, req.description, req.content)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Smart Log trigger
    if let Some(trigger) = &state.smart_log_trigger {
        trigger.trigger(SmartLogContext::new(
            skill.team_id.to_hex(),
            user.0.clone(),
            "update",
            "skill",
            skill.id.map(|id| id.to_hex()).unwrap_or_default(),
            skill.name.clone(),
            Some(format!(
                "名称: {}\n内容: {}",
                skill.name,
                skill.content.as_deref().unwrap_or("")
            )),
        ));
    }

    Ok(Json(skill_to_json(skill)))
}

/// Install a skill (cloud server returns success)
async fn install_skill(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
) -> Result<Json<InstallResponse>, (StatusCode, String)> {
    let service = SkillService::new((*state.db).clone());

    // Verify skill exists
    let skill = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Skill not found".to_string()))?;

    // Check team membership
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&skill.team_id.to_hex())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can install skills".to_string(),
        ));
    }

    // Increment use count
    let _ = service.increment_use_count(&id).await;

    Ok(Json(InstallResponse {
        success: true,
        resource_type: "skill".to_string(),
        resource_id: skill.id.map(|id| id.to_hex()).unwrap_or_default(),
        local_path: None,
        error: None,
    }))
}

/// Uninstall a skill
async fn uninstall_skill(Path(_id): Path<String>) -> Result<StatusCode, (StatusCode, String)> {
    // Cloud server just acknowledges the uninstall request
    // Actual uninstall happens on client side
    Ok(StatusCode::NO_CONTENT)
}

async fn verify_skill_access(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
    _req: Option<Json<VerifyAccessRequest>>,
) -> Result<Json<VerifyAccessResponse>, (StatusCode, String)> {
    let skill_service = SkillService::new((*state.db).clone());
    let team_service = TeamService::new((*state.db).clone());

    let skill = skill_service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Skill not found".to_string()))?;

    let team_id = skill.team_id.to_hex();
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Ok(Json(VerifyAccessResponse {
            authorized: false,
            token: None,
            expires_at: None,
            protection_level: skill.protection_level,
            allows_local_install: false,
            error: Some("User is not a member of this team".to_string()),
        }));
    }

    if skill.protection_level.eq_ignore_ascii_case("public") {
        return Ok(Json(VerifyAccessResponse {
            authorized: true,
            token: None,
            expires_at: None,
            protection_level: skill.protection_level.clone(),
            allows_local_install: allows_local_install_str(&skill.protection_level),
            error: None,
        }));
    }

    let expires_at = chrono::Utc::now() + chrono::Duration::hours(24);
    let token = generate_access_token(&team_id, &id, &user.0, &expires_at);

    Ok(Json(VerifyAccessResponse {
        authorized: true,
        token: Some(token),
        expires_at: Some(expires_at.to_rfc3339()),
        protection_level: skill.protection_level.clone(),
        allows_local_install: allows_local_install_str(&skill.protection_level),
        error: None,
    }))
}

/// Backfill skill_md for all inline skills missing it (any team member)
async fn backfill_skill_md(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Json(req): Json<BackfillRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Check team membership
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&req.team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can backfill skills".to_string(),
        ));
    }

    let service = SkillService::new((*state.db).clone());
    let count = service
        .backfill_skill_md(&req.team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "updated": count,
        "message": format!("Backfilled skill_md for {} skills", count)
    })))
}
