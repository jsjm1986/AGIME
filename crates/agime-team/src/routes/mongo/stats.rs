//! MongoDB routes - Stats & Recommendations API

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

use super::teams::{is_team_member, AppState};
use crate::services::mongo::{RecommendationService, StatsService, TeamService};
use crate::AuthenticatedUserId;

#[derive(Debug, Deserialize)]
pub struct LimitQuery {
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationsCompatQuery {
    pub team_id: Option<String>,
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationsCompatBody {
    pub team_id: Option<String>,
    pub limit: Option<u64>,
}

pub fn stats_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/teams/{team_id}/stats", get(get_stats))
        .route("/teams/{team_id}/trending", get(get_trending))
        .route("/teams/{team_id}/recommendations/popular", get(get_popular))
        .route("/teams/{team_id}/recommendations/newest", get(get_newest))
        // Compatibility endpoints used by local clients/MCP.
        .route("/recommendations", get(get_recommendations_compat))
        .route("/recommendations", post(post_recommendations_compat))
}

async fn get_stats(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view stats".to_string(),
        ));
    }

    let service = StatsService::new((*state.db).clone());
    let stats = service
        .get_resource_stats(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::to_value(stats).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?))
}

async fn get_trending(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view trending".to_string(),
        ));
    }

    let service = StatsService::new((*state.db).clone());
    let items = service
        .get_trending(&team_id, q.limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::to_value(items).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?))
}

async fn get_popular(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view recommendations".to_string(),
        ));
    }

    let service = RecommendationService::new((*state.db).clone());
    let items = service
        .get_popular(&team_id, q.limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::to_value(items).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?))
}

async fn get_newest(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view recommendations".to_string(),
        ));
    }

    let service = RecommendationService::new((*state.db).clone());
    let items = service
        .get_newest(&team_id, q.limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::to_value(items).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?))
}

async fn get_recommendations_compat(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Query(q): Query<RecommendationsCompatQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());

    let service = RecommendationService::new((*state.db).clone());
    let items = if let Some(team_id) = q.team_id {
        let team = team_service
            .get(&team_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

        if !is_team_member(&team, &user.0) {
            return Err((
                StatusCode::FORBIDDEN,
                "Only team members can view recommendations".to_string(),
            ));
        }

        service
            .get_popular(&team_id, q.limit)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        let teams = team_service
            .list_for_user(&user.0)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let mut merged = Vec::new();
        for team in teams {
            if let Ok(mut per_team) = service.get_popular(&team.id, q.limit).await {
                merged.append(&mut per_team);
            }
        }
        merged.sort_by(|a, b| b.use_count.cmp(&a.use_count));
        let max = q.limit.unwrap_or(10).min(50) as usize;
        merged.truncate(max);
        merged
    };

    Ok(Json(serde_json::to_value(items).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?))
}

async fn post_recommendations_compat(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Json(body): Json<RecommendationsCompatBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let query = RecommendationsCompatQuery {
        team_id: body.team_id,
        limit: body.limit,
    };
    get_recommendations_compat(State(state), Extension(user), Query(query)).await
}
