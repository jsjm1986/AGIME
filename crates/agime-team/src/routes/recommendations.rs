//! Recommendations HTTP routes

use axum::{
    extract::{Query, State},
    routing::get,
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::error::TeamError;
use crate::services::{Recommendation, RecommendationRequest, RecommendationService};
use crate::AuthenticatedUserId;

use super::{get_user_id, TeamState};

/// Query params for recommendations
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationsQuery {
    /// Team ID filter
    pub team_id: Option<String>,
    /// Resource type filter (skill, recipe, extension)
    pub resource_type: Option<String>,
    /// Maximum number of recommendations
    pub limit: Option<u32>,
    /// Context keywords for content-based filtering
    pub context: Option<String>,
}

/// Recommendation response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationResponse {
    pub resource_id: String,
    pub resource_type: String,
    pub resource_name: String,
    pub team_id: String,
    pub description: Option<String>,
    pub score: f64,
    pub reason: String,
    pub tags: Vec<String>,
}

impl From<Recommendation> for RecommendationResponse {
    fn from(rec: Recommendation) -> Self {
        Self {
            resource_id: rec.resource_id,
            resource_type: rec.resource_type.to_string(),
            resource_name: rec.resource_name,
            team_id: rec.team_id,
            description: rec.description,
            score: rec.score,
            reason: rec.reason.to_string(),
            tags: rec.tags,
        }
    }
}

/// Configure recommendation routes
pub fn routes(state: TeamState) -> Router {
    Router::new()
        .route("/recommendations", get(get_recommendations))
        .with_state(state)
}

/// Get recommendations for current user
async fn get_recommendations(
    State(state): State<TeamState>,
    auth_user: Option<Extension<AuthenticatedUserId>>,
    Query(query): Query<RecommendationsQuery>,
) -> Result<Json<Vec<RecommendationResponse>>, TeamError> {
    let service = RecommendationService::new();
    let user_id = get_user_id(auth_user.as_ref().map(|e| &e.0), &state);

    // Parse resource type if provided
    let resource_type = query.resource_type.and_then(|t| t.parse().ok());

    let request = RecommendationRequest {
        user_id: Some(user_id),
        team_id: query.team_id,
        resource_type,
        limit: query.limit,
        context: query.context,
        preferred_tags: None,
    };

    let recommendations = service.get_recommendations(&state.pool, request).await?;

    let response: Vec<RecommendationResponse> = recommendations
        .into_iter()
        .map(RecommendationResponse::from)
        .collect();

    Ok(Json(response))
}
