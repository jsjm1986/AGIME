//! Audit logging service
//!
//! This module provides comprehensive audit logging for team operations.
//! It tracks all changes to resources for security and compliance purposes.

use crate::error::{TeamError, TeamResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub user_id: String,
    pub action: AuditAction,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub team_id: Option<String>,
    pub details: serde_json::Value,
    pub old_value: Option<serde_json::Value>,
    pub new_value: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub success: bool,
    pub error_message: Option<String>,
}

/// Audit action types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    // Team actions
    TeamCreate,
    TeamUpdate,
    TeamDelete,
    // Member actions
    MemberAdd,
    MemberUpdate,
    MemberRemove,
    MemberRoleChange,
    // Resource actions
    ResourceCreate,
    ResourceUpdate,
    ResourceDelete,
    ResourceInstall,
    ResourceUninstall,
    ResourceShare,
    // Security actions
    SecurityReview,
    SecurityApprove,
    SecurityReject,
    // Sync actions
    SyncStart,
    SyncComplete,
    SyncFailed,
    // Authentication actions
    Login,
    Logout,
    TokenRefresh,
    // Other
    ConfigChange,
    Custom,
}

impl std::fmt::Display for AuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditAction::TeamCreate => write!(f, "team_create"),
            AuditAction::TeamUpdate => write!(f, "team_update"),
            AuditAction::TeamDelete => write!(f, "team_delete"),
            AuditAction::MemberAdd => write!(f, "member_add"),
            AuditAction::MemberUpdate => write!(f, "member_update"),
            AuditAction::MemberRemove => write!(f, "member_remove"),
            AuditAction::MemberRoleChange => write!(f, "member_role_change"),
            AuditAction::ResourceCreate => write!(f, "resource_create"),
            AuditAction::ResourceUpdate => write!(f, "resource_update"),
            AuditAction::ResourceDelete => write!(f, "resource_delete"),
            AuditAction::ResourceInstall => write!(f, "resource_install"),
            AuditAction::ResourceUninstall => write!(f, "resource_uninstall"),
            AuditAction::ResourceShare => write!(f, "resource_share"),
            AuditAction::SecurityReview => write!(f, "security_review"),
            AuditAction::SecurityApprove => write!(f, "security_approve"),
            AuditAction::SecurityReject => write!(f, "security_reject"),
            AuditAction::SyncStart => write!(f, "sync_start"),
            AuditAction::SyncComplete => write!(f, "sync_complete"),
            AuditAction::SyncFailed => write!(f, "sync_failed"),
            AuditAction::Login => write!(f, "login"),
            AuditAction::Logout => write!(f, "logout"),
            AuditAction::TokenRefresh => write!(f, "token_refresh"),
            AuditAction::ConfigChange => write!(f, "config_change"),
            AuditAction::Custom => write!(f, "custom"),
        }
    }
}

impl std::str::FromStr for AuditAction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "team_create" => Ok(AuditAction::TeamCreate),
            "team_update" => Ok(AuditAction::TeamUpdate),
            "team_delete" => Ok(AuditAction::TeamDelete),
            "member_add" => Ok(AuditAction::MemberAdd),
            "member_update" => Ok(AuditAction::MemberUpdate),
            "member_remove" => Ok(AuditAction::MemberRemove),
            "member_role_change" => Ok(AuditAction::MemberRoleChange),
            "resource_create" => Ok(AuditAction::ResourceCreate),
            "resource_update" => Ok(AuditAction::ResourceUpdate),
            "resource_delete" => Ok(AuditAction::ResourceDelete),
            "resource_install" => Ok(AuditAction::ResourceInstall),
            "resource_uninstall" => Ok(AuditAction::ResourceUninstall),
            "resource_share" => Ok(AuditAction::ResourceShare),
            "security_review" => Ok(AuditAction::SecurityReview),
            "security_approve" => Ok(AuditAction::SecurityApprove),
            "security_reject" => Ok(AuditAction::SecurityReject),
            "sync_start" => Ok(AuditAction::SyncStart),
            "sync_complete" => Ok(AuditAction::SyncComplete),
            "sync_failed" => Ok(AuditAction::SyncFailed),
            "login" => Ok(AuditAction::Login),
            "logout" => Ok(AuditAction::Logout),
            "token_refresh" => Ok(AuditAction::TokenRefresh),
            "config_change" => Ok(AuditAction::ConfigChange),
            "custom" => Ok(AuditAction::Custom),
            _ => Err(format!("Unknown audit action: {}", s)),
        }
    }
}

/// Query parameters for listing audit logs
#[derive(Debug, Clone, Default)]
pub struct AuditLogQuery {
    pub user_id: Option<String>,
    pub action: Option<AuditAction>,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub team_id: Option<String>,
    pub from_timestamp: Option<DateTime<Utc>>,
    pub to_timestamp: Option<DateTime<Utc>>,
    pub success_only: Option<bool>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// Audit service for logging and querying audit events
pub struct AuditService;

impl AuditService {
    pub fn new() -> Self {
        Self
    }

    /// Log an audit event
    pub async fn log(
        &self,
        pool: &SqlitePool,
        user_id: &str,
        action: AuditAction,
        resource_type: Option<&str>,
        resource_id: Option<&str>,
        team_id: Option<&str>,
        details: Option<serde_json::Value>,
        old_value: Option<serde_json::Value>,
        new_value: Option<serde_json::Value>,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
    ) -> TeamResult<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let action_str = action.to_string();
        let details_json = details.unwrap_or(serde_json::json!({})).to_string();
        let old_value_json = old_value.map(|v| v.to_string());
        let new_value_json = new_value.map(|v| v.to_string());

        sqlx::query(
            r#"
            INSERT INTO audit_logs (
                id, timestamp, user_id, action, resource_type, resource_id,
                team_id, details_json, old_value_json, new_value_json,
                ip_address, user_agent, success
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)
            "#,
        )
        .bind(&id)
        .bind(now)
        .bind(user_id)
        .bind(&action_str)
        .bind(resource_type)
        .bind(resource_id)
        .bind(team_id)
        .bind(&details_json)
        .bind(&old_value_json)
        .bind(&new_value_json)
        .bind(ip_address)
        .bind(user_agent)
        .execute(pool)
        .await
        .map_err(|e| TeamError::Database(e.to_string()))?;

        Ok(id)
    }

    /// Log a failed operation
    pub async fn log_failure(
        &self,
        pool: &SqlitePool,
        user_id: &str,
        action: AuditAction,
        resource_type: Option<&str>,
        resource_id: Option<&str>,
        team_id: Option<&str>,
        error_message: &str,
        ip_address: Option<&str>,
    ) -> TeamResult<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let action_str = action.to_string();

        sqlx::query(
            r#"
            INSERT INTO audit_logs (
                id, timestamp, user_id, action, resource_type, resource_id,
                team_id, success, error_message, ip_address
            ) VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(now)
        .bind(user_id)
        .bind(&action_str)
        .bind(resource_type)
        .bind(resource_id)
        .bind(team_id)
        .bind(error_message)
        .bind(ip_address)
        .execute(pool)
        .await
        .map_err(|e| TeamError::Database(e.to_string()))?;

        Ok(id)
    }

    /// Query audit logs with filters
    pub async fn query(
        &self,
        pool: &SqlitePool,
        query: AuditLogQuery,
    ) -> TeamResult<Vec<AuditLog>> {
        let mut sql = String::from(
            r#"
            SELECT id, timestamp, user_id, action, resource_type, resource_id,
                   team_id, details_json, old_value_json, new_value_json,
                   ip_address, user_agent, success, error_message
            FROM audit_logs
            WHERE 1=1
            "#,
        );

        let mut bindings: Vec<String> = vec![];

        if let Some(ref user_id) = query.user_id {
            sql.push_str(" AND user_id = ?");
            bindings.push(user_id.clone());
        }

        if let Some(ref action) = query.action {
            sql.push_str(" AND action = ?");
            bindings.push(action.to_string());
        }

        if let Some(ref resource_type) = query.resource_type {
            sql.push_str(" AND resource_type = ?");
            bindings.push(resource_type.clone());
        }

        if let Some(ref resource_id) = query.resource_id {
            sql.push_str(" AND resource_id = ?");
            bindings.push(resource_id.clone());
        }

        if let Some(ref team_id) = query.team_id {
            sql.push_str(" AND team_id = ?");
            bindings.push(team_id.clone());
        }

        if let Some(success_only) = query.success_only {
            sql.push_str(" AND success = ?");
            bindings.push(if success_only {
                "1".to_string()
            } else {
                "0".to_string()
            });
        }

        sql.push_str(" ORDER BY timestamp DESC");

        let limit = query.limit.unwrap_or(100);
        let offset = query.offset.unwrap_or(0);
        sql.push_str(" LIMIT ? OFFSET ?");

        // Build and execute query with dynamic bindings
        let mut query_builder = sqlx::query_as::<
            _,
            (
                String,
                DateTime<Utc>,
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                i32,
                Option<String>,
            ),
        >(&sql);

        for binding in bindings {
            query_builder = query_builder.bind(binding);
        }
        query_builder = query_builder.bind(limit as i64).bind(offset as i64);

        let rows = query_builder
            .fetch_all(pool)
            .await
            .map_err(|e| TeamError::Database(e.to_string()))?;

        let logs = rows
            .into_iter()
            .map(|row| {
                let action = row.3.parse().unwrap_or(AuditAction::Custom);
                let details = row
                    .7
                    .as_ref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(serde_json::json!({}));
                let old_value = row.8.as_ref().and_then(|s| serde_json::from_str(s).ok());
                let new_value = row.9.as_ref().and_then(|s| serde_json::from_str(s).ok());

                AuditLog {
                    id: row.0,
                    timestamp: row.1,
                    user_id: row.2,
                    action,
                    resource_type: row.4,
                    resource_id: row.5,
                    team_id: row.6,
                    details,
                    old_value,
                    new_value,
                    ip_address: row.10,
                    user_agent: row.11,
                    success: row.12 == 1,
                    error_message: row.13,
                }
            })
            .collect();

        Ok(logs)
    }

    /// Get audit logs for a specific resource
    pub async fn get_resource_history(
        &self,
        pool: &SqlitePool,
        resource_type: &str,
        resource_id: &str,
        limit: u32,
    ) -> TeamResult<Vec<AuditLog>> {
        self.query(
            pool,
            AuditLogQuery {
                resource_type: Some(resource_type.to_string()),
                resource_id: Some(resource_id.to_string()),
                limit: Some(limit),
                ..Default::default()
            },
        )
        .await
    }

    /// Get audit logs for a specific user
    pub async fn get_user_activity(
        &self,
        pool: &SqlitePool,
        user_id: &str,
        limit: u32,
    ) -> TeamResult<Vec<AuditLog>> {
        self.query(
            pool,
            AuditLogQuery {
                user_id: Some(user_id.to_string()),
                limit: Some(limit),
                ..Default::default()
            },
        )
        .await
    }

    /// Get audit logs for a specific team
    pub async fn get_team_activity(
        &self,
        pool: &SqlitePool,
        team_id: &str,
        limit: u32,
    ) -> TeamResult<Vec<AuditLog>> {
        self.query(
            pool,
            AuditLogQuery {
                team_id: Some(team_id.to_string()),
                limit: Some(limit),
                ..Default::default()
            },
        )
        .await
    }

    /// Get failed operations (for security monitoring)
    pub async fn get_failed_operations(
        &self,
        pool: &SqlitePool,
        limit: u32,
    ) -> TeamResult<Vec<AuditLog>> {
        self.query(
            pool,
            AuditLogQuery {
                success_only: Some(false),
                limit: Some(limit),
                ..Default::default()
            },
        )
        .await
    }

    /// Clean up old audit logs (retention policy)
    pub async fn cleanup_old_logs(&self, pool: &SqlitePool, days_to_keep: u32) -> TeamResult<u64> {
        let cutoff = Utc::now() - chrono::Duration::days(days_to_keep as i64);

        let result = sqlx::query("DELETE FROM audit_logs WHERE timestamp < ?")
            .bind(cutoff)
            .execute(pool)
            .await
            .map_err(|e| TeamError::Database(e.to_string()))?;

        Ok(result.rows_affected())
    }
}

impl Default for AuditService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_action_display() {
        assert_eq!(AuditAction::TeamCreate.to_string(), "team_create");
        assert_eq!(AuditAction::ResourceUpdate.to_string(), "resource_update");
    }

    #[test]
    fn test_audit_action_parse() {
        assert_eq!(
            "team_create".parse::<AuditAction>().unwrap(),
            AuditAction::TeamCreate
        );
        assert_eq!(
            "resource_update".parse::<AuditAction>().unwrap(),
            AuditAction::ResourceUpdate
        );
    }
}
