//! Application state

use crate::agent::rate_limit::RateLimiter;
use crate::auth::service_mongo::LoginGuard;
use crate::config::{Config, DatabaseType};
use crate::license::BrandConfig;
use agime_team::MongoDb;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Database backend enum for runtime switching
#[derive(Clone)]
pub enum DatabaseBackend {
    MongoDB(Arc<MongoDb>),
    SQLite(Arc<SqlitePool>),
}

impl DatabaseBackend {
    #[allow(dead_code)]
    pub fn database_type(&self) -> DatabaseType {
        match self {
            DatabaseBackend::MongoDB(_) => DatabaseType::MongoDB,
            DatabaseBackend::SQLite(_) => DatabaseType::SQLite,
        }
    }

    pub fn as_mongodb(&self) -> Option<&Arc<MongoDb>> {
        match self {
            DatabaseBackend::MongoDB(db) => Some(db),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn as_sqlite(&self) -> Option<&Arc<SqlitePool>> {
        match self {
            DatabaseBackend::SQLite(pool) => Some(pool),
            _ => None,
        }
    }
}

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    /// Database backend (MongoDB or SQLite)
    pub db: DatabaseBackend,

    /// Server configuration
    pub config: Config,

    /// Rate limiter for registration endpoint
    pub register_limiter: Option<Arc<RateLimiter>>,

    /// Rate limiter for login endpoint
    pub login_limiter: Option<Arc<RateLimiter>>,

    /// Login failure guard
    pub login_guard: Option<Arc<LoginGuard>>,

    /// Brand configuration (updatable at runtime via license activation)
    pub brand_config: Arc<RwLock<BrandConfig>>,
}

impl AppState {
    /// Get the MongoDB backend or return a 503 Service Unavailable response.
    /// Reduces repeated match boilerplate in route handlers.
    #[allow(clippy::result_large_err)]
    pub fn require_mongodb(&self) -> Result<Arc<MongoDb>, Response> {
        self.db.as_mongodb().cloned().ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "Database not available"})),
            )
                .into_response()
        })
    }

    /// Get the SQLite backend or return a 503 Service Unavailable response.
    #[allow(clippy::result_large_err)]
    pub fn require_sqlite(&self) -> Result<Arc<SqlitePool>, Response> {
        self.db.as_sqlite().cloned().ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "Database not available"})),
            )
                .into_response()
        })
    }
}
