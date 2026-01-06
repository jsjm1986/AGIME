use crate::state::AppState;
use agime::session::SessionManager;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

/// Request body for creating a shared session
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateShareRequest {
    /// The session ID to share
    pub session_id: String,
    /// Expiration option: 1, 7, 30 days, or null for never
    pub expires_in_days: Option<i32>,
    /// Optional password for protection
    pub password: Option<String>,
}

/// Response for creating a shared session
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateShareResponse {
    /// The unique share token
    pub share_token: String,
    /// The full share URL (if tunnel is running)
    pub share_url: Option<String>,
    /// When the share expires (null if never)
    pub expires_at: Option<DateTime<Utc>>,
    /// Whether password protection is enabled
    pub has_password: bool,
}

/// Request body for verifying password
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VerifyPasswordRequest {
    pub password: String,
}

/// Response for getting a shared session
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SharedSessionResponse {
    /// Session name/description
    pub name: String,
    /// Working directory
    pub working_dir: String,
    /// Session messages as JSON
    pub messages: serde_json::Value,
    /// Number of messages
    pub message_count: usize,
    /// Total tokens used
    pub total_tokens: Option<i32>,
    /// When the share was created
    pub created_at: DateTime<Utc>,
    /// When the share expires (null if never)
    pub expires_at: Option<DateTime<Utc>>,
}

/// Response when password is required
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasswordRequiredResponse {
    pub password_required: bool,
    pub name: String,
    pub message_count: usize,
}

/// Error response
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ShareErrorResponse {
    pub error: String,
    pub code: String,
}

/// Generate a random share token
fn generate_share_token() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..16).map(|_| rng.random()).collect();
    hex::encode(bytes)
}

/// Hash password using SHA-256 with salt
fn hash_password(password: &str, salt: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(password.as_bytes());
    hex::encode(hasher.finalize())
}

/// Verify password against hash
fn verify_password(password: &str, salt: &str, hash: &str) -> bool {
    let computed_hash = hash_password(password, salt);
    // Constant-time comparison to prevent timing attacks
    computed_hash.len() == hash.len()
        && computed_hash
            .bytes()
            .zip(hash.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

#[utoipa::path(
    post,
    path = "/sessions/share",
    request_body = CreateShareRequest,
    responses(
        (status = 200, description = "Share created successfully", body = CreateShareResponse),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Sharing"
)]
async fn create_share(
    State(_state): State<Arc<AppState>>,
    Json(request): Json<CreateShareRequest>,
) -> Result<Json<CreateShareResponse>, (StatusCode, Json<ShareErrorResponse>)> {
    // Get the session with messages
    let session = SessionManager::get_session(&request.session_id, true)
        .await
        .map_err(|_| {
            (
                StatusCode::NOT_FOUND,
                Json(ShareErrorResponse {
                    error: "Session not found".to_string(),
                    code: "SESSION_NOT_FOUND".to_string(),
                }),
            )
        })?;

    // Generate share token and salt
    let share_token = generate_share_token();
    let salt = generate_share_token(); // Use another random string as salt

    // Calculate expiration time
    let expires_at = request.expires_in_days.map(|days| {
        Utc::now() + Duration::days(days as i64)
    });

    // Hash password if provided
    let password_hash = request.password.as_ref().map(|pwd| {
        format!("{}:{}", salt, hash_password(pwd, &salt))
    });

    // Serialize messages
    let messages_json = session
        .conversation
        .as_ref()
        .map(|conv| serde_json::to_value(conv.messages()).unwrap_or(serde_json::json!([])))
        .unwrap_or(serde_json::json!([]));

    // Store in database
    let storage = SessionManager::instance().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ShareErrorResponse {
                error: format!("Database error: {}", e),
                code: "DATABASE_ERROR".to_string(),
            }),
        )
    })?;

    storage
        .create_shared_session(
            &share_token,
            &session.name,
            &session.working_dir.to_string_lossy(),
            &messages_json.to_string(),
            session.message_count as i32,
            session.total_tokens,
            expires_at,
            password_hash.as_deref(),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ShareErrorResponse {
                    error: format!("Failed to create share: {}", e),
                    code: "CREATE_SHARE_FAILED".to_string(),
                }),
            )
        })?;

    Ok(Json(CreateShareResponse {
        share_token,
        share_url: None, // Frontend will construct the URL based on tunnel status
        expires_at,
        has_password: request.password.is_some(),
    }))
}

#[utoipa::path(
    get,
    path = "/sessions/share/{token}",
    params(
        ("token" = String, Path, description = "Share token")
    ),
    responses(
        (status = 200, description = "Shared session retrieved", body = SharedSessionResponse),
        (status = 401, description = "Password required", body = PasswordRequiredResponse),
        (status = 404, description = "Share not found or expired"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Session Sharing"
)]
async fn get_shared_session(
    Path(token): Path<String>,
) -> Result<Json<SharedSessionResponse>, (StatusCode, Json<serde_json::Value>)> {
    let storage = SessionManager::instance().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Database error: {}", e),
                "code": "DATABASE_ERROR"
            })),
        )
    })?;

    let shared = storage.get_shared_session(&token).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "Share not found or expired",
                "code": "SHARE_NOT_FOUND"
            })),
        )
    })?;

    // Check if expired
    if let Some(expires_at) = shared.expires_at {
        if expires_at < Utc::now() {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "Share has expired",
                    "code": "SHARE_EXPIRED"
                })),
            ));
        }
    }

    // Check if password protected
    if shared.password_hash.is_some() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "passwordRequired": true,
                "name": shared.name,
                "messageCount": shared.message_count
            })),
        ));
    }

    let messages: serde_json::Value =
        serde_json::from_str(&shared.messages).unwrap_or(serde_json::json!([]));

    Ok(Json(SharedSessionResponse {
        name: shared.name,
        working_dir: shared.working_dir,
        messages,
        message_count: shared.message_count as usize,
        total_tokens: shared.total_tokens,
        created_at: shared.created_at,
        expires_at: shared.expires_at,
    }))
}

#[utoipa::path(
    post,
    path = "/sessions/share/{token}/verify",
    params(
        ("token" = String, Path, description = "Share token")
    ),
    request_body = VerifyPasswordRequest,
    responses(
        (status = 200, description = "Password verified, session returned", body = SharedSessionResponse),
        (status = 401, description = "Invalid password"),
        (status = 404, description = "Share not found or expired"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Session Sharing"
)]
async fn verify_and_get_shared_session(
    Path(token): Path<String>,
    Json(request): Json<VerifyPasswordRequest>,
) -> Result<Json<SharedSessionResponse>, (StatusCode, Json<ShareErrorResponse>)> {
    let storage = SessionManager::instance().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ShareErrorResponse {
                error: format!("Database error: {}", e),
                code: "DATABASE_ERROR".to_string(),
            }),
        )
    })?;

    let shared = storage.get_shared_session(&token).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ShareErrorResponse {
                error: "Share not found or expired".to_string(),
                code: "SHARE_NOT_FOUND".to_string(),
            }),
        )
    })?;

    // Check if expired
    if let Some(expires_at) = shared.expires_at {
        if expires_at < Utc::now() {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ShareErrorResponse {
                    error: "Share has expired".to_string(),
                    code: "SHARE_EXPIRED".to_string(),
                }),
            ));
        }
    }

    // Verify password
    if let Some(ref stored_hash) = shared.password_hash {
        // Parse salt:hash format
        let parts: Vec<&str> = stored_hash.split(':').collect();
        if parts.len() != 2 {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ShareErrorResponse {
                    error: "Invalid password hash format".to_string(),
                    code: "INVALID_HASH".to_string(),
                }),
            ));
        }

        let salt = parts[0];
        let hash = parts[1];

        if !verify_password(&request.password, salt, hash) {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ShareErrorResponse {
                    error: "Invalid password".to_string(),
                    code: "INVALID_PASSWORD".to_string(),
                }),
            ));
        }
    }

    let messages: serde_json::Value =
        serde_json::from_str(&shared.messages).unwrap_or(serde_json::json!([]));

    Ok(Json(SharedSessionResponse {
        name: shared.name,
        working_dir: shared.working_dir,
        messages,
        message_count: shared.message_count as usize,
        total_tokens: shared.total_tokens,
        created_at: shared.created_at,
        expires_at: shared.expires_at,
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/sessions/share", post(create_share))
        .route("/sessions/share/{token}", get(get_shared_session))
        .route("/sessions/share/{token}/verify", post(verify_and_get_shared_session))
        .with_state(state)
}
