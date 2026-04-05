//! Document model for MongoDB (simplified without GridFS)

use chrono::{DateTime, Utc};
use mongodb::bson::{oid::ObjectId, Binary};
use serde::{Deserialize, Serialize};

/// Document origin — who created the document
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DocumentOrigin {
    #[default]
    Human,
    Agent,
}

/// Document lifecycle status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStatus {
    #[default]
    Active,
    Draft,
    Accepted,
    Archived,
    Superseded,
}

/// Document category for classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DocumentCategory {
    #[default]
    General,
    Report,
    Translation,
    Summary,
    Review,
    Code,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentSourceSpaceType {
    PersonalChat,
    TeamChannel,
    AgentApp,
    Portal,
    Mission,
    System,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiWorkbenchGroup {
    Draft,
    Report,
    Summary,
    Review,
    Plan,
    Research,
    Artifact,
    Code,
    Other,
}

/// Lightweight snapshot of a source document, embedded in derived documents
/// to preserve lineage even if the source is later deleted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceDocumentSnapshot {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub origin: DocumentOrigin,
    pub category: DocumentCategory,
}

/// Archived document metadata (stored in archived_documents collection).
/// Created when a document is soft-deleted, preserving metadata without binary content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedDocument {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    /// Original document _id (for lineage lookups)
    pub original_id: ObjectId,
    pub team_id: ObjectId,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub mime_type: String,
    pub file_size: i64,
    pub folder_path: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub uploaded_by: String,
    pub origin: DocumentOrigin,
    pub status: DocumentStatus,
    pub category: DocumentCategory,
    #[serde(default)]
    pub source_document_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_space_type: Option<DocumentSourceSpaceType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_space_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_space_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_channel_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_thread_root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_channel_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_workbench_group: Option<AiWorkbenchGroup>,
    // Deletion metadata
    pub deleted_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub deleted_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletion_reason: Option<String>,
    // Original timestamps
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl ArchivedDocument {
    /// Create an archived record from a live document.
    pub fn from_document(doc: &Document, deleted_by: &str, reason: Option<String>) -> Self {
        Self {
            id: None,
            original_id: doc.id.unwrap_or_default(),
            team_id: doc.team_id,
            name: doc.name.clone(),
            display_name: doc.display_name.clone(),
            description: doc.description.clone(),
            mime_type: doc.mime_type.clone(),
            file_size: doc.file_size,
            folder_path: doc.folder_path.clone(),
            tags: doc.tags.clone(),
            uploaded_by: doc.uploaded_by.clone(),
            origin: doc.origin,
            status: doc.status,
            category: doc.category,
            source_document_ids: doc.source_document_ids.clone(),
            lineage_description: doc.lineage_description.clone(),
            created_by_agent_id: doc.created_by_agent_id.clone(),
            source_space_type: doc.source_space_type,
            source_space_id: doc.source_space_id.clone(),
            source_space_name: doc.source_space_name.clone(),
            source_channel_id: doc.source_channel_id.clone(),
            source_channel_name: doc.source_channel_name.clone(),
            source_thread_root_id: doc.source_thread_root_id.clone(),
            source_channel_run_id: doc.source_channel_run_id.clone(),
            ai_workbench_group: doc.ai_workbench_group,
            deleted_by: deleted_by.to_string(),
            deleted_at: Utc::now(),
            deletion_reason: reason,
            created_at: doc.created_at,
            updated_at: doc.updated_at,
        }
    }

    /// Convert to a DocumentSummary for lineage display (status forced to Archived).
    pub fn to_summary(&self) -> DocumentSummary {
        DocumentSummary {
            id: self.original_id.to_hex(),
            name: self.name.clone(),
            display_name: self.display_name.clone(),
            description: self.description.clone(),
            mime_type: self.mime_type.clone(),
            file_size: self.file_size,
            folder_path: self.folder_path.clone(),
            tags: self.tags.clone(),
            uploaded_by: self.uploaded_by.clone(),
            origin: self.origin,
            status: DocumentStatus::Archived,
            category: self.category,
            source_document_ids: self.source_document_ids.clone(),
            source_session_id: None,
            source_mission_id: None,
            created_by_agent_id: self.created_by_agent_id.clone(),
            source_space_type: self.source_space_type,
            source_space_id: self.source_space_id.clone(),
            source_space_name: self.source_space_name.clone(),
            source_channel_id: self.source_channel_id.clone(),
            source_channel_name: self.source_channel_name.clone(),
            source_thread_root_id: self.source_thread_root_id.clone(),
            source_channel_run_id: self.source_channel_run_id.clone(),
            ai_workbench_group: self.ai_workbench_group,
            supersedes_id: None,
            lineage_description: self.lineage_description.clone(),
            is_public: false,
            source_snapshots: vec![],
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Document metadata (stored in documents collection)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub team_id: ObjectId,
    pub folder_path: String, // e.g., "/docs/reports"
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub mime_type: String,
    pub file_size: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Binary>, // Binary content (for small files)
    #[serde(default)]
    pub tags: Vec<String>,
    pub uploaded_by: String,
    #[serde(default)]
    pub is_deleted: bool,
    /// Whether this document is public (bound to a portal, accessible externally)
    #[serde(default)]
    pub is_public: bool,
    // Phase 2: Agent integration fields
    #[serde(default)]
    pub origin: DocumentOrigin,
    #[serde(default)]
    pub status: DocumentStatus,
    #[serde(default)]
    pub category: DocumentCategory,
    #[serde(default)]
    pub source_document_ids: Vec<String>,
    /// Embedded snapshots of source documents at creation time (self-contained lineage)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_snapshots: Vec<SourceDocumentSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_mission_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_space_type: Option<DocumentSourceSpaceType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_space_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_space_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_channel_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_thread_root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_channel_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_workbench_group: Option<AiWorkbenchGroup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_id: Option<String>,
    /// Agent-provided description of what changed relative to source documents
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_description: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// Document summary for list views
#[derive(Debug, Clone, Serialize)]
pub struct DocumentSummary {
    pub id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub mime_type: String,
    pub file_size: i64,
    pub folder_path: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub uploaded_by: String,
    pub origin: DocumentOrigin,
    pub status: DocumentStatus,
    pub category: DocumentCategory,
    #[serde(default)]
    pub source_document_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_snapshots: Vec<SourceDocumentSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_mission_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_space_type: Option<DocumentSourceSpaceType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_space_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_space_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_channel_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_thread_root_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_channel_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_workbench_group: Option<AiWorkbenchGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage_description: Option<String>,
    #[serde(default)]
    pub is_public: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Document> for DocumentSummary {
    fn from(doc: Document) -> Self {
        Self {
            id: doc.id.map(|id| id.to_hex()).unwrap_or_default(),
            name: doc.name,
            display_name: doc.display_name,
            description: doc.description,
            mime_type: doc.mime_type,
            file_size: doc.file_size,
            folder_path: doc.folder_path,
            tags: doc.tags,
            uploaded_by: doc.uploaded_by,
            origin: doc.origin,
            status: doc.status,
            category: doc.category,
            source_document_ids: doc.source_document_ids,
            source_snapshots: doc.source_snapshots,
            source_session_id: doc.source_session_id,
            source_mission_id: doc.source_mission_id,
            created_by_agent_id: doc.created_by_agent_id,
            source_space_type: doc.source_space_type,
            source_space_id: doc.source_space_id,
            source_space_name: doc.source_space_name,
            source_channel_id: doc.source_channel_id,
            source_channel_name: doc.source_channel_name,
            source_thread_root_id: doc.source_thread_root_id,
            source_channel_run_id: doc.source_channel_run_id,
            ai_workbench_group: doc.ai_workbench_group,
            supersedes_id: doc.supersedes_id,
            lineage_description: doc.lineage_description,
            is_public: doc.is_public,
            created_at: doc.created_at,
            updated_at: doc.updated_at,
        }
    }
}

/// Upload document request
#[derive(Debug, Clone, Deserialize)]
pub struct UploadDocumentRequest {
    pub folder_path: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
}
