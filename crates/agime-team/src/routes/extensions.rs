//! Extensions HTTP routes

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};

use super::get_user_id;
use crate::error::TeamError;
use crate::models::{
    ExtensionConfig, ExtensionType, ListExtensionsQuery, ProtectionLevel, ResourceType,
    ReviewExtensionRequest, ShareExtensionRequest, SharedExtension, UpdateExtensionRequest,
};
use crate::routes::skills::{InstallResponse, LocalInstallAuthorizationRequest};
use crate::routes::teams::TeamState;
use crate::services::{ExtensionService, InstallService};
use crate::AuthenticatedUserId;

/// Query params for listing extensions
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListExtensionsParams {
    pub team_id: Option<String>,
    pub search: Option<String>,
    pub extension_type: Option<String>,
    pub author_id: Option<String>,
    pub tags: Option<String>,
    pub reviewed_only: Option<bool>,
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
}

/// Share extension request (API)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareExtensionApiRequest {
    pub team_id: String,
    pub name: String,
    pub extension_type: String,
    pub config: ExtensionConfig,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub visibility: Option<String>,
    pub protection_level: Option<String>,
}

/// Update extension request (API)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateExtensionApiRequest {
    pub name: Option<String>,
    pub config: Option<ExtensionConfig>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub visibility: Option<String>,
    pub protection_level: Option<String>,
}

/// Security review request (API)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewExtensionApiRequest {
    pub approved: bool,
    pub notes: Option<String>,
}

/// Local install request for extensions
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalExtensionInstallRequest {
    pub resource_id: String,
    pub team_id: String,
    pub name: String,
    pub extension_type: String,
    pub config: ExtensionConfig,
    pub version: String,
    pub protection_level: Option<String>,
    pub authorization: Option<LocalInstallAuthorizationRequest>,
}

/// Extension response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionResponse {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    pub extension_type: String,
    pub config: ExtensionConfig,
    pub author_id: String,
    pub version: String,
    pub visibility: String,
    pub protection_level: String,
    pub tags: Vec<String>,
    pub security_reviewed: bool,
    pub security_notes: Option<String>,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<String>,
    pub use_count: u32,
    pub created_at: String,
    pub updated_at: String,
}

/// Paginated extensions response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionsListResponse {
    pub extensions: Vec<ExtensionResponse>,
    pub total: u64,
    pub page: u32,
    pub limit: u32,
}

impl From<SharedExtension> for ExtensionResponse {
    fn from(ext: SharedExtension) -> Self {
        Self {
            id: ext.id,
            team_id: ext.team_id,
            name: ext.name,
            description: ext.description,
            extension_type: ext.extension_type.to_string(),
            config: ext.config,
            author_id: ext.author_id,
            version: ext.version,
            visibility: ext.visibility.to_string(),
            protection_level: ext.protection_level.to_string(),
            tags: ext.tags,
            security_reviewed: ext.security_reviewed,
            security_notes: ext.security_notes,
            reviewed_by: ext.reviewed_by,
            reviewed_at: ext.reviewed_at.map(|dt| dt.to_rfc3339()),
            use_count: ext.use_count,
            created_at: ext.created_at.to_rfc3339(),
            updated_at: ext.updated_at.to_rfc3339(),
        }
    }
}

/// Configure extensions routes
pub fn routes(state: TeamState) -> Router {
    Router::new()
        .route("/extensions", post(share_extension).get(list_extensions))
        .route("/extensions/install-local", post(install_extension_local))
        .route(
            "/extensions/{id}",
            get(get_extension)
                .put(update_extension)
                .delete(delete_extension),
        )
        .route("/extensions/{id}/install", post(install_extension))
        .route("/extensions/{id}/uninstall", delete(uninstall_extension))
        .route("/extensions/{id}/review", post(review_extension))
        .with_state(state)
}

/// Share an extension to a team
async fn share_extension(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Json(req): Json<ShareExtensionApiRequest>,
) -> Result<(StatusCode, Json<ExtensionResponse>), TeamError> {
    let service = ExtensionService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    let extension_type = req.extension_type.parse().unwrap_or(ExtensionType::Stdio);
    let visibility = req.visibility.and_then(|v| v.parse().ok());

    let request = ShareExtensionRequest {
        team_id: req.team_id,
        name: req.name,
        extension_type,
        config: req.config,
        description: req.description,
        tags: req.tags,
        visibility,
        protection_level: req.protection_level.and_then(|p| p.parse().ok()),
    };

    let extension = service
        .share_extension(&state.pool, request, &user_id)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(ExtensionResponse::from(extension)),
    ))
}

/// List extensions
async fn list_extensions(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Query(params): Query<ListExtensionsParams>,
) -> Result<Json<ExtensionsListResponse>, TeamError> {
    let service = ExtensionService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    let query = ListExtensionsQuery {
        team_id: params.team_id,
        search: params.search,
        extension_type: params.extension_type.and_then(|t| t.parse().ok()),
        author_id: params.author_id,
        tags: params
            .tags
            .map(|t| t.split(',').map(|s| s.trim().to_string()).collect()),
        reviewed_only: params.reviewed_only,
        page: params.page.unwrap_or(1),
        limit: params.limit.unwrap_or(20).min(100),
        sort: params.sort.unwrap_or_else(|| "updated_at".to_string()),
    };

    let result = service
        .list_extensions(&state.pool, query, &user_id)
        .await?;

    let response = ExtensionsListResponse {
        extensions: result
            .items
            .into_iter()
            .map(ExtensionResponse::from)
            .collect(),
        total: result.total,
        page: result.page,
        limit: result.limit,
    };

    Ok(Json(response))
}

/// Get an extension by ID
async fn get_extension(
    State(state): State<TeamState>,
    Path(extension_id): Path<String>,
) -> Result<Json<ExtensionResponse>, TeamError> {
    let service = ExtensionService::new();

    let extension = service.get_extension(&state.pool, &extension_id).await?;

    Ok(Json(ExtensionResponse::from(extension)))
}

/// Update an extension
async fn update_extension(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(extension_id): Path<String>,
    Json(req): Json<UpdateExtensionApiRequest>,
) -> Result<Json<ExtensionResponse>, TeamError> {
    let service = ExtensionService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    let request = UpdateExtensionRequest {
        name: req.name,
        config: req.config,
        description: req.description,
        tags: req.tags,
        visibility: req.visibility.and_then(|v| v.parse().ok()),
        protection_level: req.protection_level.and_then(|p| p.parse().ok()),
    };

    let extension = service
        .update_extension(&state.pool, &extension_id, request, &user_id)
        .await?;

    Ok(Json(ExtensionResponse::from(extension)))
}

/// Delete an extension
async fn delete_extension(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(extension_id): Path<String>,
) -> Result<StatusCode, TeamError> {
    let service = ExtensionService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    service
        .delete_extension(&state.pool, &extension_id, &user_id)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Install an extension
async fn install_extension(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(extension_id): Path<String>,
) -> Result<Json<InstallResponse>, TeamError> {
    let service = InstallService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    let result = service
        .install_resource(
            &state.pool,
            ResourceType::Extension,
            &extension_id,
            &user_id,
            &state.base_path,
        )
        .await?;

    Ok(Json(InstallResponse {
        success: result.success,
        resource_type: result.resource_type.to_string(),
        resource_id: result.resource_id,
        installed_version: Some(result.installed_version),
        local_path: result.local_path,
        error: result.error,
    }))
}

/// Install an extension locally from cloud-fetched content
async fn install_extension_local(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Json(req): Json<LocalExtensionInstallRequest>,
) -> Result<Json<InstallResponse>, TeamError> {
    use crate::security::validate_resource_name;

    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    // Validate resource name
    validate_resource_name(&req.name)?;

    let protection_level = req
        .protection_level
        .as_deref()
        .and_then(|p| p.parse().ok())
        .unwrap_or(ProtectionLevel::TeamInstallable);

    if !protection_level.allows_local_install() {
        return Err(TeamError::Validation(format!(
            "Extension with protection level '{}' cannot be installed locally",
            protection_level
        )));
    }

    // Determine local path
    let local_path = state.base_path.join("extensions").join(&req.name);

    // Create directory
    std::fs::create_dir_all(&local_path)
        .map_err(|e| TeamError::Internal(format!("Failed to create directory: {}", e)))?;

    // Write extension.json
    let config_json = serde_json::to_string_pretty(&req.config)
        .map_err(|e| TeamError::Internal(format!("Failed to serialize config: {}", e)))?;

    let file_path = local_path.join("extension.json");
    std::fs::write(&file_path, &config_json)
        .map_err(|e| TeamError::Internal(format!("Failed to write extension.json: {}", e)))?;

    let now = chrono::Utc::now();
    let installed_at = now.to_rfc3339();

    let authorization = super::resolve_authorization(
        &protection_level,
        req.authorization.as_ref(),
        &req.team_id,
        &req.resource_id,
        &user_id,
        &installed_at,
        "extensions",
    )?;

    // Write metadata file
    let meta = serde_json::json!({
        "source": "team",
        "teamId": req.team_id,
        "resourceId": req.resource_id,
        "userId": user_id,
        "extensionType": req.extension_type,
        "installedAt": installed_at,
        "installedVersion": req.version,
        "protectionLevel": protection_level.to_string(),
        "authorization": super::build_auth_meta_json(authorization.as_ref()),
    });

    let meta_path = local_path.join(".extension-meta.json");
    std::fs::write(&meta_path, serde_json::to_string_pretty(&meta).unwrap())
        .map_err(|e| TeamError::Internal(format!("Failed to write metadata: {}", e)))?;

    let local_path_str = local_path.to_string_lossy().to_string();
    super::record_local_install(
        &state.pool,
        ResourceType::Extension,
        &req.resource_id,
        &req.team_id,
        &req.name,
        &local_path_str,
        &req.version,
        &installed_at,
        &user_id,
        authorization.as_ref(),
        &protection_level,
    )
    .await?;

    Ok(Json(InstallResponse {
        success: true,
        resource_type: "extension".to_string(),
        resource_id: req.resource_id,
        installed_version: Some(req.version),
        local_path: Some(local_path.to_string_lossy().to_string()),
        error: None,
    }))
}

/// Uninstall an extension
async fn uninstall_extension(
    State(state): State<TeamState>,
    Path(extension_id): Path<String>,
) -> Result<StatusCode, TeamError> {
    let service = InstallService::new();

    let result = service
        .uninstall_resource(&state.pool, ResourceType::Extension, &extension_id)
        .await?;

    if result.success {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(TeamError::Internal(
            result
                .error
                .unwrap_or_else(|| "Uninstall failed".to_string()),
        ))
    }
}

/// Review an extension for security
async fn review_extension(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(extension_id): Path<String>,
    Json(req): Json<ReviewExtensionApiRequest>,
) -> Result<Json<ExtensionResponse>, TeamError> {
    let service = ExtensionService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    let request = ReviewExtensionRequest {
        approved: req.approved,
        notes: req.notes,
    };

    let extension = service
        .review_extension(&state.pool, &extension_id, request, &user_id)
        .await?;

    Ok(Json(ExtensionResponse::from(extension)))
}
