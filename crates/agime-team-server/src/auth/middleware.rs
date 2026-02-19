//! Authentication middleware

use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;

use crate::auth::service_sqlite::AuthService;
use crate::auth::session_sqlite::SessionService;
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
}

/// Authentication middleware
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let request_path = request.uri().path().to_string();
    let request_method = request.method().clone();

    // Public invite validation endpoint:
    // GET /api/team/invites/{code}
    if request_method == axum::http::Method::GET {
        let prefix = "/api/team/invites/";
        if request_path.starts_with(prefix)
            && !request_path[prefix.len()..].contains('/')
            && !request_path[prefix.len()..].is_empty()
        {
            return next.run(request).await;
        }
    }

    // First, try to authenticate via session cookie
    let session_user = {
        let cookie_opt = request
            .headers()
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|cookie_str| {
                for cookie in cookie_str.split(';') {
                    let cookie = cookie.trim();
                    if let Some(value) = cookie.strip_prefix(&format!("{}=", SESSION_COOKIE_NAME)) {
                        return Some(value.to_string());
                    }
                }
                None
            });

        if let Some(session_id) = cookie_opt {
            let pool = match state.require_sqlite() {
                Ok(pool) => pool,
                Err(resp) => return resp,
            };
            let session_service = SessionService::new(pool);
            session_service.validate_session(&session_id).await.ok()
        } else {
            None
        }
    };

    if let Some(user) = session_user {
        let user_context = UserContext {
            user_id: user.id.clone(),
            email: user.email.clone(),
            display_name: user.display_name.clone(),
        };
        request.extensions_mut().insert(user_context);
        request
            .extensions_mut()
            .insert(AuthenticatedUserId(user.id));
        return next.run(request).await;
    }

    // Fall back to API key authentication
    let api_key = {
        // Check X-API-Key header
        if let Some(value) = request.headers().get("X-API-Key") {
            value.to_str().ok().map(|s| s.to_string())
        } else if let Some(value) = request.headers().get(header::AUTHORIZATION) {
            // Check Authorization: Bearer header
            value
                .to_str()
                .ok()
                .and_then(|auth_str| auth_str.strip_prefix("Bearer ").map(|s| s.to_string()))
        } else {
            None
        }
    };

    let api_key = match api_key {
        Some(key) => key,
        None => {
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
    let pool = match state.require_sqlite() {
        Ok(pool) => pool,
        Err(resp) => return resp,
    };
    let auth_service = AuthService::new(pool);
    match auth_service.verify_api_key(&api_key).await {
        Ok(user) => {
            let _ = auth_service.update_key_last_used(&api_key).await;
            let user_context = UserContext {
                user_id: user.id.clone(),
                email: user.email.clone(),
                display_name: user.display_name.clone(),
            };
            request.extensions_mut().insert(user_context);
            request
                .extensions_mut()
                .insert(AuthenticatedUserId(user.id));
            next.run(request).await
        }
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "Authentication failed"
            })),
        )
            .into_response(),
    }
}
