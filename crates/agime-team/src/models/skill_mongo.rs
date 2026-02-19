//! Skill model for MongoDB

use chrono::{DateTime, Utc};
use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

use super::common_mongo::{default_protection_level, default_version, default_visibility};

/// Storage type for skills
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SkillStorageType {
    #[default]
    Inline,
    Package,
}

/// Shared skill document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub team_id: ObjectId,
    pub name: String,
    pub description: Option<String>,

    // Storage type and content
    #[serde(default)]
    pub storage_type: SkillStorageType,
    pub content: Option<String>,
    pub skill_md: Option<String>,
    #[serde(default)]
    pub files: Vec<SkillFile>,
    pub manifest: Option<serde_json::Value>,

    // Package info (for Package storage type)
    pub package_url: Option<String>,
    pub package_hash: Option<String>,
    pub package_size: Option<i64>,

    // Metadata
    pub metadata: Option<serde_json::Value>,
    #[serde(default = "default_version")]
    pub version: String,
    pub previous_version_id: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub dependencies: Option<Vec<String>>,

    // Access control
    #[serde(default = "default_visibility")]
    pub visibility: String,
    #[serde(default = "default_protection_level")]
    pub protection_level: String,

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

/// Skill file entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFile {
    pub path: String,
    pub content: String,
}

/// Skill summary for list responses
#[derive(Debug, Clone, Serialize)]
pub struct SkillSummary {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    pub storage_type: String,
    pub version: String,
    pub tags: Vec<String>,
    pub visibility: String,
    pub protection_level: String,
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

impl From<Skill> for SkillSummary {
    fn from(s: Skill) -> Self {
        Self {
            id: s.id.map(|id| id.to_hex()).unwrap_or_default(),
            team_id: s.team_id.to_hex(),
            name: s.name,
            description: s.description,
            storage_type: match s.storage_type {
                SkillStorageType::Inline => "inline".to_string(),
                SkillStorageType::Package => "package".to_string(),
            },
            version: s.version,
            tags: s.tags,
            visibility: s.visibility,
            protection_level: s.protection_level,
            use_count: s.use_count,
            author_id: s.created_by,
            ai_description: s.ai_description,
            ai_description_lang: s.ai_description_lang,
            ai_described_at: s.ai_described_at,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}
