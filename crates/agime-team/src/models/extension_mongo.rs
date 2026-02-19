//! Extension model for MongoDB

use chrono::{DateTime, Utc};
use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

use super::common_mongo::{default_protection_level, default_version, default_visibility};

/// Shared extension document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extension {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub team_id: ObjectId,
    pub name: String,
    pub description: Option<String>,
    pub extension_type: String,
    pub config: mongodb::bson::Document,
    #[serde(default)]
    pub tags: Vec<String>,

    // Version control
    #[serde(default = "default_version")]
    pub version: String,
    pub previous_version_id: Option<String>,

    // Access control
    #[serde(default = "default_visibility")]
    pub visibility: String,
    #[serde(default = "default_protection_level")]
    pub protection_level: String,

    // Security review
    #[serde(default)]
    pub security_reviewed: bool,
    pub security_notes: Option<String>,
    pub reviewed_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<DateTime<Utc>>,

    // AI Describe
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_description_lang: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_described_at: Option<DateTime<Utc>>,

    // Statistics
    #[serde(default)]
    pub use_count: i32,
    #[serde(default)]
    pub is_deleted: bool,

    // Author info
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// Extension summary for list responses
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionSummary {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    pub extension_type: String,
    pub config: serde_json::Value,
    pub tags: Vec<String>,
    pub version: String,
    pub visibility: String,
    pub protection_level: String,
    pub security_reviewed: bool,
    pub use_count: i32,
    pub author_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_description_lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_described_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Extension> for ExtensionSummary {
    fn from(e: Extension) -> Self {
        Self {
            id: e.id.map(|id| id.to_hex()).unwrap_or_default(),
            team_id: e.team_id.to_hex(),
            name: e.name,
            description: e.description,
            extension_type: e.extension_type,
            config: bson::from_document(e.config).unwrap_or(serde_json::json!({})),
            tags: e.tags,
            version: e.version,
            visibility: e.visibility,
            protection_level: e.protection_level,
            security_reviewed: e.security_reviewed,
            use_count: e.use_count,
            author_id: e.created_by,
            ai_description: e.ai_description,
            ai_description_lang: e.ai_description_lang,
            ai_described_at: e.ai_described_at,
            created_at: e.created_at,
            updated_at: e.updated_at,
        }
    }
}
