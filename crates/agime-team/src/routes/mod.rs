//! Routes module - HTTP API endpoints

pub mod teams;
pub mod members;
pub mod skills;
pub mod recipes;
pub mod extensions;
pub mod sync;
pub mod recommendations;
pub mod invites;

use axum::Router;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;

pub use teams::TeamState;

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

    Router::new()
        .nest("/api/team", api_routes(state))
}

fn api_routes(state: TeamState) -> Router {
    // Get base URL from environment or use default
    // IMPORTANT: Set AGIME_TEAM_API_URL to your actual LAN IP for invite links to work properly
    // Example: AGIME_TEAM_API_URL=http://192.168.1.100:7778
    // The default localhost will only work for local testing
    let base_url = std::env::var("AGIME_TEAM_API_URL")
        .unwrap_or_else(|_| {
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
        .merge(sync::routes(state.clone()))
        .merge(recommendations::routes(state.clone()))
        .merge(invites::configure(state.pool, state.user_id, base_url))
}
