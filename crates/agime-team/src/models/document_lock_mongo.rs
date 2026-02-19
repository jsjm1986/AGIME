//! Document lock model for MongoDB

use chrono::{DateTime, Utc};
use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

/// Document lock - prevents concurrent editing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentLock {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub document_id: ObjectId,
    pub team_id: ObjectId,
    pub locked_by: String,
    pub locked_by_name: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub locked_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

/// Lock info for API responses
#[derive(Debug, Clone, Serialize)]
pub struct LockInfo {
    pub document_id: String,
    pub locked_by: String,
    pub locked_by_name: String,
    pub locked_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl From<DocumentLock> for LockInfo {
    fn from(lock: DocumentLock) -> Self {
        Self {
            document_id: lock.document_id.to_hex(),
            locked_by: lock.locked_by,
            locked_by_name: lock.locked_by_name,
            locked_at: lock.locked_at,
            expires_at: lock.expires_at,
        }
    }
}
