//! Document management routes

use axum::{
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::TeamState;
use crate::models::{
    CreateFolderRequest, DocumentSearchQuery, DocumentSearchResult, DocumentSummary,
    FolderTreeNode, TeamDocument, TeamFolder,
};
use crate::services::DocumentService;

/// Create document routes with TeamState
pub fn routes(state: TeamState) -> Router {
    Router::new()
        // Folder routes
        .route("/teams/{team_id}/folders", post(create_folder))
        .route("/teams/{team_id}/folders", get(get_folder_tree))
        .route(
            "/teams/{team_id}/folders/{folder_id}",
            delete(delete_folder),
        )
        // Document routes
        .route("/teams/{team_id}/documents", post(upload_document))
        .route("/teams/{team_id}/documents", get(list_documents))
        .route("/teams/{team_id}/documents/search", get(search_documents))
        .route("/teams/{team_id}/documents/{doc_id}", get(get_document))
        .route(
            "/teams/{team_id}/documents/{doc_id}",
            delete(delete_document),
        )
        .route(
            "/teams/{team_id}/documents/{doc_id}/download",
            get(download_document),
        )
        .with_state(state)
}

// ========================================
// Folder Handlers
// ========================================

async fn create_folder(
    State(state): State<TeamState>,
    Path(team_id): Path<String>,
    Json(req): Json<CreateFolderRequest>,
) -> Result<Json<TeamFolder>, (StatusCode, String)> {
    let user_id = &state.user_id;
    let service = DocumentService::new(state.base_path.clone());

    service
        .create_folder(&state.pool, &team_id, user_id, req)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn get_folder_tree(
    State(state): State<TeamState>,
    Path(team_id): Path<String>,
) -> Result<Json<Vec<FolderTreeNode>>, (StatusCode, String)> {
    let service = DocumentService::new(state.base_path.clone());

    service
        .get_folder_tree(&state.pool, &team_id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn delete_folder(
    State(state): State<TeamState>,
    Path((_team_id, folder_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    // TODO: SEC - Verify folder belongs to team_id before deleting
    let service = DocumentService::new(state.base_path.clone());

    service
        .delete_folder(&state.pool, &folder_id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

// ========================================
// Document Handlers
// ========================================

#[derive(Deserialize)]
struct ListDocumentsQuery {
    folder_id: Option<String>,
    page: Option<u32>,
    limit: Option<u32>,
}

#[derive(Serialize)]
struct ListDocumentsResponse {
    items: Vec<DocumentSummary>,
    total: i64,
    page: u32,
    limit: u32,
}

async fn list_documents(
    State(state): State<TeamState>,
    Path(team_id): Path<String>,
    Query(query): Query<ListDocumentsQuery>,
) -> Result<Json<ListDocumentsResponse>, (StatusCode, String)> {
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(20);
    let service = DocumentService::new(state.base_path.clone());

    let (items, total) = service
        .list_documents(
            &state.pool,
            &team_id,
            query.folder_id.as_deref(),
            page,
            limit,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ListDocumentsResponse {
        items,
        total,
        page,
        limit,
    }))
}

async fn upload_document(
    State(state): State<TeamState>,
    Path(team_id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<TeamDocument>, (StatusCode, String)> {
    let user_id = state.user_id.clone();
    let service = DocumentService::new(state.base_path.clone());

    let mut file_name = String::new();
    let mut file_data = Vec::new();
    let mut mime_type = String::from("application/octet-stream");
    let mut folder_id: Option<String> = None;
    let mut description: Option<String> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Failed to read multipart: {}", e),
        )
    })? {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "file" => {
                file_name = field.file_name().unwrap_or("unknown").to_string();
                if let Some(ct) = field.content_type() {
                    mime_type = ct.to_string();
                }
                file_data = field
                    .bytes()
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::BAD_REQUEST,
                            format!("Failed to read file: {}", e),
                        )
                    })?
                    .to_vec();
            }
            "folder_id" => {
                let text = field.text().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("Failed to read folder_id: {}", e),
                    )
                })?;
                if !text.is_empty() {
                    folder_id = Some(text);
                }
            }
            "description" => {
                let text = field.text().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("Failed to read description: {}", e),
                    )
                })?;
                if !text.is_empty() {
                    description = Some(text);
                }
            }
            _ => {}
        }
    }

    if file_data.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "No file provided".to_string()));
    }

    service
        .upload_document(
            &state.pool,
            &team_id,
            &user_id,
            &file_name,
            file_data,
            &mime_type,
            folder_id,
            description,
        )
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn get_document(
    State(state): State<TeamState>,
    Path((_team_id, doc_id)): Path<(String, String)>,
) -> Result<Json<TeamDocument>, (StatusCode, String)> {
    // TODO: SEC - Verify document belongs to team_id before returning
    let service = DocumentService::new(state.base_path.clone());

    service
        .get_document(&state.pool, &doc_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Document not found".to_string()))
        .map(Json)
}

async fn delete_document(
    State(state): State<TeamState>,
    Path((_team_id, doc_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    // TODO: SEC - Verify document belongs to team_id before deleting
    let service = DocumentService::new(state.base_path.clone());

    service
        .delete_document(&state.pool, &doc_id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn download_document(
    State(state): State<TeamState>,
    Path((_team_id, doc_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // TODO: SEC - Verify document belongs to team_id before downloading
    let service = DocumentService::new(state.base_path.clone());

    let (data, name, mime_type) = service
        .download_document(&state.pool, &doc_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let headers = [
        ("content-type", mime_type),
        (
            "content-disposition",
            format!("attachment; filename=\"{}\"", name),
        ),
    ];

    Ok((headers, data))
}

async fn search_documents(
    State(state): State<TeamState>,
    Path(team_id): Path<String>,
    Query(query): Query<DocumentSearchQuery>,
) -> Result<Json<Vec<DocumentSearchResult>>, (StatusCode, String)> {
    let service = DocumentService::new(state.base_path.clone());

    service
        .search_documents(&state.pool, &team_id, &query)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}
