//! Recipe service - business logic for recipe operations

use chrono::Utc;
use sqlx::SqlitePool;

use crate::error::{TeamError, TeamResult};
use crate::models::{
    InstallStatus, ListRecipesQuery, PaginatedResponse, ProtectionLevel, RecipeWithInfo,
    ShareRecipeRequest, SharedRecipe, UpdateRecipeRequest, Visibility,
};
use crate::security::validator::{validate_recipe_content, validate_resource_name};
use crate::services::MemberService;

/// Database row struct for recipes (sqlx has tuple size limit)
#[derive(sqlx::FromRow)]
struct RecipeRow {
    id: String,
    team_id: String,
    name: String,
    description: Option<String>,
    content_yaml: String,
    author_id: String,
    version: String,
    previous_version_id: Option<String>,
    visibility: String,
    category: Option<String>,
    tags_json: String,
    dependencies_json: String,
    protection_level: String,
    use_count: i64,
    is_deleted: i32,
    created_at: String,
    updated_at: String,
}

/// Recipe service
pub struct RecipeService {
    member_service: MemberService,
}

impl RecipeService {
    /// Create a new recipe service
    pub fn new() -> Self {
        Self {
            member_service: MemberService::new(),
        }
    }

    /// Share a recipe to a team
    pub async fn share_recipe(
        &self,
        pool: &SqlitePool,
        request: ShareRecipeRequest,
        author_id: &str,
    ) -> TeamResult<SharedRecipe> {
        // Validate resource name
        validate_resource_name(&request.name)?;

        // Validate recipe content (YAML syntax and dangerous patterns)
        validate_recipe_content(&request.content_yaml)?;

        // Check membership and permission
        let member = self
            .member_service
            .get_member_by_user(pool, &request.team_id, author_id)
            .await?;
        if !member.can_share_resources() {
            return Err(TeamError::PermissionDenied {
                action: "share recipe".to_string(),
            });
        }

        // Check if recipe with same name exists
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM shared_recipes WHERE team_id = ? AND name = ? AND is_deleted = 0",
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

        let mut recipe = SharedRecipe::new(
            request.team_id,
            request.name,
            request.content_yaml,
            author_id.to_string(),
        );

        if let Some(desc) = request.description {
            recipe.description = Some(desc);
        }
        if let Some(category) = request.category {
            recipe.category = Some(category);
        }
        if let Some(tags) = request.tags {
            recipe.tags = tags;
        }
        if let Some(deps) = request.dependencies {
            recipe.dependencies = deps;
        }
        if let Some(visibility) = request.visibility {
            recipe.visibility = visibility;
        }
        // BIZ-1 FIX: Process protection_level from request
        if let Some(protection_level) = request.protection_level {
            recipe.protection_level = protection_level;
        }

        let tags_json = serde_json::to_string(&recipe.tags)?;
        let deps_json = serde_json::to_string(&recipe.dependencies)?;

        sqlx::query(
            r#"
            INSERT INTO shared_recipes (
                id, team_id, name, description, content_yaml, author_id, version,
                visibility, protection_level, category, tags_json, dependencies_json, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&recipe.id)
        .bind(&recipe.team_id)
        .bind(&recipe.name)
        .bind(&recipe.description)
        .bind(&recipe.content_yaml)
        .bind(&recipe.author_id)
        .bind(&recipe.version)
        .bind(recipe.visibility.to_string())
        .bind(recipe.protection_level.to_string())
        .bind(&recipe.category)
        .bind(&tags_json)
        .bind(&deps_json)
        .bind(recipe.created_at.to_rfc3339())
        .bind(recipe.updated_at.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(recipe)
    }

    /// Get a recipe by ID
    pub async fn get_recipe(&self, pool: &SqlitePool, recipe_id: &str) -> TeamResult<SharedRecipe> {
        let row: RecipeRow = sqlx::query_as(
            r#"
            SELECT id, team_id, name, description, content_yaml, author_id, version,
                   previous_version_id, visibility, category, tags_json, dependencies_json,
                   COALESCE(protection_level, 'team_installable') as protection_level,
                   use_count, is_deleted, created_at, updated_at
            FROM shared_recipes
            WHERE id = ? AND is_deleted = 0
            "#,
        )
        .bind(recipe_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| TeamError::ResourceNotFound {
            resource_type: "recipe".to_string(),
            resource_id: recipe_id.to_string(),
        })?;

        self.row_to_recipe(row)
    }

    /// Get recipe with additional info
    pub async fn get_recipe_with_info(
        &self,
        pool: &SqlitePool,
        recipe_id: &str,
        _user_id: &str,
    ) -> TeamResult<RecipeWithInfo> {
        let recipe = self.get_recipe(pool, recipe_id).await?;

        // Get author name
        let author_name: Option<(String,)> = sqlx::query_as(
            "SELECT display_name FROM team_members WHERE user_id = ? AND team_id = ?",
        )
        .bind(&recipe.author_id)
        .bind(&recipe.team_id)
        .fetch_optional(pool)
        .await?;

        // Get install status
        let install_status = self.get_install_status(pool, recipe_id).await?;

        Ok(RecipeWithInfo {
            recipe,
            author_name: author_name.map(|r| r.0),
            install_status,
        })
    }

    /// List recipes with safe parameterized queries
    pub async fn list_recipes(
        &self,
        pool: &SqlitePool,
        query: ListRecipesQuery,
        user_id: &str,
    ) -> TeamResult<PaginatedResponse<SharedRecipe>> {
        let offset = (query.page.saturating_sub(1)) * query.limit;

        // Determine sort column (whitelist approach to prevent SQL injection)
        let sort_col = match query.sort.as_str() {
            "name" => "r.name",
            "created_at" => "r.created_at",
            "use_count" => "r.use_count",
            _ => "r.updated_at",
        };

        // Build search pattern if provided
        let search_pattern = query.search.as_ref().map(|s| format!("%{}%", s));

        // Count total with safe parameterized query
        let total: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM shared_recipes r
            JOIN team_members tm ON r.team_id = tm.team_id AND tm.user_id = ?
            WHERE r.is_deleted = 0
              AND (? IS NULL OR r.team_id = ?)
              AND (? IS NULL OR r.category = ?)
              AND (? IS NULL OR r.name LIKE ? OR r.description LIKE ?)
              AND (? IS NULL OR r.author_id = ?)
            "#,
        )
        .bind(user_id)
        .bind(&query.team_id)
        .bind(&query.team_id)
        .bind(&query.category)
        .bind(&query.category)
        .bind(&search_pattern)
        .bind(&search_pattern)
        .bind(&search_pattern)
        .bind(&query.author_id)
        .bind(&query.author_id)
        .fetch_one(pool)
        .await?;

        // Build and execute select query based on sort column
        // Using match to handle different ORDER BY clauses safely
        let rows: Vec<RecipeRow> = match sort_col {
            "r.name" => {
                sqlx::query_as(
                    r#"
                    SELECT r.id, r.team_id, r.name, r.description, r.content_yaml, r.author_id, r.version,
                           r.previous_version_id, r.visibility, r.category, r.tags_json, r.dependencies_json,
                           COALESCE(r.protection_level, 'team_installable') as protection_level,
                           r.use_count, r.is_deleted, r.created_at, r.updated_at
                    FROM shared_recipes r
                    JOIN team_members tm ON r.team_id = tm.team_id AND tm.user_id = ?
                    WHERE r.is_deleted = 0
                      AND (? IS NULL OR r.team_id = ?)
                      AND (? IS NULL OR r.category = ?)
                      AND (? IS NULL OR r.name LIKE ? OR r.description LIKE ?)
                      AND (? IS NULL OR r.author_id = ?)
                    ORDER BY r.name DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
            }
            "r.created_at" => {
                sqlx::query_as(
                    r#"
                    SELECT r.id, r.team_id, r.name, r.description, r.content_yaml, r.author_id, r.version,
                           r.previous_version_id, r.visibility, r.category, r.tags_json, r.dependencies_json,
                           COALESCE(r.protection_level, 'team_installable') as protection_level,
                           r.use_count, r.is_deleted, r.created_at, r.updated_at
                    FROM shared_recipes r
                    JOIN team_members tm ON r.team_id = tm.team_id AND tm.user_id = ?
                    WHERE r.is_deleted = 0
                      AND (? IS NULL OR r.team_id = ?)
                      AND (? IS NULL OR r.category = ?)
                      AND (? IS NULL OR r.name LIKE ? OR r.description LIKE ?)
                      AND (? IS NULL OR r.author_id = ?)
                    ORDER BY r.created_at DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
            }
            "r.use_count" => {
                sqlx::query_as(
                    r#"
                    SELECT r.id, r.team_id, r.name, r.description, r.content_yaml, r.author_id, r.version,
                           r.previous_version_id, r.visibility, r.category, r.tags_json, r.dependencies_json,
                           COALESCE(r.protection_level, 'team_installable') as protection_level,
                           r.use_count, r.is_deleted, r.created_at, r.updated_at
                    FROM shared_recipes r
                    JOIN team_members tm ON r.team_id = tm.team_id AND tm.user_id = ?
                    WHERE r.is_deleted = 0
                      AND (? IS NULL OR r.team_id = ?)
                      AND (? IS NULL OR r.category = ?)
                      AND (? IS NULL OR r.name LIKE ? OR r.description LIKE ?)
                      AND (? IS NULL OR r.author_id = ?)
                    ORDER BY r.use_count DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
            }
            _ => {
                sqlx::query_as(
                    r#"
                    SELECT r.id, r.team_id, r.name, r.description, r.content_yaml, r.author_id, r.version,
                           r.previous_version_id, r.visibility, r.category, r.tags_json, r.dependencies_json,
                           COALESCE(r.protection_level, 'team_installable') as protection_level,
                           r.use_count, r.is_deleted, r.created_at, r.updated_at
                    FROM shared_recipes r
                    JOIN team_members tm ON r.team_id = tm.team_id AND tm.user_id = ?
                    WHERE r.is_deleted = 0
                      AND (? IS NULL OR r.team_id = ?)
                      AND (? IS NULL OR r.category = ?)
                      AND (? IS NULL OR r.name LIKE ? OR r.description LIKE ?)
                      AND (? IS NULL OR r.author_id = ?)
                    ORDER BY r.updated_at DESC
                    LIMIT ? OFFSET ?
                    "#,
                )
            }
        }
        .bind(user_id)
        .bind(&query.team_id)
        .bind(&query.team_id)
        .bind(&query.category)
        .bind(&query.category)
        .bind(&search_pattern)
        .bind(&search_pattern)
        .bind(&search_pattern)
        .bind(&query.author_id)
        .bind(&query.author_id)
        .bind(query.limit as i64)
        .bind(offset as i64)
        .fetch_all(pool)
        .await?;

        let recipes: Vec<SharedRecipe> = rows
            .into_iter()
            .filter_map(|row| self.row_to_recipe(row).ok())
            .collect();

        Ok(PaginatedResponse::new(
            recipes,
            total.0 as u64,
            query.page,
            query.limit,
        ))
    }

    /// Update a recipe
    pub async fn update_recipe(
        &self,
        pool: &SqlitePool,
        recipe_id: &str,
        request: UpdateRecipeRequest,
        requester_id: &str,
    ) -> TeamResult<SharedRecipe> {
        let mut recipe = self.get_recipe(pool, recipe_id).await?;

        // Check permission
        let member = self
            .member_service
            .get_member_by_user(pool, &recipe.team_id, requester_id)
            .await?;
        if !member.can_delete_resource(&recipe.author_id) {
            return Err(TeamError::PermissionDenied {
                action: "update recipe".to_string(),
            });
        }

        let old_id = recipe.id.clone();

        if let Some(content_yaml) = request.content_yaml {
            // Validate the new content before updating
            validate_recipe_content(&content_yaml)?;
            recipe.content_yaml = content_yaml;
            recipe.increment_version();
        }

        if let Some(description) = request.description {
            recipe.description = Some(description);
        }
        if let Some(category) = request.category {
            recipe.category = Some(category);
        }
        if let Some(tags) = request.tags {
            recipe.tags = tags;
        }
        if let Some(deps) = request.dependencies {
            recipe.dependencies = deps;
        }
        if let Some(visibility) = request.visibility {
            recipe.visibility = visibility;
        }
        if let Some(protection_level) = request.protection_level {
            recipe.protection_level = protection_level;
        }

        recipe.updated_at = Utc::now();
        recipe.previous_version_id = Some(old_id);
        recipe.id = uuid::Uuid::new_v4().to_string();

        let tags_json = serde_json::to_string(&recipe.tags)?;
        let deps_json = serde_json::to_string(&recipe.dependencies)?;

        sqlx::query(
            r#"
            INSERT INTO shared_recipes (
                id, team_id, name, description, content_yaml, author_id, version,
                previous_version_id, visibility, protection_level, category, tags_json, dependencies_json,
                use_count, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&recipe.id)
        .bind(&recipe.team_id)
        .bind(&recipe.name)
        .bind(&recipe.description)
        .bind(&recipe.content_yaml)
        .bind(&recipe.author_id)
        .bind(&recipe.version)
        .bind(&recipe.previous_version_id)
        .bind(recipe.visibility.to_string())
        .bind(recipe.protection_level.to_string())
        .bind(&recipe.category)
        .bind(&tags_json)
        .bind(&deps_json)
        .bind(0i64)
        .bind(recipe.created_at.to_rfc3339())
        .bind(recipe.updated_at.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(recipe)
    }

    /// Delete a recipe (soft delete)
    pub async fn delete_recipe(
        &self,
        pool: &SqlitePool,
        recipe_id: &str,
        requester_id: &str,
    ) -> TeamResult<()> {
        let recipe = self.get_recipe(pool, recipe_id).await?;

        let member = self
            .member_service
            .get_member_by_user(pool, &recipe.team_id, requester_id)
            .await?;
        if !member.can_delete_resource(&recipe.author_id) {
            return Err(TeamError::PermissionDenied {
                action: "delete recipe".to_string(),
            });
        }

        let now = Utc::now();
        sqlx::query("UPDATE shared_recipes SET is_deleted = 1, updated_at = ? WHERE id = ?")
            .bind(now.to_rfc3339())
            .bind(recipe_id)
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Increment use count
    pub async fn increment_use_count(&self, pool: &SqlitePool, recipe_id: &str) -> TeamResult<()> {
        sqlx::query("UPDATE shared_recipes SET use_count = use_count + 1 WHERE id = ?")
            .bind(recipe_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Get install status for a recipe
    async fn get_install_status(
        &self,
        pool: &SqlitePool,
        recipe_id: &str,
    ) -> TeamResult<Option<InstallStatus>> {
        let result: Option<(String, Option<String>, i32)> = sqlx::query_as(
            r#"
            SELECT installed_version, latest_version, has_update
            FROM installed_resources
            WHERE resource_type = 'recipe' AND resource_id = ?
            "#,
        )
        .bind(recipe_id)
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

    /// Helper to convert RecipeRow to SharedRecipe
    fn row_to_recipe(&self, row: RecipeRow) -> TeamResult<SharedRecipe> {
        let tags = serde_json::from_str(&row.tags_json).unwrap_or_default();
        let dependencies = serde_json::from_str(&row.dependencies_json).unwrap_or_default();

        Ok(SharedRecipe {
            id: row.id,
            team_id: row.team_id,
            name: row.name,
            description: row.description,
            content_yaml: row.content_yaml,
            author_id: row.author_id,
            version: row.version,
            previous_version_id: row.previous_version_id,
            visibility: row.visibility.parse().unwrap_or(Visibility::Team),
            protection_level: row
                .protection_level
                .parse()
                .unwrap_or(ProtectionLevel::TeamInstallable),
            category: row.category,
            tags,
            dependencies,
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

impl Default for RecipeService {
    fn default() -> Self {
        Self::new()
    }
}
