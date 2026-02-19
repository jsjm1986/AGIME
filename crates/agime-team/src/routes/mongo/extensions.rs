//! MongoDB routes - Extensions API

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
use crate::services::mongo::{ExtensionService, TeamService};
use crate::AuthenticatedUserId;

#[derive(Debug, Deserialize)]
pub struct ExtensionQuery {
    #[serde(rename = "teamId")]
    pub team_id: Option<String>,
    pub page: Option<u64>,
    pub limit: Option<u64>,
    pub search: Option<String>,
    pub sort: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExtensionsResponse {
    pub extensions: Vec<ExtensionInfo>,
    pub total: u64,
    pub page: u64,
    pub limit: u64,
    #[serde(rename = "totalPages")]
    pub total_pages: u64,
}

/// Extension info matching frontend SharedExtension interface
#[derive(Debug, Serialize)]
pub struct ExtensionInfo {
    pub id: String,
    #[serde(rename = "teamId")]
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "extensionType")]
    pub extension_type: String,
    pub config: serde_json::Value,
    #[serde(rename = "authorId")]
    pub author_id: String,
    pub version: String,
    pub visibility: String,
    #[serde(rename = "protectionLevel")]
    pub protection_level: String,
    pub tags: Vec<String>,
    #[serde(rename = "securityReviewed")]
    pub security_reviewed: bool,
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
pub struct CreateExtensionRequest {
    #[serde(rename = "teamId")]
    pub team_id: String,
    pub name: String,
    #[serde(rename = "extensionType")]
    pub extension_type: String,
    pub config: serde_json::Value,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub visibility: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateExtensionRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ReviewExtensionRequest {
    pub approved: bool,
    pub notes: Option<String>,
}

pub fn extension_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/extensions", get(list_extensions).post(create_extension))
        .route(
            "/extensions/{id}",
            get(get_extension)
                .put(update_extension)
                .delete(delete_extension),
        )
        .route("/extensions/{id}/install", post(install_extension))
        .route("/extensions/{id}/uninstall", delete(uninstall_extension))
        .route("/extensions/{id}/review", post(review_extension))
}

fn extension_to_json(ext: crate::models::mongo::Extension) -> Result<serde_json::Value, String> {
    let config_json: serde_json::Value = bson::from_document(ext.config.clone())
        .map_err(|e| format!("Failed to deserialize config: {}", e))?;

    Ok(serde_json::json!({
        "id": ext.id.map(|id| id.to_hex()).unwrap_or_default(),
        "teamId": ext.team_id.to_hex(),
        "name": ext.name,
        "description": ext.description,
        "extensionType": ext.extension_type,
        "config": config_json,
        "authorId": ext.created_by,
        "version": ext.version,
        "previousVersionId": ext.previous_version_id,
        "visibility": ext.visibility,
        "protectionLevel": ext.protection_level,
        "tags": ext.tags,
        "securityReviewed": ext.security_reviewed,
        "securityNotes": ext.security_notes,
        "reviewedBy": ext.reviewed_by,
        "reviewedAt": ext.reviewed_at.map(|dt| dt.to_rfc3339()),
        "useCount": ext.use_count,
        "aiDescription": ext.ai_description,
        "aiDescriptionLang": ext.ai_description_lang,
        "aiDescribedAt": ext.ai_described_at.map(|dt| dt.to_rfc3339()),
        "createdAt": ext.created_at.to_rfc3339(),
        "updatedAt": ext.updated_at.to_rfc3339()
    }))
}

async fn list_extensions(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Query(query): Query<ExtensionQuery>,
) -> Result<Json<ExtensionsResponse>, (StatusCode, String)> {
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
            "Only team members can view extensions".to_string(),
        ));
    }

    let service = ExtensionService::new((*state.db).clone());
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

    let extensions: Vec<ExtensionInfo> = result
        .items
        .into_iter()
        .map(|e| ExtensionInfo {
            id: e.id,
            team_id: e.team_id,
            name: e.name,
            description: e.description,
            extension_type: e.extension_type,
            config: e.config,
            author_id: e.author_id,
            version: e.version,
            visibility: e.visibility,
            protection_level: e.protection_level,
            tags: e.tags,
            security_reviewed: e.security_reviewed,
            use_count: e.use_count,
            ai_description: e.ai_description,
            ai_description_lang: e.ai_description_lang,
            ai_described_at: e.ai_described_at.map(|dt| dt.to_rfc3339()),
            created_at: e.created_at.to_rfc3339(),
            updated_at: e.updated_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(ExtensionsResponse {
        extensions,
        total: result.total,
        page: result.page,
        limit: result.limit,
        total_pages: result.total_pages,
    }))
}

async fn create_extension(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Json(req): Json<CreateExtensionRequest>,
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
            "Only team members can create extensions".to_string(),
        ));
    }

    let service = ExtensionService::new((*state.db).clone());

    // Convert serde_json::Value to bson::Document
    let config_bson = mongodb::bson::to_document(&req.config)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid config: {}", e)))?;

    let ext = service
        .create(
            &req.team_id,
            &user.0,
            &req.name,
            &req.extension_type,
            config_bson,
            req.description.clone(),
            req.tags,
            req.visibility,
        )
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Smart Log trigger
    if let Some(trigger) = &state.smart_log_trigger {
        trigger.trigger(SmartLogContext::new(
            req.team_id.clone(),
            user.0.clone(),
            "create",
            "extension",
            ext.id.map(|id| id.to_hex()).unwrap_or_default(),
            ext.name.clone(),
            Some(format!(
                "名称: {}\n类型: {}\n描述: {}\n配置: {}",
                ext.name,
                req.extension_type,
                req.description.unwrap_or_default(),
                serde_json::to_string(&req.config).unwrap_or_default()
            )),
        ));
    }

    let body = extension_to_json(ext).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(body))
}

async fn delete_extension(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let service = ExtensionService::new((*state.db).clone());

    // Get extension to find team_id
    let ext = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Extension not found".to_string()))?;

    // Check team permission
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&ext.team_id.to_hex())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Only admin/owner can delete extensions
    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team admin or owner can delete extensions".to_string(),
        ));
    }

    // Smart Log trigger (before delete)
    if let Some(trigger) = &state.smart_log_trigger {
        trigger.trigger(SmartLogContext::new(
            ext.team_id.to_hex(),
            user.0.clone(),
            "delete",
            "extension",
            ext.id.map(|id| id.to_hex()).unwrap_or_default(),
            ext.name.clone(),
            None,
        ));
    }

    service
        .delete(&id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn get_extension(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let service = ExtensionService::new((*state.db).clone());
    let ext = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Extension not found".to_string()))?;

    // Check team membership
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&ext.team_id.to_hex())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view extensions".to_string(),
        ));
    }

    let body = extension_to_json(ext).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(body))
}

async fn update_extension(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
    Json(req): Json<UpdateExtensionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let service = ExtensionService::new((*state.db).clone());

    // Get extension to find team_id
    let ext = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Extension not found".to_string()))?;

    // Check team permission
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&ext.team_id.to_hex())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Only admin/owner can update extensions
    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team admin or owner can update extensions".to_string(),
        ));
    }

    // Convert config if provided
    let config_bson = match req.config {
        Some(ref v) => Some(
            mongodb::bson::to_document(v)
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid config: {}", e)))?,
        ),
        None => None,
    };

    let ext = service
        .update(&id, req.name, req.description, config_bson)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Smart Log trigger
    if let Some(trigger) = &state.smart_log_trigger {
        trigger.trigger(SmartLogContext::new(
            ext.team_id.to_hex(),
            user.0.clone(),
            "update",
            "extension",
            ext.id.map(|id| id.to_hex()).unwrap_or_default(),
            ext.name.clone(),
            Some(format!(
                "名称: {}\n配置: {}",
                ext.name,
                serde_json::to_string(&ext.config).unwrap_or_default()
            )),
        ));
    }

    let body = extension_to_json(ext).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(body))
}

/// Install an extension (cloud server returns success)
async fn install_extension(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
) -> Result<Json<InstallResponse>, (StatusCode, String)> {
    let service = ExtensionService::new((*state.db).clone());

    // Verify extension exists
    let ext = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Extension not found".to_string()))?;

    // Check team membership
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&ext.team_id.to_hex())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can install extensions".to_string(),
        ));
    }

    // Increment use count
    let _ = service.increment_use_count(&id).await;

    Ok(Json(InstallResponse {
        success: true,
        resource_type: "extension".to_string(),
        resource_id: ext.id.map(|id| id.to_hex()).unwrap_or_default(),
        local_path: None,
        error: None,
    }))
}

/// Uninstall an extension
async fn uninstall_extension(Path(_id): Path<String>) -> Result<StatusCode, (StatusCode, String)> {
    // Cloud server just acknowledges the uninstall request
    // Actual uninstall happens on client side
    Ok(StatusCode::NO_CONTENT)
}

/// Review an extension (approve/reject security review)
async fn review_extension(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
    Json(req): Json<ReviewExtensionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let service = ExtensionService::new((*state.db).clone());

    let ext = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Extension not found".to_string()))?;

    // Only admin/owner can review
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&ext.team_id.to_hex())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only admin or owner can review extensions".to_string(),
        ));
    }

    let ext = service
        .review_extension(&id, req.approved, req.notes, &user.0)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let body = extension_to_json(ext).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(body))
}
