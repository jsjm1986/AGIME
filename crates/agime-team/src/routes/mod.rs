//! Routes module - HTTP API endpoints

pub mod documents;
pub mod extensions;
pub mod invites;
pub mod members;
pub mod recipes;
pub mod recommendations;
pub mod skills;
pub mod sync;
pub mod teams;
pub mod unified;

// MongoDB routes
pub mod mongo;

use axum::Router;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::TeamError;
use crate::models::{ProtectionLevel, ResourceType};

pub use crate::AuthenticatedUserId;
pub use teams::TeamState;

/// Helper function to get user ID from Extension or fallback to state
/// This allows routes to work with both authenticated (via Extension) and
/// non-authenticated (via state default) contexts.
pub fn get_user_id(auth_user: Option<&AuthenticatedUserId>, state: &TeamState) -> String {
    auth_user
        .map(|u| u.0.clone())
        .unwrap_or_else(|| state.user_id.clone())
}

/// Resolved authorization tuple: (token, expires_at, last_verified_at)
pub type ResolvedAuth = (String, String, String);

/// Resolve authorization for a local install request.
/// Shared across skill, recipe, and extension local install handlers.
pub fn resolve_authorization(
    protection_level: &ProtectionLevel,
    auth_request: Option<&skills::LocalInstallAuthorizationRequest>,
    team_id: &str,
    resource_id: &str,
    user_id: &str,
    installed_at: &str,
    resource_type_label: &str,
) -> Result<Option<ResolvedAuth>, TeamError> {
    if !protection_level.requires_authorization() {
        return Ok(None);
    }

    let now = chrono::Utc::now();

    if let Some(auth) = auth_request {
        if auth.token.trim().is_empty() {
            return Err(TeamError::Validation(format!(
                "Authorization token is required for non-public {}",
                resource_type_label
            )));
        }
        Ok(Some((
            auth.token.clone(),
            auth.expires_at.clone(),
            auth.last_verified_at
                .clone()
                .unwrap_or_else(|| installed_at.to_string()),
        )))
    } else {
        let fallback_expires = now + chrono::Duration::hours(24);
        Ok(Some((
            skills::generate_access_token(team_id, resource_id, user_id, &fallback_expires),
            fallback_expires.to_rfc3339(),
            installed_at.to_string(),
        )))
    }
}

/// Record a local installation in the installed_resources table.
/// Shared across skill, recipe, and extension local install handlers.
pub async fn record_local_install(
    pool: &SqlitePool,
    resource_type: ResourceType,
    resource_id: &str,
    team_id: &str,
    name: &str,
    local_path_str: &str,
    version: &str,
    installed_at: &str,
    user_id: &str,
    authorization: Option<&ResolvedAuth>,
    protection_level: &ProtectionLevel,
) -> Result<(), TeamError> {
    sqlx::query(
        r#"
        INSERT INTO installed_resources (
            id, resource_type, resource_id, team_id, resource_name, local_path,
            installed_version, installed_at, user_id, authorization_token,
            authorization_expires_at, last_verified_at, protection_level
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(resource_type, resource_id) DO UPDATE SET
            installed_version = excluded.installed_version,
            local_path = excluded.local_path,
            has_update = 0,
            installed_at = excluded.installed_at,
            user_id = excluded.user_id,
            authorization_token = excluded.authorization_token,
            authorization_expires_at = excluded.authorization_expires_at,
            last_verified_at = excluded.last_verified_at,
            protection_level = excluded.protection_level
        "#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(resource_type.to_string())
    .bind(resource_id)
    .bind(team_id)
    .bind(name)
    .bind(local_path_str)
    .bind(version)
    .bind(installed_at)
    .bind(user_id)
    .bind(authorization.map(|(t, _, _)| t.as_str()))
    .bind(authorization.map(|(_, e, _)| e.as_str()))
    .bind(authorization.map(|(_, _, v)| v.as_str()))
    .bind(protection_level.to_string())
    .execute(pool)
    .await?;

    Ok(())
}

/// Build the authorization JSON value for metadata files.
pub fn build_auth_meta_json(authorization: Option<&ResolvedAuth>) -> Option<serde_json::Value> {
    authorization.map(|(token, expires_at, last_verified_at)| {
        serde_json::json!({
            "token": token,
            "expiresAt": expires_at,
            "lastVerifiedAt": last_verified_at,
        })
    })
}

/// Configure all team routes
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `user_id` - Current user ID (from authentication)
/// * `base_path` - Base path for installing resources locally
pub fn configure(pool: Arc<SqlitePool>, user_id: String, base_path: PathBuf) -> Router {
    let state = TeamState {
        pool,
        user_id,
        base_path,
    };

    Router::new().nest("/api/team", api_routes(state))
}

/// Configure team routes with default settings (for team-server use)
/// Uses empty user_id (will be set by auth middleware) and temp directory for base_path
pub fn configure_routes(pool: Arc<SqlitePool>) -> Router {
    let base_path = std::env::temp_dir().join("agime-team-server");
    let state = TeamState {
        pool,
        user_id: String::new(), // Will be overridden by auth middleware
        base_path,
    };

    api_routes(state)
}

fn api_routes(state: TeamState) -> Router {
    // Get base URL from environment or use default
    // IMPORTANT: Set AGIME_TEAM_API_URL to your actual LAN IP for invite links to work properly
    // Example: AGIME_TEAM_API_URL=http://192.168.1.100:7778
    // The default localhost will only work for local testing
    let base_url = std::env::var("AGIME_TEAM_API_URL").unwrap_or_else(|_| {
        // Try to get server address from config, fallback to localhost
        std::env::var("AGIME_SERVER_ADDR")
            .ok()
            .and_then(|addr| {
                // If addr is 0.0.0.0:port, we can't use it for invite URLs
                // Users must set AGIME_TEAM_API_URL explicitly
                if !addr.starts_with("0.0.0.0") {
                    Some(format!("http://{}", addr))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "http://localhost:7778".to_string())
    });

    Router::new()
        .merge(teams::routes(state.clone()))
        .merge(members::routes(state.clone()))
        .merge(skills::routes(state.clone()))
        .merge(recipes::routes(state.clone()))
        .merge(extensions::routes(state.clone()))
        .merge(documents::routes(state.clone()))
        .merge(sync::routes(state.clone()))
        .merge(recommendations::routes(state.clone()))
        .merge(unified::routes(state.clone()))
        .merge(invites::configure(state.pool, state.user_id, base_url))
}
