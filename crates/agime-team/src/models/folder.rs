//! Team folder model for document organization

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Team folder for organizing documents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamFolder {
    pub id: String,
    pub team_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub path: String,
    pub description: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub is_deleted: bool,
}

/// Request to create a new folder
#[derive(Debug, Clone, Deserialize)]
pub struct CreateFolderRequest {
    pub name: String,
    pub parent_id: Option<String>,
    pub description: Option<String>,
}

/// Request to update a folder
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateFolderRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Folder tree node for hierarchical display
#[derive(Debug, Clone, Serialize)]
pub struct FolderTreeNode {
    pub id: String,
    pub name: String,
    pub path: String,
    pub description: Option<String>,
    pub children: Vec<FolderTreeNode>,
    pub document_count: i64,
}
