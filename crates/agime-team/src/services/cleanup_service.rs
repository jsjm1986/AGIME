//! Cleanup service - handles resource cleanup when members leave teams

use sqlx::SqlitePool;
use std::path::PathBuf;
use tracing::{info, warn};

use crate::error::TeamResult;
use crate::models::ResourceType;

/// Result of a cleanup operation
#[derive(Debug, Clone)]
pub struct CleanupResult {
    /// Number of resources cleaned up
    pub cleaned_count: usize,
    /// Resources that failed to clean up
    pub failures: Vec<CleanupFailure>,
    /// Total bytes freed (if applicable)
    pub bytes_freed: u64,
}

impl CleanupResult {
    pub fn empty() -> Self {
        Self {
            cleaned_count: 0,
            failures: Vec::new(),
            bytes_freed: 0,
        }
    }

    pub fn success(cleaned_count: usize, bytes_freed: u64) -> Self {
        Self {
            cleaned_count,
            failures: Vec::new(),
            bytes_freed,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Details of a cleanup failure
#[derive(Debug, Clone)]
pub struct CleanupFailure {
    pub resource_type: ResourceType,
    pub resource_id: String,
    pub resource_name: String,
    pub error: String,
}

/// Cleanup service for managing resource lifecycle
pub struct CleanupService;

impl CleanupService {
    /// Create a new cleanup service
    pub fn new() -> Self {
        Self
    }

    /// Clean up all resources for a user in a specific team
    /// Called when a member leaves or is removed from a team
    pub async fn cleanup_user_team_resources(
        &self,
        pool: &SqlitePool,
        team_id: &str,
        user_id: &str,
        _base_path: &PathBuf,
    ) -> TeamResult<CleanupResult> {
        info!(
            team_id = team_id,
            user_id = user_id,
            "Starting cleanup of user's team resources"
        );

        // Find all installed resources for this user in this team
        let resources: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
            r#"
            SELECT id, resource_type, resource_name, local_path
            FROM installed_resources
            WHERE team_id = ? AND user_id = ?
            "#,
        )
        .bind(team_id)
        .bind(user_id)
        .fetch_all(pool)
        .await?;

        if resources.is_empty() {
            info!("No resources found for cleanup");
            return Ok(CleanupResult::empty());
        }

        let mut cleaned_count = 0;
        let mut failures = Vec::new();
        let mut bytes_freed = 0u64;

        for (id, resource_type, resource_name, local_path) in resources {
            // Try to delete local files
            if let Some(path_str) = local_path {
                let path = PathBuf::from(&path_str);
                if path.exists() {
                    match std::fs::metadata(&path) {
                        Ok(metadata) => {
                            if metadata.is_dir() {
                                // Calculate directory size before deletion
                                bytes_freed += calculate_dir_size(&path).unwrap_or(0);
                            } else {
                                bytes_freed += metadata.len();
                            }
                        }
                        Err(_) => {}
                    }

                    if let Err(e) = std::fs::remove_dir_all(&path) {
                        warn!(
                            path = %path_str,
                            error = %e,
                            "Failed to delete local files, continuing with database cleanup"
                        );
                        // Don't fail the whole operation, just record the failure
                        failures.push(CleanupFailure {
                            resource_type: resource_type.parse().unwrap_or(ResourceType::Skill),
                            resource_id: id.clone(),
                            resource_name: resource_name.clone(),
                            error: format!("Failed to delete files: {}", e),
                        });
                    }
                }
            }

            // Delete database record
            if let Err(e) = sqlx::query("DELETE FROM installed_resources WHERE id = ?")
                .bind(&id)
                .execute(pool)
                .await
            {
                warn!(
                    id = %id,
                    error = %e,
                    "Failed to delete database record"
                );
                failures.push(CleanupFailure {
                    resource_type: resource_type.parse().unwrap_or(ResourceType::Skill),
                    resource_id: id,
                    resource_name,
                    error: format!("Failed to delete database record: {}", e),
                });
            } else {
                cleaned_count += 1;
            }
        }

        info!(
            cleaned = cleaned_count,
            failures = failures.len(),
            bytes_freed = bytes_freed,
            "Cleanup completed"
        );

        Ok(CleanupResult {
            cleaned_count,
            failures,
            bytes_freed,
        })
    }

    /// Clean up all resources with expired authorizations
    /// Should be called periodically (e.g., by a background task)
    pub async fn cleanup_expired_authorizations(
        &self,
        pool: &SqlitePool,
        _base_path: &PathBuf,
    ) -> TeamResult<CleanupResult> {
        info!("Starting cleanup of expired authorizations");

        // Find resources with expired authorizations (beyond grace period)
        // Grace period is 72 hours from last_verified_at
        let resources: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
            r#"
            SELECT id, resource_type, resource_name, local_path
            FROM installed_resources
            WHERE authorization_expires_at IS NOT NULL
              AND datetime(authorization_expires_at) < datetime('now')
              AND (
                  last_verified_at IS NULL
                  OR datetime(last_verified_at, '+72 hours') < datetime('now')
              )
              AND protection_level != 'public'
            "#,
        )
        .fetch_all(pool)
        .await?;

        if resources.is_empty() {
            info!("No expired authorizations found");
            return Ok(CleanupResult::empty());
        }

        let mut cleaned_count = 0;
        let mut failures = Vec::new();
        let mut bytes_freed = 0u64;

        for (id, resource_type, resource_name, local_path) in resources {
            // Delete local files
            if let Some(path_str) = local_path {
                let path = PathBuf::from(&path_str);
                if path.exists() {
                    if let Ok(metadata) = std::fs::metadata(&path) {
                        if metadata.is_dir() {
                            bytes_freed += calculate_dir_size(&path).unwrap_or(0);
                        } else {
                            bytes_freed += metadata.len();
                        }
                    }

                    if let Err(e) = std::fs::remove_dir_all(&path) {
                        warn!(
                            path = %path_str,
                            error = %e,
                            "Failed to delete expired resource files"
                        );
                        failures.push(CleanupFailure {
                            resource_type: resource_type.parse().unwrap_or(ResourceType::Skill),
                            resource_id: id.clone(),
                            resource_name: resource_name.clone(),
                            error: format!("Failed to delete files: {}", e),
                        });
                    }
                }
            }

            // Delete database record
            if let Err(e) = sqlx::query("DELETE FROM installed_resources WHERE id = ?")
                .bind(&id)
                .execute(pool)
                .await
            {
                failures.push(CleanupFailure {
                    resource_type: resource_type.parse().unwrap_or(ResourceType::Skill),
                    resource_id: id,
                    resource_name,
                    error: format!("Database delete failed: {}", e),
                });
            } else {
                cleaned_count += 1;
            }
        }

        info!(
            cleaned = cleaned_count,
            failures = failures.len(),
            "Expired authorization cleanup completed"
        );

        Ok(CleanupResult {
            cleaned_count,
            failures,
            bytes_freed,
        })
    }

    /// Get count of resources that would be cleaned up for a user
    /// Useful for confirmation dialogs
    pub async fn get_cleanup_count(
        &self,
        pool: &SqlitePool,
        team_id: &str,
        user_id: &str,
    ) -> TeamResult<usize> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM installed_resources
            WHERE team_id = ? AND user_id = ?
            "#,
        )
        .bind(team_id)
        .bind(user_id)
        .fetch_one(pool)
        .await?;

        Ok(count.0 as usize)
    }

    /// Get count of resources with expired authorizations
    pub async fn get_expired_count(&self, pool: &SqlitePool) -> TeamResult<usize> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM installed_resources
            WHERE authorization_expires_at IS NOT NULL
              AND datetime(authorization_expires_at) < datetime('now')
              AND (
                  last_verified_at IS NULL
                  OR datetime(last_verified_at, '+72 hours') < datetime('now')
              )
              AND protection_level != 'public'
            "#,
        )
        .fetch_one(pool)
        .await?;

        Ok(count.0 as usize)
    }
}

impl Default for CleanupService {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculate the total size of a directory recursively
pub fn calculate_dir_size(path: &PathBuf) -> std::io::Result<u64> {
    let mut size = 0u64;

    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                size += calculate_dir_size(&path)?;
            } else {
                size += entry.metadata()?.len();
            }
        }
    }

    Ok(size)
}
