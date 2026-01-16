//! Skill service - business logic for skill operations

use sqlx::{SqlitePool, Row};
use chrono::Utc;

use crate::error::{TeamError, TeamResult};
use crate::models::{
    SharedSkill, ShareSkillRequest, UpdateSkillRequest, ListSkillsQuery,
    PaginatedResponse, SkillWithInfo, InstallStatus, Visibility, ProtectionLevel,
    SkillStorageType,
};
use crate::services::MemberService;

/// Skill service
pub struct SkillService {
    member_service: MemberService,
}

impl SkillService {
    /// Create a new skill service
    pub fn new() -> Self {
        Self {
            member_service: MemberService::new(),
        }
    }

    /// Share a skill to a team
    pub async fn share_skill(
        &self,
        pool: &SqlitePool,
        request: ShareSkillRequest,
        author_id: &str,
    ) -> TeamResult<SharedSkill> {
        // Check membership and permission
        let member = self.member_service.get_member_by_user(pool, &request.team_id, author_id).await?;
        if !member.can_share_resources() {
            return Err(TeamError::PermissionDenied {
                action: "share skill".to_string(),
            });
        }

        // Check if skill with same name exists
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM shared_skills WHERE team_id = ? AND name = ? AND is_deleted = 0"
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

        // Determine storage type and create skill
        let storage_type = request.effective_storage_type();
        let mut skill = match storage_type {
            crate::models::SkillStorageType::Package => {
                let mut s = SharedSkill::new_package(
                    request.team_id,
                    request.name,
                    request.skill_md.unwrap_or_default(),
                    author_id.to_string(),
                );
                s.files = request.files;
                s.metadata = request.metadata;
                s
            }
            crate::models::SkillStorageType::Inline => {
                SharedSkill::new_inline(
                    request.team_id,
                    request.name,
                    request.content.unwrap_or_default(),
                    author_id.to_string(),
                )
            }
        };

        if let Some(desc) = request.description {
            skill.description = Some(desc);
        }
        if let Some(tags) = request.tags {
            skill.tags = tags;
        }
        if let Some(deps) = request.dependencies {
            skill.dependencies = deps;
        }
        if let Some(visibility) = request.visibility {
            skill.visibility = visibility;
        }

        let tags_json = serde_json::to_string(&skill.tags)?;
        let deps_json = serde_json::to_string(&skill.dependencies)?;

        sqlx::query(
            r#"
            INSERT INTO shared_skills (
                id, team_id, name, description, content, author_id, version,
                visibility, tags_json, dependencies_json, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&skill.id)
        .bind(&skill.team_id)
        .bind(&skill.name)
        .bind(&skill.description)
        .bind(&skill.content)
        .bind(&skill.author_id)
        .bind(&skill.version)
        .bind(skill.visibility.to_string())
        .bind(&tags_json)
        .bind(&deps_json)
        .bind(skill.created_at.to_rfc3339())
        .bind(skill.updated_at.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(skill)
    }

    /// Get a skill by ID
    pub async fn get_skill(&self, pool: &SqlitePool, skill_id: &str) -> TeamResult<SharedSkill> {
        let row = sqlx::query(
            r#"
            SELECT id, team_id, name, description, content, author_id, version,
                   previous_version_id, visibility, tags_json, dependencies_json,
                   use_count, is_deleted, created_at, updated_at,
                   storage_type, skill_md, files_json, manifest_json, metadata_json,
                   package_url, package_hash, package_size,
                   COALESCE(protection_level, 'team_installable') as protection_level
            FROM shared_skills
            WHERE id = ? AND is_deleted = 0
            "#,
        )
        .bind(skill_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| TeamError::ResourceNotFound {
            resource_type: "skill".to_string(),
            resource_id: skill_id.to_string(),
        })?;

        self.row_to_skill_from_row(&row)
    }

    /// Get skill with additional info
    pub async fn get_skill_with_info(
        &self,
        pool: &SqlitePool,
        skill_id: &str,
        _user_id: &str,
    ) -> TeamResult<SkillWithInfo> {
        let skill = self.get_skill(pool, skill_id).await?;

        // Get author name
        let author_name: Option<(String,)> = sqlx::query_as(
            "SELECT display_name FROM team_members WHERE user_id = ? AND team_id = ?"
        )
        .bind(&skill.author_id)
        .bind(&skill.team_id)
        .fetch_optional(pool)
        .await?;

        // Get install status
        let install_status = self.get_install_status(pool, skill_id).await?;

        Ok(SkillWithInfo {
            skill,
            author_name: author_name.map(|r| r.0),
            install_status,
        })
    }

    /// List skills with safe parameterized queries
    pub async fn list_skills(
        &self,
        pool: &SqlitePool,
        query: ListSkillsQuery,
        user_id: &str,
    ) -> TeamResult<PaginatedResponse<SharedSkill>> {
        let offset = (query.page.saturating_sub(1)) * query.limit;

        // Determine sort column (whitelist approach to prevent SQL injection)
        let sort_col = match query.sort.as_str() {
            "name" => "s.name",
            "created_at" => "s.created_at",
            "use_count" => "s.use_count",
            _ => "s.updated_at",
        };

        // Build search pattern if provided
        let search_pattern = query.search.as_ref().map(|s| format!("%{}%", s));

        // Count total with safe parameterized query
        let total: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM shared_skills s
            JOIN team_members tm ON s.team_id = tm.team_id AND tm.user_id = ?
            WHERE s.is_deleted = 0
              AND (? IS NULL OR s.team_id = ?)
              AND (? IS NULL OR s.name LIKE ? OR s.description LIKE ?)
              AND (? IS NULL OR s.author_id = ?)
            "#,
        )
        .bind(user_id)
        .bind(&query.team_id)
        .bind(&query.team_id)
        .bind(&search_pattern)
        .bind(&search_pattern)
        .bind(&search_pattern)
        .bind(&query.author_id)
        .bind(&query.author_id)
        .fetch_one(pool)
        .await?;

        // Build and execute select query based on sort column
        // Using match to handle different ORDER BY clauses safely
        let rows = match sort_col {
            "s.name" => {
                sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, String, String, Option<String>, String, String, String, String, i64, i32, String, String)>(
                    r#"
                    SELECT s.id, s.team_id, s.name, s.description, s.content, s.author_id, s.version,
                           s.previous_version_id, s.visibility, s.tags_json, s.dependencies_json,
                           COALESCE(s.protection_level, 'team_installable') as protection_level,
                           s.use_count, s.is_deleted, s.created_at, s.updated_at
                    FROM shared_skills s
                    JOIN team_members tm ON s.team_id = tm.team_id AND tm.user_id = ?
                    WHERE s.is_deleted = 0
                      AND (? IS NULL OR s.team_id = ?)
                      AND (? IS NULL OR s.name LIKE ? OR s.description LIKE ?)
                      AND (? IS NULL OR s.author_id = ?)
                    ORDER BY s.name DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
            }
            "s.created_at" => {
                sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, String, String, Option<String>, String, String, String, String, i64, i32, String, String)>(
                    r#"
                    SELECT s.id, s.team_id, s.name, s.description, s.content, s.author_id, s.version,
                           s.previous_version_id, s.visibility, s.tags_json, s.dependencies_json,
                           COALESCE(s.protection_level, 'team_installable') as protection_level,
                           s.use_count, s.is_deleted, s.created_at, s.updated_at
                    FROM shared_skills s
                    JOIN team_members tm ON s.team_id = tm.team_id AND tm.user_id = ?
                    WHERE s.is_deleted = 0
                      AND (? IS NULL OR s.team_id = ?)
                      AND (? IS NULL OR s.name LIKE ? OR s.description LIKE ?)
                      AND (? IS NULL OR s.author_id = ?)
                    ORDER BY s.created_at DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
            }
            "s.use_count" => {
                sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, String, String, Option<String>, String, String, String, String, i64, i32, String, String)>(
                    r#"
                    SELECT s.id, s.team_id, s.name, s.description, s.content, s.author_id, s.version,
                           s.previous_version_id, s.visibility, s.tags_json, s.dependencies_json,
                           COALESCE(s.protection_level, 'team_installable') as protection_level,
                           s.use_count, s.is_deleted, s.created_at, s.updated_at
                    FROM shared_skills s
                    JOIN team_members tm ON s.team_id = tm.team_id AND tm.user_id = ?
                    WHERE s.is_deleted = 0
                      AND (? IS NULL OR s.team_id = ?)
                      AND (? IS NULL OR s.name LIKE ? OR s.description LIKE ?)
                      AND (? IS NULL OR s.author_id = ?)
                    ORDER BY s.use_count DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
            }
            _ => {
                sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, String, String, Option<String>, String, String, String, String, i64, i32, String, String)>(
                    r#"
                    SELECT s.id, s.team_id, s.name, s.description, s.content, s.author_id, s.version,
                           s.previous_version_id, s.visibility, s.tags_json, s.dependencies_json,
                           COALESCE(s.protection_level, 'team_installable') as protection_level,
                           s.use_count, s.is_deleted, s.created_at, s.updated_at
                    FROM shared_skills s
                    JOIN team_members tm ON s.team_id = tm.team_id AND tm.user_id = ?
                    WHERE s.is_deleted = 0
                      AND (? IS NULL OR s.team_id = ?)
                      AND (? IS NULL OR s.name LIKE ? OR s.description LIKE ?)
                      AND (? IS NULL OR s.author_id = ?)
                    ORDER BY s.updated_at DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
            }
        }
        .bind(user_id)
        .bind(&query.team_id)
        .bind(&query.team_id)
        .bind(&search_pattern)
        .bind(&search_pattern)
        .bind(&search_pattern)
        .bind(&query.author_id)
        .bind(&query.author_id)
        .bind(query.limit as i64)
        .bind(offset as i64)
        .fetch_all(pool)
        .await?;

        let skills: Vec<SharedSkill> = rows
            .into_iter()
            .filter_map(|row| self.row_to_skill(row).ok())
            .collect();

        Ok(PaginatedResponse::new(skills, total.0 as u64, query.page, query.limit))
    }

    /// Update a skill
    pub async fn update_skill(
        &self,
        pool: &SqlitePool,
        skill_id: &str,
        request: UpdateSkillRequest,
        requester_id: &str,
    ) -> TeamResult<SharedSkill> {
        let mut skill = self.get_skill(pool, skill_id).await?;

        // Check permission
        let member = self.member_service.get_member_by_user(pool, &skill.team_id, requester_id).await?;
        if !member.can_delete_resource(&skill.author_id) {
            return Err(TeamError::PermissionDenied {
                action: "update skill".to_string(),
            });
        }

        let old_id = skill.id.clone();

        // Update fields and increment version
        if let Some(content) = request.content {
            skill.content = Some(content);
            skill.increment_version();
        }

        if let Some(description) = request.description {
            skill.description = Some(description);
        }
        if let Some(tags) = request.tags {
            skill.tags = tags;
        }
        if let Some(deps) = request.dependencies {
            skill.dependencies = deps;
        }
        if let Some(visibility) = request.visibility {
            skill.visibility = visibility;
        }

        skill.updated_at = Utc::now();
        skill.previous_version_id = Some(old_id);
        skill.id = uuid::Uuid::new_v4().to_string();

        let tags_json = serde_json::to_string(&skill.tags)?;
        let deps_json = serde_json::to_string(&skill.dependencies)?;

        // Insert new version
        sqlx::query(
            r#"
            INSERT INTO shared_skills (
                id, team_id, name, description, content, author_id, version,
                previous_version_id, visibility, tags_json, dependencies_json,
                use_count, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&skill.id)
        .bind(&skill.team_id)
        .bind(&skill.name)
        .bind(&skill.description)
        .bind(&skill.content)
        .bind(&skill.author_id)
        .bind(&skill.version)
        .bind(&skill.previous_version_id)
        .bind(skill.visibility.to_string())
        .bind(&tags_json)
        .bind(&deps_json)
        .bind(0i64)
        .bind(skill.created_at.to_rfc3339())
        .bind(skill.updated_at.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(skill)
    }

    /// Delete a skill (soft delete)
    pub async fn delete_skill(
        &self,
        pool: &SqlitePool,
        skill_id: &str,
        requester_id: &str,
    ) -> TeamResult<()> {
        let skill = self.get_skill(pool, skill_id).await?;

        // Check permission
        let member = self.member_service.get_member_by_user(pool, &skill.team_id, requester_id).await?;
        if !member.can_delete_resource(&skill.author_id) {
            return Err(TeamError::PermissionDenied {
                action: "delete skill".to_string(),
            });
        }

        let now = Utc::now();
        sqlx::query("UPDATE shared_skills SET is_deleted = 1, updated_at = ? WHERE id = ?")
            .bind(now.to_rfc3339())
            .bind(skill_id)
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Increment use count
    pub async fn increment_use_count(&self, pool: &SqlitePool, skill_id: &str) -> TeamResult<()> {
        sqlx::query("UPDATE shared_skills SET use_count = use_count + 1 WHERE id = ?")
            .bind(skill_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Create a skill directly from SharedSkill instance (for import)
    pub async fn create_skill(
        &self,
        pool: &SqlitePool,
        skill: SharedSkill,
    ) -> TeamResult<SharedSkill> {
        let tags_json = serde_json::to_string(&skill.tags)?;
        let deps_json = serde_json::to_string(&skill.dependencies)?;
        let files_json = skill.files.as_ref().map(|f| serde_json::to_string(f).ok()).flatten();
        let manifest_json = skill.manifest.as_ref().map(|m| serde_json::to_string(m).ok()).flatten();
        let metadata_json = skill.metadata.as_ref().map(|m| serde_json::to_string(m).ok()).flatten();

        sqlx::query(
            r#"
            INSERT INTO shared_skills (
                id, team_id, name, description, content, author_id, version,
                visibility, tags_json, dependencies_json, created_at, updated_at,
                storage_type, skill_md, files_json, manifest_json, metadata_json,
                package_url, package_hash, package_size
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&skill.id)
        .bind(&skill.team_id)
        .bind(&skill.name)
        .bind(&skill.description)
        .bind(&skill.content)
        .bind(&skill.author_id)
        .bind(&skill.version)
        .bind(skill.visibility.to_string())
        .bind(&tags_json)
        .bind(&deps_json)
        .bind(skill.created_at.to_rfc3339())
        .bind(skill.updated_at.to_rfc3339())
        .bind(skill.storage_type.to_string())
        .bind(&skill.skill_md)
        .bind(&files_json)
        .bind(&manifest_json)
        .bind(&metadata_json)
        .bind(&skill.package_url)
        .bind(&skill.package_hash)
        .bind(skill.package_size.map(|s| s as i64))
        .execute(pool)
        .await?;

        Ok(skill)
    }

    /// Get install status for a skill
    async fn get_install_status(
        &self,
        pool: &SqlitePool,
        skill_id: &str,
    ) -> TeamResult<Option<InstallStatus>> {
        let result: Option<(String, Option<String>, i32)> = sqlx::query_as(
            r#"
            SELECT installed_version, latest_version, has_update
            FROM installed_resources
            WHERE resource_type = 'skill' AND resource_id = ?
            "#,
        )
        .bind(skill_id)
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

    /// Helper to convert row tuple to SharedSkill (basic fields only, for list queries)
    fn row_to_skill(
        &self,
        row: (String, String, String, Option<String>, Option<String>, String, String, Option<String>, String, String, String, String, i64, i32, String, String),
    ) -> TeamResult<SharedSkill> {
        let tags = serde_json::from_str(&row.9).unwrap_or_default();
        let dependencies = serde_json::from_str(&row.10).unwrap_or_default();

        Ok(SharedSkill {
            id: row.0,
            team_id: row.1,
            name: row.2,
            description: row.3,
            storage_type: SkillStorageType::Inline,
            content: row.4,
            skill_md: None,
            files: None,
            manifest: None,
            package_url: None,
            package_hash: None,
            package_size: None,
            metadata: None,
            author_id: row.5,
            version: row.6,
            previous_version_id: row.7,
            visibility: row.8.parse().unwrap_or(Visibility::Team),
            protection_level: row.11.parse().unwrap_or(ProtectionLevel::TeamInstallable),
            tags,
            dependencies,
            use_count: row.12 as u32,
            is_deleted: row.13 != 0,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.14)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.15)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }

    /// Helper to convert sqlx::Row to SharedSkill (includes all package fields)
    fn row_to_skill_from_row(&self, row: &sqlx::sqlite::SqliteRow) -> TeamResult<SharedSkill> {
        let tags_json: String = row.get("tags_json");
        let deps_json: String = row.get("dependencies_json");
        let tags = serde_json::from_str(&tags_json).unwrap_or_default();
        let dependencies = serde_json::from_str(&deps_json).unwrap_or_default();

        // Parse storage type
        let storage_type_str: Option<String> = row.get("storage_type");
        let storage_type = storage_type_str.as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(SkillStorageType::Inline);

        // Parse files JSON
        let files_json: Option<String> = row.get("files_json");
        let files = files_json.as_ref()
            .and_then(|s| serde_json::from_str(s).ok());

        // Parse manifest JSON
        let manifest_json: Option<String> = row.get("manifest_json");
        let manifest = manifest_json.as_ref()
            .and_then(|s| serde_json::from_str(s).ok());

        // Parse metadata JSON
        let metadata_json: Option<String> = row.get("metadata_json");
        let metadata = metadata_json.as_ref()
            .and_then(|s| serde_json::from_str(s).ok());

        // Parse protection level
        let protection_level: String = row.get("protection_level");

        let created_at_str: String = row.get("created_at");
        let updated_at_str: String = row.get("updated_at");

        Ok(SharedSkill {
            id: row.get("id"),
            team_id: row.get("team_id"),
            name: row.get("name"),
            description: row.get("description"),
            storage_type,
            content: row.get("content"),
            skill_md: row.get("skill_md"),
            files,
            manifest,
            package_url: row.get("package_url"),
            package_hash: row.get("package_hash"),
            package_size: row.get::<Option<i64>, _>("package_size").map(|s| s as u64),
            metadata,
            author_id: row.get("author_id"),
            version: row.get("version"),
            previous_version_id: row.get("previous_version_id"),
            visibility: row.get::<String, _>("visibility").parse().unwrap_or(Visibility::Team),
            protection_level: protection_level.parse().unwrap_or(ProtectionLevel::TeamInstallable),
            tags,
            dependencies,
            use_count: row.get::<i64, _>("use_count") as u32,
            is_deleted: row.get::<i32, _>("is_deleted") != 0,
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }
}

impl Default for SkillService {
    fn default() -> Self {
        Self::new()
    }
}
