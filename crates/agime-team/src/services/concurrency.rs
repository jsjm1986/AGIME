//! Concurrency control utilities
//!
//! This module provides optimistic locking and concurrency control mechanisms
//! for team resources. It uses the `updated_at` timestamp as a version marker
//! to detect concurrent modifications.

use crate::error::{TeamError, TeamResult};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

/// ETag for optimistic locking
///
/// The ETag is derived from the resource's updated_at timestamp.
/// When updating a resource, the client must provide the expected ETag,
/// and the update will only succeed if the current ETag matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ETag(pub String);

impl ETag {
    /// Create an ETag from a timestamp
    pub fn from_timestamp(ts: DateTime<Utc>) -> Self {
        ETag(format!("W/\"{:x}\"", ts.timestamp_millis()))
    }

    /// Parse an ETag string
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.starts_with("W/\"") && s.ends_with('"') {
            Some(ETag(s.to_string()))
        } else if s.starts_with('"') && s.ends_with('"') {
            // Strong ETag
            Some(ETag(s.to_string()))
        } else {
            None
        }
    }

    /// Get the raw ETag value
    pub fn value(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ETag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Concurrency control service
pub struct ConcurrencyService;

/// Allowed table names for concurrency operations (whitelist to prevent SQL injection)
const ALLOWED_TABLES: &[&str] = &[
    "shared_skills",
    "shared_recipes",
    "shared_extensions",
    "teams",
    "team_members",
    "installed_resources",
];

/// Validate that a table name is in the allowed whitelist
fn validate_table_name(table: &str) -> TeamResult<()> {
    if ALLOWED_TABLES.contains(&table) {
        Ok(())
    } else {
        Err(TeamError::Validation(format!(
            "Invalid table name: {}",
            table
        )))
    }
}

impl ConcurrencyService {
    pub fn new() -> Self {
        Self
    }

    /// Check if a resource can be updated (optimistic lock check)
    ///
    /// Returns the current ETag if the resource exists.
    /// The caller should compare this with the expected ETag before proceeding.
    pub async fn get_resource_etag(
        &self,
        pool: &SqlitePool,
        table: &str,
        resource_id: &str,
    ) -> TeamResult<Option<ETag>> {
        validate_table_name(table)?;
        let query = format!(
            "SELECT updated_at FROM {} WHERE id = ? AND is_deleted = 0",
            table
        );

        let result: Option<(DateTime<Utc>,)> = sqlx::query_as(&query)
            .bind(resource_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| TeamError::Database(e.to_string()))?;

        Ok(result.map(|(ts,)| ETag::from_timestamp(ts)))
    }

    /// Perform an optimistic update
    ///
    /// This updates the resource only if the current updated_at matches the expected ETag.
    /// Returns true if the update was successful, false if there was a conflict.
    pub async fn optimistic_update(
        &self,
        pool: &SqlitePool,
        table: &str,
        resource_id: &str,
        expected_etag: &ETag,
        update_sql: &str,
        bindings: Vec<String>,
    ) -> TeamResult<bool> {
        // First, get current timestamp
        let current_etag = self.get_resource_etag(pool, table, resource_id).await?;

        match current_etag {
            Some(current) if current == *expected_etag => {
                // ETags match, proceed with update
                // The update should set updated_at to now
                let now = Utc::now();

                // Build the full update query
                let full_query = format!(
                    "UPDATE {} SET {}, updated_at = ? WHERE id = ? AND is_deleted = 0",
                    table, update_sql
                );

                let mut query = sqlx::query(&full_query);
                for binding in bindings {
                    query = query.bind(binding);
                }
                query = query.bind(now).bind(resource_id);

                let result = query
                    .execute(pool)
                    .await
                    .map_err(|e| TeamError::Database(e.to_string()))?;

                Ok(result.rows_affected() > 0)
            }
            Some(_) => {
                // ETag mismatch - concurrent modification detected
                Ok(false)
            }
            None => {
                // Resource not found
                Err(TeamError::ResourceNotFound {
                    resource_type: table.to_string(),
                    resource_id: resource_id.to_string(),
                })
            }
        }
    }

    /// Acquire a soft lock on a resource
    ///
    /// This is a cooperative locking mechanism - it sets a lock timestamp
    /// that other clients can check before attempting updates.
    /// The lock expires after the specified duration.
    pub async fn try_acquire_lock(
        &self,
        pool: &SqlitePool,
        resource_type: &str,
        resource_id: &str,
        user_id: &str,
        lock_duration_secs: i64,
    ) -> TeamResult<bool> {
        let now = Utc::now();
        let expires_at = now + chrono::Duration::seconds(lock_duration_secs);

        // Try to insert a lock record, or update if expired
        let result = sqlx::query(
            r#"
            INSERT INTO resource_locks (id, resource_type, resource_id, user_id, acquired_at, expires_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(resource_type, resource_id) DO UPDATE
            SET user_id = excluded.user_id,
                acquired_at = excluded.acquired_at,
                expires_at = excluded.expires_at
            WHERE expires_at < ?
            "#,
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(resource_type)
        .bind(resource_id)
        .bind(user_id)
        .bind(now)
        .bind(expires_at)
        .bind(now)  // For the WHERE clause
        .execute(pool)
        .await;

        match result {
            Ok(r) => Ok(r.rows_affected() > 0),
            Err(e) => {
                // Table might not exist - that's ok, locking is optional
                tracing::debug!("Lock acquisition failed (table may not exist): {}", e);
                Ok(true) // Allow operation to proceed
            }
        }
    }

    /// Release a lock on a resource
    pub async fn release_lock(
        &self,
        pool: &SqlitePool,
        resource_type: &str,
        resource_id: &str,
        user_id: &str,
    ) -> TeamResult<()> {
        let _ = sqlx::query(
            r#"
            DELETE FROM resource_locks
            WHERE resource_type = ? AND resource_id = ? AND user_id = ?
            "#,
        )
        .bind(resource_type)
        .bind(resource_id)
        .bind(user_id)
        .execute(pool)
        .await;

        Ok(())
    }

    /// Check if a resource is locked by another user
    pub async fn is_locked_by_other(
        &self,
        pool: &SqlitePool,
        resource_type: &str,
        resource_id: &str,
        user_id: &str,
    ) -> TeamResult<Option<String>> {
        let now = Utc::now();

        let result: Option<(String,)> = sqlx::query_as(
            r#"
            SELECT user_id FROM resource_locks
            WHERE resource_type = ? AND resource_id = ? AND user_id != ? AND expires_at > ?
            "#,
        )
        .bind(resource_type)
        .bind(resource_id)
        .bind(user_id)
        .bind(now)
        .fetch_optional(pool)
        .await
        .unwrap_or(None);

        Ok(result.map(|(uid,)| uid))
    }
}

impl Default for ConcurrencyService {
    fn default() -> Self {
        Self::new()
    }
}

/// Conflict error for optimistic locking failures
#[derive(Debug, Clone)]
pub struct ConflictError {
    pub resource_type: String,
    pub resource_id: String,
    pub expected_etag: String,
    pub current_etag: String,
}

impl std::fmt::Display for ConflictError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Conflict: {} {} was modified (expected: {}, current: {})",
            self.resource_type, self.resource_id, self.expected_etag, self.current_etag
        )
    }
}

impl std::error::Error for ConflictError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_etag_from_timestamp() {
        let ts = Utc::now();
        let etag = ETag::from_timestamp(ts);
        assert!(etag.value().starts_with("W/\""));
        assert!(etag.value().ends_with('"'));
    }

    #[test]
    fn test_etag_parse() {
        let etag = ETag::parse("W/\"abc123\"");
        assert!(etag.is_some());
        assert_eq!(etag.unwrap().value(), "W/\"abc123\"");

        let strong = ETag::parse("\"strong-etag\"");
        assert!(strong.is_some());

        let invalid = ETag::parse("invalid");
        assert!(invalid.is_none());
    }

    #[test]
    fn test_etag_equality() {
        let ts = Utc::now();
        let etag1 = ETag::from_timestamp(ts);
        let etag2 = ETag::from_timestamp(ts);
        assert_eq!(etag1, etag2);
    }
}
