//! Portal model for MongoDB — Agent-Powered Portal system

use chrono::{DateTime, Utc};
use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PortalStatus {
    #[default]
    Draft,
    Published,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PortalOutputForm {
    #[default]
    Website,
    Widget,
    AgentOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionType {
    PageView,
    ChatMessage,
    FormSubmit,
}

// ---------------------------------------------------------------------------
// Portal
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Portal {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub team_id: ObjectId,
    pub slug: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub status: PortalStatus,
    #[serde(default)]
    pub output_form: PortalOutputForm,

    // Embedded agent config
    #[serde(default)]
    pub agent_enabled: bool,
    /// Coding agent for Portal laboratory sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coding_agent_id: Option<String>,
    /// External service agent for public visitor sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_agent_id: Option<String>,
    /// Legacy single-agent field (backward compatibility).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_welcome_message: Option<String>,

    // Document binding (agent context)
    #[serde(default)]
    pub bound_document_ids: Vec<String>,

    /// Optional runtime extension allowlist for visitor sessions.
    /// Uses runtime names (e.g. "developer", "todo", "team_skills", custom extension names).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_extensions: Option<Vec<String>>,
    /// Optional skill id allowlist for visitor sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_skill_ids: Option<Vec<String>>,

    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub settings: serde_json::Value,

    /// Filesystem project path (for file-based portals)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,

    pub created_by: String,
    #[serde(default)]
    pub is_deleted: bool,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::models::mongo::common_mongo::bson_datetime_option"
    )]
    pub published_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// PortalInteraction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortalInteraction {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub portal_id: ObjectId,
    pub team_id: ObjectId,
    pub visitor_id: String,
    pub interaction_type: InteractionType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_path: Option<String>,
    #[serde(default)]
    pub data: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Summary types (for list views)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalSummary {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub status: PortalStatus,
    pub output_form: PortalOutputForm,
    pub agent_enabled: bool,
    pub coding_agent_id: Option<String>,
    pub service_agent_id: Option<String>,
    pub agent_id: Option<String>,
    pub allowed_extensions: Option<Vec<String>>,
    pub allowed_skill_ids: Option<Vec<String>>,
    pub tags: Vec<String>,
    pub project_path: Option<String>,
    pub created_by: String,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Portal> for PortalSummary {
    fn from(p: Portal) -> Self {
        Self {
            id: p.id.map(|id| id.to_hex()).unwrap_or_default(),
            slug: p.slug,
            name: p.name,
            description: p.description,
            status: p.status,
            output_form: p.output_form,
            agent_enabled: p.agent_enabled,
            coding_agent_id: p.coding_agent_id,
            service_agent_id: p.service_agent_id,
            agent_id: p.agent_id,
            allowed_extensions: p.allowed_extensions,
            allowed_skill_ids: p.allowed_skill_ids,
            tags: p.tags,
            project_path: p.project_path,
            created_by: p.created_by,
            published_at: p.published_at,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePortalRequest {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub output_form: Option<PortalOutputForm>,
    pub agent_enabled: Option<bool>,
    pub coding_agent_id: Option<String>,
    pub service_agent_id: Option<String>,
    /// Legacy single-agent field (backward compatibility).
    pub agent_id: Option<String>,
    pub agent_system_prompt: Option<String>,
    pub agent_welcome_message: Option<String>,
    pub bound_document_ids: Option<Vec<String>>,
    pub allowed_extensions: Option<Vec<String>>,
    pub allowed_skill_ids: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePortalRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub output_form: Option<PortalOutputForm>,
    pub agent_enabled: Option<bool>,
    pub coding_agent_id: Option<Option<String>>,
    pub service_agent_id: Option<Option<String>>,
    /// Supports three-state updates:
    /// - None: field omitted (keep existing)
    /// - Some(Some(v)): set to value
    /// - Some(None): clear field (set null)
    /// Legacy single-agent field (backward compatibility).
    pub agent_id: Option<Option<String>>,
    pub agent_system_prompt: Option<Option<String>>,
    pub agent_welcome_message: Option<Option<String>>,
    pub bound_document_ids: Option<Vec<String>>,
    pub allowed_extensions: Option<Vec<String>>,
    pub allowed_skill_ids: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalDetail {
    pub id: String,
    pub team_id: String,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub status: PortalStatus,
    pub output_form: PortalOutputForm,
    pub agent_enabled: bool,
    pub coding_agent_id: Option<String>,
    pub service_agent_id: Option<String>,
    /// Legacy single-agent field (backward compatibility).
    pub agent_id: Option<String>,
    pub agent_system_prompt: Option<String>,
    pub agent_welcome_message: Option<String>,
    pub bound_document_ids: Vec<String>,
    pub allowed_extensions: Option<Vec<String>>,
    pub allowed_skill_ids: Option<Vec<String>>,
    pub tags: Vec<String>,
    pub settings: serde_json::Value,
    pub project_path: Option<String>,
    pub created_by: String,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Portal> for PortalDetail {
    fn from(p: Portal) -> Self {
        Self {
            id: p.id.map(|id| id.to_hex()).unwrap_or_default(),
            team_id: p.team_id.to_hex(),
            slug: p.slug,
            name: p.name,
            description: p.description,
            status: p.status,
            output_form: p.output_form,
            agent_enabled: p.agent_enabled,
            coding_agent_id: p.coding_agent_id,
            service_agent_id: p.service_agent_id,
            agent_id: p.agent_id,
            agent_system_prompt: p.agent_system_prompt,
            agent_welcome_message: p.agent_welcome_message,
            bound_document_ids: p.bound_document_ids,
            allowed_extensions: p.allowed_extensions,
            allowed_skill_ids: p.allowed_skill_ids,
            tags: p.tags,
            settings: p.settings,
            project_path: p.project_path,
            created_by: p.created_by,
            published_at: p.published_at,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}
