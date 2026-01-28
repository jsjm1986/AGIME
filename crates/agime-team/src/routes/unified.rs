//! Unified API Routes
//! Provides aggregated resource queries across multiple data sources

use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::teams::TeamState;

/// Data source info for API responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceInfo {
    pub id: String,
    pub source_type: String,
    pub name: String,
    pub url: String,
    pub status: String,
    pub teams_count: Option<i32>,
    pub last_sync_at: Option<String>,
}

/// List data sources response
#[derive(Debug, Serialize)]
pub struct ListSourcesResponse {
    pub sources: Vec<DataSourceInfo>,
    pub total: i32,
}

/// Cached resource info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResourceInfo {
    pub id: String,
    pub source_id: String,
    pub resource_type: String,
    pub resource_id: String,
    pub cached_at: String,
    pub expires_at: Option<String>,
    pub sync_status: String,
}

/// List cached resources response
#[derive(Debug, Serialize)]
pub struct ListCachedResponse {
    pub resources: Vec<CachedResourceInfo>,
    pub total: i32,
}

/// Query params for listing sources
#[derive(Debug, Deserialize)]
pub struct ListSourcesQuery {
    pub source_type: Option<String>,
}

/// List all data sources
async fn list_sources(
    State(state): State<TeamState>,
    Query(query): Query<ListSourcesQuery>,
) -> Json<ListSourcesResponse> {
    let sources = sqlx::query_as::<_, (String, String, String, String, String, Option<i32>, Option<String>)>(
        r#"
        SELECT id, type, name, url, status, teams_count, last_sync_at
        FROM data_sources
        WHERE (? IS NULL OR type = ?)
        ORDER BY type, name
        "#,
    )
    .bind(&query.source_type)
    .bind(&query.source_type)
    .fetch_all(state.pool.as_ref())
    .await
    .unwrap_or_default();

    let sources: Vec<DataSourceInfo> = sources
        .into_iter()
        .map(|r| DataSourceInfo {
            id: r.0,
            source_type: r.1,
            name: r.2,
            url: r.3,
            status: r.4,
            teams_count: r.5,
            last_sync_at: r.6,
        })
        .collect();

    let total = sources.len() as i32;
    Json(ListSourcesResponse { sources, total })
}

/// Configure unified routes
pub fn routes(state: TeamState) -> Router {
    Router::new()
        .route("/unified/sources", get(list_sources))
        .with_state(state)
}
