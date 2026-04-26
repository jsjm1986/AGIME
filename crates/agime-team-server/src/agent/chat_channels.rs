use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use agime_team::models::mongo::Document;
use agime_team::services::mongo::{DocumentService, FolderService};
use agime_team::MongoDb;
use anyhow::Result;
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc, oid::ObjectId, Regex};
use mongodb::options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::channel_workspace_governance::ChatChannelWorkspaceGovernanceSummary;
use super::delegation_runtime::DelegationRuntimeResponse;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelVisibility {
    #[default]
    TeamPublic,
    TeamPrivate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelStatus {
    #[default]
    Active,
    Archived,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelDeleteMode {
    FullDelete,
    PreserveDocuments,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelMemberRole {
    #[default]
    Owner,
    Manager,
    Member,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelAuthorType {
    #[default]
    User,
    Agent,
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelMessageSurface {
    Temporary,
    #[default]
    Issue,
    Activity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelThreadState {
    #[default]
    Active,
    Archived,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelInteractionMode {
    #[default]
    Conversation,
    Execution,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelDisplayKind {
    Discussion,
    Onboarding,
    Suggestion,
    Result,
    Collaboration,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelDisplayStatus {
    Proposed,
    Active,
    AwaitingConfirmation,
    Adopted,
    Rejected,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelSourceKind {
    Human,
    Agent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelType {
    #[default]
    General,
    Coding,
    ScheduledTask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub channel_id: String,
    pub team_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub visibility: ChatChannelVisibility,
    #[serde(default)]
    pub channel_type: ChatChannelType,
    pub default_agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_folder_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_checkout_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_default_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_storage_mode: Option<ChatChannelRepoStorageMode>,
    pub created_by_user_id: String,
    #[serde(default)]
    pub status: ChatChannelStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<bson::DateTime>,
    #[serde(default)]
    pub message_count: i32,
    #[serde(default)]
    pub thread_count: i32,
    #[serde(default)]
    pub is_processing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_finished_at: Option<bson::DateTime>,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelMemberDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub channel_id: String,
    pub team_id: String,
    pub user_id: String,
    #[serde(default)]
    pub role: ChatChannelMemberRole,
    pub joined_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelMessageDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub message_id: String,
    pub channel_id: String,
    pub team_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
    #[serde(default)]
    pub author_type: ChatChannelAuthorType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_agent_id: Option<String>,
    #[serde(default)]
    pub author_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub surface: ChatChannelMessageSurface,
    #[serde(default)]
    pub thread_state: ChatChannelThreadState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collaboration_status: Option<ChatChannelDisplayStatus>,
    pub content_text: String,
    #[serde(default)]
    pub content_blocks: serde_json::Value,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub visible: bool,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ChatWorkspaceFileBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub path: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(default)]
    pub preview_supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelEventDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_message_id: Option<String>,
    pub event_id: i64,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelReadDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub channel_id: String,
    pub team_id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_read_message_id: Option<String>,
    pub last_read_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelUserPrefDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub channel_id: String,
    pub team_id: String,
    pub user_id: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub muted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_visited_at: Option<bson::DateTime>,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelAgentAutonomyMode {
    #[default]
    Standard,
    Proactive,
    AgentLead,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatChannelCardEmissionRegistry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding_revision: Option<String>,
    #[serde(default)]
    pub suggestion_revisions: BTreeMap<String, String>,
    #[serde(default)]
    pub progress_revisions: BTreeMap<String, String>,
    #[serde(default)]
    pub result_revisions: BTreeMap<String, String>,
    #[serde(default)]
    pub reminder_revisions: BTreeMap<String, String>,
}

impl ChatChannelCardEmissionRegistry {
    pub fn normalized_key(value: &str) -> String {
        sanitize_text(value, 160)
            .replace('.', "_")
            .replace('$', "_")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatChannelCardEmissionFamily {
    Onboarding,
    Suggestion,
    Progress,
    Result,
    Reminder,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelRepoStorageMode {
    #[default]
    BareRepoWorktree,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelOrchestratorStateDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub channel_id: String,
    pub team_id: String,
    #[serde(default)]
    pub agent_autonomy_mode: ChatChannelAgentAutonomyMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_outputs: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collaboration_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_focus: Option<String>,
    #[serde(default)]
    pub active_collaboration_summaries: Vec<String>,
    #[serde(default)]
    pub recent_suggestion_fingerprints: Vec<String>,
    #[serde(default)]
    pub ignored_suggestion_fingerprints: Vec<String>,
    #[serde(default)]
    pub card_emission_registry: ChatChannelCardEmissionRegistry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_summary_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_result_sync_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_discussion_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_document_change_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_ai_output_change_at: Option<bson::DateTime>,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelSummary {
    pub channel_id: String,
    pub team_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub visibility: ChatChannelVisibility,
    pub channel_type: ChatChannelType,
    pub default_agent_id: String,
    #[serde(default)]
    pub default_agent_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_folder_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_checkout_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_default_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_storage_mode: Option<ChatChannelRepoStorageMode>,
    pub created_by_user_id: String,
    pub status: ChatChannelStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<String>,
    pub message_count: i32,
    pub thread_count: i32,
    pub unread_count: i32,
    pub member_count: i32,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub muted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_visited_at: Option<String>,
    pub is_processing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_finished_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelDetail {
    #[serde(flatten)]
    pub summary: ChatChannelSummary,
    pub current_user_role: ChatChannelMemberRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestrator_state: Option<ChatChannelOrchestratorStateResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_governance: Option<ChatChannelWorkspaceGovernanceSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelThreadRuntimeResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_worktree_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_repo_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_execution_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_execution_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_execution_finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelOrchestratorStateResponse {
    pub channel_id: String,
    pub team_id: String,
    pub agent_autonomy_mode: ChatChannelAgentAutonomyMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_outputs: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collaboration_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_focus: Option<String>,
    #[serde(default)]
    pub active_collaboration_summaries: Vec<String>,
    #[serde(default)]
    pub ignored_suggestion_fingerprints: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_summary_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_result_sync_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_discussion_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_document_change_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_ai_output_change_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelUserPrefResponse {
    pub channel_id: String,
    pub team_id: String,
    pub user_id: String,
    pub pinned: bool,
    pub muted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_visited_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelMemberResponse {
    pub channel_id: String,
    pub team_id: String,
    pub user_id: String,
    pub role: ChatChannelMemberRole,
    pub joined_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelMessageResponse {
    pub message_id: String,
    pub channel_id: String,
    pub team_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
    pub author_type: ChatChannelAuthorType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_agent_id: Option<String>,
    pub author_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub surface: ChatChannelMessageSurface,
    #[serde(default)]
    pub thread_state: ChatChannelThreadState,
    pub display_kind: ChatChannelDisplayKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_status: Option<ChatChannelDisplayStatus>,
    pub source_kind: ChatChannelSourceKind,
    #[serde(default)]
    pub has_ai_participation: bool,
    #[serde(default)]
    pub summary_text: String,
    #[serde(default)]
    pub recent_agent_names: Vec<String>,
    pub content_text: String,
    #[serde(default)]
    pub content_blocks: serde_json::Value,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub visible: bool,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub reply_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelThreadResponse {
    pub root_message: ChatChannelMessageResponse,
    pub messages: Vec<ChatChannelMessageResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_runtime: Option<ChatChannelThreadRuntimeResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_diagnostics: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_runtime: Option<DelegationRuntimeResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelReadResponse {
    pub channel_id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_read_message_id: Option<String>,
    pub last_read_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct ChatChannelListFilters {
    pub surface: Option<ChatChannelMessageSurface>,
    pub thread_state: Option<ChatChannelThreadState>,
    pub display_kind: Option<ChatChannelDisplayKind>,
    pub display_status: Option<ChatChannelDisplayStatus>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateChatChannelRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub visibility: Option<ChatChannelVisibility>,
    #[serde(default)]
    pub channel_type: Option<ChatChannelType>,
    pub default_agent_id: String,
    #[serde(default)]
    pub member_user_ids: Vec<String>,
    #[serde(default)]
    pub workspace_display_name: Option<String>,
    #[serde(default)]
    pub repo_default_branch: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateChatChannelRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<Option<String>>,
    #[serde(default)]
    pub visibility: Option<ChatChannelVisibility>,
    #[serde(default)]
    pub channel_type: Option<ChatChannelType>,
    #[serde(default)]
    pub default_agent_id: Option<String>,
    #[serde(default)]
    pub agent_autonomy_mode: Option<ChatChannelAgentAutonomyMode>,
    #[serde(default)]
    pub channel_goal: Option<Option<String>>,
    #[serde(default)]
    pub participant_notes: Option<Option<String>>,
    #[serde(default)]
    pub expected_outputs: Option<Option<String>>,
    #[serde(default)]
    pub collaboration_style: Option<Option<String>>,
    #[serde(default)]
    pub workspace_display_name: Option<String>,
    #[serde(default)]
    pub repo_default_branch: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddChatChannelMemberRequest {
    pub user_id: String,
    #[serde(default)]
    pub role: Option<ChatChannelMemberRole>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateChatChannelMemberRequest {
    pub role: ChatChannelMemberRole,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChannelMention {
    pub mention_type: String,
    pub target_id: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendChatChannelMessageRequest {
    pub content: String,
    #[serde(default)]
    pub surface: Option<ChatChannelMessageSurface>,
    #[serde(default)]
    pub interaction_mode: Option<ChatChannelInteractionMode>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub thread_root_id: Option<String>,
    #[serde(default)]
    pub parent_message_id: Option<String>,
    #[serde(default)]
    pub attached_document_ids: Vec<String>,
    #[serde(default)]
    pub mentions: Vec<ChannelMention>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarkChatChannelReadRequest {
    #[serde(default)]
    pub last_read_message_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateChatChannelPrefsRequest {
    #[serde(default)]
    pub pinned: Option<bool>,
    #[serde(default)]
    pub muted: Option<bool>,
}

fn sanitize_text(value: &str, max_len: usize) -> String {
    value.trim().chars().take(max_len).collect()
}

fn sanitized_opt_text(value: Option<String>, max_len: usize) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = sanitize_text(&raw, max_len);
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn sanitize_folder_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.trim().chars() {
        match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => out.push('-'),
            _ => out.push(ch),
        }
    }
    let collapsed = out
        .split_whitespace()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    collapsed
        .trim_matches('.')
        .trim()
        .chars()
        .take(80)
        .collect()
}

fn bson_now() -> bson::DateTime {
    bson::DateTime::from_chrono(Utc::now())
}

fn to_rfc3339(value: bson::DateTime) -> String {
    value.to_chrono().to_rfc3339()
}

pub struct ChatChannelService {
    db: Arc<MongoDb>,
}

impl ChatChannelService {
    pub fn new(db: Arc<MongoDb>) -> Self {
        Self { db }
    }

    fn channels(&self) -> mongodb::Collection<ChatChannelDoc> {
        self.db.collection("chat_channels")
    }

    fn members(&self) -> mongodb::Collection<ChatChannelMemberDoc> {
        self.db.collection("chat_channel_members")
    }

    fn messages(&self) -> mongodb::Collection<ChatChannelMessageDoc> {
        self.db.collection("chat_channel_messages")
    }

    fn events(&self) -> mongodb::Collection<ChatChannelEventDoc> {
        self.db.collection("chat_channel_events")
    }

    fn reads(&self) -> mongodb::Collection<ChatChannelReadDoc> {
        self.db.collection("chat_channel_reads")
    }

    fn prefs(&self) -> mongodb::Collection<ChatChannelUserPrefDoc> {
        self.db.collection("chat_channel_user_prefs")
    }

    fn orchestrator_states(&self) -> mongodb::Collection<ChatChannelOrchestratorStateDoc> {
        self.db.collection("chat_channel_orchestrator_states")
    }

    fn default_card_emission_registry_bson() -> bson::Bson {
        bson::to_bson(&ChatChannelCardEmissionRegistry::default())
            .unwrap_or_else(|_| bson::Bson::Document(doc! {}))
    }

    fn default_orchestrator_state_doc(
        channel: &ChatChannelDoc,
        now: bson::DateTime,
    ) -> ChatChannelOrchestratorStateDoc {
        ChatChannelOrchestratorStateDoc {
            id: None,
            channel_id: channel.channel_id.clone(),
            team_id: channel.team_id.clone(),
            agent_autonomy_mode: ChatChannelAgentAutonomyMode::Standard,
            channel_goal: None,
            participant_notes: None,
            expected_outputs: None,
            collaboration_style: None,
            current_focus: None,
            active_collaboration_summaries: Vec::new(),
            recent_suggestion_fingerprints: Vec::new(),
            ignored_suggestion_fingerprints: Vec::new(),
            card_emission_registry: ChatChannelCardEmissionRegistry::default(),
            last_heartbeat_at: None,
            last_heartbeat_reason: None,
            last_summary_at: None,
            last_result_sync_at: None,
            last_seen_discussion_message_id: None,
            last_seen_document_change_at: None,
            last_seen_ai_output_change_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn orchestrator_set_on_insert(channel: &ChatChannelDoc, now: bson::DateTime) -> bson::Document {
        doc! {
            "channel_id": &channel.channel_id,
            "team_id": &channel.team_id,
            "agent_autonomy_mode": "standard",
            "active_collaboration_summaries": bson::Bson::Array(Vec::new()),
            "recent_suggestion_fingerprints": bson::Bson::Array(Vec::new()),
            "ignored_suggestion_fingerprints": bson::Bson::Array(Vec::new()),
            "card_emission_registry": Self::default_card_emission_registry_bson(),
            "created_at": now,
        }
    }

    fn apply_card_revision(
        state: &mut ChatChannelOrchestratorStateDoc,
        family: ChatChannelCardEmissionFamily,
        key: &str,
        revision: String,
    ) {
        let normalized_key = ChatChannelCardEmissionRegistry::normalized_key(key);
        match family {
            ChatChannelCardEmissionFamily::Onboarding => {
                state.card_emission_registry.onboarding_revision = Some(revision);
            }
            ChatChannelCardEmissionFamily::Suggestion => {
                state
                    .card_emission_registry
                    .suggestion_revisions
                    .insert(normalized_key, revision);
            }
            ChatChannelCardEmissionFamily::Progress => {
                state
                    .card_emission_registry
                    .progress_revisions
                    .insert(normalized_key, revision);
            }
            ChatChannelCardEmissionFamily::Result => {
                state
                    .card_emission_registry
                    .result_revisions
                    .insert(normalized_key, revision);
            }
            ChatChannelCardEmissionFamily::Reminder => {
                state
                    .card_emission_registry
                    .reminder_revisions
                    .insert(normalized_key, revision);
            }
        }
    }

    fn to_orchestrator_response(
        &self,
        state: ChatChannelOrchestratorStateDoc,
    ) -> ChatChannelOrchestratorStateResponse {
        ChatChannelOrchestratorStateResponse {
            channel_id: state.channel_id,
            team_id: state.team_id,
            agent_autonomy_mode: state.agent_autonomy_mode,
            channel_goal: state.channel_goal,
            participant_notes: state.participant_notes,
            expected_outputs: state.expected_outputs,
            collaboration_style: state.collaboration_style,
            current_focus: state.current_focus,
            active_collaboration_summaries: state.active_collaboration_summaries,
            ignored_suggestion_fingerprints: state.ignored_suggestion_fingerprints,
            last_heartbeat_at: state.last_heartbeat_at.map(to_rfc3339),
            last_heartbeat_reason: state.last_heartbeat_reason,
            last_summary_at: state.last_summary_at.map(to_rfc3339),
            last_result_sync_at: state.last_result_sync_at.map(to_rfc3339),
            last_seen_discussion_message_id: state.last_seen_discussion_message_id,
            last_seen_document_change_at: state.last_seen_document_change_at.map(to_rfc3339),
            last_seen_ai_output_change_at: state.last_seen_ai_output_change_at.map(to_rfc3339),
            created_at: to_rfc3339(state.created_at),
            updated_at: to_rfc3339(state.updated_at),
        }
    }

    fn discussion_card_kind(metadata: &serde_json::Value) -> Option<&str> {
        metadata
            .as_object()
            .and_then(|obj| obj.get("discussion_card_kind"))
            .and_then(|value| value.as_str())
    }

    fn display_kind_for(
        surface: ChatChannelMessageSurface,
        author_type: ChatChannelAuthorType,
        metadata: &serde_json::Value,
    ) -> ChatChannelDisplayKind {
        match surface {
            ChatChannelMessageSurface::Activity => match Self::discussion_card_kind(metadata) {
                Some("onboarding") => ChatChannelDisplayKind::Onboarding,
                Some("suggestion") => ChatChannelDisplayKind::Suggestion,
                Some("result") => ChatChannelDisplayKind::Result,
                _ => match author_type {
                    ChatChannelAuthorType::Agent | ChatChannelAuthorType::System => {
                        ChatChannelDisplayKind::Suggestion
                    }
                    ChatChannelAuthorType::User => ChatChannelDisplayKind::Discussion,
                },
            },
            ChatChannelMessageSurface::Temporary | ChatChannelMessageSurface::Issue => {
                ChatChannelDisplayKind::Collaboration
            }
        }
    }

    fn display_status_for(
        surface: ChatChannelMessageSurface,
        thread_state: ChatChannelThreadState,
        collaboration_status: Option<ChatChannelDisplayStatus>,
    ) -> Option<ChatChannelDisplayStatus> {
        if matches!(surface, ChatChannelMessageSurface::Activity) {
            return collaboration_status;
        }
        if let Some(status) = collaboration_status {
            return Some(status);
        }
        Some(match thread_state {
            ChatChannelThreadState::Archived => ChatChannelDisplayStatus::Rejected,
            ChatChannelThreadState::Active => ChatChannelDisplayStatus::Active,
        })
    }

    fn source_kind_for(author_type: ChatChannelAuthorType) -> ChatChannelSourceKind {
        match author_type {
            ChatChannelAuthorType::Agent | ChatChannelAuthorType::System => {
                ChatChannelSourceKind::Agent
            }
            ChatChannelAuthorType::User => ChatChannelSourceKind::Human,
        }
    }

    async fn ensure_channel_document_folder(
        &self,
        team_id: &str,
        user_id: &str,
        channel_id: &str,
        channel_name: &str,
    ) -> Result<String, mongodb::error::Error> {
        let folder_service = FolderService::new((*self.db).clone());
        folder_service
            .ensure_system_folder(team_id, "团队频道", "/")
            .await
            .map_err(|err| mongodb::error::Error::custom(err.to_string()))?;

        let base_name = sanitize_folder_segment(channel_name);
        let folder_name = if base_name.is_empty() {
            format!("频道-{}", &channel_id[..8])
        } else {
            base_name
        };
        let primary_path = format!("/团队频道/{}", folder_name);
        let existing = folder_service
            .list(team_id, Some("/团队频道"))
            .await
            .map_err(|err| mongodb::error::Error::custom(err.to_string()))?;
        if existing.iter().any(|item| item.full_path == primary_path) {
            let fallback_name = format!("{}-{}", folder_name, &channel_id[..8]);
            let fallback_path = format!("/团队频道/{}", fallback_name);
            if !existing.iter().any(|item| item.full_path == fallback_path) {
                folder_service
                    .create(team_id, user_id, &fallback_name, "/团队频道")
                    .await
                    .map_err(|err| mongodb::error::Error::custom(err.to_string()))?;
                return Ok(fallback_path);
            }
            return Ok(fallback_path);
        }

        folder_service
            .create(team_id, user_id, &folder_name, "/团队频道")
            .await
            .map_err(|err| mongodb::error::Error::custom(err.to_string()))?;
        Ok(primary_path)
    }

    pub async fn ensure_indexes(&self) -> Result<(), mongodb::error::Error> {
        use mongodb::{options::IndexOptions, IndexModel};

        self.channels()
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "status": 1, "last_message_at": -1, "updated_at": -1 })
                        .build(),
                ],
                None,
            )
            .await?;
        self.members()
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1, "user_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "user_id": 1 })
                        .build(),
                ],
                None,
            )
            .await?;
        self.messages()
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "message_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1, "thread_root_id": 1, "created_at": 1 })
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1, "parent_message_id": 1 })
                        .build(),
                ],
                None,
            )
            .await?;
        self.events()
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1, "event_id": 1 })
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1, "run_id": 1, "event_id": 1 })
                        .build(),
                ],
                None,
            )
            .await?;
        self.reads()
            .create_indexes(
                vec![IndexModel::builder()
                    .keys(doc! { "channel_id": 1, "user_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build()],
                None,
            )
            .await?;
        self.prefs()
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1, "user_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "user_id": 1, "pinned": -1, "updated_at": -1 })
                        .build(),
                ],
                None,
            )
            .await?;
        self.orchestrator_states()
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "agent_autonomy_mode": 1, "updated_at": -1 })
                        .build(),
                ],
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn get_channel(
        &self,
        channel_id: &str,
    ) -> Result<Option<ChatChannelDoc>, mongodb::error::Error> {
        self.channels()
            .find_one(doc! { "channel_id": channel_id }, None)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn set_channel_project_binding(
        &self,
        channel_id: &str,
        workspace_id: &str,
        workspace_path: &str,
        workspace_kind: &str,
        workspace_display_name: &str,
        repo_path: &str,
        main_checkout_path: &str,
        repo_root: &str,
        repo_default_branch: &str,
        repo_storage_mode: ChatChannelRepoStorageMode,
    ) -> Result<Option<ChatChannelDoc>, mongodb::error::Error> {
        self.channels()
            .find_one_and_update(
                doc! { "channel_id": channel_id },
                doc! {
                    "$set": {
                        "workspace_id": workspace_id,
                        "workspace_path": workspace_path,
                        "workspace_kind": workspace_kind,
                        "workspace_display_name": workspace_display_name,
                        "repo_path": repo_path,
                        "main_checkout_path": main_checkout_path,
                        "repo_root": repo_root,
                        "repo_default_branch": repo_default_branch,
                        "repo_storage_mode": bson::to_bson(&repo_storage_mode).unwrap_or(bson::Bson::String("bare_repo_worktree".to_string())),
                        "updated_at": bson_now(),
                    }
                },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await
    }

    pub async fn clear_channel_project_binding(
        &self,
        channel_id: &str,
    ) -> Result<Option<ChatChannelDoc>, mongodb::error::Error> {
        self.channels()
            .find_one_and_update(
                doc! { "channel_id": channel_id },
                doc! {
                    "$set": {
                        "workspace_id": bson::Bson::Null,
                        "workspace_path": bson::Bson::Null,
                        "workspace_kind": bson::Bson::Null,
                        "repo_path": bson::Bson::Null,
                        "main_checkout_path": bson::Bson::Null,
                        "repo_root": bson::Bson::Null,
                        "repo_storage_mode": bson::Bson::Null,
                        "updated_at": bson_now(),
                    }
                },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await
    }

    pub async fn update_channel_type(
        &self,
        channel_id: &str,
        channel_type: ChatChannelType,
    ) -> Result<Option<ChatChannelDoc>, mongodb::error::Error> {
        self.channels()
            .find_one_and_update(
                doc! { "channel_id": channel_id },
                doc! {
                    "$set": {
                        "channel_type": bson::to_bson(&channel_type).unwrap_or(bson::Bson::String("general".to_string())),
                        "updated_at": bson_now(),
                    }
                },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await
    }

    pub async fn get_member_role(
        &self,
        channel_id: &str,
        user_id: &str,
    ) -> Result<Option<ChatChannelMemberRole>, mongodb::error::Error> {
        Ok(self
            .members()
            .find_one(doc! { "channel_id": channel_id, "user_id": user_id }, None)
            .await?
            .map(|doc| doc.role))
    }

    pub async fn create_channel(
        &self,
        team_id: &str,
        creator_user_id: &str,
        request: CreateChatChannelRequest,
    ) -> Result<ChatChannelDoc, mongodb::error::Error> {
        let now = bson_now();
        let channel_id = Uuid::new_v4().to_string();
        let document_folder_path = Some(
            self.ensure_channel_document_folder(
                team_id,
                creator_user_id,
                &channel_id,
                &request.name,
            )
            .await?,
        );
        let channel = ChatChannelDoc {
            id: None,
            channel_id,
            team_id: team_id.to_string(),
            name: sanitize_text(&request.name, 80),
            description: sanitized_opt_text(request.description, 280),
            visibility: request.visibility.unwrap_or_default(),
            channel_type: request.channel_type.unwrap_or_default(),
            default_agent_id: request.default_agent_id,
            document_folder_path,
            workspace_id: None,
            workspace_path: None,
            workspace_kind: None,
            workspace_display_name: sanitized_opt_text(request.workspace_display_name, 120),
            repo_path: None,
            main_checkout_path: None,
            repo_root: None,
            repo_default_branch: sanitized_opt_text(request.repo_default_branch, 120),
            repo_storage_mode: None,
            created_by_user_id: creator_user_id.to_string(),
            status: ChatChannelStatus::Active,
            last_message_preview: None,
            last_message_at: None,
            message_count: 0,
            thread_count: 0,
            is_processing: false,
            active_run_id: None,
            last_run_status: None,
            last_run_error: None,
            last_run_finished_at: None,
            created_at: now,
            updated_at: now,
        };
        self.channels().insert_one(&channel, None).await?;

        let mut seen = HashSet::from([creator_user_id.to_string()]);
        let mut members = vec![ChatChannelMemberDoc {
            id: None,
            channel_id: channel.channel_id.clone(),
            team_id: team_id.to_string(),
            user_id: creator_user_id.to_string(),
            role: ChatChannelMemberRole::Owner,
            joined_at: now,
        }];
        for user_id in request.member_user_ids {
            if seen.insert(user_id.clone()) {
                members.push(ChatChannelMemberDoc {
                    id: None,
                    channel_id: channel.channel_id.clone(),
                    team_id: team_id.to_string(),
                    user_id,
                    role: ChatChannelMemberRole::Member,
                    joined_at: now,
                });
            }
        }
        self.members().insert_many(members, None).await?;
        let _ = self
            .orchestrator_states()
            .insert_one(Self::default_orchestrator_state_doc(&channel, now), None)
            .await;
        Ok(channel)
    }

    pub async fn list_members(
        &self,
        channel_id: &str,
    ) -> Result<Vec<ChatChannelMemberDoc>, mongodb::error::Error> {
        self.members()
            .find(
                doc! { "channel_id": channel_id },
                FindOptions::builder().sort(doc! { "joined_at": 1 }).build(),
            )
            .await?
            .try_collect()
            .await
    }

    pub async fn add_member(
        &self,
        channel: &ChatChannelDoc,
        user_id: &str,
        role: ChatChannelMemberRole,
    ) -> Result<ChatChannelMemberDoc, mongodb::error::Error> {
        let doc = ChatChannelMemberDoc {
            id: None,
            channel_id: channel.channel_id.clone(),
            team_id: channel.team_id.clone(),
            user_id: user_id.to_string(),
            role,
            joined_at: bson_now(),
        };
        self.members()
            .replace_one(
                doc! { "channel_id": &channel.channel_id, "user_id": user_id },
                &doc,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(doc)
    }

    pub async fn update_member_role(
        &self,
        channel_id: &str,
        user_id: &str,
        role: ChatChannelMemberRole,
    ) -> Result<bool, mongodb::error::Error> {
        let result = self
            .members()
            .update_one(
                doc! { "channel_id": channel_id, "user_id": user_id },
                doc! { "$set": { "role": bson::to_bson(&role).unwrap_or(bson::Bson::String("member".into())) } },
                None,
            )
            .await?;
        Ok(result.matched_count > 0)
    }

    pub async fn remove_member(
        &self,
        channel_id: &str,
        user_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let result = self
            .members()
            .delete_one(doc! { "channel_id": channel_id, "user_id": user_id }, None)
            .await?;
        Ok(result.deleted_count > 0)
    }

    fn to_summary(
        &self,
        channel: ChatChannelDoc,
        agent_name_by_id: &HashMap<String, String>,
        unread_count: i32,
        member_count: i32,
        pref: Option<&ChatChannelUserPrefDoc>,
    ) -> ChatChannelSummary {
        ChatChannelSummary {
            channel_id: channel.channel_id.clone(),
            team_id: channel.team_id.clone(),
            name: channel.name.clone(),
            description: channel.description.clone(),
            visibility: channel.visibility,
            channel_type: channel.channel_type,
            default_agent_id: channel.default_agent_id.clone(),
            default_agent_name: agent_name_by_id
                .get(&channel.default_agent_id)
                .cloned()
                .unwrap_or_else(|| channel.default_agent_id.clone()),
            document_folder_path: channel.document_folder_path.clone(),
            workspace_id: channel.workspace_id.clone(),
            workspace_path: channel.workspace_path.clone(),
            workspace_kind: channel.workspace_kind.clone(),
            workspace_display_name: channel.workspace_display_name.clone(),
            repo_path: channel.repo_path.clone(),
            main_checkout_path: channel.main_checkout_path.clone(),
            repo_root: channel.repo_root.clone(),
            repo_default_branch: channel.repo_default_branch.clone(),
            repo_storage_mode: channel.repo_storage_mode,
            created_by_user_id: channel.created_by_user_id.clone(),
            status: channel.status,
            last_message_preview: channel.last_message_preview.clone(),
            last_message_at: channel.last_message_at.map(to_rfc3339),
            message_count: channel.message_count,
            thread_count: channel.thread_count,
            unread_count,
            member_count,
            pinned: pref.map(|item| item.pinned).unwrap_or(false),
            muted: pref.map(|item| item.muted).unwrap_or(false),
            last_visited_at: pref.and_then(|item| item.last_visited_at.map(to_rfc3339)),
            is_processing: channel.is_processing,
            active_run_id: channel.active_run_id.clone(),
            last_run_status: channel.last_run_status.clone(),
            last_run_error: channel.last_run_error.clone(),
            last_run_finished_at: channel.last_run_finished_at.map(to_rfc3339),
            created_at: to_rfc3339(channel.created_at),
            updated_at: to_rfc3339(channel.updated_at),
        }
    }

    pub async fn list_channels_for_user(
        &self,
        team_id: &str,
        user_id: &str,
        is_team_admin: bool,
        agent_name_by_id: &HashMap<String, String>,
    ) -> Result<Vec<ChatChannelSummary>, mongodb::error::Error> {
        let private_channel_ids = if is_team_admin {
            HashSet::new()
        } else {
            self.members()
                .find(doc! { "team_id": team_id, "user_id": user_id }, None)
                .await?
                .try_collect::<Vec<ChatChannelMemberDoc>>()
                .await?
                .into_iter()
                .map(|item| item.channel_id)
                .collect::<HashSet<_>>()
        };

        let mut filter = doc! {
            "team_id": team_id,
            "status": "active",
        };
        if !is_team_admin {
            filter.insert(
                "$or",
                vec![
                    doc! { "visibility": "team_public" },
                    doc! { "channel_id": { "$in": private_channel_ids.iter().cloned().collect::<Vec<_>>() } },
                ],
            );
        }

        let channels: Vec<ChatChannelDoc> = self
            .channels()
            .find(
                filter,
                FindOptions::builder()
                    .sort(doc! { "last_message_at": -1, "updated_at": -1 })
                    .build(),
            )
            .await?
            .try_collect()
            .await?;

        let mut unread_by_channel = HashMap::new();
        let mut member_counts = HashMap::new();
        let channel_ids = channels
            .iter()
            .map(|c| c.channel_id.clone())
            .collect::<Vec<_>>();
        let prefs_by_channel = self
            .list_prefs_for_user(team_id, user_id, &channel_ids)
            .await?;
        if !channel_ids.is_empty() {
            let members: Vec<ChatChannelMemberDoc> = self
                .members()
                .find(doc! { "channel_id": { "$in": channel_ids.clone() } }, None)
                .await?
                .try_collect()
                .await?;
            for member in members {
                *member_counts.entry(member.channel_id).or_insert(0_i32) += 1;
            }
            for channel in &channels {
                unread_by_channel.insert(
                    channel.channel_id.clone(),
                    self.unread_count(&channel.channel_id, user_id).await?,
                );
            }
        }

        let mut summaries = channels
            .into_iter()
            .map(|channel| {
                let channel_id = channel.channel_id.clone();
                self.to_summary(
                    channel,
                    agent_name_by_id,
                    unread_by_channel.get(&channel_id).copied().unwrap_or(0),
                    member_counts.get(&channel_id).copied().unwrap_or(0),
                    prefs_by_channel.get(&channel_id),
                )
            })
            .collect::<Vec<_>>();

        summaries.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| a.muted.cmp(&b.muted))
                .then_with(|| (b.unread_count > 0).cmp(&(a.unread_count > 0)))
                .then_with(|| b.last_message_at.cmp(&a.last_message_at))
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });

        Ok(summaries)
    }

    pub async fn build_detail(
        &self,
        channel: ChatChannelDoc,
        user_role: ChatChannelMemberRole,
        agent_name_by_id: &HashMap<String, String>,
        unread_count: i32,
        member_count: i32,
        pref: Option<&ChatChannelUserPrefDoc>,
    ) -> ChatChannelDetail {
        let orchestrator_state = self
            .get_orchestrator_state(&channel.channel_id)
            .await
            .ok()
            .flatten()
            .map(|state| self.to_orchestrator_response(state));
        ChatChannelDetail {
            summary: self.to_summary(channel, agent_name_by_id, unread_count, member_count, pref),
            current_user_role: user_role,
            orchestrator_state,
            workspace_governance: None,
        }
    }

    pub async fn list_active_channels(&self) -> Result<Vec<ChatChannelDoc>, mongodb::error::Error> {
        self.channels()
            .find(
                doc! { "status": "active" },
                FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await?
            .try_collect()
            .await
    }

    pub async fn list_recent_root_message_docs(
        &self,
        channel_id: &str,
        limit: i64,
    ) -> Result<Vec<ChatChannelMessageDoc>, mongodb::error::Error> {
        self.messages()
            .find(
                doc! {
                    "channel_id": channel_id,
                    "$or": [
                        { "thread_root_id": { "$exists": false } },
                        { "thread_root_id": bson::Bson::Null }
                    ]
                },
                FindOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .limit(limit)
                    .build(),
            )
            .await?
            .try_collect()
            .await
    }

    pub async fn get_orchestrator_state(
        &self,
        channel_id: &str,
    ) -> Result<Option<ChatChannelOrchestratorStateDoc>, mongodb::error::Error> {
        self.orchestrator_states()
            .find_one(doc! { "channel_id": channel_id }, None)
            .await
    }

    pub async fn update_orchestrator_mode(
        &self,
        channel: &ChatChannelDoc,
        mode: ChatChannelAgentAutonomyMode,
    ) -> Result<ChatChannelOrchestratorStateDoc, mongodb::error::Error> {
        let now = bson_now();
        let mut set_on_insert = Self::orchestrator_set_on_insert(channel, now);
        if !matches!(mode, ChatChannelAgentAutonomyMode::Standard) {
            set_on_insert.insert(
                "agent_autonomy_mode",
                bson::to_bson(&ChatChannelAgentAutonomyMode::Standard)
                    .unwrap_or(bson::Bson::String("standard".into())),
            );
        }
        self.orchestrator_states()
            .find_one_and_update(
                doc! {
                    "channel_id": &channel.channel_id,
                    "team_id": &channel.team_id,
                },
                doc! {
                    "$set": {
                        "agent_autonomy_mode": bson::to_bson(&mode).unwrap_or(bson::Bson::String("standard".into())),
                        "updated_at": now,
                    },
                    "$setOnInsert": set_on_insert,
                },
                FindOneAndUpdateOptions::builder()
                    .upsert(true)
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await?
            .ok_or_else(|| mongodb::error::Error::custom("failed to update orchestrator mode"))
    }

    pub async fn update_orchestrator_profile(
        &self,
        channel: &ChatChannelDoc,
        request: &UpdateChatChannelRequest,
    ) -> Result<ChatChannelOrchestratorStateDoc, mongodb::error::Error> {
        let now = bson_now();
        let mut set_doc = doc! {
            "updated_at": now,
        };
        let mut set_on_insert = Self::orchestrator_set_on_insert(channel, now);
        if let Some(mode) = request.agent_autonomy_mode {
            set_doc.insert(
                "agent_autonomy_mode",
                bson::to_bson(&mode).unwrap_or(bson::Bson::String("standard".into())),
            );
        } else {
            set_on_insert.insert("agent_autonomy_mode", "standard");
        }
        if let Some(value) = &request.channel_goal {
            set_doc.insert(
                "channel_goal",
                value
                    .clone()
                    .and_then(|item| sanitized_opt_text(Some(item), 400))
                    .map(bson::Bson::String)
                    .unwrap_or(bson::Bson::Null),
            );
        }
        if let Some(value) = &request.participant_notes {
            set_doc.insert(
                "participant_notes",
                value
                    .clone()
                    .and_then(|item| sanitized_opt_text(Some(item), 600))
                    .map(bson::Bson::String)
                    .unwrap_or(bson::Bson::Null),
            );
        }
        if let Some(value) = &request.expected_outputs {
            set_doc.insert(
                "expected_outputs",
                value
                    .clone()
                    .and_then(|item| sanitized_opt_text(Some(item), 600))
                    .map(bson::Bson::String)
                    .unwrap_or(bson::Bson::Null),
            );
        }
        if let Some(value) = &request.collaboration_style {
            set_doc.insert(
                "collaboration_style",
                value
                    .clone()
                    .and_then(|item| sanitized_opt_text(Some(item), 280))
                    .map(bson::Bson::String)
                    .unwrap_or(bson::Bson::Null),
            );
        }
        self.orchestrator_states()
            .update_one(
                doc! {
                    "channel_id": &channel.channel_id,
                    "team_id": &channel.team_id,
                },
                doc! {
                    "$set": set_doc,
                    "$setOnInsert": set_on_insert
                },
                mongodb::options::UpdateOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        self.orchestrator_states()
            .find_one(
                doc! {
                    "channel_id": &channel.channel_id,
                    "team_id": &channel.team_id,
                },
                None,
            )
            .await?
            .ok_or_else(|| mongodb::error::Error::custom("failed to update orchestrator profile"))
    }

    pub async fn record_orchestrator_heartbeat(
        &self,
        channel: &ChatChannelDoc,
        reason: &str,
        suggestion_fingerprint: Option<String>,
    ) -> Result<ChatChannelOrchestratorStateDoc, mongodb::error::Error> {
        let now = bson_now();
        let mut set_doc = doc! {
            "last_heartbeat_at": now,
            "last_heartbeat_reason": sanitize_text(reason, 200),
            "updated_at": now,
        };
        if reason.contains("result") {
            set_doc.insert("last_result_sync_at", now);
        }
        if reason.contains("summary") {
            set_doc.insert("last_summary_at", now);
        }
        let normalized_fingerprint = suggestion_fingerprint
            .map(|value| sanitize_text(&value, 120))
            .filter(|value| !value.is_empty());
        let mut set_on_insert = Self::orchestrator_set_on_insert(channel, now);
        if normalized_fingerprint.is_none() {
            set_on_insert.insert(
                "recent_suggestion_fingerprints",
                bson::Bson::Array(Vec::new()),
            );
        }
        let mut update = doc! {
            "$set": set_doc,
            "$setOnInsert": set_on_insert,
        };
        if let Some(fingerprint) = normalized_fingerprint {
            update.insert(
                "$addToSet",
                doc! { "recent_suggestion_fingerprints": fingerprint },
            );
        }
        self.orchestrator_states()
            .find_one_and_update(
                doc! {
                    "channel_id": &channel.channel_id,
                    "team_id": &channel.team_id,
                },
                update,
                FindOneAndUpdateOptions::builder()
                    .upsert(true)
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await?
            .ok_or_else(|| mongodb::error::Error::custom("failed to record orchestrator heartbeat"))
    }

    pub async fn record_orchestrator_card_emission(
        &self,
        channel: &ChatChannelDoc,
        reason: &str,
        family: ChatChannelCardEmissionFamily,
        key: &str,
        revision: &str,
        suggestion_fingerprint: Option<String>,
    ) -> Result<ChatChannelOrchestratorStateDoc, mongodb::error::Error> {
        let now = bson_now();
        let filter = doc! {
            "channel_id": &channel.channel_id,
            "team_id": &channel.team_id,
        };
        let normalized_fingerprint = suggestion_fingerprint
            .map(|value| sanitize_text(&value, 120))
            .filter(|value| !value.is_empty());
        let mut state = self
            .orchestrator_states()
            .find_one(filter.clone(), None)
            .await?
            .unwrap_or_else(|| Self::default_orchestrator_state_doc(channel, now));
        state.channel_id = channel.channel_id.clone();
        state.team_id = channel.team_id.clone();
        state.last_heartbeat_at = Some(now);
        state.last_heartbeat_reason = Some(sanitize_text(reason, 200));
        state.updated_at = now;
        Self::apply_card_revision(&mut state, family, key, sanitize_text(revision, 240));
        if matches!(family, ChatChannelCardEmissionFamily::Result) || reason.contains("result") {
            state.last_result_sync_at = Some(now);
        }
        if reason.contains("summary") {
            state.last_summary_at = Some(now);
        }
        if let Some(fingerprint) = normalized_fingerprint {
            if !state
                .recent_suggestion_fingerprints
                .iter()
                .any(|item| item == &fingerprint)
            {
                state.recent_suggestion_fingerprints.push(fingerprint);
            }
        }
        self.orchestrator_states()
            .replace_one(
                filter,
                &state,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        self.orchestrator_states()
            .find_one(
                doc! { "channel_id": &channel.channel_id, "team_id": &channel.team_id },
                None,
            )
            .await?
            .ok_or_else(|| {
                mongodb::error::Error::custom("failed to record orchestrator card emission")
            })
    }

    pub async fn note_orchestrator_discussion_activity(
        &self,
        channel: &ChatChannelDoc,
        message_id: &str,
    ) -> Result<ChatChannelOrchestratorStateDoc, mongodb::error::Error> {
        let now = bson_now();
        self.orchestrator_states()
            .find_one_and_update(
                doc! {
                    "channel_id": &channel.channel_id,
                    "team_id": &channel.team_id,
                },
                doc! {
                    "$set": {
                        "last_seen_discussion_message_id": sanitize_text(message_id, 120),
                        "updated_at": now,
                    },
                    "$setOnInsert": Self::orchestrator_set_on_insert(channel, now)
                },
                FindOneAndUpdateOptions::builder()
                    .upsert(true)
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await?
            .ok_or_else(|| mongodb::error::Error::custom("failed to note discussion activity"))
    }

    pub async fn ignore_orchestrator_suggestion_fingerprint(
        &self,
        channel: &ChatChannelDoc,
        fingerprint: &str,
    ) -> Result<ChatChannelOrchestratorStateDoc, mongodb::error::Error> {
        let normalized = sanitize_text(fingerprint, 120);
        let now = bson_now();
        self.orchestrator_states()
            .find_one_and_update(
                doc! {
                    "channel_id": &channel.channel_id,
                    "team_id": &channel.team_id,
                },
                doc! {
                    "$set": {
                        "updated_at": now,
                    },
                    "$addToSet": {
                        "ignored_suggestion_fingerprints": &normalized,
                    },
                    "$setOnInsert": Self::orchestrator_set_on_insert(channel, now)
                },
                FindOneAndUpdateOptions::builder()
                    .upsert(true)
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await?
            .ok_or_else(|| mongodb::error::Error::custom("failed to ignore suggestion fingerprint"))
    }

    pub async fn get_pref_for_user(
        &self,
        channel_id: &str,
        user_id: &str,
    ) -> Result<Option<ChatChannelUserPrefDoc>, mongodb::error::Error> {
        self.prefs()
            .find_one(doc! { "channel_id": channel_id, "user_id": user_id }, None)
            .await
    }

    async fn list_prefs_for_user(
        &self,
        team_id: &str,
        user_id: &str,
        channel_ids: &[String],
    ) -> Result<HashMap<String, ChatChannelUserPrefDoc>, mongodb::error::Error> {
        if channel_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let prefs: Vec<ChatChannelUserPrefDoc> = self
            .prefs()
            .find(
                doc! {
                    "team_id": team_id,
                    "user_id": user_id,
                    "channel_id": { "$in": channel_ids.to_vec() }
                },
                None,
            )
            .await?
            .try_collect()
            .await?;
        Ok(prefs
            .into_iter()
            .map(|pref| (pref.channel_id.clone(), pref))
            .collect())
    }

    pub async fn upsert_user_pref(
        &self,
        channel: &ChatChannelDoc,
        user_id: &str,
        request: UpdateChatChannelPrefsRequest,
    ) -> Result<ChatChannelUserPrefDoc, mongodb::error::Error> {
        let now = bson_now();
        let mut set_doc = doc! {
            "updated_at": now,
        };
        if let Some(value) = request.pinned {
            set_doc.insert("pinned", value);
        }
        if let Some(value) = request.muted {
            set_doc.insert("muted", value);
        }
        self.prefs()
            .find_one_and_update(
                doc! {
                    "channel_id": &channel.channel_id,
                    "team_id": &channel.team_id,
                    "user_id": user_id,
                },
                doc! {
                    "$set": set_doc,
                    "$setOnInsert": {
                        "channel_id": &channel.channel_id,
                        "team_id": &channel.team_id,
                        "user_id": user_id,
                        "created_at": now,
                    }
                },
                FindOneAndUpdateOptions::builder()
                    .upsert(true)
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await?
            .ok_or_else(|| mongodb::error::Error::custom("failed to update channel prefs"))
    }

    pub async fn touch_last_visited(
        &self,
        channel: &ChatChannelDoc,
        user_id: &str,
    ) -> Result<ChatChannelUserPrefDoc, mongodb::error::Error> {
        let now = bson_now();
        self.prefs()
            .find_one_and_update(
                doc! {
                    "channel_id": &channel.channel_id,
                    "team_id": &channel.team_id,
                    "user_id": user_id,
                },
                doc! {
                    "$set": {
                        "last_visited_at": now,
                        "updated_at": now,
                    },
                    "$setOnInsert": {
                        "channel_id": &channel.channel_id,
                        "team_id": &channel.team_id,
                        "user_id": user_id,
                        "pinned": false,
                        "muted": false,
                        "created_at": now,
                    }
                },
                FindOneAndUpdateOptions::builder()
                    .upsert(true)
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await?
            .ok_or_else(|| mongodb::error::Error::custom("failed to touch last visited"))
    }

    pub async fn update_channel(
        &self,
        channel_id: &str,
        request: UpdateChatChannelRequest,
    ) -> Result<Option<ChatChannelDoc>, mongodb::error::Error> {
        let mut set_doc = doc! {};
        if let Some(name) = request.name {
            let value = sanitize_text(&name, 80);
            if !value.is_empty() {
                set_doc.insert("name", value);
            }
        }
        if let Some(description) = request.description {
            set_doc.insert(
                "description",
                sanitized_opt_text(description, 280)
                    .map(bson::Bson::String)
                    .unwrap_or(bson::Bson::Null),
            );
        }
        if let Some(visibility) = request.visibility {
            set_doc.insert(
                "visibility",
                bson::to_bson(&visibility).unwrap_or(bson::Bson::String("team_public".into())),
            );
        }
        if let Some(channel_type) = request.channel_type {
            set_doc.insert(
                "channel_type",
                bson::to_bson(&channel_type).unwrap_or(bson::Bson::String("general".into())),
            );
        }
        if let Some(default_agent_id) = request.default_agent_id {
            set_doc.insert("default_agent_id", default_agent_id);
        }
        if let Some(workspace_display_name) = request.workspace_display_name {
            let value = sanitize_text(&workspace_display_name, 120);
            if !value.is_empty() {
                set_doc.insert("workspace_display_name", value);
            }
        }
        if let Some(repo_default_branch) = request.repo_default_branch {
            let value = sanitize_text(&repo_default_branch, 120);
            if !value.is_empty() {
                set_doc.insert("repo_default_branch", value);
            }
        }
        if set_doc.is_empty() {
            return self.get_channel(channel_id).await;
        }
        set_doc.insert("updated_at", bson_now());
        self.channels()
            .find_one_and_update(
                doc! { "channel_id": channel_id },
                doc! { "$set": set_doc },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await
    }

    pub async fn archive_channel(&self, channel_id: &str) -> Result<bool, mongodb::error::Error> {
        let result = self
            .channels()
            .update_one(
                doc! { "channel_id": channel_id },
                doc! { "$set": { "status": "archived", "updated_at": bson_now() } },
                None,
            )
            .await?;
        Ok(result.matched_count > 0)
    }

    pub async fn delete_channel(
        &self,
        channel_id: &str,
        deleted_by: &str,
        mode: ChatChannelDeleteMode,
    ) -> Result<bool> {
        let Some(channel) = self
            .channels()
            .find_one(doc! { "channel_id": channel_id }, None)
            .await?
        else {
            return Ok(false);
        };

        if matches!(mode, ChatChannelDeleteMode::FullDelete) {
            self.delete_channel_documents(&channel, deleted_by).await?;
            self.delete_channel_folder_tree(&channel, deleted_by)
                .await?;
        }

        self.channels()
            .delete_one(doc! { "channel_id": channel_id }, None)
            .await?;
        self.members()
            .delete_many(doc! { "channel_id": channel_id }, None)
            .await?;
        self.messages()
            .delete_many(doc! { "channel_id": channel_id }, None)
            .await?;
        self.events()
            .delete_many(doc! { "channel_id": channel_id }, None)
            .await?;
        self.reads()
            .delete_many(doc! { "channel_id": channel_id }, None)
            .await?;
        self.orchestrator_states()
            .delete_many(doc! { "channel_id": channel_id }, None)
            .await?;
        Ok(true)
    }

    async fn delete_channel_documents(
        &self,
        channel: &ChatChannelDoc,
        deleted_by: &str,
    ) -> Result<()> {
        let team_oid = ObjectId::parse_str(&channel.team_id)?;
        let documents = self.db.collection::<Document>("documents");
        let folder_regex = channel
            .document_folder_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(|path| Regex {
                pattern: format!("^{}/", regex::escape(path)),
                options: String::new(),
            });

        let mut filter = doc! {
            "team_id": team_oid,
            "is_deleted": { "$ne": true },
            "$or": [
                { "source_channel_id": &channel.channel_id },
            ],
        };
        if let Some(folder_path) = channel
            .document_folder_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            let mut clauses = vec![
                doc! { "source_channel_id": &channel.channel_id },
                doc! { "folder_path": folder_path },
            ];
            if let Some(regex) = folder_regex {
                clauses.push(doc! { "folder_path": { "$regex": regex } });
            }
            filter = doc! {
                "team_id": team_oid,
                "is_deleted": { "$ne": true },
                "$or": clauses,
            };
        }

        let docs: Vec<Document> = documents.find(filter, None).await?.try_collect().await?;
        let doc_service = DocumentService::new((*self.db).clone());
        for doc in docs {
            if let Some(id) = doc.id.map(|value| value.to_hex()) {
                doc_service
                    .delete(&channel.team_id, &id, deleted_by)
                    .await?;
            }
        }
        Ok(())
    }

    async fn delete_channel_folder_tree(
        &self,
        channel: &ChatChannelDoc,
        deleted_by: &str,
    ) -> Result<()> {
        let Some(folder_path) = channel
            .document_folder_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
        else {
            return Ok(());
        };

        let folder_service = FolderService::new((*self.db).clone());
        let mut folders = folder_service.list(&channel.team_id, None).await?;
        folders.retain(|folder| {
            folder.full_path == folder_path
                || folder.full_path.starts_with(&format!("{folder_path}/"))
        });
        folders.sort_by(|a, b| b.full_path.len().cmp(&a.full_path.len()));
        for folder in folders {
            if !folder.is_system {
                let _ = folder_service.delete(&folder.id, deleted_by).await;
            }
        }
        Ok(())
    }

    pub async fn get_message(
        &self,
        message_id: &str,
    ) -> Result<Option<ChatChannelMessageDoc>, mongodb::error::Error> {
        self.messages()
            .find_one(doc! { "message_id": message_id }, None)
            .await
    }

    pub async fn create_message(
        &self,
        channel: &ChatChannelDoc,
        thread_root_id: Option<String>,
        parent_message_id: Option<String>,
        surface: ChatChannelMessageSurface,
        thread_state: ChatChannelThreadState,
        collaboration_status: Option<ChatChannelDisplayStatus>,
        author_type: ChatChannelAuthorType,
        author_user_id: Option<String>,
        author_agent_id: Option<String>,
        author_name: String,
        agent_id: Option<String>,
        content_text: String,
        content_blocks: serde_json::Value,
        metadata: serde_json::Value,
    ) -> Result<ChatChannelMessageDoc, mongodb::error::Error> {
        let now = bson_now();
        let doc = ChatChannelMessageDoc {
            id: None,
            message_id: Uuid::new_v4().to_string(),
            channel_id: channel.channel_id.clone(),
            team_id: channel.team_id.clone(),
            thread_root_id,
            parent_message_id,
            surface,
            thread_state,
            collaboration_status,
            author_type,
            author_user_id,
            author_agent_id,
            author_name,
            agent_id,
            content_text: sanitize_text(&content_text, 32_000),
            content_blocks,
            metadata,
            visible: true,
            created_at: now,
            updated_at: now,
        };
        self.messages().insert_one(&doc, None).await?;
        Ok(doc)
    }

    fn to_message_response(
        &self,
        message: ChatChannelMessageDoc,
        reply_count: i32,
        has_ai_participation: bool,
        summary_text: String,
        recent_agent_names: Vec<String>,
    ) -> ChatChannelMessageResponse {
        let display_kind =
            Self::display_kind_for(message.surface, message.author_type, &message.metadata);
        ChatChannelMessageResponse {
            message_id: message.message_id,
            channel_id: message.channel_id,
            team_id: message.team_id,
            thread_root_id: message.thread_root_id,
            parent_message_id: message.parent_message_id,
            author_type: message.author_type,
            author_user_id: message.author_user_id,
            author_agent_id: message.author_agent_id,
            author_name: message.author_name,
            agent_id: message.agent_id,
            surface: message.surface,
            thread_state: message.thread_state,
            display_kind,
            display_status: Self::display_status_for(
                message.surface,
                message.thread_state,
                message.collaboration_status,
            ),
            source_kind: Self::source_kind_for(message.author_type),
            has_ai_participation,
            summary_text,
            recent_agent_names,
            content_text: message.content_text,
            content_blocks: message.content_blocks,
            metadata: message.metadata,
            visible: message.visible,
            created_at: to_rfc3339(message.created_at),
            updated_at: to_rfc3339(message.updated_at),
            reply_count,
        }
    }

    pub async fn list_root_messages(
        &self,
        channel_id: &str,
        filters: ChatChannelListFilters,
    ) -> Result<Vec<ChatChannelMessageResponse>, mongodb::error::Error> {
        let mut filter = doc! {
            "channel_id": channel_id,
            "$or": [
                { "thread_root_id": { "$exists": false } },
                { "thread_root_id": bson::Bson::Null }
            ]
        };
        if let Some(surface) = filters.surface {
            match surface {
                ChatChannelMessageSurface::Issue => {
                    filter.insert(
                        "$and",
                        vec![doc! {
                            "$or": [
                                { "surface": "issue" },
                                { "surface": { "$exists": false } },
                                { "surface": bson::Bson::Null }
                            ]
                        }],
                    );
                }
                _ => {
                    filter.insert(
                        "surface",
                        bson::to_bson(&surface)
                            .unwrap_or(bson::Bson::String("activity".to_string())),
                    );
                }
            }
        }
        if let Some(thread_state) = filters.thread_state {
            match thread_state {
                ChatChannelThreadState::Active => {
                    let extra = doc! {
                        "$or": [
                            { "thread_state": "active" },
                            { "thread_state": { "$exists": false } },
                            { "thread_state": bson::Bson::Null }
                        ]
                    };
                    if let Some(bson::Bson::Array(existing)) = filter.get("$and").cloned() {
                        let mut next = existing;
                        next.push(extra.into());
                        filter.insert("$and", bson::Bson::Array(next));
                    } else {
                        filter.insert("$and", vec![extra]);
                    }
                }
                _ => {
                    filter.insert(
                        "thread_state",
                        bson::to_bson(&thread_state)
                            .unwrap_or(bson::Bson::String("active".to_string())),
                    );
                }
            }
        }
        let messages: Vec<ChatChannelMessageDoc> = self
            .messages()
            .find(
                filter,
                FindOptions::builder()
                    .sort(doc! { "created_at": 1 })
                    .build(),
            )
            .await?
            .try_collect()
            .await?;

        let root_ids = messages
            .iter()
            .map(|item| item.message_id.clone())
            .collect::<Vec<_>>();
        let related: Vec<ChatChannelMessageDoc> = if root_ids.is_empty() {
            Vec::new()
        } else {
            self.messages()
                .find(
                    doc! {
                        "channel_id": channel_id,
                        "$or": [
                            { "thread_root_id": { "$in": root_ids.clone() } },
                            {
                                "parent_message_id": { "$in": root_ids.clone() },
                                "$or": [
                                    { "thread_root_id": { "$exists": false } },
                                    { "thread_root_id": bson::Bson::Null }
                                ]
                            }
                        ]
                    },
                    FindOptions::builder()
                        .sort(doc! { "created_at": 1 })
                        .build(),
                )
                .await?
                .try_collect()
                .await?
        };
        let mut reply_counts = HashMap::<String, i32>::new();
        let mut latest_ai_summary = HashMap::<String, String>::new();
        let mut has_ai_map = HashMap::<String, bool>::new();
        let mut recent_agent_names = HashMap::<String, Vec<String>>::new();
        for item in &related {
            let root_id = item
                .thread_root_id
                .clone()
                .or_else(|| item.parent_message_id.clone());
            let Some(root_id) = root_id else {
                continue;
            };
            *reply_counts.entry(root_id.clone()).or_insert(0) += 1;
            if matches!(item.author_type, ChatChannelAuthorType::Agent) {
                has_ai_map.insert(root_id.clone(), true);
                let names = recent_agent_names.entry(root_id.clone()).or_default();
                if !item.author_name.trim().is_empty() {
                    let normalized = sanitize_text(&item.author_name, 80);
                    if !names.contains(&normalized) {
                        names.push(normalized);
                        if names.len() > 2 {
                            names.remove(0);
                        }
                    }
                }
                if !item.content_text.trim().is_empty() {
                    latest_ai_summary.insert(root_id, sanitize_text(&item.content_text, 180));
                }
            }
        }
        let mut rendered = messages
            .into_iter()
            .map(|item| {
                let reply_count = reply_counts.get(&item.message_id).copied().unwrap_or(0);
                let has_ai_participation = matches!(item.author_type, ChatChannelAuthorType::Agent)
                    || has_ai_map.get(&item.message_id).copied().unwrap_or(false);
                let summary_text = latest_ai_summary
                    .get(&item.message_id)
                    .cloned()
                    .unwrap_or_else(|| sanitize_text(&item.content_text, 180));
                let recent_agents = recent_agent_names
                    .get(&item.message_id)
                    .cloned()
                    .unwrap_or_else(|| {
                        let metadata_agent_name = item
                            .metadata
                            .as_object()
                            .and_then(|meta| meta.get("selected_agent_name"))
                            .and_then(|value| value.as_str())
                            .map(|value| sanitize_text(value, 80))
                            .filter(|value| !value.is_empty());
                        if matches!(item.author_type, ChatChannelAuthorType::Agent)
                            && !item.author_name.trim().is_empty()
                        {
                            vec![sanitize_text(&item.author_name, 80)]
                        } else if let Some(name) = metadata_agent_name {
                            vec![name]
                        } else {
                            Vec::new()
                        }
                    });
                self.to_message_response(
                    item,
                    reply_count,
                    has_ai_participation,
                    summary_text,
                    recent_agents,
                )
            })
            .collect::<Vec<_>>();
        if let Some(display_kind) = filters.display_kind {
            rendered.retain(|item| item.display_kind == display_kind);
        }
        if let Some(display_status) = filters.display_status {
            rendered.retain(|item| item.display_status == Some(display_status));
        }
        Ok(rendered)
    }

    pub async fn list_thread_messages(
        &self,
        channel_id: &str,
        thread_root_id: &str,
    ) -> Result<Option<ChatChannelThreadResponse>, mongodb::error::Error> {
        let Some(root) = self
            .messages()
            .find_one(
                doc! { "channel_id": channel_id, "message_id": thread_root_id },
                None,
            )
            .await?
        else {
            return Ok(None);
        };
        let replies: Vec<ChatChannelMessageDoc> = self
            .messages()
            .find(
                doc! {
                    "channel_id": channel_id,
                    "$or": [
                        { "thread_root_id": thread_root_id },
                        {
                            "parent_message_id": thread_root_id,
                            "$or": [
                                { "thread_root_id": { "$exists": false } },
                                { "thread_root_id": bson::Bson::Null }
                            ]
                        }
                    ]
                },
                FindOptions::builder()
                    .sort(doc! { "created_at": 1 })
                    .build(),
            )
            .await?
            .try_collect()
            .await?;
        let reply_count = replies.len() as i32;
        let recent_agent_names = replies
            .iter()
            .filter(|item| matches!(item.author_type, ChatChannelAuthorType::Agent))
            .filter_map(|item| {
                let normalized = sanitize_text(&item.author_name, 80);
                if normalized.is_empty() {
                    None
                } else {
                    Some(normalized)
                }
            })
            .fold(Vec::<String>::new(), |mut acc, name| {
                if !acc.contains(&name) {
                    acc.push(name);
                    if acc.len() > 2 {
                        acc.remove(0);
                    }
                }
                acc
            });
        let recent_agent_names = if recent_agent_names.is_empty()
            && matches!(root.author_type, ChatChannelAuthorType::Agent)
            && !root.author_name.trim().is_empty()
        {
            vec![sanitize_text(&root.author_name, 80)]
        } else {
            recent_agent_names
        };
        let latest_ai_summary = replies
            .iter()
            .rev()
            .find(|item| {
                matches!(item.author_type, ChatChannelAuthorType::Agent)
                    && !item.content_text.trim().is_empty()
            })
            .map(|item| sanitize_text(&item.content_text, 180))
            .unwrap_or_else(|| sanitize_text(&root.content_text, 180));
        let has_ai_participation = matches!(root.author_type, ChatChannelAuthorType::Agent)
            || replies
                .iter()
                .any(|item| matches!(item.author_type, ChatChannelAuthorType::Agent));
        Ok(Some(ChatChannelThreadResponse {
            root_message: self.to_message_response(
                root,
                reply_count,
                has_ai_participation,
                latest_ai_summary,
                recent_agent_names.clone(),
            ),
            messages: replies
                .into_iter()
                .map(|item| {
                    let has_ai = matches!(item.author_type, ChatChannelAuthorType::Agent);
                    let summary = sanitize_text(&item.content_text, 180);
                    self.to_message_response(item, 0, has_ai, summary, recent_agent_names.clone())
                })
                .collect(),
            thread_runtime: None,
            runtime_diagnostics: None,
            delegation_runtime: None,
        }))
    }

    pub async fn update_thread_classification(
        &self,
        channel_id: &str,
        root_message_id: &str,
        surface: Option<ChatChannelMessageSurface>,
        thread_state: Option<ChatChannelThreadState>,
        collaboration_status: Option<Option<ChatChannelDisplayStatus>>,
    ) -> Result<bool, mongodb::error::Error> {
        let mut set_doc = doc! {
            "updated_at": bson_now(),
        };
        if let Some(surface) = surface {
            set_doc.insert(
                "surface",
                bson::to_bson(&surface).unwrap_or(bson::Bson::String("activity".to_string())),
            );
        }
        if let Some(thread_state) = thread_state {
            set_doc.insert(
                "thread_state",
                bson::to_bson(&thread_state).unwrap_or(bson::Bson::String("active".to_string())),
            );
        }
        if let Some(collaboration_status) = collaboration_status {
            set_doc.insert(
                "collaboration_status",
                collaboration_status
                    .map(|value| {
                        bson::to_bson(&value).unwrap_or(bson::Bson::String("active".to_string()))
                    })
                    .unwrap_or(bson::Bson::Null),
            );
        }
        let result = self
            .messages()
            .update_many(
                doc! {
                    "channel_id": channel_id,
                    "$or": [
                        { "message_id": root_message_id },
                        { "thread_root_id": root_message_id },
                        { "parent_message_id": root_message_id },
                    ]
                },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        if result.matched_count > 0 {
            self.refresh_channel_stats(channel_id).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn mark_read(
        &self,
        channel: &ChatChannelDoc,
        user_id: &str,
        last_read_message_id: Option<String>,
    ) -> Result<ChatChannelReadDoc, mongodb::error::Error> {
        let doc = ChatChannelReadDoc {
            id: None,
            channel_id: channel.channel_id.clone(),
            team_id: channel.team_id.clone(),
            user_id: user_id.to_string(),
            last_read_message_id,
            last_read_at: bson_now(),
        };
        self.reads()
            .replace_one(
                doc! { "channel_id": &channel.channel_id, "user_id": user_id },
                &doc,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(doc)
    }

    pub async fn unread_count(
        &self,
        channel_id: &str,
        user_id: &str,
    ) -> Result<i32, mongodb::error::Error> {
        let read = self
            .reads()
            .find_one(doc! { "channel_id": channel_id, "user_id": user_id }, None)
            .await?;
        let filter = if let Some(read_doc) = read {
            doc! {
                "channel_id": channel_id,
                "created_at": { "$gt": read_doc.last_read_at },
                "author_user_id": { "$ne": user_id }
            }
        } else {
            doc! {
                "channel_id": channel_id,
                "author_user_id": { "$ne": user_id }
            }
        };
        Ok(self.messages().count_documents(filter, None).await? as i32)
    }

    pub async fn set_processing(
        &self,
        channel_id: &str,
        run_id: &str,
        processing: bool,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson_now();
        let mut set_doc = doc! {
            "is_processing": processing,
            "updated_at": now,
        };
        if processing {
            set_doc.insert("active_run_id", run_id);
            set_doc.insert("last_run_status", "running");
            set_doc.insert("last_run_error", bson::Bson::Null);
            set_doc.insert("last_run_finished_at", bson::Bson::Null);
        } else {
            set_doc.insert("active_run_id", bson::Bson::Null);
        }
        self.channels()
            .update_one(
                doc! { "channel_id": channel_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn try_start_processing(
        &self,
        channel_id: &str,
        run_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let result = self
            .channels()
            .update_one(
                doc! { "channel_id": channel_id, "is_processing": false, "status": "active" },
                doc! {
                    "$set": {
                        "is_processing": true,
                        "active_run_id": run_id,
                        "last_run_status": "running",
                        "last_run_error": bson::Bson::Null,
                        "last_run_finished_at": bson::Bson::Null,
                        "updated_at": bson_now(),
                    }
                },
                None,
            )
            .await?;
        Ok(result.matched_count > 0)
    }

    pub async fn update_run_result(
        &self,
        channel_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson_now();
        let mut set_doc = doc! {
            "is_processing": false,
            "active_run_id": bson::Bson::Null,
            "last_run_status": status,
            "last_run_finished_at": now,
            "updated_at": now,
        };
        set_doc.insert(
            "last_run_error",
            error
                .map(|value| bson::Bson::String(value.to_string()))
                .unwrap_or(bson::Bson::Null),
        );
        self.channels()
            .update_one(
                doc! { "channel_id": channel_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_run_state(
        &self,
        channel_id: &str,
        run_id: &str,
        status: &str,
        processing: bool,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson_now();
        let mut set_doc = doc! {
            "is_processing": processing,
            "last_run_status": status,
            "updated_at": now,
        };
        if processing {
            set_doc.insert("active_run_id", run_id);
            set_doc.insert("last_run_error", bson::Bson::Null);
            set_doc.insert("last_run_finished_at", bson::Bson::Null);
        } else {
            set_doc.insert("active_run_id", bson::Bson::Null);
            set_doc.insert("last_run_finished_at", now);
        }
        self.channels()
            .update_one(
                doc! { "channel_id": channel_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn refresh_channel_stats(
        &self,
        channel_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let latest = self
            .messages()
            .find_one(
                doc! { "channel_id": channel_id },
                mongodb::options::FindOneOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .build(),
            )
            .await?;
        let message_count = self
            .messages()
            .count_documents(doc! { "channel_id": channel_id }, None)
            .await? as i32;
        let thread_count = self
            .messages()
            .count_documents(
                doc! {
                    "channel_id": channel_id,
                    "$or": [
                        { "thread_root_id": { "$exists": false } },
                        { "thread_root_id": bson::Bson::Null }
                    ]
                },
                None,
            )
            .await? as i32;
        let mut set_doc = doc! {
            "message_count": message_count,
            "thread_count": thread_count,
            "updated_at": bson_now(),
        };
        if let Some(message) = latest {
            set_doc.insert(
                "last_message_preview",
                sanitize_text(&message.content_text, 200),
            );
            set_doc.insert("last_message_at", message.created_at);
        }
        self.channels()
            .update_one(
                doc! { "channel_id": channel_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn save_channel_events(
        &self,
        channel_id: &str,
        thread_root_id: Option<&str>,
        root_message_id: Option<&str>,
        records: &[(String, u64, super::task_manager::StreamEvent)],
    ) -> Result<(), mongodb::error::Error> {
        if records.is_empty() {
            return Ok(());
        }
        let docs = records
            .iter()
            .map(|(run_id, event_id, event)| ChatChannelEventDoc {
                id: None,
                channel_id: channel_id.to_string(),
                run_id: Some(run_id.clone()),
                thread_root_id: thread_root_id.map(|value| value.to_string()),
                root_message_id: root_message_id.map(|value| value.to_string()),
                event_id: (*event_id).try_into().unwrap_or(i64::MAX),
                event_type: event.event_type().to_string(),
                payload: serde_json::to_value(event).unwrap_or_else(|_| serde_json::json!({})),
                created_at: bson_now(),
            })
            .collect::<Vec<_>>();
        self.events().insert_many(docs, None).await?;
        Ok(())
    }

    pub async fn list_channel_events(
        &self,
        channel_id: &str,
        run_id: Option<&str>,
        limit: u32,
        descending: bool,
    ) -> Result<Vec<ChatChannelEventDoc>, mongodb::error::Error> {
        let mut filter = doc! { "channel_id": channel_id };
        if let Some(run_id) = run_id {
            filter.insert("run_id", run_id);
        }
        self.events()
            .find(
                filter,
                FindOptions::builder()
                    .sort(doc! { "event_id": if descending { -1 } else { 1 } })
                    .limit(limit as i64)
                    .build(),
            )
            .await?
            .try_collect()
            .await
    }
}
