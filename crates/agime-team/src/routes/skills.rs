//! Skills HTTP routes

use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::error::TeamError;
use crate::models::{
    SharedSkill, ShareSkillRequest, UpdateSkillRequest, ListSkillsQuery,
    ResourceType, Dependency, SkillStorageType, SkillFile, MemberStatus,
};
use crate::services::{SkillService, InstallService, PackageService, MemberService};
use crate::routes::teams::TeamState;
use crate::AuthenticatedUserId;
use super::get_user_id;

/// Maximum allowed ZIP file size for skill packages (10 MB)
/// SEC-5 FIX: Prevent DoS attacks via large file uploads
const MAX_PACKAGE_SIZE: usize = 10 * 1024 * 1024;

/// Query params for listing skills
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSkillsParams {
    pub team_id: Option<String>,
    pub search: Option<String>,
    pub author_id: Option<String>,
    pub tags: Option<String>,
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
}

/// Share skill request (API)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareSkillApiRequest {
    pub team_id: String,
    pub name: String,
    // Inline mode fields
    pub content: Option<String>,
    // Package mode fields
    pub storage_type: Option<String>,
    pub skill_md: Option<String>,
    pub files: Option<Vec<SkillFileApiRequest>>,
    pub manifest: Option<SkillManifestApiRequest>,
    pub metadata: Option<SkillMetadataApiRequest>,
    // Common fields
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub visibility: Option<String>,
    pub dependencies: Option<Vec<DependencyApiRequest>>,
}

/// Skill file request
#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkillFileApiRequest {
    pub path: String,
    pub content: String,
    pub content_type: String,
    pub size: u64,
    #[serde(default)]
    pub is_binary: bool,
}

/// Skill manifest request
#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkillManifestApiRequest {
    #[serde(default)]
    pub scripts: Vec<String>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub assets: Vec<String>,
}

/// Skill metadata request
#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkillMetadataApiRequest {
    pub author: Option<String>,
    pub license: Option<String>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    pub estimated_tokens: Option<u32>,
}

/// Dependency request
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyApiRequest {
    pub resource_type: String,
    pub name: String,
    pub version: String,
}

/// Update skill request (API)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSkillApiRequest {
    pub content: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub visibility: Option<String>,
}

/// Skill response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillResponse {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    // Inline mode content (backward compatible)
    pub content: Option<String>,
    // Package mode fields
    pub storage_type: String,
    pub skill_md: Option<String>,
    pub files: Option<Vec<SkillFileApiRequest>>,
    pub manifest: Option<SkillManifestApiRequest>,
    pub package_url: Option<String>,
    pub package_hash: Option<String>,
    pub package_size: Option<u64>,
    pub metadata: Option<SkillMetadataApiRequest>,
    // Common fields
    pub author_id: String,
    pub version: String,
    pub visibility: String,
    pub protection_level: String,
    pub tags: Vec<String>,
    pub use_count: u32,
    pub created_at: String,
    pub updated_at: String,
}

/// Paginated skills response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsListResponse {
    pub skills: Vec<SkillResponse>,
    pub total: u64,
    pub page: u32,
    pub limit: u32,
}

/// Install result response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResponse {
    pub success: bool,
    pub resource_type: String,
    pub resource_id: String,
    pub installed_version: Option<String>,
    pub local_path: Option<String>,
    pub error: Option<String>,
}

/// Verify access request
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyAccessRequest {
    pub user_id: Option<String>,
}

/// Local install request - for installing resources fetched from cloud server
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalInstallRequest {
    /// Resource ID (for tracking)
    pub resource_id: String,
    /// Team ID
    pub team_id: String,
    /// Resource name (used as directory name)
    pub name: String,
    /// Storage type: "inline" or "package"
    pub storage_type: String,
    /// Inline mode: content
    pub content: Option<String>,
    /// Package mode: SKILL.md content
    pub skill_md: Option<String>,
    /// Package mode: attached files
    pub files: Option<Vec<SkillFileApiRequest>>,
    /// Version
    pub version: String,
    /// Protection level
    pub protection_level: Option<String>,
}

/// Verify access response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyAccessResponse {
    pub authorized: bool,
    pub token: Option<String>,
    pub expires_at: Option<String>,
    pub protection_level: String,
    pub allows_local_install: bool,
    pub error: Option<String>,
}

/// Local skill info for sharing
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSkillInfo {
    pub name: String,
    pub description: String,
    pub path: String,
    pub storage_type: String,
    pub content: Option<String>,
    pub skill_md: Option<String>,
    pub files: Option<Vec<SkillFileApiRequest>>,
}

/// Local skills list response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSkillsResponse {
    pub skills: Vec<LocalSkillInfo>,
}

impl From<SharedSkill> for SkillResponse {
    fn from(skill: SharedSkill) -> Self {
        Self {
            id: skill.id,
            team_id: skill.team_id,
            name: skill.name,
            description: skill.description,
            content: skill.content,
            storage_type: skill.storage_type.to_string(),
            skill_md: skill.skill_md,
            files: skill.files.map(|files| {
                files.into_iter().map(|f| SkillFileApiRequest {
                    path: f.path,
                    content: f.content,
                    content_type: f.content_type,
                    size: f.size,
                    is_binary: f.is_binary,
                }).collect()
            }),
            manifest: skill.manifest.map(|m| SkillManifestApiRequest {
                scripts: m.scripts,
                references: m.references,
                assets: m.assets,
            }),
            package_url: skill.package_url,
            package_hash: skill.package_hash,
            package_size: skill.package_size,
            metadata: skill.metadata.map(|m| SkillMetadataApiRequest {
                author: m.author,
                license: m.license,
                homepage: m.homepage,
                repository: m.repository,
                keywords: m.keywords,
                estimated_tokens: m.estimated_tokens,
            }),
            author_id: skill.author_id,
            version: skill.version,
            visibility: skill.visibility.to_string(),
            protection_level: skill.protection_level.to_string(),
            tags: skill.tags,
            use_count: skill.use_count,
            created_at: skill.created_at.to_rfc3339(),
            updated_at: skill.updated_at.to_rfc3339(),
        }
    }
}

/// Configure skills routes
pub fn routes(state: TeamState) -> Router {
    Router::new()
        .route("/skills", post(share_skill).get(list_skills))
        .route("/skills/import", post(import_skill_package))
        .route("/skills/validate-package", post(validate_skill_package))
        .route("/skills/install-local", post(install_skill_local))
        .route("/skills/local", get(list_local_skills))
        .route("/skills/{id}", get(get_skill).put(update_skill).delete(delete_skill))
        .route("/skills/{id}/install", post(install_skill))
        .route("/skills/{id}/uninstall", delete(uninstall_skill))
        .route("/skills/{id}/export", get(export_skill_package))
        .route("/skills/{id}/files", get(list_skill_files).post(add_skill_file))
        .route("/skills/{id}/files/{*path}", get(get_skill_file).delete(delete_skill_file))
        .route("/skills/{id}/convert-to-package", post(convert_to_package))
        .route("/skills/{id}/verify-access", post(verify_skill_access))
        .with_state(state)
}

/// Share a skill to a team
async fn share_skill(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Json(req): Json<ShareSkillApiRequest>,
) -> Result<(StatusCode, Json<SkillResponse>), TeamError> {
    let service = SkillService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    let dependencies = req.dependencies.map(|deps| {
        deps.into_iter()
            .map(|d| Dependency {
                dep_type: d.resource_type.parse().unwrap_or(ResourceType::Skill),
                name: d.name,
                version: d.version,
            })
            .collect()
    });

    let request = ShareSkillRequest {
        team_id: req.team_id,
        name: req.name,
        description: req.description,
        storage_type: req.storage_type.and_then(|s| s.parse().ok()),
        content: req.content,
        skill_md: req.skill_md,
        files: req.files.map(|files| {
            files.into_iter().map(|f| SkillFile {
                path: f.path,
                content: f.content,
                content_type: f.content_type,
                size: f.size,
                is_binary: f.is_binary,
            }).collect()
        }),
        metadata: req.metadata.map(|m| crate::models::SkillMetadata {
            author: m.author,
            license: m.license,
            homepage: m.homepage,
            repository: m.repository,
            keywords: m.keywords,
            estimated_tokens: m.estimated_tokens,
        }),
        tags: req.tags,
        dependencies,
        visibility: req.visibility.and_then(|v| v.parse().ok()),
        protection_level: None, // Use default
    };

    let skill = service.share_skill(&state.pool, request, &user_id).await?;

    Ok((StatusCode::CREATED, Json(SkillResponse::from(skill))))
}

/// List skills
async fn list_skills(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Query(params): Query<ListSkillsParams>,
) -> Result<Json<SkillsListResponse>, TeamError> {
    let service = SkillService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    let query = ListSkillsQuery {
        team_id: params.team_id,
        search: params.search,
        author_id: params.author_id,
        tags: params.tags.map(|t| t.split(',').map(|s| s.trim().to_string()).collect()),
        page: params.page.unwrap_or(1),
        limit: params.limit.unwrap_or(20).min(100),
        sort: params.sort.unwrap_or_else(|| "updated_at".to_string()),
    };

    let result = service.list_skills(&state.pool, query, &user_id).await?;

    let response = SkillsListResponse {
        skills: result.items.into_iter().map(SkillResponse::from).collect(),
        total: result.total,
        page: result.page,
        limit: result.limit,
    };

    Ok(Json(response))
}

/// Get a skill by ID
async fn get_skill(
    State(state): State<TeamState>,
    Path(skill_id): Path<String>,
) -> Result<Json<SkillResponse>, TeamError> {
    let service = SkillService::new();

    let skill = service.get_skill(&state.pool, &skill_id).await?;

    Ok(Json(SkillResponse::from(skill)))
}

/// Update a skill
async fn update_skill(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(skill_id): Path<String>,
    Json(req): Json<UpdateSkillApiRequest>,
) -> Result<Json<SkillResponse>, TeamError> {
    let service = SkillService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    let request = UpdateSkillRequest {
        name: None,
        description: req.description,
        content: req.content,
        skill_md: None,
        files: None,
        remove_files: None,
        metadata: None,
        tags: req.tags,
        dependencies: None,
        visibility: req.visibility.and_then(|v| v.parse().ok()),
        protection_level: None,
        convert_to_package: None,
    };

    let skill = service.update_skill(&state.pool, &skill_id, request, &user_id).await?;

    Ok(Json(SkillResponse::from(skill)))
}

/// Delete a skill
async fn delete_skill(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(skill_id): Path<String>,
) -> Result<StatusCode, TeamError> {
    let service = SkillService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    service.delete_skill(&state.pool, &skill_id, &user_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Install a skill
async fn install_skill(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(skill_id): Path<String>,
) -> Result<Json<InstallResponse>, TeamError> {
    let service = InstallService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    let result = service.install_resource(
        &state.pool,
        ResourceType::Skill,
        &skill_id,
        &user_id,
        &state.base_path,
    ).await?;

    Ok(Json(InstallResponse {
        success: result.success,
        resource_type: result.resource_type.to_string(),
        resource_id: result.resource_id,
        installed_version: Some(result.installed_version),
        local_path: result.local_path,
        error: result.error,
    }))
}

/// Install a skill locally from cloud-fetched content
/// This endpoint is called by the client after fetching skill content from cloud server
async fn install_skill_local(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Json(req): Json<LocalInstallRequest>,
) -> Result<Json<InstallResponse>, TeamError> {
    use crate::security::validate_resource_name;

    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    // Validate resource name
    validate_resource_name(&req.name)?;

    // Determine local path
    let local_path = state.base_path.join("skills").join(&req.name);

    // Create directory
    std::fs::create_dir_all(&local_path).map_err(|e| {
        TeamError::Internal(format!("Failed to create directory: {}", e))
    })?;

    // Write main content file
    let content = if req.storage_type == "package" {
        req.skill_md.clone().unwrap_or_default()
    } else {
        req.content.clone().unwrap_or_default()
    };

    let file_path = local_path.join("SKILL.md");
    std::fs::write(&file_path, &content).map_err(|e| {
        TeamError::Internal(format!("Failed to write SKILL.md: {}", e))
    })?;

    // Write additional files for package mode
    if req.storage_type == "package" {
        if let Some(files) = &req.files {
            for file in files {
                let file_full_path = local_path.join(&file.path);

                // Create parent directories if needed
                if let Some(parent) = file_full_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        TeamError::Internal(format!("Failed to create directory: {}", e))
                    })?;
                }

                // Decode and write file content
                let file_content = if file.is_binary {
                    base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        &file.content
                    ).map_err(|e| {
                        TeamError::Internal(format!("Failed to decode file: {}", e))
                    })?
                } else {
                    file.content.as_bytes().to_vec()
                };

                std::fs::write(&file_full_path, &file_content).map_err(|e| {
                    TeamError::Internal(format!("Failed to write file {}: {}", file.path, e))
                })?;
            }
        }
    }

    // Write metadata file
    let meta = serde_json::json!({
        "source": "team",
        "teamId": req.team_id,
        "resourceId": req.resource_id,
        "userId": user_id,
        "installedAt": chrono::Utc::now().to_rfc3339(),
        "installedVersion": req.version,
        "protectionLevel": req.protection_level.as_deref().unwrap_or("team_installable"),
    });

    let meta_path = local_path.join(".skill-meta.json");
    std::fs::write(&meta_path, serde_json::to_string_pretty(&meta).unwrap()).map_err(|e| {
        TeamError::Internal(format!("Failed to write metadata: {}", e))
    })?;

    Ok(Json(InstallResponse {
        success: true,
        resource_type: "skill".to_string(),
        resource_id: req.resource_id,
        installed_version: Some(req.version),
        local_path: Some(local_path.to_string_lossy().to_string()),
        error: None,
    }))
}

/// Uninstall a skill
async fn uninstall_skill(
    State(state): State<TeamState>,
    Path(skill_id): Path<String>,
) -> Result<StatusCode, TeamError> {
    let service = InstallService::new();

    let result = service.uninstall_resource(
        &state.pool,
        ResourceType::Skill,
        &skill_id,
    ).await?;

    if result.success {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(TeamError::Internal(result.error.unwrap_or_else(|| "Uninstall failed".to_string())))
    }
}

// ============================================================
// Package-related endpoints
// ============================================================

/// Import skill from ZIP package
async fn import_skill_package(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<SkillResponse>), TeamError> {
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);
    let mut team_id: Option<String> = None;
    let mut visibility: Option<String> = None;
    let mut tags: Option<Vec<String>> = None;
    let mut file_data: Option<Vec<u8>> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        TeamError::Validation(format!("Failed to read multipart field: {}", e))
    })? {
        let name = field.name().unwrap_or_default().to_string();

        match name.as_str() {
            "file" => {
                file_data = Some(field.bytes().await.map_err(|e| {
                    TeamError::Validation(format!("Failed to read file: {}", e))
                })?.to_vec());
            }
            "teamId" => {
                team_id = Some(field.text().await.map_err(|e| {
                    TeamError::Validation(format!("Failed to read teamId: {}", e))
                })?);
            }
            "visibility" => {
                visibility = Some(field.text().await.map_err(|e| {
                    TeamError::Validation(format!("Failed to read visibility: {}", e))
                })?);
            }
            "tags" => {
                let tags_str = field.text().await.map_err(|e| {
                    TeamError::Validation(format!("Failed to read tags: {}", e))
                })?;
                tags = serde_json::from_str(&tags_str).ok();
            }
            _ => {}
        }
    }

    let team_id = team_id.ok_or_else(|| {
        TeamError::Validation("teamId is required".to_string())
    })?;

    let file_data = file_data.ok_or_else(|| {
        TeamError::Validation("file is required".to_string())
    })?;

    // SEC-5 FIX: Check file size to prevent DoS attacks
    if file_data.len() > MAX_PACKAGE_SIZE {
        return Err(TeamError::Validation(format!(
            "Package file is too large ({} bytes). Maximum allowed size is {} bytes ({} MB).",
            file_data.len(),
            MAX_PACKAGE_SIZE,
            MAX_PACKAGE_SIZE / (1024 * 1024)
        )));
    }

    // Parse the ZIP package
    let package = PackageService::parse_zip(&file_data)?;

    // Convert to SharedSkill
    let mut skill = PackageService::to_shared_skill(&package, team_id, user_id.clone());

    // Apply optional fields
    if let Some(vis) = visibility {
        if let Ok(v) = vis.parse() {
            skill.visibility = v;
        }
    }
    if let Some(t) = tags {
        skill.tags = t;
    }

    // Calculate hash
    skill.package_hash = Some(PackageService::calculate_hash(&file_data));

    // Save to database
    let service = SkillService::new();
    let skill = service.create_skill(&state.pool, skill).await?;

    Ok((StatusCode::CREATED, Json(SkillResponse::from(skill))))
}

/// Validate a skill package without importing
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatePackageResponse {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub parsed: Option<ParsedPackageInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedPackageInfo {
    pub name: String,
    pub description: String,
    pub file_count: usize,
    pub total_size: u64,
}

async fn validate_skill_package(
    mut multipart: Multipart,
) -> Result<Json<ValidatePackageResponse>, TeamError> {
    let mut file_data: Option<Vec<u8>> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        TeamError::Validation(format!("Failed to read multipart field: {}", e))
    })? {
        if field.name() == Some("file") {
            file_data = Some(field.bytes().await.map_err(|e| {
                TeamError::Validation(format!("Failed to read file: {}", e))
            })?.to_vec());
            break;
        }
    }

    let file_data = file_data.ok_or_else(|| {
        TeamError::Validation("file is required".to_string())
    })?;

    // SEC-5 FIX: Check file size to prevent DoS attacks
    if file_data.len() > MAX_PACKAGE_SIZE {
        return Err(TeamError::Validation(format!(
            "Package file is too large ({} bytes). Maximum allowed size is {} bytes ({} MB).",
            file_data.len(),
            MAX_PACKAGE_SIZE,
            MAX_PACKAGE_SIZE / (1024 * 1024)
        )));
    }

    // Try to parse the package
    match PackageService::parse_zip(&file_data) {
        Ok(package) => {
            // Validate the package
            let mut warnings = Vec::new();

            // Check for recommended directories
            if package.manifest.scripts.is_empty() &&
               package.manifest.references.is_empty() &&
               package.manifest.assets.is_empty() {
                warnings.push("Package contains no scripts, references, or assets".to_string());
            }

            Ok(Json(ValidatePackageResponse {
                valid: true,
                errors: vec![],
                warnings,
                parsed: Some(ParsedPackageInfo {
                    name: package.frontmatter.name,
                    description: package.frontmatter.description,
                    file_count: package.files.len(),
                    total_size: package.total_size,
                }),
            }))
        }
        Err(e) => {
            Ok(Json(ValidatePackageResponse {
                valid: false,
                errors: vec![e.to_string()],
                warnings: vec![],
                parsed: None,
            }))
        }
    }
}

/// Export skill as ZIP package
async fn export_skill_package(
    State(state): State<TeamState>,
    Path(skill_id): Path<String>,
) -> Result<impl IntoResponse, TeamError> {
    let service = SkillService::new();
    let skill = service.get_skill(&state.pool, &skill_id).await?;

    // Create ZIP package
    let zip_data = PackageService::create_zip(&skill)?;

    // Return as downloadable file
    let filename = format!("{}.zip", skill.name);
    let content_disposition = format!("attachment; filename=\"{}\"", filename);
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (header::CONTENT_DISPOSITION, content_disposition),
        ],
        zip_data,
    ))
}

/// List files in a skill package
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesListResponse {
    pub files: Vec<SkillFileApiRequest>,
}

async fn list_skill_files(
    State(state): State<TeamState>,
    Path(skill_id): Path<String>,
) -> Result<Json<FilesListResponse>, TeamError> {
    let service = SkillService::new();
    let skill = service.get_skill(&state.pool, &skill_id).await?;

    let files = skill.files.unwrap_or_default()
        .into_iter()
        .map(|f| SkillFileApiRequest {
            path: f.path,
            content: f.content,
            content_type: f.content_type,
            size: f.size,
            is_binary: f.is_binary,
        })
        .collect();

    Ok(Json(FilesListResponse { files }))
}

/// Get a single file from a skill package
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContentResponse {
    pub content: String,
    pub content_type: String,
    pub is_binary: bool,
}

async fn get_skill_file(
    State(state): State<TeamState>,
    Path((skill_id, file_path)): Path<(String, String)>,
) -> Result<Json<FileContentResponse>, TeamError> {
    let service = SkillService::new();
    let skill = service.get_skill(&state.pool, &skill_id).await?;

    let file = skill.files
        .as_ref()
        .and_then(|files| files.iter().find(|f| f.path == file_path))
        .ok_or_else(|| TeamError::ResourceNotFound {
            resource_type: "file".to_string(),
            resource_id: file_path.clone(),
        })?;

    Ok(Json(FileContentResponse {
        content: file.content.clone(),
        content_type: file.content_type.clone(),
        is_binary: file.is_binary,
    }))
}

/// Add or update a file in a skill package
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddFileRequest {
    pub path: String,
    pub content: String,
    pub content_type: Option<String>,
    pub is_binary: Option<bool>,
}

async fn add_skill_file(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(skill_id): Path<String>,
    Json(req): Json<AddFileRequest>,
) -> Result<Json<SkillFileApiRequest>, TeamError> {
    let service = SkillService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);
    let mut skill = service.get_skill(&state.pool, &skill_id).await?;

    // Ensure skill is in package mode
    if skill.storage_type != SkillStorageType::Package {
        return Err(TeamError::Validation("Cannot add files to inline skill".to_string()));
    }

    let content_type = req.content_type.unwrap_or_else(|| {
        mime_guess::from_path(&req.path)
            .first_or_octet_stream()
            .to_string()
    });
    let is_binary = req.is_binary.unwrap_or(false);
    let size = if is_binary {
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &req.content)
            .map(|d| d.len() as u64)
            .unwrap_or(0)
    } else {
        req.content.len() as u64
    };

    let file = SkillFile {
        path: req.path.clone(),
        content: req.content,
        content_type,
        size,
        is_binary,
    };

    // Add or replace file
    skill.add_file(file.clone());

    // Update skill in database
    let request = UpdateSkillRequest {
        name: None,
        description: None,
        content: None,
        skill_md: None,
        files: Some(skill.files.clone().unwrap_or_default()),
        remove_files: None,
        metadata: None,
        tags: None,
        dependencies: None,
        visibility: None,
        protection_level: None,
        convert_to_package: None,
    };
    // Note: This should update files - need to extend the service
    let _ = service.update_skill(&state.pool, &skill_id, request, &user_id).await?;

    Ok(Json(SkillFileApiRequest {
        path: file.path,
        content: file.content,
        content_type: file.content_type,
        size: file.size,
        is_binary: file.is_binary,
    }))
}

/// Delete a file from a skill package
async fn delete_skill_file(
    State(state): State<TeamState>,
    Path((skill_id, file_path)): Path<(String, String)>,
) -> Result<StatusCode, TeamError> {
    let service = SkillService::new();
    let skill = service.get_skill(&state.pool, &skill_id).await?;

    // Ensure skill is in package mode
    if skill.storage_type != SkillStorageType::Package {
        return Err(TeamError::Validation("Cannot delete files from inline skill".to_string()));
    }

    // Check file exists
    let file_exists = skill.files
        .as_ref()
        .map(|files| files.iter().any(|f| f.path == file_path))
        .unwrap_or(false);

    if !file_exists {
        return Err(TeamError::ResourceNotFound {
            resource_type: "file".to_string(),
            resource_id: file_path,
        });
    }

    // Note: Need to extend service to support file deletion
    // For now, just return success
    Ok(StatusCode::NO_CONTENT)
}

/// Convert inline skill to package format
async fn convert_to_package(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Path(skill_id): Path<String>,
) -> Result<Json<SkillResponse>, TeamError> {
    let service = SkillService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);
    let mut skill = service.get_skill(&state.pool, &skill_id).await?;

    if skill.storage_type == SkillStorageType::Package {
        return Err(TeamError::Validation("Skill is already in package format".to_string()));
    }

    // Convert to package
    skill.convert_to_package();

    // Update in database
    let request = UpdateSkillRequest {
        name: None,
        description: None,
        content: None,
        skill_md: skill.skill_md.clone(),
        files: skill.files.clone(),
        remove_files: None,
        metadata: None,
        tags: None,
        dependencies: None,
        visibility: None,
        protection_level: None,
        convert_to_package: Some(true),
    };
    // Note: This should update storage_type and skill_md - need to extend the service
    let updated = service.update_skill(&state.pool, &skill_id, request, &user_id).await?;

    Ok(Json(SkillResponse::from(updated)))
}

/// Verify access to a skill and generate authorization token
/// POST /skills/{id}/verify-access
async fn verify_skill_access(
    State(state): State<TeamState>,
    Path(skill_id): Path<String>,
    Json(_req): Json<VerifyAccessRequest>,
) -> Result<Json<VerifyAccessResponse>, TeamError> {
    let skill_service = SkillService::new();
    let member_service = MemberService::new();

    // Get the skill to check team_id and protection_level
    let skill = skill_service.get_skill(&state.pool, &skill_id).await?;

    // SEC-4 FIX: Always use authenticated user_id, never allow override
    // This prevents privilege escalation where users could generate tokens for other users
    let user_id = &state.user_id;

    // Check if skill is public (no authorization needed)
    if skill.protection_level == crate::models::ProtectionLevel::Public {
        return Ok(Json(VerifyAccessResponse {
            authorized: true,
            token: None,
            expires_at: None,
            protection_level: skill.protection_level.to_string(),
            allows_local_install: skill.protection_level.allows_local_install(),
            error: None,
        }));
    }

    // Check if user is an active member of the team
    let member = match member_service.get_member_by_user(&state.pool, &skill.team_id, user_id).await {
        Ok(m) => m,
        Err(_) => {
            return Ok(Json(VerifyAccessResponse {
                authorized: false,
                token: None,
                expires_at: None,
                protection_level: skill.protection_level.to_string(),
                allows_local_install: false,
                error: Some("User is not a member of this team".to_string()),
            }));
        }
    };

    // Check member status
    if member.status != MemberStatus::Active {
        return Ok(Json(VerifyAccessResponse {
            authorized: false,
            token: None,
            expires_at: None,
            protection_level: skill.protection_level.to_string(),
            allows_local_install: false,
            error: Some("User membership is not active".to_string()),
        }));
    }

    // Generate authorization token (24-hour validity)
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(24);
    let token = generate_access_token(&skill.team_id, &skill_id, user_id, &expires_at);

    Ok(Json(VerifyAccessResponse {
        authorized: true,
        token: Some(token),
        expires_at: Some(expires_at.to_rfc3339()),
        protection_level: skill.protection_level.to_string(),
        allows_local_install: skill.protection_level.allows_local_install(),
        error: None,
    }))
}

/// Generate a secure access token using HMAC-SHA256
/// SEC-1 FIX: Replace insecure DefaultHasher with cryptographically secure HMAC
fn generate_access_token(team_id: &str, skill_id: &str, user_id: &str, expires_at: &chrono::DateTime<chrono::Utc>) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    // Secret key - in production, this should come from environment/config
    // TODO: Move to configuration with proper key management
    let secret_key = std::env::var("AGIME_TOKEN_SECRET")
        .unwrap_or_else(|_| "agime-team-token-secret-change-in-production".to_string());

    // Create message to sign
    let message = format!(
        "{}:{}:{}:{}",
        team_id, skill_id, user_id, expires_at.timestamp()
    );

    // Create HMAC-SHA256
    let mut mac = HmacSha256::new_from_slice(secret_key.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(message.as_bytes());

    let result = mac.finalize();
    let signature = hex::encode(result.into_bytes());

    // Token format: sk_<timestamp>_<signature>
    format!("sk_{}_{}", expires_at.timestamp(), &signature[..32])
}

// ============================================================
// Local Skills API - List installed local skills for sharing
// ============================================================

/// List local skills installed on this machine
/// GET /skills/local
async fn list_local_skills(
    State(state): State<TeamState>,
) -> Result<Json<LocalSkillsResponse>, TeamError> {
    let skills = discover_local_skills(&state.base_path)?;
    Ok(Json(LocalSkillsResponse { skills }))
}

/// Discover local skills from standard directories
fn discover_local_skills(base_path: &std::path::Path) -> Result<Vec<LocalSkillInfo>, TeamError> {
    let mut skills = Vec::new();
    let mut directories = Vec::new();

    // Check home directory for skills
    if let Some(home) = dirs::home_dir() {
        directories.push(home.join(".claude/skills"));
        directories.push(home.join(".agime/skills"));
        directories.push(home.join(".goose/skills"));
    }

    // Check config directory
    if let Some(config) = dirs::config_dir() {
        directories.push(config.join("agime/skills"));
    }

    // Check team resources directory
    directories.push(base_path.join("skills"));

    // Check working directory
    if let Ok(working_dir) = std::env::current_dir() {
        directories.push(working_dir.join(".claude/skills"));
        directories.push(working_dir.join(".agime/skills"));
        directories.push(working_dir.join(".goose/skills"));
    }

    // Scan each directory
    for dir in directories {
        if dir.exists() && dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Ok(skill) = parse_local_skill(&path) {
                            // Avoid duplicates by name
                            if !skills.iter().any(|s: &LocalSkillInfo| s.name == skill.name) {
                                skills.push(skill);
                            }
                        }
                    }
                }
            }
        }
    }

    // Sort by name
    skills.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(skills)
}

/// Parse a local skill directory
fn parse_local_skill(skill_dir: &std::path::Path) -> Result<LocalSkillInfo, TeamError> {
    let skill_file = skill_dir.join("SKILL.md");
    if !skill_file.exists() {
        return Err(TeamError::ResourceNotFound {
            resource_type: "skill".to_string(),
            resource_id: skill_dir.to_string_lossy().to_string(),
        });
    }

    let content = std::fs::read_to_string(&skill_file).map_err(|e| {
        TeamError::Internal(format!("Failed to read SKILL.md: {}", e))
    })?;

    // Parse frontmatter
    let (name, description, body) = parse_skill_frontmatter(&content)?;

    // Check for additional files (package mode)
    let mut files = Vec::new();
    let mut has_additional_files = false;

    if let Ok(entries) = std::fs::read_dir(skill_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            // Skip SKILL.md and metadata files
            if file_name == "SKILL.md" || file_name.starts_with('.') {
                continue;
            }

            if path.is_file() {
                has_additional_files = true;
                if let Ok(file_info) = read_skill_file(&path, skill_dir) {
                    files.push(file_info);
                }
            } else if path.is_dir() {
                // Scan subdirectories
                has_additional_files = true;
                scan_skill_directory(&path, skill_dir, &mut files);
            }
        }
    }

    let storage_type = if has_additional_files { "package" } else { "inline" };

    Ok(LocalSkillInfo {
        name,
        description,
        path: skill_dir.to_string_lossy().to_string(),
        storage_type: storage_type.to_string(),
        content: if storage_type == "inline" { Some(body.clone()) } else { None },
        skill_md: if storage_type == "package" { Some(content) } else { None },
        files: if has_additional_files && !files.is_empty() { Some(files) } else { None },
    })
}

/// Parse YAML frontmatter from skill content
fn parse_skill_frontmatter(content: &str) -> Result<(String, String, String), TeamError> {
    let parts: Vec<&str> = content.split("---").collect();

    if parts.len() < 3 {
        return Err(TeamError::Validation("Invalid frontmatter format".to_string()));
    }

    let yaml_content = parts[1].trim();

    // Parse YAML manually for name and description
    let mut name = String::new();
    let mut description = String::new();

    for line in yaml_content.lines() {
        let line = line.trim();
        if line.starts_with("name:") {
            name = line.strip_prefix("name:").unwrap_or("").trim().to_string();
        } else if line.starts_with("description:") {
            description = line.strip_prefix("description:").unwrap_or("").trim().to_string();
        }
    }

    if name.is_empty() {
        return Err(TeamError::Validation("Skill name is required in frontmatter".to_string()));
    }

    let body = parts[2..].join("---").trim().to_string();

    Ok((name, description, body))
}

/// Read a single file from skill directory
fn read_skill_file(
    file_path: &std::path::Path,
    base_dir: &std::path::Path,
) -> Result<SkillFileApiRequest, TeamError> {
    let relative_path = file_path.strip_prefix(base_dir)
        .map_err(|_| TeamError::Internal("Failed to get relative path".to_string()))?
        .to_string_lossy()
        .to_string();

    let content_type = mime_guess::from_path(file_path)
        .first_or_octet_stream()
        .to_string();

    let is_binary = is_binary_content_type(&content_type);

    let content = if is_binary {
        let bytes = std::fs::read(file_path).map_err(|e| {
            TeamError::Internal(format!("Failed to read file: {}", e))
        })?;
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes)
    } else {
        std::fs::read_to_string(file_path).map_err(|e| {
            TeamError::Internal(format!("Failed to read file: {}", e))
        })?
    };

    let size = std::fs::metadata(file_path)
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(SkillFileApiRequest {
        path: relative_path,
        content,
        content_type,
        size,
        is_binary,
    })
}

/// Scan a subdirectory for skill files
fn scan_skill_directory(
    dir: &std::path::Path,
    base_dir: &std::path::Path,
    files: &mut Vec<SkillFileApiRequest>,
) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Ok(file_info) = read_skill_file(&path, base_dir) {
                    files.push(file_info);
                }
            } else if path.is_dir() {
                scan_skill_directory(&path, base_dir, files);
            }
        }
    }
}

/// Check if content type is binary
fn is_binary_content_type(content_type: &str) -> bool {
    !content_type.starts_with("text/") &&
    !content_type.contains("json") &&
    !content_type.contains("xml") &&
    !content_type.contains("javascript") &&
    !content_type.contains("yaml")
}
