//! Local Cache Manager
//! Manages caching of remote resources for offline access

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::Arc;

/// Resource type for caching
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CachedResourceType {
    Skill,
    Recipe,
    Extension,
}

impl std::fmt::Display for CachedResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CachedResourceType::Skill => write!(f, "skill"),
            CachedResourceType::Recipe => write!(f, "recipe"),
            CachedResourceType::Extension => write!(f, "extension"),
        }
    }
}

/// Source type for cached resources
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    Cloud,
    Lan,
}

impl std::fmt::Display for SourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceType::Cloud => write!(f, "cloud"),
            SourceType::Lan => write!(f, "lan"),
        }
    }
}

/// Sync status for cached resources
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Synced,
    LocalOnly,
    RemoteOnly,
    Conflict,
    Pending,
}

impl std::fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncStatus::Synced => write!(f, "synced"),
            SyncStatus::LocalOnly => write!(f, "local-only"),
            SyncStatus::RemoteOnly => write!(f, "remote-only"),
            SyncStatus::Conflict => write!(f, "conflict"),
            SyncStatus::Pending => write!(f, "pending"),
        }
    }
}

/// Cached resource entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResource {
    pub id: String,
    pub source_id: String,
    pub source_type: String,
    pub resource_type: String,
    pub resource_id: String,
    pub content_json: String,
    pub cached_at: String,
    pub expires_at: Option<String>,
    pub sync_status: String,
}

/// Sync result for a batch operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResult {
    pub source_id: String,
    pub success: bool,
    pub synced_at: String,
    pub created: u32,
    pub updated: u32,
    pub deleted: u32,
    pub conflicts: u32,
    pub errors: Vec<String>,
}

/// Local cache manager for remote resources
pub struct LocalCacheManager {
    pool: Arc<SqlitePool>,
}

impl LocalCacheManager {
    /// Create a new cache manager
    pub fn new(pool: Arc<SqlitePool>) -> Self {
        Self { pool }
    }

    /// Cache a remote resource locally
    pub async fn cache_resource(
        &self,
        source_id: &str,
        source_type: &str,
        resource_type: &str,
        resource_id: &str,
        content_json: &str,
        expires_at: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO cached_resources
                (id, source_id, source_type, resource_type, resource_id,
                 content_json, cached_at, expires_at, sync_status)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'synced')
            ON CONFLICT(source_id, resource_type, resource_id)
            DO UPDATE SET
                content_json = excluded.content_json,
                cached_at = excluded.cached_at,
                expires_at = excluded.expires_at,
                sync_status = 'synced'
            "#,
        )
        .bind(&id)
        .bind(source_id)
        .bind(source_type)
        .bind(resource_type)
        .bind(resource_id)
        .bind(content_json)
        .bind(&now)
        .bind(expires_at)
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    /// Get a cached resource
    pub async fn get_cached(
        &self,
        source_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<Option<CachedResource>, sqlx::Error> {
        let row = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                Option<String>,
                String,
            ),
        >(
            r#"
            SELECT id, source_id, source_type, resource_type, resource_id,
                   content_json, cached_at, expires_at, sync_status
            FROM cached_resources
            WHERE source_id = ? AND resource_type = ? AND resource_id = ?
            "#,
        )
        .bind(source_id)
        .bind(resource_type)
        .bind(resource_id)
        .fetch_optional(self.pool.as_ref())
        .await?;

        Ok(row.map(|r| CachedResource {
            id: r.0,
            source_id: r.1,
            source_type: r.2,
            resource_type: r.3,
            resource_id: r.4,
            content_json: r.5,
            cached_at: r.6,
            expires_at: r.7,
            sync_status: r.8,
        }))
    }
}
