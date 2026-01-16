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

use crate::auth::service::AuthService;
use crate::state::AppState;

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
    // Extract API key from header
    let api_key = match request.headers().get("X-API-Key") {
        Some(value) => match value.to_str() {
            Ok(key) => key.to_string(),
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "Invalid X-API-Key header"
                    })),
                )
                    .into_response();
            }
        },
        None => {
            // Also check Authorization header for Bearer token
            match request.headers().get(header::AUTHORIZATION) {
                Some(value) => {
                    let auth_str = match value.to_str() {
                        Ok(s) => s,
                        Err(_) => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(serde_json::json!({
                                    "error": "Invalid Authorization header"
                                })),
                            )
                                .into_response();
                        }
                    };
                    if auth_str.starts_with("Bearer ") {
                        auth_str.trim_start_matches("Bearer ").to_string()
                    } else {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(serde_json::json!({
                                "error": "Missing API key",
                                "hint": "Provide X-API-Key header or Authorization: Bearer <key>"
                            })),
                        )
                            .into_response();
                    }
                }
                None => {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({
                            "error": "Missing API key",
                            "hint": "Provide X-API-Key header or Authorization: Bearer <key>"
                        })),
                    )
                        .into_response();
                }
            }
        }
    };

    // Verify API key
    let auth_service = AuthService::new(state.pool.clone());
    match auth_service.verify_api_key(&api_key).await {
        Ok(user) => {
            // Update last used timestamp
            let _ = auth_service.update_key_last_used(&api_key).await;

            // Create user context
            let user_context = UserContext {
                user_id: user.id.clone(),
                email: user.email.clone(),
                display_name: user.display_name.clone(),
            };

            // Insert user context into request extensions
            request.extensions_mut().insert(user_context.clone());

            // Also update the team routes state with user_id
            // This is done by modifying the request path or using a custom extension
            request.extensions_mut().insert(user.id);

            next.run(request).await
        }
        Err(e) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "Invalid API key",
                "message": e.to_string()
            })),
        )
            .into_response(),
    }
}
