//! MongoDB routes - Folders API

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::teams::{can_manage_team, is_team_member, AppState};
use crate::services::mongo::{FolderService, TeamService};
use crate::AuthenticatedUserId;

#[derive(Debug, Deserialize)]
pub struct FolderQuery {
    pub parent_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFolderRequest {
    pub name: String,
    pub parent_path: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFolderRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FolderInfo {
    pub id: String,
    pub name: String,
    #[serde(rename = "parentPath")]
    pub parent_path: String,
    #[serde(rename = "fullPath")]
    pub full_path: String,
    pub description: Option<String>,
    #[serde(rename = "createdBy")]
    pub created_by: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct FolderTreeNodeInfo {
    pub id: String,
    pub name: String,
    #[serde(rename = "fullPath")]
    pub full_path: String,
    pub children: Vec<FolderTreeNodeInfo>,
}

pub fn folder_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/teams/{team_id}/folders",
            get(list_folders).post(create_folder),
        )
        .route("/teams/{team_id}/folders/tree", get(get_folder_tree))
        .route(
            "/teams/{team_id}/folders/{folder_id}",
            get(get_folder).put(update_folder).delete(delete_folder),
        )
}

async fn list_folders(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<FolderQuery>,
) -> Result<Json<Vec<FolderInfo>>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view folders".to_string(),
        ));
    }

    let service = FolderService::new((*state.db).clone());
    let folders = service
        .list(&team_id, q.parent_path.as_deref())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<FolderInfo> = folders
        .into_iter()
        .map(|f| FolderInfo {
            id: f.id,
            name: f.name,
            parent_path: f.parent_path,
            full_path: f.full_path,
            description: f.description,
            created_by: f.created_by,
            created_at: f.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(items))
}

async fn create_folder(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Json(req): Json<CreateFolderRequest>,
) -> Result<Json<FolderInfo>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can create folders".to_string(),
        ));
    }

    let parent_path = req.parent_path.as_deref().unwrap_or("/");
    let service = FolderService::new((*state.db).clone());
    let folder = service
        .create(&team_id, &user.0, &req.name, parent_path)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(Json(FolderInfo {
        id: folder.id.map(|id| id.to_hex()).unwrap_or_default(),
        name: folder.name,
        parent_path: folder.parent_path,
        full_path: folder.full_path,
        description: folder.description,
        created_by: folder.created_by,
        created_at: folder.created_at.to_rfc3339(),
    }))
}

async fn get_folder_tree(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
) -> Result<Json<Vec<FolderTreeNodeInfo>>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view folders".to_string(),
        ));
    }

    let service = FolderService::new((*state.db).clone());
    let tree = service
        .get_folder_tree(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    fn convert_tree(nodes: Vec<crate::models::mongo::FolderTreeNode>) -> Vec<FolderTreeNodeInfo> {
        nodes
            .into_iter()
            .map(|n| FolderTreeNodeInfo {
                id: n.id,
                name: n.name,
                full_path: n.full_path,
                children: convert_tree(n.children),
            })
            .collect()
    }

    Ok(Json(convert_tree(tree)))
}

async fn get_folder(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, folder_id)): Path<(String, String)>,
) -> Result<Json<FolderInfo>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view folders".to_string(),
        ));
    }

    let service = FolderService::new((*state.db).clone());
    let folder = service
        .get(&folder_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Folder not found".to_string()))?;

    // Verify the folder belongs to the requested team
    if folder.team_id.to_hex() != team_id {
        return Err((StatusCode::NOT_FOUND, "Folder not found".to_string()));
    }

    Ok(Json(FolderInfo {
        id: folder.id.map(|id| id.to_hex()).unwrap_or_default(),
        name: folder.name,
        parent_path: folder.parent_path,
        full_path: folder.full_path,
        description: folder.description,
        created_by: folder.created_by,
        created_at: folder.created_at.to_rfc3339(),
    }))
}

async fn update_folder(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, folder_id)): Path<(String, String)>,
    Json(req): Json<UpdateFolderRequest>,
) -> Result<Json<FolderInfo>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only admin or owner can update folders".to_string(),
        ));
    }

    let service = FolderService::new((*state.db).clone());
    let folder = service
        .update(&folder_id, req.name, req.description)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(Json(FolderInfo {
        id: folder.id.map(|id| id.to_hex()).unwrap_or_default(),
        name: folder.name,
        parent_path: folder.parent_path,
        full_path: folder.full_path,
        description: folder.description,
        created_by: folder.created_by,
        created_at: folder.created_at.to_rfc3339(),
    }))
}

async fn delete_folder(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, folder_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only admin or owner can delete folders".to_string(),
        ));
    }

    let service = FolderService::new((*state.db).clone());
    service
        .delete(&folder_id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}
