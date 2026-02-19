//! MongoDB routes - Smart Log API

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::teams::{is_team_member, AppState};
use crate::models::mongo::SmartLogSummary;
use crate::services::mongo::{SmartLogService, TeamService};
use crate::AuthenticatedUserId;

#[derive(Debug, Deserialize)]
pub struct SmartLogQuery {
    #[serde(rename = "resourceType")]
    pub resource_type: Option<String>,
    pub action: Option<String>,
    pub source: Option<String>,
    #[serde(rename = "userId")]
    pub user_id: Option<String>,
    pub page: Option<u64>,
    pub limit: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct SmartLogsResponse {
    pub items: Vec<SmartLogSummary>,
    pub total: u64,
    pub page: u64,
    pub limit: u64,
    #[serde(rename = "totalPages")]
    pub total_pages: u64,
}

pub fn smart_log_routes() -> Router<Arc<AppState>> {
    Router::new().route("/teams/{team_id}/smart-logs", get(list_smart_logs))
}

async fn list_smart_logs(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(query): Query<SmartLogQuery>,
) -> Result<Json<SmartLogsResponse>, (StatusCode, String)> {
    // Check team membership
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view smart logs".to_string(),
        ));
    }

    let service = SmartLogService::new((*state.db).clone());
    let result = service
        .query(
            &team_id,
            query.resource_type.as_deref(),
            query.action.as_deref(),
            query.source.as_deref(),
            query.user_id.as_deref(),
            query.page,
            query.limit,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(SmartLogsResponse {
        items: result.items,
        total: result.total,
        page: result.page,
        limit: result.limit,
        total_pages: result.total_pages,
    }))
}
