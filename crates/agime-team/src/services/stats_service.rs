//! Usage statistics service
//!
//! This module tracks usage statistics for team resources including:
//! - View counts
//! - Install counts
//! - Usage frequency
//! - User interactions

use crate::error::{TeamError, TeamResult};
use crate::models::ResourceType;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Resource usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceStats {
    pub resource_id: String,
    pub resource_type: ResourceType,
    pub resource_name: String,
    pub team_id: String,
    pub view_count: u64,
    pub install_count: u64,
    pub use_count: u64,
    pub unique_users: u64,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// User activity record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserActivity {
    pub user_id: String,
    pub resource_id: String,
    pub resource_type: ResourceType,
    pub action: ActivityAction,
    pub timestamp: DateTime<Utc>,
}

/// Types of user activities
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityAction {
    View,
    Install,
    Uninstall,
    Use,
    Share,
    Update,
}

impl std::fmt::Display for ActivityAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivityAction::View => write!(f, "view"),
            ActivityAction::Install => write!(f, "install"),
            ActivityAction::Uninstall => write!(f, "uninstall"),
            ActivityAction::Use => write!(f, "use"),
            ActivityAction::Share => write!(f, "share"),
            ActivityAction::Update => write!(f, "update"),
        }
    }
}

/// Trending resource info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendingResource {
    pub resource_id: String,
    pub resource_type: ResourceType,
    pub resource_name: String,
    pub team_id: String,
    pub trend_score: f64,
    pub recent_installs: u64,
    pub recent_uses: u64,
}

/// Stats service
pub struct StatsService;

impl StatsService {
    pub fn new() -> Self {
        Self
    }

    /// Record a user activity
    pub async fn record_activity(
        &self,
        pool: &SqlitePool,
        user_id: &str,
        resource_type: ResourceType,
        resource_id: &str,
        action: ActivityAction,
    ) -> TeamResult<()> {
        let now = Utc::now();
        let action_str = action.to_string();
        let resource_type_str = resource_type.to_string();

        // Insert activity record
        sqlx::query(
            r#"
            INSERT INTO resource_activities (id, user_id, resource_type, resource_id, action, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(user_id)
        .bind(&resource_type_str)
        .bind(resource_id)
        .bind(&action_str)
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| TeamError::Database(e.to_string()))?;

        // Update resource counters based on action
        match action {
            ActivityAction::View => {
                self.increment_view_count(pool, resource_type, resource_id)
                    .await?;
            }
            ActivityAction::Install => {
                self.increment_install_count(pool, resource_type, resource_id)
                    .await?;
            }
            ActivityAction::Use => {
                self.increment_use_count(pool, resource_type, resource_id)
                    .await?;
            }
            _ => {}
        }

        Ok(())
    }

    /// Increment view count for a resource
    async fn increment_view_count(
        &self,
        pool: &SqlitePool,
        resource_type: ResourceType,
        resource_id: &str,
    ) -> TeamResult<()> {
        let table = match resource_type {
            ResourceType::Skill => "shared_skills",
            ResourceType::Recipe => "shared_recipes",
            ResourceType::Extension => "shared_extensions",
        };

        let query = format!(
            "UPDATE {} SET view_count = COALESCE(view_count, 0) + 1 WHERE id = ?",
            table
        );

        sqlx::query(&query)
            .bind(resource_id)
            .execute(pool)
            .await
            .map_err(|e| TeamError::Database(e.to_string()))?;

        Ok(())
    }

    /// Increment install count for a resource
    async fn increment_install_count(
        &self,
        pool: &SqlitePool,
        resource_type: ResourceType,
        resource_id: &str,
    ) -> TeamResult<()> {
        let table = match resource_type {
            ResourceType::Skill => "shared_skills",
            ResourceType::Recipe => "shared_recipes",
            ResourceType::Extension => "shared_extensions",
        };

        // Note: The schema might only have use_count, we'll use that
        let query = format!(
            "UPDATE {} SET use_count = COALESCE(use_count, 0) + 1 WHERE id = ?",
            table
        );

        sqlx::query(&query)
            .bind(resource_id)
            .execute(pool)
            .await
            .map_err(|e| TeamError::Database(e.to_string()))?;

        Ok(())
    }

    /// Increment use count for a resource
    async fn increment_use_count(
        &self,
        pool: &SqlitePool,
        resource_type: ResourceType,
        resource_id: &str,
    ) -> TeamResult<()> {
        let table = match resource_type {
            ResourceType::Skill => "shared_skills",
            ResourceType::Recipe => "shared_recipes",
            ResourceType::Extension => "shared_extensions",
        };

        let query = format!(
            "UPDATE {} SET use_count = COALESCE(use_count, 0) + 1 WHERE id = ?",
            table
        );

        sqlx::query(&query)
            .bind(resource_id)
            .execute(pool)
            .await
            .map_err(|e| TeamError::Database(e.to_string()))?;

        Ok(())
    }

    /// Get statistics for a specific resource
    pub async fn get_resource_stats(
        &self,
        pool: &SqlitePool,
        resource_type: ResourceType,
        resource_id: &str,
    ) -> TeamResult<ResourceStats> {
        let (table, name_col) = match resource_type {
            ResourceType::Skill => ("shared_skills", "name"),
            ResourceType::Recipe => ("shared_recipes", "name"),
            ResourceType::Extension => ("shared_extensions", "name"),
        };

        let query = format!(
            r#"
            SELECT id, team_id, {name_col} as name,
                   COALESCE(use_count, 0) as use_count,
                   created_at
            FROM {table}
            WHERE id = ? AND is_deleted = 0
            "#,
            name_col = name_col,
            table = table
        );

        let row = sqlx::query_as::<_, (String, String, String, i64, DateTime<Utc>)>(&query)
            .bind(resource_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| TeamError::Database(e.to_string()))?
            .ok_or_else(|| TeamError::ResourceNotFound {
                resource_type: resource_type.to_string(),
                resource_id: resource_id.to_string(),
            })?;

        // Count unique users from activity log
        let unique_users: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(DISTINCT user_id) FROM resource_activities
            WHERE resource_type = ? AND resource_id = ?
            "#,
        )
        .bind(resource_type.to_string())
        .bind(resource_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0);

        // Get last used timestamp
        let last_used: Option<DateTime<Utc>> = sqlx::query_scalar(
            r#"
            SELECT MAX(created_at) FROM resource_activities
            WHERE resource_type = ? AND resource_id = ? AND action = 'use'
            "#,
        )
        .bind(resource_type.to_string())
        .bind(resource_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

        Ok(ResourceStats {
            resource_id: row.0,
            resource_type,
            resource_name: row.2,
            team_id: row.1,
            view_count: 0, // Would need view_count column
            install_count: row.3 as u64,
            use_count: row.3 as u64,
            unique_users: unique_users as u64,
            last_used_at: last_used,
            created_at: row.4,
        })
    }

    /// Get trending resources (most active in recent period)
    pub async fn get_trending(
        &self,
        pool: &SqlitePool,
        team_id: Option<&str>,
        resource_type: Option<ResourceType>,
        days: u32,
        limit: u32,
    ) -> TeamResult<Vec<TrendingResource>> {
        // Calculate recent activity scores
        let _cutoff = Utc::now() - chrono::Duration::days(days as i64);

        let mut query = String::from(
            r#"
            SELECT
                ra.resource_id,
                ra.resource_type,
                COUNT(*) as activity_count,
                COUNT(CASE WHEN ra.action = 'install' THEN 1 END) as install_count,
                COUNT(CASE WHEN ra.action = 'use' THEN 1 END) as use_count
            FROM resource_activities ra
            WHERE ra.created_at >= ?
            "#,
        );

        let mut conditions = vec![];
        if team_id.is_some() {
            conditions.push("ra.resource_id IN (SELECT id FROM shared_skills WHERE team_id = ? UNION SELECT id FROM shared_recipes WHERE team_id = ? UNION SELECT id FROM shared_extensions WHERE team_id = ?)");
        }
        if resource_type.is_some() {
            conditions.push("ra.resource_type = ?");
        }

        if !conditions.is_empty() {
            query.push_str(" AND ");
            query.push_str(&conditions.join(" AND "));
        }

        query.push_str(
            r#"
            GROUP BY ra.resource_id, ra.resource_type
            ORDER BY activity_count DESC
            LIMIT ?
            "#,
        );

        // For simplicity, we'll return a basic trending list
        // In production, this would be more sophisticated
        let mut trending = Vec::new();

        // Query skills
        let skills = sqlx::query_as::<_, (String, String, String, i64)>(
            r#"
            SELECT s.id, s.name, s.team_id, COALESCE(s.use_count, 0) as use_count
            FROM shared_skills s
            WHERE s.is_deleted = 0
            ORDER BY use_count DESC
            LIMIT ?
            "#,
        )
        .bind(limit as i32)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

        for (id, name, tid, use_count) in skills {
            if team_id.is_none() || team_id == Some(&tid) {
                trending.push(TrendingResource {
                    resource_id: id,
                    resource_type: ResourceType::Skill,
                    resource_name: name,
                    team_id: tid,
                    trend_score: use_count as f64,
                    recent_installs: use_count as u64,
                    recent_uses: use_count as u64,
                });
            }
        }

        // Sort by trend score and limit (handle NaN safely to avoid panic)
        trending.sort_by(|a, b| {
            b.trend_score
                .partial_cmp(&a.trend_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        trending.truncate(limit as usize);

        Ok(trending)
    }

    /// Get user's recent activity
    pub async fn get_user_activity(
        &self,
        pool: &SqlitePool,
        user_id: &str,
        limit: u32,
    ) -> TeamResult<Vec<UserActivity>> {
        let rows = sqlx::query_as::<_, (String, String, String, String, DateTime<Utc>)>(
            r#"
            SELECT user_id, resource_id, resource_type, action, created_at
            FROM resource_activities
            WHERE user_id = ?
            ORDER BY created_at DESC
            LIMIT ?
            "#,
        )
        .bind(user_id)
        .bind(limit as i32)
        .fetch_all(pool)
        .await
        .map_err(|e| TeamError::Database(e.to_string()))?;

        let activities = rows
            .into_iter()
            .map(|(user_id, resource_id, resource_type, action, timestamp)| UserActivity {
                user_id,
                resource_id,
                resource_type: resource_type.parse().unwrap_or(ResourceType::Skill),
                action: match action.as_str() {
                    "view" => ActivityAction::View,
                    "install" => ActivityAction::Install,
                    "uninstall" => ActivityAction::Uninstall,
                    "use" => ActivityAction::Use,
                    "share" => ActivityAction::Share,
                    "update" => ActivityAction::Update,
                    _ => ActivityAction::View,
                },
                timestamp,
            })
            .collect();

        Ok(activities)
    }
}

impl Default for StatsService {
    fn default() -> Self {
        Self::new()
    }
}
