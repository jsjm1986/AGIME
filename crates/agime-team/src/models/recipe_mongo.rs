//! Recipe model for MongoDB

use chrono::{DateTime, Utc};
use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

use super::common_mongo::{default_protection_level, default_version, default_visibility};

/// Shared recipe document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub team_id: ObjectId,
    pub name: String,
    pub description: Option<String>,
    pub content_yaml: String,
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,

    // Version control
    #[serde(default = "default_version")]
    pub version: String,
    pub previous_version_id: Option<String>,
    pub dependencies: Option<Vec<String>>,

    // Access control
    #[serde(default = "default_visibility")]
    pub visibility: String,
    #[serde(default = "default_protection_level")]
    pub protection_level: String,

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

/// Recipe summary for list responses
#[derive(Debug, Clone, Serialize)]
pub struct RecipeSummary {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    pub content_yaml: String,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub version: String,
    pub visibility: String,
    pub protection_level: String,
    pub use_count: i32,
    pub author_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Recipe> for RecipeSummary {
    fn from(r: Recipe) -> Self {
        Self {
            id: r.id.map(|id| id.to_hex()).unwrap_or_default(),
            team_id: r.team_id.to_hex(),
            name: r.name,
            description: r.description,
            content_yaml: r.content_yaml,
            category: r.category,
            tags: r.tags,
            version: r.version,
            visibility: r.visibility,
            protection_level: r.protection_level,
            use_count: r.use_count,
            author_id: r.created_by,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}
