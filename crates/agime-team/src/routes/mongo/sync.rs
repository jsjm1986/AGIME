//! MongoDB routes - Sync API
//! Provides endpoints for client-side sync operations

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::teams::{is_team_member, AppState};
use crate::db::collections;
use crate::services::mongo::TeamService;
use crate::AuthenticatedUserId;

#[derive(Debug, Deserialize)]
pub struct CheckUpdatesQuery {
    /// ISO 8601 timestamp - return resources updated after this time
    pub since: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SyncStatus {
    pub skills: u64,
    pub recipes: u64,
    pub extensions: u64,
    #[serde(rename = "lastSync")]
    pub last_sync: Option<String>,
}

pub fn sync_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/teams/{team_id}/sync/status", get(sync_status))
        .route("/teams/{team_id}/sync/check", get(check_updates))
        // Compatibility endpoints used by local/legacy clients
        .route("/resources/installed", get(list_installed))
        .route("/resources/check-updates", post(check_updates_compat))
        .route("/resources/batch-install", post(batch_install_compat))
}

#[derive(Debug, Deserialize, Default)]
struct CheckUpdatesBody {
    #[serde(default, alias = "resourceIds", alias = "resource_ids")]
    resource_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ResourceRefInput {
    #[serde(default, alias = "resourceType", alias = "type")]
    resource_type: String,
    id: String,
}

#[derive(Debug, Deserialize, Default)]
struct BatchInstallBody {
    #[serde(default, alias = "resources")]
    resources: Vec<ResourceRefInput>,
    #[serde(default, alias = "resourceIds", alias = "resource_ids")]
    resource_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstalledResourceCompat {
    id: String,
    resource_type: String,
    resource_id: String,
    team_id: String,
    resource_name: String,
    local_path: Option<String>,
    installed_version: String,
    latest_version: Option<String>,
    has_update: bool,
    installed_at: String,
    last_checked_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ListInstalledCompatResponse {
    resources: Vec<InstalledResourceCompat>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateInfoCompat {
    resource_id: String,
    resource_type: String,
    resource_name: String,
    current_version: String,
    latest_version: String,
    has_update: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckUpdatesCompatResponse {
    updates: Vec<UpdateInfoCompat>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallResultCompat {
    success: bool,
    resource_type: String,
    resource_id: String,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchInstallCompatResponse {
    total: usize,
    successful: usize,
    failed: usize,
    results: Vec<InstallResultCompat>,
}

async fn sync_status(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
) -> Result<Json<SyncStatus>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view sync status".to_string(),
        ));
    }

    let not_deleted = mongodb::bson::doc! {
        "team_id": mongodb::bson::oid::ObjectId::parse_str(&team_id)
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?,
        "is_deleted": { "$ne": true }
    };

    let skills = state
        .db
        .collection::<mongodb::bson::Document>(collections::SKILLS)
        .count_documents(not_deleted.clone(), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let recipes = state
        .db
        .collection::<mongodb::bson::Document>(collections::RECIPES)
        .count_documents(not_deleted.clone(), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let extensions = state
        .db
        .collection::<mongodb::bson::Document>(collections::EXTENSIONS)
        .count_documents(not_deleted.clone(), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(SyncStatus {
        skills,
        recipes,
        extensions,
        last_sync: None,
    }))
}

async fn check_updates(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<CheckUpdatesQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can check updates".to_string(),
        ));
    }

    // Parse since timestamp
    let since_filter = q.since.as_deref().and_then(|since| {
        chrono::DateTime::parse_from_rfc3339(since)
            .ok()
            .map(|dt| bson::DateTime::from_chrono(dt.with_timezone(&chrono::Utc)))
    });

    let team_oid = mongodb::bson::oid::ObjectId::parse_str(&team_id)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let mut filter = mongodb::bson::doc! {
        "team_id": &team_oid,
        "is_deleted": { "$ne": true }
    };
    if let Some(since_dt) = since_filter {
        filter.insert("updated_at", mongodb::bson::doc! { "$gt": since_dt });
    }

    let skills_count = state
        .db
        .collection::<mongodb::bson::Document>(collections::SKILLS)
        .count_documents(filter.clone(), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let recipes_count = state
        .db
        .collection::<mongodb::bson::Document>(collections::RECIPES)
        .count_documents(filter.clone(), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let extensions_count = state
        .db
        .collection::<mongodb::bson::Document>(collections::EXTENSIONS)
        .count_documents(filter.clone(), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let has_updates = skills_count > 0 || recipes_count > 0 || extensions_count > 0;

    Ok(Json(serde_json::json!({
        "hasUpdates": has_updates,
        "updatedSkills": skills_count,
        "updatedRecipes": recipes_count,
        "updatedExtensions": extensions_count,
        "checkedAt": chrono::Utc::now().to_rfc3339(),
    })))
}

async fn list_installed() -> Result<Json<ListInstalledCompatResponse>, (StatusCode, String)> {
    // Mongo cloud server does not maintain local installed_resources state.
    Ok(Json(ListInstalledCompatResponse { resources: vec![] }))
}

async fn check_updates_compat(
    Json(req): Json<CheckUpdatesBody>,
) -> Result<Json<CheckUpdatesCompatResponse>, (StatusCode, String)> {
    // Compatibility mode: cloud server has no local install registry, so no per-install updates.
    let updates = req
        .resource_ids
        .into_iter()
        .map(|id| UpdateInfoCompat {
            resource_id: id,
            resource_type: "unknown".to_string(),
            resource_name: String::new(),
            current_version: String::new(),
            latest_version: String::new(),
            has_update: false,
        })
        .collect();

    Ok(Json(CheckUpdatesCompatResponse { updates }))
}

async fn batch_install_compat(
    Json(req): Json<BatchInstallBody>,
) -> Result<Json<BatchInstallCompatResponse>, (StatusCode, String)> {
    let mut results = Vec::new();
    for item in req.resources {
        results.push(InstallResultCompat {
            success: true,
            resource_type: if item.resource_type.is_empty() {
                "unknown".to_string()
            } else {
                item.resource_type
            },
            resource_id: item.id,
            error: None,
        });
    }

    for id in req.resource_ids {
        results.push(InstallResultCompat {
            success: true,
            resource_type: "unknown".to_string(),
            resource_id: id,
            error: None,
        });
    }

    let total = results.len();
    Ok(Json(BatchInstallCompatResponse {
        total,
        successful: total,
        failed: 0,
        results,
    }))
}
