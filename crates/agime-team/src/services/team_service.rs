//! Team service - business logic for team operations

use chrono::Utc;
use sqlx::SqlitePool;

use crate::error::{TeamError, TeamResult};
use crate::models::{
    CreateTeamRequest, ListTeamsQuery, MemberRole, PaginatedResponse, Team, TeamMember,
    TeamSettings, TeamSummary, UpdateTeamRequest,
};

/// Team service
pub struct TeamService;

impl TeamService {
    /// Create a new team service
    pub fn new() -> Self {
        Self
    }

    /// Create a new team
    pub async fn create_team(
        &self,
        pool: &SqlitePool,
        request: CreateTeamRequest,
        owner_id: &str,
    ) -> TeamResult<Team> {
        let mut team = Team::new(request.name, owner_id.to_string());
        if let Some(desc) = request.description {
            team = team.with_description(desc);
        }
        if let Some(repo) = request.repository_url {
            team = team.with_repository(repo);
        }
        if let Some(settings) = request.settings {
            team.settings = settings;
        }

        let settings_json = serde_json::to_string(&team.settings)?;

        sqlx::query(
            r#"
            INSERT INTO teams (id, name, description, repository_url, owner_id, settings_json, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&team.id)
        .bind(&team.name)
        .bind(&team.description)
        .bind(&team.repository_url)
        .bind(&team.owner_id)
        .bind(&settings_json)
        .bind(team.created_at.to_rfc3339())
        .bind(team.updated_at.to_rfc3339())
        .execute(pool)
        .await?;

        // Create owner member
        let owner_member = TeamMember::new(
            team.id.clone(),
            owner_id.to_string(),
            "Owner".to_string(),
            MemberRole::Owner,
        );

        let permissions_json = serde_json::to_string(&owner_member.permissions)?;

        sqlx::query(
            r#"
            INSERT INTO team_members (id, team_id, user_id, display_name, role, status, permissions_json, joined_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&owner_member.id)
        .bind(&owner_member.team_id)
        .bind(&owner_member.user_id)
        .bind(&owner_member.display_name)
        .bind(owner_member.role.to_string())
        .bind(owner_member.status.to_string())
        .bind(&permissions_json)
        .bind(owner_member.joined_at.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(team)
    }

    /// Get a team by ID
    pub async fn get_team(&self, pool: &SqlitePool, team_id: &str) -> TeamResult<Team> {
        let row = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<String>,
                Option<String>,
                String,
                i32,
                String,
                String,
                String,
            ),
        >(
            r#"
            SELECT id, name, description, repository_url, owner_id, is_deleted,
                   created_at, updated_at, settings_json
            FROM teams
            WHERE id = ? AND is_deleted = 0
            "#,
        )
        .bind(team_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| TeamError::TeamNotFound(team_id.to_string()))?;

        let settings: TeamSettings = serde_json::from_str(&row.8).unwrap_or_default();

        Ok(Team {
            id: row.0,
            name: row.1,
            description: row.2,
            repository_url: row.3,
            owner_id: row.4,
            is_deleted: row.5 != 0,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.6)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.7)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            settings,
        })
    }

    /// Get team summary with counts
    pub async fn get_team_summary(
        &self,
        pool: &SqlitePool,
        team_id: &str,
    ) -> TeamResult<TeamSummary> {
        let team = self.get_team(pool, team_id).await?;

        let members_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM team_members WHERE team_id = ?")
                .bind(team_id)
                .fetch_one(pool)
                .await?;

        let skills_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM shared_skills WHERE team_id = ? AND is_deleted = 0",
        )
        .bind(team_id)
        .fetch_one(pool)
        .await?;

        let recipes_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM shared_recipes WHERE team_id = ? AND is_deleted = 0",
        )
        .bind(team_id)
        .fetch_one(pool)
        .await?;

        let extensions_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM shared_extensions WHERE team_id = ? AND is_deleted = 0",
        )
        .bind(team_id)
        .fetch_one(pool)
        .await?;

        Ok(TeamSummary {
            team,
            members_count: members_count.0 as u32,
            skills_count: skills_count.0 as u32,
            recipes_count: recipes_count.0 as u32,
            extensions_count: extensions_count.0 as u32,
        })
    }

    /// List teams for a user
    pub async fn list_teams(
        &self,
        pool: &SqlitePool,
        query: ListTeamsQuery,
        user_id: &str,
    ) -> TeamResult<PaginatedResponse<Team>> {
        let offset = (query.page.saturating_sub(1)) * query.limit;

        // Count total
        let total: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(DISTINCT t.id)
            FROM teams t
            LEFT JOIN team_members tm ON t.id = tm.team_id
            WHERE t.is_deleted = 0 AND (t.owner_id = ? OR tm.user_id = ?)
            "#,
        )
        .bind(user_id)
        .bind(user_id)
        .fetch_one(pool)
        .await?;

        // Fetch teams
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<String>,
                Option<String>,
                String,
                i32,
                String,
                String,
                String,
            ),
        >(
            r#"
            SELECT DISTINCT t.id, t.name, t.description, t.repository_url, t.owner_id,
                   t.is_deleted, t.created_at, t.updated_at, t.settings_json
            FROM teams t
            LEFT JOIN team_members tm ON t.id = tm.team_id
            WHERE t.is_deleted = 0 AND (t.owner_id = ? OR tm.user_id = ?)
            ORDER BY t.updated_at DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(user_id)
        .bind(user_id)
        .bind(query.limit as i64)
        .bind(offset as i64)
        .fetch_all(pool)
        .await?;

        let teams: Vec<Team> = rows
            .into_iter()
            .map(|row| {
                let settings: TeamSettings = serde_json::from_str(&row.8).unwrap_or_default();
                Team {
                    id: row.0,
                    name: row.1,
                    description: row.2,
                    repository_url: row.3,
                    owner_id: row.4,
                    is_deleted: row.5 != 0,
                    created_at: chrono::DateTime::parse_from_rfc3339(&row.6)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: chrono::DateTime::parse_from_rfc3339(&row.7)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    settings,
                }
            })
            .collect();

        Ok(PaginatedResponse::new(
            teams,
            total.0 as u64,
            query.page,
            query.limit,
        ))
    }

    /// Delete a team (soft delete) with cascade to related resources
    pub async fn delete_team(&self, pool: &SqlitePool, team_id: &str) -> TeamResult<()> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Use transaction to ensure atomicity
        let mut tx = pool.begin().await?;

        // Soft delete the team
        sqlx::query("UPDATE teams SET is_deleted = 1, updated_at = ? WHERE id = ?")
            .bind(&now_str)
            .bind(team_id)
            .execute(&mut *tx)
            .await?;

        // Soft delete related shared skills
        sqlx::query("UPDATE shared_skills SET is_deleted = 1 WHERE team_id = ?")
            .bind(team_id)
            .execute(&mut *tx)
            .await?;

        // Soft delete related shared recipes
        sqlx::query("UPDATE shared_recipes SET is_deleted = 1 WHERE team_id = ?")
            .bind(team_id)
            .execute(&mut *tx)
            .await?;

        // Soft delete related shared extensions
        sqlx::query("UPDATE shared_extensions SET is_deleted = 1 WHERE team_id = ?")
            .bind(team_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(())
    }

    /// Update a team
    pub async fn update_team(
        &self,
        pool: &SqlitePool,
        team_id: &str,
        request: UpdateTeamRequest,
    ) -> TeamResult<Team> {
        // First get the existing team
        let mut team = self.get_team(pool, team_id).await?;

        // Update fields if provided
        if let Some(name) = request.name {
            team.name = name;
        }
        if let Some(description) = request.description {
            team.description = Some(description);
        }
        if let Some(repository_url) = request.repository_url {
            team.repository_url = Some(repository_url);
        }
        if let Some(settings) = request.settings {
            team.settings = settings;
        }

        let now = Utc::now();
        team.updated_at = now;

        let settings_json = serde_json::to_string(&team.settings)?;

        sqlx::query(
            r#"
            UPDATE teams
            SET name = ?, description = ?, repository_url = ?, settings_json = ?, updated_at = ?
            WHERE id = ? AND is_deleted = 0
            "#,
        )
        .bind(&team.name)
        .bind(&team.description)
        .bind(&team.repository_url)
        .bind(&settings_json)
        .bind(team.updated_at.to_rfc3339())
        .bind(team_id)
        .execute(pool)
        .await?;

        Ok(team)
    }
}

impl Default for TeamService {
    fn default() -> Self {
        Self::new()
    }
}
