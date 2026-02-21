//! MongoDB routes - Document API

use axum::{
    extract::{Extension, Multipart, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::teams::{can_manage_team, is_team_member, AppState};
use crate::models::mongo::{
    DocumentAnalysisContext, DocumentStatus, DocumentSummary,
    DocumentVersionSummary, LockInfo, SmartLogContext,
};
use crate::services::mongo::{DocumentService, DocumentVersionService, SmartLogService, TeamService};
use crate::AuthenticatedUserId;

type RouteError = (StatusCode, String);

/// Fetch team and verify the user is a member. Returns the team on success.
async fn require_member(
    state: &AppState,
    team_id: &str,
    user_id: &str,
    action: &str,
) -> Result<(), RouteError> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, user_id) {
        return Err((StatusCode::FORBIDDEN, format!("Only team members can {action}")));
    }
    Ok(())
}

/// Fetch team and verify the user is an admin or owner.
async fn require_manager(
    state: &AppState,
    team_id: &str,
    user_id: &str,
    action: &str,
) -> Result<(), RouteError> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !can_manage_team(&team, user_id) {
        return Err((StatusCode::FORBIDDEN, format!("Only admin/owner can {action}")));
    }
    Ok(())
}

pub fn document_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/teams/{team_id}/documents",
            get(list_docs).post(upload_doc),
        )
        .route("/teams/{team_id}/documents/search", get(search_docs))
        .route(
            "/teams/{team_id}/documents/archived",
            get(list_archived_docs),
        )
        .route(
            "/teams/{team_id}/documents/{doc_id}",
            delete(delete_doc).put(update_doc_metadata),
        )
        .route(
            "/teams/{team_id}/documents/{doc_id}/download",
            get(download_doc),
        )
        // Phase 1: Inline content
        .route(
            "/teams/{team_id}/documents/{doc_id}/content",
            get(get_content).put(update_content),
        )
        // Phase 2: Locking
        .route(
            "/teams/{team_id}/documents/{doc_id}/lock",
            get(get_lock).post(acquire_lock).delete(release_lock),
        )
        // Phase 3: Versions
        .route(
            "/teams/{team_id}/documents/{doc_id}/versions",
            get(list_versions),
        )
        .route(
            "/teams/{team_id}/documents/{doc_id}/versions/{version_id}/content",
            get(get_version_content),
        )
        .route(
            "/teams/{team_id}/documents/{doc_id}/versions/{version_id}/rollback",
            post(rollback_version),
        )
        .route(
            "/teams/{team_id}/documents/{doc_id}/versions/{version_id}/tag",
            put(tag_version),
        )
        // Phase 2: Agent integration routes
        .route(
            "/teams/{team_id}/documents/ai-workbench",
            get(list_ai_workbench),
        )
        .route("/teams/{team_id}/documents/by-origin", get(list_by_origin))
        .route(
            "/teams/{team_id}/documents/{doc_id}/status",
            put(update_doc_status),
        )
        .route(
            "/teams/{team_id}/documents/{doc_id}/lineage",
            get(get_lineage),
        )
        .route(
            "/teams/{team_id}/documents/{doc_id}/derived",
            get(list_derived),
        )
        .route(
            "/teams/{team_id}/documents/{doc_id}/retry-analysis",
            post(retry_analysis),
        )
}

#[derive(Deserialize)]
struct ListQuery {
    folder_path: Option<String>,
    page: Option<u64>,
    limit: Option<u64>,
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
    page: Option<u64>,
    limit: Option<u64>,
    mime_type: Option<String>,
    folder_path: Option<String>,
}

#[derive(Serialize)]
struct DocumentsResponse {
    items: Vec<DocumentSummary>,
    total: u64,
    page: u64,
    limit: u64,
    #[serde(rename = "totalPages")]
    total_pages: u64,
}

async fn list_docs(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<ListQuery>,
) -> Result<Json<DocumentsResponse>, RouteError> {
    require_member(&state, &team_id, &user.0, "view documents").await?;

    let service = DocumentService::new((*state.db).clone());
    let result = service
        .list_paginated(&team_id, q.folder_path.as_deref(), q.page, q.limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DocumentsResponse {
        items: result.items,
        total: result.total,
        page: result.page,
        limit: result.limit,
        total_pages: result.total_pages,
    }))
}

async fn upload_doc(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<DocumentSummary>, RouteError> {
    require_member(&state, &team_id, &user.0, "upload documents").await?;

    let mut file_name = String::new();
    let mut file_data = Vec::new();
    let mut mime_type = "application/octet-stream".to_string();
    let mut folder_path: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    {
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
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
                    .to_vec();
            }
            "folder_path" => {
                let text = field.text().await.unwrap_or_default();
                if !text.is_empty() {
                    folder_path = Some(text);
                }
            }
            _ => {}
        }
    }

    if file_data.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "No file".to_string()));
    }

    // Fix MIME type for common extensions that browsers often send as octet-stream
    if mime_type == "application/octet-stream" || mime_type.is_empty() {
        if let Some(corrected) = guess_mime_from_extension(&file_name) {
            mime_type = corrected.to_string();
        }
    }

    // File size limit: 50MB
    const MAX_FILE_SIZE: usize = 50 * 1024 * 1024;
    if file_data.len() > MAX_FILE_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("File exceeds 50MB limit ({})", file_data.len()),
        ));
    }

    let file_size = file_data.len() as i64;
    let service = DocumentService::new((*state.db).clone());
    let doc = service
        .upload(
            &team_id,
            &user.0,
            &file_name,
            file_data,
            &mime_type,
            folder_path,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let doc_id_str = doc.id.map(|id| id.to_hex()).unwrap_or_default();

    // Create initial version (v1) so version history starts from upload
    let version_service = DocumentVersionService::new((*state.db).clone());
    if let Ok((data, _, _)) = service.download(&team_id, &doc_id_str).await {
        let _ = version_service
            .create_version(&doc_id_str, &team_id, &user.0, &user.0, data, "Initial upload")
            .await;
    }

    // Smart Log trigger
    if let Some(trigger) = &state.smart_log_trigger {
        let content_for_ai = match service
            .get_text_content_chunked(&team_id, &doc_id_str, None, Some(2000))
            .await
        {
            Ok((text, _mime, _total)) => Some(format!(
                "文件名: {}\nMIME类型: {}\n文档内容:\n{}",
                file_name,
                mime_type,
                text.chars().take(1500).collect::<String>()
            )),
            Err(_) => {
                Some(format!("文件名: {}\nMIME类型: {}", file_name, mime_type))
            }
        };

        trigger.trigger(
            SmartLogContext::new(
                team_id.clone(),
                user.0.clone(),
                "upload",
                "document",
                doc_id_str.clone(),
                file_name.clone(),
                content_for_ai,
            )
            .with_pending_analysis(state.doc_analysis_trigger.is_some()),
        );
    }

    // Trigger automatic document analysis
    if let Some(trigger) = &state.doc_analysis_trigger {
        trigger.trigger(DocumentAnalysisContext {
            team_id: team_id.clone(),
            doc_id: doc_id_str,
            doc_name: file_name.clone(),
            mime_type: mime_type.clone(),
            file_size,
            user_id: user.0.clone(),
            lang: None,
            extra_instructions: None,
        });
    }

    Ok(Json(DocumentSummary::from(doc)))
}

async fn delete_doc(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, doc_id)): Path<(String, String)>,
) -> Result<StatusCode, RouteError> {
    require_manager(&state, &team_id, &user.0, "delete documents").await?;

    let service = DocumentService::new((*state.db).clone());

    // Cancel any pending AI analysis for this document
    let smart_log_svc = SmartLogService::new((*state.db).clone());
    let _ = smart_log_svc.cancel_pending_analysis(&team_id, &doc_id).await;

    // Smart Log trigger (before delete)
    if let Some(trigger) = &state.smart_log_trigger {
        let doc_name = service
            .get_metadata(&team_id, &doc_id)
            .await
            .map(|s| s.name)
            .unwrap_or_else(|_| doc_id.clone());
        trigger.trigger(SmartLogContext::new(
            team_id.clone(),
            user.0.clone(),
            "delete",
            "document",
            doc_id.clone(),
            doc_name,
            None,
        ));
    }

    service
        .delete(&team_id, &doc_id, &user.0)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn download_doc(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, doc_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, RouteError> {
    require_member(&state, &team_id, &user.0, "download documents").await?;

    let service = DocumentService::new((*state.db).clone());
    let (data, name, mime) = service
        .download(&team_id, &doc_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let headers = [
        ("content-type", mime),
        (
            "content-disposition",
            format!("attachment; filename=\"{}\"", name),
        ),
    ];
    Ok((headers, data))
}

async fn search_docs(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<DocumentsResponse>, RouteError> {
    require_member(&state, &team_id, &user.0, "search documents").await?;

    let query_str = q.q.unwrap_or_default();
    if query_str.is_empty() {
        return Ok(Json(DocumentsResponse {
            items: vec![],
            total: 0,
            page: 1,
            limit: q.limit.unwrap_or(50),
            total_pages: 0,
        }));
    }

    let service = DocumentService::new((*state.db).clone());
    let result = service
        .search(
            &team_id,
            &query_str,
            q.page,
            q.limit,
            q.mime_type.as_deref(),
            q.folder_path.as_deref(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DocumentsResponse {
        items: result.items,
        total: result.total,
        page: result.page,
        limit: result.limit,
        total_pages: result.total_pages,
    }))
}

// ── Archived documents ──

async fn list_archived_docs(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<ListQuery>,
) -> Result<Json<DocumentsResponse>, RouteError> {
    require_member(&state, &team_id, &user.0, "view archived documents").await?;

    let service = DocumentService::new((*state.db).clone());
    let result = service
        .list_archived(&team_id, q.page, q.limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Convert ArchivedDocument to DocumentSummary so frontend type matches
    let items: Vec<DocumentSummary> = result.items.into_iter().map(|a| a.to_summary()).collect();

    Ok(Json(DocumentsResponse {
        items,
        total: result.total,
        page: result.page,
        limit: result.limit,
        total_pages: result.total_pages,
    }))
}

// ── Phase 1: Inline content ──

#[derive(Deserialize)]
struct ContentQuery {
    format: Option<String>,
}

#[derive(Serialize)]
struct TextContentResponse {
    text: String,
    mime_type: String,
}

async fn get_content(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, doc_id)): Path<(String, String)>,
    Query(q): Query<ContentQuery>,
) -> Result<impl IntoResponse, RouteError> {
    require_member(&state, &team_id, &user.0, "view content").await?;

    let service = DocumentService::new((*state.db).clone());

    // If format=text, return JSON with text content
    if q.format.as_deref() == Some("text") {
        let (text, mime_type) = service
            .get_text_content(&team_id, &doc_id)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

        let body = serde_json::to_string(&TextContentResponse { text, mime_type })
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        return Ok((
            [
                ("content-type".to_string(), "application/json".to_string()),
                ("content-disposition".to_string(), "inline".to_string()),
            ],
            body.into_bytes(),
        ));
    }

    // Otherwise return raw content inline
    let (data, name, mime) = service
        .download(&team_id, &doc_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok((
        [
            ("content-type".to_string(), mime),
            (
                "content-disposition".to_string(),
                format!("inline; filename=\"{}\"", name),
            ),
        ],
        data,
    ))
}

// ── Phase 2: Content update & Locking ──

#[derive(Deserialize)]
struct UpdateContentRequest {
    content: String,
    message: Option<String>,
}

async fn update_content(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, doc_id)): Path<(String, String)>,
    Json(body): Json<UpdateContentRequest>,
) -> Result<Json<DocumentSummary>, RouteError> {
    require_member(&state, &team_id, &user.0, "update content").await?;

    let service = DocumentService::new((*state.db).clone());

    // Check lock
    let has_lock = service
        .check_lock(&doc_id, &user.0)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !has_lock {
        return Err((
            StatusCode::CONFLICT,
            "You must hold the lock to update content".to_string(),
        ));
    }

    // Save old version before updating
    let version_service = DocumentVersionService::new((*state.db).clone());
    if let Ok((old_data, _, _)) = service.download(&team_id, &doc_id).await {
        let _ = version_service
            .create_version(
                &doc_id,
                &team_id,
                &user.0,
                &user.0,
                old_data,
                &body.message.clone().unwrap_or_else(|| "Update".to_string()),
            )
            .await;
    }

    let message = body.message.unwrap_or_else(|| "Update".to_string());
    // Capture content preview before into_bytes() consumes it
    let content_preview: String = body.content.chars().take(1500).collect();
    let data = body.content.into_bytes();
    let data_len = data.len() as i64;

    let doc = service
        .update_content(&team_id, &doc_id, &user.0, data, &message)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Smart Log trigger
    if let Some(trigger) = &state.smart_log_trigger {
        trigger.trigger(
            SmartLogContext::new(
                team_id.clone(),
                user.0.clone(),
                "update",
                "document",
                doc_id.clone(),
                doc.name.clone(),
                Some(format!(
                    "文件名: {}\n更新说明: {}\n文档内容:\n{}",
                    doc.name, message, content_preview
                )),
            )
            .with_pending_analysis(state.doc_analysis_trigger.is_some()),
        );
    }

    // Re-analyze document after content update
    if let Some(trigger) = &state.doc_analysis_trigger {
        trigger.trigger(DocumentAnalysisContext {
            team_id: team_id.clone(),
            doc_id: doc_id.clone(),
            doc_name: doc.name.clone(),
            mime_type: doc.mime_type.clone(),
            file_size: data_len,
            user_id: user.0.clone(),
            lang: None,
            extra_instructions: None,
        });
    }

    Ok(Json(DocumentSummary::from(doc)))
}

async fn acquire_lock(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, doc_id)): Path<(String, String)>,
) -> Result<Json<LockInfo>, RouteError> {
    require_member(&state, &team_id, &user.0, "acquire locks").await?;

    let service = DocumentService::new((*state.db).clone());
    let lock = service
        .acquire_lock(&doc_id, &team_id, &user.0, &user.0)
        .await
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    Ok(Json(LockInfo::from(lock)))
}

async fn release_lock(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, doc_id)): Path<(String, String)>,
) -> Result<StatusCode, RouteError> {
    require_member(&state, &team_id, &user.0, "release locks").await?;

    let service = DocumentService::new((*state.db).clone());
    service
        .release_lock(&doc_id, &user.0)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

async fn get_lock(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, doc_id)): Path<(String, String)>,
) -> Result<Json<Option<LockInfo>>, RouteError> {
    require_member(&state, &team_id, &user.0, "view locks").await?;

    let service = DocumentService::new((*state.db).clone());
    let lock = service
        .get_lock(&doc_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(lock.map(LockInfo::from)))
}

// ── Phase 3: Versions ──

#[derive(Deserialize)]
struct VersionQuery {
    page: Option<u64>,
    limit: Option<u64>,
}

#[derive(Serialize)]
struct VersionsResponse {
    items: Vec<DocumentVersionSummary>,
    total: u64,
    page: u64,
    limit: u64,
    #[serde(rename = "totalPages")]
    total_pages: u64,
}

async fn list_versions(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, doc_id)): Path<(String, String)>,
    Query(q): Query<VersionQuery>,
) -> Result<Json<VersionsResponse>, RouteError> {
    require_member(&state, &team_id, &user.0, "view versions").await?;

    let service = DocumentVersionService::new((*state.db).clone());
    let result = service
        .list_versions(&doc_id, q.page, q.limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(VersionsResponse {
        items: result.items,
        total: result.total,
        page: result.page,
        limit: result.limit,
        total_pages: result.total_pages,
    }))
}

async fn get_version_content(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, _doc_id, version_id)): Path<(String, String, String)>,
) -> Result<Json<TextContentResponse>, RouteError> {
    require_member(&state, &team_id, &user.0, "view version content").await?;

    let service = DocumentVersionService::new((*state.db).clone());
    let (text, mime_type) = service
        .get_version_text(&version_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(Json(TextContentResponse { text, mime_type }))
}

async fn rollback_version(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, doc_id, version_id)): Path<(String, String, String)>,
) -> Result<Json<DocumentVersionSummary>, RouteError> {
    require_manager(&state, &team_id, &user.0, "rollback versions").await?;

    let service = DocumentVersionService::new((*state.db).clone());
    let version = service
        .rollback(&doc_id, &version_id, &user.0, &user.0)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DocumentVersionSummary::from(version)))
}

#[derive(Deserialize)]
struct TagRequest {
    tag: String,
}

async fn tag_version(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, _doc_id, version_id)): Path<(String, String, String)>,
    Json(body): Json<TagRequest>,
) -> Result<StatusCode, RouteError> {
    require_manager(&state, &team_id, &user.0, "tag versions").await?;

    let service = DocumentVersionService::new((*state.db).clone());
    service
        .tag_version(&version_id, &body.tag)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Document metadata update ──

#[derive(Deserialize)]
struct UpdateDocumentRequest {
    display_name: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
}

async fn update_doc_metadata(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, doc_id)): Path<(String, String)>,
    Json(body): Json<UpdateDocumentRequest>,
) -> Result<Json<DocumentSummary>, RouteError> {
    require_member(&state, &team_id, &user.0, "update documents").await?;

    let service = DocumentService::new((*state.db).clone());
    let summary = service
        .update_metadata(
            &team_id,
            &doc_id,
            body.display_name,
            body.description,
            body.tags,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Smart Log trigger
    if let Some(trigger) = &state.smart_log_trigger {
        trigger.trigger(SmartLogContext::new(
            team_id.clone(),
            user.0.clone(),
            "update",
            "document",
            doc_id.clone(),
            summary.name.clone(),
            Some(format!(
                "文件名: {}\n描述: {}",
                summary.name,
                summary.description.as_deref().unwrap_or("")
            )),
        ));
    }

    Ok(Json(summary))
}

// ── Phase 2: Agent integration routes ──

#[derive(Deserialize)]
struct AiWorkbenchQuery {
    session_id: Option<String>,
    mission_id: Option<String>,
    page: Option<u64>,
    limit: Option<u64>,
}

async fn list_ai_workbench(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<AiWorkbenchQuery>,
) -> Result<Json<DocumentsResponse>, RouteError> {
    require_member(&state, &team_id, &user.0, "view AI workbench").await?;

    let service = DocumentService::new((*state.db).clone());
    let result = service
        .list_ai_workbench(
            &team_id,
            q.session_id.as_deref(),
            q.mission_id.as_deref(),
            q.page,
            q.limit,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DocumentsResponse {
        items: result.items,
        total: result.total,
        page: result.page,
        limit: result.limit,
        total_pages: result.total_pages,
    }))
}

#[derive(Deserialize)]
struct ByOriginQuery {
    origin: String,
    page: Option<u64>,
    limit: Option<u64>,
}

async fn list_by_origin(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<ByOriginQuery>,
) -> Result<Json<DocumentsResponse>, RouteError> {
    require_member(&state, &team_id, &user.0, "view documents").await?;

    let service = DocumentService::new((*state.db).clone());
    let result = service
        .list_by_origin(&team_id, &q.origin, q.page, q.limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DocumentsResponse {
        items: result.items,
        total: result.total,
        page: result.page,
        limit: result.limit,
        total_pages: result.total_pages,
    }))
}

#[derive(Deserialize)]
struct UpdateStatusRequest {
    status: DocumentStatus,
}

async fn update_doc_status(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, doc_id)): Path<(String, String)>,
    Json(body): Json<UpdateStatusRequest>,
) -> Result<StatusCode, RouteError> {
    require_member(&state, &team_id, &user.0, "update document status").await?;

    let service = DocumentService::new((*state.db).clone());
    service
        .update_status(&team_id, &doc_id, body.status)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Smart Log: trigger when document status changes to Accepted
    if body.status == DocumentStatus::Accepted {
        if let Some(trigger) = &state.smart_log_trigger {
            let meta = service.get_metadata(&team_id, &doc_id).await.ok();
            let doc_name = meta
                .as_ref()
                .map(|m| m.name.clone())
                .unwrap_or_else(|| doc_id.clone());

            let content_for_ai = match service
                .get_text_content_chunked(&team_id, &doc_id, None, Some(2000))
                .await
            {
                Ok((text, _mime, _total)) => {
                    let name = meta.as_ref().map(|m| m.name.as_str()).unwrap_or("unknown");
                    let origin = meta
                        .as_ref()
                        .map(|m| format!("{:?}", m.origin))
                        .unwrap_or_default();
                    let category = meta
                        .as_ref()
                        .map(|m| format!("{:?}", m.category))
                        .unwrap_or_default();
                    Some(format!(
                        "文件名: {}\n来源: {}\n类别: {}\n内容:\n{}",
                        name,
                        origin,
                        category,
                        text.chars().take(1500).collect::<String>()
                    ))
                }
                Err(_) => None,
            };

            trigger.trigger(SmartLogContext::new(
                team_id.clone(),
                user.0.clone(),
                "accept",
                "document",
                doc_id.clone(),
                doc_name,
                content_for_ai,
            ));
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn get_lineage(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, doc_id)): Path<(String, String)>,
) -> Result<Json<Vec<DocumentSummary>>, RouteError> {
    require_member(&state, &team_id, &user.0, "view lineage").await?;

    let service = DocumentService::new((*state.db).clone());
    let lineage = service
        .get_lineage(&team_id, &doc_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(lineage))
}

async fn list_derived(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, doc_id)): Path<(String, String)>,
    Query(q): Query<ListQuery>,
) -> Result<Json<DocumentsResponse>, RouteError> {
    require_member(&state, &team_id, &user.0, "view derived documents").await?;

    let service = DocumentService::new((*state.db).clone());
    let result = service
        .list_derived(&team_id, &doc_id, q.page, q.limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DocumentsResponse {
        items: result.items,
        total: result.total,
        page: result.page,
        limit: result.limit,
        total_pages: result.total_pages,
    }))
}

#[derive(Deserialize, Default)]
struct RetryAnalysisRequest {
    prompt: Option<String>,
}

/// Re-trigger document analysis for a specific document.
async fn retry_analysis(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, doc_id)): Path<(String, String)>,
    body: Option<Json<RetryAnalysisRequest>>,
) -> Result<StatusCode, RouteError> {
    require_member(&state, &team_id, &user.0, "retry analysis").await?;

    let trigger = state.doc_analysis_trigger.as_ref().ok_or((
        StatusCode::BAD_REQUEST,
        "Document analysis not available".to_string(),
    ))?;

    let service = DocumentService::new((*state.db).clone());
    let meta = service
        .get_metadata(&team_id, &doc_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    // Reset SmartLog analysis status to pending
    let smart_log_svc = SmartLogService::new((*state.db).clone());
    let _ = smart_log_svc.reset_analysis_to_pending(&team_id, &doc_id).await;

    let extra = body.and_then(|b| b.0.prompt.filter(|s| !s.trim().is_empty()));

    trigger.trigger(DocumentAnalysisContext {
        team_id,
        doc_id,
        doc_name: meta.name,
        mime_type: meta.mime_type,
        file_size: meta.file_size,
        user_id: user.0,
        lang: None,
        extra_instructions: extra,
    });

    Ok(StatusCode::ACCEPTED)
}

/// Guess MIME type from file extension when browser sends octet-stream.
fn guess_mime_from_extension(filename: &str) -> Option<&'static str> {
    let ext = filename.rsplit('.').next()?.to_lowercase();
    match ext.as_str() {
        "md" | "markdown" => Some("text/markdown"),
        "txt" => Some("text/plain"),
        "json" => Some("application/json"),
        "yaml" | "yml" => Some("application/x-yaml"),
        "xml" => Some("application/xml"),
        "html" | "htm" => Some("text/html"),
        "css" => Some("text/css"),
        "js" => Some("application/javascript"),
        "ts" => Some("application/typescript"),
        "csv" => Some("text/csv"),
        "py" => Some("text/x-python"),
        "rs" => Some("text/x-rust"),
        "go" => Some("text/x-go"),
        "java" => Some("text/x-java"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}
