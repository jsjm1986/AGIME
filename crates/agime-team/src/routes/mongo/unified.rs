//! MongoDB routes - Unified search API
//! Provides cross-resource-type aggregated queries

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::teams::{is_team_member, AppState};
use crate::services::mongo::{ExtensionService, RecipeService, SkillService, TeamService};
use crate::AuthenticatedUserId;

#[derive(Debug, Deserialize)]
pub struct UnifiedSearchQuery {
    pub q: Option<String>,
    pub limit: Option<u32>,
    /// Filter by resource type: skill, recipe, extension, or all (default)
    pub resource_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UnifiedSearchItem {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(rename = "authorId")]
    pub author_id: String,
    pub version: String,
    #[serde(rename = "useCount")]
    pub use_count: i64,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct UnifiedSearchResponse {
    pub items: Vec<UnifiedSearchItem>,
    pub total: usize,
}

pub fn unified_routes() -> Router<Arc<AppState>> {
    Router::new().route("/teams/{team_id}/search", get(unified_search))
}

async fn unified_search(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<UnifiedSearchQuery>,
) -> Result<Json<UnifiedSearchResponse>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can search".to_string(),
        ));
    }

    let limit = q.limit.unwrap_or(20).min(100) as u64;
    let search = q.q.as_deref();
    let resource_type = q.resource_type.as_deref().unwrap_or("all");

    let mut items: Vec<UnifiedSearchItem> = Vec::new();

    // Fetch skills
    if resource_type == "all" || resource_type == "skill" {
        let skill_service = SkillService::new((*state.db).clone());
        let result = skill_service
            .list(&team_id, Some(1), Some(limit), search, Some("updated_at"))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        for s in result.items {
            items.push(UnifiedSearchItem {
                id: s.id,
                name: s.name,
                description: s.description,
                resource_type: "skill".to_string(),
                author_id: s.author_id,
                version: s.version,
                use_count: s.use_count as i64,
                updated_at: s.updated_at.to_rfc3339(),
            });
        }
    }

    // Fetch recipes
    if resource_type == "all" || resource_type == "recipe" {
        let recipe_service = RecipeService::new((*state.db).clone());
        let result = recipe_service
            .list(&team_id, Some(1), Some(limit), search, Some("updated_at"))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        for r in result.items {
            items.push(UnifiedSearchItem {
                id: r.id,
                name: r.name,
                description: r.description,
                resource_type: "recipe".to_string(),
                author_id: r.author_id,
                version: r.version,
                use_count: r.use_count as i64,
                updated_at: r.updated_at.to_rfc3339(),
            });
        }
    }

    // Fetch extensions
    if resource_type == "all" || resource_type == "extension" {
        let ext_service = ExtensionService::new((*state.db).clone());
        let result = ext_service
            .list(&team_id, Some(1), Some(limit), search, Some("updated_at"))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        for e in result.items {
            items.push(UnifiedSearchItem {
                id: e.id,
                name: e.name,
                description: e.description,
                resource_type: "extension".to_string(),
                author_id: e.author_id,
                version: e.version,
                use_count: e.use_count as i64,
                updated_at: e.updated_at.to_rfc3339(),
            });
        }
    }

    // Sort all items by updated_at descending
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    // Trim to limit
    let total = items.len();
    items.truncate(limit as usize);

    Ok(Json(UnifiedSearchResponse { items, total }))
}
