//! Document version model for MongoDB

use chrono::{DateTime, Utc};
use mongodb::bson::{oid::ObjectId, Binary};
use serde::{Deserialize, Serialize};

/// Document version - stores a snapshot of document content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentVersion {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub document_id: ObjectId,
    pub team_id: ObjectId,
    pub version_number: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Binary>,
    pub file_size: i64,
    pub message: String,
    pub created_by: String,
    pub created_by_name: String,
    pub tag: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// Version summary for list views (without content)
#[derive(Debug, Clone, Serialize)]
pub struct DocumentVersionSummary {
    pub id: String,
    pub version_number: i32,
    pub message: String,
    pub file_size: i64,
    pub created_by: String,
    pub created_by_name: String,
    pub tag: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl From<DocumentVersion> for DocumentVersionSummary {
    fn from(v: DocumentVersion) -> Self {
        Self {
            id: v.id.map(|id| id.to_hex()).unwrap_or_default(),
            version_number: v.version_number,
            message: v.message,
            file_size: v.file_size,
            created_by: v.created_by,
            created_by_name: v.created_by_name,
            tag: v.tag,
            created_at: v.created_at,
        }
    }
}
