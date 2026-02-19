//! MongoDB session document types for agent conversations
//!
//! Sessions persist multi-turn conversation state, enabling:
//! - Conversation history across multiple task submissions
//! - Token counting and context compaction tracking
//! - Session lifecycle management (active/archived)

use agime::agents::types::RetryConfig;
use serde::{Deserialize, Serialize};

/// MongoDB document for agent conversation sessions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<bson::oid::ObjectId>,
    pub session_id: String,
    pub team_id: String,
    pub agent_id: String,
    pub user_id: String,
    pub name: Option<String>,
    /// "active" or "archived"
    pub status: String,
    /// Serialized Vec<Message> as JSON string
    #[serde(default)]
    pub messages_json: String,
    pub message_count: i32,
    pub total_tokens: Option<i32>,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    #[serde(default)]
    pub compaction_count: i32,
    /// "legacy_segmented" or "cfpm_memory_v1"
    pub compaction_strategy: Option<String>,
    /// Extensions disabled by user during this session (persisted across messages)
    #[serde(default)]
    pub disabled_extensions: Vec<String>,
    /// Extensions enabled by user during this session (persisted across messages)
    #[serde(default)]
    pub enabled_extensions: Vec<String>,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,

    // === Chat Track fields (Phase 1) ===
    /// Session title (auto-generated from first message)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Whether this session is pinned to the top
    #[serde(default)]
    pub pinned: bool,
    /// Preview of the last message (≤200 chars, for list display)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_preview: Option<String>,
    /// Timestamp of the last message (for sorting)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<bson::DateTime>,
    /// Whether the session is currently processing a message (prevents concurrent sends)
    #[serde(default)]
    pub is_processing: bool,

    // === Phase 2: Document attachment ===
    /// Document IDs attached to this session as context
    #[serde(default)]
    pub attached_document_ids: Vec<String>,

    // === Workspace isolation ===
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,

    /// Extra instructions injected into system prompt (e.g. portal project path)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_instructions: Option<String>,

    /// Optional runtime extension allowlist for this session.
    /// Uses runtime names (e.g. "developer", "todo", custom extension names).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_extensions: Option<Vec<String>>,

    /// Optional skill id allowlist for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_skill_ids: Option<Vec<String>>,

    /// Optional retry/success-check contract for completion gating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_config: Option<RetryConfig>,

    /// Optional max turns for this session. None means no turn cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<i32>,

    /// Optional tool timeout in seconds for this session. None means no timeout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_timeout_seconds: Option<u64>,

    /// Optional cap for portal auto-retry rounds. None means unlimited retries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_portal_retry_rounds: Option<u32>,

    /// Whether portal coding sessions must output a structured final report.
    #[serde(default)]
    pub require_final_report: bool,

    /// Whether this is a portal visitor restricted session.
    #[serde(default)]
    pub portal_restricted: bool,

    /// Optional portal id this session is bound to.
    /// Used to prevent cross-portal session reuse/leakage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal_id: Option<String>,

    /// Optional portal slug this session is bound to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal_slug: Option<String>,

    /// Optional raw visitor id for portal public sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visitor_id: Option<String>,
}

/// Request to create a new session
#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub team_id: String,
    pub agent_id: String,
    pub user_id: String,
    pub name: Option<String>,
    #[serde(default)]
    pub attached_document_ids: Vec<String>,
    /// Extra instructions injected into system prompt
    #[serde(default)]
    pub extra_instructions: Option<String>,
    /// Optional runtime extension allowlist
    #[serde(default)]
    pub allowed_extensions: Option<Vec<String>>,
    /// Optional skill id allowlist
    #[serde(default)]
    pub allowed_skill_ids: Option<Vec<String>>,
    /// Optional retry/success-check contract
    #[serde(default)]
    pub retry_config: Option<RetryConfig>,
    /// Optional max turns for this session. None means no turn cap.
    #[serde(default)]
    pub max_turns: Option<i32>,
    /// Optional tool timeout in seconds for this session. None means no timeout.
    #[serde(default)]
    pub tool_timeout_seconds: Option<u64>,
    /// Optional cap for portal auto-retry rounds. None means unlimited retries.
    #[serde(default)]
    pub max_portal_retry_rounds: Option<u32>,
    /// Whether a structured final report is required before completion
    #[serde(default)]
    pub require_final_report: bool,
    /// Whether this is a restricted portal visitor session
    #[serde(default)]
    pub portal_restricted: bool,
}

/// Query parameters for listing sessions
#[derive(Debug, Deserialize)]
pub struct SessionListQuery {
    pub team_id: String,
    pub agent_id: String,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_page() -> u32 {
    1
}

fn default_limit() -> u32 {
    20
}

// === Chat Track types (Phase 1) ===

/// Request to send a chat message (bypasses Task system)
#[derive(Debug, Deserialize)]
pub struct SendChatMessageRequest {
    pub content: String,
}

/// Lightweight session list item (without messages_json)
#[derive(Debug, Clone, Serialize)]
pub struct SessionListItem {
    pub session_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub title: Option<String>,
    pub last_message_preview: Option<String>,
    pub last_message_at: Option<String>,
    pub message_count: i32,
    pub status: String,
    pub pinned: bool,
    pub created_at: String,
}

/// Query parameters for user session list
#[derive(Debug, Deserialize)]
pub struct UserSessionListQuery {
    pub team_id: String,
    /// Injected from auth context (not from query string)
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

/// Request to rename a session
#[derive(Debug, Deserialize)]
pub struct RenameSessionRequest {
    pub title: String,
}

/// Request to update session pin status
#[derive(Debug, Deserialize)]
pub struct PinSessionRequest {
    pub pinned: bool,
}

/// Request for creating a chat session
#[derive(Debug, Deserialize)]
pub struct CreateChatSessionRequest {
    pub agent_id: String,
    #[serde(default)]
    pub attached_document_ids: Vec<String>,
    /// Extra instructions injected into system prompt for this session
    #[serde(default)]
    pub extra_instructions: Option<String>,
    /// Optional runtime extension allowlist for this session
    #[serde(default)]
    pub allowed_extensions: Option<Vec<String>>,
    /// Optional skill id allowlist for this session
    #[serde(default)]
    pub allowed_skill_ids: Option<Vec<String>>,
    /// Optional retry/success-check contract
    #[serde(default)]
    pub retry_config: Option<RetryConfig>,
    /// Optional max turns for this session. None means no turn cap.
    #[serde(default)]
    pub max_turns: Option<i32>,
    /// Optional tool timeout in seconds for this session. None means no timeout.
    #[serde(default)]
    pub tool_timeout_seconds: Option<u64>,
    /// Optional cap for portal auto-retry rounds. None means unlimited retries.
    #[serde(default)]
    pub max_portal_retry_rounds: Option<u32>,
    /// Whether a structured final report is required before completion
    #[serde(default)]
    pub require_final_report: bool,
    /// Whether this should run as a restricted portal-style session
    #[serde(default)]
    pub portal_restricted: bool,
}

/// Response after sending a message
#[derive(Debug, Serialize)]
pub struct SendMessageResponse {
    pub session_id: String,
    pub streaming: bool,
}
