//! User Group model for MongoDB
//! Provides group-based access control for team resources and agents

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// User group for team-level access control
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserGroup {
    /// Group name (unique within team)
    pub name: String,
    /// Group description
    #[serde(default)]
    pub description: Option<String>,
    /// Team this group belongs to
    pub team_id: String,
    /// User IDs in this group
    #[serde(default)]
    pub members: Vec<String>,
    /// Group color for UI display
    #[serde(default)]
    pub color: Option<String>,
    /// Whether this is a system-generated group
    #[serde(default)]
    pub is_system: bool,
    /// Soft delete flag
    #[serde(default)]
    pub is_deleted: bool,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Summary view for listing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserGroupSummary {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "memberCount")]
    pub member_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(rename = "isSystem")]
    pub is_system: bool,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: DateTime<Utc>,
}

/// Detailed view including members
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserGroupDetail {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub members: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(rename = "isSystem")]
    pub is_system: bool,
    #[serde(rename = "createdBy")]
    pub created_by: String,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: DateTime<Utc>,
}

/// Request to create a user group
#[derive(Debug, Clone, Deserialize)]
pub struct CreateUserGroupRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub color: Option<String>,
}

/// Request to update a user group
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateUserGroupRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
}

/// Request to add/remove members
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateGroupMembersRequest {
    #[serde(default)]
    pub add: Vec<String>,
    #[serde(default)]
    pub remove: Vec<String>,
}
