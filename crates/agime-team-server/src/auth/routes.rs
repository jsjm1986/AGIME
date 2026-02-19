//! Authentication API routes

use axum::{
    extract::{Path, State},
    http::{header::SET_COOKIE, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use super::middleware_sqlite::UserContext;
use super::service_sqlite::{AuthService, CreateApiKeyRequest, RegisterRequest};
use super::session_sqlite::SessionService;
use crate::state::AppState;

const SESSION_COOKIE_NAME: &str = "agime_session";

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
    let pool = match state.require_sqlite() {
        Ok(pool) => pool,
        Err(resp) => return resp,
    };
    let service = AuthService::new(pool);

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
        Err(e) => {
            tracing::warn!("Registration failed: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Registration failed"
                })),
            )
                .into_response()
        }
    }
}

/// Get current user info (requires auth)
async fn get_current_user(Extension(ctx): Extension<UserContext>) -> Response {
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
    let pool = match state.require_sqlite() {
        Ok(pool) => pool,
        Err(resp) => return resp,
    };
    let service = AuthService::new(pool);
    match service.list_api_keys(&ctx.user_id).await {
        Ok(keys) => (StatusCode::OK, Json(json!({ "keys": keys }))).into_response(),
        Err(e) => {
            tracing::error!("Failed to list API keys: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Failed to list API keys" })),
            )
                .into_response()
        }
    }
}

/// Create a new API key (requires auth)
async fn create_api_key(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<UserContext>,
    Json(request): Json<CreateApiKeyRequest>,
) -> Response {
    let pool = match state.require_sqlite() {
        Ok(pool) => pool,
        Err(resp) => return resp,
    };
    let service = AuthService::new(pool);
    match service.create_api_key(&ctx.user_id, request).await {
        Ok(response) => (
            StatusCode::CREATED,
            Json(json!({
                "key": response,
                "message": "Save your API key securely. It will only be shown once."
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to create API key: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Failed to create API key" })),
            )
                .into_response()
        }
    }
}

/// Revoke an API key (requires auth)
async fn revoke_api_key(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<UserContext>,
    Path(key_id): Path<String>,
) -> Response {
    let pool = match state.require_sqlite() {
        Ok(pool) => pool,
        Err(resp) => return resp,
    };
    let service = AuthService::new(pool);
    match service.revoke_api_key(&ctx.user_id, &key_id).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "message": "API key revoked" })),
        )
            .into_response(),
        Err(e) => {
            tracing::warn!("Failed to revoke API key: {}", e);
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "API key not found" })),
            )
                .into_response()
        }
    }
}

/// Login request
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub api_key: String,
}

/// Login with API key (public endpoint)
pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(request): Json<LoginRequest>,
) -> Response {
    let pool = match state.require_sqlite() {
        Ok(pool) => pool,
        Err(resp) => return resp,
    };
    let service = SessionService::new(pool);

    match service.create_session(&request.api_key).await {
        Ok((session, user)) => {
            let cookie = format!(
                "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800",
                SESSION_COOKIE_NAME, session.id
            );
            (
                StatusCode::OK,
                [(SET_COOKIE, cookie)],
                Json(json!({ "user": {
                    "id": user.id,
                    "email": user.email,
                    "display_name": user.display_name
                }})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!("Login failed: {}", e);
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Authentication failed" })),
            )
                .into_response()
        }
    }
}

/// Logout (public endpoint)
pub async fn logout(State(state): State<Arc<AppState>>, jar: CookieJar) -> Response {
    if let Some(cookie) = jar.get(SESSION_COOKIE_NAME) {
        if let Ok(pool) = state.require_sqlite() {
            let service = SessionService::new(pool);
            let _ = service.delete_session(cookie.value()).await;
        }
    }

    let clear_cookie = format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        SESSION_COOKIE_NAME
    );
    (
        StatusCode::OK,
        [(SET_COOKIE, clear_cookie)],
        Json(json!({ "message": "Logged out" })),
    )
        .into_response()
}

/// Get current session (public endpoint)
pub async fn get_session(State(state): State<Arc<AppState>>, jar: CookieJar) -> Response {
    let session_id = match jar.get(SESSION_COOKIE_NAME) {
        Some(cookie) => cookie.value(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "No session" })),
            )
                .into_response()
        }
    };

    let pool = match state.require_sqlite() {
        Ok(pool) => pool,
        Err(resp) => return resp,
    };
    let service = SessionService::new(pool);
    match service.validate_session(session_id).await {
        Ok(user) => (
            StatusCode::OK,
            Json(json!({ "user": {
                "id": user.id,
                "email": user.email,
                "display_name": user.display_name
            }})),
        )
            .into_response(),
        Err(_) => {
            let clear_cookie = format!(
                "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
                SESSION_COOKIE_NAME
            );
            (
                StatusCode::UNAUTHORIZED,
                [(SET_COOKIE, clear_cookie)],
                Json(json!({ "error": "Invalid session" })),
            )
                .into_response()
        }
    }
}
