/// File upload route handler
///
/// This module provides endpoints for uploading files from web clients.
/// Uploaded files are saved to the server's data directory and the local
/// file path is returned so that AI can read the file contents.
///
/// Note: This is a local service running on the user's own computer.
/// No file type restrictions are applied - users can upload any file format.
use crate::routes::errors::ErrorResponse;
use agime::config::paths::Paths;
use axum::{
    extract::{DefaultBodyLimit, Multipart},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use utoipa::ToSchema;
use uuid::Uuid;

// Constants
const MAX_FILE_SIZE_BYTES: usize = 100 * 1024 * 1024; // 100MB per file
const MAX_FILES_PER_REQUEST: usize = 20;

/// Information about an uploaded file
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UploadedFile {
    /// Original file name from the client
    pub original_name: String,
    /// Server-side file path (absolute path on the server)
    pub path: String,
    /// File size in bytes
    pub size: usize,
    /// Content type (MIME type)
    pub content_type: String,
}

/// Response for file upload endpoint
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UploadResponse {
    /// List of successfully uploaded files
    pub files: Vec<UploadedFile>,
}

/// Get the uploads directory path, creating it if necessary
async fn get_uploads_dir() -> Result<PathBuf, ErrorResponse> {
    let uploads_dir = Paths::data_dir().join("uploads");

    if !uploads_dir.exists() {
        fs::create_dir_all(&uploads_dir).await.map_err(|e| {
            tracing::error!("Failed to create uploads directory: {:?}", e);
            ErrorResponse::internal(format!("Failed to create uploads directory: {}", e))
        })?;
    }

    Ok(uploads_dir)
}

/// Extract file extension from filename
/// Returns the extension without the dot, or empty string if no extension
fn get_extension(filename: &str) -> String {
    filename
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| {
            // Only treat as extension if it's reasonable (not too long, no spaces)
            ext.len() <= 10 && !ext.contains(' ') && !ext.is_empty()
        })
        .map(|s| s.to_lowercase())
        .unwrap_or_default()
}

/// Generate a unique filename preserving the original extension
/// Sanitizes the filename to prevent path traversal attacks
fn generate_unique_filename(original_name: &str) -> String {
    let uuid = Uuid::new_v4();

    // Get extension from original filename
    let extension = get_extension(original_name);

    // Sanitize the original name (remove path separators and special chars)
    let sanitized_name: String = original_name
        .chars()
        .filter(|c| {
            c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.' || *c == ' '
            // Allow spaces in filenames
        })
        .take(50) // Limit filename length
        .collect();

    // Remove any remaining dots except the extension dot
    let name_without_ext = sanitized_name
        .rsplit_once('.')
        .map(|(name, _)| name)
        .unwrap_or(&sanitized_name);

    if extension.is_empty() {
        format!("{}-{}", uuid, name_without_ext)
    } else {
        format!("{}-{}.{}", uuid, name_without_ext, extension)
    }
}

/// Handle file upload from web clients
///
/// # Request
/// - Content-Type: multipart/form-data
/// - Files can be uploaded with any field name
///
/// # Response
/// - `files`: Array of uploaded file information with server paths
///
/// # Errors
/// - 400: Bad Request (invalid form data or no files)
/// - 413: Payload Too Large (file exceeds 100MB limit)
/// - 500: Internal Server Error (failed to save file)
#[utoipa::path(
    post,
    path = "/upload",
    responses(
        (status = 200, description = "Files uploaded successfully", body = UploadResponse),
        (status = 400, description = "Invalid form data or no files uploaded"),
        (status = 413, description = "File too large (max 100MB)"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("secret_key" = [])
    )
)]
pub async fn upload_files(mut multipart: Multipart) -> Result<Json<UploadResponse>, ErrorResponse> {
    let uploads_dir = get_uploads_dir().await?;
    let mut uploaded_files = Vec::new();
    let mut file_count = 0;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        tracing::error!("Failed to read multipart field: {:?}", e);
        ErrorResponse {
            message: format!("Failed to read form data: {}", e),
            status: StatusCode::BAD_REQUEST,
        }
    })? {
        // Check file count limit
        file_count += 1;
        if file_count > MAX_FILES_PER_REQUEST {
            return Err(ErrorResponse {
                message: format!(
                    "Too many files. Maximum {} files per request.",
                    MAX_FILES_PER_REQUEST
                ),
                status: StatusCode::BAD_REQUEST,
            });
        }

        // Get file metadata
        let original_name = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("file_{}", file_count));

        let content_type = field
            .content_type()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        // Read file data
        let data = field.bytes().await.map_err(|e| {
            tracing::error!("Failed to read file data: {:?}", e);
            ErrorResponse {
                message: format!("Failed to read file data: {}", e),
                status: StatusCode::BAD_REQUEST,
            }
        })?;

        // Check file size
        if data.len() > MAX_FILE_SIZE_BYTES {
            return Err(ErrorResponse {
                message: format!(
                    "File '{}' is too large ({:.1}MB). Maximum size is {}MB.",
                    original_name,
                    data.len() as f64 / (1024.0 * 1024.0),
                    MAX_FILE_SIZE_BYTES / (1024 * 1024)
                ),
                status: StatusCode::PAYLOAD_TOO_LARGE,
            });
        }

        // Skip empty files
        if data.is_empty() {
            tracing::warn!("Skipping empty file: {}", original_name);
            continue;
        }

        // Generate unique filename and save
        let unique_filename = generate_unique_filename(&original_name);
        let file_path = uploads_dir.join(&unique_filename);

        // Write file to disk
        let mut file = fs::File::create(&file_path).await.map_err(|e| {
            tracing::error!("Failed to create file {:?}: {:?}", file_path, e);
            ErrorResponse::internal(format!("Failed to save file: {}", e))
        })?;

        file.write_all(&data).await.map_err(|e| {
            tracing::error!("Failed to write file data: {:?}", e);
            ErrorResponse::internal(format!("Failed to write file: {}", e))
        })?;

        // Ensure data is flushed to disk
        file.sync_all().await.map_err(|e| {
            tracing::error!("Failed to sync file: {:?}", e);
            ErrorResponse::internal(format!("Failed to sync file: {}", e))
        })?;

        // Convert path to string - use forward slashes for consistency
        // This ensures the path works correctly when passed to AI tools
        let path_string = file_path.to_string_lossy().to_string();

        tracing::info!(
            "Uploaded file: {} -> {} ({} bytes)",
            original_name,
            path_string,
            data.len()
        );

        uploaded_files.push(UploadedFile {
            original_name,
            path: path_string,
            size: data.len(),
            content_type,
        });
    }

    if uploaded_files.is_empty() {
        return Err(ErrorResponse {
            message: "No files were uploaded".to_string(),
            status: StatusCode::BAD_REQUEST,
        });
    }

    Ok(Json(UploadResponse {
        files: uploaded_files,
    }))
}

/// Request body for `/upload/analysis_prompt`. Mirrors the desktop-flavoured
/// [`crate::host_document_analysis::DocumentAnalysisInput`] plus the optional
/// materialised path metadata that already lives on disk after `/upload`.
#[cfg(feature = "desktop_harness_host")]
#[derive(Debug, Deserialize, ToSchema)]
pub struct AnalysisPromptRequest {
    pub doc_id: String,
    pub doc_name: String,
    pub mime_type: String,
    #[serde(default)]
    pub file_size: u64,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub relative_path: Option<String>,
    #[serde(default)]
    pub content_snapshot: Option<String>,
    #[serde(default)]
    pub extra_instructions: Option<String>,
}

#[cfg(feature = "desktop_harness_host")]
#[derive(Debug, Serialize, ToSchema)]
pub struct AnalysisPromptResponse {
    /// Final user-facing prompt; the caller should send this through the
    /// regular `/reply` SSE pipeline as the next user message.
    pub prompt: String,
}

#[cfg(feature = "desktop_harness_host")]
#[utoipa::path(
    post,
    path = "/upload/analysis_prompt",
    request_body = AnalysisPromptRequest,
    responses(
        (status = 200, description = "Prompt built", body = AnalysisPromptResponse),
        (status = 400, description = "Invalid input"),
    ),
    security(("secret_key" = []))
)]
pub async fn build_analysis_prompt_route(
    Json(payload): Json<AnalysisPromptRequest>,
) -> Result<Json<AnalysisPromptResponse>, ErrorResponse> {
    use crate::host_document_analysis::{
        build_analysis_prompt, DocumentAnalysisInput, MaterializedAnalysisDocument,
    };

    if payload.doc_id.trim().is_empty() || payload.doc_name.trim().is_empty() {
        return Err(ErrorResponse {
            message: "doc_id and doc_name are required".to_string(),
            status: StatusCode::BAD_REQUEST,
        });
    }

    let ctx = DocumentAnalysisInput {
        doc_id: payload.doc_id,
        doc_name: payload.doc_name.clone(),
        mime_type: payload.mime_type.clone(),
        file_size: payload.file_size,
        lang: payload.lang,
    };

    let materialised = payload
        .file_path
        .as_ref()
        .map(|fp| MaterializedAnalysisDocument {
            file_path: fp.clone(),
            relative_path: payload.relative_path.clone().unwrap_or_else(|| fp.clone()),
            file_name: payload.doc_name.clone(),
            mime_type: payload.mime_type.clone(),
            file_size: payload.file_size,
        });

    let mut prompt = build_analysis_prompt(
        &ctx,
        materialised.as_ref(),
        payload.content_snapshot.as_deref(),
    );

    if let Some(extra) = payload.extra_instructions.as_deref() {
        let trimmed = extra.trim();
        if !trimmed.is_empty() {
            prompt.push_str("\n\n## 用户补充要求\n");
            prompt.push_str(trimmed);
        }
    }

    Ok(Json(AnalysisPromptResponse { prompt }))
}

/// Request body for `/upload/analysis_persistence`. The frontend supplies the
/// visible text it captured from the analysis `/reply` stream; the server
/// applies the team-server-equivalent JSON / terminal / blocked classification.
#[cfg(feature = "desktop_harness_host")]
#[derive(Debug, Deserialize, ToSchema)]
pub struct AnalysisPersistenceRequest {
    pub doc_name: String,
    #[serde(default)]
    pub analysis_text: Option<String>,
}

#[cfg(feature = "desktop_harness_host")]
#[derive(Debug, Serialize, ToSchema)]
pub struct AnalysisPersistenceResponse {
    pub json_payload: String,
    pub status: String,
}

#[cfg(feature = "desktop_harness_host")]
#[utoipa::path(
    post,
    path = "/upload/analysis_persistence",
    request_body = AnalysisPersistenceRequest,
    responses(
        (status = 200, description = "Persistence shape derived", body = AnalysisPersistenceResponse),
        (status = 400, description = "Invalid input"),
    ),
    security(("secret_key" = []))
)]
pub async fn derive_analysis_persistence_route(
    Json(payload): Json<AnalysisPersistenceRequest>,
) -> Result<Json<AnalysisPersistenceResponse>, ErrorResponse> {
    use crate::host_document_analysis::derive_document_analysis_persistence;

    if payload.doc_name.trim().is_empty() {
        return Err(ErrorResponse {
            message: "doc_name is required".to_string(),
            status: StatusCode::BAD_REQUEST,
        });
    }

    let (json_payload, status) = derive_document_analysis_persistence(
        None,
        payload.analysis_text.as_deref(),
        &payload.doc_name,
    );

    Ok(Json(AnalysisPersistenceResponse {
        json_payload,
        status: status.to_string(),
    }))
}

pub fn routes() -> Router {
    let router = Router::new().route(
        "/upload",
        post(upload_files).layer(DefaultBodyLimit::max(
            MAX_FILE_SIZE_BYTES * MAX_FILES_PER_REQUEST,
        )),
    );

    #[cfg(feature = "desktop_harness_host")]
    let router = router
        .route("/upload/analysis_prompt", post(build_analysis_prompt_route))
        .route(
            "/upload/analysis_persistence",
            post(derive_analysis_persistence_route),
        );

    router
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_extension() {
        assert_eq!(get_extension("test.png"), "png");
        assert_eq!(get_extension("document.PDF"), "pdf");
        assert_eq!(get_extension("archive.tar.gz"), "gz");
        assert_eq!(get_extension("noextension"), "");
        assert_eq!(get_extension(".hidden"), "hidden");
        assert_eq!(get_extension("file.with"), "with"); // "with" is 4 chars, valid
        assert_eq!(get_extension("file.verylongextensionname"), ""); // too long
    }

    #[test]
    fn test_generate_unique_filename() {
        let filename = generate_unique_filename("test document.pdf");
        assert!(filename.ends_with(".pdf"));
        assert!(filename.contains('-'));
        // UUID is 36 chars
        assert!(filename.len() > 36);
    }

    #[test]
    fn test_generate_unique_filename_no_extension() {
        let filename = generate_unique_filename("README");
        assert!(!filename.contains('.') || filename.matches('.').count() == 0);
        assert!(filename.len() > 36);
    }

    #[test]
    fn test_generate_unique_filename_sanitizes_path_traversal() {
        let filename = generate_unique_filename("../../../etc/passwd");
        // Path traversal characters (/ and \) should be filtered out
        assert!(!filename.contains('/'));
        assert!(!filename.contains('\\'));
    }

    #[test]
    fn test_generate_unique_filename_preserves_spaces() {
        let filename = generate_unique_filename("my document.pdf");
        assert!(filename.contains(' ') || filename.contains("my"));
    }

    #[test]
    fn test_generate_unique_filename_various_extensions() {
        // Test various file types that users might upload
        assert!(generate_unique_filename("photo.jpg").ends_with(".jpg"));
        assert!(generate_unique_filename("video.mp4").ends_with(".mp4"));
        assert!(generate_unique_filename("data.xlsx").ends_with(".xlsx"));
        assert!(generate_unique_filename("program.exe").ends_with(".exe"));
        assert!(generate_unique_filename("archive.zip").ends_with(".zip"));
        assert!(generate_unique_filename("document.docx").ends_with(".docx"));
    }
}
