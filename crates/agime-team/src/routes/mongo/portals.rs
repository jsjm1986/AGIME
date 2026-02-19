//! MongoDB routes - Portal API (authenticated)

use axum::{
    extract::{Extension, Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::{
    io::Read,
    path::{Component, Path as FsPath, PathBuf},
    time::SystemTime,
};

use super::teams::{can_manage_team, is_team_member, AppState};
use crate::models::mongo::{
    CreatePortalRequest, PaginatedResponse, PortalDetail, PortalInteraction, PortalStatus,
    UpdatePortalRequest,
};
use crate::services::mongo::{PortalService, TeamService};
use crate::AuthenticatedUserId;

pub fn portal_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/teams/{team_id}/portals",
            get(list_portals).post(create_portal),
        )
        .route(
            "/teams/{team_id}/portals/check-slug",
            get(check_slug),
        )
        .route(
            "/teams/{team_id}/portals/{portal_id}",
            get(get_portal).put(update_portal).delete(delete_portal),
        )
        .route(
            "/teams/{team_id}/portals/{portal_id}/publish",
            post(publish_portal),
        )
        .route(
            "/teams/{team_id}/portals/{portal_id}/unpublish",
            post(unpublish_portal),
        )
        .route(
            "/teams/{team_id}/portals/{portal_id}/interactions",
            get(list_interactions),
        )
        .route(
            "/teams/{team_id}/portals/{portal_id}/stats",
            get(get_stats),
        )
        .route(
            "/teams/{team_id}/portals/{portal_id}/files",
            get(list_portal_files),
        )
        .route(
            "/teams/{team_id}/portals/{portal_id}/file",
            get(read_portal_file),
        )
        .route(
            "/teams/{team_id}/portals/{portal_id}/preview",
            get(preview_portal_index),
        )
        .route(
            "/teams/{team_id}/portals/{portal_id}/preview/{*path}",
            get(preview_portal_page),
        )
}

// ---------------------------------------------------------------------------
// Query types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PaginationQuery {
    page: Option<u64>,
    limit: Option<u64>,
}

#[derive(Deserialize)]
struct SlugQuery {
    slug: String,
}

#[derive(Deserialize)]
struct PortalFileQuery {
    path: Option<String>,
}

#[derive(Deserialize)]
struct PortalFileContentQuery {
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PortalFileEntry {
    name: String,
    path: String,
    is_dir: bool,
    size: Option<u64>,
    modified_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PortalFileListResponse {
    path: String,
    parent_path: Option<String>,
    entries: Vec<PortalFileEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PortalFileContentResponse {
    path: String,
    name: String,
    content_type: String,
    size: u64,
    modified_at: Option<String>,
    is_text: bool,
    truncated: bool,
    content: Option<String>,
}

const MAX_FILE_PREVIEW_BYTES: usize = 512 * 1024;

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_portals(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let team = get_team_checked(&state, &team_id, &user.0).await?;
    if !is_team_member(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Not a team member".to_string()));
    }

    let svc = PortalService::new((*state.db).clone());
    let result = svc
        .list(&team_id, q.page.unwrap_or(1), q.limit.unwrap_or(20))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Inject publicUrl/previewUrl into each item and include portalBaseUrl at top level
    let base = &state.portal_base_url;
    let items: Vec<serde_json::Value> = result
        .items
        .iter()
        .map(|p| {
            let mut v = serde_json::to_value(p).unwrap_or_default();
            if let Some(obj) = v.as_object_mut() {
                inject_portal_urls(
                    obj,
                    &team_id,
                    &p.id,
                    &p.slug,
                    p.status,
                    base,
                    state.portal_base_url_configured,
                    state.portal_test_base_url.as_deref(),
                );
            }
            v
        })
        .collect();

    Ok(Json(serde_json::json!({
        "items": items,
        "total": result.total,
        "page": result.page,
        "limit": result.limit,
        "totalPages": result.total_pages,
        "portalBaseUrl": if state.portal_base_url_configured {
            serde_json::Value::String(base.clone())
        } else {
            serde_json::Value::Null
        },
        "portalTestBaseUrl": state
            .portal_test_base_url
            .as_ref()
            .map(|s| serde_json::Value::String(s.clone()))
            .unwrap_or(serde_json::Value::Null),
    })))
}

async fn create_portal(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Json(req): Json<CreatePortalRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let team = get_team_checked(&state, &team_id, &user.0).await?;
    if !can_manage_team(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Admin or owner required".to_string()));
    }

    let svc = PortalService::new((*state.db).clone());
    let created_portal = svc
        .create(&team_id, &user.0, req)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let created_detail = PortalDetail::from(created_portal);

    if let Err(e) = svc
        .initialize_project_folder(
            &team_id,
            &created_detail.id,
            &created_detail.slug,
            &created_detail.name,
            &state.workspace_root,
        )
        .await
    {
        // Best effort cleanup to avoid leaving broken portals without project_path.
        if let Err(cleanup_err) = svc.delete(&team_id, &created_detail.id).await {
            tracing::warn!(
                "failed to cleanup portal {} after project init error: {}",
                created_detail.id,
                cleanup_err
            );
        }
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to initialize portal project: {}", e),
        ));
    }

    let portal = svc
        .get(&team_id, &created_detail.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let detail = PortalDetail::from(portal);
    let mut v = serde_json::to_value(&detail).unwrap_or_default();
    if let Some(obj) = v.as_object_mut() {
        inject_portal_urls(
            obj,
            &detail.team_id,
            &detail.id,
            &detail.slug,
            detail.status,
            &state.portal_base_url,
            state.portal_base_url_configured,
            state.portal_test_base_url.as_deref(),
        );
    }
    Ok(Json(v))
}

async fn check_slug(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<SlugQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let team = get_team_checked(&state, &team_id, &user.0).await?;
    if !is_team_member(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Not a team member".to_string()));
    }

    let svc = PortalService::new((*state.db).clone());
    let available = svc
        .check_slug(&q.slug)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "available": available, "slug": q.slug })))
}

async fn get_portal(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, portal_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let team = get_team_checked(&state, &team_id, &user.0).await?;
    if !is_team_member(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Not a team member".to_string()));
    }

    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .get(&team_id, &portal_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let detail = PortalDetail::from(portal);
    let mut v = serde_json::to_value(&detail).unwrap_or_default();
    if let Some(obj) = v.as_object_mut() {
        inject_portal_urls(
            obj,
            &detail.team_id,
            &detail.id,
            &detail.slug,
            detail.status,
            &state.portal_base_url,
            state.portal_base_url_configured,
            state.portal_test_base_url.as_deref(),
        );
    }
    Ok(Json(v))
}

async fn update_portal(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, portal_id)): Path<(String, String)>,
    Json(req): Json<UpdatePortalRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let team = get_team_checked(&state, &team_id, &user.0).await?;
    if !can_manage_team(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Admin or owner required".to_string()));
    }

    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .update(&team_id, &portal_id, req)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let detail = PortalDetail::from(portal);
    let mut v = serde_json::to_value(&detail).unwrap_or_default();
    if let Some(obj) = v.as_object_mut() {
        inject_portal_urls(
            obj,
            &detail.team_id,
            &detail.id,
            &detail.slug,
            detail.status,
            &state.portal_base_url,
            state.portal_base_url_configured,
            state.portal_test_base_url.as_deref(),
        );
    }
    Ok(Json(v))
}

async fn delete_portal(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, portal_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let team = get_team_checked(&state, &team_id, &user.0).await?;
    if !can_manage_team(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Admin or owner required".to_string()));
    }

    let svc = PortalService::new((*state.db).clone());
    let portal = get_portal_by_id_or_slug_for_team(&svc, &team_id, &portal_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let resolved_id = portal
        .id
        .map(|id| id.to_hex())
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "Portal id missing".to_string()))?;
    let project_path = portal.project_path.clone();

    svc.delete(&team_id, &resolved_id)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Best-effort cleanup of filesystem project directory
    if let Some(path) = project_path {
        if !path.trim().is_empty() {
            if is_workspace_subdir(state.workspace_root.as_str(), path.as_str()) {
                if let Err(e) = std::fs::remove_dir_all(&path) {
                    tracing::warn!("Failed to remove portal project folder '{}': {}", path, e);
                }
            } else {
                tracing::warn!(
                    "Skip removing portal project folder outside workspace root: {}",
                    path
                );
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn publish_portal(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, portal_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let team = get_team_checked(&state, &team_id, &user.0).await?;
    if !can_manage_team(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Admin or owner required".to_string()));
    }

    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .publish(&team_id, &portal_id)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let detail = PortalDetail::from(portal);
    let mut v = serde_json::to_value(&detail).unwrap_or_default();
    if let Some(obj) = v.as_object_mut() {
        inject_portal_urls(
            obj,
            &detail.team_id,
            &detail.id,
            &detail.slug,
            detail.status,
            &state.portal_base_url,
            state.portal_base_url_configured,
            state.portal_test_base_url.as_deref(),
        );
    }
    Ok(Json(v))
}

async fn unpublish_portal(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, portal_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let team = get_team_checked(&state, &team_id, &user.0).await?;
    if !can_manage_team(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Admin or owner required".to_string()));
    }

    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .unpublish(&team_id, &portal_id)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let detail = PortalDetail::from(portal);
    let mut v = serde_json::to_value(&detail).unwrap_or_default();
    if let Some(obj) = v.as_object_mut() {
        inject_portal_urls(
            obj,
            &detail.team_id,
            &detail.id,
            &detail.slug,
            detail.status,
            &state.portal_base_url,
            state.portal_base_url_configured,
            state.portal_test_base_url.as_deref(),
        );
    }
    Ok(Json(v))
}

async fn list_interactions(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, portal_id)): Path<(String, String)>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<PortalInteraction>>, (StatusCode, String)> {
    let team = get_team_checked(&state, &team_id, &user.0).await?;
    if !is_team_member(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Not a team member".to_string()));
    }

    let svc = PortalService::new((*state.db).clone());
    let result = svc
        .list_interactions(&team_id, &portal_id, q.page.unwrap_or(1), q.limit.unwrap_or(20))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(result))
}

async fn get_stats(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, portal_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let team = get_team_checked(&state, &team_id, &user.0).await?;
    if !is_team_member(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Not a team member".to_string()));
    }

    let svc = PortalService::new((*state.db).clone());
    let stats = svc
        .get_stats(&team_id, &portal_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(stats))
}

async fn list_portal_files(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, portal_id)): Path<(String, String)>,
    Query(q): Query<PortalFileQuery>,
) -> Result<Json<PortalFileListResponse>, (StatusCode, String)> {
    let team = get_team_checked(&state, &team_id, &user.0).await?;
    if !is_team_member(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Not a team member".to_string()));
    }

    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .get(&team_id, &portal_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let project_path = portal.project_path.ok_or((
        StatusCode::BAD_REQUEST,
        "Portal has no project path".to_string(),
    ))?;

    let base = FsPath::new(&project_path);
    let rel_path = normalize_relative_path(q.path.unwrap_or_default().as_str()).ok_or((
        StatusCode::BAD_REQUEST,
        "Invalid path".to_string(),
    ))?;
    let target = if rel_path.is_empty() {
        base.to_path_buf()
    } else {
        base.join(&rel_path)
    };

    let base_canon = base
        .canonicalize()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let target_canon = target
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Path not found".to_string()))?;
    if !target_canon.starts_with(&base_canon) {
        return Err((StatusCode::FORBIDDEN, "Path traversal detected".to_string()));
    }
    if !target_canon.is_dir() {
        return Err((StatusCode::BAD_REQUEST, "Path is not a directory".to_string()));
    }

    let mut entries = Vec::<PortalFileEntry>::new();
    let read_dir =
        std::fs::read_dir(&target_canon).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    for item in read_dir {
        let entry = match item {
            Ok(v) => v,
            Err(_) => continue,
        };
        let file_name = match entry.file_name().into_string() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let path = entry.path();
        let meta = match entry.metadata() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let is_dir = meta.is_dir();
        let rel = match path.strip_prefix(&base_canon) {
            Ok(v) => normalize_relative_path(v.to_string_lossy().as_ref()).unwrap_or_default(),
            Err(_) => continue,
        };
        let modified_at = meta
            .modified()
            .ok()
            .map(system_time_to_rfc3339);
        entries.push(PortalFileEntry {
            name: file_name,
            path: rel,
            is_dir,
            size: if is_dir { None } else { Some(meta.len()) },
            modified_at,
        });
    }

    entries.sort_by(|a, b| {
        if a.is_dir != b.is_dir {
            return b.is_dir.cmp(&a.is_dir);
        }
        a.name.to_lowercase().cmp(&b.name.to_lowercase())
    });

    Ok(Json(PortalFileListResponse {
        path: rel_path.clone(),
        parent_path: parent_relative_path(&rel_path),
        entries,
    }))
}

async fn read_portal_file(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, portal_id)): Path<(String, String)>,
    Query(q): Query<PortalFileContentQuery>,
) -> Result<Json<PortalFileContentResponse>, (StatusCode, String)> {
    let team = get_team_checked(&state, &team_id, &user.0).await?;
    if !is_team_member(&team, &user.0) {
        return Err((StatusCode::FORBIDDEN, "Not a team member".to_string()));
    }

    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .get(&team_id, &portal_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let project_path = portal.project_path.ok_or((
        StatusCode::BAD_REQUEST,
        "Portal has no project path".to_string(),
    ))?;

    let rel_path = normalize_relative_path(q.path.as_str()).ok_or((
        StatusCode::BAD_REQUEST,
        "Invalid path".to_string(),
    ))?;
    if rel_path.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "File path is required".to_string()));
    }

    let base = FsPath::new(&project_path);
    let target = base.join(&rel_path);
    let base_canon = base
        .canonicalize()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let target_canon = target
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Path not found".to_string()))?;
    if !target_canon.starts_with(&base_canon) {
        return Err((StatusCode::FORBIDDEN, "Path traversal detected".to_string()));
    }
    if !target_canon.is_file() {
        return Err((StatusCode::BAD_REQUEST, "Path is not a file".to_string()));
    }

    let meta = std::fs::metadata(&target_canon)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let size = meta.len();
    let modified_at = meta.modified().ok().map(system_time_to_rfc3339);
    let name = target_canon
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| rel_path.clone());
    let content_type = mime_guess::from_path(&target_canon)
        .first_or_octet_stream()
        .to_string();

    let file = std::fs::File::open(&target_canon)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut bytes = Vec::new();
    file.take((MAX_FILE_PREVIEW_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let truncated = bytes.len() > MAX_FILE_PREVIEW_BYTES;
    if truncated {
        bytes.truncate(MAX_FILE_PREVIEW_BYTES);
    }

    let has_nul = bytes.iter().take(4096).any(|b| *b == 0);
    let looks_textual_mime = content_type.starts_with("text/")
        || content_type.contains("json")
        || content_type.contains("xml")
        || content_type.contains("javascript")
        || content_type.contains("typescript");
    let is_text = !has_nul && (looks_textual_mime || std::str::from_utf8(&bytes).is_ok());
    let content = if is_text {
        Some(String::from_utf8_lossy(&bytes).to_string())
    } else {
        None
    };

    Ok(Json(PortalFileContentResponse {
        path: rel_path,
        name,
        content_type,
        size,
        modified_at,
        is_text,
        truncated,
        content,
    }))
}

async fn preview_portal_index(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, portal_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (body, content_type) =
        read_portal_preview_bytes(&state, &team_id, &portal_id, &user.0, "").await?;
    Ok(([(header::CONTENT_TYPE, content_type)], body))
}

async fn preview_portal_page(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path((team_id, portal_id, path)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (body, content_type) =
        read_portal_preview_bytes(&state, &team_id, &portal_id, &user.0, &path).await?;
    Ok(([(header::CONTENT_TYPE, content_type)], body))
}

async fn read_portal_preview_bytes(
    state: &Arc<AppState>,
    team_id: &str,
    portal_id: &str,
    user_id: &str,
    relative_path: &str,
) -> Result<(Vec<u8>, String), (StatusCode, String)> {
    let team = get_team_checked(state, team_id, user_id).await?;
    if !is_team_member(&team, user_id) {
        return Err((StatusCode::FORBIDDEN, "Not a team member".to_string()));
    }

    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .get(team_id, portal_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let project_path = portal.project_path.ok_or((
        StatusCode::BAD_REQUEST,
        "Portal has no project path".to_string(),
    ))?;

    serve_portal_file_from_filesystem(&project_path, relative_path)
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn build_preview_url(team_id: &str, portal_id: &str) -> String {
    format!("/api/team/teams/{}/portals/{}/preview", team_id, portal_id)
}

fn build_public_url(
    base_url: &str,
    slug: &str,
    status: PortalStatus,
    base_url_configured: bool,
) -> Option<String> {
    if status != PortalStatus::Published {
        return None;
    }
    if !base_url_configured {
        return Some(format!("/p/{}", slug));
    }
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        Some(format!("/p/{}", slug))
    } else {
        Some(format!("{}/p/{}", trimmed, slug))
    }
}

fn inject_portal_urls(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    team_id: &str,
    portal_id: &str,
    slug: &str,
    status: PortalStatus,
    base_url: &str,
    base_url_configured: bool,
    test_base_url: Option<&str>,
) {
    let public_url = build_public_url(base_url, slug, status, base_url_configured)
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null);
    obj.insert("publicUrl".to_string(), public_url);
    let test_public_url = build_test_public_url(test_base_url, slug, status)
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null);
    obj.insert("testPublicUrl".to_string(), test_public_url);
    obj.insert(
        "previewUrl".to_string(),
        serde_json::Value::String(build_preview_url(team_id, portal_id)),
    );
}

fn build_test_public_url(
    test_base_url: Option<&str>,
    slug: &str,
    status: PortalStatus,
) -> Option<String> {
    if status != PortalStatus::Published {
        return None;
    }
    let base = test_base_url?;
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        None
    } else {
        Some(format!("{}/p/{}", trimmed, slug))
    }
}

fn serve_portal_file_from_filesystem(
    project_path: &str,
    relative_path: &str,
) -> Result<(Vec<u8>, String), (StatusCode, String)> {
    let base = FsPath::new(project_path);
    let base_canon = base
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Project path not found".to_string()))?;

    let clean = normalize_relative_path(relative_path)
        .ok_or((StatusCode::BAD_REQUEST, "Invalid path".to_string()))?;

    // Block access to _private/ directory (used for server-side data storage)
    if clean.starts_with("_private/") || clean.starts_with("_private\\") || clean == "_private" {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    let file_path = if clean.is_empty() || clean == "index" {
        base_canon.join("index.html")
    } else {
        let candidate = base_canon.join(&clean);
        if candidate.is_dir() {
            candidate.join("index.html")
        } else if candidate.exists() {
            candidate
        } else {
            let has_ext = FsPath::new(&clean).extension().is_some();
            if has_ext {
                return Err((StatusCode::NOT_FOUND, "File not found".to_string()));
            }
            base_canon.join("index.html")
        }
    };

    let file_canon = file_path
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "File not found".to_string()))?;
    if !file_canon.starts_with(&base_canon) {
        return Err((StatusCode::FORBIDDEN, "Path traversal detected".to_string()));
    }
    if !file_canon.is_file() {
        return Err((StatusCode::NOT_FOUND, "File not found".to_string()));
    }

    let body = std::fs::read(&file_canon)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let content_type = mime_guess::from_path(&file_canon)
        .first_or_octet_stream()
        .to_string();
    Ok((body, content_type))
}

fn normalize_relative_path(raw: &str) -> Option<String> {
    let p = FsPath::new(raw);
    let mut parts = Vec::<String>::new();
    for comp in p.components() {
        match comp {
            Component::Normal(seg) => {
                let s = seg.to_string_lossy().trim().to_string();
                if !s.is_empty() {
                    parts.push(s);
                }
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

fn parent_relative_path(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let parent = PathBuf::from(path).parent()?.to_string_lossy().to_string();
    Some(parent.replace('\\', "/"))
}

fn is_workspace_subdir(workspace_root: &str, candidate_path: &str) -> bool {
    let root = PathBuf::from(workspace_root);
    let candidate = PathBuf::from(candidate_path);
    let root_canon = match root.canonicalize() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let candidate_canon = match candidate.canonicalize() {
        Ok(v) => v,
        Err(_) => return false,
    };
    if candidate_canon == root_canon {
        return false;
    }
    candidate_canon.starts_with(&root_canon)
}

fn system_time_to_rfc3339(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()
}

async fn get_team_checked(
    state: &AppState,
    team_id: &str,
    _user_id: &str,
) -> Result<crate::models::mongo::Team, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    team_service
        .get(team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))
}

async fn get_portal_by_id_or_slug_for_team(
    svc: &PortalService,
    team_id: &str,
    portal_ref: &str,
) -> anyhow::Result<crate::models::mongo::Portal> {
    if let Ok(portal) = svc.get(team_id, portal_ref).await {
        return Ok(portal);
    }

    let portal = svc.get_by_slug(portal_ref).await?;
    if portal.team_id.to_hex() != team_id {
        return Err(anyhow::anyhow!("Portal not found"));
    }
    if portal.is_deleted {
        return Err(anyhow::anyhow!("Portal already deleted"));
    }
    Ok(portal)
}
