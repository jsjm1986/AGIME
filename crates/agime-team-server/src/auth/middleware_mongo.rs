//! Authentication middleware (MongoDB version)

use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;
use tracing::{debug, warn};

use crate::auth::service_mongo::{AuthService, UserPreferences};
use crate::auth::session_mongo::SessionService;
use crate::auth::system_admin_session_mongo::{
    SystemAdminSessionService, SYSTEM_ADMIN_SESSION_COOKIE_NAME,
};
use crate::state::AppState;

// Import AuthenticatedUserId from agime_team
use agime_team::AuthenticatedUserId;

const SESSION_COOKIE_NAME: &str = "agime_session";

/// User context extracted from authentication
#[derive(Clone, Debug)]
pub struct UserContext {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub preferences: UserPreferences,
}

/// Dedicated system-admin context extracted from isolated admin authentication.
#[derive(Clone, Debug)]
pub struct SystemAdminContext {
    pub admin_id: String,
    pub username: String,
    pub display_name: String,
}

fn extract_cookie_value(request: &Request<Body>, cookie_name: &str) -> Option<String> {
    request
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookie_str| {
            for cookie in cookie_str.split(';') {
                let cookie = cookie.trim();
                if let Some(value) = cookie.strip_prefix(&format!("{}=", cookie_name)) {
                    return Some(value.to_string());
                }
            }
            None
        })
}

/// Authentication middleware for admin routes.
/// Only the dedicated system-admin session is accepted here.
pub async fn admin_auth_middleware(
    State(state): State<Arc<AppState>>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let session_id = match extract_cookie_value(&request, SYSTEM_ADMIN_SESSION_COOKIE_NAME) {
        Some(session_id) => session_id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "System admin session required",
                    "hint": "Login via /system-admin/login"
                })),
            )
                .into_response();
        }
    };

    let db = match state.db.as_mongodb() {
        Some(db) => db.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "Database not available"})),
            )
                .into_response();
        }
    };

    let session_service = SystemAdminSessionService::new(db);
    match session_service.validate_session(&session_id).await {
        Ok(admin) => {
            let admin_context = SystemAdminContext {
                admin_id: admin.id.clone(),
                username: admin.username.clone(),
                display_name: admin.display_name.clone(),
            };
            request.extensions_mut().insert(admin_context);

            let sliding_hours = state.config.session_sliding_window_hours as i64;
            let sid = session_id.clone();
            if let Some(mongo_db) = state.db.as_mongodb() {
                let ss = SystemAdminSessionService::new(mongo_db.clone());
                tokio::spawn(async move {
                    if let Err(e) = ss.try_extend_session(&sid, sliding_hours, 7).await {
                        tracing::warn!("System admin session sliding renewal failed: {}", e);
                    }
                });
            }

            next.run(request).await
        }
        Err(e) => {
            warn!("System admin session validation failed: {}", e);
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "System admin authentication failed"
                })),
            )
                .into_response()
        }
    }
}

/// Authentication middleware
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let request_path = request.uri().path().to_string();
    let request_method = request.method().clone();
    debug!("Auth middleware processing request: {}", request_path);

    // Public invite validation endpoint:
    // GET /api/team/invites/{code}
    // Keep accept endpoint authenticated in Mongo mode (handler requires user context).
    if request_method == axum::http::Method::GET {
        for prefix in ["/api/team/invites/", "/invites/"] {
            if request_path.starts_with(prefix)
                && !request_path[prefix.len()..].contains('/')
                && !request_path[prefix.len()..].is_empty()
            {
                debug!(
                    "Bypassing auth for public invite validation: {}",
                    request_path
                );
                return next.run(request).await;
            }
        }
    }

    // First, try to authenticate via session cookie
    let session_user = {
        let cookie_header = request.headers().get(header::COOKIE);
        debug!("Cookie header present: {}", cookie_header.is_some());

        let cookie_opt = extract_cookie_value(&request, SESSION_COOKIE_NAME);

        if let Some(session_id) = cookie_opt {
            // Get MongoDB connection from DatabaseBackend
            let db = match state.db.as_mongodb() {
                Some(db) => db.clone(),
                None => {
                    warn!("MongoDB not available for session validation");
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(serde_json::json!({"error": "Database not available"})),
                    )
                        .into_response();
                }
            };
            let session_service = SessionService::new(db);
            match session_service.validate_session(&session_id).await {
                Ok(user) => {
                    debug!("Session validated for user: {}", user.id);
                    // Sliding window renewal in background
                    let sliding_hours = state.config.session_sliding_window_hours as i64;
                    let sid = session_id.clone();
                    if let Some(mongo_db) = state.db.as_mongodb() {
                        let ss = SessionService::new(mongo_db.clone());
                        tokio::spawn(async move {
                            if let Err(e) = ss.try_extend_session(&sid, sliding_hours, 7).await {
                                tracing::warn!("Session sliding renewal failed: {}", e);
                            }
                        });
                    }
                    Some(user)
                }
                Err(e) => {
                    warn!("Session validation failed: {}", e);
                    None
                }
            }
        } else {
            debug!("No session cookie found");
            None
        }
    };

    if let Some(user) = session_user {
        debug!("Authenticated via session cookie");
        let user_context = UserContext {
            user_id: user.id.clone(),
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            role: user.role.clone(),
            preferences: user.preferences.clone(),
        };
        request.extensions_mut().insert(user_context);
        request
            .extensions_mut()
            .insert(AuthenticatedUserId(user.id));
        return next.run(request).await;
    }

    // Fall back to API key authentication
    let api_key = extract_api_key(&request);
    debug!("API key present: {}", api_key.is_some());

    let api_key = match api_key {
        Some(key) => key,
        None => {
            warn!("No authentication found for request: {}", request_path);
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Missing API key",
                    "hint": "Provide X-API-Key header, Authorization: Bearer <key>, or login via session"
                })),
            )
                .into_response();
        }
    };

    // Verify API key
    let db = match state.db.as_mongodb() {
        Some(db) => db.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "Database not available"})),
            )
                .into_response();
        }
    };
    let auth_service = AuthService::new(db);
    match auth_service.verify_api_key(&api_key).await {
        Ok((user, key_id)) => {
            // Update last used in background (non-blocking)
            tokio::spawn(async move {
                let _ = auth_service.update_key_last_used_by_id(&key_id).await;
            });
            let user_context = UserContext {
                user_id: user.id.clone(),
                email: user.email.clone(),
                display_name: user.display_name.clone(),
                role: user.role.clone(),
                preferences: user.preferences.clone(),
            };
            request.extensions_mut().insert(user_context);
            request
                .extensions_mut()
                .insert(AuthenticatedUserId(user.id));
            next.run(request).await
        }
        Err(e) => {
            warn!("API key verification failed: {}", e);
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Authentication failed"
                })),
            )
                .into_response()
        }
    }
}

fn extract_api_key(request: &Request<Body>) -> Option<String> {
    // Check X-API-Key header
    if let Some(value) = request.headers().get("X-API-Key") {
        return value.to_str().ok().map(|s| s.to_string());
    }

    // Check Authorization: Bearer header
    if let Some(value) = request.headers().get(header::AUTHORIZATION) {
        return value
            .to_str()
            .ok()
            .and_then(|auth_str| auth_str.strip_prefix("Bearer ").map(|s| s.to_string()));
    }

    None
}
