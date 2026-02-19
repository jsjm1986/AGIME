//! Team document model - simplified for agent-based processing
//!
//! Design: Store raw files only, let Agent process them on demand

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Team document - stores any file type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamDocument {
    pub id: String,
    pub team_id: String,
    pub folder_id: Option<String>,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub mime_type: String,
    pub file_size: i64,
    pub file_path: String, // Always filesystem storage
    pub uploaded_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Document summary for list views
#[derive(Debug, Clone, Serialize)]
pub struct DocumentSummary {
    pub id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub mime_type: String,
    pub file_size: i64,
    pub folder_id: Option<String>,
    pub uploaded_by: String,
    pub created_at: DateTime<Utc>,
}

/// Search query for documents (by name/description only)
#[derive(Debug, Clone, Deserialize)]
pub struct DocumentSearchQuery {
    pub query: String,
    pub folder_id: Option<String>,
    pub page: Option<u32>,
    pub limit: Option<u32>,
}

/// Search result item
#[derive(Debug, Clone, Serialize)]
pub struct DocumentSearchResult {
    pub document: DocumentSummary,
    pub score: f64,
}

/// Response for agent to get file path
#[derive(Debug, Clone, Serialize)]
pub struct AgentFileResponse {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub file_path: String,
    pub file_size: i64,
}
