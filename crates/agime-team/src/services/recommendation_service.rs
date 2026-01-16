//! Recommendation service
//!
//! This module provides intelligent recommendations for team resources based on:
//! - User activity patterns
//! - Resource popularity
//! - Content similarity
//! - Team context

use crate::error::TeamResult;
use crate::models::ResourceType;
use crate::services::stats_service::StatsService;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// A recommended resource
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub resource_id: String,
    pub resource_type: ResourceType,
    pub resource_name: String,
    pub team_id: String,
    pub description: Option<String>,
    pub score: f64,
    pub reason: RecommendationReason,
    pub tags: Vec<String>,
}

/// Why a resource was recommended
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationReason {
    /// Popular among team members
    Popular,
    /// Based on user's past activity
    PersonalHistory,
    /// Similar to resources user has used
    SimilarContent,
    /// Trending recently
    Trending,
    /// New resource in the team
    New,
    /// Used by similar users
    CollaborativeFiltering,
}

impl std::fmt::Display for RecommendationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecommendationReason::Popular => write!(f, "popular"),
            RecommendationReason::PersonalHistory => write!(f, "personal_history"),
            RecommendationReason::SimilarContent => write!(f, "similar_content"),
            RecommendationReason::Trending => write!(f, "trending"),
            RecommendationReason::New => write!(f, "new"),
            RecommendationReason::CollaborativeFiltering => write!(f, "collaborative_filtering"),
        }
    }
}

/// Recommendation request parameters
#[derive(Debug, Clone, Deserialize)]
pub struct RecommendationRequest {
    /// User ID for personalized recommendations
    pub user_id: Option<String>,
    /// Team ID to filter by
    pub team_id: Option<String>,
    /// Resource type filter
    pub resource_type: Option<ResourceType>,
    /// Maximum number of recommendations
    pub limit: Option<u32>,
    /// Context keywords for content-based filtering
    pub context: Option<String>,
    /// Tags to prefer
    pub preferred_tags: Option<Vec<String>>,
}

/// Recommendation service
pub struct RecommendationService {
    stats_service: StatsService,
}

impl RecommendationService {
    pub fn new() -> Self {
        Self {
            stats_service: StatsService::new(),
        }
    }

    /// Get recommendations for a user
    pub async fn get_recommendations(
        &self,
        pool: &SqlitePool,
        request: RecommendationRequest,
    ) -> TeamResult<Vec<Recommendation>> {
        let limit = request.limit.unwrap_or(10);
        let mut recommendations = Vec::new();

        // 1. Get popular resources
        let popular = self
            .get_popular_resources(pool, request.team_id.as_deref(), request.resource_type, 5)
            .await?;
        recommendations.extend(popular);

        // 2. Get trending resources
        let trending = self
            .get_trending_resources(pool, request.team_id.as_deref(), request.resource_type, 5)
            .await?;
        recommendations.extend(trending);

        // 3. Get new resources
        let new = self
            .get_new_resources(pool, request.team_id.as_deref(), request.resource_type, 5)
            .await?;
        recommendations.extend(new);

        // 4. If user is provided, get personalized recommendations
        if let Some(ref user_id) = request.user_id {
            let personal = self
                .get_personalized_recommendations(
                    pool,
                    user_id,
                    request.team_id.as_deref(),
                    request.resource_type,
                    5,
                )
                .await?;
            recommendations.extend(personal);
        }

        // 5. If context is provided, do content-based filtering
        if let Some(ref context) = request.context {
            let content_based = self
                .get_content_based_recommendations(
                    pool,
                    context,
                    request.team_id.as_deref(),
                    request.resource_type,
                    5,
                )
                .await?;
            recommendations.extend(content_based);
        }

        // Deduplicate by resource_id and sort by score
        let mut seen = std::collections::HashSet::new();
        recommendations.retain(|r| seen.insert(r.resource_id.clone()));
        recommendations.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Limit results
        recommendations.truncate(limit as usize);

        Ok(recommendations)
    }

    /// Get popular resources
    async fn get_popular_resources(
        &self,
        pool: &SqlitePool,
        team_id: Option<&str>,
        resource_type: Option<ResourceType>,
        limit: u32,
    ) -> TeamResult<Vec<Recommendation>> {
        let mut recommendations = Vec::new();

        // Query skills
        if resource_type.is_none() || resource_type == Some(ResourceType::Skill) {
            let mut query = String::from(
                r#"
                SELECT id, name, team_id, description, tags_json, COALESCE(use_count, 0) as use_count
                FROM shared_skills
                WHERE is_deleted = 0
                "#,
            );
            if team_id.is_some() {
                query.push_str(" AND team_id = ?");
            }
            query.push_str(" ORDER BY use_count DESC LIMIT ?");

            let rows = if let Some(tid) = team_id {
                sqlx::query_as::<_, (String, String, String, Option<String>, String, i64)>(&query)
                    .bind(tid)
                    .bind(limit as i32)
                    .fetch_all(pool)
                    .await
            } else {
                sqlx::query_as::<_, (String, String, String, Option<String>, String, i64)>(&query)
                    .bind(limit as i32)
                    .fetch_all(pool)
                    .await
            }
            .unwrap_or_default();

            for (id, name, tid, description, tags_json, use_count) in rows {
                let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                recommendations.push(Recommendation {
                    resource_id: id,
                    resource_type: ResourceType::Skill,
                    resource_name: name,
                    team_id: tid,
                    description,
                    score: use_count as f64 * 1.0,
                    reason: RecommendationReason::Popular,
                    tags,
                });
            }
        }

        // Query recipes
        if resource_type.is_none() || resource_type == Some(ResourceType::Recipe) {
            let mut query = String::from(
                r#"
                SELECT id, name, team_id, description, tags_json, COALESCE(use_count, 0) as use_count
                FROM shared_recipes
                WHERE is_deleted = 0
                "#,
            );
            if team_id.is_some() {
                query.push_str(" AND team_id = ?");
            }
            query.push_str(" ORDER BY use_count DESC LIMIT ?");

            let rows = if let Some(tid) = team_id {
                sqlx::query_as::<_, (String, String, String, Option<String>, String, i64)>(&query)
                    .bind(tid)
                    .bind(limit as i32)
                    .fetch_all(pool)
                    .await
            } else {
                sqlx::query_as::<_, (String, String, String, Option<String>, String, i64)>(&query)
                    .bind(limit as i32)
                    .fetch_all(pool)
                    .await
            }
            .unwrap_or_default();

            for (id, name, tid, description, tags_json, use_count) in rows {
                let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                recommendations.push(Recommendation {
                    resource_id: id,
                    resource_type: ResourceType::Recipe,
                    resource_name: name,
                    team_id: tid,
                    description,
                    score: use_count as f64 * 1.0,
                    reason: RecommendationReason::Popular,
                    tags,
                });
            }
        }

        Ok(recommendations)
    }

    /// Get trending resources (recent activity spike)
    async fn get_trending_resources(
        &self,
        pool: &SqlitePool,
        team_id: Option<&str>,
        resource_type: Option<ResourceType>,
        limit: u32,
    ) -> TeamResult<Vec<Recommendation>> {
        let trending = self
            .stats_service
            .get_trending(pool, team_id, resource_type, 7, limit)
            .await?;

        let recommendations = trending
            .into_iter()
            .map(|t| Recommendation {
                resource_id: t.resource_id,
                resource_type: t.resource_type,
                resource_name: t.resource_name,
                team_id: t.team_id,
                description: None,
                score: t.trend_score * 1.5, // Boost trending items
                reason: RecommendationReason::Trending,
                tags: vec![],
            })
            .collect();

        Ok(recommendations)
    }

    /// Get new resources
    async fn get_new_resources(
        &self,
        pool: &SqlitePool,
        team_id: Option<&str>,
        resource_type: Option<ResourceType>,
        limit: u32,
    ) -> TeamResult<Vec<Recommendation>> {
        let mut recommendations = Vec::new();
        let cutoff = Utc::now() - chrono::Duration::days(7);

        // Query skills
        if resource_type.is_none() || resource_type == Some(ResourceType::Skill) {
            let mut query = String::from(
                r#"
                SELECT id, name, team_id, description, tags_json, created_at
                FROM shared_skills
                WHERE is_deleted = 0 AND created_at >= ?
                "#,
            );
            if team_id.is_some() {
                query.push_str(" AND team_id = ?");
            }
            query.push_str(" ORDER BY created_at DESC LIMIT ?");

            let rows = if let Some(tid) = team_id {
                sqlx::query_as::<_, (String, String, String, Option<String>, String, DateTime<Utc>)>(
                    &query,
                )
                .bind(cutoff)
                .bind(tid)
                .bind(limit as i32)
                .fetch_all(pool)
                .await
            } else {
                sqlx::query_as::<_, (String, String, String, Option<String>, String, DateTime<Utc>)>(
                    &query,
                )
                .bind(cutoff)
                .bind(limit as i32)
                .fetch_all(pool)
                .await
            }
            .unwrap_or_default();

            for (id, name, tid, description, tags_json, created_at) in rows {
                let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                // Score based on how new it is
                let age_hours = (Utc::now() - created_at).num_hours().max(1) as f64;
                let score = 100.0 / age_hours.sqrt();

                recommendations.push(Recommendation {
                    resource_id: id,
                    resource_type: ResourceType::Skill,
                    resource_name: name,
                    team_id: tid,
                    description,
                    score,
                    reason: RecommendationReason::New,
                    tags,
                });
            }
        }

        Ok(recommendations)
    }

    /// Get personalized recommendations based on user history
    async fn get_personalized_recommendations(
        &self,
        pool: &SqlitePool,
        user_id: &str,
        _team_id: Option<&str>,
        _resource_type: Option<ResourceType>,
        limit: u32,
    ) -> TeamResult<Vec<Recommendation>> {
        // Get user's recent activity
        let activities = self.stats_service.get_user_activity(pool, user_id, 50).await?;

        if activities.is_empty() {
            return Ok(vec![]);
        }

        // Collect tags from user's used resources
        let mut used_resource_ids: Vec<&str> = activities
            .iter()
            .map(|a| a.resource_id.as_str())
            .collect();
        used_resource_ids.dedup();

        // Find resources with similar tags that user hasn't used
        let mut recommendations = Vec::new();

        // Get tags from user's resources
        let mut user_tags: Vec<String> = Vec::new();
        for resource_id in used_resource_ids.iter().take(10) {
            let tags: Option<String> = sqlx::query_scalar(
                "SELECT tags_json FROM shared_skills WHERE id = ? UNION SELECT tags_json FROM shared_recipes WHERE id = ?",
            )
            .bind(resource_id)
            .bind(resource_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

            if let Some(tags_json) = tags {
                let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                user_tags.extend(tags);
            }
        }

        // Find resources with matching tags that user hasn't used
        if !user_tags.is_empty() {
            let tag_pattern = user_tags.first().cloned().unwrap_or_default();

            let rows = sqlx::query_as::<_, (String, String, String, Option<String>, String, i64)>(
                r#"
                SELECT id, name, team_id, description, tags_json, COALESCE(use_count, 0) as use_count
                FROM shared_skills
                WHERE is_deleted = 0 AND tags_json LIKE ?
                ORDER BY use_count DESC
                LIMIT ?
                "#,
            )
            .bind(format!("%{}%", tag_pattern))
            .bind(limit as i32)
            .fetch_all(pool)
            .await
            .unwrap_or_default();

            for (id, name, tid, description, tags_json, use_count) in rows {
                if !used_resource_ids.contains(&id.as_str()) {
                    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                    recommendations.push(Recommendation {
                        resource_id: id,
                        resource_type: ResourceType::Skill,
                        resource_name: name,
                        team_id: tid,
                        description,
                        score: use_count as f64 * 1.2,
                        reason: RecommendationReason::PersonalHistory,
                        tags,
                    });
                }
            }
        }

        Ok(recommendations)
    }

    /// Get content-based recommendations
    async fn get_content_based_recommendations(
        &self,
        pool: &SqlitePool,
        context: &str,
        team_id: Option<&str>,
        resource_type: Option<ResourceType>,
        limit: u32,
    ) -> TeamResult<Vec<Recommendation>> {
        let mut recommendations = Vec::new();

        // Simple keyword matching in name and description
        let search_pattern = format!("%{}%", context);

        // Search skills
        if resource_type.is_none() || resource_type == Some(ResourceType::Skill) {
            let mut query = String::from(
                r#"
                SELECT id, name, team_id, description, tags_json, COALESCE(use_count, 0) as use_count
                FROM shared_skills
                WHERE is_deleted = 0 AND (name LIKE ? OR description LIKE ? OR content LIKE ?)
                "#,
            );
            if team_id.is_some() {
                query.push_str(" AND team_id = ?");
            }
            query.push_str(" ORDER BY use_count DESC LIMIT ?");

            let rows = if let Some(tid) = team_id {
                sqlx::query_as::<_, (String, String, String, Option<String>, String, i64)>(&query)
                    .bind(&search_pattern)
                    .bind(&search_pattern)
                    .bind(&search_pattern)
                    .bind(tid)
                    .bind(limit as i32)
                    .fetch_all(pool)
                    .await
            } else {
                sqlx::query_as::<_, (String, String, String, Option<String>, String, i64)>(&query)
                    .bind(&search_pattern)
                    .bind(&search_pattern)
                    .bind(&search_pattern)
                    .bind(limit as i32)
                    .fetch_all(pool)
                    .await
            }
            .unwrap_or_default();

            for (id, name, tid, description, tags_json, use_count) in rows {
                let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                recommendations.push(Recommendation {
                    resource_id: id,
                    resource_type: ResourceType::Skill,
                    resource_name: name,
                    team_id: tid,
                    description,
                    score: use_count as f64 * 1.3, // Boost content matches
                    reason: RecommendationReason::SimilarContent,
                    tags,
                });
            }
        }

        Ok(recommendations)
    }
}

impl Default for RecommendationService {
    fn default() -> Self {
        Self::new()
    }
}
