//! Audit log model for MongoDB

use chrono::{DateTime, Utc};
use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

/// Audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub team_id: ObjectId,
    pub user_id: String,
    pub user_name: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub resource_name: Option<String>,
    pub details: Option<String>,
    #[serde(skip_serializing)]
    pub ip_address: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// Audit log summary for list views
#[derive(Debug, Clone, Serialize)]
pub struct AuditLogSummary {
    pub id: String,
    #[serde(rename = "teamId")]
    pub team_id: String,
    #[serde(rename = "userId")]
    pub user_id: String,
    #[serde(rename = "userName")]
    pub user_name: Option<String>,
    pub action: String,
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(rename = "resourceId")]
    pub resource_id: Option<String>,
    #[serde(rename = "resourceName")]
    pub resource_name: Option<String>,
    pub details: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
}

impl From<AuditLog> for AuditLogSummary {
    fn from(log: AuditLog) -> Self {
        Self {
            id: log.id.map(|id| id.to_hex()).unwrap_or_default(),
            team_id: log.team_id.to_hex(),
            user_id: log.user_id,
            user_name: log.user_name,
            action: log.action,
            resource_type: log.resource_type,
            resource_id: log.resource_id,
            resource_name: log.resource_name,
            details: log.details,
            created_at: log.created_at,
        }
    }
}
