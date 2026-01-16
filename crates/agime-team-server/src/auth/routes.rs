//! Authentication API routes

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use serde_json::json;
use std::sync::Arc;

use super::middleware::UserContext;
use super::service::{AuthService, CreateApiKeyRequest, RegisterRequest};
use crate::state::AppState;

/// Configure protected auth routes (require authentication)
pub fn protected_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/me", get(get_current_user))
        .route("/keys", get(list_api_keys))
        .route("/keys", post(create_api_key))
        .route("/keys/{key_id}", delete(revoke_api_key))
}

/// Register a new user (public endpoint)
pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RegisterRequest>,
) -> Response {
    let service = AuthService::new(state.pool.clone());

    match service.register(request).await {
        Ok(response) => (
            StatusCode::CREATED,
            Json(json!({
                "user": response.user,
                "api_key": response.api_key,
                "message": "Save your API key securely. It will only be shown once."
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": e.to_string()
            })),
        )
            .into_response(),
    }
}

/// Get current user info (requires auth)
async fn get_current_user(
    Extension(ctx): Extension<UserContext>,
) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "id": ctx.user_id,
            "email": ctx.email,
            "display_name": ctx.display_name
        })),
    )
        .into_response()
}

/// List API keys for current user (requires auth)
async fn list_api_keys(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<UserContext>,
) -> Response {
    let service = AuthService::new(state.pool.clone());
    match service.list_api_keys(&ctx.user_id).await {
        Ok(keys) => (StatusCode::OK, Json(json!({ "keys": keys }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Create a new API key (requires auth)
async fn create_api_key(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<UserContext>,
    Json(request): Json<CreateApiKeyRequest>,
) -> Response {
    let service = AuthService::new(state.pool.clone());
    match service.create_api_key(&ctx.user_id, request).await {
        Ok(response) => (
            StatusCode::CREATED,
            Json(json!({
                "key": response,
                "message": "Save your API key securely. It will only be shown once."
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Revoke an API key (requires auth)
async fn revoke_api_key(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<UserContext>,
    Path(key_id): Path<String>,
) -> Response {
    let service = AuthService::new(state.pool.clone());
    match service.revoke_api_key(&ctx.user_id, &key_id).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "message": "API key revoked" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
