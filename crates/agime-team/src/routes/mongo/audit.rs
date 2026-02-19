//! MongoDB routes - Audit Log API

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

use super::teams::{is_team_member, AppState};
use crate::services::mongo::{AuditService, TeamService};
use crate::AuthenticatedUserId;

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub action: Option<String>,
    pub resource_type: Option<String>,
    pub user_id: Option<String>,
    pub page: Option<u64>,
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    pub limit: Option<u64>,
}

pub fn audit_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/teams/{team_id}/audit", get(query_audit))
        .route("/teams/{team_id}/activity", get(get_activity))
}

async fn query_audit(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<AuditQuery>,
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
            "Only team members can view audit logs".to_string(),
        ));
    }

    let service = AuditService::new((*state.db).clone());
    let result = service
        .query(
            &team_id,
            q.action.as_deref(),
            q.resource_type.as_deref(),
            q.user_id.as_deref(),
            q.page,
            q.limit,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "items": result.items,
        "total": result.total,
        "page": result.page,
        "limit": result.limit,
        "totalPages": result.total_pages,
    })))
}

async fn get_activity(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(team_id): Path<String>,
    Query(q): Query<ActivityQuery>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view activity".to_string(),
        ));
    }

    let service = AuditService::new((*state.db).clone());
    let items = service
        .get_team_activity(&team_id, q.limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let json_items: Vec<serde_json::Value> = items
        .into_iter()
        .map(|item| serde_json::to_value(item))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json_items))
}
