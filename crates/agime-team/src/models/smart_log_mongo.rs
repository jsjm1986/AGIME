//! Smart Log model for MongoDB

use chrono::{DateTime, Utc};
use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

use super::common_mongo::bson_datetime_option;

/// Smart log entry document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartLogEntry {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub team_id: ObjectId,
    pub user_id: String,
    pub user_name: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub resource_name: Option<String>,
    pub ai_summary: Option<String>,
    #[serde(default = "default_pending")]
    pub ai_summary_status: String,
    pub content_snapshot: Option<String>,
    #[serde(default = "default_human")]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_analysis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_analysis_status: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub ai_completed_at: Option<DateTime<Utc>>,
}

fn default_pending() -> String {
    "pending".to_string()
}

fn default_human() -> String {
    "human".to_string()
}

/// Smart log summary for API responses (excludes content_snapshot)
#[derive(Debug, Clone, Serialize)]
pub struct SmartLogSummary {
    pub id: String,
    #[serde(rename = "teamId")]
    pub team_id: String,
    #[serde(rename = "userId")]
    pub user_id: String,
    #[serde(rename = "userName")]
    pub user_name: Option<String>,
    pub action: String,
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(rename = "resourceId")]
    pub resource_id: Option<String>,
    #[serde(rename = "resourceName")]
    pub resource_name: Option<String>,
    #[serde(rename = "aiSummary")]
    pub ai_summary: Option<String>,
    #[serde(rename = "aiSummaryStatus")]
    pub ai_summary_status: String,
    pub source: String,
    #[serde(rename = "aiAnalysis", skip_serializing_if = "Option::is_none")]
    pub ai_analysis: Option<String>,
    #[serde(rename = "aiAnalysisStatus", skip_serializing_if = "Option::is_none")]
    pub ai_analysis_status: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "aiCompletedAt")]
    pub ai_completed_at: Option<String>,
}

impl From<SmartLogEntry> for SmartLogSummary {
    fn from(e: SmartLogEntry) -> Self {
        Self {
            id: e.id.map(|id| id.to_hex()).unwrap_or_default(),
            team_id: e.team_id.to_hex(),
            user_id: e.user_id,
            user_name: e.user_name,
            action: e.action,
            resource_type: e.resource_type,
            resource_id: e.resource_id,
            resource_name: e.resource_name,
            ai_summary: e.ai_summary,
            ai_summary_status: e.ai_summary_status,
            source: e.source,
            ai_analysis: e.ai_analysis,
            ai_analysis_status: e.ai_analysis_status,
            created_at: e.created_at.to_rfc3339(),
            ai_completed_at: e.ai_completed_at.map(|dt| dt.to_rfc3339()),
        }
    }
}

/// Context for triggering a smart log entry
#[derive(Debug, Clone)]
pub struct SmartLogContext {
    pub team_id: String,
    pub user_id: String,
    pub user_name: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub resource_name: String,
    pub content_for_ai: Option<String>,
    pub source: Option<String>,
    pub has_pending_analysis: bool,
    pub lang: Option<String>,
}

impl SmartLogContext {
    /// Create a context for a human-initiated action (the common case in route handlers).
    /// Defaults `user_name` and `source` to `None`, `has_pending_analysis` to `false`.
    pub fn new(
        team_id: String,
        user_id: String,
        action: &str,
        resource_type: &str,
        resource_id: String,
        resource_name: String,
        content_for_ai: Option<String>,
    ) -> Self {
        Self {
            team_id,
            user_id,
            user_name: None,
            action: action.into(),
            resource_type: resource_type.into(),
            resource_id,
            resource_name,
            content_for_ai,
            source: None,
            has_pending_analysis: false,
            lang: None,
        }
    }

    /// Build a fallback SmartLogContext for AI-generated entries (e.g. document analysis).
    /// Used when attach_analysis() finds no existing entry to update.
    pub fn ai_fallback(team_id: String, resource_id: String, resource_name: String) -> Self {
        Self {
            team_id,
            user_id: "system".to_string(),
            user_name: Some("AI 助手".to_string()),
            action: "analyze".to_string(),
            resource_type: "document".to_string(),
            resource_id,
            resource_name,
            content_for_ai: None,
            source: Some("ai".to_string()),
            has_pending_analysis: false,
            lang: None,
        }
    }

    /// Mark this context as having a pending document analysis.
    pub fn with_pending_analysis(mut self, pending: bool) -> Self {
        self.has_pending_analysis = pending;
        self
    }
}

/// Translate an action key to a Chinese verb label.
pub fn action_verb(action: &str) -> &str {
    match action {
        "create" => "创建",
        "update" => "更新",
        "delete" => "删除",
        "upload" => "上传",
        "accept" => "接受",
        "analyze" => "解读",
        _ => action,
    }
}

/// Translate a resource_type key to a Chinese label.
pub fn resource_type_label(rt: &str) -> &str {
    match rt {
        "document" => "文档",
        "skill" => "技能",
        "extension" => "扩展",
        "recipe" => "预设",
        _ => rt,
    }
}

/// Build a simple fallback summary string from action, resource_type, and resource_name.
pub fn build_fallback_summary(action: &str, resource_type: &str, resource_name: &str) -> String {
    format!(
        "{}了{}「{}」",
        action_verb(action),
        resource_type_label(resource_type),
        resource_name
    )
}

/// Trait for triggering smart log entries across crate boundaries
pub trait SmartLogTrigger: Send + Sync {
    fn trigger(&self, ctx: SmartLogContext);
}

/// Context for triggering automatic document analysis after upload
#[derive(Debug, Clone)]
pub struct DocumentAnalysisContext {
    pub team_id: String,
    pub doc_id: String,
    pub doc_name: String,
    pub mime_type: String,
    pub file_size: i64,
    pub user_id: String,
    pub lang: Option<String>,
    pub extra_instructions: Option<String>,
}

/// Trait for triggering automatic document analysis across crate boundaries.
/// Called after document upload to have an agent read and interpret the document.
pub trait DocumentAnalysisTrigger: Send + Sync {
    fn trigger(&self, ctx: DocumentAnalysisContext);
}
