//! Agent service layer for business logic (MongoDB version)

use super::capability_policy::resolve_document_policy;
use super::chat_channels::ChatWorkspaceFileBlock;
use super::delegation_runtime::DelegationRuntimeResponse;
use super::harness_core::{
    RunCheckpoint, RunCheckpointKind, RunJournal, RunLease, RunMemory, RunState, RunStatus,
    SubagentRun, TaskGraph, TurnOutcome,
};
use super::normalize_workspace_path;
use super::session_mongo::{
    AgentSessionDoc, ChatEventDoc, CreateSessionRequest, SessionListItem, SessionListQuery,
    UserSessionListQuery,
};
use super::task_manager::StreamEvent;
use super::workspace_service::WorkspaceBinding;
use agime::agents::types::RetryConfig;
use agime::context_runtime::ContextRuntimeState;
use agime_team::models::mongo::Document as TeamDocument;
use agime_team::models::{
    AgentExtensionConfig, AgentSkillConfig, AgentStatus, AgentTask, ApiFormat,
    AttachedTeamExtensionRef, BuiltinExtension, CreateAgentRequest, CustomExtensionConfig,
    DelegationPolicy, ListAgentsQuery, ListTasksQuery, PaginatedResponse, RuntimeOptimizationMode,
    SkillBindingMode, SubmitTaskRequest, TaskResult, TaskResultType, TaskStatus, TaskType,
    TeamAgent, UpdateAgentRequest,
};
use agime_team::MongoDb;
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

/// Custom serde module for Option<DateTime<Utc>> with BSON datetime
mod bson_datetime_option {
    use chrono::{DateTime, Utc};
    use serde::{self, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(date: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match date {
            Some(dt) => {
                let bson_dt = bson::DateTime::from_chrono(*dt);
                Serialize::serialize(&bson_dt, serializer)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<bson::DateTime> = Option::deserialize(deserializer)?;
        Ok(opt.map(|dt| dt.to_chrono()))
    }
}

/// Validation error for agent operations
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Name is required and must be 1-100 characters")]
    Name,
    #[error("Invalid API URL format")]
    ApiUrl,
    #[error("Model name must be 1-100 characters")]
    Model,
    #[error("Priority must be between 0 and 100")]
    Priority,
    #[error("Invalid extension config: missing uri_or_cmd (or legacy uriOrCmd/command)")]
    ExtensionConfig,
    #[error(
        "Invalid custom extension type. Use one of: stdio | sse | streamable_http | streamablehttp"
    )]
    CustomExtensionType,
}

/// Service error that includes both database and validation errors
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("Database error: {0}")]
    Database(#[from] mongodb::error::Error),
    #[error("Validation error: {0}")]
    Validation(#[from] ValidationError),
    #[error("Internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionSlotAcquireOutcome {
    Acquired,
    Saturated,
}

/// Validate agent name
fn validate_name(name: &str) -> Result<(), ValidationError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 100 {
        return Err(ValidationError::Name);
    }
    Ok(())
}

/// Validate API URL format
fn validate_api_url(url: &Option<String>) -> Result<(), ValidationError> {
    if let Some(ref u) = url {
        let trimmed = u.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("http://")
            && !trimmed.starts_with("https://")
        {
            return Err(ValidationError::ApiUrl);
        }
    }
    Ok(())
}

/// Validate model name
fn validate_model(model: &Option<String>) -> Result<(), ValidationError> {
    if let Some(ref m) = model {
        let trimmed = m.trim();
        if trimmed.len() > 100 {
            return Err(ValidationError::Model);
        }
    }
    Ok(())
}

/// Validate priority
fn validate_priority(priority: i32) -> Result<(), ValidationError> {
    if !(0..=100).contains(&priority) {
        return Err(ValidationError::Priority);
    }
    Ok(())
}

fn normalize_max_concurrent_tasks(value: Option<u32>) -> u32 {
    value.unwrap_or(1).max(1)
}

/// Serialize custom extension configs for MongoDB persistence.
/// We build BSON explicitly so secret envs are stored in DB even though
/// API responses keep envs redacted via `skip_serializing`.
fn custom_extension_to_bson_document(ext: &CustomExtensionConfig) -> Document {
    let args = ext
        .args
        .iter()
        .map(|arg| Bson::String(arg.clone()))
        .collect::<Vec<_>>();

    let envs = ext
        .envs
        .iter()
        .map(|(k, v)| (k.clone(), Bson::String(v.clone())))
        .collect::<Document>();

    let mut doc = doc! {
        "name": ext.name.clone(),
        "type": ext.ext_type.clone(),
        "uri_or_cmd": ext.uri_or_cmd.clone(),
        "args": args,
        "envs": envs,
        "enabled": ext.enabled,
    };

    if let Some(source) = &ext.source {
        doc.insert("source", source.clone());
    }
    if let Some(source_extension_id) = &ext.source_extension_id {
        doc.insert("source_extension_id", source_extension_id.clone());
    }

    doc
}

fn custom_extensions_to_bson(exts: &[CustomExtensionConfig]) -> Bson {
    Bson::Array(
        exts.iter()
            .map(|ext| Bson::Document(custom_extension_to_bson_document(ext)))
            .collect::<Vec<_>>(),
    )
}

fn validate_custom_extension_name(name: &str) -> Result<(), ValidationError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 120 {
        return Err(ValidationError::Name);
    }
    Ok(())
}

fn normalize_custom_extension_type(ext_type: &str) -> Result<String, ValidationError> {
    let normalized = ext_type
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .replace(' ', "_");
    match normalized.as_str() {
        "stdio" => Ok("stdio".to_string()),
        "sse" => Ok("sse".to_string()),
        "streamablehttp" | "streamable_http" => Ok("streamable_http".to_string()),
        _ => Err(ValidationError::CustomExtensionType),
    }
}

fn normalize_custom_extension_config(
    extension: CustomExtensionConfig,
) -> Result<CustomExtensionConfig, ValidationError> {
    validate_custom_extension_name(&extension.name)?;

    let uri_or_cmd = extension.uri_or_cmd.trim().to_string();
    if uri_or_cmd.is_empty() {
        return Err(ValidationError::ExtensionConfig);
    }

    let args = extension
        .args
        .into_iter()
        .map(|arg| arg.trim().to_string())
        .filter(|arg| !arg.is_empty())
        .collect::<Vec<_>>();

    let envs = extension
        .envs
        .into_iter()
        .filter_map(|(key, value)| {
            let normalized_key = key.trim().to_string();
            if normalized_key.is_empty() {
                return None;
            }
            Some((normalized_key, value))
        })
        .collect::<HashMap<_, _>>();

    let source = extension
        .source
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| Some("custom".to_string()));
    let source_extension_id = extension
        .source_extension_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    Ok(CustomExtensionConfig {
        name: extension.name.trim().to_string(),
        ext_type: normalize_custom_extension_type(&extension.ext_type)?,
        uri_or_cmd,
        args,
        envs,
        enabled: extension.enabled,
        source,
        source_extension_id,
    })
}

fn custom_extension_name_eq(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn custom_extension_matches_source_extension(
    extension: &CustomExtensionConfig,
    source_extension_id: &str,
) -> bool {
    extension
        .source_extension_id
        .as_deref()
        .map(|value| value == source_extension_id)
        .unwrap_or(false)
}

fn extract_task_runtime_binding(
    content: &serde_json::Value,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let run_id = content
        .get("run_id")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string());
    let session_id = content
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string());
    let task_role = content
        .get("task_role")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string());
    let task_node_id = content
        .get("task_node_id")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string());
    (run_id, session_id, task_role, task_node_id)
}

// MongoDB document types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamAgentDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub agent_id: String,
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub avatar: Option<String>,
    pub system_prompt: Option<String>,
    pub api_url: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub api_format: String,
    pub status: String,
    pub last_error: Option<String>,
    pub enabled_extensions: Vec<AgentExtensionConfig>,
    pub custom_extensions: Vec<CustomExtensionConfig>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub owner_manager_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub template_source_agent_id: Option<String>,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_tasks: u32,
    #[serde(default)]
    pub active_execution_slots: u32,
    /// LLM temperature (0.0 - 1.0)
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub temperature: Option<f32>,
    /// Maximum output tokens per LLM call
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_tokens: Option<i32>,
    /// Context window limit override
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_limit: Option<usize>,
    /// Whether think/reasoning mode should be enabled for this agent.
    #[serde(default = "default_thinking_enabled")]
    pub thinking_enabled: bool,
    /// Optional thinking budget override for supported models.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub thinking_budget: Option<u32>,
    /// Optional reasoning effort override for reasoning-capable models.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reasoning_effort: Option<String>,
    /// Optional reserved output budget for context runtime.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub output_reserve_tokens: Option<usize>,
    /// Optional auto-compact threshold override for context runtime.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub auto_compact_threshold: Option<f64>,
    /// Whether this agent's configured model/provider should receive image inputs.
    #[serde(default)]
    pub supports_multimodal: bool,
    /// Prompt caching preference for providers that support it.
    #[serde(default)]
    pub prompt_caching_mode: RuntimeOptimizationMode,
    /// Cache-edit preference for providers that support it.
    #[serde(default)]
    pub cache_edit_mode: RuntimeOptimizationMode,
    /// Skills assigned from team shared skills
    #[serde(default)]
    pub assigned_skills: Vec<AgentSkillConfig>,
    #[serde(default)]
    pub skill_binding_mode: SkillBindingMode,
    #[serde(default)]
    pub delegation_policy: DelegationPolicy,
    #[serde(default)]
    pub attached_team_extensions: Vec<AttachedTeamExtensionRef>,
    /// Auto-approve chat tasks (skip manual approval for chat messages)
    #[serde(default = "default_auto_approve_chat")]
    pub auto_approve_chat: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvatarGovernanceCounts {
    #[serde(default)]
    pub pending_capability_requests: u32,
    #[serde(default)]
    pub pending_gap_proposals: u32,
    #[serde(default)]
    pub pending_optimization_tickets: u32,
    #[serde(default)]
    pub pending_runtime_logs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarInstanceDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub portal_id: String,
    pub team_id: String,
    pub slug: String,
    pub name: String,
    pub status: String,
    pub avatar_type: String,
    pub manager_agent_id: Option<String>,
    pub service_agent_id: Option<String>,
    pub document_access_mode: String,
    pub governance_counts: AvatarGovernanceCounts,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub portal_updated_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub projected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarInstanceSummary {
    pub portal_id: String,
    pub team_id: String,
    pub slug: String,
    pub name: String,
    pub status: String,
    pub avatar_type: String,
    pub manager_agent_id: Option<String>,
    pub service_agent_id: Option<String>,
    pub document_access_mode: String,
    pub governance_counts: AvatarGovernanceCounts,
    pub portal_updated_at: DateTime<Utc>,
    pub projected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceStateDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub portal_id: String,
    pub team_id: String,
    pub state: serde_json::Value,
    pub config: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceStatePayload {
    pub portal_id: String,
    pub team_id: String,
    pub state: serde_json::Value,
    pub config: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceEventDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub portal_id: String,
    pub team_id: String,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: Option<String>,
    pub title: String,
    pub status: Option<String>,
    pub detail: Option<String>,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub meta: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceEventPayload {
    pub event_id: String,
    pub portal_id: String,
    pub team_id: String,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: Option<String>,
    pub title: String,
    pub status: Option<String>,
    pub detail: Option<String>,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub meta: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceQueueItemPayload {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub status: String,
    pub ts: String,
    pub meta: Vec<String>,
    pub source_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarWorkbenchSummaryPayload {
    pub portal_id: String,
    pub team_id: String,
    pub avatar_name: String,
    pub avatar_type: String,
    pub avatar_status: String,
    pub manager_agent_id: Option<String>,
    pub service_agent_id: Option<String>,
    pub document_access_mode: String,
    pub work_object_count: u32,
    pub pending_decision_count: u32,
    pub last_activity_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarWorkbenchReportItemPayload {
    pub id: String,
    pub ts: DateTime<Utc>,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub source: String,
    pub recommendation: Option<String>,
    pub action_kind: Option<String>,
    pub action_target_id: Option<String>,
    #[serde(default)]
    pub work_objects: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub needs_decision: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarWorkbenchDecisionItemPayload {
    pub id: String,
    pub ts: DateTime<Utc>,
    #[serde(default)]
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub status: String,
    pub risk: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub recommendation: Option<String>,
    #[serde(default)]
    pub work_objects: Vec<String>,
    pub action_kind: Option<String>,
    pub action_target_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarWorkbenchSnapshotPayload {
    pub portal_id: String,
    pub team_id: String,
    pub summary: AvatarWorkbenchSummaryPayload,
    pub reports: Vec<AvatarWorkbenchReportItemPayload>,
    pub decisions: Vec<AvatarWorkbenchDecisionItemPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarManagerReportDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub report_id: String,
    pub portal_id: String,
    pub team_id: String,
    #[serde(default = "default_avatar_manager_report_source")]
    pub report_source: String,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub recommendation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub action_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub action_target_id: Option<String>,
    #[serde(default)]
    pub work_objects: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub needs_decision: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub synced_at: DateTime<Utc>,
}

fn default_avatar_manager_report_source() -> String {
    "derived".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvatarGovernanceBackfillReport {
    pub portals_scanned: u64,
    pub states_created: u64,
    pub events_seeded: u64,
    pub projections_synced_teams: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvatarGovernanceDivergenceKind {
    DefaultOnly,
    InSync,
    StateOnly,
    SettingsOnly,
    Differing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceDivergenceRow {
    pub team_id: String,
    pub portal_id: String,
    pub portal_name: String,
    pub slug: String,
    pub classification: AvatarGovernanceDivergenceKind,
    pub state_doc_exists: bool,
    pub portal_state_exists: bool,
    pub portal_config_exists: bool,
    pub state_matches_portal_state: bool,
    pub config_matches_portal_config: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceOrphanStateRow {
    pub team_id: String,
    pub portal_id: String,
    pub state_doc_count: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceDuplicateStateRow {
    pub team_id: String,
    pub portal_id: String,
    pub state_doc_count: u64,
    pub retained_updated_at: String,
    pub duplicate_updated_at: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvatarGovernanceDivergenceAuditReport {
    pub total_avatar_portals: u64,
    pub total_state_docs: u64,
    pub default_only: u64,
    pub in_sync: u64,
    pub state_only: u64,
    pub settings_only: u64,
    pub differing: u64,
    pub duplicate_state_docs: u64,
    pub orphan_state_docs: u64,
    pub rows: Vec<AvatarGovernanceDivergenceRow>,
    pub duplicate_rows: Vec<AvatarGovernanceDuplicateStateRow>,
    pub orphan_rows: Vec<AvatarGovernanceOrphanStateRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvatarBindingIssueKind {
    MissingExplicitManagerBinding,
    MissingExplicitServiceBinding,
    MissingManagerBinding,
    MissingServiceBinding,
    ManagerAgentNotFound,
    ServiceAgentNotFound,
    ManagerRoleMismatch,
    ServiceRoleMismatch,
    OwnerManagerMismatch,
    SameAgentReused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarBindingAuditRow {
    pub team_id: String,
    pub portal_id: String,
    pub portal_name: String,
    pub slug: String,
    pub explicit_coding_agent_id: Option<String>,
    pub explicit_service_agent_id: Option<String>,
    pub effective_manager_agent_id: Option<String>,
    pub effective_service_agent_id: Option<String>,
    pub manager_agent_domain: Option<String>,
    pub manager_agent_role: Option<String>,
    pub service_agent_domain: Option<String>,
    pub service_agent_role: Option<String>,
    pub service_owner_manager_agent_id: Option<String>,
    pub issues: Vec<AvatarBindingIssueKind>,
    pub issue_messages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvatarBindingAuditReport {
    pub total_avatar_portals: u64,
    pub valid: u64,
    pub missing_explicit_manager_binding: u64,
    pub missing_explicit_service_binding: u64,
    pub missing_manager_binding: u64,
    pub missing_service_binding: u64,
    pub manager_agent_not_found: u64,
    pub service_agent_not_found: u64,
    pub manager_role_mismatch: u64,
    pub service_role_mismatch: u64,
    pub owner_manager_mismatch: u64,
    pub same_agent_reused: u64,
    pub shadow_invariant_rows: u64,
    pub rows: Vec<AvatarBindingAuditRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarReadSideEffectAuditItem {
    pub operation: String,
    pub file: String,
    pub side_effects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarDeepWaterAuditReport {
    pub generated_at: String,
    pub requested_team_id: Option<String>,
    pub governance: AvatarGovernanceDivergenceAuditReport,
    pub bindings: AvatarBindingAuditReport,
    pub read_side_effects: Vec<AvatarReadSideEffectAuditItem>,
}

#[derive(Debug, Clone)]
struct GovernanceEntitySnapshot {
    entity_type: &'static str,
    id: String,
    title: String,
    status: String,
    detail: String,
    meta: serde_json::Value,
}

fn queue_item_risk(meta: &[String]) -> &'static str {
    if meta.iter().any(|item| item.contains("高风险")) {
        "high"
    } else if meta.iter().any(|item| item.contains("中风险")) {
        "medium"
    } else {
        "low"
    }
}

fn queue_item_source(kind: &str, meta: &[String]) -> String {
    meta.iter()
        .find(|item| {
            !item.contains("风险")
                && !item.contains("high")
                && !item.contains("medium")
                && !item.contains("low")
        })
        .cloned()
        .unwrap_or_else(|| match kind {
            "capability" => "能力边界".to_string(),
            "proposal" => "岗位提案".to_string(),
            "ticket" => "运行优化".to_string(),
            _ => "管理台".to_string(),
        })
}

fn queue_item_recommendation(kind: &str, risk: &str) -> String {
    match kind {
        "capability" => {
            if risk == "high" {
                "先确认这次能力放开是否触及高风险边界，再决定是交人工审批还是保持收敛。".to_string()
            } else {
                "先判断是否需要放开当前能力边界，再决定批准、试运行或继续收敛。".to_string()
            }
        }
        "proposal" => "先判断这项岗位提案是否值得试运行，再决定批准、试点或拒绝。".to_string(),
        "ticket" => {
            if risk == "high" {
                "先查看这次运行问题与风险证据，再决定回滚、人工确认或进入试运行。".to_string()
            } else {
                "先确认优化证据和预期收益，再决定试运行、批准或继续观察。".to_string()
            }
        }
        _ => "先阅读当前决策背景，再决定继续执行、调整方案或转人工确认。".to_string(),
    }
}

fn parse_rfc3339_utc(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn default_max_concurrent() -> u32 {
    1
}

fn default_auto_approve_chat() -> bool {
    true
}

fn default_thinking_enabled() -> bool {
    true
}

fn is_runtime_legacy_builtin_extension(extension: BuiltinExtension) -> bool {
    matches!(extension, BuiltinExtension::Team)
}

fn sanitize_group_ids(groups: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    groups
        .into_iter()
        .map(|group| group.trim().to_string())
        .filter(|group| !group.is_empty())
        .filter(|group| seen.insert(group.clone()))
        .collect()
}

fn sanitize_enabled_extensions(configs: Vec<AgentExtensionConfig>) -> Vec<AgentExtensionConfig> {
    configs
        .into_iter()
        .map(|mut config| {
            config.allowed_groups = sanitize_group_ids(config.allowed_groups);
            config
        })
        .filter(|config| !is_runtime_legacy_builtin_extension(config.extension))
        .collect()
}

fn sanitize_assigned_skills(skills: Vec<AgentSkillConfig>) -> Vec<AgentSkillConfig> {
    skills
        .into_iter()
        .map(|mut skill| {
            skill.skill_id = skill.skill_id.trim().to_string();
            skill.allowed_groups = sanitize_group_ids(skill.allowed_groups);
            skill
        })
        .filter(|skill| !skill.skill_id.is_empty())
        .collect()
}

fn sanitize_attached_team_extensions(
    refs: Vec<AttachedTeamExtensionRef>,
) -> Vec<AttachedTeamExtensionRef> {
    let mut seen = HashSet::new();
    refs.into_iter()
        .map(|mut item| {
            item.extension_id = item.extension_id.trim().to_string();
            item.runtime_name = item
                .runtime_name
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            item.display_name = item
                .display_name
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            item.transport = item
                .transport
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            item.allowed_groups = sanitize_group_ids(item.allowed_groups);
            item
        })
        .filter(|item| !item.extension_id.is_empty())
        .filter(|item| seen.insert(item.extension_id.clone()))
        .collect()
}

impl From<TeamAgentDoc> for TeamAgent {
    fn from(doc: TeamAgentDoc) -> Self {
        Self {
            id: doc.agent_id,
            team_id: doc.team_id,
            name: doc.name,
            description: doc.description,
            avatar: doc.avatar,
            system_prompt: doc.system_prompt,
            api_url: doc.api_url,
            model: doc.model,
            api_key: None, // Don't expose API key
            api_format: doc.api_format.parse().unwrap_or(ApiFormat::OpenAI),
            enabled_extensions: sanitize_enabled_extensions(doc.enabled_extensions),
            custom_extensions: doc.custom_extensions,
            agent_domain: doc.agent_domain,
            agent_role: doc.agent_role,
            owner_manager_agent_id: doc.owner_manager_agent_id,
            template_source_agent_id: doc.template_source_agent_id,
            status: doc.status.parse().unwrap_or(AgentStatus::Idle),
            last_error: doc.last_error,
            allowed_groups: doc.allowed_groups,
            max_concurrent_tasks: doc.max_concurrent_tasks,
            active_execution_slots: doc.active_execution_slots,
            temperature: doc.temperature,
            max_tokens: doc.max_tokens,
            context_limit: doc.context_limit,
            thinking_enabled: doc.thinking_enabled,
            thinking_budget: doc.thinking_budget,
            reasoning_effort: doc.reasoning_effort,
            output_reserve_tokens: doc.output_reserve_tokens,
            auto_compact_threshold: doc.auto_compact_threshold,
            supports_multimodal: doc.supports_multimodal,
            prompt_caching_mode: doc.prompt_caching_mode,
            cache_edit_mode: doc.cache_edit_mode,
            assigned_skills: sanitize_assigned_skills(doc.assigned_skills),
            skill_binding_mode: doc.skill_binding_mode,
            delegation_policy: doc.delegation_policy,
            attached_team_extensions: sanitize_attached_team_extensions(
                doc.attached_team_extensions,
            ),
            auto_approve_chat: doc.auto_approve_chat,
            created_at: doc.created_at,
            updated_at: doc.updated_at,
        }
    }
}

impl From<AvatarManagerReportDoc> for AvatarWorkbenchReportItemPayload {
    fn from(doc: AvatarManagerReportDoc) -> Self {
        Self {
            id: doc.report_id,
            ts: doc.created_at,
            kind: doc.kind,
            title: doc.title,
            summary: doc.summary,
            status: doc.status,
            source: doc.source,
            recommendation: doc.recommendation,
            action_kind: doc.action_kind,
            action_target_id: doc.action_target_id,
            work_objects: doc.work_objects,
            outputs: doc.outputs,
            needs_decision: doc.needs_decision,
        }
    }
}

impl From<AvatarInstanceDoc> for AvatarInstanceSummary {
    fn from(doc: AvatarInstanceDoc) -> Self {
        Self {
            portal_id: doc.portal_id,
            team_id: doc.team_id,
            slug: doc.slug,
            name: doc.name,
            status: doc.status,
            avatar_type: doc.avatar_type,
            manager_agent_id: doc.manager_agent_id,
            service_agent_id: doc.service_agent_id,
            document_access_mode: doc.document_access_mode,
            governance_counts: doc.governance_counts,
            portal_updated_at: doc.portal_updated_at,
            projected_at: doc.projected_at,
        }
    }
}

impl From<AvatarGovernanceStateDoc> for AvatarGovernanceStatePayload {
    fn from(doc: AvatarGovernanceStateDoc) -> Self {
        Self {
            portal_id: doc.portal_id,
            team_id: doc.team_id,
            state: doc.state,
            config: doc.config,
            updated_at: doc.updated_at,
        }
    }
}

impl From<AvatarGovernanceEventDoc> for AvatarGovernanceEventPayload {
    fn from(doc: AvatarGovernanceEventDoc) -> Self {
        let event_id = doc.id.map(|value| value.to_hex()).unwrap_or_else(|| {
            format!(
                "synthetic:{}:{}:{}:{}:{}:{}",
                doc.team_id,
                doc.portal_id,
                doc.event_type,
                doc.entity_type,
                doc.entity_id.as_deref().unwrap_or("none"),
                doc.created_at.timestamp_millis()
            )
        });
        Self {
            event_id,
            portal_id: doc.portal_id,
            team_id: doc.team_id,
            event_type: doc.event_type,
            entity_type: doc.entity_type,
            entity_id: doc.entity_id,
            title: doc.title,
            status: doc.status,
            detail: doc.detail,
            actor_id: doc.actor_id,
            actor_name: doc.actor_name,
            meta: doc.meta,
            created_at: doc.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTaskDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_node_id: Option<String>,
    pub team_id: String,
    pub agent_id: String,
    pub submitter_id: String,
    pub approver_id: Option<String>,
    pub task_type: String,
    pub content: serde_json::Value,
    pub status: String,
    pub priority: i32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub submitted_at: DateTime<Utc>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub approved_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
}

impl From<AgentTaskDoc> for AgentTask {
    fn from(doc: AgentTaskDoc) -> Self {
        Self {
            id: doc.task_id,
            team_id: doc.team_id,
            agent_id: doc.agent_id,
            submitter_id: doc.submitter_id,
            approver_id: doc.approver_id,
            task_type: doc.task_type.parse().unwrap_or(TaskType::Chat),
            content: doc.content,
            status: doc.status.parse().unwrap_or(TaskStatus::Pending),
            priority: doc.priority,
            submitted_at: doc.submitted_at,
            approved_at: doc.approved_at,
            started_at: doc.started_at,
            completed_at: doc.completed_at,
            error_message: doc.error_message,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResultDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub result_id: String,
    pub task_id: String,
    pub result_type: String,
    pub content: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl From<TaskResultDoc> for TaskResult {
    fn from(doc: TaskResultDoc) -> Self {
        let result_type = match doc.result_type.as_str() {
            "message" => TaskResultType::Message,
            "tool_call" => TaskResultType::ToolCall,
            _ => TaskResultType::Error,
        };
        Self {
            id: doc.result_id,
            task_id: doc.task_id,
            result_type,
            content: doc.content,
            created_at: doc.created_at,
        }
    }
}

/// Convert a TeamAgentDoc to TeamAgent while preserving the api_key field
/// (which is normally stripped by the From impl for API safety).
fn agent_doc_with_key(doc: TeamAgentDoc) -> TeamAgent {
    let api_key = doc.api_key.clone();
    let mut agent: TeamAgent = doc.into();
    agent.api_key = api_key;
    agent
}

/// Agent service for managing team agents and tasks (MongoDB)
pub struct AgentService {
    db: Arc<MongoDb>,
}

impl AgentService {
    pub fn new(db: Arc<MongoDb>) -> Self {
        Self { db }
    }

    fn normalize_session_source(raw: Option<String>, portal_restricted: bool) -> String {
        let v = raw
            .unwrap_or_else(|| {
                if portal_restricted {
                    "portal".to_string()
                } else {
                    "chat".to_string()
                }
            })
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_");
        match v.as_str() {
            "portal"
            | "portal_coding"
            | "portal_manager"
            | "system"
            | "document_analysis"
            | "agent_task"
            | "subagent"
            | "chat"
            | "automation_builder"
            | "automation_runtime"
            | "scheduled_task"
            | "channel_runtime"
            | "channel_conversation" => v,
            _ => {
                if portal_restricted {
                    "portal".to_string()
                } else {
                    "chat".to_string()
                }
            }
        }
    }

    fn normalize_skill_id_list(items: Vec<String>) -> Vec<String> {
        let mut seen = HashSet::new();
        items
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .filter(|item| seen.insert(item.clone()))
            .collect()
    }

    async fn resolve_session_allowed_skill_ids(
        &self,
        agent_id: &str,
        requested_allowed_skill_ids: Option<Vec<String>>,
        session_source: &str,
        portal_restricted: bool,
    ) -> Result<Option<Vec<String>>, mongodb::error::Error> {
        let requested_allowed_skill_ids =
            requested_allowed_skill_ids.map(Self::normalize_skill_id_list);

        let Some(agent) = self.get_agent(agent_id).await? else {
            return Ok(requested_allowed_skill_ids);
        };

        let assigned_skill_ids = Self::normalize_skill_id_list(
            agent
                .assigned_skills
                .iter()
                .filter(|skill| skill.enabled)
                .map(|skill| skill.skill_id.clone())
                .collect(),
        );
        let restricted_scope = portal_restricted
            || matches!(
                session_source,
                "portal" | "portal_coding" | "portal_manager" | "system" | "document_analysis"
            );

        let base_scope = match agent.skill_binding_mode {
            SkillBindingMode::AssignedOnly => Some(assigned_skill_ids.clone()),
            SkillBindingMode::Hybrid => {
                if restricted_scope {
                    Some(assigned_skill_ids.clone())
                } else {
                    None
                }
            }
            SkillBindingMode::OnDemandOnly => {
                if restricted_scope {
                    Some(Vec::new())
                } else {
                    None
                }
            }
        };

        let effective = match (base_scope, requested_allowed_skill_ids) {
            (Some(base), Some(requested)) => Some(
                requested
                    .into_iter()
                    .filter(|skill_id| base.contains(skill_id))
                    .collect::<Vec<_>>(),
            ),
            (Some(base), None) => Some(base),
            (None, Some(requested)) => Some(requested),
            (None, None) => None,
        };

        Ok(effective.map(Self::normalize_skill_id_list))
    }

    /// M12: Ensure MongoDB indexes for agent_sessions collection (chat track)
    pub async fn ensure_chat_indexes(&self) {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        let session_indexes = vec![
            // User session list query (sorted by last message time)
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "user_id": 1, "status": 1, "last_message_at": -1 })
                .build(),
            // User session list query with visibility filter
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "user_id": 1, "status": 1, "hidden_from_chat_list": 1, "last_message_at": -1 })
                .build(),
            // Filter by agent
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "agent_id": 1, "status": 1 })
                .build(),
            // Pinned + time sort
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "user_id": 1, "pinned": -1, "last_message_at": -1 })
                .build(),
            // Session lookup by session_id (unique)
            IndexModel::builder()
                .keys(doc! { "session_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        ];

        if let Err(e) = self.sessions().create_indexes(session_indexes, None).await {
            tracing::warn!("Failed to create chat session indexes: {}", e);
        } else {
            tracing::info!("Chat session indexes ensured");
        }

        let event_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "event_id": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "run_id": 1, "event_id": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "run_id": { "$exists": true } })
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "created_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "run_id": 1, "created_at": 1 })
                .build(),
        ];

        if let Err(e) = self.chat_events().create_indexes(event_indexes, None).await {
            tracing::warn!("Failed to create chat event indexes: {}", e);
        } else {
            tracing::info!("Chat event indexes ensured");
        }
    }

    fn agents(&self) -> mongodb::Collection<TeamAgentDoc> {
        self.db.collection("team_agents")
    }

    fn tasks(&self) -> mongodb::Collection<AgentTaskDoc> {
        self.db.collection("agent_tasks")
    }

    fn run_states(&self) -> mongodb::Collection<RunState> {
        self.db.collection("agent_run_states")
    }

    fn run_journal(&self) -> mongodb::Collection<RunJournal> {
        self.db.collection("agent_run_journal")
    }

    fn run_checkpoints(&self) -> mongodb::Collection<RunCheckpoint> {
        self.db.collection("agent_run_checkpoints")
    }

    fn subagent_runs(&self) -> mongodb::Collection<SubagentRun> {
        self.db.collection("agent_subagent_runs")
    }

    fn task_graphs(&self) -> mongodb::Collection<TaskGraph> {
        self.db.collection("agent_task_graphs")
    }

    pub async fn upsert_run_state(&self, state: &RunState) -> Result<(), mongodb::error::Error> {
        let mut state = state.clone();
        let now = bson::DateTime::now();
        if state.created_at.is_none() {
            state.created_at = Some(now);
        }
        state.updated_at = Some(now);
        self.run_states()
            .replace_one(
                doc! { "run_id": &state.run_id },
                state,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(())
    }

    pub async fn patch_run_state_after_turn(
        &self,
        run_id: &str,
        current_node_id: &str,
        status: RunStatus,
        memory: Option<&RunMemory>,
        outcome: &TurnOutcome,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        let mut set_doc = doc! {
            "current_node_id": current_node_id,
            "status": bson::to_bson(&status)
                .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
            "last_turn_outcome": bson::to_bson(outcome)
                .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
            "updated_at": now,
        };
        if let Some(memory) = memory {
            set_doc.insert(
                "memory",
                bson::to_bson(memory).map_err(|e| {
                    mongodb::error::Error::custom(format!("BSON serialize error: {}", e))
                })?,
            );
        }
        self.run_states()
            .update_one(doc! { "run_id": run_id }, doc! { "$set": set_doc }, None)
            .await?;
        Ok(())
    }

    pub async fn append_run_journal(
        &self,
        entries: &[RunJournal],
    ) -> Result<(), mongodb::error::Error> {
        if entries.is_empty() {
            return Ok(());
        }
        let now = bson::DateTime::now();
        let docs = entries
            .iter()
            .cloned()
            .map(|mut entry| {
                if entry.created_at.is_none() {
                    entry.created_at = Some(now);
                }
                entry
            })
            .collect::<Vec<_>>();
        self.run_journal().insert_many(docs, None).await?;
        Ok(())
    }

    pub async fn save_run_checkpoint(
        &self,
        checkpoint: &RunCheckpoint,
    ) -> Result<(), mongodb::error::Error> {
        let mut checkpoint = checkpoint.clone();
        if checkpoint.created_at.is_none() {
            checkpoint.created_at = Some(bson::DateTime::now());
        }
        self.run_checkpoints().insert_one(checkpoint, None).await?;
        Ok(())
    }

    pub async fn upsert_task_graph(&self, graph: &TaskGraph) -> Result<(), mongodb::error::Error> {
        let mut graph = graph.clone();
        let now = bson::DateTime::now();
        if graph.created_at.is_none() {
            graph.created_at = Some(now);
        }
        graph.updated_at = Some(now);
        self.task_graphs()
            .replace_one(
                doc! { "task_graph_id": &graph.task_graph_id },
                graph,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(())
    }

    pub async fn get_run_state(
        &self,
        run_id: &str,
    ) -> Result<Option<RunState>, mongodb::error::Error> {
        self.run_states()
            .find_one(doc! { "run_id": run_id }, None)
            .await
    }

    pub async fn ensure_run_checkpoint_exists(
        &self,
        run_id: &str,
        task_graph_id: Option<&str>,
        current_node_id: Option<&str>,
        status: RunStatus,
        lease: Option<&RunLease>,
        memory: Option<&RunMemory>,
    ) -> Result<bool, mongodb::error::Error> {
        let existing = self
            .run_checkpoints()
            .count_documents(doc! { "run_id": run_id }, None)
            .await?;
        if existing > 0 {
            return Ok(false);
        }
        self.save_run_checkpoint(&RunCheckpoint {
            id: None,
            run_id: run_id.to_string(),
            task_graph_id: task_graph_id.map(str::to_string),
            current_node_id: current_node_id.map(str::to_string),
            checkpoint_kind: RunCheckpointKind::NodeStart,
            status,
            lease: lease.cloned(),
            memory: memory.cloned(),
            last_turn_outcome: None,
            created_at: Some(bson::DateTime::now()),
        })
        .await?;
        Ok(true)
    }

    pub async fn get_task_graph(
        &self,
        task_graph_id: &str,
    ) -> Result<Option<TaskGraph>, mongodb::error::Error> {
        self.task_graphs()
            .find_one(doc! { "task_graph_id": task_graph_id }, None)
            .await
    }

    pub async fn upsert_subagent_run(
        &self,
        run: &SubagentRun,
    ) -> Result<(), mongodb::error::Error> {
        let mut run = run.clone();
        let now = bson::DateTime::now();
        if run.created_at.is_none() {
            run.created_at = Some(now);
        }
        run.updated_at = Some(now);
        self.subagent_runs()
            .replace_one(
                doc! { "subagent_run_id": &run.subagent_run_id },
                run,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(())
    }

    pub async fn mark_run_subagent_started(
        &self,
        parent_run_id: &str,
        subagent_run_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.mark_run_child_task_started(parent_run_id, subagent_run_id)
            .await?;
        self.run_states()
            .update_one(
                doc! { "run_id": parent_run_id },
                doc! {
                    "$addToSet": { "active_subagents": subagent_run_id },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn mark_run_child_task_started(
        &self,
        parent_run_id: &str,
        child_task_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.run_states()
            .update_one(
                doc! { "run_id": parent_run_id },
                doc! {
                    "$addToSet": { "active_child_tasks": child_task_id },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn mark_run_subagent_finished(
        &self,
        parent_run_id: &str,
        subagent_run_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.mark_run_child_task_finished(parent_run_id, subagent_run_id)
            .await?;
        self.run_states()
            .update_one(
                doc! { "run_id": parent_run_id },
                doc! {
                    "$pull": { "active_subagents": subagent_run_id },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn mark_run_child_task_finished(
        &self,
        parent_run_id: &str,
        child_task_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.run_states()
            .update_one(
                doc! { "run_id": parent_run_id },
                doc! {
                    "$pull": { "active_child_tasks": child_task_id },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    fn results(&self) -> mongodb::Collection<TaskResultDoc> {
        self.db.collection("agent_task_results")
    }

    fn sessions(&self) -> mongodb::Collection<AgentSessionDoc> {
        self.db.collection("agent_sessions")
    }

    fn chat_events(&self) -> mongodb::Collection<ChatEventDoc> {
        self.db.collection("agent_chat_events")
    }

    fn teams(&self) -> mongodb::Collection<Document> {
        self.db.collection("teams")
    }

    fn portals(&self) -> mongodb::Collection<Document> {
        self.db.collection(agime_team::db::collections::PORTALS)
    }

    fn documents_store(&self) -> mongodb::Collection<TeamDocument> {
        self.db.collection(agime_team::db::collections::DOCUMENTS)
    }

    fn avatar_instances(&self) -> mongodb::Collection<AvatarInstanceDoc> {
        self.db
            .collection(agime_team::db::collections::AVATAR_INSTANCES)
    }

    fn avatar_governance_states(&self) -> mongodb::Collection<AvatarGovernanceStateDoc> {
        self.db
            .collection(agime_team::db::collections::AVATAR_GOVERNANCE_STATES)
    }

    fn avatar_governance_events(&self) -> mongodb::Collection<AvatarGovernanceEventDoc> {
        self.db
            .collection(agime_team::db::collections::AVATAR_GOVERNANCE_EVENTS)
    }

    fn avatar_manager_reports(&self) -> mongodb::Collection<AvatarManagerReportDoc> {
        self.db
            .collection(agime_team::db::collections::AVATAR_MANAGER_REPORTS)
    }

    fn is_dedicated_avatar_marker(description: Option<&str>) -> bool {
        let desc = description.unwrap_or("").to_ascii_lowercase();
        desc.contains("[digital-avatar-manager]") || desc.contains("[digital-avatar-service]")
    }

    fn explicit_avatar_marker_role(description: Option<&str>) -> Option<&'static str> {
        let desc = description.unwrap_or("").to_ascii_lowercase();
        if desc.contains("[digital-avatar-manager]") {
            return Some("manager");
        }
        if desc.contains("[digital-avatar-service]") {
            return Some("service");
        }
        None
    }

    fn normalize_compact_text(value: Option<&str>) -> String {
        value
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect()
    }

    fn is_legacy_avatar_manager_candidate(agent: &TeamAgent) -> bool {
        if agent.agent_domain.as_deref() == Some("digital_avatar")
            && agent.agent_role.as_deref() == Some("manager")
        {
            return true;
        }
        if Self::is_dedicated_avatar_marker(agent.description.as_deref()) {
            return true;
        }
        let compact_name = Self::normalize_compact_text(Some(agent.name.as_str()));
        compact_name.contains("管理agent") || compact_name.contains("manageragent")
    }

    fn is_legacy_avatar_service_candidate(agent: &TeamAgent) -> bool {
        if agent.agent_domain.as_deref() == Some("digital_avatar")
            && agent.agent_role.as_deref() == Some("service")
        {
            return true;
        }
        if Self::is_dedicated_avatar_marker(agent.description.as_deref()) {
            return true;
        }
        let compact_name = Self::normalize_compact_text(Some(agent.name.as_str()));
        compact_name.ends_with("分身agent") || compact_name.contains("avataragent")
    }

    fn doc_string(doc: &Document, key: &str) -> Option<String> {
        doc.get(key).and_then(Bson::as_str).map(str::to_string)
    }

    fn nested_doc_string(doc: &Document, path: &[&str]) -> Option<String> {
        let mut current = doc;
        for key in &path[..path.len().saturating_sub(1)] {
            current = current.get_document(key).ok()?;
        }
        let leaf = *path.last()?;
        current.get(leaf).and_then(Bson::as_str).map(str::to_string)
    }

    fn doc_has_string(doc: &Document, key: &str, expected: &str) -> bool {
        Self::doc_string(doc, key)
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| value == expected)
    }

    fn nested_doc_has_string(doc: &Document, path: &[&str], expected: &str) -> bool {
        Self::nested_doc_string(doc, path)
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| value == expected)
    }

    fn doc_has_manager_tag(doc: &Document, expected: &str) -> bool {
        doc.get_array("tags")
            .ok()
            .map(|tags| {
                tags.iter()
                    .filter_map(Bson::as_str)
                    .map(str::trim)
                    .any(|tag| tag == format!("manager:{expected}"))
            })
            .unwrap_or(false)
    }

    fn extract_portal_manager_id(doc: &Document) -> Option<String> {
        Self::doc_string(doc, "coding_agent_id")
            .filter(|v| !v.trim().is_empty())
            .or_else(|| {
                Self::nested_doc_string(doc, &["settings", "managerAgentId"])
                    .filter(|v| !v.trim().is_empty())
            })
            .or_else(|| {
                Self::nested_doc_string(doc, &["settings", "managerGroupId"])
                    .filter(|v| !v.trim().is_empty())
            })
            .or_else(|| {
                doc.get_array("tags").ok().and_then(|tags| {
                    tags.iter()
                        .filter_map(Bson::as_str)
                        .map(str::trim)
                        .find_map(|tag| tag.strip_prefix("manager:").map(str::to_string))
                })
            })
            .or_else(|| Self::doc_string(doc, "agent_id").filter(|v| !v.trim().is_empty()))
    }

    fn count_pending_with_status(
        items: Option<&Vec<serde_json::Value>>,
        pending_statuses: &[&str],
    ) -> u32 {
        items.map_or(0, |entries| {
            entries
                .iter()
                .filter(|entry| {
                    entry
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|status| {
                            pending_statuses.iter().any(|expected| status == *expected)
                        })
                })
                .count() as u32
        })
    }

    fn avatar_type_from_doc(doc: &Document) -> String {
        Self::nested_doc_string(doc, &["settings", "avatarType"])
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                doc.get_array("tags").ok().and_then(|tags| {
                    if tags
                        .iter()
                        .filter_map(Bson::as_str)
                        .any(|tag| tag.eq_ignore_ascii_case("avatar:internal"))
                    {
                        Some("internal_worker".to_string())
                    } else if tags
                        .iter()
                        .filter_map(Bson::as_str)
                        .any(|tag| tag.eq_ignore_ascii_case("avatar:external"))
                    {
                        Some("external_service".to_string())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| "external_service".to_string())
    }

    fn default_governance_state_json() -> serde_json::Value {
        serde_json::json!({
            "capabilityRequests": [],
            "gapProposals": [],
            "optimizationTickets": [],
            "runtimeLogs": []
        })
    }

    fn default_governance_config_json() -> serde_json::Value {
        serde_json::json!({
            "autoProposalTriggerCount": 3,
            "managerApprovalMode": "manager_decides",
            "optimizationMode": "dual_loop",
            "lowRiskAction": "auto_execute",
            "mediumRiskAction": "manager_review",
            "highRiskAction": "human_review",
            "autoCreateCapabilityRequests": true,
            "autoCreateOptimizationTickets": true,
            "requireHumanForPublish": true
        })
    }

    fn normalize_governance_state_candidate(value: serde_json::Value) -> Option<serde_json::Value> {
        match value {
            serde_json::Value::Object(mut map) => {
                map.remove("config");
                if map.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::Object(map))
                }
            }
            serde_json::Value::Null => None,
            other => Some(other),
        }
    }

    fn raw_portal_governance_json(doc: &Document) -> Option<serde_json::Value> {
        doc.get_document("settings")
            .ok()
            .and_then(|settings| settings.get("digitalAvatarGovernance"))
            .and_then(|value| bson::from_bson::<serde_json::Value>(value.clone()).ok())
    }

    fn raw_portal_governance_config_top_level_json(doc: &Document) -> Option<serde_json::Value> {
        doc.get_document("settings")
            .ok()
            .and_then(|settings| settings.get("digitalAvatarGovernanceConfig"))
            .and_then(|value| bson::from_bson::<serde_json::Value>(value.clone()).ok())
    }

    fn governance_state_json_from_portal_doc(doc: &Document) -> serde_json::Value {
        Self::raw_portal_governance_json(doc)
            .and_then(Self::normalize_governance_state_candidate)
            .unwrap_or_else(Self::default_governance_state_json)
    }

    fn raw_portal_governance_state_json(doc: &Document) -> Option<serde_json::Value> {
        Self::raw_portal_governance_json(doc).and_then(Self::normalize_governance_state_candidate)
    }

    fn governance_config_json_from_portal_doc(doc: &Document) -> serde_json::Value {
        let from_top = Self::raw_portal_governance_config_top_level_json(doc);
        if let Some(value) = from_top {
            return value;
        }
        Self::raw_portal_governance_json(doc)
            .and_then(|governance| governance.get("config").cloned())
            .unwrap_or_else(Self::default_governance_config_json)
    }

    fn raw_portal_governance_config_json(doc: &Document) -> Option<serde_json::Value> {
        let from_top = Self::raw_portal_governance_config_top_level_json(doc);
        if from_top.is_some() {
            return from_top;
        }
        Self::raw_portal_governance_json(doc)
            .and_then(|governance| governance.get("config").cloned())
    }

    fn governance_counts_from_state_json(state: &serde_json::Value) -> AvatarGovernanceCounts {
        let capability_requests = state
            .get("capabilityRequests")
            .and_then(serde_json::Value::as_array)
            .cloned();
        let gap_proposals = state
            .get("gapProposals")
            .and_then(serde_json::Value::as_array)
            .cloned();
        let optimization_tickets = state
            .get("optimizationTickets")
            .and_then(serde_json::Value::as_array)
            .cloned();
        let runtime_logs = state
            .get("runtimeLogs")
            .and_then(serde_json::Value::as_array)
            .cloned();

        AvatarGovernanceCounts {
            pending_capability_requests: Self::count_pending_with_status(
                capability_requests.as_ref(),
                &["pending"],
            ),
            pending_gap_proposals: Self::count_pending_with_status(
                gap_proposals.as_ref(),
                &["pending_approval"],
            ),
            pending_optimization_tickets: Self::count_pending_with_status(
                optimization_tickets.as_ref(),
                &["pending"],
            ),
            pending_runtime_logs: Self::count_pending_with_status(
                runtime_logs.as_ref(),
                &["pending"],
            ),
        }
    }

    fn governance_counts_from_doc(doc: &Document) -> AvatarGovernanceCounts {
        Self::governance_counts_from_state_json(&Self::governance_state_json_from_portal_doc(doc))
    }

    fn default_avatar_governance_payload(
        team_id: &str,
        portal_id: &str,
    ) -> AvatarGovernanceStatePayload {
        AvatarGovernanceStatePayload {
            portal_id: portal_id.to_string(),
            team_id: team_id.to_string(),
            state: Self::default_governance_state_json(),
            config: Self::default_governance_config_json(),
            updated_at: Utc::now(),
        }
    }

    fn governance_payload_from_portal_doc(
        team_id: &str,
        portal_id: &str,
        portal_doc: &Document,
        updated_at: DateTime<Utc>,
    ) -> AvatarGovernanceStatePayload {
        AvatarGovernanceStatePayload {
            portal_id: portal_id.to_string(),
            team_id: team_id.to_string(),
            state: Self::governance_state_json_from_portal_doc(portal_doc),
            config: Self::governance_config_json_from_portal_doc(portal_doc),
            updated_at,
        }
    }

    async fn upsert_avatar_governance_state_payload(
        &self,
        payload: &AvatarGovernanceStatePayload,
    ) -> Result<(), mongodb::error::Error> {
        self.avatar_governance_states()
            .replace_one(
                doc! { "team_id": &payload.team_id, "portal_id": &payload.portal_id },
                AvatarGovernanceStateDoc {
                    id: None,
                    portal_id: payload.portal_id.clone(),
                    team_id: payload.team_id.clone(),
                    state: payload.state.clone(),
                    config: payload.config.clone(),
                    updated_at: payload.updated_at,
                },
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(())
    }

    async fn sync_avatar_governance_payload_to_portal(
        &self,
        payload: &AvatarGovernanceStatePayload,
    ) -> Result<Option<u64>, mongodb::error::Error> {
        let Ok(team_oid) = ObjectId::parse_str(&payload.team_id) else {
            return Ok(None);
        };
        let Ok(portal_oid) = ObjectId::parse_str(&payload.portal_id) else {
            return Ok(None);
        };

        let update_result = self
            .portals()
            .update_one(
                doc! {
                    "_id": portal_oid,
                    "team_id": team_oid,
                    "is_deleted": { "$ne": true }
                },
                doc! {
                    "$set": {
                        "settings.digitalAvatarGovernance": bson::to_bson(&payload.state).unwrap_or(Bson::Null),
                        "settings.digitalAvatarGovernanceConfig": bson::to_bson(&payload.config).unwrap_or(Bson::Null),
                        "updated_at": bson::DateTime::from_chrono(payload.updated_at),
                    }
                },
                None,
            )
            .await?;

        Ok(Some(update_result.matched_count))
    }

    async fn sync_avatar_governance_payload_to_portal_if_current(
        &self,
        payload: &AvatarGovernanceStatePayload,
        expected_updated_at: DateTime<Utc>,
    ) -> Result<Option<u64>, mongodb::error::Error> {
        let Ok(team_oid) = ObjectId::parse_str(&payload.team_id) else {
            return Ok(None);
        };
        let Ok(portal_oid) = ObjectId::parse_str(&payload.portal_id) else {
            return Ok(None);
        };

        let update_result = self
            .portals()
            .update_one(
                doc! {
                    "_id": portal_oid,
                    "team_id": team_oid,
                    "is_deleted": { "$ne": true },
                    "updated_at": bson::DateTime::from_chrono(expected_updated_at),
                },
                doc! {
                    "$set": {
                        "settings.digitalAvatarGovernance": bson::to_bson(&payload.state).unwrap_or(Bson::Null),
                        "settings.digitalAvatarGovernanceConfig": bson::to_bson(&payload.config).unwrap_or(Bson::Null),
                        "updated_at": bson::DateTime::from_chrono(payload.updated_at),
                    }
                },
                None,
            )
            .await?;

        Ok(Some(update_result.matched_count))
    }

    fn extract_portal_service_agent_id(doc: &Document) -> Option<String> {
        Self::doc_string(doc, "service_agent_id")
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                Self::nested_doc_string(doc, &["settings", "serviceRuntimeAgentId"])
                    .filter(|value| !value.trim().is_empty())
            })
            .or_else(|| Self::doc_string(doc, "agent_id").filter(|value| !value.trim().is_empty()))
    }

    fn normalize_optional_scope_id(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    fn index_governance_state_docs(
        docs: Vec<AvatarGovernanceStateDoc>,
    ) -> (
        HashMap<(String, String), AvatarGovernanceStateDoc>,
        HashMap<(String, String), u64>,
        Vec<AvatarGovernanceDuplicateStateRow>,
        u64,
    ) {
        let mut grouped: HashMap<(String, String), Vec<AvatarGovernanceStateDoc>> = HashMap::new();
        for doc in docs {
            grouped
                .entry((doc.team_id.clone(), doc.portal_id.clone()))
                .or_default()
                .push(doc);
        }

        let mut indexed = HashMap::new();
        let mut counts = HashMap::new();
        let mut duplicate_rows = Vec::new();
        let mut duplicate_state_docs = 0u64;

        for ((team_id, portal_id), mut entries) in grouped {
            entries.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
            let state_doc_count = entries.len() as u64;
            counts.insert((team_id.clone(), portal_id.clone()), state_doc_count);

            if state_doc_count > 1 {
                duplicate_state_docs += state_doc_count - 1;
                duplicate_rows.push(AvatarGovernanceDuplicateStateRow {
                    team_id: team_id.clone(),
                    portal_id: portal_id.clone(),
                    state_doc_count,
                    retained_updated_at: entries[0].updated_at.to_rfc3339(),
                    duplicate_updated_at: entries
                        .iter()
                        .skip(1)
                        .map(|doc| doc.updated_at.to_rfc3339())
                        .collect(),
                });
            }

            let retained = entries.remove(0);
            indexed.insert((team_id, portal_id), retained);
        }

        duplicate_rows.sort_by(|left, right| {
            left.team_id
                .cmp(&right.team_id)
                .then_with(|| left.portal_id.cmp(&right.portal_id))
        });

        (indexed, counts, duplicate_rows, duplicate_state_docs)
    }

    fn classify_governance_divergence(
        has_state_doc: bool,
        has_portal_governance: bool,
        state_matches_portal_state: bool,
        config_matches_portal_config: bool,
    ) -> AvatarGovernanceDivergenceKind {
        match (has_state_doc, has_portal_governance) {
            (false, false) => AvatarGovernanceDivergenceKind::DefaultOnly,
            (true, false) => AvatarGovernanceDivergenceKind::StateOnly,
            (false, true) => AvatarGovernanceDivergenceKind::SettingsOnly,
            (true, true) if state_matches_portal_state && config_matches_portal_config => {
                AvatarGovernanceDivergenceKind::InSync
            }
            _ => AvatarGovernanceDivergenceKind::Differing,
        }
    }

    async fn refresh_avatar_governance_state_read_model(
        &self,
        payload: &AvatarGovernanceStatePayload,
    ) -> Result<(), mongodb::error::Error> {
        self.upsert_avatar_governance_state_payload(payload).await
    }

    async fn refresh_avatar_governance_event_read_model(
        &self,
        team_id: &str,
        portal_id: &str,
        state: &serde_json::Value,
        config: &serde_json::Value,
    ) -> Result<u64, mongodb::error::Error> {
        self.seed_avatar_governance_events_if_missing(team_id, portal_id, state, config)
            .await
    }

    async fn persist_avatar_governance_events(
        &self,
        events: Vec<AvatarGovernanceEventDoc>,
    ) -> Result<u64, mongodb::error::Error> {
        if events.is_empty() {
            return Ok(0);
        }
        let inserted = events.len() as u64;
        let _ = self
            .avatar_governance_events()
            .insert_many(events, None)
            .await?;
        Ok(inserted)
    }

    async fn refresh_avatar_governance_projection_after_write(
        &self,
        payload: &AvatarGovernanceStatePayload,
    ) -> Result<Option<u64>, mongodb::error::Error> {
        let matched_portals = self
            .sync_avatar_governance_payload_to_portal(payload)
            .await?;
        let _ = self
            .sync_avatar_instance_projections(&payload.team_id)
            .await?;
        Ok(matched_portals)
    }

    fn derive_avatar_governance_payload_from_portal_doc(
        team_id: &str,
        portal_id: &str,
        portal_doc: &Document,
    ) -> AvatarGovernanceStatePayload {
        let updated_at = portal_doc
            .get_datetime("updated_at")
            .ok()
            .map(|value| value.to_chrono())
            .unwrap_or_else(Utc::now);
        Self::governance_payload_from_portal_doc(team_id, portal_id, portal_doc, updated_at)
    }

    fn sort_and_limit_avatar_governance_events(
        mut events: Vec<AvatarGovernanceEventPayload>,
        limit: usize,
    ) -> Vec<AvatarGovernanceEventPayload> {
        events.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.event_id.cmp(&left.event_id))
        });
        events.truncate(limit);
        events
    }

    async fn derive_avatar_governance_event_payloads(
        &self,
        team_id: &str,
        portal_id: &str,
    ) -> Result<Vec<AvatarGovernanceEventPayload>, mongodb::error::Error> {
        let payload = self.get_avatar_governance_state(team_id, portal_id).await?;
        Ok(Self::governance_event_payloads_from_snapshot(
            team_id,
            portal_id,
            &payload.state,
            &payload.config,
            payload.updated_at,
        ))
    }

    async fn derive_team_avatar_governance_event_payloads(
        &self,
        team_id: &str,
        portal_id: Option<&str>,
        excluded_portal_ids: &HashSet<String>,
    ) -> Result<Vec<AvatarGovernanceEventPayload>, mongodb::error::Error> {
        let scoped_portal_id = portal_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut resolved_portal_ids = excluded_portal_ids.clone();
        let mut derived = Vec::new();

        let mut state_filter = doc! { "team_id": team_id };
        if let Some(portal_id) = scoped_portal_id.as_deref() {
            state_filter.insert("portal_id", portal_id);
        }
        let state_docs: Vec<AvatarGovernanceStateDoc> = self
            .avatar_governance_states()
            .find(state_filter, None)
            .await?
            .try_collect()
            .await?;
        for state_doc in state_docs {
            if !resolved_portal_ids.insert(state_doc.portal_id.clone()) {
                continue;
            }
            derived.extend(Self::governance_event_payloads_from_snapshot(
                &state_doc.team_id,
                &state_doc.portal_id,
                &state_doc.state,
                &state_doc.config,
                state_doc.updated_at,
            ));
        }

        let team_oid = match ObjectId::parse_str(team_id) {
            Ok(value) => value,
            Err(_) => return Ok(derived),
        };
        let mut portal_filter = Self::avatar_portal_filter(Some(team_oid));
        if let Some(portal_id) = scoped_portal_id.as_deref() {
            let portal_oid = match ObjectId::parse_str(portal_id) {
                Ok(value) => value,
                Err(_) => return Ok(derived),
            };
            portal_filter.insert("_id", portal_oid);
        }
        let portal_docs: Vec<Document> = self
            .portals()
            .find(portal_filter, None)
            .await?
            .try_collect()
            .await?;
        for portal_doc in portal_docs {
            let Some(portal_oid) = portal_doc.get_object_id("_id").ok() else {
                continue;
            };
            let portal_id = portal_oid.to_hex();
            if !resolved_portal_ids.insert(portal_id.clone()) {
                continue;
            }
            let payload = Self::derive_avatar_governance_payload_from_portal_doc(
                team_id,
                &portal_id,
                &portal_doc,
            );
            derived.extend(Self::governance_event_payloads_from_snapshot(
                team_id,
                &portal_id,
                &payload.state,
                &payload.config,
                payload.updated_at,
            ));
        }

        Ok(derived)
    }

    fn binding_issue_message(
        issue: &AvatarBindingIssueKind,
        explicit_coding_agent_id: Option<&str>,
        explicit_service_agent_id: Option<&str>,
        effective_manager_agent_id: Option<&str>,
        effective_service_agent_id: Option<&str>,
        manager_agent: Option<&TeamAgentDoc>,
        service_agent: Option<&TeamAgentDoc>,
    ) -> String {
        match issue {
            AvatarBindingIssueKind::MissingExplicitManagerBinding => {
                "portal 缺少显式 coding_agent_id 绑定".to_string()
            }
            AvatarBindingIssueKind::MissingExplicitServiceBinding => {
                "portal 缺少显式 service_agent_id 绑定".to_string()
            }
            AvatarBindingIssueKind::MissingManagerBinding => {
                "无法解析有效 manager agent".to_string()
            }
            AvatarBindingIssueKind::MissingServiceBinding => {
                "无法解析有效 service agent".to_string()
            }
            AvatarBindingIssueKind::ManagerAgentNotFound => format!(
                "manager agent '{}' 在团队中不存在",
                effective_manager_agent_id.unwrap_or(explicit_coding_agent_id.unwrap_or(""))
            ),
            AvatarBindingIssueKind::ServiceAgentNotFound => format!(
                "service agent '{}' 在团队中不存在",
                effective_service_agent_id.unwrap_or(explicit_service_agent_id.unwrap_or(""))
            ),
            AvatarBindingIssueKind::ManagerRoleMismatch => format!(
                "manager agent '{}' 是 {}:{}，不是 digital_avatar:manager",
                effective_manager_agent_id.unwrap_or(explicit_coding_agent_id.unwrap_or("")),
                manager_agent
                    .and_then(|agent| agent.agent_domain.as_deref())
                    .unwrap_or("general"),
                manager_agent
                    .and_then(|agent| agent.agent_role.as_deref())
                    .unwrap_or("default")
            ),
            AvatarBindingIssueKind::ServiceRoleMismatch => format!(
                "service agent '{}' 是 {}:{}，不是 digital_avatar:service",
                effective_service_agent_id.unwrap_or(explicit_service_agent_id.unwrap_or("")),
                service_agent
                    .and_then(|agent| agent.agent_domain.as_deref())
                    .unwrap_or("general"),
                service_agent
                    .and_then(|agent| agent.agent_role.as_deref())
                    .unwrap_or("default")
            ),
            AvatarBindingIssueKind::OwnerManagerMismatch => format!(
                "service agent '{}' 的 owner_manager_agent_id 是 '{}'，但当前 manager agent 是 '{}'",
                effective_service_agent_id.unwrap_or(explicit_service_agent_id.unwrap_or("")),
                service_agent
                    .and_then(|agent| agent.owner_manager_agent_id.as_deref())
                    .unwrap_or(""),
                effective_manager_agent_id.unwrap_or(explicit_coding_agent_id.unwrap_or(""))
            ),
            AvatarBindingIssueKind::SameAgentReused => format!(
                "manager agent '{}' 和 service agent '{}' 指向了同一个 agent",
                effective_manager_agent_id.unwrap_or(explicit_coding_agent_id.unwrap_or("")),
                effective_service_agent_id.unwrap_or(explicit_service_agent_id.unwrap_or(""))
            ),
        }
    }

    fn binding_issue_messages(
        issues: &[AvatarBindingIssueKind],
        explicit_coding_agent_id: Option<&str>,
        explicit_service_agent_id: Option<&str>,
        effective_manager_agent_id: Option<&str>,
        effective_service_agent_id: Option<&str>,
        manager_agent: Option<&TeamAgentDoc>,
        service_agent: Option<&TeamAgentDoc>,
    ) -> Vec<String> {
        issues
            .iter()
            .map(|issue| {
                Self::binding_issue_message(
                    issue,
                    explicit_coding_agent_id,
                    explicit_service_agent_id,
                    effective_manager_agent_id,
                    effective_service_agent_id,
                    manager_agent,
                    service_agent,
                )
            })
            .collect()
    }

    fn governance_event_payloads_from_snapshot(
        team_id: &str,
        portal_id: &str,
        state: &serde_json::Value,
        config: &serde_json::Value,
        created_at: DateTime<Utc>,
    ) -> Vec<AvatarGovernanceEventPayload> {
        Self::diff_governance_events_at(
            portal_id,
            team_id,
            &Self::default_governance_state_json(),
            &Self::default_governance_config_json(),
            state,
            config,
            None,
            Some("system"),
            created_at,
        )
        .into_iter()
        .map(Into::into)
        .collect()
    }

    fn avatar_read_side_effect_audit_items() -> Vec<AvatarReadSideEffectAuditItem> {
        vec![
            AvatarReadSideEffectAuditItem {
                operation: "get_avatar_governance_state".to_string(),
                file: "crates/agime-team-server/src/agent/service_mongo.rs".to_string(),
                side_effects: vec![
                    "no database writes; derives payload from portal doc when state doc is missing"
                        .to_string(),
                ],
            },
            AvatarReadSideEffectAuditItem {
                operation: "list_avatar_governance_queue".to_string(),
                file: "crates/agime-team-server/src/agent/service_mongo.rs".to_string(),
                side_effects: vec![
                    "no database writes; reuses derived governance payload fallback".to_string(),
                ],
            },
            AvatarReadSideEffectAuditItem {
                operation: "list_avatar_instance_projections".to_string(),
                file: "crates/agime-team-server/src/agent/service_mongo.rs".to_string(),
                side_effects: vec![
                    "no database writes; derives projection rows from portals and governance state"
                        .to_string(),
                ],
            },
            AvatarReadSideEffectAuditItem {
                operation: "get_avatar_workbench_snapshot".to_string(),
                file: "crates/agime-team-server/src/agent/service_mongo.rs".to_string(),
                side_effects: vec![
                    "no database writes for derived reports; merges persisted non-derived reports in memory"
                        .to_string(),
                    "reads governance queues through derived state fallback without writing"
                        .to_string(),
                ],
            },
        ]
    }

    fn governance_entity_snapshots_from_state(
        state: &serde_json::Value,
    ) -> HashMap<String, GovernanceEntitySnapshot> {
        fn first_text(item: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> String {
            keys.iter()
                .find_map(|key| item.get(*key).and_then(serde_json::Value::as_str))
                .unwrap_or("")
                .trim()
                .to_string()
        }

        let mut entries = HashMap::new();
        let Some(root) = state.as_object() else {
            return entries;
        };

        let specs: [(&str, &str, &[&str]); 4] = [
            (
                "capabilityRequests",
                "capability",
                &["detail", "decisionReason", "requestedScope"],
            ),
            (
                "gapProposals",
                "proposal",
                &["description", "decisionReason", "expectedGain"],
            ),
            (
                "optimizationTickets",
                "ticket",
                &["proposal", "decisionReason", "evidence", "expectedGain"],
            ),
            (
                "runtimeLogs",
                "runtime",
                &["proposal", "evidence", "expectedGain"],
            ),
        ];

        for (array_key, entity_type, detail_keys) in specs {
            let Some(items) = root.get(array_key).and_then(serde_json::Value::as_array) else {
                continue;
            };
            for item in items {
                let Some(map) = item.as_object() else {
                    continue;
                };
                let id = map
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let title = map
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if id.is_empty() || title.is_empty() {
                    continue;
                }

                let status = map
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let detail = first_text(map, detail_keys);
                let key = format!("{entity_type}:{id}");
                entries.insert(
                    key,
                    GovernanceEntitySnapshot {
                        entity_type,
                        id,
                        title,
                        status,
                        detail,
                        meta: serde_json::Value::Object(map.clone()),
                    },
                );
            }
        }

        entries
    }

    fn diff_governance_events_at(
        portal_id: &str,
        team_id: &str,
        current_state: &serde_json::Value,
        current_config: &serde_json::Value,
        next_state: &serde_json::Value,
        next_config: &serde_json::Value,
        actor_id: Option<&str>,
        actor_name: Option<&str>,
        created_at: DateTime<Utc>,
    ) -> Vec<AvatarGovernanceEventDoc> {
        let current_entries = Self::governance_entity_snapshots_from_state(current_state);
        let next_entries = Self::governance_entity_snapshots_from_state(next_state);
        let mut events = Vec::new();

        for (key, next_entry) in &next_entries {
            match current_entries.get(key) {
                None => events.push(AvatarGovernanceEventDoc {
                    id: None,
                    portal_id: portal_id.to_string(),
                    team_id: team_id.to_string(),
                    event_type: "created".to_string(),
                    entity_type: next_entry.entity_type.to_string(),
                    entity_id: Some(next_entry.id.clone()),
                    title: next_entry.title.clone(),
                    status: Some(next_entry.status.clone()),
                    detail: (!next_entry.detail.is_empty()).then(|| next_entry.detail.clone()),
                    actor_id: actor_id.map(str::to_string),
                    actor_name: actor_name.map(str::to_string),
                    meta: next_entry.meta.clone(),
                    created_at,
                }),
                Some(current_entry)
                    if current_entry.status != next_entry.status
                        || current_entry.title != next_entry.title
                        || current_entry.detail != next_entry.detail
                        || current_entry.meta != next_entry.meta =>
                {
                    events.push(AvatarGovernanceEventDoc {
                        id: None,
                        portal_id: portal_id.to_string(),
                        team_id: team_id.to_string(),
                        event_type: "updated".to_string(),
                        entity_type: next_entry.entity_type.to_string(),
                        entity_id: Some(next_entry.id.clone()),
                        title: next_entry.title.clone(),
                        status: Some(next_entry.status.clone()),
                        detail: (!next_entry.detail.is_empty()).then(|| next_entry.detail.clone()),
                        actor_id: actor_id.map(str::to_string),
                        actor_name: actor_name.map(str::to_string),
                        meta: serde_json::json!({
                            "before": current_entry.meta,
                            "after": next_entry.meta,
                        }),
                        created_at,
                    });
                }
                Some(_) => {}
            }
        }

        for (key, current_entry) in &current_entries {
            if next_entries.contains_key(key) {
                continue;
            }
            events.push(AvatarGovernanceEventDoc {
                id: None,
                portal_id: portal_id.to_string(),
                team_id: team_id.to_string(),
                event_type: "removed".to_string(),
                entity_type: current_entry.entity_type.to_string(),
                entity_id: Some(current_entry.id.clone()),
                title: current_entry.title.clone(),
                status: Some(current_entry.status.clone()),
                detail: (!current_entry.detail.is_empty()).then(|| current_entry.detail.clone()),
                actor_id: actor_id.map(str::to_string),
                actor_name: actor_name.map(str::to_string),
                meta: current_entry.meta.clone(),
                created_at,
            });
        }

        if current_config != next_config {
            events.push(AvatarGovernanceEventDoc {
                id: None,
                portal_id: portal_id.to_string(),
                team_id: team_id.to_string(),
                event_type: "config_updated".to_string(),
                entity_type: "config".to_string(),
                entity_id: None,
                title: "治理配置已更新".to_string(),
                status: None,
                detail: None,
                actor_id: actor_id.map(str::to_string),
                actor_name: actor_name.map(str::to_string),
                meta: serde_json::json!({
                    "before": current_config,
                    "after": next_config,
                }),
                created_at,
            });
        }

        events
    }

    fn diff_governance_events(
        portal_id: &str,
        team_id: &str,
        current_state: &serde_json::Value,
        current_config: &serde_json::Value,
        next_state: &serde_json::Value,
        next_config: &serde_json::Value,
        actor_id: Option<&str>,
        actor_name: Option<&str>,
    ) -> Vec<AvatarGovernanceEventDoc> {
        Self::diff_governance_events_at(
            portal_id,
            team_id,
            current_state,
            current_config,
            next_state,
            next_config,
            actor_id,
            actor_name,
            Utc::now(),
        )
    }

    fn governance_queue_items_from_state(
        state: &serde_json::Value,
    ) -> Vec<AvatarGovernanceQueueItemPayload> {
        let mut rows = Vec::new();
        let Some(root) = state.as_object() else {
            return rows;
        };

        if let Some(items) = root
            .get("capabilityRequests")
            .and_then(serde_json::Value::as_array)
        {
            for item in items {
                let Some(map) = item.as_object() else {
                    continue;
                };
                let status = map
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("pending");
                if status != "pending" && status != "needs_human" {
                    continue;
                }
                let source_id = map
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let title = map
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if source_id.is_empty() || title.is_empty() {
                    continue;
                }
                let detail = map
                    .get("detail")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| {
                        map.get("decisionReason")
                            .and_then(serde_json::Value::as_str)
                    })
                    .unwrap_or("")
                    .to_string();
                let ts = map
                    .get("updatedAt")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| map.get("createdAt").and_then(serde_json::Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let mut meta = Vec::new();
                if let Some(risk) = map.get("risk").and_then(serde_json::Value::as_str) {
                    meta.push(risk.to_string());
                }
                if let Some(source) = map.get("source").and_then(serde_json::Value::as_str) {
                    meta.push(source.to_string());
                }
                rows.push(AvatarGovernanceQueueItemPayload {
                    id: format!("queue:capability:{source_id}"),
                    kind: "capability".to_string(),
                    title,
                    detail,
                    status: status.to_string(),
                    ts,
                    meta,
                    source_id,
                });
            }
        }

        if let Some(items) = root
            .get("gapProposals")
            .and_then(serde_json::Value::as_array)
        {
            for item in items {
                let Some(map) = item.as_object() else {
                    continue;
                };
                let status = map
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("draft");
                if status != "pending_approval" && status != "approved" && status != "pilot" {
                    continue;
                }
                let source_id = map
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let title = map
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if source_id.is_empty() || title.is_empty() {
                    continue;
                }
                let detail = map
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let ts = map
                    .get("updatedAt")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| map.get("createdAt").and_then(serde_json::Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let meta = map
                    .get("expectedGain")
                    .and_then(serde_json::Value::as_str)
                    .map(|value| vec![value.to_string()])
                    .unwrap_or_default();
                let mut meta = meta;
                if let Some(reason) = map
                    .get("decisionReason")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                {
                    meta.push(format!("决策说明: {}", reason.trim()));
                }
                rows.push(AvatarGovernanceQueueItemPayload {
                    id: format!("queue:proposal:{source_id}"),
                    kind: "proposal".to_string(),
                    title,
                    detail,
                    status: status.to_string(),
                    ts,
                    meta,
                    source_id,
                });
            }
        }

        if let Some(items) = root
            .get("optimizationTickets")
            .and_then(serde_json::Value::as_array)
        {
            for item in items {
                let Some(map) = item.as_object() else {
                    continue;
                };
                let status = map
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("pending");
                if status != "pending" && status != "approved" && status != "experimenting" {
                    continue;
                }
                let source_id = map
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let title = map
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if source_id.is_empty() || title.is_empty() {
                    continue;
                }
                let detail = map
                    .get("proposal")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| map.get("evidence").and_then(serde_json::Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let ts = map
                    .get("updatedAt")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| map.get("createdAt").and_then(serde_json::Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let mut meta = Vec::new();
                if let Some(problem_type) =
                    map.get("problemType").and_then(serde_json::Value::as_str)
                {
                    meta.push(problem_type.to_string());
                }
                if let Some(risk) = map.get("risk").and_then(serde_json::Value::as_str) {
                    meta.push(risk.to_string());
                }
                if let Some(reason) = map
                    .get("decisionReason")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                {
                    meta.push(format!("决策说明: {}", reason.trim()));
                }
                rows.push(AvatarGovernanceQueueItemPayload {
                    id: format!("queue:ticket:{source_id}"),
                    kind: "ticket".to_string(),
                    title,
                    detail,
                    status: status.to_string(),
                    ts,
                    meta,
                    source_id,
                });
            }
        }

        rows.sort_by(|a, b| b.ts.cmp(&a.ts));
        rows
    }

    fn avatar_portal_filter(team_oid: Option<ObjectId>) -> Document {
        let mut filter = doc! {
            "is_deleted": { "$ne": true },
            "$or": [
                { "domain": "avatar" },
                { "tags": "digital-avatar" },
                { "tags": { "$regex": "^avatar:", "$options": "i" } }
            ]
        };
        if let Some(value) = team_oid {
            filter.insert("team_id", value);
        }
        filter
    }

    fn is_avatar_portal_doc(doc: &Document) -> bool {
        if Self::doc_has_string(doc, "domain", "avatar")
            || Self::nested_doc_has_string(doc, &["settings", "domain"], "avatar")
        {
            return true;
        }

        doc.get_array("tags").ok().is_some_and(|tags| {
            tags.iter()
                .filter_map(Bson::as_str)
                .map(str::trim)
                .any(|tag| {
                    tag.eq_ignore_ascii_case("digital-avatar")
                        || tag.to_ascii_lowercase().starts_with("avatar:")
                })
        })
    }

    async fn seed_avatar_governance_events_if_missing(
        &self,
        team_id: &str,
        portal_id: &str,
        state: &serde_json::Value,
        config: &serde_json::Value,
    ) -> Result<u64, mongodb::error::Error> {
        let existing = self
            .avatar_governance_events()
            .count_documents(doc! { "team_id": team_id, "portal_id": portal_id }, None)
            .await?;
        if existing > 0 {
            return Ok(0);
        }

        let events = Self::diff_governance_events(
            portal_id,
            team_id,
            &Self::default_governance_state_json(),
            &Self::default_governance_config_json(),
            state,
            config,
            None,
            Some("system"),
        );
        if events.is_empty() {
            return Ok(0);
        }

        self.persist_avatar_governance_events(events).await
    }

    fn avatar_instance_from_portal_doc(team_id: &str, doc: &Document) -> Option<AvatarInstanceDoc> {
        let portal_id = doc.get_object_id("_id").ok()?.to_hex();
        let slug = Self::doc_string(doc, "slug")?;
        let name = Self::doc_string(doc, "name")?;
        let status = Self::doc_string(doc, "status").unwrap_or_else(|| "draft".to_string());
        let document_access_mode = Self::doc_string(doc, "document_access_mode")
            .unwrap_or_else(|| "read_only".to_string());
        let portal_updated_at = doc
            .get_datetime("updated_at")
            .ok()
            .map(|value| value.to_chrono())
            .unwrap_or_else(Utc::now);

        Some(AvatarInstanceDoc {
            id: None,
            portal_id,
            team_id: team_id.to_string(),
            slug,
            name,
            status,
            avatar_type: Self::avatar_type_from_doc(doc),
            manager_agent_id: Self::extract_portal_manager_id(doc),
            service_agent_id: Self::extract_portal_service_agent_id(doc),
            document_access_mode,
            governance_counts: Self::governance_counts_from_doc(doc),
            portal_updated_at,
            projected_at: Utc::now(),
        })
    }

    async fn infer_legacy_avatar_metadata(
        &self,
        team_id: &str,
        agent: &TeamAgent,
    ) -> Result<Option<(String, Option<String>)>, mongodb::error::Error> {
        let manager_candidate = Self::is_legacy_avatar_manager_candidate(agent);
        let service_candidate = Self::is_legacy_avatar_service_candidate(agent);
        if !manager_candidate && !service_candidate {
            return Ok(None);
        }

        let team_oid = match ObjectId::parse_str(team_id) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        let agent_id = agent.id.trim();
        if agent_id.is_empty() {
            return Ok(None);
        }

        let binding_filter = doc! {
            "$or": [
                { "coding_agent_id": agent_id },
                { "service_agent_id": agent_id },
                { "agent_id": agent_id },
                { "tags": format!("manager:{agent_id}") },
                { "settings.managerAgentId": agent_id },
                { "settings.managerGroupId": agent_id },
                { "settings.serviceRuntimeAgentId": agent_id }
            ]
        };
        let filter = doc! {
            "$and": [
                Self::avatar_portal_filter(Some(team_oid)),
                binding_filter,
            ]
        };

        let mut cursor = self.portals().find(filter, None).await?;
        let mut manager_match = false;
        let mut service_owner_manager_id: Option<String> = None;

        while let Some(portal) = cursor.try_next().await? {
            let explicit_manager_match = Self::doc_has_string(&portal, "coding_agent_id", agent_id)
                || Self::doc_has_manager_tag(&portal, agent_id)
                || Self::nested_doc_has_string(&portal, &["settings", "managerAgentId"], agent_id)
                || Self::nested_doc_has_string(&portal, &["settings", "managerGroupId"], agent_id);
            let explicit_service_match =
                Self::doc_has_string(&portal, "service_agent_id", agent_id)
                    || Self::nested_doc_has_string(
                        &portal,
                        &["settings", "serviceRuntimeAgentId"],
                        agent_id,
                    );
            let legacy_single_agent_match = Self::doc_has_string(&portal, "agent_id", agent_id);

            if manager_candidate && (explicit_manager_match || legacy_single_agent_match) {
                manager_match = true;
                break;
            }

            if service_candidate && (explicit_service_match || legacy_single_agent_match) {
                service_owner_manager_id = Self::extract_portal_manager_id(&portal)
                    .filter(|value| value.trim() != agent_id);
            }
        }

        if manager_match {
            return Ok(Some(("manager".to_string(), None)));
        }
        if service_candidate && service_owner_manager_id.is_some() {
            return Ok(Some(("service".to_string(), service_owner_manager_id)));
        }
        if service_candidate {
            let has_orphan_service_hint =
                Self::is_dedicated_avatar_marker(agent.description.as_deref())
                    || Self::normalize_compact_text(Some(agent.name.as_str()))
                        .ends_with("分身agent");
            if has_orphan_service_hint {
                return Ok(Some(("service".to_string(), None)));
            }
        }

        Ok(None)
    }

    async fn backfill_legacy_avatar_agent_metadata(
        &self,
        agent: &mut TeamAgent,
    ) -> Result<(), mongodb::error::Error> {
        let explicit_role = Self::explicit_avatar_marker_role(agent.description.as_deref());
        if agent.agent_domain.as_deref() == Some("digital_avatar") && agent.agent_role.is_some() {
            if explicit_role.is_none() || explicit_role == agent.agent_role.as_deref() {
                return Ok(());
            }
        }

        let inferred = self
            .infer_legacy_avatar_metadata(&agent.team_id, agent)
            .await?;
        let (role, owner_manager_agent_id) = match explicit_role {
            Some("manager") => ("manager".to_string(), None),
            Some("service") => inferred.unwrap_or_else(|| ("service".to_string(), None)),
            Some(other) => (other.to_string(), None),
            None => {
                let Some(pair) = inferred else {
                    return Ok(());
                };
                pair
            }
        };

        let mut set_doc = doc! {
            "agent_domain": "digital_avatar",
            "agent_role": role.clone(),
            "updated_at": bson::DateTime::from_chrono(Utc::now()),
        };
        match owner_manager_agent_id.clone() {
            Some(value) => {
                set_doc.insert("owner_manager_agent_id", value);
            }
            None => {
                set_doc.insert("owner_manager_agent_id", Bson::Null);
            }
        }

        self.agents()
            .update_one(
                doc! { "agent_id": &agent.id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;

        agent.agent_domain = Some("digital_avatar".to_string());
        agent.agent_role = Some(role);
        agent.owner_manager_agent_id = owner_manager_agent_id;
        Ok(())
    }

    /// Whether an agent is considered a digital-avatar dedicated agent and therefore
    /// must not be reused as a generic provision template.
    pub async fn is_dedicated_avatar_agent(
        &self,
        team_id: &str,
        agent: &TeamAgent,
    ) -> Result<bool, mongodb::error::Error> {
        if agent.agent_domain.as_deref() == Some("digital_avatar") {
            return Ok(true);
        }
        if Self::is_legacy_avatar_manager_candidate(agent)
            || Self::is_legacy_avatar_service_candidate(agent)
        {
            return Ok(self
                .infer_legacy_avatar_metadata(team_id, agent)
                .await?
                .is_some());
        }
        Ok(false)
    }

    // Permission checks - query embedded members array in teams collection
    pub async fn is_team_member(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        // team_id is ObjectId string
        let oid = match mongodb::bson::oid::ObjectId::parse_str(team_id) {
            Ok(oid) => oid,
            Err(_) => return Ok(false),
        };
        let result = self
            .teams()
            .find_one(
                doc! {
                    "_id": oid,
                    "members.user_id": user_id
                },
                None,
            )
            .await?;
        Ok(result.is_some())
    }

    pub async fn is_team_admin(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        // team_id is ObjectId string
        let oid = match mongodb::bson::oid::ObjectId::parse_str(team_id) {
            Ok(oid) => oid,
            Err(_) => return Ok(false),
        };
        let result = self
            .teams()
            .find_one(
                doc! {
                    "_id": oid,
                    "members": {
                        "$elemMatch": {
                            "user_id": user_id,
                            "role": { "$in": ["admin", "owner"] }
                        }
                    }
                },
                None,
            )
            .await?;
        Ok(result.is_some())
    }

    pub async fn get_agent_team_id(
        &self,
        agent_id: &str,
    ) -> Result<Option<String>, mongodb::error::Error> {
        let result = self
            .agents()
            .find_one(doc! { "agent_id": agent_id }, None)
            .await?;
        Ok(result.map(|r| r.team_id))
    }

    pub async fn get_task_team_id(
        &self,
        task_id: &str,
    ) -> Result<Option<String>, mongodb::error::Error> {
        let result = self
            .tasks()
            .find_one(doc! { "task_id": task_id }, None)
            .await?;
        Ok(result.map(|r| r.team_id))
    }

    // Agent CRUD operations
    pub async fn create_agent(&self, req: CreateAgentRequest) -> Result<TeamAgent, ServiceError> {
        validate_name(&req.name)?;
        validate_api_url(&req.api_url)?;
        validate_model(&req.model)?;

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let api_format = req.api_format.as_deref().unwrap_or("openai");
        let enabled_extensions =
            sanitize_enabled_extensions(req.enabled_extensions.unwrap_or_else(|| {
                BuiltinExtension::defaults()
                    .into_iter()
                    .map(|ext| AgentExtensionConfig {
                        extension: ext,
                        enabled: true,
                        allowed_groups: Vec::new(),
                    })
                    .collect()
            }));
        let attached_team_extensions =
            sanitize_attached_team_extensions(req.attached_team_extensions.unwrap_or_default());

        let doc = TeamAgentDoc {
            id: None,
            agent_id: id.clone(),
            team_id: req.team_id,
            name: req.name,
            description: req.description,
            avatar: req.avatar,
            system_prompt: req.system_prompt,
            api_url: req.api_url,
            model: req.model,
            api_key: req.api_key,
            api_format: api_format.to_string(),
            status: "idle".to_string(),
            last_error: None,
            enabled_extensions,
            custom_extensions: req.custom_extensions.unwrap_or_default(),
            agent_domain: req.agent_domain,
            agent_role: req.agent_role,
            owner_manager_agent_id: req.owner_manager_agent_id,
            template_source_agent_id: req.template_source_agent_id,
            allowed_groups: req.allowed_groups.unwrap_or_default(),
            max_concurrent_tasks: normalize_max_concurrent_tasks(req.max_concurrent_tasks),
            active_execution_slots: 0,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            context_limit: req.context_limit,
            thinking_enabled: req.thinking_enabled.unwrap_or(true),
            thinking_budget: req.thinking_budget,
            reasoning_effort: req.reasoning_effort,
            output_reserve_tokens: req.output_reserve_tokens,
            auto_compact_threshold: req.auto_compact_threshold,
            supports_multimodal: req.supports_multimodal.unwrap_or(false),
            prompt_caching_mode: req.prompt_caching_mode.unwrap_or_default(),
            cache_edit_mode: req.cache_edit_mode.unwrap_or_default(),
            assigned_skills: sanitize_assigned_skills(req.assigned_skills.unwrap_or_default()),
            skill_binding_mode: req.skill_binding_mode.unwrap_or_default(),
            delegation_policy: req.delegation_policy.unwrap_or_default(),
            attached_team_extensions,
            auto_approve_chat: true,
            created_at: now,
            updated_at: now,
        };

        // Insert via raw BSON document to preserve custom extension envs.
        let mut insert_doc = mongodb::bson::to_document(&doc)
            .map_err(|e| ServiceError::Internal(format!("Serialize agent doc failed: {}", e)))?;
        insert_doc.insert(
            "custom_extensions",
            custom_extensions_to_bson(&doc.custom_extensions),
        );
        self.db
            .collection::<Document>("team_agents")
            .insert_one(insert_doc, None)
            .await?;
        self.get_agent(&id)
            .await?
            .ok_or_else(|| ServiceError::Internal(format!("Agent not found after insert: {}", id)))
    }

    pub async fn get_agent(&self, id: &str) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let doc = self
            .agents()
            .find_one(doc! { "agent_id": id }, None)
            .await?;
        let Some(doc) = doc else {
            return Ok(None);
        };
        let mut agent: TeamAgent = doc.into();
        self.backfill_legacy_avatar_agent_metadata(&mut agent)
            .await?;
        Ok(Some(agent))
    }

    /// Get agent with API key preserved (for internal server-side use only, never expose to API).
    pub async fn get_agent_with_key(
        &self,
        id: &str,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let doc = self
            .agents()
            .find_one(doc! { "agent_id": id, "is_deleted": { "$ne": true } }, None)
            .await?;
        let Some(doc) = doc else {
            return Ok(None);
        };
        let mut agent = agent_doc_with_key(doc);
        self.backfill_legacy_avatar_agent_metadata(&mut agent)
            .await?;
        Ok(Some(agent))
    }

    /// Get the first agent for a team that has an API key configured (for internal server-side use).
    pub async fn get_first_agent_with_key(
        &self,
        team_id: &str,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let options = mongodb::options::FindOneOptions::builder()
            .sort(doc! { "created_at": -1 })
            .build();
        let doc = self
            .agents()
            .find_one(
                doc! {
                    "team_id": team_id,
                    "is_deleted": { "$ne": true },
                    "api_key": { "$exists": true, "$nin": [null, ""] }
                },
                options,
            )
            .await?;
        Ok(doc.map(agent_doc_with_key))
    }

    /// Pick the preferred default AI-describe provider agent for a team.
    ///
    /// Preference order:
    /// 1. Oldest non-digital-avatar agent with an API key
    /// 2. Oldest digital-avatar manager agent with an API key
    /// 3. Oldest remaining non-deleted agent with an API key
    pub async fn get_default_ai_describe_agent_with_key(
        &self,
        team_id: &str,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "created_at": 1 })
            .limit(64)
            .build();
        let cursor = self
            .agents()
            .find(
                doc! {
                    "team_id": team_id,
                    "is_deleted": { "$ne": true },
                    "api_key": { "$exists": true, "$nin": [null, ""] }
                },
                options,
            )
            .await?;
        let docs: Vec<TeamAgentDoc> = cursor.try_collect().await?;
        let mut agents: Vec<TeamAgent> = Vec::with_capacity(docs.len());
        for doc in docs {
            let mut agent = agent_doc_with_key(doc);
            self.backfill_legacy_avatar_agent_metadata(&mut agent)
                .await?;
            agents.push(agent);
        }

        if let Some(agent) = agents
            .iter()
            .find(|agent| agent.agent_domain.as_deref() != Some("digital_avatar"))
            .cloned()
        {
            return Ok(Some(agent));
        }

        if let Some(agent) = agents
            .iter()
            .find(|agent| {
                agent.agent_domain.as_deref() == Some("digital_avatar")
                    && agent.agent_role.as_deref() == Some("manager")
            })
            .cloned()
        {
            return Ok(Some(agent));
        }

        Ok(agents.into_iter().next())
    }

    pub async fn list_agents(
        &self,
        query: ListAgentsQuery,
    ) -> Result<PaginatedResponse<TeamAgent>, mongodb::error::Error> {
        let clamped_limit = query.limit.min(100);
        let limit = clamped_limit as i64;
        let skip = ((query.page.saturating_sub(1)) * clamped_limit) as u64;

        let filter = doc! { "team_id": &query.team_id };
        let total = self.agents().count_documents(filter.clone(), None).await?;

        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "created_at": -1 })
            .skip(skip)
            .limit(limit)
            .build();
        let cursor = self.agents().find(filter, options).await?;

        let docs: Vec<TeamAgentDoc> = cursor.try_collect().await?;
        let mut items: Vec<TeamAgent> = docs.into_iter().map(|d| d.into()).collect();
        for agent in &mut items {
            self.backfill_legacy_avatar_agent_metadata(agent).await?;
        }

        Ok(PaginatedResponse::new(
            items,
            total,
            query.page,
            query.limit,
        ))
    }

    pub async fn sync_avatar_instance_projections(
        &self,
        team_id: &str,
    ) -> Result<Vec<AvatarInstanceSummary>, mongodb::error::Error> {
        let projection_docs = self.derive_avatar_instance_projection_docs(team_id).await?;
        let mut projections = Vec::with_capacity(projection_docs.len());
        for projection in projection_docs {
            let portal_id = projection.portal_id.clone();
            self.avatar_instances()
                .replace_one(
                    doc! { "portal_id": &portal_id, "team_id": team_id },
                    projection.clone(),
                    mongodb::options::ReplaceOptions::builder()
                        .upsert(true)
                        .build(),
                )
                .await?;
            projections.push(AvatarInstanceSummary::from(projection));
        }

        Ok(projections)
    }

    async fn derive_avatar_instance_projection_docs(
        &self,
        team_id: &str,
    ) -> Result<Vec<AvatarInstanceDoc>, mongodb::error::Error> {
        let team_oid = match ObjectId::parse_str(team_id) {
            Ok(v) => v,
            Err(_) => return Ok(Vec::new()),
        };

        let filter = Self::avatar_portal_filter(Some(team_oid));
        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .build();
        let cursor = self.portals().find(filter, options).await?;
        let portal_docs: Vec<Document> = cursor.try_collect().await?;
        let governance_docs: Vec<AvatarGovernanceStateDoc> = self
            .avatar_governance_states()
            .find(doc! { "team_id": team_id }, None)
            .await?
            .try_collect()
            .await?;
        let governance_counts_by_portal: std::collections::HashMap<String, AvatarGovernanceCounts> =
            governance_docs
                .into_iter()
                .map(|doc| {
                    (
                        doc.portal_id,
                        Self::governance_counts_from_state_json(&doc.state),
                    )
                })
                .collect();
        let mut projections = Vec::with_capacity(portal_docs.len());

        for portal_doc in portal_docs {
            let Some(mut projection) = Self::avatar_instance_from_portal_doc(team_id, &portal_doc)
            else {
                continue;
            };
            if let Some(counts) = governance_counts_by_portal.get(&projection.portal_id) {
                projection.governance_counts = counts.clone();
            }
            projections.push(projection);
        }

        Ok(projections)
    }

    pub async fn get_avatar_governance_state(
        &self,
        team_id: &str,
        portal_id: &str,
    ) -> Result<AvatarGovernanceStatePayload, mongodb::error::Error> {
        if let Some(doc) = self
            .avatar_governance_states()
            .find_one(doc! { "team_id": team_id, "portal_id": portal_id }, None)
            .await?
        {
            return Ok(doc.into());
        }

        let portal_oid = match ObjectId::parse_str(portal_id) {
            Ok(value) => value,
            Err(_) => {
                tracing::warn!(
                    team_id = %team_id,
                    portal_id = %portal_id,
                    "avatar governance requested with invalid portal id; returning default payload"
                );
                return Ok(Self::default_avatar_governance_payload(team_id, portal_id));
            }
        };

        let team_oid = match ObjectId::parse_str(team_id) {
            Ok(value) => value,
            Err(_) => {
                tracing::warn!(
                    team_id = %team_id,
                    portal_id = %portal_id,
                    "avatar governance requested with invalid team id; returning default payload"
                );
                return Ok(Self::default_avatar_governance_payload(team_id, portal_id));
            }
        };

        let Some(portal_doc) = self
            .portals()
            .find_one(
                doc! {
                    "_id": portal_oid,
                    "team_id": team_oid,
                    "is_deleted": { "$ne": true }
                },
                None,
            )
            .await?
        else {
            tracing::warn!(
                team_id = %team_id,
                portal_id = %portal_id,
                "avatar governance requested for missing portal; returning default payload"
            );
            return Ok(Self::default_avatar_governance_payload(team_id, portal_id));
        };

        if !Self::is_avatar_portal_doc(&portal_doc) {
            tracing::warn!(
                team_id = %team_id,
                portal_id = %portal_id,
                "avatar governance requested for non-avatar portal; continuing with compatibility fallback"
            );
        }

        Ok(Self::derive_avatar_governance_payload_from_portal_doc(
            team_id,
            portal_id,
            &portal_doc,
        ))
    }

    pub async fn update_avatar_governance_state(
        &self,
        team_id: &str,
        portal_id: &str,
        next_state: Option<serde_json::Value>,
        next_config: Option<serde_json::Value>,
        actor_id: Option<&str>,
        actor_name: Option<&str>,
    ) -> Result<AvatarGovernanceStatePayload, mongodb::error::Error> {
        let current = self.get_avatar_governance_state(team_id, portal_id).await?;
        let previous_state = current.state.clone();
        let previous_config = current.config.clone();
        let state = next_state.unwrap_or(previous_state.clone());
        let config = next_config.unwrap_or(previous_config.clone());
        let updated_at = Utc::now();
        let payload = AvatarGovernanceStatePayload {
            portal_id: portal_id.to_string(),
            team_id: team_id.to_string(),
            state: state.clone(),
            config: config.clone(),
            updated_at,
        };
        let governance_events = Self::diff_governance_events(
            portal_id,
            team_id,
            &previous_state,
            &previous_config,
            &state,
            &config,
            actor_id,
            actor_name,
        );

        self.refresh_avatar_governance_state_read_model(&payload)
            .await?;

        let matched_portals = self
            .refresh_avatar_governance_projection_after_write(&payload)
            .await?;
        if matches!(matched_portals, Some(0)) {
            tracing::warn!(
                team_id = %team_id,
                portal_id = %portal_id,
                "avatar governance state updated without a matching portal document; governance projection may be orphaned"
            );
        }

        if !governance_events.is_empty() {
            let _ = self
                .persist_avatar_governance_events(governance_events)
                .await?;
        }

        Ok(payload)
    }

    pub async fn update_avatar_governance_state_if_current(
        &self,
        current: &AvatarGovernanceStatePayload,
        next_state: Option<serde_json::Value>,
        next_config: Option<serde_json::Value>,
        actor_id: Option<&str>,
        actor_name: Option<&str>,
    ) -> Result<Option<AvatarGovernanceStatePayload>, mongodb::error::Error> {
        let previous_state = current.state.clone();
        let previous_config = current.config.clone();
        let state = next_state.unwrap_or(previous_state.clone());
        let config = next_config.unwrap_or(previous_config.clone());
        let updated_at = Utc::now();
        let payload = AvatarGovernanceStatePayload {
            portal_id: current.portal_id.clone(),
            team_id: current.team_id.clone(),
            state: state.clone(),
            config: config.clone(),
            updated_at,
        };
        let governance_events = Self::diff_governance_events(
            &current.portal_id,
            &current.team_id,
            &previous_state,
            &previous_config,
            &state,
            &config,
            actor_id,
            actor_name,
        );

        let matched_portals = self
            .sync_avatar_governance_payload_to_portal_if_current(&payload, current.updated_at)
            .await?;
        if matches!(matched_portals, Some(0)) {
            return Ok(None);
        }

        self.refresh_avatar_governance_state_read_model(&payload)
            .await?;

        let _ = self
            .sync_avatar_instance_projections(&payload.team_id)
            .await?;

        if !governance_events.is_empty() {
            let _ = self
                .persist_avatar_governance_events(governance_events)
                .await?;
        }

        Ok(Some(payload))
    }

    pub async fn list_avatar_governance_events(
        &self,
        team_id: &str,
        portal_id: &str,
        limit: u32,
    ) -> Result<Vec<AvatarGovernanceEventPayload>, mongodb::error::Error> {
        let limit = limit.clamp(1, 200) as usize;
        let docs: Vec<AvatarGovernanceEventDoc> = self
            .avatar_governance_events()
            .find(
                doc! { "team_id": team_id, "portal_id": portal_id },
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "created_at": -1, "_id": -1 })
                    .limit(limit as i64)
                    .build(),
            )
            .await?
            .try_collect()
            .await?;

        if !docs.is_empty() {
            return Ok(Self::sort_and_limit_avatar_governance_events(
                docs.into_iter().map(Into::into).collect(),
                limit,
            ));
        }

        Ok(Self::sort_and_limit_avatar_governance_events(
            self.derive_avatar_governance_event_payloads(team_id, portal_id)
                .await?,
            limit,
        ))
    }

    pub async fn list_team_avatar_governance_events(
        &self,
        team_id: &str,
        portal_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<AvatarGovernanceEventPayload>, mongodb::error::Error> {
        let limit = limit.clamp(1, 500) as usize;
        let mut filter = doc! { "team_id": team_id };
        if let Some(value) = portal_id.filter(|value| !value.trim().is_empty()) {
            filter.insert("portal_id", value);
        }

        let docs: Vec<AvatarGovernanceEventDoc> = self
            .avatar_governance_events()
            .find(
                filter,
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "created_at": -1, "_id": -1 })
                    .limit(limit as i64)
                    .build(),
            )
            .await?
            .try_collect()
            .await?;
        let mut events: Vec<AvatarGovernanceEventPayload> =
            docs.into_iter().map(Into::into).collect();
        if events.len() < limit {
            let excluded_portal_ids = events
                .iter()
                .map(|event| event.portal_id.clone())
                .collect::<HashSet<_>>();
            events.extend(
                self.derive_team_avatar_governance_event_payloads(
                    team_id,
                    portal_id,
                    &excluded_portal_ids,
                )
                .await?,
            );
        }

        Ok(Self::sort_and_limit_avatar_governance_events(events, limit))
    }

    pub async fn list_avatar_governance_queue(
        &self,
        team_id: &str,
        portal_id: &str,
    ) -> Result<Vec<AvatarGovernanceQueueItemPayload>, mongodb::error::Error> {
        let payload = self.get_avatar_governance_state(team_id, portal_id).await?;
        Ok(Self::governance_queue_items_from_state(&payload.state))
    }

    pub async fn backfill_avatar_governance_storage(
        &self,
        team_id: Option<&str>,
    ) -> Result<AvatarGovernanceBackfillReport, ServiceError> {
        let team_oid = match team_id {
            Some(value) if !value.trim().is_empty() => {
                Some(ObjectId::parse_str(value).map_err(|e| {
                    ServiceError::Internal(format!("Invalid team id for governance backfill: {e}"))
                })?)
            }
            _ => None,
        };

        let filter = Self::avatar_portal_filter(team_oid);
        let portal_docs: Vec<Document> = self
            .portals()
            .find(
                filter,
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await?
            .try_collect()
            .await?;

        let mut report = AvatarGovernanceBackfillReport::default();
        let mut touched_team_ids = HashSet::new();

        for portal_doc in portal_docs {
            let Some(portal_oid) = portal_doc.get_object_id("_id").ok() else {
                continue;
            };
            let Some(portal_team_oid) = portal_doc.get_object_id("team_id").ok() else {
                continue;
            };

            let portal_id = portal_oid.to_hex();
            let portal_team_id = portal_team_oid.to_hex();
            let payload = Self::governance_payload_from_portal_doc(
                &portal_team_id,
                &portal_id,
                &portal_doc,
                Utc::now(),
            );

            report.portals_scanned += 1;
            touched_team_ids.insert(portal_team_id.clone());

            let existing = self
                .avatar_governance_states()
                .find_one(
                    doc! { "team_id": &portal_team_id, "portal_id": &portal_id },
                    None,
                )
                .await?;

            if existing.is_none() {
                self.refresh_avatar_governance_state_read_model(&payload)
                    .await?;
                report.states_created += 1;
            }

            report.events_seeded += self
                .refresh_avatar_governance_event_read_model(
                    &portal_team_id,
                    &portal_id,
                    &payload.state,
                    &payload.config,
                )
                .await?;
        }

        for team_id in touched_team_ids {
            let _ = self.sync_avatar_instance_projections(&team_id).await?;
            report.projections_synced_teams += 1;
        }

        Ok(report)
    }

    pub async fn audit_avatar_deep_water(
        &self,
        team_id: Option<&str>,
    ) -> Result<AvatarDeepWaterAuditReport, ServiceError> {
        let requested_team_id = Self::normalize_optional_scope_id(team_id);
        let team_oid = match requested_team_id.as_deref() {
            Some(value) => Some(ObjectId::parse_str(value).map_err(|e| {
                ServiceError::Internal(format!("Invalid team id for avatar deep-water audit: {e}"))
            })?),
            _ => None,
        };

        let filter = Self::avatar_portal_filter(team_oid);
        let portal_docs: Vec<Document> = self
            .portals()
            .find(
                filter,
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await?
            .try_collect()
            .await?;

        let mut team_ids = Vec::new();
        for portal_doc in &portal_docs {
            if let Ok(team_oid) = portal_doc.get_object_id("team_id") {
                let team_id = team_oid.to_hex();
                if !team_ids.iter().any(|value| value == &team_id) {
                    team_ids.push(team_id);
                }
            }
        }

        let governance_docs: Vec<AvatarGovernanceStateDoc> =
            if let Some(team_id) = requested_team_id.as_deref() {
                self.avatar_governance_states()
                    .find(doc! { "team_id": team_id }, None)
                    .await?
                    .try_collect()
                    .await?
            } else {
                self.avatar_governance_states()
                    .find(doc! {}, None)
                    .await?
                    .try_collect()
                    .await?
            };
        let total_state_docs = governance_docs.len() as u64;

        let agent_docs: Vec<TeamAgentDoc> = if team_ids.is_empty() {
            Vec::new()
        } else if team_ids.len() == 1 {
            self.agents()
                .find(doc! { "team_id": &team_ids[0] }, None)
                .await?
                .try_collect()
                .await?
        } else {
            self.agents()
                .find(doc! { "team_id": { "$in": &team_ids } }, None)
                .await?
                .try_collect()
                .await?
        };

        let (mut governance_map, governance_state_doc_counts, duplicate_rows, duplicate_state_docs) =
            Self::index_governance_state_docs(governance_docs);

        let agent_map: HashMap<(String, String), TeamAgentDoc> = agent_docs
            .into_iter()
            .map(|doc| ((doc.team_id.clone(), doc.agent_id.clone()), doc))
            .collect();

        let mut governance = AvatarGovernanceDivergenceAuditReport {
            total_avatar_portals: portal_docs.len() as u64,
            total_state_docs,
            duplicate_state_docs,
            duplicate_rows,
            ..AvatarGovernanceDivergenceAuditReport::default()
        };
        let mut bindings = AvatarBindingAuditReport {
            total_avatar_portals: portal_docs.len() as u64,
            ..AvatarBindingAuditReport::default()
        };

        for portal_doc in portal_docs {
            let Some(portal_oid) = portal_doc.get_object_id("_id").ok() else {
                continue;
            };
            let Some(portal_team_oid) = portal_doc.get_object_id("team_id").ok() else {
                continue;
            };

            let portal_id = portal_oid.to_hex();
            let portal_team_id = portal_team_oid.to_hex();
            let portal_name =
                Self::doc_string(&portal_doc, "name").unwrap_or_else(|| "未命名分身".to_string());
            let slug = Self::doc_string(&portal_doc, "slug").unwrap_or_default();

            let explicit_portal_state = Self::raw_portal_governance_state_json(&portal_doc);
            let explicit_portal_config = Self::raw_portal_governance_config_json(&portal_doc);
            let state_doc = governance_map.remove(&(portal_team_id.clone(), portal_id.clone()));

            let state_doc_exists = state_doc.is_some();
            let portal_state_exists = explicit_portal_state.is_some();
            let portal_config_exists = explicit_portal_config.is_some();
            let has_portal_governance = portal_state_exists || portal_config_exists;
            let state_matches_portal_state = state_doc
                .as_ref()
                .and_then(|doc| {
                    explicit_portal_state
                        .as_ref()
                        .map(|value| doc.state == *value)
                })
                .unwrap_or(false);
            let config_matches_portal_config = state_doc
                .as_ref()
                .and_then(|doc| {
                    explicit_portal_config
                        .as_ref()
                        .map(|value| doc.config == *value)
                })
                .unwrap_or(false);

            let governance_classification = Self::classify_governance_divergence(
                state_doc_exists,
                has_portal_governance,
                state_matches_portal_state,
                config_matches_portal_config,
            );

            match governance_classification {
                AvatarGovernanceDivergenceKind::DefaultOnly => governance.default_only += 1,
                AvatarGovernanceDivergenceKind::InSync => governance.in_sync += 1,
                AvatarGovernanceDivergenceKind::StateOnly => governance.state_only += 1,
                AvatarGovernanceDivergenceKind::SettingsOnly => governance.settings_only += 1,
                AvatarGovernanceDivergenceKind::Differing => governance.differing += 1,
            }

            governance.rows.push(AvatarGovernanceDivergenceRow {
                team_id: portal_team_id.clone(),
                portal_id: portal_id.clone(),
                portal_name: portal_name.clone(),
                slug: slug.clone(),
                classification: governance_classification,
                state_doc_exists,
                portal_state_exists,
                portal_config_exists,
                state_matches_portal_state,
                config_matches_portal_config,
            });

            let explicit_coding_agent_id = Self::doc_string(&portal_doc, "coding_agent_id")
                .filter(|value| !value.trim().is_empty());
            let explicit_service_agent_id = Self::doc_string(&portal_doc, "service_agent_id")
                .filter(|value| !value.trim().is_empty());
            let effective_manager_agent_id = Self::extract_portal_manager_id(&portal_doc);
            let effective_service_agent_id = Self::extract_portal_service_agent_id(&portal_doc);

            let manager_agent = effective_manager_agent_id
                .as_ref()
                .and_then(|agent_id| agent_map.get(&(portal_team_id.clone(), agent_id.clone())));
            let service_agent = effective_service_agent_id
                .as_ref()
                .and_then(|agent_id| agent_map.get(&(portal_team_id.clone(), agent_id.clone())));

            let mut issues = Vec::new();

            if explicit_coding_agent_id.is_none() {
                issues.push(AvatarBindingIssueKind::MissingExplicitManagerBinding);
                bindings.missing_explicit_manager_binding += 1;
            }
            if explicit_service_agent_id.is_none() {
                issues.push(AvatarBindingIssueKind::MissingExplicitServiceBinding);
                bindings.missing_explicit_service_binding += 1;
            }
            if effective_manager_agent_id.is_none() {
                issues.push(AvatarBindingIssueKind::MissingManagerBinding);
                bindings.missing_manager_binding += 1;
            }
            if effective_service_agent_id.is_none() {
                issues.push(AvatarBindingIssueKind::MissingServiceBinding);
                bindings.missing_service_binding += 1;
            }
            if effective_manager_agent_id.is_some() && manager_agent.is_none() {
                issues.push(AvatarBindingIssueKind::ManagerAgentNotFound);
                bindings.manager_agent_not_found += 1;
            }
            if effective_service_agent_id.is_some() && service_agent.is_none() {
                issues.push(AvatarBindingIssueKind::ServiceAgentNotFound);
                bindings.service_agent_not_found += 1;
            }
            if let Some(manager_agent) = manager_agent {
                if manager_agent.agent_domain.as_deref() != Some("digital_avatar")
                    || manager_agent.agent_role.as_deref() != Some("manager")
                {
                    issues.push(AvatarBindingIssueKind::ManagerRoleMismatch);
                    bindings.manager_role_mismatch += 1;
                }
            }
            if let Some(service_agent) = service_agent {
                if service_agent.agent_domain.as_deref() != Some("digital_avatar")
                    || service_agent.agent_role.as_deref() != Some("service")
                {
                    issues.push(AvatarBindingIssueKind::ServiceRoleMismatch);
                    bindings.service_role_mismatch += 1;
                }
                if service_agent.owner_manager_agent_id.as_deref()
                    != effective_manager_agent_id.as_deref()
                {
                    issues.push(AvatarBindingIssueKind::OwnerManagerMismatch);
                    bindings.owner_manager_mismatch += 1;
                }
            }
            if effective_manager_agent_id.is_some()
                && effective_manager_agent_id == effective_service_agent_id
            {
                issues.push(AvatarBindingIssueKind::SameAgentReused);
                bindings.same_agent_reused += 1;
            }

            let issue_messages = Self::binding_issue_messages(
                &issues,
                explicit_coding_agent_id.as_deref(),
                explicit_service_agent_id.as_deref(),
                effective_manager_agent_id.as_deref(),
                effective_service_agent_id.as_deref(),
                manager_agent,
                service_agent,
            );
            let has_shadow_invariant_issue = issues.iter().any(|issue| {
                matches!(
                    issue,
                    AvatarBindingIssueKind::ManagerRoleMismatch
                        | AvatarBindingIssueKind::ServiceRoleMismatch
                        | AvatarBindingIssueKind::OwnerManagerMismatch
                )
            });
            if has_shadow_invariant_issue {
                bindings.shadow_invariant_rows += 1;
            }

            if issues.is_empty() {
                bindings.valid += 1;
            }

            bindings.rows.push(AvatarBindingAuditRow {
                team_id: portal_team_id,
                portal_id,
                portal_name,
                slug,
                explicit_coding_agent_id,
                explicit_service_agent_id,
                effective_manager_agent_id,
                effective_service_agent_id,
                manager_agent_domain: manager_agent.and_then(|agent| agent.agent_domain.clone()),
                manager_agent_role: manager_agent.and_then(|agent| agent.agent_role.clone()),
                service_agent_domain: service_agent.and_then(|agent| agent.agent_domain.clone()),
                service_agent_role: service_agent.and_then(|agent| agent.agent_role.clone()),
                service_owner_manager_agent_id: service_agent
                    .and_then(|agent| agent.owner_manager_agent_id.clone()),
                issues,
                issue_messages,
            });
        }

        let mut orphan_rows: Vec<_> = governance_map
            .into_values()
            .map(|doc| {
                let state_key = (doc.team_id.clone(), doc.portal_id.clone());
                AvatarGovernanceOrphanStateRow {
                    team_id: doc.team_id,
                    portal_id: doc.portal_id,
                    state_doc_count: governance_state_doc_counts
                        .get(&state_key)
                        .copied()
                        .unwrap_or(1),
                    updated_at: doc.updated_at.to_rfc3339(),
                }
            })
            .collect();
        orphan_rows.sort_by(|left, right| {
            left.team_id
                .cmp(&right.team_id)
                .then_with(|| left.portal_id.cmp(&right.portal_id))
        });
        governance.orphan_state_docs = orphan_rows.iter().map(|row| row.state_doc_count).sum();
        governance.orphan_rows = orphan_rows;

        Ok(AvatarDeepWaterAuditReport {
            generated_at: Utc::now().to_rfc3339(),
            requested_team_id,
            governance,
            bindings,
            read_side_effects: Self::avatar_read_side_effect_audit_items(),
        })
    }

    pub async fn list_avatar_instance_projections(
        &self,
        team_id: &str,
    ) -> Result<Vec<AvatarInstanceSummary>, mongodb::error::Error> {
        Ok(self
            .derive_avatar_instance_projection_docs(team_id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    fn build_avatar_manager_reports(
        governance_events: &[AvatarGovernanceEventPayload],
        portal_id: &str,
        work_object_labels: &[String],
    ) -> Vec<AvatarWorkbenchReportItemPayload> {
        let mut reports = Vec::new();

        for event in governance_events.iter().take(6) {
            let needs_decision = Self::governance_event_needs_decision(event);
            reports.push(AvatarWorkbenchReportItemPayload {
                id: format!("event:{}", event.event_id),
                ts: event.created_at,
                kind: if event.entity_type == "runtime" {
                    "runtime".to_string()
                } else {
                    "governance".to_string()
                },
                title: event.title.clone(),
                summary: event
                    .detail
                    .clone()
                    .unwrap_or_else(|| "新的治理或运行记录已经写入工作台。".to_string()),
                status: event.status.clone().unwrap_or_else(|| "logged".to_string()),
                source: event
                    .actor_name
                    .clone()
                    .or_else(|| event.actor_id.clone())
                    .unwrap_or_else(|| "系统汇总".to_string()),
                recommendation: Some(if event.entity_type == "runtime" {
                    "如果涉及失败恢复或对象补充，建议先打开日志与治理台查看细节。".to_string()
                } else {
                    "如果需要进一步确认权限、提案或策略，建议先打开治理台处理。".to_string()
                }),
                action_kind: Some(if event.entity_type == "runtime" {
                    "open_logs".to_string()
                } else {
                    "open_governance".to_string()
                }),
                action_target_id: Some(portal_id.to_string()),
                work_objects: work_object_labels.iter().take(2).cloned().collect(),
                outputs: Vec::new(),
                needs_decision,
            });
        }

        reports.sort_by(|left, right| right.ts.cmp(&left.ts));
        reports.dedup_by(|left, right| left.id == right.id);
        reports
    }

    fn merge_avatar_workbench_reports(
        derived_reports: &[AvatarWorkbenchReportItemPayload],
        persisted_reports: &[AvatarWorkbenchReportItemPayload],
        limit: usize,
    ) -> Vec<AvatarWorkbenchReportItemPayload> {
        let mut reports = persisted_reports
            .iter()
            .cloned()
            .chain(derived_reports.iter().cloned())
            .collect::<Vec<_>>();
        reports.sort_by(|left, right| right.ts.cmp(&left.ts));
        let mut seen = HashSet::new();
        let mut deduped = Vec::with_capacity(reports.len());
        for report in reports {
            if seen.insert(report.id.clone()) {
                deduped.push(report);
            }
        }
        deduped.truncate(limit);
        deduped
    }

    async fn list_persisted_avatar_manager_reports(
        &self,
        team_id: &str,
        portal_id: &str,
        limit: u32,
    ) -> Result<Vec<AvatarWorkbenchReportItemPayload>, mongodb::error::Error> {
        let docs = self
            .avatar_manager_reports()
            .find(
                doc! {
                    "team_id": team_id,
                    "portal_id": portal_id,
                    "report_source": {
                        "$exists": true,
                        "$ne": default_avatar_manager_report_source(),
                    }
                },
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .limit(i64::from(limit.clamp(1, 20)))
                    .build(),
            )
            .await?
            .try_collect::<Vec<AvatarManagerReportDoc>>()
            .await?;
        Ok(docs.into_iter().map(Into::into).collect())
    }

    pub async fn upsert_avatar_manager_report(
        &self,
        team_id: &str,
        portal_id: &str,
        report: AvatarWorkbenchReportItemPayload,
        report_source: &str,
    ) -> Result<(), mongodb::error::Error> {
        let now = Utc::now();
        self.avatar_manager_reports()
            .update_one(
                doc! {
                    "team_id": team_id,
                    "portal_id": portal_id,
                    "report_id": &report.id,
                    "report_source": report_source,
                },
                doc! {
                    "$set": {
                        "team_id": team_id,
                        "portal_id": portal_id,
                        "report_id": &report.id,
                        "report_source": report_source,
                        "kind": &report.kind,
                        "title": &report.title,
                        "summary": &report.summary,
                        "status": &report.status,
                        "source": &report.source,
                        "recommendation": bson::to_bson(&report.recommendation).unwrap_or(Bson::Null),
                        "action_kind": bson::to_bson(&report.action_kind).unwrap_or(Bson::Null),
                        "action_target_id": bson::to_bson(&report.action_target_id).unwrap_or(Bson::Null),
                        "work_objects": bson::to_bson(&report.work_objects).unwrap_or_else(|_| Bson::Array(Vec::new())),
                        "outputs": bson::to_bson(&report.outputs).unwrap_or_else(|_| Bson::Array(Vec::new())),
                        "needs_decision": report.needs_decision,
                        "created_at": bson::DateTime::from_chrono(report.ts),
                        "synced_at": bson::DateTime::from_chrono(now),
                    },
                    "$setOnInsert": {
                        "_id": ObjectId::new(),
                    }
                },
                mongodb::options::UpdateOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(())
    }

    pub async fn list_avatar_manager_reports(
        &self,
        team_id: &str,
        portal_id: &str,
        limit: u32,
    ) -> Result<Vec<AvatarWorkbenchReportItemPayload>, mongodb::error::Error> {
        let docs = self
            .avatar_manager_reports()
            .find(
                doc! { "team_id": team_id, "portal_id": portal_id },
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .limit(i64::from(limit.clamp(1, 20)))
                    .build(),
            )
            .await?
            .try_collect::<Vec<AvatarManagerReportDoc>>()
            .await?;
        Ok(docs.into_iter().map(Into::into).collect())
    }

    fn governance_event_needs_decision(event: &AvatarGovernanceEventPayload) -> bool {
        event
            .status
            .as_deref()
            .map(|status| {
                matches!(
                    status,
                    "pending"
                        | "review"
                        | "needs_review"
                        | "needs_human"
                        | "requires_approval"
                        | "attention"
                        | "failed"
                )
            })
            .unwrap_or(false)
            || event
                .meta
                .get("needs_decision")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            || matches!(
                event
                    .meta
                    .get("risk")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default(),
                "high" | "critical"
            )
    }

    async fn resolve_work_object_labels(
        &self,
        team_oid: Option<ObjectId>,
        portal_doc: Option<&Document>,
    ) -> Result<Vec<String>, mongodb::error::Error> {
        let Some(bound_ids) = portal_doc.and_then(|doc| doc.get_array("bound_document_ids").ok())
        else {
            return Ok(Vec::new());
        };
        let object_ids = bound_ids
            .iter()
            .filter_map(|value| value.as_str())
            .filter_map(|raw| ObjectId::parse_str(raw).ok())
            .collect::<Vec<_>>();
        if object_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut filter = doc! { "_id": { "$in": object_ids }, "is_deleted": false };
        if let Some(team_oid) = team_oid {
            filter.insert("team_id", team_oid);
        }
        let docs = self
            .documents_store()
            .find(
                filter,
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .limit(6)
                    .build(),
            )
            .await?
            .try_collect::<Vec<TeamDocument>>()
            .await?;
        Ok(docs
            .into_iter()
            .map(|doc| doc.display_name.unwrap_or(doc.name))
            .collect())
    }

    pub async fn get_avatar_workbench_snapshot(
        &self,
        team_id: &str,
        portal_id: &str,
    ) -> Result<AvatarWorkbenchSnapshotPayload, mongodb::error::Error> {
        let projections = self.list_avatar_instance_projections(team_id).await?;
        let projection = projections
            .iter()
            .find(|item| item.portal_id == portal_id)
            .cloned();

        let team_oid = ObjectId::parse_str(team_id).ok();
        let portal_oid = ObjectId::parse_str(portal_id).ok();
        let portal_doc = match (team_oid, portal_oid) {
            (Some(team_oid), Some(portal_oid)) => {
                self.portals()
                    .find_one(doc! { "_id": portal_oid, "team_id": team_oid }, None)
                    .await?
            }
            (_, Some(portal_oid)) => {
                self.portals()
                    .find_one(doc! { "_id": portal_oid }, None)
                    .await?
            }
            _ => None,
        };

        let avatar_name = projection
            .as_ref()
            .map(|item| item.name.clone())
            .or_else(|| {
                portal_doc
                    .as_ref()
                    .and_then(|doc| doc.get_str("name").ok().map(ToOwned::to_owned))
            })
            .unwrap_or_else(|| "未命名岗位".to_string());
        let avatar_type = projection
            .as_ref()
            .map(|item| item.avatar_type.clone())
            .or_else(|| {
                portal_doc.as_ref().and_then(|doc| {
                    doc.get_document("settings")
                        .ok()
                        .and_then(|settings| settings.get_document("digitalAvatarProfile").ok())
                        .and_then(|profile| {
                            profile.get_str("avatar_type").ok().map(ToOwned::to_owned)
                        })
                })
            })
            .unwrap_or_else(|| "unknown".to_string());
        let avatar_status = projection
            .as_ref()
            .map(|item| item.status.clone())
            .or_else(|| {
                portal_doc
                    .as_ref()
                    .and_then(|doc| doc.get_str("status").ok().map(ToOwned::to_owned))
            })
            .unwrap_or_else(|| "draft".to_string());
        let manager_agent_id = projection
            .as_ref()
            .and_then(|item| item.manager_agent_id.clone());
        let service_agent_id = projection
            .as_ref()
            .and_then(|item| item.service_agent_id.clone());
        let document_access_mode = projection
            .as_ref()
            .map(|item| item.document_access_mode.clone())
            .or_else(|| {
                portal_doc.as_ref().and_then(|doc| {
                    doc.get_str("document_access_mode")
                        .ok()
                        .map(ToOwned::to_owned)
                })
            })
            .unwrap_or_else(|| "read_only".to_string());
        let work_object_count = portal_doc
            .as_ref()
            .and_then(|doc| doc.get_array("bound_document_ids").ok())
            .map_or(0, |items| items.len() as u32);

        let governance_events = self
            .list_avatar_governance_events(team_id, portal_id, 40)
            .await
            .unwrap_or_default();
        let queue_items = self
            .list_avatar_governance_queue(team_id, portal_id)
            .await
            .unwrap_or_default();

        let work_object_labels = self
            .resolve_work_object_labels(team_oid, portal_doc.as_ref())
            .await
            .unwrap_or_default();
        let agent_ids = [manager_agent_id.as_deref(), service_agent_id.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let _agent_names = self.batch_get_agent_names(&agent_ids).await;

        let derived_reports =
            Self::build_avatar_manager_reports(&governance_events, portal_id, &work_object_labels);
        let persisted_reports = self
            .list_persisted_avatar_manager_reports(team_id, portal_id, 6)
            .await
            .unwrap_or_default();
        let reports = Self::merge_avatar_workbench_reports(&derived_reports, &persisted_reports, 4);

        let mut decisions = Vec::new();
        for item in queue_items.iter().take(4) {
            let risk = queue_item_risk(&item.meta).to_string();
            decisions.push(AvatarWorkbenchDecisionItemPayload {
                id: format!("decision-queue:{}", item.id),
                ts: parse_rfc3339_utc(&item.ts).unwrap_or_else(Utc::now),
                kind: item.kind.clone(),
                title: item.title.clone(),
                detail: item.detail.clone(),
                status: item.status.clone(),
                risk: risk.clone(),
                source: queue_item_source(&item.kind, &item.meta),
                recommendation: Some(queue_item_recommendation(&item.kind, &risk)),
                work_objects: work_object_labels.iter().take(2).cloned().collect(),
                action_kind: Some("open_governance".to_string()),
                action_target_id: Some(portal_id.to_string()),
            });
        }
        decisions.sort_by(|left, right| right.ts.cmp(&left.ts));
        decisions.dedup_by(|left, right| left.id == right.id);
        decisions.truncate(4);

        let latest_activity_at = reports
            .iter()
            .map(|item| item.ts)
            .chain(decisions.iter().map(|item| item.ts))
            .chain(projection.as_ref().map(|item| item.projected_at))
            .max()
            .unwrap_or_else(Utc::now);

        Ok(AvatarWorkbenchSnapshotPayload {
            portal_id: portal_id.to_string(),
            team_id: team_id.to_string(),
            summary: AvatarWorkbenchSummaryPayload {
                portal_id: portal_id.to_string(),
                team_id: team_id.to_string(),
                avatar_name,
                avatar_type,
                avatar_status,
                manager_agent_id,
                service_agent_id,
                document_access_mode,
                work_object_count,
                pending_decision_count: decisions.len() as u32,
                last_activity_at: latest_activity_at,
            },
            reports,
            decisions,
        })
    }

    pub async fn update_agent(
        &self,
        id: &str,
        req: UpdateAgentRequest,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let now = Utc::now();
        let mut set_doc = doc! { "updated_at": bson::DateTime::from_chrono(now) };

        if let Some(name) = req.name {
            set_doc.insert("name", name);
        }
        if let Some(desc) = req.description {
            set_doc.insert("description", desc);
        }
        if let Some(avatar) = req.avatar {
            set_doc.insert("avatar", avatar);
        }
        if let Some(system_prompt) = req.system_prompt {
            set_doc.insert("system_prompt", system_prompt);
        }
        if let Some(api_url) = req.api_url {
            set_doc.insert("api_url", api_url);
        }
        if let Some(model) = req.model {
            set_doc.insert("model", model);
        }
        if let Some(ref api_key) = req.api_key {
            tracing::info!(
                "Updating API key for agent {}: key length = {}",
                id,
                api_key.len()
            );
            set_doc.insert("api_key", api_key.clone());
        }
        if let Some(api_format) = req.api_format {
            set_doc.insert("api_format", api_format);
        }
        if let Some(status) = req.status {
            set_doc.insert("status", status.to_string());
        }
        if let Some(ref extensions) = req.enabled_extensions {
            let sanitized = sanitize_enabled_extensions(extensions.clone());
            let ext_bson = mongodb::bson::to_bson(&sanitized).unwrap_or(bson::Bson::Array(vec![]));
            set_doc.insert("enabled_extensions", ext_bson);
        }
        if let Some(ref custom_ext) = req.custom_extensions {
            let ext_bson = custom_extensions_to_bson(custom_ext);
            set_doc.insert("custom_extensions", ext_bson);
        }
        if let Some(ref agent_domain) = req.agent_domain {
            set_doc.insert("agent_domain", agent_domain.clone());
        }
        if let Some(ref agent_role) = req.agent_role {
            set_doc.insert("agent_role", agent_role.clone());
        }
        if let Some(ref owner_manager_agent_id) = req.owner_manager_agent_id {
            set_doc.insert("owner_manager_agent_id", owner_manager_agent_id.clone());
        }
        if let Some(ref template_source_agent_id) = req.template_source_agent_id {
            set_doc.insert("template_source_agent_id", template_source_agent_id.clone());
        }
        if let Some(ref allowed_groups) = req.allowed_groups {
            let bson_val =
                mongodb::bson::to_bson(allowed_groups).unwrap_or(bson::Bson::Array(vec![]));
            set_doc.insert("allowed_groups", bson_val);
        }
        if let Some(max_concurrent) = req.max_concurrent_tasks {
            set_doc.insert(
                "max_concurrent_tasks",
                normalize_max_concurrent_tasks(Some(max_concurrent)) as i32,
            );
        }
        if let Some(temperature) = req.temperature {
            set_doc.insert("temperature", temperature as f64);
        }
        if let Some(max_tokens) = req.max_tokens {
            set_doc.insert("max_tokens", max_tokens);
        }
        if let Some(context_limit) = req.context_limit {
            set_doc.insert("context_limit", context_limit as i64);
        }
        if let Some(thinking_enabled) = req.thinking_enabled {
            set_doc.insert("thinking_enabled", thinking_enabled);
        }
        if let Some(thinking_budget) = req.thinking_budget {
            set_doc.insert("thinking_budget", thinking_budget as i64);
        }
        if let Some(ref reasoning_effort) = req.reasoning_effort {
            set_doc.insert("reasoning_effort", reasoning_effort.clone());
        }
        if let Some(output_reserve_tokens) = req.output_reserve_tokens {
            set_doc.insert("output_reserve_tokens", output_reserve_tokens as i64);
        }
        if let Some(auto_compact_threshold) = req.auto_compact_threshold {
            set_doc.insert("auto_compact_threshold", auto_compact_threshold);
        }
        if let Some(supports_multimodal) = req.supports_multimodal {
            set_doc.insert("supports_multimodal", supports_multimodal);
        }
        if let Some(prompt_caching_mode) = req.prompt_caching_mode {
            set_doc.insert(
                "prompt_caching_mode",
                mongodb::bson::to_bson(&prompt_caching_mode)
                    .unwrap_or(bson::Bson::String("auto".to_string())),
            );
        }
        if let Some(cache_edit_mode) = req.cache_edit_mode {
            set_doc.insert(
                "cache_edit_mode",
                mongodb::bson::to_bson(&cache_edit_mode)
                    .unwrap_or(bson::Bson::String("auto".to_string())),
            );
        }
        if let Some(ref assigned_skills) = req.assigned_skills {
            let skills_bson =
                mongodb::bson::to_bson(&sanitize_assigned_skills(assigned_skills.clone()))
                    .unwrap_or(bson::Bson::Array(vec![]));
            set_doc.insert("assigned_skills", skills_bson);
        }
        if let Some(skill_binding_mode) = req.skill_binding_mode {
            set_doc.insert(
                "skill_binding_mode",
                mongodb::bson::to_bson(&skill_binding_mode)
                    .unwrap_or(bson::Bson::String("hybrid".to_string())),
            );
        }
        if let Some(ref delegation_policy) = req.delegation_policy {
            set_doc.insert(
                "delegation_policy",
                mongodb::bson::to_bson(delegation_policy).unwrap_or(bson::Bson::Document(doc! {})),
            );
        }
        if let Some(ref attached_team_extensions) = req.attached_team_extensions {
            let refs_bson = mongodb::bson::to_bson(&sanitize_attached_team_extensions(
                attached_team_extensions.clone(),
            ))
            .unwrap_or(bson::Bson::Array(vec![]));
            set_doc.insert("attached_team_extensions", refs_bson);
        }
        if let Some(auto_approve) = req.auto_approve_chat {
            set_doc.insert("auto_approve_chat", auto_approve);
        }

        self.agents()
            .update_one(doc! { "agent_id": id }, doc! { "$set": set_doc }, None)
            .await?;

        self.get_agent(id).await
    }

    pub async fn delete_agent(&self, id: &str) -> Result<bool, mongodb::error::Error> {
        let result = self
            .agents()
            .delete_one(doc! { "agent_id": id }, None)
            .await?;
        Ok(result.deleted_count > 0)
    }

    // Task operations
    pub async fn submit_task(
        &self,
        submitter_id: &str,
        req: SubmitTaskRequest,
    ) -> Result<AgentTask, ServiceError> {
        validate_priority(req.priority)?;

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let (run_id, session_id, task_role, task_node_id) =
            extract_task_runtime_binding(&req.content);

        let doc = AgentTaskDoc {
            id: None,
            task_id: id.clone(),
            run_id,
            session_id,
            task_role,
            task_node_id,
            team_id: req.team_id,
            agent_id: req.agent_id,
            submitter_id: submitter_id.to_string(),
            approver_id: None,
            task_type: req.task_type.to_string(),
            content: req.content,
            status: "pending".to_string(),
            priority: req.priority,
            submitted_at: now,
            approved_at: None,
            started_at: None,
            completed_at: None,
            error_message: None,
        };

        self.tasks().insert_one(&doc, None).await?;
        self.get_task(&id)
            .await?
            .ok_or_else(|| ServiceError::Internal(format!("Task not found after insert: {}", id)))
    }

    pub async fn get_task(&self, id: &str) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let doc = self.tasks().find_one(doc! { "task_id": id }, None).await?;
        Ok(doc.map(|d| d.into()))
    }

    pub async fn list_tasks(
        &self,
        query: ListTasksQuery,
    ) -> Result<PaginatedResponse<AgentTask>, mongodb::error::Error> {
        let clamped_limit = query.limit.min(100);
        let limit = clamped_limit as i64;
        let skip = ((query.page.saturating_sub(1)) * clamped_limit) as u64;

        let mut filter = doc! { "team_id": &query.team_id };
        if let Some(agent_id) = &query.agent_id {
            filter.insert("agent_id", agent_id);
        }
        if let Some(status) = &query.status {
            filter.insert("status", status.to_string());
        }

        let total = self.tasks().count_documents(filter.clone(), None).await?;

        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "priority": -1, "submitted_at": -1 })
            .skip(skip)
            .limit(limit)
            .build();
        let cursor = self.tasks().find(filter, options).await?;

        let docs: Vec<AgentTaskDoc> = cursor.try_collect().await?;
        let items: Vec<AgentTask> = docs.into_iter().map(|d| d.into()).collect();

        Ok(PaginatedResponse::new(
            items,
            total,
            query.page,
            query.limit,
        ))
    }

    pub async fn approve_task(
        &self,
        task_id: &str,
        approver_id: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let now = Utc::now();
        let result = self
            .tasks()
            .update_one(
                doc! { "task_id": task_id, "status": "pending" },
                doc! { "$set": {
                    "status": "approved",
                    "approver_id": approver_id,
                    "approved_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.matched_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    pub async fn mark_task_queued(
        &self,
        task_id: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let result = self
            .tasks()
            .update_one(
                doc! {
                    "task_id": task_id,
                    "status": { "$in": ["approved", "queued"] }
                },
                doc! { "$set": { "status": "queued" } },
                None,
            )
            .await?;
        if result.matched_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    pub async fn claim_next_queued_task_for_agent(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

        let options = FindOneAndUpdateOptions::builder()
            .sort(doc! {
                "priority": -1,
                "submitted_at": 1,
                "_id": 1,
            })
            .return_document(ReturnDocument::After)
            .build();
        self.tasks()
            .find_one_and_update(
                doc! {
                    "agent_id": agent_id,
                    "status": "queued",
                },
                doc! { "$set": { "status": "approved" } },
                options,
            )
            .await
            .map(|doc| doc.map(AgentTask::from))
    }

    pub async fn list_agents_with_queued_tasks(
        &self,
    ) -> Result<Vec<String>, mongodb::error::Error> {
        let values = self
            .tasks()
            .distinct("agent_id", doc! { "status": "queued" }, None)
            .await?;
        Ok(values
            .into_iter()
            .filter_map(|value| match value {
                Bson::String(agent_id) if !agent_id.trim().is_empty() => Some(agent_id),
                _ => None,
            })
            .collect())
    }

    pub async fn reject_task(
        &self,
        task_id: &str,
        approver_id: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let now = Utc::now();
        let result = self
            .tasks()
            .update_one(
                doc! { "task_id": task_id, "status": "pending" },
                doc! { "$set": {
                    "status": "rejected",
                    "approver_id": approver_id,
                    "approved_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.modified_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    pub async fn cancel_task(
        &self,
        task_id: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let now = Utc::now();
        let result = self
            .tasks()
            .update_one(
                doc! {
                    "task_id": task_id,
                    "status": { "$in": ["pending", "approved", "queued", "running"] }
                },
                doc! { "$set": {
                    "status": "cancelled",
                    "completed_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.modified_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    /// Mark a task as failed with an error message.
    /// Only updates if current status is running or approved (won't overwrite cancelled/completed).
    pub async fn fail_task(
        &self,
        task_id: &str,
        error: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let now = Utc::now();
        let result = self
            .tasks()
            .update_one(
                doc! {
                    "task_id": task_id,
                    "status": { "$in": ["running", "approved", "queued"] }
                },
                doc! { "$set": {
                    "status": "failed",
                    "error_message": error,
                    "completed_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.modified_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    pub async fn mark_task_running(
        &self,
        task_id: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let now = Utc::now();
        let result = self
            .tasks()
            .update_one(
                doc! {
                    "task_id": task_id,
                    "status": { "$in": ["approved", "queued", "running"] }
                },
                doc! { "$set": {
                    "status": "running",
                    "started_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.matched_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    pub async fn complete_task(
        &self,
        task_id: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let now = Utc::now();
        let result = self
            .tasks()
            .update_one(
                doc! {
                    "task_id": task_id,
                    "status": { "$in": ["running", "approved", "queued"] }
                },
                doc! { "$set": {
                    "status": "completed",
                    "completed_at": bson::DateTime::from_chrono(now),
                    "error_message": bson::Bson::Null
                }},
                None,
            )
            .await?;

        if result.matched_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    pub async fn save_task_result(
        &self,
        task_id: &str,
        result_type: TaskResultType,
        content: serde_json::Value,
    ) -> Result<TaskResult, mongodb::error::Error> {
        let result = TaskResult::new(task_id.to_string(), result_type, content);
        let doc = TaskResultDoc {
            id: None,
            result_id: result.id.clone(),
            task_id: result.task_id.clone(),
            result_type: result.result_type.to_string(),
            content: result.content.clone(),
            created_at: result.created_at,
        };
        self.results().insert_one(doc, None).await?;
        Ok(result)
    }

    pub async fn try_acquire_execution_slot(
        &self,
        agent_id: &str,
    ) -> Result<ExecutionSlotAcquireOutcome, mongodb::error::Error> {
        let result = self
            .agents()
            .update_one(
                doc! {
                    "agent_id": agent_id,
                    "$expr": {
                        "$lt": [
                            { "$ifNull": ["$active_execution_slots", 0] },
                            {
                                "$cond": [
                                    { "$gt": ["$max_concurrent_tasks", 0] },
                                    "$max_concurrent_tasks",
                                    1
                                ]
                            }
                        ]
                    }
                },
                doc! {
                    "$inc": { "active_execution_slots": 1i32 },
                    "$set": { "updated_at": bson::DateTime::now() }
                },
                None,
            )
            .await?;
        if result.modified_count > 0 {
            Ok(ExecutionSlotAcquireOutcome::Acquired)
        } else {
            Ok(ExecutionSlotAcquireOutcome::Saturated)
        }
    }

    pub async fn release_execution_slot(
        &self,
        agent_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.agents()
            .update_one(
                doc! {
                    "agent_id": agent_id,
                    "active_execution_slots": { "$gt": 0 }
                },
                doc! {
                    "$inc": { "active_execution_slots": -1i32 },
                    "$set": { "updated_at": bson::DateTime::now() }
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Reset runtime-only execution slot counters after process restart.
    /// In-memory runners do not survive a restart, so persisted slot counters
    /// must not continue to throttle new V4 executions.
    pub async fn reset_execution_slots(&self) -> Result<u64, mongodb::error::Error> {
        let result = self
            .agents()
            .update_many(
                doc! { "active_execution_slots": { "$gt": 0 } },
                doc! {
                    "$set": {
                        "active_execution_slots": 0i32,
                        "updated_at": bson::DateTime::now()
                    }
                },
                None,
            )
            .await?;
        Ok(result.modified_count)
    }

    pub async fn get_task_results(
        &self,
        task_id: &str,
    ) -> Result<Vec<TaskResult>, mongodb::error::Error> {
        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "created_at": 1 })
            .build();
        let cursor = self
            .results()
            .find(doc! { "task_id": task_id }, options)
            .await?;

        let docs: Vec<TaskResultDoc> = cursor.try_collect().await?;
        Ok(docs.into_iter().map(|d| d.into()).collect())
    }

    /// Check if a user has access to a specific agent based on group-based access control
    pub async fn check_agent_access(
        &self,
        agent_id: &str,
        _user_id: &str,
        user_group_ids: &[String],
    ) -> Result<bool, mongodb::error::Error> {
        let agent = match self.get_agent(agent_id).await? {
            Some(a) => a,
            None => return Ok(false),
        };

        // Empty allowed_groups = all team members can use
        if agent.allowed_groups.is_empty() {
            return Ok(true);
        }
        // User must be in at least one allowed group
        Ok(user_group_ids
            .iter()
            .any(|gid| agent.allowed_groups.contains(gid)))
    }

    /// Update agent access control settings
    pub async fn update_access_control(
        &self,
        agent_id: &str,
        allowed_groups: Vec<String>,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let now = Utc::now();
        let bson_val = mongodb::bson::to_bson(&allowed_groups).unwrap_or(bson::Bson::Array(vec![]));
        let set = doc! {
            "updated_at": bson::DateTime::from_chrono(now),
            "allowed_groups": bson_val,
        };

        self.agents()
            .update_one(doc! { "agent_id": agent_id }, doc! { "$set": set }, None)
            .await?;

        self.get_agent(agent_id).await
    }

    // ========== Team Extension Bridge ==========

    fn shared_extension_to_custom_extension(
        ext: &agime_team::models::mongo::Extension,
    ) -> Result<CustomExtensionConfig, ServiceError> {
        let uri_or_cmd = ext
            .config
            .get_str("uri_or_cmd")
            .or_else(|_| ext.config.get_str("uriOrCmd"))
            .or_else(|_| ext.config.get_str("command"))
            .unwrap_or_default()
            .to_string();

        if uri_or_cmd.trim().is_empty() {
            return Err(ServiceError::Validation(ValidationError::ExtensionConfig));
        }

        let empty_args = Vec::new();
        let args: Vec<String> = ext
            .config
            .get_array("args")
            .unwrap_or(&empty_args)
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let envs: std::collections::HashMap<String, String> = ext
            .config
            .get_document("envs")
            .map(|doc| {
                doc.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        let ext_id_hex = ext.id.map(|id| id.to_hex()).unwrap_or_default();

        Ok(CustomExtensionConfig {
            name: ext.name.clone(),
            ext_type: ext.extension_type.clone(),
            uri_or_cmd,
            args,
            envs,
            enabled: true,
            source: Some("team".to_string()),
            source_extension_id: Some(ext_id_hex),
        })
    }

    fn shared_extension_to_attached_ref(
        ext: &agime_team::models::mongo::Extension,
    ) -> AttachedTeamExtensionRef {
        AttachedTeamExtensionRef {
            extension_id: ext.id.map(|id| id.to_hex()).unwrap_or_default(),
            enabled: true,
            allowed_groups: Vec::new(),
            runtime_name: Some(ext.name.clone()),
            display_name: Some(ext.name.clone()),
            transport: Some(ext.extension_type.clone()),
        }
    }

    /// Attach a team shared extension to an agent by reference.
    pub async fn add_team_extension_to_agent(
        &self,
        agent_id: &str,
        extension_id: &str,
        team_id: &str,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        use agime_team::services::mongo::extension_service_mongo::ExtensionService;

        let ext_service = ExtensionService::new((*self.db).clone());

        // 1. Fetch the shared extension
        let ext = ext_service
            .get(extension_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?
            .ok_or_else(|| ServiceError::Internal("Extension not found".to_string()))?;

        // Verify team ownership
        if ext.team_id.to_hex() != team_id {
            return Err(ServiceError::Internal(
                "Extension does not belong to this team".to_string(),
            ));
        }

        // 2. Get current agent
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;

        // 3. Check for duplicate attachment or conflicting runtime name
        if agent
            .attached_team_extensions
            .iter()
            .any(|existing| existing.extension_id == extension_id)
            || agent
                .custom_extensions
                .iter()
                .any(|existing| custom_extension_name_eq(&existing.name, &ext.name))
        {
            return Err(ServiceError::Internal(format!(
                "Extension '{}' already exists in this agent",
                ext.name
            )));
        }

        // 4. Append to attached_team_extensions via update_agent
        let mut next_refs = agent.attached_team_extensions.clone();
        next_refs.push(Self::shared_extension_to_attached_ref(&ext));
        next_refs.sort_by(|left, right| left.extension_id.cmp(&right.extension_id));

        self.update_agent(
            agent_id,
            UpdateAgentRequest {
                name: None,
                description: None,
                avatar: None,
                system_prompt: None,
                api_url: None,
                model: None,
                api_key: None,
                api_format: None,
                status: None,
                enabled_extensions: None,
                custom_extensions: None,
                agent_domain: None,
                agent_role: None,
                owner_manager_agent_id: None,
                template_source_agent_id: None,
                allowed_groups: None,
                max_concurrent_tasks: None,
                temperature: None,
                max_tokens: None,
                context_limit: None,
                thinking_enabled: None,
                thinking_budget: None,
                reasoning_effort: None,
                output_reserve_tokens: None,
                auto_compact_threshold: None,
                supports_multimodal: None,
                prompt_caching_mode: None,
                cache_edit_mode: None,
                assigned_skills: None,
                skill_binding_mode: None,
                delegation_policy: None,
                attached_team_extensions: Some(next_refs),
                auto_approve_chat: None,
            },
        )
        .await
        .map_err(ServiceError::Database)
    }

    /// Return team agents that currently have a team-sourced custom extension attached.
    pub async fn list_agents_attached_to_team_extension(
        &self,
        team_id: &str,
        extension_id: &str,
    ) -> Result<Vec<TeamAgent>, ServiceError> {
        let cursor = self
            .agents()
            .find(
                doc! {
                    "team_id": team_id,
                    "is_deleted": { "$ne": true },
                    "$or": [
                        { "attached_team_extensions.extension_id": extension_id },
                        { "custom_extensions.source_extension_id": extension_id }
                    ],
                },
                None,
            )
            .await
            .map_err(ServiceError::Database)?;
        let docs: Vec<TeamAgentDoc> = cursor.try_collect().await.map_err(ServiceError::Database)?;
        let mut items: Vec<TeamAgent> = docs.into_iter().map(Into::into).collect();
        for agent in &mut items {
            self.backfill_legacy_avatar_agent_metadata(agent)
                .await
                .map_err(ServiceError::Database)?;
        }
        Ok(items)
    }

    /// Sync the current team extension definition into every attached agent copy.
    pub async fn sync_team_extension_to_attached_agents(
        &self,
        team_id: &str,
        extension_id: &str,
    ) -> Result<Vec<TeamAgent>, ServiceError> {
        use agime_team::services::mongo::extension_service_mongo::ExtensionService;

        let ext_service = ExtensionService::new((*self.db).clone());
        let extension = ext_service
            .get(extension_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?
            .ok_or_else(|| ServiceError::Internal("Extension not found".to_string()))?;

        if extension.team_id.to_hex() != team_id {
            return Err(ServiceError::Internal(
                "Extension does not belong to this team".to_string(),
            ));
        }

        let attached_agents = self
            .list_agents_attached_to_team_extension(team_id, extension_id)
            .await?;
        let mut updated_agents = Vec::new();

        for agent in attached_agents {
            let mut next_refs = agent.attached_team_extensions.clone();
            let mut changed = false;
            for existing in &mut next_refs {
                if existing.extension_id != extension_id {
                    continue;
                }
                let enabled = existing.enabled;
                *existing = Self::shared_extension_to_attached_ref(&extension);
                existing.enabled = enabled;
                changed = true;
            }
            if !changed
                && !agent.custom_extensions.iter().any(|existing| {
                    custom_extension_matches_source_extension(existing, extension_id)
                })
            {
                continue;
            }

            if let Some(updated) = self
                .update_agent(
                    &agent.id,
                    UpdateAgentRequest {
                        name: None,
                        description: None,
                        avatar: None,
                        system_prompt: None,
                        api_url: None,
                        model: None,
                        api_key: None,
                        api_format: None,
                        status: None,
                        enabled_extensions: None,
                        custom_extensions: Some(
                            agent
                                .custom_extensions
                                .clone()
                                .into_iter()
                                .filter(|existing| {
                                    !custom_extension_matches_source_extension(
                                        existing,
                                        extension_id,
                                    )
                                })
                                .collect(),
                        ),
                        agent_domain: None,
                        agent_role: None,
                        owner_manager_agent_id: None,
                        template_source_agent_id: None,
                        allowed_groups: None,
                        max_concurrent_tasks: None,
                        temperature: None,
                        max_tokens: None,
                        context_limit: None,
                        thinking_enabled: None,
                        thinking_budget: None,
                        reasoning_effort: None,
                        output_reserve_tokens: None,
                        auto_compact_threshold: None,
                        supports_multimodal: None,
                        prompt_caching_mode: None,
                        cache_edit_mode: None,
                        assigned_skills: None,
                        skill_binding_mode: None,
                        delegation_policy: None,
                        attached_team_extensions: Some(next_refs),
                        auto_approve_chat: None,
                    },
                )
                .await
                .map_err(ServiceError::Database)?
            {
                updated_agents.push(updated);
            }
        }

        Ok(updated_agents)
    }

    /// Remove one team extension from every attached agent copy in the same team.
    pub async fn detach_team_extension_from_attached_agents(
        &self,
        team_id: &str,
        extension_id: &str,
    ) -> Result<Vec<TeamAgent>, ServiceError> {
        let attached_agents = self
            .list_agents_attached_to_team_extension(team_id, extension_id)
            .await?;
        let mut updated_agents = Vec::new();

        for agent in attached_agents {
            let mut next_refs = agent.attached_team_extensions.clone();
            let original_ref_len = next_refs.len();
            next_refs.retain(|existing| existing.extension_id != extension_id);
            let mut next_custom_extensions = agent.custom_extensions.clone();
            let original_custom_len = next_custom_extensions.len();
            next_custom_extensions.retain(|existing| {
                !custom_extension_matches_source_extension(existing, extension_id)
            });
            if next_refs.len() == original_ref_len
                && next_custom_extensions.len() == original_custom_len
            {
                continue;
            }

            if let Some(updated) = self
                .update_agent(
                    &agent.id,
                    UpdateAgentRequest {
                        name: None,
                        description: None,
                        avatar: None,
                        system_prompt: None,
                        api_url: None,
                        model: None,
                        api_key: None,
                        api_format: None,
                        status: None,
                        enabled_extensions: None,
                        custom_extensions: Some(next_custom_extensions),
                        agent_domain: None,
                        agent_role: None,
                        owner_manager_agent_id: None,
                        template_source_agent_id: None,
                        allowed_groups: None,
                        max_concurrent_tasks: None,
                        temperature: None,
                        max_tokens: None,
                        context_limit: None,
                        thinking_enabled: None,
                        thinking_budget: None,
                        reasoning_effort: None,
                        output_reserve_tokens: None,
                        auto_compact_threshold: None,
                        supports_multimodal: None,
                        prompt_caching_mode: None,
                        cache_edit_mode: None,
                        assigned_skills: None,
                        skill_binding_mode: None,
                        delegation_policy: None,
                        attached_team_extensions: Some(next_refs),
                        auto_approve_chat: None,
                    },
                )
                .await
                .map_err(ServiceError::Database)?
            {
                updated_agents.push(updated);
            }
        }

        Ok(updated_agents)
    }

    /// Add a custom MCP extension directly onto an agent.
    pub async fn add_custom_extension_to_agent(
        &self,
        agent_id: &str,
        extension: CustomExtensionConfig,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;
        let normalized = normalize_custom_extension_config(extension)?;
        if agent
            .custom_extensions
            .iter()
            .any(|existing| custom_extension_name_eq(&existing.name, &normalized.name))
        {
            return Err(ServiceError::Internal(format!(
                "Extension '{}' already exists in this agent",
                normalized.name
            )));
        }

        let mut next_custom_extensions = agent.custom_extensions.clone();
        next_custom_extensions.push(normalized);
        next_custom_extensions.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        });

        self.update_agent(
            agent_id,
            UpdateAgentRequest {
                name: None,
                description: None,
                avatar: None,
                system_prompt: None,
                api_url: None,
                model: None,
                api_key: None,
                api_format: None,
                status: None,
                enabled_extensions: None,
                custom_extensions: Some(next_custom_extensions),
                agent_domain: None,
                agent_role: None,
                owner_manager_agent_id: None,
                template_source_agent_id: None,
                allowed_groups: None,
                max_concurrent_tasks: None,
                temperature: None,
                max_tokens: None,
                context_limit: None,
                thinking_enabled: None,
                thinking_budget: None,
                reasoning_effort: None,
                output_reserve_tokens: None,
                auto_compact_threshold: None,
                supports_multimodal: None,
                prompt_caching_mode: None,
                cache_edit_mode: None,
                assigned_skills: None,
                skill_binding_mode: None,
                delegation_policy: None,
                attached_team_extensions: None,
                auto_approve_chat: None,
            },
        )
        .await
        .map_err(ServiceError::Database)
    }

    /// Enable or disable a custom MCP extension on an agent.
    pub async fn set_custom_extension_enabled(
        &self,
        agent_id: &str,
        extension_name: &str,
        enabled: bool,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;
        let mut next_custom_extensions = agent.custom_extensions.clone();
        let Some(existing) = next_custom_extensions
            .iter_mut()
            .find(|existing| custom_extension_name_eq(&existing.name, extension_name))
        else {
            return Err(ServiceError::Internal(format!(
                "Extension '{}' not found in this agent",
                extension_name
            )));
        };

        existing.enabled = enabled;

        self.update_agent(
            agent_id,
            UpdateAgentRequest {
                name: None,
                description: None,
                avatar: None,
                system_prompt: None,
                api_url: None,
                model: None,
                api_key: None,
                api_format: None,
                status: None,
                enabled_extensions: None,
                custom_extensions: Some(next_custom_extensions),
                agent_domain: None,
                agent_role: None,
                owner_manager_agent_id: None,
                template_source_agent_id: None,
                allowed_groups: None,
                max_concurrent_tasks: None,
                temperature: None,
                max_tokens: None,
                context_limit: None,
                thinking_enabled: None,
                thinking_budget: None,
                reasoning_effort: None,
                output_reserve_tokens: None,
                auto_compact_threshold: None,
                supports_multimodal: None,
                prompt_caching_mode: None,
                cache_edit_mode: None,
                assigned_skills: None,
                skill_binding_mode: None,
                delegation_policy: None,
                attached_team_extensions: None,
                auto_approve_chat: None,
            },
        )
        .await
        .map_err(ServiceError::Database)
    }

    /// Remove a custom MCP extension from an agent.
    pub async fn remove_custom_extension_from_agent(
        &self,
        agent_id: &str,
        extension_name: &str,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;
        let mut next_custom_extensions = agent.custom_extensions.clone();
        let original_len = next_custom_extensions.len();
        next_custom_extensions
            .retain(|existing| !custom_extension_name_eq(&existing.name, extension_name));
        if next_custom_extensions.len() == original_len {
            return Err(ServiceError::Internal(format!(
                "Extension '{}' not found in this agent",
                extension_name
            )));
        }

        self.update_agent(
            agent_id,
            UpdateAgentRequest {
                name: None,
                description: None,
                avatar: None,
                system_prompt: None,
                api_url: None,
                model: None,
                api_key: None,
                api_format: None,
                status: None,
                enabled_extensions: None,
                custom_extensions: Some(next_custom_extensions),
                agent_domain: None,
                agent_role: None,
                owner_manager_agent_id: None,
                template_source_agent_id: None,
                allowed_groups: None,
                max_concurrent_tasks: None,
                temperature: None,
                max_tokens: None,
                context_limit: None,
                thinking_enabled: None,
                thinking_budget: None,
                reasoning_effort: None,
                output_reserve_tokens: None,
                auto_compact_threshold: None,
                supports_multimodal: None,
                prompt_caching_mode: None,
                cache_edit_mode: None,
                assigned_skills: None,
                skill_binding_mode: None,
                delegation_policy: None,
                attached_team_extensions: None,
                auto_approve_chat: None,
            },
        )
        .await
        .map_err(ServiceError::Database)
    }

    // ========== Team Skill Bridge ==========

    /// Add a team shared skill to an agent's assigned_skills.
    pub async fn add_team_skill_to_agent(
        &self,
        agent_id: &str,
        skill_id: &str,
        team_id: &str,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        use agime_team::services::mongo::skill_service_mongo::SkillService;

        let skill_service = SkillService::new((*self.db).clone());

        // 1. Fetch the shared skill
        let skill = skill_service
            .get(skill_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?
            .ok_or_else(|| ServiceError::Internal("Skill not found".to_string()))?;

        // Verify team ownership
        if skill.team_id.to_hex() != team_id {
            return Err(ServiceError::Internal(
                "Skill does not belong to this team".to_string(),
            ));
        }
        if !agime_team::services::mongo::skill_service_mongo::SkillService::is_approved(
            &skill.review_status,
        ) {
            return Err(ServiceError::Internal(
                "Skill is not approved for runtime use".to_string(),
            ));
        }

        // 2. Get current agent
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;

        // 3. Check for duplicate skill_id
        let skill_id_hex = skill.id.map(|id| id.to_hex()).unwrap_or_default();
        if agent
            .assigned_skills
            .iter()
            .any(|s| s.skill_id == skill_id_hex)
        {
            return Err(ServiceError::Internal(format!(
                "Skill '{}' already assigned to this agent",
                skill.name
            )));
        }

        // 4. Build AgentSkillConfig
        let new_skill = AgentSkillConfig {
            skill_id: skill_id_hex,
            name: skill.name,
            description: skill.description.clone(),
            enabled: true,
            allowed_groups: Vec::new(),
            version: skill.version.clone(),
        };

        // 5. $push to assigned_skills
        let skill_bson = mongodb::bson::to_bson(&new_skill)
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        let now = Utc::now();
        self.agents()
            .update_one(
                doc! { "agent_id": agent_id },
                doc! {
                    "$push": { "assigned_skills": skill_bson },
                    "$set": { "updated_at": bson::DateTime::from_chrono(now) }
                },
                None,
            )
            .await?;

        Ok(self.get_agent(agent_id).await?)
    }

    /// Remove a skill from an agent's assigned_skills.
    pub async fn remove_skill_from_agent(
        &self,
        agent_id: &str,
        skill_id: &str,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        let now = Utc::now();
        self.agents()
            .update_one(
                doc! { "agent_id": agent_id },
                doc! {
                    "$pull": { "assigned_skills": { "skill_id": skill_id } },
                    "$set": { "updated_at": bson::DateTime::from_chrono(now) }
                },
                None,
            )
            .await?;

        Ok(self.get_agent(agent_id).await?)
    }

    /// List available team skills that are not yet assigned to the agent.
    pub async fn list_available_skills(
        &self,
        agent_id: &str,
        team_id: &str,
    ) -> Result<Vec<serde_json::Value>, ServiceError> {
        use agime_team::services::mongo::skill_service_mongo::SkillService;

        let skill_service = SkillService::new((*self.db).clone());

        // Get agent's currently assigned skill IDs
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;

        let assigned_ids: std::collections::HashSet<String> = agent
            .assigned_skills
            .iter()
            .map(|s| s.skill_id.clone())
            .collect();

        // Get all team skills via list() with large limit
        let result = skill_service
            .list_runtime_approved(team_id, Some(1), Some(200), None, None)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        // Filter out already assigned
        let available: Vec<serde_json::Value> = result
            .items
            .into_iter()
            .filter(|s| !assigned_ids.contains(&s.id))
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "name": s.name,
                    "description": s.description,
                    "version": s.version,
                })
            })
            .collect();

        Ok(available)
    }

    // ========== Session Management ==========

    /// Create a new agent session
    pub async fn create_session(
        &self,
        req: CreateSessionRequest,
    ) -> Result<AgentSessionDoc, mongodb::error::Error> {
        let session_source =
            Self::normalize_session_source(req.session_source.clone(), req.portal_restricted);
        let effective_allowed_skill_ids = self
            .resolve_session_allowed_skill_ids(
                &req.agent_id,
                req.allowed_skill_ids.clone(),
                &session_source,
                req.portal_restricted,
            )
            .await?;
        let session_id = Uuid::new_v4().to_string();
        let now = bson::DateTime::now();
        let hidden_from_chat_list = req.hidden_from_chat_list.unwrap_or_else(|| {
            req.portal_restricted
                || session_source == "system"
                || session_source == "document_analysis"
                || session_source == "agent_task"
                || session_source == "subagent"
                || session_source == "scheduled_task"
                || session_source == "portal_coding"
                || session_source == "portal_manager"
        });
        let document_policy = resolve_document_policy(
            req.document_access_mode.as_deref(),
            req.document_scope_mode.as_deref(),
            req.document_write_mode.as_deref(),
            Some(session_source.as_str()),
            req.portal_restricted,
        );

        let doc = AgentSessionDoc {
            id: None,
            session_id: session_id.clone(),
            team_id: req.team_id,
            agent_id: req.agent_id,
            user_id: req.user_id,
            name: req.name,
            status: "active".to_string(),
            messages_json: "[]".to_string(),
            message_count: 0,
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            context_runtime_state: None,
            disabled_extensions: Vec::new(),
            enabled_extensions: Vec::new(),
            created_at: now,
            updated_at: now,
            // Chat Track fields
            title: None,
            pinned: false,
            last_message_preview: None,
            last_message_at: None,
            is_processing: false,
            last_execution_status: None,
            last_execution_error: None,
            last_execution_finished_at: None,
            last_runtime_session_id: None,
            last_delegation_runtime: None,
            attached_document_ids: req.attached_document_ids,
            workspace_path: None,
            workspace_id: None,
            workspace_kind: None,
            workspace_manifest_path: None,
            thread_branch: None,
            thread_repo_ref: None,
            extra_instructions: req.extra_instructions,
            allowed_extensions: req.allowed_extensions,
            allowed_skill_ids: effective_allowed_skill_ids,
            retry_config: req.retry_config,
            max_turns: req.max_turns,
            tool_timeout_seconds: req.tool_timeout_seconds,
            max_portal_retry_rounds: req.max_portal_retry_rounds,
            require_final_report: req.require_final_report,
            portal_restricted: req.portal_restricted,
            document_access_mode: document_policy.document_access_mode,
            document_scope_mode: document_policy.document_scope_mode,
            document_write_mode: document_policy.document_write_mode,
            delegation_policy_override: req.delegation_policy_override,
            portal_id: None,
            portal_slug: None,
            visitor_id: None,
            session_source,
            source_channel_id: req.source_channel_id,
            source_channel_name: req.source_channel_name,
            source_thread_root_id: req.source_thread_root_id,
            hidden_from_chat_list,
            pending_message_workspace_files: Vec::new(),
        };

        self.sessions().insert_one(&doc, None).await?;

        Ok(self
            .sessions()
            .find_one(doc! { "session_id": &session_id }, None)
            .await?
            .unwrap_or(doc))
    }

    /// Get a session by session_id
    pub async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AgentSessionDoc>, mongodb::error::Error> {
        self.sessions()
            .find_one(doc! { "session_id": session_id }, None)
            .await
    }

    pub async fn find_active_channel_session(
        &self,
        channel_id: &str,
        thread_root_id: &str,
        agent_id: &str,
        session_source: &str,
    ) -> Result<Option<AgentSessionDoc>, mongodb::error::Error> {
        self.sessions()
            .find_one(
                doc! {
                    "status": "active",
                    "session_source": session_source,
                    "source_channel_id": channel_id,
                    "source_thread_root_id": thread_root_id,
                    "agent_id": agent_id,
                    "hidden_from_chat_list": true,
                },
                mongodb::options::FindOneOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await
    }

    pub async fn find_latest_channel_session(
        &self,
        channel_id: &str,
        thread_root_id: &str,
    ) -> Result<Option<AgentSessionDoc>, mongodb::error::Error> {
        self.sessions()
            .find_one(
                doc! {
                    "source_channel_id": channel_id,
                    "source_thread_root_id": thread_root_id,
                    "hidden_from_chat_list": true,
                },
                mongodb::options::FindOneOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await
    }

    pub async fn update_channel_session_runtime_context(
        &self,
        session_id: &str,
        attached_document_ids: Vec<String>,
        extra_instructions: Option<String>,
        require_final_report: bool,
    ) -> Result<(), mongodb::error::Error> {
        let mut set_doc = doc! {
            "attached_document_ids": attached_document_ids,
            "require_final_report": require_final_report,
            "updated_at": bson::DateTime::now(),
        };
        match extra_instructions {
            Some(value) if !value.trim().is_empty() => {
                set_doc.insert("extra_instructions", value);
            }
            _ => {
                set_doc.insert("extra_instructions", bson::Bson::Null);
            }
        }
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_session_workspace(
        &self,
        session_id: &str,
        path: &str,
    ) -> Result<(), mongodb::error::Error> {
        let normalized = normalize_workspace_path(path);
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$set": {
                        "workspace_path": normalized,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_session_workspace_binding(
        &self,
        session_id: &str,
        binding: &WorkspaceBinding,
    ) -> Result<(), mongodb::error::Error> {
        let normalized = normalize_workspace_path(&binding.root_path);
        let manifest_path = normalize_workspace_path(&binding.manifest_path);
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$set": {
                        "workspace_path": normalized,
                        "workspace_id": &binding.workspace_id,
                        "workspace_kind": binding.workspace_kind.as_str(),
                        "workspace_manifest_path": manifest_path,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_session_thread_repo_context(
        &self,
        session_id: &str,
        thread_branch: &str,
        thread_repo_ref: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$set": {
                        "thread_branch": thread_branch,
                        "thread_repo_ref": thread_repo_ref,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn clear_session_workspace_binding(
        &self,
        session_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$set": {
                        "workspace_path": bson::Bson::Null,
                        "workspace_id": bson::Bson::Null,
                        "workspace_kind": bson::Bson::Null,
                        "workspace_manifest_path": bson::Bson::Null,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn clear_session_thread_repo_context(
        &self,
        session_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$set": {
                        "thread_branch": bson::Bson::Null,
                        "thread_repo_ref": bson::Bson::Null,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_session_channel_context(
        &self,
        session_id: &str,
        channel_id: &str,
        channel_name: &str,
        thread_root_id: Option<&str>,
    ) -> Result<(), mongodb::error::Error> {
        let mut set_doc = doc! {
            "source_channel_id": channel_id,
            "updated_at": bson::DateTime::now(),
        };
        if !channel_name.trim().is_empty() {
            set_doc.insert("source_channel_name", channel_name.trim());
        } else {
            set_doc.insert("source_channel_name", bson::Bson::Null);
        }
        match thread_root_id {
            Some(value) if !value.trim().is_empty() => {
                set_doc.insert("source_thread_root_id", value.trim());
            }
            _ => {
                set_doc.insert("source_thread_root_id", bson::Bson::Null);
            }
        }
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_session_portal_context(
        &self,
        session_id: &str,
        portal_id: &str,
        portal_slug: &str,
        visitor_id: Option<&str>,
        document_access_mode: Option<&str>,
        portal_restricted: bool,
    ) -> Result<(), mongodb::error::Error> {
        let document_policy = resolve_document_policy(
            document_access_mode,
            None,
            None,
            Some("portal"),
            portal_restricted,
        );
        let mut set_doc = doc! {
            "portal_restricted": portal_restricted,
            "portal_id": portal_id,
            "portal_slug": portal_slug,
            "updated_at": bson::DateTime::now(),
        };
        match document_policy.document_access_mode {
            Some(mode) if !mode.trim().is_empty() => {
                set_doc.insert("document_access_mode", mode);
            }
            _ => {
                set_doc.insert("document_access_mode", bson::Bson::Null);
            }
        }
        match document_policy.document_scope_mode {
            Some(mode) if !mode.trim().is_empty() => {
                set_doc.insert("document_scope_mode", mode);
            }
            _ => {
                set_doc.insert("document_scope_mode", bson::Bson::Null);
            }
        }
        match document_policy.document_write_mode {
            Some(mode) if !mode.trim().is_empty() => {
                set_doc.insert("document_write_mode", mode);
            }
            _ => {
                set_doc.insert("document_write_mode", bson::Bson::Null);
            }
        }
        match visitor_id {
            Some(value) if !value.trim().is_empty() => {
                set_doc.insert("visitor_id", value.trim());
            }
            _ => {
                set_doc.insert("visitor_id", bson::Bson::Null);
            }
        }
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn sync_portal_session_policy(
        &self,
        session_id: &str,
        attached_document_ids: Vec<String>,
        extra_instructions: Option<String>,
        allowed_extensions: Option<Vec<String>>,
        allowed_skill_ids: Option<Vec<String>>,
        delegation_policy_override: Option<agime_team::models::DelegationPolicyOverride>,
        retry_config: Option<RetryConfig>,
        require_final_report: bool,
        document_access_mode: Option<String>,
    ) -> Result<(), mongodb::error::Error> {
        let document_policy = resolve_document_policy(
            document_access_mode.as_deref(),
            None,
            None,
            Some("portal"),
            true,
        );
        let mut set_doc = doc! {
            "attached_document_ids": attached_document_ids,
            "portal_restricted": true,
            "require_final_report": require_final_report,
            "updated_at": bson::DateTime::now(),
        };
        match document_policy.document_access_mode {
            Some(value) if !value.trim().is_empty() => {
                set_doc.insert("document_access_mode", value);
            }
            _ => {
                set_doc.insert("document_access_mode", bson::Bson::Null);
            }
        }
        match document_policy.document_scope_mode {
            Some(value) if !value.trim().is_empty() => {
                set_doc.insert("document_scope_mode", value);
            }
            _ => {
                set_doc.insert("document_scope_mode", bson::Bson::Null);
            }
        }
        match document_policy.document_write_mode {
            Some(value) if !value.trim().is_empty() => {
                set_doc.insert("document_write_mode", value);
            }
            _ => {
                set_doc.insert("document_write_mode", bson::Bson::Null);
            }
        }
        match extra_instructions {
            Some(value) if !value.trim().is_empty() => {
                set_doc.insert("extra_instructions", value);
            }
            _ => {
                set_doc.insert("extra_instructions", bson::Bson::Null);
            }
        }
        match allowed_extensions {
            Some(value) => {
                set_doc.insert(
                    "allowed_extensions",
                    mongodb::bson::to_bson(&value).unwrap_or(bson::Bson::Array(vec![])),
                );
            }
            None => {
                set_doc.insert("allowed_extensions", bson::Bson::Null);
            }
        }
        match allowed_skill_ids {
            Some(value) => {
                set_doc.insert(
                    "allowed_skill_ids",
                    mongodb::bson::to_bson(&value).unwrap_or(bson::Bson::Array(vec![])),
                );
            }
            None => {
                set_doc.insert("allowed_skill_ids", bson::Bson::Null);
            }
        }
        match retry_config {
            Some(value) => {
                set_doc.insert(
                    "retry_config",
                    mongodb::bson::to_bson(&value).unwrap_or(bson::Bson::Null),
                );
            }
            None => {
                set_doc.insert("retry_config", bson::Bson::Null);
            }
        }
        match delegation_policy_override {
            Some(value) => {
                set_doc.insert(
                    "delegation_policy_override",
                    mongodb::bson::to_bson(&value).unwrap_or(bson::Bson::Null),
                );
            }
            None => {
                set_doc.insert("delegation_policy_override", bson::Bson::Null);
            }
        }
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    /// Find an active (non-deleted) session for a given user + agent pair.
    /// Returns the most recently updated session, if any.
    pub async fn find_active_session_by_user(
        &self,
        user_id: &str,
        agent_id: &str,
    ) -> Result<Option<AgentSessionDoc>, mongodb::error::Error> {
        let opts = mongodb::options::FindOneOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .build();
        self.sessions()
            .find_one(
                doc! {
                    "user_id": user_id,
                    "agent_id": agent_id,
                    "status": "active",
                },
                opts,
            )
            .await
    }

    /// Find an active restricted portal session by user + agent + portal.
    /// Returns the most recently updated session, if any.
    pub async fn find_active_portal_session(
        &self,
        user_id: &str,
        agent_id: &str,
        portal_id: &str,
    ) -> Result<Option<AgentSessionDoc>, mongodb::error::Error> {
        let opts = mongodb::options::FindOneOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .build();
        self.sessions()
            .find_one(
                doc! {
                    "user_id": user_id,
                    "agent_id": agent_id,
                    "status": "active",
                    "portal_restricted": true,
                    "portal_id": portal_id,
                },
                opts,
            )
            .await
    }

    /// List portal-scoped sessions for a visitor (M-4: filter by portal_id).
    pub async fn list_portal_sessions(
        &self,
        portal_id: &str,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<AgentSessionDoc>, mongodb::error::Error> {
        use futures::TryStreamExt;
        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .limit(limit)
            .build();
        let cursor = self
            .sessions()
            .find(
                doc! {
                    "user_id": user_id,
                    "portal_id": portal_id,
                    "portal_restricted": true,
                },
                opts,
            )
            .await?;
        cursor.try_collect().await
    }

    /// Update session messages and token stats (called by executor after each loop)
    pub async fn update_session_messages(
        &self,
        session_id: &str,
        messages_json: &str,
        message_count: i32,
        total_tokens: Option<i32>,
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
        context_runtime_state: Option<&ContextRuntimeState>,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        let mut set = doc! {
            "messages_json": messages_json,
            "message_count": message_count,
            "updated_at": now,
        };
        if let Some(t) = total_tokens {
            set.insert("total_tokens", t);
        }
        if let Some(t) = input_tokens {
            set.insert("input_tokens", t);
        }
        if let Some(t) = output_tokens {
            set.insert("output_tokens", t);
        }
        if let Some(state) = context_runtime_state {
            if let Ok(serialized_state) = mongodb::bson::to_bson(state) {
                set.insert("context_runtime_state", serialized_state);
            }
        }

        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_session_context_runtime_state(
        &self,
        session_id: &str,
        context_runtime_state: &ContextRuntimeState,
    ) -> Result<(), mongodb::error::Error> {
        let mut set = doc! {
            "updated_at": bson::DateTime::now(),
        };
        if let Ok(serialized_state) = mongodb::bson::to_bson(context_runtime_state) {
            set.insert("context_runtime_state", serialized_state);
        }

        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set },
                None,
            )
            .await?;
        Ok(())
    }

    /// Update session extension overrides (disabled/enabled extensions)
    pub async fn update_session_extensions(
        &self,
        session_id: &str,
        disabled_extensions: &[String],
        enabled_extensions: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$set": {
                        "disabled_extensions": disabled_extensions,
                        "enabled_extensions": enabled_extensions,
                        "updated_at": now,
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// List sessions for an agent
    pub async fn list_sessions(
        &self,
        query: SessionListQuery,
    ) -> Result<Vec<AgentSessionDoc>, mongodb::error::Error> {
        let clamped_limit = query.limit.min(100) as i64;
        let skip = ((query.page.saturating_sub(1)) * query.limit.min(100)) as u64;

        let mut filter = doc! {
            "team_id": &query.team_id,
            "agent_id": &query.agent_id,
        };
        if let Some(ref uid) = query.user_id {
            filter.insert("user_id", uid);
        }

        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .skip(skip)
            .limit(clamped_limit)
            .build();

        let cursor = self.sessions().find(filter, options).await?;
        cursor.try_collect().await
    }

    /// Archive a session
    pub async fn archive_session(&self, session_id: &str) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": {
                    "status": "archived",
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Atomically archive a session only if it is not currently processing.
    pub async fn archive_session_if_idle(
        &self,
        session_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let now = bson::DateTime::now();
        let result = self
            .sessions()
            .update_one(
                doc! { "session_id": session_id, "is_processing": false },
                doc! { "$set": {
                    "status": "archived",
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(result.modified_count > 0)
    }

    // ========== Direct chat methods ==========

    /// List user sessions with lightweight items (no messages_json).
    /// Joins agent_name from team_agents collection.
    pub async fn list_user_sessions(
        &self,
        query: UserSessionListQuery,
    ) -> Result<Vec<SessionListItem>, mongodb::error::Error> {
        let clamped_limit = query.limit.min(100) as i64;
        let skip = ((query.page.saturating_sub(1)) * query.limit.min(100)) as u64;

        let mut filter = doc! { "team_id": &query.team_id };
        // C1 fix: Always filter by user_id to prevent data leakage
        if let Some(ref user_id) = query.user_id {
            filter.insert("user_id", user_id);
        }
        if let Some(ref agent_id) = query.agent_id {
            filter.insert("agent_id", agent_id);
        }
        if let Some(ref status) = query.status {
            filter.insert("status", status);
        } else {
            filter.insert("status", "active");
        }
        if !query.include_hidden {
            filter.insert(
                "$or",
                vec![
                    doc! { "hidden_from_chat_list": { "$exists": false } },
                    doc! { "hidden_from_chat_list": false },
                ],
            );
        }

        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "pinned": -1, "last_message_at": -1, "updated_at": -1 })
            .skip(skip)
            .limit(clamped_limit)
            .projection(doc! {
                "session_id": 1, "agent_id": 1, "title": 1,
                "last_message_preview": 1, "last_message_at": 1,
                "message_count": 1, "status": 1, "pinned": 1,
                "created_at": 1, "user_id": 1,
                "team_id": 1, "updated_at": 1,
            })
            .build();

        let cursor = self.sessions().find(filter, options).await?;
        let sessions: Vec<AgentSessionDoc> = cursor.try_collect().await?;

        // Batch-fetch agent names
        let agent_ids: Vec<&str> = sessions.iter().map(|s| s.agent_id.as_str()).collect();
        let agent_names = self.batch_get_agent_names(&agent_ids).await;

        let items = sessions
            .into_iter()
            .map(|s| {
                let agent_name = agent_names
                    .get(&s.agent_id)
                    .cloned()
                    .unwrap_or_else(|| s.agent_id.clone());
                SessionListItem {
                    session_id: s.session_id,
                    agent_id: s.agent_id,
                    agent_name,
                    title: s.title,
                    last_message_preview: s.last_message_preview,
                    last_message_at: s.last_message_at.map(|d| d.to_chrono().to_rfc3339()),
                    message_count: s.message_count,
                    status: s.status,
                    pinned: s.pinned,
                    created_at: s.created_at.to_chrono().to_rfc3339(),
                }
            })
            .collect();

        Ok(items)
    }

    /// Backfill session source/visibility fields for existing data.
    /// Safe to run repeatedly (idempotent best effort).
    pub async fn backfill_session_source_and_visibility(
        &self,
    ) -> Result<(), mongodb::error::Error> {
        // 1) Portal sessions -> portal source + hidden.
        let _ = self
            .sessions()
            .update_many(
                doc! { "portal_restricted": true },
                doc! { "$set": { "session_source": "portal", "hidden_from_chat_list": true } },
                None,
            )
            .await?;

        // 1.5) Portal coding sessions (legacy rows may have been stored as chat)
        // detect by presence of portal + workspace context and mark as hidden.
        let _ = self
            .sessions()
            .update_many(
                doc! {
                    "portal_restricted": { "$ne": true },
                    "portal_id": { "$exists": true, "$ne": bson::Bson::Null },
                    "workspace_path": { "$exists": true, "$ne": bson::Bson::Null },
                },
                doc! { "$set": { "session_source": "portal_coding", "hidden_from_chat_list": true } },
                None,
            )
            .await?;

        // 2) Remaining sessions with missing source -> default chat + visible.
        let _ = self
            .sessions()
            .update_many(
                doc! { "session_source": { "$exists": false } },
                doc! { "$set": { "session_source": "chat" } },
                None,
            )
            .await?;
        let _ = self
            .sessions()
            .update_many(
                doc! { "hidden_from_chat_list": { "$exists": false } },
                doc! { "$set": { "hidden_from_chat_list": false } },
                None,
            )
            .await?;
        Ok(())
    }

    /// Batch-fetch agent names by IDs
    async fn batch_get_agent_names(
        &self,
        agent_ids: &[&str],
    ) -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();
        if agent_ids.is_empty() {
            return map;
        }
        let unique: std::collections::HashSet<&str> = agent_ids.iter().copied().collect();
        let ids_bson: Vec<bson::Bson> = unique
            .iter()
            .map(|id| bson::Bson::String(id.to_string()))
            .collect();

        let filter = doc! { "agent_id": { "$in": ids_bson } };
        let opts = mongodb::options::FindOptions::builder()
            .projection(doc! { "agent_id": 1, "name": 1 })
            .build();

        if let Ok(cursor) = self.agents().find(filter, opts).await {
            if let Ok(docs) = cursor.try_collect::<Vec<_>>().await {
                for d in docs {
                    map.insert(d.agent_id.clone(), d.name.clone());
                }
            }
        }
        map
    }

    /// Create a chat session for a specific agent, optionally with attached documents
    #[allow(clippy::too_many_arguments)]
    pub async fn create_chat_session(
        &self,
        team_id: &str,
        agent_id: &str,
        user_id: &str,
        attached_document_ids: Vec<String>,
        extra_instructions: Option<String>,
        allowed_extensions: Option<Vec<String>>,
        allowed_skill_ids: Option<Vec<String>>,
        retry_config: Option<RetryConfig>,
        max_turns: Option<i32>,
        tool_timeout_seconds: Option<u64>,
        max_portal_retry_rounds: Option<u32>,
        require_final_report: bool,
        portal_restricted: bool,
        document_access_mode: Option<String>,
        delegation_policy_override: Option<agime_team::models::DelegationPolicyOverride>,
        session_source: Option<String>,
        hidden_from_chat_list: Option<bool>,
    ) -> Result<AgentSessionDoc, mongodb::error::Error> {
        self.create_session(CreateSessionRequest {
            team_id: team_id.to_string(),
            agent_id: agent_id.to_string(),
            user_id: user_id.to_string(),
            name: None,
            attached_document_ids,
            extra_instructions,
            allowed_extensions,
            allowed_skill_ids,
            retry_config,
            max_turns,
            tool_timeout_seconds,
            max_portal_retry_rounds,
            require_final_report,
            portal_restricted,
            document_access_mode,
            document_scope_mode: None,
            document_write_mode: None,
            delegation_policy_override,
            session_source,
            source_channel_id: None,
            source_channel_name: None,
            source_thread_root_id: None,
            hidden_from_chat_list,
        })
        .await
    }

    /// Rename a session
    pub async fn rename_session(
        &self,
        session_id: &str,
        title: &str,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": { "title": title, "updated_at": now } },
                None,
            )
            .await?;
        Ok(())
    }

    /// Pin or unpin a session
    pub async fn pin_session(
        &self,
        session_id: &str,
        pinned: bool,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": { "pinned": pinned, "updated_at": now } },
                None,
            )
            .await?;
        Ok(())
    }

    /// Permanently delete a session
    pub async fn delete_session(&self, session_id: &str) -> Result<bool, mongodb::error::Error> {
        let result = self
            .sessions()
            .delete_one(doc! { "session_id": session_id }, None)
            .await?;
        if result.deleted_count > 0 {
            let _ = self
                .chat_events()
                .delete_many(doc! { "session_id": session_id }, None)
                .await?;
        }
        Ok(result.deleted_count > 0)
    }

    /// Atomically delete a session only if it is not currently processing.
    pub async fn delete_session_if_idle(
        &self,
        session_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let result = self
            .sessions()
            .delete_one(
                doc! { "session_id": session_id, "is_processing": false },
                None,
            )
            .await?;
        if result.deleted_count > 0 {
            let _ = self
                .chat_events()
                .delete_many(doc! { "session_id": session_id }, None)
                .await?;
        }
        Ok(result.deleted_count > 0)
    }

    /// Attach documents to a session
    pub async fn attach_documents_to_session(
        &self,
        session_id: &str,
        document_ids: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$addToSet": { "attached_document_ids": { "$each": document_ids } },
                    "$set": { "updated_at": now },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Detach documents from a session
    pub async fn detach_documents_from_session(
        &self,
        session_id: &str,
        document_ids: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$pullAll": { "attached_document_ids": document_ids },
                    "$set": { "updated_at": now },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Set session processing state (prevents concurrent sends)
    pub async fn set_session_processing(
        &self,
        session_id: &str,
        is_processing: bool,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": {
                    "is_processing": is_processing,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_session_runtime_session_id(
        &self,
        session_id: &str,
        runtime_session_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": {
                    "last_runtime_session_id": runtime_session_id,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Atomically try to set is_processing = true, only if currently false.
    /// Returns Ok(true) if successfully claimed, Ok(false) if already processing.
    /// This prevents TOCTOU race conditions on concurrent send_message calls.
    pub async fn try_start_processing(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let now = bson::DateTime::now();
        // Try normal claim: is_processing == false
        let result = self
            .sessions()
            .find_one_and_update(
                doc! {
                    "session_id": session_id,
                    "user_id": user_id,
                    "is_processing": false,
                },
                doc! { "$set": {
                    "is_processing": true,
                    "last_execution_status": "running",
                    "last_execution_error": bson::Bson::Null,
                    "last_execution_finished_at": bson::Bson::Null,
                    "last_delegation_runtime": bson::Bson::Null,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        if result.is_some() {
            return Ok(true);
        }
        // Auto-recover: if stuck processing > 10 minutes, force claim
        let stale_cutoff =
            bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::minutes(10));
        let recovered = self
            .sessions()
            .find_one_and_update(
                doc! {
                    "session_id": session_id,
                    "user_id": user_id,
                    "is_processing": true,
                    "updated_at": { "$lt": stale_cutoff },
                },
                doc! { "$set": {
                    "is_processing": true,
                    "last_execution_status": "running",
                    "last_execution_error": bson::Bson::Null,
                    "last_execution_finished_at": bson::Bson::Null,
                    "last_delegation_runtime": bson::Bson::Null,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        if recovered.is_some() {
            tracing::warn!("Auto-recovered stuck session {}", session_id);
        }
        Ok(recovered.is_some())
    }

    /// Reset stuck `is_processing` flags for sessions that have been processing
    /// longer than the given timeout. This recovers from crashed/timed-out executions.
    pub async fn reset_stuck_processing(
        &self,
        max_age: std::time::Duration,
    ) -> Result<u64, mongodb::error::Error> {
        let cutoff = bson::DateTime::from_chrono(
            chrono::Utc::now() - chrono::Duration::from_std(max_age).unwrap_or_default(),
        );
        let now = bson::DateTime::now();
        let result = self
            .sessions()
            .update_many(
                doc! {
                    "is_processing": true,
                    "updated_at": { "$lt": cutoff },
                },
                doc! { "$set": {
                    "is_processing": false,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(result.modified_count)
    }

    /// Update session metadata after a message completes
    pub async fn update_session_after_message(
        &self,
        session_id: &str,
        messages_json: &str,
        message_count: i32,
        last_preview: &str,
        title: Option<&str>,
        tokens: Option<i32>,
        context_runtime_state: Option<&ContextRuntimeState>,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        // H2 fix: use chars() for safe Unicode slicing
        let preview: String = if last_preview.chars().count() > 200 {
            let truncated: String = last_preview.chars().take(197).collect();
            format!("{}...", truncated)
        } else {
            last_preview.to_string()
        };

        let mut set = doc! {
            "messages_json": messages_json,
            "message_count": message_count,
            "last_message_preview": &preview,
            "last_message_at": now,
            "is_processing": false,
            "updated_at": now,
        };
        if let Some(t) = title {
            set.insert("title", t);
        }
        if let Some(t) = tokens {
            set.insert("total_tokens", t);
        }
        if let Some(state) = context_runtime_state {
            if let Ok(serialized_state) = mongodb::bson::to_bson(state) {
                set.insert("context_runtime_state", serialized_state);
            }
        }

        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set },
                None,
            )
            .await?;
        Ok(())
    }

    /// Persist the result of the most recent send_message execution without
    /// changing the session lifecycle status (active/archived).
    pub async fn update_session_execution_result(
        &self,
        session_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        let mut set = doc! {
            "is_processing": false,
            "last_execution_status": status,
            "last_execution_finished_at": now,
            "updated_at": now,
        };
        set.insert(
            "last_execution_error",
            error
                .map(|value| bson::Bson::String(value.to_string()))
                .unwrap_or(bson::Bson::Null),
        );
        let filter = if status.eq_ignore_ascii_case("cancelled") {
            doc! { "session_id": session_id }
        } else {
            doc! {
                "session_id": session_id,
                "last_execution_status": { "$ne": "cancelled" },
            }
        };
        self.sessions()
            .update_one(filter, doc! { "$set": set }, None)
            .await?;
        Ok(())
    }

    pub async fn set_session_execution_state(
        &self,
        session_id: &str,
        status: &str,
        is_processing: bool,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        let update = doc! {
            "$set": {
                "is_processing": is_processing,
                "last_execution_status": status,
                "last_execution_error": bson::Bson::Null,
                "last_execution_finished_at": bson::Bson::Null,
                "updated_at": now,
            }
        };
        self.sessions()
            .update_one(doc! { "session_id": session_id }, update, None)
            .await?;
        Ok(())
    }

    pub async fn update_session_delegation_runtime(
        &self,
        session_id: &str,
        delegation_runtime: Option<&DelegationRuntimeResponse>,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        let value = delegation_runtime
            .map(mongodb::bson::to_bson)
            .transpose()?
            .unwrap_or(bson::Bson::Null);
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": {
                    "last_delegation_runtime": value,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn update_session_total_tokens(
        &self,
        session_id: &str,
        total_tokens: i32,
    ) -> Result<(), mongodb::error::Error> {
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$set": {
                        "total_tokens": total_tokens,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn update_session_preview(
        &self,
        session_id: &str,
        preview: &str,
    ) -> Result<(), mongodb::error::Error> {
        let preview = preview.trim();
        if preview.is_empty() {
            return Ok(());
        }
        let limited = if preview.chars().count() > 200 {
            let truncated: String = preview.chars().take(197).collect();
            format!("{}...", truncated)
        } else {
            preview.to_string()
        };
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$set": {
                        "last_message_preview": limited,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn queue_pending_message_workspace_file(
        &self,
        session_id: &str,
        block: &ChatWorkspaceFileBlock,
    ) -> Result<(), mongodb::error::Error> {
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$push": {
                        "pending_message_workspace_files": mongodb::bson::to_bson(block)
                            .unwrap_or(Bson::Null)
                    },
                    "$set": {
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn list_pending_message_workspace_files(
        &self,
        session_id: &str,
    ) -> Result<Vec<ChatWorkspaceFileBlock>, mongodb::error::Error> {
        Ok(self
            .get_session(session_id)
            .await?
            .map(|session| session.pending_message_workspace_files)
            .unwrap_or_default())
    }

    pub async fn clear_pending_message_workspace_files(
        &self,
        session_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$set": {
                        "pending_message_workspace_files": Vec::<Bson>::new(),
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn consume_pending_message_workspace_files(
        &self,
        session_id: &str,
    ) -> Result<Vec<ChatWorkspaceFileBlock>, mongodb::error::Error> {
        use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

        let previous = self
            .sessions()
            .find_one_and_update(
                doc! { "session_id": session_id },
                doc! {
                    "$set": {
                        "pending_message_workspace_files": Vec::<Bson>::new(),
                        "updated_at": bson::DateTime::now(),
                    }
                },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::Before)
                    .build(),
            )
            .await?;

        Ok(previous
            .map(|session| session.pending_message_workspace_files)
            .unwrap_or_default())
    }

    async fn append_session_notice_internal(
        &self,
        session_id: &str,
        text: &str,
        user_visible: bool,
        agent_visible: bool,
    ) -> Result<(), mongodb::error::Error> {
        let notice = text.trim();
        if notice.is_empty() {
            return Ok(());
        }
        let Some(session) = self.get_session(session_id).await? else {
            return Ok(());
        };
        let mut messages: Vec<serde_json::Value> =
            serde_json::from_str(&session.messages_json).unwrap_or_default();
        let duplicate_last = messages.last().and_then(|msg| {
            let role = msg.get("role").and_then(serde_json::Value::as_str)?;
            if role != "assistant" {
                return None;
            }
            let content = msg.get("content")?.as_array()?;
            let first = content.first()?;
            first.get("text").and_then(serde_json::Value::as_str)
        }) == Some(notice);
        if duplicate_last {
            return Ok(());
        }

        messages.push(serde_json::json!({
            "id": null,
            "role": "assistant",
            "created": chrono::Utc::now().timestamp(),
            "content": [
                {
                    "type": "text",
                    "text": notice,
                }
            ],
            "metadata": {
                "userVisible": user_visible,
                "agentVisible": agent_visible,
                "systemGenerated": true,
            }
        }));

        let messages_json =
            serde_json::to_string(&messages).unwrap_or_else(|_| session.messages_json.clone());
        if user_visible {
            let preview = notice.to_string();
            self.update_session_after_message(
                session_id,
                &messages_json,
                messages.len() as i32,
                &preview,
                session.title.as_deref(),
                session.total_tokens,
                session.context_runtime_state.as_ref(),
            )
            .await
        } else {
            self.sessions()
                .update_one(
                    doc! { "session_id": session_id },
                    doc! {
                        "$set": {
                            "messages_json": &messages_json,
                            "updated_at": bson::DateTime::now(),
                        }
                    },
                    None,
                )
                .await?;
            Ok(())
        }
    }

    /// Append a hidden assistant-visible system notice into an existing session.
    /// This is useful when runtime policy changes and the agent must treat old
    /// conversation assumptions as stale without cluttering the user's UI.
    pub async fn append_hidden_session_notice(
        &self,
        session_id: &str,
        text: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.append_session_notice_internal(session_id, text, false, true)
            .await
    }

    /// Append a visible assistant system notice into an existing session so the
    /// user can continue a runtime conversation with awareness of the latest run.
    pub async fn append_visible_session_notice(
        &self,
        session_id: &str,
        text: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.append_session_notice_internal(session_id, text, true, true)
            .await
    }

    /// Persist chat runtime stream events for replay/analysis.
    pub async fn save_chat_stream_events(
        &self,
        items: &[(String, String, u64, StreamEvent)],
    ) -> Result<(), mongodb::error::Error> {
        if items.is_empty() {
            return Ok(());
        }

        let docs: Vec<ChatEventDoc> = items
            .iter()
            .map(|(session_id, run_id, event_id, event)| ChatEventDoc {
                id: None,
                session_id: session_id.clone(),
                run_id: Some(run_id.clone()),
                event_id: (*event_id).try_into().unwrap_or(i64::MAX),
                event_type: event.event_type().to_string(),
                payload: serde_json::to_value(event).unwrap_or_else(|_| serde_json::json!({})),
                created_at: bson::DateTime::now(),
            })
            .collect();

        // Best-effort retry for transient write failures.
        // Use unordered insert so one duplicate key does not abort the whole batch.
        let opts = mongodb::options::InsertManyOptions::builder()
            .ordered(false)
            .build();
        let mut attempt: u8 = 0;
        loop {
            match self
                .chat_events()
                .insert_many(docs.clone(), opts.clone())
                .await
            {
                Ok(_) => return Ok(()),
                Err(e) => {
                    let msg = e.to_string();
                    let duplicate_only = msg.contains("E11000")
                        || msg.to_ascii_lowercase().contains("duplicate key");
                    if duplicate_only {
                        return Ok(());
                    }

                    attempt = attempt.saturating_add(1);
                    if attempt >= 3 {
                        return Err(e);
                    }

                    let backoff_ms = 50_u64 * (attempt as u64);
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                }
            }
        }
    }

    pub async fn list_chat_events(
        &self,
        session_id: &str,
        run_id: Option<&str>,
        after_event_id: Option<u64>,
        before_event_id: Option<u64>,
        limit: u32,
        descending: bool,
    ) -> Result<Vec<ChatEventDoc>, mongodb::error::Error> {
        let clamped_limit = limit.clamp(1, 2000);
        let mut filter = doc! { "session_id": session_id };
        if let Some(run) = run_id {
            filter.insert("run_id", run);
        }
        let mut event_id_filter = Document::new();
        if let Some(after) = after_event_id {
            event_id_filter.insert("$gt", after as i64);
        }
        if let Some(before) = before_event_id {
            event_id_filter.insert("$lt", before as i64);
        }
        if !event_id_filter.is_empty() {
            filter.insert("event_id", event_id_filter);
        }
        let sort_dir = if descending { -1 } else { 1 };

        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "event_id": sort_dir, "created_at": sort_dir })
            .limit(clamped_limit as i64)
            .build();
        let mut events: Vec<ChatEventDoc> = self
            .chat_events()
            .find(filter.clone(), opts.clone())
            .await?
            .try_collect()
            .await?;

        // Handle run switches gracefully:
        // if caller sends a stale `after_event_id` from a previous run,
        // restart from beginning of current run instead of returning empty forever.
        if !descending && before_event_id.is_none() && events.is_empty() {
            if let (Some(run), Some(after)) = (run_id, after_event_id) {
                let max_opts = mongodb::options::FindOneOptions::builder()
                    .sort(doc! { "event_id": -1, "created_at": -1 })
                    .build();
                let max_doc = self
                    .chat_events()
                    .find_one(doc! { "session_id": session_id, "run_id": run }, max_opts)
                    .await?;
                if let Some(latest) = max_doc {
                    if latest.event_id < after as i64 {
                        let restart_filter = doc! { "session_id": session_id, "run_id": run };
                        events = self
                            .chat_events()
                            .find(restart_filter, opts)
                            .await?
                            .try_collect()
                            .await?;
                    }
                }
            }
        }

        Ok(events)
    }

    // ═══════════════════════════════════════════════════════
}

#[cfg(test)]
mod tests {
    use super::AgentService;

    #[test]
    fn normalize_session_source_keeps_portal_sources_stable() {
        assert_eq!(
            AgentService::normalize_session_source(Some("portal".to_string()), true),
            "portal"
        );
        assert_eq!(
            AgentService::normalize_session_source(Some("portal-coding".to_string()), true),
            "portal_coding"
        );
        assert_eq!(
            AgentService::normalize_session_source(Some("portal_manager".to_string()), true),
            "portal_manager"
        );
        assert_eq!(
            AgentService::normalize_session_source(Some("automation_builder".to_string()), false),
            "automation_builder"
        );
        assert_eq!(
            AgentService::normalize_session_source(Some("automation_runtime".to_string()), false),
            "automation_runtime"
        );
    }

    #[test]
    fn normalize_session_source_falls_back_by_boundary() {
        assert_eq!(
            AgentService::normalize_session_source(Some("unknown".to_string()), true),
            "portal"
        );
        assert_eq!(
            AgentService::normalize_session_source(Some("unknown".to_string()), false),
            "chat"
        );
        assert_eq!(AgentService::normalize_session_source(None, true), "portal");
        assert_eq!(AgentService::normalize_session_source(None, false), "chat");
    }
}
