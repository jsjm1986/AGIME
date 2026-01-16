//! Extension service - business logic for extension operations

use sqlx::SqlitePool;
use chrono::Utc;

use crate::error::{TeamError, TeamResult};
use crate::models::{
    SharedExtension, ExtensionType, ExtensionConfig, ShareExtensionRequest,
    UpdateExtensionRequest, ReviewExtensionRequest, ListExtensionsQuery,
    PaginatedResponse, ExtensionWithInfo, InstallStatus, Visibility, ProtectionLevel,
};
use crate::services::MemberService;

/// Database row struct for extensions (sqlx has tuple size limit)
#[derive(sqlx::FromRow)]
struct ExtensionRow {
    id: String,
    team_id: String,
    name: String,
    description: Option<String>,
    extension_type: String,
    config_json: String,
    author_id: String,
    version: String,
    previous_version_id: Option<String>,
    visibility: String,
    protection_level: String,
    tags_json: String,
    security_reviewed: i32,
    security_notes: Option<String>,
    reviewed_by: Option<String>,
    reviewed_at: Option<String>,
    use_count: i64,
    is_deleted: i32,
    created_at: String,
    updated_at: String,
}

/// Extension service
pub struct ExtensionService {
    member_service: MemberService,
}

impl ExtensionService {
    /// Create a new extension service
    pub fn new() -> Self {
        Self {
            member_service: MemberService::new(),
        }
    }

    /// Share an extension to a team
    pub async fn share_extension(
        &self,
        pool: &SqlitePool,
        request: ShareExtensionRequest,
        author_id: &str,
    ) -> TeamResult<SharedExtension> {
        // Check membership and permission
        let member = self.member_service.get_member_by_user(pool, &request.team_id, author_id).await?;
        if !member.can_share_resources() {
            return Err(TeamError::PermissionDenied {
                action: "share extension".to_string(),
            });
        }

        // Check if extension with same name exists
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM shared_extensions WHERE team_id = ? AND name = ? AND is_deleted = 0"
        )
        .bind(&request.team_id)
        .bind(&request.name)
        .fetch_optional(pool)
        .await?;

        if existing.is_some() {
            return Err(TeamError::ResourceExists {
                name: request.name,
                version: "1.0.0".to_string(),
            });
        }

        let mut extension = SharedExtension::new(
            request.team_id,
            request.name,
            request.extension_type,
            request.config,
            author_id.to_string(),
        );

        if let Some(desc) = request.description {
            extension.description = Some(desc);
        }
        if let Some(tags) = request.tags {
            extension.tags = tags;
        }
        if let Some(visibility) = request.visibility {
            extension.visibility = visibility;
        }
        // BIZ-2 FIX: Process protection_level from request
        if let Some(protection_level) = request.protection_level {
            extension.protection_level = protection_level;
        }

        let config_json = serde_json::to_string(&extension.config)?;
        let tags_json = serde_json::to_string(&extension.tags)?;

        sqlx::query(
            r#"
            INSERT INTO shared_extensions (
                id, team_id, name, description, extension_type, config_json, author_id, version,
                visibility, protection_level, tags_json, security_reviewed, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&extension.id)
        .bind(&extension.team_id)
        .bind(&extension.name)
        .bind(&extension.description)
        .bind(extension.extension_type.to_string())
        .bind(&config_json)
        .bind(&extension.author_id)
        .bind(&extension.version)
        .bind(extension.visibility.to_string())
        .bind(extension.protection_level.to_string())
        .bind(&tags_json)
        .bind(0i32)
        .bind(extension.created_at.to_rfc3339())
        .bind(extension.updated_at.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(extension)
    }

    /// Get an extension by ID
    pub async fn get_extension(&self, pool: &SqlitePool, extension_id: &str) -> TeamResult<SharedExtension> {
        let row: ExtensionRow = sqlx::query_as(
            r#"
            SELECT id, team_id, name, description, extension_type, config_json, author_id, version,
                   previous_version_id, visibility,
                   COALESCE(protection_level, 'team_installable') as protection_level,
                   tags_json, security_reviewed, security_notes,
                   reviewed_by, reviewed_at, use_count, is_deleted, created_at, updated_at
            FROM shared_extensions
            WHERE id = ? AND is_deleted = 0
            "#,
        )
        .bind(extension_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| TeamError::ResourceNotFound {
            resource_type: "extension".to_string(),
            resource_id: extension_id.to_string(),
        })?;

        self.row_to_extension(row)
    }

    /// Get extension with additional info
    pub async fn get_extension_with_info(
        &self,
        pool: &SqlitePool,
        extension_id: &str,
        _user_id: &str,
    ) -> TeamResult<ExtensionWithInfo> {
        let extension = self.get_extension(pool, extension_id).await?;

        // Get author name
        let author_name: Option<(String,)> = sqlx::query_as(
            "SELECT display_name FROM team_members WHERE user_id = ? AND team_id = ?"
        )
        .bind(&extension.author_id)
        .bind(&extension.team_id)
        .fetch_optional(pool)
        .await?;

        // Get reviewer name if reviewed
        let reviewer_name = if let Some(ref reviewer_id) = extension.reviewed_by {
            let result: Option<(String,)> = sqlx::query_as(
                "SELECT display_name FROM team_members WHERE user_id = ? AND team_id = ?"
            )
            .bind(reviewer_id)
            .bind(&extension.team_id)
            .fetch_optional(pool)
            .await?;
            result.map(|r| r.0)
        } else {
            None
        };

        // Get install status
        let install_status = self.get_install_status(pool, extension_id).await?;

        Ok(ExtensionWithInfo {
            extension,
            author_name: author_name.map(|r| r.0),
            reviewer_name,
            install_status,
        })
    }

    /// List extensions with safe parameterized queries
    pub async fn list_extensions(
        &self,
        pool: &SqlitePool,
        query: ListExtensionsQuery,
        user_id: &str,
    ) -> TeamResult<PaginatedResponse<SharedExtension>> {
        let offset = (query.page.saturating_sub(1)) * query.limit;

        // Determine sort column (whitelist approach to prevent SQL injection)
        let sort_col = match query.sort.as_str() {
            "name" => "e.name",
            "created_at" => "e.created_at",
            "use_count" => "e.use_count",
            _ => "e.updated_at",
        };

        // Build search pattern if provided
        let search_pattern = query.search.as_ref().map(|s| format!("%{}%", s));

        // Count total with safe parameterized query
        let total: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM shared_extensions e
            JOIN team_members tm ON e.team_id = tm.team_id AND tm.user_id = ?
            WHERE e.is_deleted = 0
              AND (? IS NULL OR e.team_id = ?)
              AND (? IS NULL OR e.extension_type = ?)
              AND (? = 0 OR e.security_reviewed = 1)
              AND (? IS NULL OR e.name LIKE ? OR e.description LIKE ?)
              AND (? IS NULL OR e.author_id = ?)
            "#,
        )
        .bind(user_id)
        .bind(&query.team_id)
        .bind(&query.team_id)
        .bind(query.extension_type.as_ref().map(|t| t.to_string()))
        .bind(query.extension_type.as_ref().map(|t| t.to_string()))
        .bind(if query.reviewed_only.unwrap_or(false) { 1i32 } else { 0i32 })
        .bind(&search_pattern)
        .bind(&search_pattern)
        .bind(&search_pattern)
        .bind(&query.author_id)
        .bind(&query.author_id)
        .fetch_one(pool)
        .await?;

        // Build and execute select query based on sort column
        // Using match to handle different ORDER BY clauses safely
        let rows: Vec<ExtensionRow> = match sort_col {
            "e.name" => {
                sqlx::query_as(
                    r#"
                    SELECT e.id, e.team_id, e.name, e.description, e.extension_type, e.config_json, e.author_id, e.version,
                           e.previous_version_id, e.visibility,
                           COALESCE(e.protection_level, 'team_installable') as protection_level,
                           e.tags_json, e.security_reviewed, e.security_notes,
                           e.reviewed_by, e.reviewed_at, e.use_count, e.is_deleted, e.created_at, e.updated_at
                    FROM shared_extensions e
                    JOIN team_members tm ON e.team_id = tm.team_id AND tm.user_id = ?
                    WHERE e.is_deleted = 0
                      AND (? IS NULL OR e.team_id = ?)
                      AND (? IS NULL OR e.extension_type = ?)
                      AND (? = 0 OR e.security_reviewed = 1)
                      AND (? IS NULL OR e.name LIKE ? OR e.description LIKE ?)
                      AND (? IS NULL OR e.author_id = ?)
                    ORDER BY e.name DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
            }
            "e.created_at" => {
                sqlx::query_as(
                    r#"
                    SELECT e.id, e.team_id, e.name, e.description, e.extension_type, e.config_json, e.author_id, e.version,
                           e.previous_version_id, e.visibility,
                           COALESCE(e.protection_level, 'team_installable') as protection_level,
                           e.tags_json, e.security_reviewed, e.security_notes,
                           e.reviewed_by, e.reviewed_at, e.use_count, e.is_deleted, e.created_at, e.updated_at
                    FROM shared_extensions e
                    JOIN team_members tm ON e.team_id = tm.team_id AND tm.user_id = ?
                    WHERE e.is_deleted = 0
                      AND (? IS NULL OR e.team_id = ?)
                      AND (? IS NULL OR e.extension_type = ?)
                      AND (? = 0 OR e.security_reviewed = 1)
                      AND (? IS NULL OR e.name LIKE ? OR e.description LIKE ?)
                      AND (? IS NULL OR e.author_id = ?)
                    ORDER BY e.created_at DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
            }
            "e.use_count" => {
                sqlx::query_as(
                    r#"
                    SELECT e.id, e.team_id, e.name, e.description, e.extension_type, e.config_json, e.author_id, e.version,
                           e.previous_version_id, e.visibility,
                           COALESCE(e.protection_level, 'team_installable') as protection_level,
                           e.tags_json, e.security_reviewed, e.security_notes,
                           e.reviewed_by, e.reviewed_at, e.use_count, e.is_deleted, e.created_at, e.updated_at
                    FROM shared_extensions e
                    JOIN team_members tm ON e.team_id = tm.team_id AND tm.user_id = ?
                    WHERE e.is_deleted = 0
                      AND (? IS NULL OR e.team_id = ?)
                      AND (? IS NULL OR e.extension_type = ?)
                      AND (? = 0 OR e.security_reviewed = 1)
                      AND (? IS NULL OR e.name LIKE ? OR e.description LIKE ?)
                      AND (? IS NULL OR e.author_id = ?)
                    ORDER BY e.use_count DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
            }
            _ => {
                sqlx::query_as(
                    r#"
                    SELECT e.id, e.team_id, e.name, e.description, e.extension_type, e.config_json, e.author_id, e.version,
                           e.previous_version_id, e.visibility,
                           COALESCE(e.protection_level, 'team_installable') as protection_level,
                           e.tags_json, e.security_reviewed, e.security_notes,
                           e.reviewed_by, e.reviewed_at, e.use_count, e.is_deleted, e.created_at, e.updated_at
                    FROM shared_extensions e
                    JOIN team_members tm ON e.team_id = tm.team_id AND tm.user_id = ?
                    WHERE e.is_deleted = 0
                      AND (? IS NULL OR e.team_id = ?)
                      AND (? IS NULL OR e.extension_type = ?)
                      AND (? = 0 OR e.security_reviewed = 1)
                      AND (? IS NULL OR e.name LIKE ? OR e.description LIKE ?)
                      AND (? IS NULL OR e.author_id = ?)
                    ORDER BY e.updated_at DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
            }
        }
        .bind(user_id)
        .bind(&query.team_id)
        .bind(&query.team_id)
        .bind(query.extension_type.as_ref().map(|t| t.to_string()))
        .bind(query.extension_type.as_ref().map(|t| t.to_string()))
        .bind(if query.reviewed_only.unwrap_or(false) { 1i32 } else { 0i32 })
        .bind(&search_pattern)
        .bind(&search_pattern)
        .bind(&search_pattern)
        .bind(&query.author_id)
        .bind(&query.author_id)
        .bind(query.limit as i64)
        .bind(offset as i64)
        .fetch_all(pool)
        .await?;

        let extensions: Vec<SharedExtension> = rows
            .into_iter()
            .filter_map(|row| self.row_to_extension(row).ok())
            .collect();

        Ok(PaginatedResponse::new(extensions, total.0 as u64, query.page, query.limit))
    }

    /// Update an extension
    pub async fn update_extension(
        &self,
        pool: &SqlitePool,
        extension_id: &str,
        request: UpdateExtensionRequest,
        requester_id: &str,
    ) -> TeamResult<SharedExtension> {
        let mut extension = self.get_extension(pool, extension_id).await?;

        // Check permission
        let member = self.member_service.get_member_by_user(pool, &extension.team_id, requester_id).await?;
        if !member.can_delete_resource(&extension.author_id) {
            return Err(TeamError::PermissionDenied {
                action: "update extension".to_string(),
            });
        }

        let old_id = extension.id.clone();

        // Update fields and increment version if config changed
        if let Some(config) = request.config {
            extension.config = config;
            extension.increment_version();
            // Reset security review when config changes
            extension.security_reviewed = false;
            extension.security_notes = None;
            extension.reviewed_by = None;
            extension.reviewed_at = None;
        }

        if let Some(description) = request.description {
            extension.description = Some(description);
        }
        if let Some(tags) = request.tags {
            extension.tags = tags;
        }
        if let Some(visibility) = request.visibility {
            extension.visibility = visibility;
        }
        if let Some(name) = request.name {
            extension.name = name;
        }

        extension.updated_at = Utc::now();
        extension.previous_version_id = Some(old_id);
        extension.id = uuid::Uuid::new_v4().to_string();

        let config_json = serde_json::to_string(&extension.config)?;
        let tags_json = serde_json::to_string(&extension.tags)?;

        sqlx::query(
            r#"
            INSERT INTO shared_extensions (
                id, team_id, name, description, extension_type, config_json, author_id, version,
                previous_version_id, visibility, tags_json, security_reviewed, security_notes,
                reviewed_by, reviewed_at, use_count, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&extension.id)
        .bind(&extension.team_id)
        .bind(&extension.name)
        .bind(&extension.description)
        .bind(extension.extension_type.to_string())
        .bind(&config_json)
        .bind(&extension.author_id)
        .bind(&extension.version)
        .bind(&extension.previous_version_id)
        .bind(extension.visibility.to_string())
        .bind(&tags_json)
        .bind(if extension.security_reviewed { 1i32 } else { 0i32 })
        .bind(&extension.security_notes)
        .bind(&extension.reviewed_by)
        .bind(extension.reviewed_at.map(|dt| dt.to_rfc3339()))
        .bind(0i64)
        .bind(extension.created_at.to_rfc3339())
        .bind(extension.updated_at.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(extension)
    }

    /// Review an extension
    pub async fn review_extension(
        &self,
        pool: &SqlitePool,
        extension_id: &str,
        request: ReviewExtensionRequest,
        reviewer_id: &str,
    ) -> TeamResult<SharedExtension> {
        let mut extension = self.get_extension(pool, extension_id).await?;

        // Check permission
        let member = self.member_service.get_member_by_user(pool, &extension.team_id, reviewer_id).await?;
        if !member.can_review_extensions() {
            return Err(TeamError::PermissionDenied {
                action: "review extension".to_string(),
            });
        }

        if request.approved {
            extension.mark_reviewed(reviewer_id.to_string(), request.notes.clone());
        } else {
            extension.security_reviewed = false;
            extension.security_notes = request.notes.clone();
            extension.reviewed_by = Some(reviewer_id.to_string());
            extension.reviewed_at = Some(Utc::now());
        }

        sqlx::query(
            r#"
            UPDATE shared_extensions
            SET security_reviewed = ?, security_notes = ?, reviewed_by = ?, reviewed_at = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(if extension.security_reviewed { 1i32 } else { 0i32 })
        .bind(&extension.security_notes)
        .bind(&extension.reviewed_by)
        .bind(extension.reviewed_at.map(|dt| dt.to_rfc3339()))
        .bind(Utc::now().to_rfc3339())
        .bind(extension_id)
        .execute(pool)
        .await?;

        Ok(extension)
    }

    /// Delete an extension (soft delete)
    pub async fn delete_extension(
        &self,
        pool: &SqlitePool,
        extension_id: &str,
        requester_id: &str,
    ) -> TeamResult<()> {
        let extension = self.get_extension(pool, extension_id).await?;

        let member = self.member_service.get_member_by_user(pool, &extension.team_id, requester_id).await?;
        if !member.can_delete_resource(&extension.author_id) {
            return Err(TeamError::PermissionDenied {
                action: "delete extension".to_string(),
            });
        }

        let now = Utc::now();
        sqlx::query("UPDATE shared_extensions SET is_deleted = 1, updated_at = ? WHERE id = ?")
            .bind(now.to_rfc3339())
            .bind(extension_id)
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Increment use count
    pub async fn increment_use_count(&self, pool: &SqlitePool, extension_id: &str) -> TeamResult<()> {
        sqlx::query("UPDATE shared_extensions SET use_count = use_count + 1 WHERE id = ?")
            .bind(extension_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Get install status for an extension
    async fn get_install_status(
        &self,
        pool: &SqlitePool,
        extension_id: &str,
    ) -> TeamResult<Option<InstallStatus>> {
        let result: Option<(String, Option<String>, i32)> = sqlx::query_as(
            r#"
            SELECT installed_version, latest_version, has_update
            FROM installed_resources
            WHERE resource_type = 'extension' AND resource_id = ?
            "#,
        )
        .bind(extension_id)
        .fetch_optional(pool)
        .await?;

        match result {
            Some((installed_version, _latest_version, has_update)) => Ok(Some(InstallStatus {
                installed: true,
                installed_version: Some(installed_version),
                has_update: Some(has_update != 0),
            })),
            None => Ok(None),
        }
    }

    /// Helper to convert row to SharedExtension
    fn row_to_extension(&self, row: ExtensionRow) -> TeamResult<SharedExtension> {
        let config: ExtensionConfig = serde_json::from_str(&row.config_json).unwrap_or_default();
        let tags = serde_json::from_str(&row.tags_json).unwrap_or_default();

        Ok(SharedExtension {
            id: row.id,
            team_id: row.team_id,
            name: row.name,
            description: row.description,
            extension_type: row.extension_type.parse().unwrap_or(ExtensionType::Stdio),
            config,
            author_id: row.author_id,
            version: row.version,
            previous_version_id: row.previous_version_id,
            visibility: row.visibility.parse().unwrap_or(Visibility::Team),
            protection_level: row.protection_level.parse().unwrap_or(ProtectionLevel::TeamInstallable),
            tags,
            security_reviewed: row.security_reviewed != 0,
            security_notes: row.security_notes,
            reviewed_by: row.reviewed_by,
            reviewed_at: row.reviewed_at
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            use_count: row.use_count as u32,
            is_deleted: row.is_deleted != 0,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }
}

impl Default for ExtensionService {
    fn default() -> Self {
        Self::new()
    }
}
