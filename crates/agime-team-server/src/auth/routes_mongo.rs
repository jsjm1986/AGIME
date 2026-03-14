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

use super::middleware_mongo::{SystemAdminContext, UserContext};
use super::service_mongo::{AuthService, CreateApiKeyRequest, RegisterRequest};
use super::session_mongo::SessionService;
use super::system_admin_session_mongo::{
    SystemAdminSessionService, SYSTEM_ADMIN_SESSION_COOKIE_NAME,
};
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

fn build_system_admin_session_cookie(session_id: &str, secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800{}",
        SYSTEM_ADMIN_SESSION_COOKIE_NAME, session_id, secure_flag
    )
}

fn build_clear_system_admin_cookie(secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}",
        SYSTEM_ADMIN_SESSION_COOKIE_NAME, secure_flag
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

/// Protected routes for the isolated system-admin console.
pub fn system_admin_protected_router() -> Router<Arc<AppState>> {
    Router::new().route("/change-password", post(change_system_admin_password))
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
    let service = AuthService::new(db).with_admin_emails(state.config.admin_emails.clone());

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

/// Dedicated system-admin login request
#[derive(Debug, Deserialize)]
pub struct SystemAdminLoginRequest {
    pub username: String,
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
    let service = AuthService::new(db.clone()).with_admin_emails(state.config.admin_emails.clone());

    match service
        .login_with_password(&request.email, &request.password)
        .await
    {
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
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error": "Session creation failed"})),
                    )
                        .into_response()
                }
            }
        }
        Err(e) => {
            tracing::warn!("Password login failed: {}", e);
            if let Some(ref guard) = state.login_guard {
                guard.record_failure(&lock_key).await;
            }
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Authentication failed"})),
            )
                .into_response()
        }
    }
}

/// Dedicated system-admin login using the bootstrap admin alias.
pub async fn login_system_admin(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<SystemAdminLoginRequest>,
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

    let lock_key = format!(
        "system-admin:{}",
        request.username.trim().to_ascii_lowercase()
    );
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
    let service = AuthService::new(db.clone());

    match service
        .login_system_admin(
            &request.username,
            &request.password,
            &state.config.bootstrap_admin_username,
            &state.config.bootstrap_admin_password,
            &state.config.bootstrap_admin_email,
        )
        .await
    {
        Ok(admin) => {
            if let Some(ref guard) = state.login_guard {
                guard.clear(&lock_key).await;
            }
            let session_service = SystemAdminSessionService::new(db);
            match session_service.create_session_for_admin(&admin).await {
                Ok(session) => {
                    let cookie =
                        build_system_admin_session_cookie(&session.id, state.config.secure_cookies);
                    (
                        StatusCode::OK,
                        [(SET_COOKIE, cookie)],
                        Json(json!({ "admin": {
                            "id": admin.id,
                            "username": admin.username,
                            "display_name": admin.display_name
                        }})),
                    )
                        .into_response()
                }
                Err(e) => {
                    tracing::error!("System admin session creation failed: {}", e);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error": "Session creation failed"})),
                    )
                        .into_response()
                }
            }
        }
        Err(e) => {
            tracing::warn!("System admin login failed: {}", e);
            if let Some(ref guard) = state.login_guard {
                guard.record_failure(&lock_key).await;
            }
            service
                .log_audit_public(
                    "login_system_admin_denied",
                    None,
                    None,
                    Some(&client_ip),
                    Some(&format!("reason: {}", e)),
                )
                .await;
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}

/// Get current dedicated system-admin session.
pub async fn get_system_admin_session(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> Response {
    let session_id = match jar.get(SYSTEM_ADMIN_SESSION_COOKIE_NAME) {
        Some(cookie) => cookie.value(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "No system admin session" })),
            )
                .into_response()
        }
    };

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = SystemAdminSessionService::new(db);

    match service.validate_session(session_id).await {
        Ok(admin) => (
            StatusCode::OK,
            Json(json!({ "admin": {
                "id": admin.id,
                "username": admin.username,
                "display_name": admin.display_name
            }})),
        )
            .into_response(),
        Err(_) => {
            let clear_cookie = build_clear_system_admin_cookie(state.config.secure_cookies);
            (
                StatusCode::UNAUTHORIZED,
                [(SET_COOKIE, clear_cookie)],
                Json(json!({ "error": "Invalid system admin session" })),
            )
                .into_response()
        }
    }
}

/// Logout dedicated system-admin session.
pub async fn logout_system_admin(State(state): State<Arc<AppState>>, jar: CookieJar) -> Response {
    if let Some(cookie) = jar.get(SYSTEM_ADMIN_SESSION_COOKIE_NAME) {
        if let Some(db) = state.db.as_mongodb() {
            let service = SystemAdminSessionService::new(db.clone());
            let _ = service.delete_session(cookie.value()).await;
        }
    }

    let clear_cookie = build_clear_system_admin_cookie(state.config.secure_cookies);
    (
        StatusCode::OK,
        [(SET_COOKIE, clear_cookie)],
        Json(json!({ "message": "System admin logged out" })),
    )
        .into_response()
}

/// Change password request
#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: Option<String>,
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
pub struct SystemAdminChangePasswordRequest {
    pub current_password: String,
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
        .change_password(
            &ctx.user_id,
            request.current_password.as_deref(),
            &request.new_password,
        )
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

async fn change_system_admin_password(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<SystemAdminContext>,
    Json(body): Json<SystemAdminChangePasswordRequest>,
) -> Response {
    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };

    let service = AuthService::new(db);
    match service
        .change_system_admin_password(&ctx.admin_id, &body.current_password, &body.new_password)
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({"message": "System admin password updated"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

fn require_system_admin_context(
    admin_ctx: Option<Extension<SystemAdminContext>>,
) -> Result<SystemAdminContext, Response> {
    admin_ctx.map(|ctx| ctx.0).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "System admin session required"})),
        )
            .into_response()
    })
}

/// Configure admin auth routes (require dedicated system-admin authentication)
pub fn admin_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/overview", get(get_system_admin_overview))
        .route("/teams", get(list_admin_teams))
        .route("/users", get(list_admin_users))
        .route("/users/{id}/role", post(update_admin_user_role))
        .route("/users/{id}/deactivate", post(admin_deactivate_user))
        .route("/users/{id}/reactivate", post(admin_reactivate_user))
        .route("/registrations", get(list_registrations))
        .route("/registrations/history", get(list_registration_history))
        .route("/registrations/{id}/approve", post(approve_registration))
        .route("/registrations/{id}/reject", post(reject_registration))
        .route("/audit-logs", get(list_auth_audit_logs))
}

/// Get overview counts for the dedicated system-admin console.
async fn get_system_admin_overview(
    State(state): State<Arc<AppState>>,
    admin_ctx: Option<Extension<SystemAdminContext>>,
) -> Response {
    if let Err(resp) = require_system_admin_context(admin_ctx) {
        return resp;
    }

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);

    match service.get_system_admin_overview().await {
        Ok(overview) => (StatusCode::OK, Json(json!({ "overview": overview }))).into_response(),
        Err(e) => {
            tracing::error!("Failed to load system admin overview: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to load overview"})),
            )
                .into_response()
        }
    }
}

/// List teams for the dedicated system-admin console.
async fn list_admin_teams(
    State(state): State<Arc<AppState>>,
    admin_ctx: Option<Extension<SystemAdminContext>>,
) -> Response {
    if let Err(resp) = require_system_admin_context(admin_ctx) {
        return resp;
    }

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);

    match service.list_teams_for_admin().await {
        Ok(teams) => (StatusCode::OK, Json(json!({ "teams": teams }))).into_response(),
        Err(e) => {
            tracing::error!("Failed to list system admin teams: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to load teams"})),
            )
                .into_response()
        }
    }
}

/// List pending registration requests (admin)
async fn list_registrations(
    State(state): State<Arc<AppState>>,
    admin_ctx: Option<Extension<SystemAdminContext>>,
) -> Response {
    if let Err(resp) = require_system_admin_context(admin_ctx) {
        return resp;
    }

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);

    match service.list_pending_registrations().await {
        Ok(requests) => (
            StatusCode::OK,
            Json(json!({
                "requests": requests
                    .into_iter()
                    .map(super::service_mongo::RegistrationRequestSummary::from)
                    .collect::<Vec<_>>()
            })),
        )
            .into_response(),
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

/// List recent processed registration requests (admin)
async fn list_registration_history(
    State(state): State<Arc<AppState>>,
    admin_ctx: Option<Extension<SystemAdminContext>>,
) -> Response {
    if let Err(resp) = require_system_admin_context(admin_ctx) {
        return resp;
    }

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);

    match service.list_processed_registrations(30).await {
        Ok(requests) => (StatusCode::OK, Json(json!({"requests": requests}))).into_response(),
        Err(e) => {
            tracing::error!("Failed to list registration history: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to list registration history"})),
            )
                .into_response()
        }
    }
}

/// List users for the system admin console (admin)
async fn list_admin_users(
    State(state): State<Arc<AppState>>,
    admin_ctx: Option<Extension<SystemAdminContext>>,
) -> Response {
    if let Err(resp) = require_system_admin_context(admin_ctx) {
        return resp;
    }

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);

    let excluded_emails = vec![state
        .config
        .bootstrap_admin_email
        .trim()
        .to_ascii_lowercase()];
    match service.list_users_for_admin(&excluded_emails).await {
        Ok(users) => (StatusCode::OK, Json(json!({ "users": users }))).into_response(),
        Err(e) => {
            tracing::error!("Failed to list admin users: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to load users"})),
            )
                .into_response()
        }
    }
}

/// Update user role request
#[derive(Debug, Deserialize)]
pub struct UpdateUserRoleRequest {
    pub role: String,
}

/// Update a user's global role from the system admin console
async fn update_admin_user_role(
    State(state): State<Arc<AppState>>,
    admin_ctx: Option<Extension<SystemAdminContext>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateUserRoleRequest>,
) -> Response {
    let admin_ctx = match require_system_admin_context(admin_ctx) {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);
    let actor_user_id = Some(admin_ctx.admin_id.as_str());

    match service
        .update_user_role_for_admin(actor_user_id, &id, &body.role)
        .await
    {
        Ok(user) => (StatusCode::OK, Json(json!({ "user": user }))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// Deactivate a user account from the system admin console
async fn admin_deactivate_user(
    State(state): State<Arc<AppState>>,
    admin_ctx: Option<Extension<SystemAdminContext>>,
    Path(id): Path<String>,
) -> Response {
    let admin_ctx = match require_system_admin_context(admin_ctx) {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db.clone());
    let actor_user_id = Some(admin_ctx.admin_id.as_str());

    match service
        .set_user_active_for_admin(actor_user_id, &id, false)
        .await
    {
        Ok(user) => {
            let session_service = SessionService::new(db);
            let _ = session_service.delete_user_sessions(&id).await;
            (StatusCode::OK, Json(json!({ "user": user }))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// Reactivate a user account from the system admin console
async fn admin_reactivate_user(
    State(state): State<Arc<AppState>>,
    admin_ctx: Option<Extension<SystemAdminContext>>,
    Path(id): Path<String>,
) -> Response {
    let admin_ctx = match require_system_admin_context(admin_ctx) {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);
    let actor_user_id = Some(admin_ctx.admin_id.as_str());

    match service
        .set_user_active_for_admin(actor_user_id, &id, true)
        .await
    {
        Ok(user) => (StatusCode::OK, Json(json!({ "user": user }))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// List auth audit logs for the system admin console (admin)
async fn list_auth_audit_logs(
    State(state): State<Arc<AppState>>,
    admin_ctx: Option<Extension<SystemAdminContext>>,
) -> Response {
    if let Err(resp) = require_system_admin_context(admin_ctx) {
        return resp;
    }

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);

    match service.list_auth_audit_logs(100).await {
        Ok(logs) => (StatusCode::OK, Json(json!({ "logs": logs }))).into_response(),
        Err(e) => {
            tracing::error!("Failed to list auth audit logs: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to load audit logs"})),
            )
                .into_response()
        }
    }
}

/// Approve a registration request (admin)
async fn approve_registration(
    State(state): State<Arc<AppState>>,
    admin_ctx: Option<Extension<SystemAdminContext>>,
    Path(id): Path<String>,
) -> Response {
    let admin_ctx = match require_system_admin_context(admin_ctx) {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };
    let reviewer = format!("system-admin:{}", admin_ctx.username);

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db).with_admin_emails(state.config.admin_emails.clone());

    match service.approve_registration(&id, &reviewer).await {
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
    admin_ctx: Option<Extension<SystemAdminContext>>,
    Path(id): Path<String>,
    Json(body): Json<RejectRequest>,
) -> Response {
    let admin_ctx = match require_system_admin_context(admin_ctx) {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };
    let reviewer = format!("system-admin:{}", admin_ctx.username);

    let db = match state.require_mongodb() {
        Ok(db) => db,
        Err(resp) => return resp,
    };
    let service = AuthService::new(db);
    let reason = body.reason.as_deref();

    match service.reject_registration(&id, &reviewer, reason).await {
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
