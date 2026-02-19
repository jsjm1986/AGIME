//! Install service - unified installation for all resource types

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::path::PathBuf;

use crate::error::{TeamError, TeamResult};
use crate::models::{
    BatchInstallRequest, BatchInstallResult, CheckUpdatesRequest, CheckUpdatesResponse,
    InstallResult, InstalledResource, ProtectionLevel, ResourceType, UninstallResult, UpdateInfo,
};
use crate::security::validate_resource_name;
use crate::services::{ExtensionService, MemberService, RecipeService, SkillService};

/// Skill metadata file structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillMeta {
    /// Source of the skill: null for local, "team" for team skills
    pub source: Option<String>,
    /// Team ID if source is team
    pub team_id: Option<String>,
    /// Resource ID in the team database
    pub resource_id: Option<String>,
    /// User who installed this skill
    pub user_id: Option<String>,
    /// Installation timestamp
    pub installed_at: String,
    /// Installed version
    pub installed_version: String,
    /// Protection level
    pub protection_level: String,
    /// Authorization info
    pub authorization: Option<SkillAuthorization>,
}

/// Authorization info stored in meta file
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillAuthorization {
    /// Authorization token
    pub token: String,
    /// Token expiration time
    pub expires_at: String,
    /// Last verification time
    pub last_verified_at: String,
}

/// Install service
pub struct InstallService {
    skill_service: SkillService,
    recipe_service: RecipeService,
    extension_service: ExtensionService,
    member_service: MemberService,
}

impl InstallService {
    /// Create a new install service
    pub fn new() -> Self {
        Self {
            skill_service: SkillService::new(),
            recipe_service: RecipeService::new(),
            extension_service: ExtensionService::new(),
            member_service: MemberService::new(),
        }
    }

    /// Install a resource
    pub async fn install_resource(
        &self,
        pool: &SqlitePool,
        resource_type: ResourceType,
        resource_id: &str,
        user_id: &str,
        base_path: &PathBuf,
    ) -> TeamResult<InstallResult> {
        // Get resource info and check permission
        let (team_id, resource_name, version, content, protection_level) = match resource_type {
            ResourceType::Skill => {
                let skill = self.skill_service.get_skill(pool, resource_id).await?;
                let member = self
                    .member_service
                    .get_member_by_user(pool, &skill.team_id, user_id)
                    .await?;
                if !member.can_install_resources() {
                    return Err(TeamError::PermissionDenied {
                        action: "install skill".to_string(),
                    });
                }

                // Check protection level - deny installation for online-only and controlled
                if !skill.protection_level.allows_local_install() {
                    return Err(TeamError::Validation(format!(
                        "Skill with protection level '{}' cannot be installed locally. Use online access instead.",
                        skill.protection_level
                    )));
                }

                // Get effective content (SKILL.md for package, content for inline)
                let content = skill.get_effective_content().unwrap_or("").to_string();
                (
                    skill.team_id,
                    skill.name,
                    skill.version,
                    content,
                    skill.protection_level,
                )
            }
            ResourceType::Recipe => {
                let recipe = self.recipe_service.get_recipe(pool, resource_id).await?;
                let member = self
                    .member_service
                    .get_member_by_user(pool, &recipe.team_id, user_id)
                    .await?;
                if !member.can_install_resources() {
                    return Err(TeamError::PermissionDenied {
                        action: "install recipe".to_string(),
                    });
                }

                // Check protection level
                if !recipe.protection_level.allows_local_install() {
                    return Err(TeamError::Validation(format!(
                        "Recipe with protection level '{}' cannot be installed locally.",
                        recipe.protection_level
                    )));
                }

                (
                    recipe.team_id,
                    recipe.name,
                    recipe.version,
                    recipe.content_yaml,
                    recipe.protection_level,
                )
            }
            ResourceType::Extension => {
                let extension = self
                    .extension_service
                    .get_extension(pool, resource_id)
                    .await?;
                let member = self
                    .member_service
                    .get_member_by_user(pool, &extension.team_id, user_id)
                    .await?;
                if !member.can_install_resources() {
                    return Err(TeamError::PermissionDenied {
                        action: "install extension".to_string(),
                    });
                }

                // Check protection level
                if !extension.protection_level.allows_local_install() {
                    return Err(TeamError::Validation(format!(
                        "Extension with protection level '{}' cannot be installed locally.",
                        extension.protection_level
                    )));
                }

                let config_json = serde_json::to_string(&extension.config)?;
                (
                    extension.team_id,
                    extension.name,
                    extension.version,
                    config_json,
                    extension.protection_level,
                )
            }
        };

        // Validate resource name to prevent path traversal attacks
        validate_resource_name(&resource_name)?;

        // Determine local path - unified path (no "team" subdirectory)
        let type_dir = match resource_type {
            ResourceType::Skill => "skills",
            ResourceType::Recipe => "recipes",
            ResourceType::Extension => "extensions",
        };
        let local_path = base_path.join(type_dir).join(&resource_name);

        // Create directory and write content
        std::fs::create_dir_all(&local_path)?;

        let file_name = match resource_type {
            ResourceType::Skill => "SKILL.md",
            ResourceType::Recipe => "recipe.yaml",
            ResourceType::Extension => "extension.json",
        };
        let file_path = local_path.join(file_name);
        std::fs::write(&file_path, &content)?;

        // Generate authorization token (24-hour validity)
        let now = Utc::now();
        let expires_at = now + chrono::Duration::hours(24);
        let authorization = if protection_level.requires_authorization() {
            Some(SkillAuthorization {
                token: generate_access_token(&team_id, resource_id, user_id, &expires_at),
                expires_at: expires_at.to_rfc3339(),
                last_verified_at: now.to_rfc3339(),
            })
        } else {
            None
        };

        // Write .skill-meta.json file
        let meta = SkillMeta {
            source: Some("team".to_string()),
            team_id: Some(team_id.clone()),
            resource_id: Some(resource_id.to_string()),
            user_id: Some(user_id.to_string()),
            installed_at: now.to_rfc3339(),
            installed_version: version.clone(),
            protection_level: protection_level.to_string(),
            authorization: authorization.clone(),
        };
        let meta_path = local_path.join(".skill-meta.json");
        let meta_json = serde_json::to_string_pretty(&meta)?;
        std::fs::write(&meta_path, &meta_json)?;

        // Record installation with user_id and authorization info
        let installed = InstalledResource::new(
            resource_type,
            resource_id.to_string(),
            team_id.clone(),
            resource_name.clone(),
            version.clone(),
            Some(local_path.to_string_lossy().to_string()),
        );

        // Upsert into installed_resources with user_id and authorization fields
        sqlx::query(
            r#"
            INSERT INTO installed_resources (
                id, resource_type, resource_id, team_id, resource_name, local_path,
                installed_version, installed_at, user_id, authorization_token,
                authorization_expires_at, last_verified_at, protection_level
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(resource_type, resource_id) DO UPDATE SET
                installed_version = excluded.installed_version,
                local_path = excluded.local_path,
                has_update = 0,
                installed_at = excluded.installed_at,
                user_id = excluded.user_id,
                authorization_token = excluded.authorization_token,
                authorization_expires_at = excluded.authorization_expires_at,
                last_verified_at = excluded.last_verified_at,
                protection_level = excluded.protection_level
            "#,
        )
        .bind(&installed.id)
        .bind(resource_type.to_string())
        .bind(resource_id)
        .bind(&installed.team_id)
        .bind(&installed.resource_name)
        .bind(&installed.local_path)
        .bind(&version)
        .bind(installed.installed_at.to_rfc3339())
        .bind(user_id)
        .bind(authorization.as_ref().map(|a| &a.token))
        .bind(authorization.as_ref().map(|a| &a.expires_at))
        .bind(authorization.as_ref().map(|_| now.to_rfc3339()))
        .bind(protection_level.to_string())
        .execute(pool)
        .await?;

        // Increment use count
        match resource_type {
            ResourceType::Skill => {
                self.skill_service
                    .increment_use_count(pool, resource_id)
                    .await?;
            }
            ResourceType::Recipe => {
                self.recipe_service
                    .increment_use_count(pool, resource_id)
                    .await?;
            }
            ResourceType::Extension => {
                self.extension_service
                    .increment_use_count(pool, resource_id)
                    .await?;
            }
        }

        Ok(InstallResult::success(
            resource_type,
            resource_id.to_string(),
            version,
            Some(local_path.to_string_lossy().to_string()),
        ))
    }

    /// Uninstall a resource
    pub async fn uninstall_resource(
        &self,
        pool: &SqlitePool,
        resource_type: ResourceType,
        resource_id: &str,
    ) -> TeamResult<UninstallResult> {
        // Get installed resource
        let installed: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT id, local_path FROM installed_resources WHERE resource_type = ? AND resource_id = ?"
        )
        .bind(resource_type.to_string())
        .bind(resource_id)
        .fetch_optional(pool)
        .await?;

        match installed {
            Some((id, local_path)) => {
                // Delete local files if they exist
                if let Some(path) = local_path {
                    let path = PathBuf::from(path);
                    if path.exists() {
                        if path.is_dir() {
                            std::fs::remove_dir_all(&path)?;
                        } else {
                            std::fs::remove_file(&path)?;
                        }
                    }
                }

                // Remove from database
                sqlx::query("DELETE FROM installed_resources WHERE id = ?")
                    .bind(&id)
                    .execute(pool)
                    .await?;

                Ok(UninstallResult {
                    success: true,
                    error: None,
                })
            }
            None => Ok(UninstallResult {
                success: false,
                error: Some("Resource not installed".to_string()),
            }),
        }
    }

    /// Batch install resources
    pub async fn batch_install(
        &self,
        pool: &SqlitePool,
        request: BatchInstallRequest,
        user_id: &str,
        base_path: &PathBuf,
    ) -> TeamResult<BatchInstallResult> {
        let mut results = Vec::new();

        for resource_ref in request.resources {
            let result = self
                .install_resource(
                    pool,
                    resource_ref.resource_type,
                    &resource_ref.id,
                    user_id,
                    base_path,
                )
                .await;

            match result {
                Ok(install_result) => results.push(install_result),
                Err(e) => results.push(InstallResult::failure(
                    resource_ref.resource_type,
                    resource_ref.id,
                    e.to_string(),
                )),
            }
        }

        Ok(BatchInstallResult::new(results))
    }

    /// Check for updates
    pub async fn check_updates(
        &self,
        pool: &SqlitePool,
        request: CheckUpdatesRequest,
    ) -> TeamResult<CheckUpdatesResponse> {
        let mut updates = Vec::new();

        for resource_id in request.resource_ids {
            // Get installed info
            let installed: Option<(String, String, String, String)> = sqlx::query_as(
                r#"
                SELECT resource_type, resource_name, installed_version, team_id
                FROM installed_resources
                WHERE resource_id = ?
                "#,
            )
            .bind(&resource_id)
            .fetch_optional(pool)
            .await?;

            if let Some((type_str, name, installed_version, team_id)) = installed {
                let resource_type: ResourceType = type_str.parse().unwrap_or(ResourceType::Skill);

                // Get latest version
                let table = match resource_type {
                    ResourceType::Skill => "shared_skills",
                    ResourceType::Recipe => "shared_recipes",
                    ResourceType::Extension => "shared_extensions",
                };
                let query = format!(
                    "SELECT version FROM {} WHERE team_id = ? AND name = ? AND is_deleted = 0 ORDER BY created_at DESC LIMIT 1",
                    table
                );
                let latest_version: Option<(String,)> = sqlx::query_as(&query)
                    .bind(&team_id)
                    .bind(&name)
                    .fetch_optional(pool)
                    .await?;
                let latest_version = latest_version.map(|r| r.0);

                if let Some(latest) = latest_version {
                    let has_update = latest != installed_version;

                    // Update the has_update flag
                    sqlx::query(
                        "UPDATE installed_resources SET latest_version = ?, has_update = ?, last_checked_at = ? WHERE resource_id = ?"
                    )
                    .bind(&latest)
                    .bind(if has_update { 1i32 } else { 0i32 })
                    .bind(Utc::now().to_rfc3339())
                    .bind(&resource_id)
                    .execute(pool)
                    .await?;

                    updates.push(UpdateInfo {
                        resource_type,
                        resource_id: resource_id.clone(),
                        resource_name: name,
                        current_version: installed_version,
                        latest_version: latest,
                        has_update,
                    });
                }
            }
        }

        Ok(CheckUpdatesResponse { updates })
    }

    /// List all installed resources
    pub async fn list_installed(&self, pool: &SqlitePool) -> TeamResult<Vec<InstalledResource>> {
        self.query_installed(pool, "", &[]).await
    }

    /// Get installed resources for a specific team
    pub async fn list_installed_by_team(
        &self,
        pool: &SqlitePool,
        team_id: &str,
    ) -> TeamResult<Vec<InstalledResource>> {
        self.query_installed(pool, "WHERE team_id = ?", &[team_id]).await
    }

    /// Get installed resources with updates available
    pub async fn list_with_updates(&self, pool: &SqlitePool) -> TeamResult<Vec<InstalledResource>> {
        self.query_installed(pool, "WHERE has_update = 1", &[]).await
    }

    /// Shared query helper for installed resources
    async fn query_installed(
        &self,
        pool: &SqlitePool,
        where_clause: &str,
        params: &[&str],
    ) -> TeamResult<Vec<InstalledResource>> {
        let sql = format!(
            r#"
            SELECT id, resource_type, resource_id, team_id, resource_name, local_path,
                   installed_version, latest_version, has_update, installed_at, last_checked_at,
                   COALESCE(protection_level, 'team_installable') as protection_level
            FROM installed_resources
            {} ORDER BY installed_at DESC
            "#,
            where_clause
        );

        let mut query = sqlx::query_as::<_, InstalledResourceRow>(&sql);
        for param in params {
            query = query.bind(*param);
        }

        let rows = query.fetch_all(pool).await?;
        Ok(rows.into_iter().map(row_to_installed_resource).collect())
    }
}

/// Database row struct for installed resources (avoids tuple size limits and duplication)
#[derive(sqlx::FromRow)]
struct InstalledResourceRow {
    id: String,
    resource_type: String,
    resource_id: String,
    team_id: String,
    resource_name: String,
    local_path: Option<String>,
    installed_version: String,
    latest_version: Option<String>,
    has_update: i32,
    installed_at: String,
    last_checked_at: Option<String>,
    protection_level: String,
}

/// Convert an InstalledResourceRow to an InstalledResource
fn row_to_installed_resource(row: InstalledResourceRow) -> InstalledResource {
    InstalledResource {
        id: row.id,
        resource_type: row.resource_type.parse().unwrap_or(ResourceType::Skill),
        resource_id: row.resource_id,
        team_id: row.team_id,
        resource_name: row.resource_name,
        local_path: row.local_path,
        installed_version: row.installed_version,
        latest_version: row.latest_version,
        has_update: row.has_update != 0,
        installed_at: chrono::DateTime::parse_from_rfc3339(&row.installed_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        last_checked_at: row
            .last_checked_at
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc)),
        user_id: None,
        authorization_token: None,
        authorization_expires_at: None,
        last_verified_at: None,
        protection_level: row
            .protection_level
            .parse()
            .unwrap_or(ProtectionLevel::TeamInstallable),
    }
}

impl Default for InstallService {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate a simple access token
/// In production, this should use proper JWT with signing
fn generate_access_token(
    team_id: &str,
    resource_id: &str,
    user_id: &str,
    expires_at: &chrono::DateTime<Utc>,
) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // Create a simple hash-based token
    // In production, use proper JWT with HMAC signing
    let mut hasher = DefaultHasher::new();
    team_id.hash(&mut hasher);
    resource_id.hash(&mut hasher);
    user_id.hash(&mut hasher);
    expires_at.timestamp().hash(&mut hasher);
    // Add a secret component (should be from config in production)
    "agime-skill-access-secret".hash(&mut hasher);

    let hash = hasher.finish();
    format!("sk_{}_{:x}", expires_at.timestamp(), hash)
}
