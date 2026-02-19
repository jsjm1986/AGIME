//! Folder model for MongoDB

use chrono::{DateTime, Utc};
use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

/// Folder document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub team_id: ObjectId,
    pub name: String,
    pub parent_path: String,
    pub full_path: String,
    pub description: Option<String>,
    pub created_by: String,
    #[serde(default)]
    pub is_deleted: bool,
    /// System folders cannot be deleted or renamed
    #[serde(default)]
    pub is_system: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// Folder summary for list views
#[derive(Debug, Clone, Serialize)]
pub struct FolderSummary {
    pub id: String,
    pub name: String,
    pub parent_path: String,
    pub full_path: String,
    pub description: Option<String>,
    pub created_by: String,
    pub is_system: bool,
    pub created_at: DateTime<Utc>,
}

impl From<Folder> for FolderSummary {
    fn from(f: Folder) -> Self {
        Self {
            id: f.id.map(|id| id.to_hex()).unwrap_or_default(),
            name: f.name,
            parent_path: f.parent_path,
            full_path: f.full_path,
            description: f.description,
            created_by: f.created_by,
            is_system: f.is_system,
            created_at: f.created_at,
        }
    }
}

/// Folder tree node for hierarchical display
#[derive(Debug, Clone, Serialize)]
pub struct FolderTreeNode {
    pub id: String,
    pub name: String,
    pub full_path: String,
    #[serde(default)]
    pub is_system: bool,
    pub children: Vec<FolderTreeNode>,
}
