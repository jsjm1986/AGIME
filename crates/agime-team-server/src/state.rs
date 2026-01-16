//! Application state

use crate::config::Config;
use sqlx::SqlitePool;
use std::sync::Arc;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    /// Database connection pool
    pub pool: Arc<SqlitePool>,

    /// Server configuration
    pub config: Config,
}
