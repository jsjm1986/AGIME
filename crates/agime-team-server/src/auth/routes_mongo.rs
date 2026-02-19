//! Authentication API routes (MongoDB version)

use axum::{
    extract::{Path, State},
    http::{header::SET_COOKIE, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use super::middleware_mongo::UserContext;
use super::service_mongo::{AuthService, CreateApiKeyRequest, RegisterRequest};
use super::session_mongo::SessionService;
use crate::state::AppState;

const SESSION_COOKIE_NAME: &str = "agime_session";

/// Extract client IP from X-Forwarded-For header
fn extract_client_ip(headers: &HeaderMap) -> String {
    if let Some(xff) = headers.get("X-Forwarded-For") {
        if let Ok(s) = xff.to_str() {
            if let Some(first) = s.split(',').next() {
                return first.trim().to_string();
            }
        }
    }
    "unknown".to_string()
}

/// Build cookie string with optional Secure flag
fn build_session_cookie(session_id: &str, secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800{}",
        SESSION_COOKIE_NAME, session_id, secure_flag
    )
}

/// Build clear-cookie string with optional Secure flag
fn build_clear_cookie(secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}",
        SESSION_COOKIE_NAME, secure_flag
    )
}

/// Configure protected auth routes (require authentication)
pub fn protected_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/me", get(get_current_user))
        .route("/keys", get(list_api_keys))
        .route("/keys", post(create_api_key))
        .route("/keys/{key_id}", delete(revoke_api_key))
        .route("/deactivate", post(deactivate_account))
        .route("/change-password", post(change_password))
}

/// Register a new user (public endpoint, with rate limiting and registration mode)
pub async fn register(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<RegisterRequest>,
) -> Response {
    // Rate limiting
    let client_ip = extract_client_ip(&headers);
    if let Some(ref limiter) = state.register_limiter {
        if !limiter.check(&client_ip).await {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error": "Too many requests, please try again later"})),
            )
                .into_response();
        }
    }

    // Check registration mode
    match state.config.registration_mode.as_str() {
        "disabled" => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "Registration is disabled"})),
            )
                .into_response();
        }
        "approval" => {
            return register_approval(state, request).await;
        }
        _ => {} // "open" - continue with normal registration
    }

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db)
        .with_admin_emails(state.config.admin_emails.clone());

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
            tracing::error!("Registration failed: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Registration failed"})),
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
            "display_name": ctx.display_name,
            "role": ctx.role
        })),
    )
        .into_response()
}

/// List API keys for current user (requires auth)
async fn list_api_keys(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<UserContext>,
) -> Response {
    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);
    match service.list_api_keys(&ctx.user_id).await {
        Ok(keys) => (StatusCode::OK, Json(json!({ "keys": keys }))).into_response(),
        Err(e) => {
            tracing::error!("Failed to list API keys: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to list API keys"})),
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
    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);
    let max_keys = state.config.max_api_keys_per_user;
    match service
        .create_api_key(&ctx.user_id, request, max_keys)
        .await
    {
        Ok(response) => (
            StatusCode::CREATED,
            Json(json!({
                "key": response,
                "message": "Save your API key securely. It will only be shown once."
            })),
        )
            .into_response(),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("limit reached") {
                (StatusCode::BAD_REQUEST, Json(json!({"error": msg}))).into_response()
            } else {
                tracing::error!("Failed to create API key: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "Failed to create API key"})),
                )
                    .into_response()
            }
        }
    }
}

/// Revoke an API key (requires auth)
async fn revoke_api_key(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<UserContext>,
    Path(key_id): Path<String>,
) -> Response {
    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);
    match service.revoke_api_key(&ctx.user_id, &key_id).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "message": "API key revoked" })),
        )
            .into_response(),
        Err(e) => {
            tracing::warn!("Revoke API key failed: {}", e);
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "API key not found"})),
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

/// Login with API key (public endpoint, with rate limiting and lockout)
pub async fn login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> Response {
    let client_ip = extract_client_ip(&headers);

    // Rate limiting
    if let Some(ref limiter) = state.login_limiter {
        if !limiter.check(&client_ip).await {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error": "Too many requests"})),
            )
                .into_response();
        }
    }

    // Login lockout check (by key prefix as pseudo-identifier)
    let lock_key = request.api_key.chars().take(20).collect::<String>();
    if let Some(ref guard) = state.login_guard {
        if let Err(remaining) = guard.check_locked(&lock_key).await {
            return (
                StatusCode::LOCKED,
                Json(json!({
                    "error": "Account temporarily locked",
                    "retry_after_seconds": remaining
                })),
            )
                .into_response();
        }
    }

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = SessionService::new(db);

    match service.create_session(&request.api_key).await {
        Ok((session, user)) => {
            // Clear lockout on success
            if let Some(ref guard) = state.login_guard {
                guard.clear(&lock_key).await;
            }
            let cookie = build_session_cookie(&session.id, state.config.secure_cookies);
            (
                StatusCode::OK,
                [(SET_COOKIE, cookie)],
                Json(json!({ "user": {
                    "id": user.id,
                    "email": user.email,
                    "display_name": user.display_name,
                    "role": user.role
                }})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!("Login failed: {}", e);
            // Record failure
            if let Some(ref guard) = state.login_guard {
                let count = guard.record_failure(&lock_key).await;
                if count >= state.config.login_max_failures {
                    // Log lockout event
                    if let Some(db) = state.db.as_mongodb() {
                        let svc = AuthService::new(db.clone());
                        svc.log_audit_public(
                            "login_locked",
                            None,
                            None,
                            Some(&client_ip),
                            Some(&format!("failures: {}", count)),
                        )
                        .await;
                    }
                }
            }
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Authentication failed"})),
            )
                .into_response()
        }
    }
}

/// Logout (public endpoint)
pub async fn logout(State(state): State<Arc<AppState>>, jar: CookieJar) -> Response {
    if let Some(cookie) = jar.get(SESSION_COOKIE_NAME) {
        if let Some(db) = state.db.as_mongodb() {
            let service = SessionService::new(db.clone());
            let _ = service.delete_session(cookie.value()).await;
        }
    }

    let clear_cookie = build_clear_cookie(state.config.secure_cookies);
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

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = SessionService::new(db);
    match service.validate_session(session_id).await {
        Ok(user) => (
            StatusCode::OK,
            Json(json!({ "user": {
                "id": user.id,
                "email": user.email,
                "display_name": user.display_name,
                "role": user.role
            }})),
        )
            .into_response(),
        Err(_) => {
            let clear_cookie = build_clear_cookie(state.config.secure_cookies);
            (
                StatusCode::UNAUTHORIZED,
                [(SET_COOKIE, clear_cookie)],
                Json(json!({ "error": "Invalid session" })),
            )
                .into_response()
        }
    }
}

/// Password login request
#[derive(Debug, Deserialize)]
pub struct PasswordLoginRequest {
    pub email: String,
    pub password: String,
}

/// Login with email and password (public endpoint)
pub async fn login_with_password(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<PasswordLoginRequest>,
) -> Response {
    let client_ip = extract_client_ip(&headers);

    if let Some(ref limiter) = state.login_limiter {
        if !limiter.check(&client_ip).await {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error": "Too many requests"})),
            )
                .into_response();
        }
    }

    let lock_key = request.email.clone();
    if let Some(ref guard) = state.login_guard {
        if let Err(remaining) = guard.check_locked(&lock_key).await {
            return (
                StatusCode::LOCKED,
                Json(json!({"error": "Account temporarily locked", "retry_after_seconds": remaining})),
            )
                .into_response();
        }
    }

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db.clone())
        .with_admin_emails(state.config.admin_emails.clone());

    match service.login_with_password(&request.email, &request.password).await {
        Ok(user) => {
            if let Some(ref guard) = state.login_guard {
                guard.clear(&lock_key).await;
            }
            let session_service = SessionService::new(db);
            match session_service.create_session_for_user(&user).await {
                Ok(session) => {
                    let cookie = build_session_cookie(&session.id, state.config.secure_cookies);
                    (
                        StatusCode::OK,
                        [(SET_COOKIE, cookie)],
                        Json(json!({ "user": {
                            "id": user.id,
                            "email": user.email,
                            "display_name": user.display_name,
                            "role": user.role
                        }})),
                    )
                        .into_response()
                }
                Err(e) => {
                    tracing::error!("Session creation failed: {}", e);
                    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Session creation failed"}))).into_response()
                }
            }
        }
        Err(e) => {
            tracing::warn!("Password login failed: {}", e);
            if let Some(ref guard) = state.login_guard {
                guard.record_failure(&lock_key).await;
            }
            (StatusCode::UNAUTHORIZED, Json(json!({"error": "Authentication failed"}))).into_response()
        }
    }
}

/// Change password request
#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: Option<String>,
    pub new_password: String,
}

/// Change password (requires auth)
async fn change_password(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<UserContext>,
    Json(request): Json<ChangePasswordRequest>,
) -> Response {
    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);

    match service
        .change_password(&ctx.user_id, request.current_password.as_deref(), &request.new_password)
        .await
    {
        Ok(()) => (StatusCode::OK, Json(json!({"message": "Password changed"}))).into_response(),
        Err(e) => {
            let msg = e.to_string();
            (StatusCode::BAD_REQUEST, Json(json!({"error": msg}))).into_response()
        }
    }
}

/// Register in approval mode - creates a pending request
async fn register_approval(state: Arc<AppState>, request: RegisterRequest) -> Response {
    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);

    match service.submit_registration(request).await {
        Ok(request_id) => (
            StatusCode::ACCEPTED,
            Json(json!({
                "message": "Registration request submitted for approval",
                "request_id": request_id
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Registration request failed: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Registration request failed"})),
            )
                .into_response()
        }
    }
}

/// Constant-time string comparison to prevent timing attacks.
/// Always iterates over the longer of the two inputs so that
/// neither the length nor the content of the secret leaks via timing.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();

    // Use a usize for the length mismatch flag to avoid truncation.
    // Casting usize XOR to u8 would silently wrap (e.g. 0 ^ 256 == 0 as u8),
    // allowing a false positive when lengths differ by a multiple of 256.
    let len_diff = a_bytes.len() ^ b_bytes.len();

    let max_len = a_bytes.len().max(b_bytes.len());
    let mut result: u8 = 0;
    for i in 0..max_len {
        let a_byte = if i < a_bytes.len() { a_bytes[i] } else { 0 };
        let b_byte = if i < b_bytes.len() { b_bytes[i] } else { 0 };
        result |= a_byte ^ b_byte;
    }

    // Both the content comparison and the length check must pass
    result == 0 && len_diff == 0
}

/// Verify admin API key from X-Admin-Key header
fn verify_admin_key(headers: &HeaderMap, config: &crate::config::Config) -> bool {
    let admin_key = match &config.admin_api_key {
        Some(key) => key,
        None => return false,
    };
    headers
        .get("X-Admin-Key")
        .and_then(|v| v.to_str().ok())
        .map(|k| constant_time_eq(k, admin_key))
        .unwrap_or(false)
}

/// Check admin access: X-Admin-Key header OR authenticated user with admin role
fn is_admin(headers: &HeaderMap, config: &crate::config::Config, user_ctx: Option<&UserContext>) -> bool {
    if verify_admin_key(headers, config) {
        return true;
    }
    if let Some(ctx) = user_ctx {
        return ctx.role == "admin";
    }
    false
}

/// Configure admin auth routes (require admin API key or admin role)
pub fn admin_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/registrations", get(list_registrations))
        .route("/registrations/{id}/approve", post(approve_registration))
        .route("/registrations/{id}/reject", post(reject_registration))
}

/// List pending registration requests (admin)
async fn list_registrations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    user_ctx: Option<Extension<UserContext>>,
) -> Response {
    if !is_admin(&headers, &state.config, user_ctx.as_ref().map(|e| &e.0)) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "Admin access required"}))).into_response();
    }

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);

    match service.list_pending_registrations().await {
        Ok(requests) => (StatusCode::OK, Json(json!({"requests": requests}))).into_response(),
        Err(e) => {
            tracing::error!("Failed to list registrations: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to list registrations"})),
            )
                .into_response()
        }
    }
}

/// Approve a registration request (admin)
async fn approve_registration(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    user_ctx: Option<Extension<UserContext>>,
    Path(id): Path<String>,
) -> Response {
    let reviewer = user_ctx.as_ref().map(|e| e.0.email.as_str()).unwrap_or("admin");
    if !is_admin(&headers, &state.config, user_ctx.as_ref().map(|e| &e.0)) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "Admin access required"}))).into_response();
    }

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db)
        .with_admin_emails(state.config.admin_emails.clone());

    match service.approve_registration(&id, reviewer).await {
        Ok(response) => (
            StatusCode::OK,
            Json(json!({
                "user": response.user,
                "api_key": response.api_key,
                "message": "Registration approved"
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Approve failed: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Approval failed"})),
            )
                .into_response()
        }
    }
}

/// Reject request body
#[derive(Debug, Deserialize)]
pub struct RejectRequest {
    pub reason: Option<String>,
}

/// Reject a registration request (admin)
async fn reject_registration(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    user_ctx: Option<Extension<UserContext>>,
    Path(id): Path<String>,
    Json(body): Json<RejectRequest>,
) -> Response {
    let reviewer = user_ctx.as_ref().map(|e| e.0.email.as_str()).unwrap_or("admin");
    if !is_admin(&headers, &state.config, user_ctx.as_ref().map(|e| &e.0)) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "Admin access required"}))).into_response();
    }

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);
    let reason = body.reason.as_deref();

    match service.reject_registration(&id, reviewer, reason).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({"message": "Registration rejected"})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Reject failed: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Rejection failed"})),
            )
                .into_response()
        }
    }
}

/// Deactivate own account (requires auth)
async fn deactivate_account(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<UserContext>,
) -> Response {
    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };

    let auth_service = AuthService::new(db.clone());
    if let Err(e) = auth_service.deactivate_user(&ctx.user_id).await {
        tracing::error!("Deactivate failed: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Deactivation failed"})),
        )
            .into_response();
    }

    // Clear all sessions for this user
    let session_service = SessionService::new(db);
    let _ = session_service.delete_user_sessions(&ctx.user_id).await;

    // Clear client cookie
    let clear_cookie = build_clear_cookie(state.config.secure_cookies);
    (
        StatusCode::OK,
        [(SET_COOKIE, clear_cookie)],
        Json(json!({"message": "Account deactivated"})),
    )
        .into_response()
}
