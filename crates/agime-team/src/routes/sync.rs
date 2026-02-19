//! Sync HTTP routes

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};

use super::get_user_id;
use crate::error::TeamError;
use crate::models::{
    BatchInstallRequest, CheckUpdatesRequest, ResourceRef as ModelResourceRef, ResourceType,
};
use crate::routes::skills::InstallResponse;
use crate::routes::teams::TeamState;
use crate::services::InstallService;
use crate::sync::GitSync;
use crate::AuthenticatedUserId;

/// Check updates request (API)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckUpdatesApiRequest {
    pub resource_ids: Vec<String>,
}

/// Batch install request (API)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchInstallApiRequest {
    #[serde(default)]
    pub resources: Vec<ResourceRefApi>,
    #[serde(default, alias = "resourceIds", alias = "resource_ids")]
    pub resource_ids: Vec<String>,
}

/// Resource reference for batch operations (API)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRefApi {
    pub resource_type: String,
    pub id: String,
}

/// Update info response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfoResponse {
    pub resource_id: String,
    pub resource_type: String,
    pub resource_name: String,
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
}

/// Check updates response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckUpdatesApiResponse {
    pub updates: Vec<UpdateInfoResponse>,
}

/// Batch install response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchInstallApiResponse {
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
    pub results: Vec<InstallResponse>,
}

/// Installed resource response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledResourceResponse {
    pub id: String,
    pub resource_type: String,
    pub resource_id: String,
    pub team_id: String,
    pub resource_name: String,
    pub local_path: Option<String>,
    pub installed_version: String,
    pub latest_version: Option<String>,
    pub has_update: bool,
    pub installed_at: String,
    pub last_checked_at: Option<String>,
}

/// List installed response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListInstalledResponse {
    pub resources: Vec<InstalledResourceResponse>,
}

/// Sync status response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatusResponse {
    pub team_id: String,
    pub state: String,
    pub last_sync_at: Option<String>,
    pub last_commit_hash: Option<String>,
    pub error_message: Option<String>,
}

/// Configure sync routes
pub fn routes(state: TeamState) -> Router {
    Router::new()
        .route("/teams/{team_id}/sync", post(trigger_sync))
        .route("/teams/{team_id}/sync/status", get(sync_status))
        .route("/resources/check-updates", post(check_updates))
        .route("/resources/batch-install", post(batch_install))
        .route("/resources/installed", get(list_installed))
        .with_state(state)
}

/// Trigger sync for a team
async fn trigger_sync(
    State(_state): State<TeamState>,
    Path(team_id): Path<String>,
) -> Result<Json<SyncStatusResponse>, TeamError> {
    tracing::info!("Sync requested for team: {}", team_id);

    let git_sync = GitSync::new();

    // Initialize repo if not exists (async to avoid blocking)
    git_sync.init_repo_async(&team_id, None).await?;

    // Pull latest changes (async to avoid blocking)
    let status = git_sync.pull_async(&team_id).await?;

    Ok(Json(SyncStatusResponse {
        team_id: status.team_id,
        state: status.state.to_string(),
        last_sync_at: status.last_sync_at.map(|dt| dt.to_rfc3339()),
        last_commit_hash: status.last_commit_hash,
        error_message: status.error_message,
    }))
}

/// Get sync status for a team
async fn sync_status(
    State(_state): State<TeamState>,
    Path(team_id): Path<String>,
) -> Result<Json<SyncStatusResponse>, TeamError> {
    let git_sync = GitSync::new();

    // Use async version to avoid blocking
    let status = git_sync.get_status_async(&team_id).await?;

    Ok(Json(SyncStatusResponse {
        team_id: status.team_id,
        state: status.state.to_string(),
        last_sync_at: status.last_sync_at.map(|dt| dt.to_rfc3339()),
        last_commit_hash: status.last_commit_hash,
        error_message: status.error_message,
    }))
}

/// Check for updates to installed resources
async fn check_updates(
    State(state): State<TeamState>,
    Json(req): Json<CheckUpdatesApiRequest>,
) -> Result<Json<CheckUpdatesApiResponse>, TeamError> {
    let service = InstallService::new();

    let request = CheckUpdatesRequest {
        resource_ids: req.resource_ids,
    };

    let response = service.check_updates(&state.pool, request).await?;

    let api_response = CheckUpdatesApiResponse {
        updates: response
            .updates
            .into_iter()
            .map(|u| UpdateInfoResponse {
                resource_id: u.resource_id,
                resource_type: u.resource_type.to_string(),
                resource_name: u.resource_name,
                current_version: u.current_version,
                latest_version: u.latest_version,
                has_update: u.has_update,
            })
            .collect(),
    };

    Ok(Json(api_response))
}

/// Batch install multiple resources
async fn batch_install(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Json(req): Json<BatchInstallApiRequest>,
) -> Result<Json<BatchInstallApiResponse>, TeamError> {
    let service = InstallService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    let mut resources: Vec<ModelResourceRef> = req
        .resources
        .into_iter()
        .map(|r| ModelResourceRef {
            resource_type: r
                .resource_type
                .parse()
                .unwrap_or(crate::models::ResourceType::Skill),
            id: r.id,
        })
        .collect();

    // Backward compatibility: accept `resourceIds` payload and infer type from installed records.
    if resources.is_empty() && !req.resource_ids.is_empty() {
        for rid in req.resource_ids {
            let inferred_type = sqlx::query_scalar::<_, String>(
                "SELECT resource_type FROM installed_resources WHERE resource_id = ? LIMIT 1",
            )
            .bind(&rid)
            .fetch_optional(&*state.pool)
            .await?
            .and_then(|s| s.parse::<ResourceType>().ok())
            .unwrap_or(ResourceType::Skill);
            resources.push(ModelResourceRef {
                resource_type: inferred_type,
                id: rid,
            });
        }
    }

    let request = BatchInstallRequest { resources };

    let result = service
        .batch_install(&state.pool, request, &user_id, &state.base_path)
        .await?;

    let api_response = BatchInstallApiResponse {
        total: result.total,
        successful: result.successful,
        failed: result.failed,
        results: result
            .results
            .into_iter()
            .map(|r| InstallResponse {
                success: r.success,
                resource_type: r.resource_type.to_string(),
                resource_id: r.resource_id,
                installed_version: Some(r.installed_version),
                local_path: r.local_path,
                error: r.error,
            })
            .collect(),
    };

    Ok(Json(api_response))
}

/// List all installed resources
async fn list_installed(
    State(state): State<TeamState>,
) -> Result<Json<ListInstalledResponse>, TeamError> {
    let service = InstallService::new();

    let installed = service.list_installed(&state.pool).await?;

    let response = ListInstalledResponse {
        resources: installed
            .into_iter()
            .map(|r| InstalledResourceResponse {
                id: r.id,
                resource_type: r.resource_type.to_string(),
                resource_id: r.resource_id,
                team_id: r.team_id,
                resource_name: r.resource_name,
                local_path: r.local_path,
                installed_version: r.installed_version,
                latest_version: r.latest_version,
                has_update: r.has_update,
                installed_at: r.installed_at.to_rfc3339(),
                last_checked_at: r.last_checked_at.map(|dt| dt.to_rfc3339()),
            })
            .collect(),
    };

    Ok(Json(response))
}
