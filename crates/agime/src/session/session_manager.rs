use crate::config::paths::Paths;
use crate::conversation::message::{Message, MessageContent, SystemNotificationType};
use crate::conversation::Conversation;
use crate::model::ModelConfig;
use crate::providers::base::{Provider, MSG_COUNT_FOR_SESSION_NAME_GENERATION};
use crate::recipe::Recipe;
use crate::session::extension_data::ExtensionData;
use anyhow::Result;
use chrono::{DateTime, Utc};
use regex::Regex;
use rmcp::model::Role;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Pool, Sqlite};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::OnceCell;
use tracing::{info, warn};
use utoipa::ToSchema;
use uuid::Uuid;

const CURRENT_SCHEMA_VERSION: i32 = 11;
pub const SESSIONS_FOLDER: &str = "sessions";
pub const DB_NAME: &str = "sessions.db";
const MEMORY_SOURCE_CFPM_AUTO: &str = "cfpm_auto";
const MEMORY_SOURCE_USER: &str = "user";
const DEFAULT_MEMORY_CONFIDENCE_USER: f64 = 1.0;
const DEFAULT_MEMORY_CONFIDENCE_CFPM: f64 = 0.7;
const DEFAULT_MEMORY_CONFIDENCE_INVALID_PATH: f64 = 0.9;
const MIN_MEMORY_CONFIDENCE: f64 = 0.05;
const MAX_MEMORY_CONFIDENCE: f64 = 1.0;
const MAX_MEMORY_SNAPSHOTS_PER_SESSION: i64 = 100;
const MAX_MEMORY_CANDIDATES_PER_SESSION: i64 = 800;
const MAX_CFPM_AUTO_FACTS: usize = 120;
const MAX_CFPM_TOOL_GATE_EVENTS_LIMIT: u32 = 200;
const DEFAULT_CFPM_TOOL_GATE_EVENTS_LIMIT: u32 = 30;
const CFPM_TOOL_GATE_NOTIFICATION_PREFIX: &str = "[CFPM_TOOL_GATE_V1]";

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryFactStatus {
    Active,
    Stale,
    Forgotten,
    Superseded,
}

impl Default for MemoryFactStatus {
    fn default() -> Self {
        Self::Active
    }
}

impl std::fmt::Display for MemoryFactStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Stale => write!(f, "stale"),
            Self::Forgotten => write!(f, "forgotten"),
            Self::Superseded => write!(f, "superseded"),
        }
    }
}

impl std::str::FromStr for MemoryFactStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "stale" => Ok(Self::Stale),
            "forgotten" => Ok(Self::Forgotten),
            "superseded" => Ok(Self::Superseded),
            _ => Err(anyhow::anyhow!("Invalid memory fact status: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFact {
    pub id: String,
    pub session_id: String,
    pub category: String,
    pub content: String,
    pub status: MemoryFactStatus,
    pub pinned: bool,
    pub source: String,
    #[serde(default = "default_memory_confidence")]
    pub confidence: f64,
    #[serde(default = "default_memory_evidence_count")]
    pub evidence_count: i64,
    #[serde(default)]
    pub last_validated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub validation_command: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCandidate {
    pub id: String,
    pub session_id: String,
    pub category: String,
    pub content: String,
    pub source: String,
    pub decision: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CfpmToolGateEventRecord {
    pub action: String,
    pub tool: String,
    pub target: String,
    pub path: String,
    pub original_command: String,
    pub rewritten_command: String,
    pub verbosity: String,
    pub created_timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFactDraft {
    pub category: String,
    pub content: String,
    pub source: String,
    pub pinned: bool,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub evidence_count: Option<i64>,
    #[serde(default)]
    pub last_validated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub validation_command: Option<String>,
}

impl MemoryFactDraft {
    pub fn new(
        category: impl Into<String>,
        content: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            category: category.into(),
            content: content.into(),
            source: source.into(),
            pinned: false,
            confidence: None,
            evidence_count: None,
            last_validated_at: None,
            validation_command: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFactPatch {
    pub category: Option<String>,
    pub content: Option<String>,
    pub status: Option<MemoryFactStatus>,
    pub pinned: Option<bool>,
}

type MemoryFactRow = (
    String,
    String,
    String,
    String,
    String,
    bool,
    String,
    f64,
    i64,
    Option<DateTime<Utc>>,
    Option<String>,
    DateTime<Utc>,
    DateTime<Utc>,
);

#[derive(Debug, Clone)]
struct MemoryCandidateRecord {
    category: String,
    content: String,
    source: String,
    decision: String,
    reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshotRecord {
    pub id: i64,
    pub session_id: String,
    pub reason: String,
    pub fact_count: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CfpmRuntimeReport {
    pub reason: String,
    pub mode: String,
    pub accepted_count: u32,
    pub rejected_count: u32,
    pub rejected_reason_breakdown: Vec<String>,
    pub pruned_count: u32,
    pub fact_count: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfpmToolGatePayload {
    #[serde(default)]
    action: String,
    #[serde(default)]
    tool: String,
    #[serde(default)]
    target: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    original_command: String,
    #[serde(default)]
    rewritten_command: String,
    #[serde(default)]
    verbosity: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionType {
    User,
    Scheduled,
    SubAgent,
    Hidden,
    Terminal,
}

impl Default for SessionType {
    fn default() -> Self {
        Self::User
    }
}

impl std::fmt::Display for SessionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionType::User => write!(f, "user"),
            SessionType::SubAgent => write!(f, "sub_agent"),
            SessionType::Hidden => write!(f, "hidden"),
            SessionType::Scheduled => write!(f, "scheduled"),
            SessionType::Terminal => write!(f, "terminal"),
        }
    }
}

impl std::str::FromStr for SessionType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(SessionType::User),
            "sub_agent" => Ok(SessionType::SubAgent),
            "hidden" => Ok(SessionType::Hidden),
            "scheduled" => Ok(SessionType::Scheduled),
            "terminal" => Ok(SessionType::Terminal),
            _ => Err(anyhow::anyhow!("Invalid session type: {}", s)),
        }
    }
}

static SESSION_STORAGE: OnceCell<Arc<SessionStorage>> = OnceCell::const_new();

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Session {
    pub id: String,
    #[schema(value_type = String)]
    pub working_dir: PathBuf,
    #[serde(alias = "description")]
    pub name: String,
    #[serde(default)]
    pub user_set_name: bool,
    #[serde(default)]
    pub session_type: SessionType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub extension_data: ExtensionData,
    pub total_tokens: Option<i32>,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub accumulated_total_tokens: Option<i32>,
    pub accumulated_input_tokens: Option<i32>,
    pub accumulated_output_tokens: Option<i32>,
    pub schedule_id: Option<String>,
    pub recipe: Option<Recipe>,
    pub user_recipe_values: Option<HashMap<String, String>>,
    pub conversation: Option<Conversation>,
    pub message_count: usize,
    pub provider_name: Option<String>,
    pub model_config: Option<ModelConfig>,
}

/// Shared session data for session sharing feature
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedSession {
    pub share_token: String,
    pub name: String,
    pub working_dir: String,
    pub messages: String,
    pub message_count: i32,
    pub total_tokens: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub password_hash: Option<String>,
}

pub struct SessionUpdateBuilder {
    session_id: String,
    name: Option<String>,
    user_set_name: Option<bool>,
    session_type: Option<SessionType>,
    working_dir: Option<PathBuf>,
    extension_data: Option<ExtensionData>,
    total_tokens: Option<Option<i32>>,
    input_tokens: Option<Option<i32>>,
    output_tokens: Option<Option<i32>>,
    accumulated_total_tokens: Option<Option<i32>>,
    accumulated_input_tokens: Option<Option<i32>>,
    accumulated_output_tokens: Option<Option<i32>>,
    schedule_id: Option<Option<String>>,
    recipe: Option<Option<Recipe>>,
    user_recipe_values: Option<Option<HashMap<String, String>>>,
    provider_name: Option<Option<String>>,
    model_config: Option<Option<ModelConfig>>,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionInsights {
    pub total_sessions: usize,
    pub total_tokens: i64,
}

impl SessionUpdateBuilder {
    fn new(session_id: String) -> Self {
        Self {
            session_id,
            name: None,
            user_set_name: None,
            session_type: None,
            working_dir: None,
            extension_data: None,
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            accumulated_total_tokens: None,
            accumulated_input_tokens: None,
            accumulated_output_tokens: None,
            schedule_id: None,
            recipe: None,
            user_recipe_values: None,
            provider_name: None,
            model_config: None,
        }
    }

    pub fn user_provided_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into().trim().to_string();
        if !name.is_empty() {
            self.name = Some(name);
            self.user_set_name = Some(true);
        }
        self
    }

    pub fn system_generated_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into().trim().to_string();
        if !name.is_empty() {
            self.name = Some(name);
            self.user_set_name = Some(false);
        }
        self
    }

    pub fn session_type(mut self, session_type: SessionType) -> Self {
        self.session_type = Some(session_type);
        self
    }

    pub fn working_dir(mut self, working_dir: PathBuf) -> Self {
        self.working_dir = Some(working_dir);
        self
    }

    pub fn extension_data(mut self, data: ExtensionData) -> Self {
        self.extension_data = Some(data);
        self
    }

    pub fn total_tokens(mut self, tokens: Option<i32>) -> Self {
        self.total_tokens = Some(tokens);
        self
    }

    pub fn input_tokens(mut self, tokens: Option<i32>) -> Self {
        self.input_tokens = Some(tokens);
        self
    }

    pub fn output_tokens(mut self, tokens: Option<i32>) -> Self {
        self.output_tokens = Some(tokens);
        self
    }

    pub fn accumulated_total_tokens(mut self, tokens: Option<i32>) -> Self {
        self.accumulated_total_tokens = Some(tokens);
        self
    }

    pub fn accumulated_input_tokens(mut self, tokens: Option<i32>) -> Self {
        self.accumulated_input_tokens = Some(tokens);
        self
    }

    pub fn accumulated_output_tokens(mut self, tokens: Option<i32>) -> Self {
        self.accumulated_output_tokens = Some(tokens);
        self
    }

    pub fn schedule_id(mut self, schedule_id: Option<String>) -> Self {
        self.schedule_id = Some(schedule_id);
        self
    }

    pub fn recipe(mut self, recipe: Option<Recipe>) -> Self {
        self.recipe = Some(recipe);
        self
    }

    pub fn user_recipe_values(
        mut self,
        user_recipe_values: Option<HashMap<String, String>>,
    ) -> Self {
        self.user_recipe_values = Some(user_recipe_values);
        self
    }

    pub fn provider_name(mut self, provider_name: impl Into<String>) -> Self {
        self.provider_name = Some(Some(provider_name.into()));
        self
    }

    pub fn model_config(mut self, model_config: ModelConfig) -> Self {
        self.model_config = Some(Some(model_config));
        self
    }

    pub async fn apply(self) -> Result<()> {
        SessionManager::apply_update(self).await
    }
}

pub struct SessionManager;

impl SessionManager {
    pub async fn instance() -> Result<Arc<SessionStorage>> {
        SESSION_STORAGE
            .get_or_try_init(|| async { SessionStorage::new().await.map(Arc::new) })
            .await
            .map(Arc::clone)
    }

    pub async fn create_session(
        working_dir: PathBuf,
        name: String,
        session_type: SessionType,
    ) -> Result<Session> {
        Self::instance()
            .await?
            .create_session(working_dir, name, session_type)
            .await
    }

    pub async fn get_session(id: &str, include_messages: bool) -> Result<Session> {
        Self::instance()
            .await?
            .get_session(id, include_messages)
            .await
    }

    pub fn update_session(id: &str) -> SessionUpdateBuilder {
        SessionUpdateBuilder::new(id.to_string())
    }

    async fn apply_update(builder: SessionUpdateBuilder) -> Result<()> {
        Self::instance().await?.apply_update(builder).await
    }

    pub async fn add_message(id: &str, message: &Message) -> Result<()> {
        Self::instance().await?.add_message(id, message).await
    }

    pub async fn replace_conversation(id: &str, conversation: &Conversation) -> Result<()> {
        Self::instance()
            .await?
            .replace_conversation(id, conversation)
            .await
    }

    pub async fn list_sessions() -> Result<Vec<Session>> {
        Self::instance().await?.list_sessions().await
    }

    /// List sessions with pagination support
    /// Returns (sessions, total_count)
    pub async fn list_sessions_paginated(
        limit: i64,
        before: Option<DateTime<Utc>>,
        favorites_only: bool,
        tags: Option<Vec<String>>,
        working_dir: Option<String>,
        date_from: Option<DateTime<Utc>>,
        date_to: Option<DateTime<Utc>>,
        dates: Option<Vec<String>>,
        timezone_offset: Option<i32>,
        sort_by: String,
        sort_order: String,
    ) -> Result<(Vec<Session>, i64)> {
        Self::instance()
            .await?
            .list_sessions_paginated_impl(
                limit,
                before,
                favorites_only,
                tags,
                working_dir,
                date_from,
                date_to,
                dates,
                timezone_offset,
                sort_by,
                sort_order,
            )
            .await
    }

    pub async fn list_sessions_by_types(types: &[SessionType]) -> Result<Vec<Session>> {
        Self::instance().await?.list_sessions_by_types(types).await
    }

    pub async fn delete_session(id: &str) -> Result<()> {
        Self::instance().await?.delete_session(id).await
    }

    pub async fn get_insights() -> Result<SessionInsights> {
        Self::instance().await?.get_insights().await
    }

    pub async fn export_session(id: &str) -> Result<String> {
        Self::instance().await?.export_session(id).await
    }

    pub async fn import_session(json: &str) -> Result<Session> {
        Self::instance().await?.import_session(json).await
    }

    pub async fn copy_session(session_id: &str, new_name: String) -> Result<Session> {
        Self::instance()
            .await?
            .copy_session(session_id, new_name)
            .await
    }

    pub async fn truncate_conversation(session_id: &str, timestamp: i64) -> Result<()> {
        Self::instance()
            .await?
            .truncate_conversation(session_id, timestamp)
            .await
    }

    pub async fn list_memory_facts(session_id: &str) -> Result<Vec<MemoryFact>> {
        Self::instance().await?.list_memory_facts(session_id).await
    }

    pub async fn list_memory_candidates(
        session_id: &str,
        decision: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<MemoryCandidate>> {
        Self::instance()
            .await?
            .list_memory_candidates(session_id, decision, limit)
            .await
    }

    pub async fn list_recent_cfpm_tool_gate_events(
        session_id: &str,
        limit: Option<u32>,
    ) -> Result<Vec<CfpmToolGateEventRecord>> {
        Self::instance()
            .await?
            .list_recent_cfpm_tool_gate_events(session_id, limit)
            .await
    }

    pub async fn create_memory_fact(
        session_id: &str,
        draft: MemoryFactDraft,
    ) -> Result<MemoryFact> {
        Self::instance()
            .await?
            .create_memory_fact(session_id, draft)
            .await
    }

    pub async fn update_memory_fact(
        session_id: &str,
        fact_id: &str,
        patch: MemoryFactPatch,
    ) -> Result<MemoryFact> {
        Self::instance()
            .await?
            .update_memory_fact(session_id, fact_id, patch)
            .await
    }

    pub async fn rename_memory_paths(
        session_id: &str,
        from_path: &str,
        to_path: &str,
    ) -> Result<u64> {
        Self::instance()
            .await?
            .rename_memory_paths(session_id, from_path, to_path)
            .await
    }

    pub async fn list_memory_snapshots(session_id: &str) -> Result<Vec<MemorySnapshotRecord>> {
        Self::instance()
            .await?
            .list_memory_snapshots(session_id)
            .await
    }

    pub async fn rollback_memory_snapshot(session_id: &str, snapshot_id: i64) -> Result<u64> {
        Self::instance()
            .await?
            .rollback_memory_snapshot(session_id, snapshot_id)
            .await
    }

    pub async fn replace_cfpm_memory_facts(
        session_id: &str,
        drafts: Vec<MemoryFactDraft>,
        reason: &str,
    ) -> Result<()> {
        Self::instance()
            .await?
            .replace_cfpm_memory_facts(session_id, drafts, reason)
            .await
    }

    pub async fn merge_cfpm_memory_facts(
        session_id: &str,
        drafts: Vec<MemoryFactDraft>,
        reason: &str,
    ) -> Result<()> {
        Self::instance()
            .await?
            .merge_cfpm_memory_facts(session_id, drafts, reason)
            .await
            .map(|_| ())
    }

    pub async fn refresh_cfpm_memory_facts_from_recent_messages_with_report(
        session_id: &str,
        messages: &[Message],
        reason: &str,
    ) -> Result<CfpmRuntimeReport> {
        let storage = Self::instance().await?;
        let drafts = extract_runtime_cfpm_memory_drafts(messages);
        if drafts.is_empty() {
            let removed = storage
                .prune_cfpm_auto_memory_facts(session_id, reason)
                .await?;
            let fact_count = storage.count_active_cfpm_auto_facts(session_id).await? as u32;
            return Ok(CfpmRuntimeReport {
                reason: reason.to_string(),
                mode: if removed > 0 {
                    "prune".to_string()
                } else {
                    "noop".to_string()
                },
                accepted_count: 0,
                rejected_count: 0,
                rejected_reason_breakdown: Vec::new(),
                pruned_count: removed as u32,
                fact_count,
            });
        }

        let mut report = storage
            .merge_cfpm_memory_facts(session_id, drafts, reason)
            .await?;
        let removed = storage
            .prune_cfpm_auto_memory_facts(session_id, reason)
            .await?;
        if removed > 0 {
            report.pruned_count = report.pruned_count.saturating_add(removed as u32);
            report.mode = if report.mode == "merge" {
                "merge+prune".to_string()
            } else {
                format!("{}+prune", report.mode)
            };
            report.fact_count = storage.count_active_cfpm_auto_facts(session_id).await? as u32;
        }
        Ok(report)
    }

    pub async fn prune_cfpm_auto_memory_facts(session_id: &str, reason: &str) -> Result<u64> {
        Self::instance()
            .await?
            .prune_cfpm_auto_memory_facts(session_id, reason)
            .await
    }

    pub async fn refresh_cfpm_memory_facts_from_recent_messages(
        session_id: &str,
        messages: &[Message],
        reason: &str,
    ) -> Result<()> {
        let report = Self::refresh_cfpm_memory_facts_from_recent_messages_with_report(
            session_id, messages, reason,
        )
        .await?;
        if report.pruned_count > 0 {
            info!(
                "Pruned {} CFPM auto facts for session {} (reason: {})",
                report.pruned_count, session_id, reason
            );
        }
        Ok(())
    }

    pub async fn replace_cfpm_memory_facts_from_conversation(
        session_id: &str,
        conversation: &Conversation,
        reason: &str,
    ) -> Result<()> {
        let maybe_memory_message = conversation.messages().iter().rev().find(|msg| {
            msg.is_agent_visible()
                && !msg.is_user_visible()
                && msg.as_concat_text().contains("[CFPM_MEMORY_V1]")
        });

        let Some(memory_message) = maybe_memory_message else {
            return Ok(());
        };

        let drafts = parse_cfpm_memory_fact_drafts(&memory_message.as_concat_text());
        if drafts.is_empty() {
            return Ok(());
        }

        Self::replace_cfpm_memory_facts(session_id, drafts, reason).await
    }

    pub async fn maybe_update_name(id: &str, provider: Arc<dyn Provider>) -> Result<()> {
        let session = Self::get_session(id, true).await?;

        if session.user_set_name {
            return Ok(());
        }

        let conversation = session
            .conversation
            .ok_or_else(|| anyhow::anyhow!("No messages found"))?;

        let user_message_count = conversation
            .messages()
            .iter()
            .filter(|m| matches!(m.role, Role::User))
            .count();

        if user_message_count <= MSG_COUNT_FOR_SESSION_NAME_GENERATION {
            let name = provider.generate_session_name(&conversation).await?;
            Self::update_session(id)
                .system_generated_name(name)
                .apply()
                .await
        } else {
            Ok(())
        }
    }

    pub async fn search_chat_history(
        query: &str,
        limit: Option<usize>,
        after_date: Option<chrono::DateTime<chrono::Utc>>,
        before_date: Option<chrono::DateTime<chrono::Utc>>,
        exclude_session_id: Option<String>,
    ) -> Result<crate::session::chat_history_search::ChatRecallResults> {
        Self::instance()
            .await?
            .search_chat_history(query, limit, after_date, before_date, exclude_session_id)
            .await
    }
}

pub struct SessionStorage {
    pool: Pool<Sqlite>,
}

pub fn ensure_session_dir() -> Result<PathBuf> {
    let session_dir = Paths::data_dir().join(SESSIONS_FOLDER);

    if !session_dir.exists() {
        fs::create_dir_all(&session_dir)?;
    }

    Ok(session_dir)
}

fn role_to_string(role: &Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

fn normalize_memory_category(category: &str) -> String {
    let trimmed = category.trim();
    if trimmed.is_empty() {
        return "note".to_string();
    }
    trimmed.to_ascii_lowercase().replace(' ', "_")
}

fn normalize_memory_content(content: &str) -> String {
    content.trim().to_string()
}

fn normalize_memory_source(source: &str) -> String {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        MEMORY_SOURCE_USER.to_string()
    } else {
        trimmed.to_ascii_lowercase()
    }
}

fn default_memory_confidence() -> f64 {
    DEFAULT_MEMORY_CONFIDENCE_CFPM
}

fn default_memory_evidence_count() -> i64 {
    1
}

fn normalize_memory_confidence(confidence: f64) -> f64 {
    if !confidence.is_finite() {
        return default_memory_confidence();
    }
    confidence.clamp(MIN_MEMORY_CONFIDENCE, MAX_MEMORY_CONFIDENCE)
}

fn normalize_memory_evidence_count(evidence_count: i64) -> i64 {
    evidence_count.max(1)
}

fn normalize_validation_command(command: Option<String>) -> Option<String> {
    command
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn default_confidence_for_fact(source: &str, category: &str) -> f64 {
    if category == "invalid_path" {
        return DEFAULT_MEMORY_CONFIDENCE_INVALID_PATH;
    }
    if source == MEMORY_SOURCE_USER {
        return DEFAULT_MEMORY_CONFIDENCE_USER;
    }
    DEFAULT_MEMORY_CONFIDENCE_CFPM
}

fn is_invalid_path_category(category: &str) -> bool {
    category == "invalid_path" || category == "artifact_invalid_path"
}

fn resolve_fact_metadata(
    source: &str,
    category: &str,
    confidence: Option<f64>,
    evidence_count: Option<i64>,
    last_validated_at: Option<DateTime<Utc>>,
    validation_command: Option<String>,
) -> (f64, i64, Option<DateTime<Utc>>, Option<String>) {
    let confidence = normalize_memory_confidence(
        confidence.unwrap_or_else(|| default_confidence_for_fact(source, category)),
    );
    let evidence_count = normalize_memory_evidence_count(evidence_count.unwrap_or(1));
    let last_validated_at = if is_invalid_path_category(category) || is_artifact_category(category)
    {
        Some(last_validated_at.unwrap_or_else(Utc::now))
    } else {
        last_validated_at
    };
    let validation_command = normalize_validation_command(validation_command);
    (
        confidence,
        evidence_count,
        last_validated_at,
        validation_command,
    )
}

fn merge_validation_timestamp(
    current: Option<DateTime<Utc>>,
    incoming: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    match (current, incoming) {
        (Some(current), Some(incoming)) => Some(current.max(incoming)),
        (None, Some(incoming)) => Some(incoming),
        (Some(current), None) => Some(current),
        (None, None) => None,
    }
}

fn is_artifact_category(category: &str) -> bool {
    category.starts_with("artifact")
}

fn looks_like_date_token(value: &str) -> bool {
    let token = value
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | ',' | ';' | '.' | ')' | '(' | '[' | ']' | '{' | '}' | '<' | '>'
            )
        })
        .trim();
    if token.is_empty() {
        return false;
    }

    for separator in ['/', '-', '.'] {
        if !token.contains(separator) {
            continue;
        }

        let parts = token.split(separator).collect::<Vec<_>>();
        if parts.len() != 3 {
            continue;
        }
        if parts
            .iter()
            .any(|part| part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()))
        {
            continue;
        }

        let Ok(first) = parts[0].parse::<u32>() else {
            continue;
        };
        let Ok(second) = parts[1].parse::<u32>() else {
            continue;
        };
        let Ok(third) = parts[2].parse::<u32>() else {
            continue;
        };

        if parts[0].len() == 4
            && (1900..=2200).contains(&first)
            && (1..=12).contains(&second)
            && (1..=31).contains(&third)
        {
            return true;
        }

        if parts[2].len() == 4
            && (1900..=2200).contains(&third)
            && (1..=12).contains(&second)
            && (1..=31).contains(&first)
        {
            return true;
        }
    }

    false
}

fn is_unhelpful_artifact(content: &str) -> bool {
    let trimmed = normalize_memory_content(content);
    let lowered = trimmed.to_ascii_lowercase();
    trimmed.is_empty()
        || looks_like_date_token(&trimmed)
        || trimmed.len() > 320
        || trimmed.contains('\n')
        || trimmed.contains('\r')
        || lowered.contains("\\appdata\\local\\temp\\")
        || lowered.contains("/appdata/local/temp/")
        || lowered.contains("\\temp\\.")
        || lowered.ends_with(".tmp")
        || looks_like_transient_tool_dump(&trimmed)
}

fn looks_like_transient_tool_dump(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    let markers = [
        "private note: output was",
        "truncated output",
        "do not show tmp file to user",
        "categoryinfo",
        "fullyqualifiederrorid",
        "itemnotfoundexception",
        "pathnotfound",
        "commandnotfoundexception",
        "available windows:",
        "lastwritetime",
    ];
    line.contains('\u{1b}') || markers.iter().any(|marker| lowered.contains(marker))
}

fn parse_cfpm_memory_fact_drafts(text: &str) -> Vec<MemoryFactDraft> {
    let mut current_category: Option<&str> = None;
    let mut drafts = Vec::new();
    let mut dedupe = HashSet::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        current_category = match line {
            "User goals:" => Some("goal"),
            "Verified actions:" => Some("verified_action"),
            "Important artifacts/paths:" => Some("artifact"),
            "Known artifacts/paths (prefer direct use):" => Some("artifact"),
            "Known invalid paths (avoid reuse unless user asks to re-verify):" => {
                Some("invalid_path")
            }
            "Open items:" => Some("open_item"),
            _ => current_category,
        };

        if !line.starts_with("- ") {
            continue;
        }

        let Some(category) = current_category else {
            continue;
        };

        let content = normalize_memory_content(line.trim_start_matches("- "));
        if content.is_empty() {
            continue;
        }
        if evaluate_cfpm_auto_candidate(category, &content).is_err() {
            continue;
        }

        let dedupe_key = format!("{}::{}", category, content.to_ascii_lowercase());
        if !dedupe.insert(dedupe_key) {
            continue;
        }

        drafts.push(MemoryFactDraft {
            category: category.to_string(),
            content,
            source: MEMORY_SOURCE_CFPM_AUTO.to_string(),
            pinned: false,
            confidence: None,
            evidence_count: None,
            last_validated_at: None,
            validation_command: None,
        });
    }

    drafts
}

fn looks_like_noise_line(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    let noise_markers = [
        "error",
        "failed",
        "failure",
        "exception",
        "traceback",
        "cannot find",
        "cannot access",
        "access denied",
        "permission denied",
        "is denied",
        "not found",
        "enoent",
        "exit code: 1",
        "exit code: 2",
        "does not exist",
        "path not found",
        "could not find",
        "no such file",
        "no such file or directory",
        "pathnotfound",
        "itemnotfoundexception",
        "fullyqualifiederrorid",
        "categoryinfo",
        "the system cannot find the path specified",
        "commandnotfoundexception",
        "系统找不到指定的路径",
        "找不到指定的路径",
        "无法访问",
        "访问不了",
        "拒绝访问",
        "找不到路径",
        "权限不足",
        "失败",
        "报错",
        "错误",
        "未找到",
        "不存在",
    ];
    looks_like_transient_tool_dump(line)
        || noise_markers.iter().any(|marker| lowered.contains(marker))
}

fn looks_like_runtime_log_noise(line: &str) -> bool {
    if looks_like_transient_tool_dump(line) {
        return true;
    }

    let lowered = line.to_ascii_lowercase();
    let markers = [
        "[stdout]",
        "[stderr]",
        "running ",
        "tool details",
        "systemnotification",
        "traceback",
        "stack trace",
        "command output",
        "directory:",
        "mode   ",
        "日志",
        "工具详情",
    ];
    markers.iter().any(|marker| lowered.contains(marker))
}

fn looks_like_path_failure_line(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    let failure_markers = [
        "cannot find path",
        "path not found",
        "the system cannot find the path specified",
        "does not exist",
        "not found",
        "itemnotfoundexception",
        "pathnotfound",
        "cannot access",
        "access denied",
        "permission denied",
        "enoent",
        "no such file",
        "系统找不到指定的路径",
        "找不到指定的路径",
        "找不到路径",
        "未找到",
        "不存在",
        "无法访问",
        "访问不了",
        "权限不足",
        "拒绝访问",
    ];
    failure_markers
        .iter()
        .any(|marker| lowered.contains(marker))
}

fn extract_invalid_paths_from_failure_line(line: &str) -> Vec<String> {
    if !looks_like_path_failure_line(line) {
        return Vec::new();
    }
    if line
        .to_ascii_lowercase()
        .contains("do not show tmp file to user")
    {
        return Vec::new();
    }

    extract_candidate_paths_from_text(line)
        .into_iter()
        .filter(|path| {
            let lowered = path.to_ascii_lowercase();
            !lowered.contains("\\appdata\\local\\temp\\.")
                && !lowered.ends_with(".tmp")
                && !is_symbolic_path_reference(path)
        })
        .collect()
}

fn extract_invalid_paths_from_command_hint(command_hint: Option<&str>) -> Vec<String> {
    let Some(command_hint) = command_hint else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for candidate in extract_candidate_paths_from_text(command_hint) {
        if !looks_like_path_candidate(&candidate)
            || is_unhelpful_artifact(&candidate)
            || is_symbolic_path_reference(&candidate)
            || !is_concrete_absolute_path(&candidate)
        {
            continue;
        }
        let dedupe_key = candidate.to_ascii_lowercase();
        if seen.insert(dedupe_key) {
            paths.push(candidate);
        }
    }
    paths
}

fn truncate_for_memory_metadata(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

fn extract_tool_request_command_hint(
    request: &crate::conversation::message::ToolRequest,
) -> Option<String> {
    let Ok(tool_call) = &request.tool_call else {
        return None;
    };
    let args = tool_call.arguments.as_ref()?;

    for key in ["command", "cmd", "script"] {
        let Some(raw) = args.get(key).and_then(|value| value.as_str()) else {
            continue;
        };
        let normalized = normalize_memory_content(raw);
        if normalized.is_empty() {
            continue;
        }
        return Some(truncate_for_memory_metadata(&normalized, 320));
    }

    None
}

fn looks_like_goal_line(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    let keywords = [
        "need", "must", "should", "want", "goal", "需要", "必须", "目标", "要求",
    ];
    keywords.iter().any(|keyword| lowered.contains(keyword))
}

fn looks_like_structured_catalog_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lowered = trimmed.to_ascii_lowercase();
    trimmed.starts_with('#')
        || trimmed.starts_with('|')
        || trimmed.ends_with('|')
        || trimmed.starts_with("```")
        || trimmed.starts_with("- `")
        || trimmed.starts_with("* `")
        || trimmed.contains("| `")
        || trimmed.contains("` |")
        || (lowered.contains("skills") && trimmed.contains('`'))
}

fn looks_like_open_item_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || looks_like_structured_catalog_line(trimmed) {
        return false;
    }

    let lowered = trimmed.to_ascii_lowercase();
    let prefix_markers = [
        "todo:",
        "todo ",
        "- todo",
        "* todo",
        "[ ]",
        "- [ ]",
        "pending:",
        "next step:",
        "next:",
        "later:",
        "待办",
        "下一步",
        "后续",
        "继续:",
        "继续：",
    ];
    let has_prefix = prefix_markers
        .iter()
        .any(|marker| lowered.starts_with(marker) || trimmed.starts_with(marker));
    if !has_prefix {
        return false;
    }

    let weak_labels = [
        "task management",
        "skills",
        "能力列表",
        "功能列表",
        "任务管理",
    ];
    if weak_labels
        .iter()
        .any(|label| lowered.contains(label) || trimmed.contains(label))
        && trimmed.chars().count() <= 24
    {
        return false;
    }

    true
}

fn looks_like_verified_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || looks_like_structured_catalog_line(trimmed)
        || looks_like_path_failure_line(trimmed)
    {
        return false;
    }

    let lowered = trimmed.to_ascii_lowercase();
    let keywords = [
        "done",
        "completed",
        "completed successfully",
        "successfully",
        "saved to",
        "saved at",
        "written to",
        "resolved to",
        "found at",
        "renamed to",
        "moved to",
        "fixed",
        "resolved",
        "verified",
        "validated",
        "exit code: 0",
        "已完成",
        "完成了",
        "已保存",
        "保存到",
        "写入到",
        "成功找到",
        "成功定位",
        "成功执行",
        "找到了",
        "已找到",
        "已修复",
        "已解决",
        "已验证",
        "已确认",
    ];
    keywords.iter().any(|keyword| lowered.contains(keyword))
}

fn line_wraps_only_paths(line: &str, path_candidates: &[String]) -> bool {
    if path_candidates.is_empty() {
        return false;
    }

    let mut normalized = line.to_string();
    let mut unique_candidates = path_candidates
        .iter()
        .map(|candidate| normalize_path_token(candidate))
        .filter(|candidate| !candidate.is_empty())
        .collect::<Vec<_>>();
    unique_candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.len()));
    unique_candidates.dedup_by(|a, b| a.eq_ignore_ascii_case(b));

    for candidate in &unique_candidates {
        normalized = normalized.replace(candidate, " ");
    }

    normalized
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .all(|ch| {
            matches!(
                ch,
                '`' | '"'
                    | '\''
                    | ','
                    | ';'
                    | '.'
                    | ':'
                    | '!'
                    | '?'
                    | '，'
                    | '。'
                    | '：'
                    | '；'
                    | '！'
                    | '？'
                    | '、'
                    | '“'
                    | '”'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '|'
            )
        })
}

fn normalize_path_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\''
                    | '`'
                    | '*'
                    | ','
                    | ';'
                    | '.'
                    | ':'
                    | '!'
                    | '?'
                    | '，'
                    | '。'
                    | '：'
                    | '；'
                    | '！'
                    | '？'
                    | '、'
                    | '“'
                    | '”'
                    | '（'
                    | '）'
                    | ')'
                    | '('
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
            )
        })
        .trim()
        .to_string()
}

fn path_candidate_regexes() -> &'static [Regex; 2] {
    static REGEXES: OnceLock<[Regex; 2]> = OnceLock::new();
    REGEXES.get_or_init(|| {
        [
            Regex::new(
                r#"[A-Za-z]:\\(?:[^\\/:*?"<>|\r\n\s`。，：；！？、]+\\)*[^\\/:*?"<>|\r\n\s`。，：；！？、]*"#,
            )
            .expect("valid windows path regex"),
            Regex::new(
                r"(?:\./|\.\./|/)?(?:[A-Za-z0-9._-]+/)+[A-Za-z0-9._-]+(?:\.[A-Za-z0-9._-]+)?",
            )
            .expect("valid unix path regex"),
        ]
    })
}

fn is_symbolic_path_reference(path: &str) -> bool {
    let lowered = path.trim().to_ascii_lowercase();
    lowered.starts_with("$env:")
        || lowered.starts_with("$home")
        || lowered.starts_with("~/")
        || lowered.starts_with("~\\")
        || lowered.starts_with("%userprofile%")
        || lowered.starts_with("%homepath%")
        || lowered.contains("[environment]::getfolderpath")
        || lowered.contains("%userprofile%")
}

fn is_concrete_absolute_path(path: &str) -> bool {
    let token = normalize_path_token(path);
    if token.is_empty() || is_symbolic_path_reference(&token) {
        return false;
    }
    let bytes = token.as_bytes();
    let has_drive_prefix =
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    let is_unc = token.starts_with("\\\\");
    let is_unix_absolute = token.starts_with('/');
    has_drive_prefix || is_unc || is_unix_absolute
}

fn canonicalize_memory_path_for_compare(path: &str) -> Option<String> {
    let token = normalize_path_token(path);
    if token.is_empty() || !looks_like_path_candidate(&token) || !is_concrete_absolute_path(&token)
    {
        return None;
    }
    let mut canonical = token.to_ascii_lowercase().replace('/', "\\");
    while canonical.ends_with('\\') {
        canonical.pop();
    }
    if canonical.is_empty() {
        return None;
    }
    Some(canonical)
}

fn collect_canonical_paths_for_compare(content: &str) -> HashSet<String> {
    let mut paths = HashSet::new();
    for candidate in extract_candidate_paths_from_text(content) {
        if let Some(canonical) = canonicalize_memory_path_for_compare(&candidate) {
            paths.insert(canonical);
        }
    }
    paths
}

fn collect_invalid_path_canonicals_from_memory_facts(facts: &[MemoryFact]) -> HashSet<String> {
    let mut invalid_paths = HashSet::new();
    for fact in facts {
        if fact.status != MemoryFactStatus::Active && !fact.pinned {
            continue;
        }
        let category = normalize_memory_category(&fact.category);
        if !is_invalid_path_category(&category) {
            continue;
        }
        invalid_paths.extend(collect_canonical_paths_for_compare(&fact.content));
    }
    invalid_paths
}

fn collect_invalid_path_canonicals_from_drafts(drafts: &[MemoryFactDraft]) -> HashSet<String> {
    let mut invalid_paths = HashSet::new();
    for draft in drafts {
        let category = normalize_memory_category(&draft.category);
        if !is_invalid_path_category(&category) {
            continue;
        }
        invalid_paths.extend(collect_canonical_paths_for_compare(&draft.content));
    }
    invalid_paths
}

fn artifact_conflicts_with_invalid_paths(
    category: &str,
    content: &str,
    invalid_paths: &HashSet<String>,
) -> bool {
    if invalid_paths.is_empty() || !is_artifact_category(category) {
        return false;
    }
    collect_canonical_paths_for_compare(content)
        .into_iter()
        .any(|canonical| invalid_paths.contains(&canonical))
}

fn looks_like_path_candidate(content: &str) -> bool {
    let token = normalize_path_token(content);
    if token.len() < 3 {
        return false;
    }
    if token.len() > 320
        || token.contains('\n')
        || token.contains('\r')
        || token.contains('\u{1b}')
        || token.contains('�')
    {
        return false;
    }
    if token.contains('`')
        || token.contains('|')
        || token
            .chars()
            .any(|ch| matches!(ch, '，' | '。' | '：' | '；' | '！' | '？' | '、'))
    {
        return false;
    }
    if looks_like_date_token(&token) || looks_like_transient_tool_dump(&token) {
        return false;
    }

    if is_symbolic_path_reference(&token) {
        return false;
    }

    if token.starts_with("http://") || token.starts_with("https://") {
        return false;
    }
    // Reject slash-command style tokens like /think, /help, /fix.
    if token.starts_with('/')
        && !token.contains('\\')
        && token.matches('/').count() == 1
        && token
            .chars()
            .skip(1)
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return false;
    }
    if token.split_whitespace().count() > 8 {
        return false;
    }

    if token.contains(":\\")
        && !matches!(
            (token.chars().nth(0), token.chars().nth(1), token.chars().nth(2)),
            (Some(drive), Some(':'), Some('\\')) if drive.is_ascii_alphabetic()
        )
    {
        return false;
    }
    if token.contains(":\\") && token.chars().skip(2).any(|ch| ch == ':') {
        return false;
    }

    let has_alpha = token.chars().any(|ch| ch.is_alphabetic());
    if token.contains('/') && !token.contains('\\') && !token.contains(':') && !has_alpha {
        return false;
    }

    let is_windows_path = token.contains(":\\")
        || token.starts_with("\\\\")
        || token.starts_with(".\\")
        || token.starts_with("~\\");
    let is_unix_path = token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with('/')
        || token.starts_with("~/");
    let has_path_separator = token.contains('\\') || token.contains('/');

    (is_windows_path || is_unix_path || has_path_separator)
        && !token.starts_with("--")
        && !token.starts_with('-')
}

fn is_project_relative_path_candidate(path: &str) -> bool {
    let token = normalize_path_token(path);
    if token.is_empty()
        || is_symbolic_path_reference(&token)
        || is_concrete_absolute_path(&token)
        || token.starts_with('/')
    {
        return false;
    }

    let normalized = token.replace('\\', "/");
    let segments = normalized
        .split('/')
        .filter(|segment| !segment.trim().is_empty())
        .collect::<Vec<_>>();
    if segments.len() < 2 {
        return false;
    }

    let has_extension = segments
        .last()
        .map(|name| name.contains('.') && !name.starts_with('.'))
        .unwrap_or(false);
    let has_common_project_root = segments.first().is_some_and(|first| {
        matches!(
            first.to_ascii_lowercase().as_str(),
            "src"
                | "lib"
                | "test"
                | "tests"
                | "docs"
                | "crates"
                | "ui"
                | "scripts"
                | "app"
                | "server"
                | "client"
        )
    });

    has_extension || has_common_project_root
}

fn looks_like_known_folder_path(path: &str) -> bool {
    let normalized = normalize_path_token(path)
        .to_ascii_lowercase()
        .replace('/', "\\");
    let normalized = normalized.trim_end_matches('\\');
    normalized.ends_with("\\desktop")
        || normalized.ends_with("\\documents")
        || normalized.ends_with("\\downloads")
}

fn looks_like_clean_path_memory_content(content: &str) -> bool {
    let normalized = normalize_memory_content(content);
    if normalized.is_empty()
        || looks_like_transient_tool_dump(&normalized)
        || looks_like_runtime_log_noise(&normalized)
    {
        return false;
    }

    let path_candidates = extract_candidate_paths_from_text(&normalized);
    if path_candidates.is_empty() {
        return false;
    }

    is_explicit_path_line(&normalized, &path_candidates)
        || line_wraps_only_paths(&normalized, &path_candidates)
}

fn evaluate_cfpm_auto_candidate(
    category: &str,
    content: &str,
) -> std::result::Result<(), &'static str> {
    if content.is_empty() {
        return Err("empty_content");
    }
    if looks_like_noise_line(content) {
        return Err("noise_error_line");
    }
    if looks_like_runtime_log_noise(content) {
        return Err("runtime_log_noise");
    }
    if content.chars().count() < 2 {
        return Err("too_short");
    }

    if is_invalid_path_category(category) {
        if is_unhelpful_artifact(content) {
            return Err("invalid_path_unhelpful");
        }
        if !looks_like_path_candidate(content) {
            return Err("invalid_path_not_path_like");
        }
        if !looks_like_clean_path_memory_content(content) {
            return Err("invalid_path_not_clean_path");
        }
        let token = normalize_path_token(content);
        if !is_concrete_absolute_path(&token) {
            return Err("invalid_path_not_concrete_absolute");
        }
    } else if is_artifact_category(category) {
        if is_unhelpful_artifact(content) {
            return Err("artifact_unhelpful");
        }
        if !looks_like_path_candidate(content) {
            return Err("artifact_not_path_like");
        }
        if !looks_like_clean_path_memory_content(content) {
            return Err("artifact_not_clean_path");
        }
        let token = normalize_path_token(content);
        if !is_concrete_absolute_path(&token) && !is_project_relative_path_candidate(&token) {
            return Err("artifact_not_absolute_or_project_relative");
        }
        if is_concrete_absolute_path(&token)
            && looks_like_known_folder_path(&token)
            && !Path::new(&token).is_dir()
        {
            return Err("artifact_known_folder_missing");
        }
    } else if category == "open_item" && !looks_like_open_item_line(content) {
        return Err("open_item_unconfirmed");
    } else if category == "verified_action"
        && !looks_like_verified_line(content)
        && !content.to_ascii_lowercase().contains("exit code: 0")
    {
        return Err("verified_action_unconfirmed");
    }

    Ok(())
}

fn extract_candidate_paths_from_text(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut dedupe = HashSet::new();

    let normalized_whole = normalize_path_token(text);
    if !normalized_whole.chars().any(|ch| ch.is_whitespace())
        && looks_like_path_candidate(&normalized_whole)
    {
        let key = normalized_whole.to_ascii_lowercase();
        if dedupe.insert(key) {
            paths.push(normalized_whole);
        }
    }

    for regex in path_candidate_regexes() {
        for captures in regex.find_iter(text) {
            let token = normalize_path_token(captures.as_str());
            if !looks_like_path_candidate(&token) {
                continue;
            }
            let key = token.to_ascii_lowercase();
            if dedupe.insert(key) {
                paths.push(token);
            }
        }
    }

    for raw in text.split_whitespace() {
        let token = normalize_path_token(raw);
        if token.len() < 3 {
            continue;
        }

        if looks_like_path_candidate(&token) {
            let key = token.to_ascii_lowercase();
            if dedupe.insert(key) {
                paths.push(token);
            }
        }
    }

    paths
}

fn is_explicit_path_line(line: &str, path_candidates: &[String]) -> bool {
    if path_candidates.is_empty() {
        return false;
    }

    let normalized_line = line
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | ',' | ';' | ')' | '(' | '[' | ']' | '{' | '}' | '<' | '>'
            )
        })
        .trim();
    if normalized_line.is_empty() {
        return false;
    }

    path_candidates
        .iter()
        .any(|candidate| normalized_line.eq_ignore_ascii_case(candidate))
}

fn push_runtime_memory_draft(
    drafts: &mut Vec<MemoryFactDraft>,
    dedupe: &mut HashSet<String>,
    category: &str,
    content: &str,
    validation_command: Option<&str>,
) {
    let category = normalize_memory_category(category);
    let content = normalize_memory_content(content);
    if content.is_empty()
        || looks_like_noise_line(&content)
        || looks_like_runtime_log_noise(&content)
    {
        return;
    }
    if evaluate_cfpm_auto_candidate(&category, &content).is_err() {
        return;
    }

    let key = format!("{}::{}", category, content.to_ascii_lowercase());
    if !dedupe.insert(key) {
        return;
    }

    drafts.push(MemoryFactDraft {
        category,
        content,
        source: MEMORY_SOURCE_CFPM_AUTO.to_string(),
        pinned: false,
        confidence: None,
        evidence_count: None,
        last_validated_at: None,
        validation_command: normalize_validation_command(validation_command.map(|v| v.to_string())),
    });
}

fn collect_rejected_reason_breakdown(records: &[MemoryCandidateRecord]) -> Vec<String> {
    let mut reason_counts: HashMap<String, u32> = HashMap::new();
    for record in records {
        if record.decision != "rejected" {
            continue;
        }
        *reason_counts.entry(record.reason.clone()).or_insert(0) += 1;
    }

    let mut pairs = reason_counts.into_iter().collect::<Vec<_>>();
    pairs.sort_by(|(reason_a, count_a), (reason_b, count_b)| {
        count_b.cmp(count_a).then_with(|| reason_a.cmp(reason_b))
    });

    pairs
        .into_iter()
        .take(5)
        .map(|(reason, count)| format!("{}={}", reason, count))
        .collect()
}

fn extract_runtime_cfpm_memory_drafts(messages: &[Message]) -> Vec<MemoryFactDraft> {
    let mut drafts = Vec::new();
    let mut dedupe = HashSet::new();
    let mut tool_request_command_hints: HashMap<String, String> = HashMap::new();

    for message in messages {
        for content in &message.content {
            match content {
                MessageContent::ToolRequest(request) => {
                    if let Some(command_hint) = extract_tool_request_command_hint(request) {
                        tool_request_command_hints.insert(request.id.clone(), command_hint);
                    }
                }
                MessageContent::Text(text) => {
                    for line in text.text.lines() {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        for invalid_path in extract_invalid_paths_from_failure_line(trimmed) {
                            push_runtime_memory_draft(
                                &mut drafts,
                                &mut dedupe,
                                "invalid_path",
                                &invalid_path,
                                None,
                            );
                        }
                        if looks_like_noise_line(trimmed) || looks_like_runtime_log_noise(trimmed) {
                            continue;
                        }

                        if matches!(message.role, Role::User) && looks_like_goal_line(trimmed) {
                            push_runtime_memory_draft(
                                &mut drafts,
                                &mut dedupe,
                                "goal",
                                trimmed,
                                None,
                            );
                        }

                        if matches!(message.role, Role::Assistant) {
                            if looks_like_verified_line(trimmed) {
                                push_runtime_memory_draft(
                                    &mut drafts,
                                    &mut dedupe,
                                    "verified_action",
                                    trimmed,
                                    None,
                                );
                            }
                            if looks_like_open_item_line(trimmed) {
                                push_runtime_memory_draft(
                                    &mut drafts,
                                    &mut dedupe,
                                    "open_item",
                                    trimmed,
                                    None,
                                );
                            }
                            let path_candidates = extract_candidate_paths_from_text(trimmed);
                            if is_explicit_path_line(trimmed, &path_candidates)
                                || line_wraps_only_paths(trimmed, &path_candidates)
                            {
                                for path in path_candidates {
                                    push_runtime_memory_draft(
                                        &mut drafts,
                                        &mut dedupe,
                                        "artifact",
                                        &path,
                                        None,
                                    );
                                }
                            }
                        }
                    }
                }
                MessageContent::ToolResponse(res) => {
                    let command_hint = tool_request_command_hints.get(&res.id).cloned();
                    let output = match &res.tool_result {
                        Ok(result) => result
                            .content
                            .iter()
                            .filter_map(|item| item.as_text().map(|text| text.text.clone()))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        Err(error_message) => error_message.to_string(),
                    };
                    let output = output.trim();
                    if output.is_empty() {
                        continue;
                    }

                    let mut stable_lines = Vec::new();
                    let mut recorded_invalid_paths = false;
                    for line in output
                        .lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                    {
                        let mut invalid_paths = extract_invalid_paths_from_failure_line(line);
                        if invalid_paths.is_empty() && looks_like_path_failure_line(line) {
                            invalid_paths =
                                extract_invalid_paths_from_command_hint(command_hint.as_deref());
                        }
                        for invalid_path in invalid_paths {
                            recorded_invalid_paths = true;
                            push_runtime_memory_draft(
                                &mut drafts,
                                &mut dedupe,
                                "invalid_path",
                                &invalid_path,
                                command_hint.as_deref(),
                            );
                        }
                        if looks_like_noise_line(line) || looks_like_runtime_log_noise(line) {
                            continue;
                        }
                        stable_lines.push(line);
                    }
                    if !recorded_invalid_paths && looks_like_path_failure_line(output) {
                        for invalid_path in
                            extract_invalid_paths_from_command_hint(command_hint.as_deref())
                        {
                            push_runtime_memory_draft(
                                &mut drafts,
                                &mut dedupe,
                                "invalid_path",
                                &invalid_path,
                                command_hint.as_deref(),
                            );
                        }
                    }
                    if stable_lines.is_empty() {
                        continue;
                    }

                    if let Some(first_line) = stable_lines.first() {
                        if looks_like_verified_line(first_line)
                            || first_line.to_ascii_lowercase().contains("exit code: 0")
                        {
                            push_runtime_memory_draft(
                                &mut drafts,
                                &mut dedupe,
                                "verified_action",
                                first_line,
                                command_hint.as_deref(),
                            );
                        }
                    }

                    for line in stable_lines {
                        let path_candidates = extract_candidate_paths_from_text(line);
                        if path_candidates.is_empty() {
                            continue;
                        }
                        if !is_explicit_path_line(line, &path_candidates)
                            && !line_wraps_only_paths(line, &path_candidates)
                        {
                            continue;
                        }
                        for path in path_candidates {
                            push_runtime_memory_draft(
                                &mut drafts,
                                &mut dedupe,
                                "artifact",
                                &path,
                                command_hint.as_deref(),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    drafts
}

impl Default for Session {
    fn default() -> Self {
        Self {
            id: String::new(),
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            name: String::new(),
            user_set_name: false,
            session_type: SessionType::default(),
            created_at: Default::default(),
            updated_at: Default::default(),
            extension_data: ExtensionData::default(),
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            accumulated_total_tokens: None,
            accumulated_input_tokens: None,
            accumulated_output_tokens: None,
            schedule_id: None,
            recipe: None,
            user_recipe_values: None,
            conversation: None,
            message_count: 0,
            provider_name: None,
            model_config: None,
        }
    }
}

impl Session {
    pub fn without_messages(mut self) -> Self {
        self.conversation = None;
        self
    }
}

impl sqlx::FromRow<'_, sqlx::sqlite::SqliteRow> for Session {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;

        let recipe_json: Option<String> = row.try_get("recipe_json")?;
        let recipe = recipe_json.and_then(|json| serde_json::from_str(&json).ok());

        let user_recipe_values_json: Option<String> = row.try_get("user_recipe_values_json")?;
        let user_recipe_values =
            user_recipe_values_json.and_then(|json| serde_json::from_str(&json).ok());

        let model_config_json: Option<String> = row.try_get("model_config_json").ok().flatten();
        let model_config = model_config_json.and_then(|json| serde_json::from_str(&json).ok());

        let name: String = {
            let name_val: String = row.try_get("name").unwrap_or_default();
            if !name_val.is_empty() {
                name_val
            } else {
                row.try_get("description").unwrap_or_default()
            }
        };

        let user_set_name = row.try_get("user_set_name").unwrap_or(false);

        let session_type_str: String = row
            .try_get("session_type")
            .unwrap_or_else(|_| "user".to_string());
        let session_type = session_type_str.parse().unwrap_or_default();

        Ok(Session {
            id: row.try_get("id")?,
            working_dir: PathBuf::from(row.try_get::<String, _>("working_dir")?),
            name,
            user_set_name,
            session_type,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            extension_data: serde_json::from_str(&row.try_get::<String, _>("extension_data")?)
                .unwrap_or_default(),
            total_tokens: row.try_get("total_tokens")?,
            input_tokens: row.try_get("input_tokens")?,
            output_tokens: row.try_get("output_tokens")?,
            accumulated_total_tokens: row.try_get("accumulated_total_tokens")?,
            accumulated_input_tokens: row.try_get("accumulated_input_tokens")?,
            accumulated_output_tokens: row.try_get("accumulated_output_tokens")?,
            schedule_id: row.try_get("schedule_id")?,
            recipe,
            user_recipe_values,
            conversation: None,
            message_count: row.try_get("message_count").unwrap_or(0) as usize,
            provider_name: row.try_get("provider_name").ok().flatten(),
            model_config,
        })
    }
}

impl SessionStorage {
    async fn new() -> Result<Self> {
        let session_dir = ensure_session_dir()?;
        let db_path = session_dir.join(DB_NAME);

        let storage = if db_path.exists() {
            Self::open(&db_path).await?
        } else {
            let storage = Self::create(&db_path).await?;

            if let Err(e) = storage.import_legacy(&session_dir).await {
                warn!("Failed to import some legacy sessions: {}", e);
            }

            storage
        };

        Ok(storage)
    }

    async fn get_pool(db_path: &Path, create_if_missing: bool) -> Result<Pool<Sqlite>> {
        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(create_if_missing)
            .busy_timeout(std::time::Duration::from_secs(5))
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

        sqlx::SqlitePool::connect_with(options).await.map_err(|e| {
            anyhow::anyhow!(
                "Failed to open SQLite database at '{}': {}",
                db_path.display(),
                e
            )
        })
    }

    async fn open(db_path: &Path) -> Result<Self> {
        let pool = Self::get_pool(db_path, false).await?;

        let storage = Self { pool };
        storage.run_migrations().await?;
        Ok(storage)
    }

    async fn create(db_path: &Path) -> Result<Self> {
        let pool = Self::get_pool(db_path, true).await?;

        sqlx::query(
            r#"
            CREATE TABLE schema_version (
                version INTEGER PRIMARY KEY,
                applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
        "#,
        )
        .execute(&pool)
        .await?;

        sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
            .bind(CURRENT_SCHEMA_VERSION)
            .execute(&pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL DEFAULT '',
                description TEXT NOT NULL DEFAULT '',
                user_set_name BOOLEAN DEFAULT FALSE,
                session_type TEXT NOT NULL DEFAULT 'user',
                working_dir TEXT NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                extension_data TEXT DEFAULT '{}',
                total_tokens INTEGER,
                input_tokens INTEGER,
                output_tokens INTEGER,
                accumulated_total_tokens INTEGER,
                accumulated_input_tokens INTEGER,
                accumulated_output_tokens INTEGER,
                schedule_id TEXT,
                recipe_json TEXT,
                user_recipe_values_json TEXT,
                provider_name TEXT,
                model_config_json TEXT
            )
        "#,
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id),
                role TEXT NOT NULL,
                content_json TEXT NOT NULL,
                created_timestamp INTEGER NOT NULL,
                timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                tokens INTEGER,
                metadata_json TEXT
            )
        "#,
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX idx_messages_session ON messages(session_id)")
            .execute(&pool)
            .await?;
        sqlx::query("CREATE INDEX idx_messages_timestamp ON messages(timestamp)")
            .execute(&pool)
            .await?;
        sqlx::query("CREATE INDEX idx_sessions_updated ON sessions(updated_at DESC)")
            .execute(&pool)
            .await?;
        sqlx::query("CREATE INDEX idx_sessions_type ON sessions(session_type)")
            .execute(&pool)
            .await?;

        // Create shared_sessions table for session sharing feature
        sqlx::query(
            r#"
            CREATE TABLE shared_sessions (
                share_token TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                working_dir TEXT NOT NULL,
                messages TEXT NOT NULL,
                message_count INTEGER NOT NULL,
                total_tokens INTEGER,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                expires_at TIMESTAMP,
                password_hash TEXT
            )
        "#,
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX idx_shared_sessions_expires ON shared_sessions(expires_at)")
            .execute(&pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE memory_facts (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id),
                category TEXT NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                pinned BOOLEAN NOT NULL DEFAULT FALSE,
                source TEXT NOT NULL DEFAULT 'user',
                confidence REAL NOT NULL DEFAULT 0.7,
                evidence_count INTEGER NOT NULL DEFAULT 1,
                last_validated_at TIMESTAMP,
                validation_command TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX idx_memory_facts_session ON memory_facts(session_id)")
            .execute(&pool)
            .await?;
        sqlx::query("CREATE INDEX idx_memory_facts_status ON memory_facts(status)")
            .execute(&pool)
            .await?;
        sqlx::query("CREATE INDEX idx_memory_facts_source ON memory_facts(source)")
            .execute(&pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE memory_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id),
                reason TEXT NOT NULL DEFAULT '',
                facts_json TEXT NOT NULL,
                fact_count INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE INDEX idx_memory_snapshots_session ON memory_snapshots(session_id, created_at DESC)",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE memory_edit_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id),
                fact_id TEXT,
                action TEXT NOT NULL,
                before_json TEXT,
                after_json TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX idx_memory_edit_log_session ON memory_edit_log(session_id, created_at DESC)")
            .execute(&pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE memory_candidates (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id),
                category TEXT NOT NULL,
                content TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'cfpm_auto',
                decision TEXT NOT NULL DEFAULT 'accepted',
                reason TEXT NOT NULL DEFAULT '',
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX idx_memory_candidates_session ON memory_candidates(session_id, created_at DESC)")
            .execute(&pool)
            .await?;
        sqlx::query("CREATE INDEX idx_memory_candidates_decision ON memory_candidates(decision)")
            .execute(&pool)
            .await?;

        Ok(Self { pool })
    }

    async fn import_legacy(&self, session_dir: &PathBuf) -> Result<()> {
        use crate::session::legacy;

        let sessions = match legacy::list_sessions(session_dir) {
            Ok(sessions) => sessions,
            Err(_) => {
                warn!("No legacy sessions found to import");
                return Ok(());
            }
        };

        if sessions.is_empty() {
            return Ok(());
        }

        let mut imported_count = 0;
        let mut failed_count = 0;

        for (session_name, session_path) in sessions {
            match legacy::load_session(&session_name, &session_path) {
                Ok(session) => match self.import_legacy_session(&session).await {
                    Ok(_) => {
                        imported_count += 1;
                        info!("  ✓ Imported: {}", session_name);
                    }
                    Err(e) => {
                        failed_count += 1;
                        info!("  ✗ Failed to import {}: {}", session_name, e);
                    }
                },
                Err(e) => {
                    failed_count += 1;
                    info!("  ✗ Failed to load {}: {}", session_name, e);
                }
            }
        }

        info!(
            "Import complete: {} successful, {} failed",
            imported_count, failed_count
        );
        Ok(())
    }

    async fn import_legacy_session(&self, session: &Session) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        let recipe_json = match &session.recipe {
            Some(recipe) => Some(serde_json::to_string(recipe)?),
            None => None,
        };

        let user_recipe_values_json = match &session.user_recipe_values {
            Some(user_recipe_values) => Some(serde_json::to_string(user_recipe_values)?),
            None => None,
        };

        let model_config_json = match &session.model_config {
            Some(model_config) => Some(serde_json::to_string(model_config)?),
            None => None,
        };

        sqlx::query(
            r#"
        INSERT INTO sessions (
            id, name, user_set_name, session_type, working_dir, created_at, updated_at, extension_data,
            total_tokens, input_tokens, output_tokens,
            accumulated_total_tokens, accumulated_input_tokens, accumulated_output_tokens,
            schedule_id, recipe_json, user_recipe_values_json,
            provider_name, model_config_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
        )
            .bind(&session.id)
            .bind(&session.name)
            .bind(session.user_set_name)
            .bind(session.session_type.to_string())
            .bind(session.working_dir.to_string_lossy().as_ref())
            .bind(session.created_at)
            .bind(session.updated_at)
            .bind(serde_json::to_string(&session.extension_data)?)
            .bind(session.total_tokens)
            .bind(session.input_tokens)
            .bind(session.output_tokens)
            .bind(session.accumulated_total_tokens)
            .bind(session.accumulated_input_tokens)
            .bind(session.accumulated_output_tokens)
            .bind(&session.schedule_id)
            .bind(recipe_json)
            .bind(user_recipe_values_json)
            .bind(&session.provider_name)
            .bind(model_config_json)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        if let Some(conversation) = &session.conversation {
            self.replace_conversation(&session.id, conversation).await?;
        }
        Ok(())
    }

    async fn run_migrations(&self) -> Result<()> {
        let current_version = self.get_schema_version().await?;

        if current_version < CURRENT_SCHEMA_VERSION {
            info!(
                "Running database migrations from v{} to v{}...",
                current_version, CURRENT_SCHEMA_VERSION
            );

            for version in (current_version + 1)..=CURRENT_SCHEMA_VERSION {
                info!("  Applying migration v{}...", version);
                self.apply_migration(version).await?;
                self.update_schema_version(version).await?;
                info!("  ✓ Migration v{} complete", version);
            }

            info!("All migrations complete");
        }

        Ok(())
    }

    async fn get_schema_version(&self) -> Result<i32> {
        let table_exists = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS (
                SELECT name FROM sqlite_master
                WHERE type='table' AND name='schema_version'
            )
        "#,
        )
        .fetch_one(&self.pool)
        .await?;

        if !table_exists {
            return Ok(0);
        }

        let version = sqlx::query_scalar::<_, i32>("SELECT MAX(version) FROM schema_version")
            .fetch_one(&self.pool)
            .await?;

        Ok(version)
    }

    async fn update_schema_version(&self, version: i32) -> Result<()> {
        sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
            .bind(version)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn apply_migration(&self, version: i32) -> Result<()> {
        match version {
            1 => {
                sqlx::query(
                    r#"
                    CREATE TABLE IF NOT EXISTS schema_version (
                        version INTEGER PRIMARY KEY,
                        applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                    )
                "#,
                )
                .execute(&self.pool)
                .await?;
            }
            2 => {
                sqlx::query(
                    r#"
                    ALTER TABLE sessions ADD COLUMN user_recipe_values_json TEXT
                "#,
                )
                .execute(&self.pool)
                .await?;
            }
            3 => {
                sqlx::query(
                    r#"
                    ALTER TABLE messages ADD COLUMN metadata_json TEXT
                "#,
                )
                .execute(&self.pool)
                .await?;
            }
            4 => {
                sqlx::query(
                    r#"
                    ALTER TABLE sessions ADD COLUMN name TEXT DEFAULT ''
                "#,
                )
                .execute(&self.pool)
                .await?;

                sqlx::query(
                    r#"
                    ALTER TABLE sessions ADD COLUMN user_set_name BOOLEAN DEFAULT FALSE
                "#,
                )
                .execute(&self.pool)
                .await?;
            }
            5 => {
                sqlx::query(
                    r#"
                    ALTER TABLE sessions ADD COLUMN session_type TEXT NOT NULL DEFAULT 'user'
                "#,
                )
                .execute(&self.pool)
                .await?;

                sqlx::query("CREATE INDEX idx_sessions_type ON sessions(session_type)")
                    .execute(&self.pool)
                    .await?;
            }
            6 => {
                sqlx::query(
                    r#"
                    ALTER TABLE sessions ADD COLUMN provider_name TEXT
                "#,
                )
                .execute(&self.pool)
                .await?;

                sqlx::query(
                    r#"
                    ALTER TABLE sessions ADD COLUMN model_config_json TEXT
                "#,
                )
                .execute(&self.pool)
                .await?;
            }
            7 => {
                // Add shared_sessions table for session sharing feature
                sqlx::query(
                    r#"
                    CREATE TABLE IF NOT EXISTS shared_sessions (
                        share_token TEXT PRIMARY KEY,
                        name TEXT NOT NULL,
                        working_dir TEXT NOT NULL,
                        messages TEXT NOT NULL,
                        message_count INTEGER NOT NULL,
                        total_tokens INTEGER,
                        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                        expires_at TIMESTAMP,
                        password_hash TEXT
                    )
                "#,
                )
                .execute(&self.pool)
                .await?;

                sqlx::query("CREATE INDEX IF NOT EXISTS idx_shared_sessions_expires ON shared_sessions(expires_at)")
                    .execute(&self.pool)
                    .await?;
            }
            8 => {
                sqlx::query(
                    r#"
                    CREATE TABLE IF NOT EXISTS memory_facts (
                        id TEXT PRIMARY KEY,
                        session_id TEXT NOT NULL REFERENCES sessions(id),
                        category TEXT NOT NULL,
                        content TEXT NOT NULL,
                        status TEXT NOT NULL DEFAULT 'active',
                        pinned BOOLEAN NOT NULL DEFAULT FALSE,
                        source TEXT NOT NULL DEFAULT 'user',
                        confidence REAL NOT NULL DEFAULT 0.7,
                        evidence_count INTEGER NOT NULL DEFAULT 1,
                        last_validated_at TIMESTAMP,
                        validation_command TEXT,
                        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                        updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                    )
                    "#,
                )
                .execute(&self.pool)
                .await?;

                sqlx::query("CREATE INDEX IF NOT EXISTS idx_memory_facts_session ON memory_facts(session_id)")
                    .execute(&self.pool)
                    .await?;
                sqlx::query(
                    "CREATE INDEX IF NOT EXISTS idx_memory_facts_status ON memory_facts(status)",
                )
                .execute(&self.pool)
                .await?;
                sqlx::query(
                    "CREATE INDEX IF NOT EXISTS idx_memory_facts_source ON memory_facts(source)",
                )
                .execute(&self.pool)
                .await?;

                sqlx::query(
                    r#"
                    CREATE TABLE IF NOT EXISTS memory_snapshots (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        session_id TEXT NOT NULL REFERENCES sessions(id),
                        reason TEXT NOT NULL DEFAULT '',
                        facts_json TEXT NOT NULL,
                        fact_count INTEGER NOT NULL DEFAULT 0,
                        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                    )
                    "#,
                )
                .execute(&self.pool)
                .await?;

                sqlx::query(
                    "CREATE INDEX IF NOT EXISTS idx_memory_snapshots_session ON memory_snapshots(session_id, created_at DESC)",
                )
                .execute(&self.pool)
                .await?;

                sqlx::query(
                    r#"
                    CREATE TABLE IF NOT EXISTS memory_edit_log (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        session_id TEXT NOT NULL REFERENCES sessions(id),
                        fact_id TEXT,
                        action TEXT NOT NULL,
                        before_json TEXT,
                        after_json TEXT,
                        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                    )
                    "#,
                )
                .execute(&self.pool)
                .await?;

                sqlx::query("CREATE INDEX IF NOT EXISTS idx_memory_edit_log_session ON memory_edit_log(session_id, created_at DESC)")
                    .execute(&self.pool)
                    .await?;
            }
            9 => {
                sqlx::query(
                    r#"
                    CREATE TABLE IF NOT EXISTS memory_candidates (
                        id TEXT PRIMARY KEY,
                        session_id TEXT NOT NULL REFERENCES sessions(id),
                        category TEXT NOT NULL,
                        content TEXT NOT NULL,
                        source TEXT NOT NULL DEFAULT 'cfpm_auto',
                        decision TEXT NOT NULL DEFAULT 'accepted',
                        reason TEXT NOT NULL DEFAULT '',
                        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                    )
                    "#,
                )
                .execute(&self.pool)
                .await?;

                sqlx::query(
                    "CREATE INDEX IF NOT EXISTS idx_memory_candidates_session ON memory_candidates(session_id, created_at DESC)",
                )
                .execute(&self.pool)
                .await?;

                sqlx::query(
                    "CREATE INDEX IF NOT EXISTS idx_memory_candidates_decision ON memory_candidates(decision)",
                )
                .execute(&self.pool)
                .await?;
            }
            10 => {
                for statement in [
                    "ALTER TABLE memory_facts ADD COLUMN confidence REAL NOT NULL DEFAULT 0.7",
                    "ALTER TABLE memory_facts ADD COLUMN evidence_count INTEGER NOT NULL DEFAULT 1",
                    "ALTER TABLE memory_facts ADD COLUMN last_validated_at TIMESTAMP",
                    "ALTER TABLE memory_facts ADD COLUMN validation_command TEXT",
                ] {
                    if let Err(err) = sqlx::query(statement).execute(&self.pool).await {
                        let message = err.to_string().to_ascii_lowercase();
                        if !message.contains("duplicate column name") {
                            return Err(err.into());
                        }
                    }
                }

                sqlx::query(
                    r#"
                    UPDATE memory_facts
                    SET evidence_count = CASE
                        WHEN evidence_count IS NULL OR evidence_count < 1 THEN 1
                        ELSE evidence_count
                    END
                    "#,
                )
                .execute(&self.pool)
                .await?;

                sqlx::query(
                    r#"
                    UPDATE memory_facts
                    SET confidence = CASE
                        WHEN source = 'user' THEN 1.0
                        WHEN lower(category) = 'invalid_path' THEN 0.9
                        WHEN confidence IS NULL OR confidence <= 0 THEN 0.7
                        WHEN confidence > 1.0 THEN 1.0
                        ELSE confidence
                    END
                    "#,
                )
                .execute(&self.pool)
                .await?;

                // One-time cleanup for polluted historical CFPM artifact rows.
                sqlx::query(
                    r#"
                    DELETE FROM memory_facts
                    WHERE source = 'cfpm_auto'
                      AND lower(category) LIKE 'artifact%'
                      AND (
                          lower(content) LIKE '%private note: output was%'
                          OR lower(content) LIKE '%do not show tmp file to user%'
                          OR lower(content) LIKE '%truncated output%'
                          OR lower(content) LIKE '%categoryinfo%'
                          OR lower(content) LIKE '%fullyqualifiederrorid%'
                          OR lower(content) LIKE '%itemnotfoundexception%'
                          OR lower(content) LIKE '%pathnotfound%'
                          OR lower(content) LIKE '%available windows:%'
                          OR lower(content) LIKE '%\appdata\local\temp\%'
                          OR lower(content) LIKE '%/appdata/local/temp/%'
                      )
                    "#,
                )
                .execute(&self.pool)
                .await?;
            }
            11 => {
                let fact_rows = sqlx::query_as::<_, (String, String, String, String)>(
                    r#"
                    SELECT id, category, content, source
                    FROM memory_facts
                    WHERE source = ?
                    "#,
                )
                .bind(MEMORY_SOURCE_CFPM_AUTO)
                .fetch_all(&self.pool)
                .await?;

                let mut stale_fact_ids = Vec::new();
                for (id, category, content, _) in fact_rows {
                    let normalized_category = normalize_memory_category(&category);
                    let normalized_content = normalize_memory_content(&content);
                    if normalized_content.is_empty()
                        || evaluate_cfpm_auto_candidate(&normalized_category, &normalized_content)
                            .is_err()
                    {
                        stale_fact_ids.push(id);
                    }
                }

                for chunk in stale_fact_ids.chunks(200) {
                    let placeholders = std::iter::repeat("?")
                        .take(chunk.len())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let delete_sql =
                        format!("DELETE FROM memory_facts WHERE id IN ({})", placeholders);
                    let mut query = sqlx::query(&delete_sql);
                    for fact_id in chunk {
                        query = query.bind(fact_id);
                    }
                    query.execute(&self.pool).await?;
                }

                let candidate_rows = sqlx::query_as::<_, (String, String, String, String)>(
                    r#"
                    SELECT id, category, content, source
                    FROM memory_candidates
                    WHERE source = ?
                      AND decision = 'accepted'
                    "#,
                )
                .bind(MEMORY_SOURCE_CFPM_AUTO)
                .fetch_all(&self.pool)
                .await?;

                let mut stale_candidate_ids = Vec::new();
                for (id, category, content, _) in candidate_rows {
                    let normalized_category = normalize_memory_category(&category);
                    let normalized_content = normalize_memory_content(&content);
                    if normalized_content.is_empty()
                        || evaluate_cfpm_auto_candidate(&normalized_category, &normalized_content)
                            .is_err()
                    {
                        stale_candidate_ids.push(id);
                    }
                }

                for chunk in stale_candidate_ids.chunks(200) {
                    let placeholders = std::iter::repeat("?")
                        .take(chunk.len())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let delete_sql = format!(
                        "DELETE FROM memory_candidates WHERE id IN ({})",
                        placeholders
                    );
                    let mut query = sqlx::query(&delete_sql);
                    for candidate_id in chunk {
                        query = query.bind(candidate_id);
                    }
                    query.execute(&self.pool).await?;
                }
            }
            _ => {
                anyhow::bail!("Unknown migration version: {}", version);
            }
        }

        Ok(())
    }

    async fn create_session(
        &self,
        working_dir: PathBuf,
        name: String,
        session_type: SessionType,
    ) -> Result<Session> {
        let mut tx = self.pool.begin().await?;

        let today = chrono::Utc::now().format("%Y%m%d").to_string();
        let session = sqlx::query_as(
            r#"
                INSERT INTO sessions (id, name, user_set_name, session_type, working_dir, extension_data)
                VALUES (
                    ? || '_' || CAST(COALESCE((
                        SELECT MAX(CAST(SUBSTR(id, 10) AS INTEGER))
                        FROM sessions
                        WHERE id LIKE ? || '_%'
                    ), 0) + 1 AS TEXT),
                    ?,
                    FALSE,
                    ?,
                    ?,
                    '{}'
                )
                RETURNING *
                "#,
        )
            .bind(&today)
            .bind(&today)
            .bind(&name)
            .bind(session_type.to_string())
            .bind(working_dir.to_string_lossy().as_ref())
            .fetch_one(&mut *tx)
            .await?;

        tx.commit().await?;
        crate::posthog::emit_session_started();
        Ok(session)
    }

    async fn get_session(&self, id: &str, include_messages: bool) -> Result<Session> {
        let mut session = sqlx::query_as::<_, Session>(
            r#"
        SELECT id, working_dir, name, description, user_set_name, session_type, created_at, updated_at, extension_data,
               total_tokens, input_tokens, output_tokens,
               accumulated_total_tokens, accumulated_input_tokens, accumulated_output_tokens,
               schedule_id, recipe_json, user_recipe_values_json,
               provider_name, model_config_json
        FROM sessions
        WHERE id = ?
    "#,
        )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        if include_messages {
            let conv = self.get_conversation(&session.id).await?;
            session.message_count = conv.messages().len();
            session.conversation = Some(conv);
        } else {
            let count =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE session_id = ?")
                    .bind(&session.id)
                    .fetch_one(&self.pool)
                    .await? as usize;
            session.message_count = count;
        }

        Ok(session)
    }

    #[allow(clippy::too_many_lines)]
    async fn apply_update(&self, builder: SessionUpdateBuilder) -> Result<()> {
        let mut updates = Vec::new();
        let mut query = String::from("UPDATE sessions SET ");

        macro_rules! add_update {
            ($field:expr, $name:expr) => {
                if $field.is_some() {
                    if !updates.is_empty() {
                        query.push_str(", ");
                    }
                    updates.push($name);
                    query.push_str($name);
                    query.push_str(" = ?");
                }
            };
        }

        add_update!(builder.name, "name");
        add_update!(builder.user_set_name, "user_set_name");
        add_update!(builder.session_type, "session_type");
        add_update!(builder.working_dir, "working_dir");
        add_update!(builder.extension_data, "extension_data");
        add_update!(builder.total_tokens, "total_tokens");
        add_update!(builder.input_tokens, "input_tokens");
        add_update!(builder.output_tokens, "output_tokens");
        add_update!(builder.accumulated_total_tokens, "accumulated_total_tokens");
        add_update!(builder.accumulated_input_tokens, "accumulated_input_tokens");
        add_update!(
            builder.accumulated_output_tokens,
            "accumulated_output_tokens"
        );
        add_update!(builder.schedule_id, "schedule_id");
        add_update!(builder.recipe, "recipe_json");
        add_update!(builder.user_recipe_values, "user_recipe_values_json");
        add_update!(builder.provider_name, "provider_name");
        add_update!(builder.model_config, "model_config_json");

        if updates.is_empty() {
            return Ok(());
        }

        query.push_str(", ");
        query.push_str("updated_at = datetime('now') WHERE id = ?");

        let mut q = sqlx::query(&query);

        if let Some(name) = builder.name {
            q = q.bind(name);
        }
        if let Some(user_set_name) = builder.user_set_name {
            q = q.bind(user_set_name);
        }
        if let Some(session_type) = builder.session_type {
            q = q.bind(session_type.to_string());
        }
        if let Some(wd) = builder.working_dir {
            q = q.bind(wd.to_string_lossy().to_string());
        }
        if let Some(ed) = builder.extension_data {
            q = q.bind(serde_json::to_string(&ed)?);
        }
        if let Some(tt) = builder.total_tokens {
            q = q.bind(tt);
        }
        if let Some(it) = builder.input_tokens {
            q = q.bind(it);
        }
        if let Some(ot) = builder.output_tokens {
            q = q.bind(ot);
        }
        if let Some(att) = builder.accumulated_total_tokens {
            q = q.bind(att);
        }
        if let Some(ait) = builder.accumulated_input_tokens {
            q = q.bind(ait);
        }
        if let Some(aot) = builder.accumulated_output_tokens {
            q = q.bind(aot);
        }
        if let Some(sid) = builder.schedule_id {
            q = q.bind(sid);
        }
        if let Some(recipe) = builder.recipe {
            let recipe_json = recipe.map(|r| serde_json::to_string(&r)).transpose()?;
            q = q.bind(recipe_json);
        }
        if let Some(user_recipe_values) = builder.user_recipe_values {
            let user_recipe_values_json = user_recipe_values
                .map(|urv| serde_json::to_string(&urv))
                .transpose()?;
            q = q.bind(user_recipe_values_json);
        }
        if let Some(provider_name) = builder.provider_name {
            q = q.bind(provider_name);
        }
        if let Some(model_config) = builder.model_config {
            let model_config_json = model_config
                .map(|mc| serde_json::to_string(&mc))
                .transpose()?;
            q = q.bind(model_config_json);
        }

        let mut tx = self.pool.begin().await?;
        q = q.bind(&builder.session_id);
        q.execute(&mut *tx).await?;

        tx.commit().await?;
        Ok(())
    }

    async fn get_conversation(&self, session_id: &str) -> Result<Conversation> {
        let rows = sqlx::query_as::<_, (String, String, i64, Option<String>)>(
            "SELECT role, content_json, created_timestamp, metadata_json FROM messages WHERE session_id = ? ORDER BY timestamp",
        )
            .bind(session_id)
            .fetch_all(&self.pool)
            .await?;

        let mut messages = Vec::new();
        for (idx, (role_str, content_json, created_timestamp, metadata_json)) in
            rows.into_iter().enumerate()
        {
            let role = match role_str.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => continue,
            };

            let content = serde_json::from_str(&content_json)?;
            let metadata = metadata_json
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default();

            let mut message = Message::new(role, created_timestamp, content);
            message.metadata = metadata;
            message = message.with_id(format!("msg_{}_{}", session_id, idx));
            messages.push(message);
        }

        Ok(Conversation::new_unvalidated(messages))
    }

    async fn add_message(&self, session_id: &str, message: &Message) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        let metadata_json = serde_json::to_string(&message.metadata)?;

        sqlx::query(
            r#"
            INSERT INTO messages (session_id, role, content_json, created_timestamp, metadata_json)
            VALUES (?, ?, ?, ?, ?)
        "#,
        )
        .bind(session_id)
        .bind(role_to_string(&message.role))
        .bind(serde_json::to_string(&message.content)?)
        .bind(message.created)
        .bind(metadata_json)
        .execute(&mut *tx)
        .await?;

        sqlx::query("UPDATE sessions SET updated_at = datetime('now') WHERE id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn replace_conversation(
        &self,
        session_id: &str,
        conversation: &Conversation,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM messages WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        for message in conversation.messages() {
            let metadata_json = serde_json::to_string(&message.metadata)?;

            sqlx::query(
                r#"
            INSERT INTO messages (session_id, role, content_json, created_timestamp, metadata_json)
            VALUES (?, ?, ?, ?, ?)
        "#,
            )
            .bind(session_id)
            .bind(role_to_string(&message.role))
            .bind(serde_json::to_string(&message.content)?)
            .bind(message.created)
            .bind(metadata_json)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn list_sessions_by_types(&self, types: &[SessionType]) -> Result<Vec<Session>> {
        if types.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: String = types.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let query = format!(
            r#"
            SELECT s.id, s.working_dir, s.name, s.description, s.user_set_name, s.session_type, s.created_at, s.updated_at, s.extension_data,
                   s.total_tokens, s.input_tokens, s.output_tokens,
                   s.accumulated_total_tokens, s.accumulated_input_tokens, s.accumulated_output_tokens,
                   s.schedule_id, s.recipe_json, s.user_recipe_values_json,
                   s.provider_name, s.model_config_json,
                   COUNT(m.id) as message_count
            FROM sessions s
            INNER JOIN messages m ON s.id = m.session_id
            WHERE s.session_type IN ({})
            GROUP BY s.id
            ORDER BY s.updated_at DESC
            "#,
            placeholders
        );

        let mut q = sqlx::query_as::<_, Session>(&query);
        for t in types {
            q = q.bind(t.to_string());
        }

        q.fetch_all(&self.pool).await.map_err(Into::into)
    }

    async fn list_sessions(&self) -> Result<Vec<Session>> {
        self.list_sessions_by_types(&[SessionType::User, SessionType::Scheduled])
            .await
    }

    /// Internal implementation of paginated session listing
    async fn list_sessions_paginated_impl(
        &self,
        limit: i64,
        before: Option<DateTime<Utc>>,
        favorites_only: bool,
        tags: Option<Vec<String>>,
        working_dir: Option<String>,
        date_from: Option<DateTime<Utc>>,
        date_to: Option<DateTime<Utc>>,
        dates: Option<Vec<String>>,
        timezone_offset: Option<i32>,
        sort_by: String,
        sort_order: String,
    ) -> Result<(Vec<Session>, i64)> {
        let types = [SessionType::User, SessionType::Scheduled];
        let type_placeholders: String = types.iter().map(|_| "?").collect::<Vec<_>>().join(", ");

        // Build base WHERE conditions (without cursor - for count query)
        let mut base_conditions = vec![format!("s.session_type IN ({})", type_placeholders)];
        let bind_values: Vec<String> = types.iter().map(|t| t.to_string()).collect();

        // Favorites filter using json_extract
        // Note: Key contains dot so must use bracket notation or quoted key
        if favorites_only {
            base_conditions
                .push("json_extract(s.extension_data, '$.\"favorites.v0\"') = 1".to_string());
        }

        // Tags filter - check if any of the specified tags exist in the session's tags array
        // Note: Key contains dot so must use quoted key in JSON path
        if let Some(ref tag_list) = tags {
            if !tag_list.is_empty() {
                // Build OR conditions for each tag
                let tag_conditions: Vec<String> = tag_list
                    .iter()
                    .map(|_| "json_extract(s.extension_data, '$.\"tags.v0\"') LIKE ?".to_string())
                    .collect();
                base_conditions.push(format!("({})", tag_conditions.join(" OR ")));
            }
        }

        // Working directory filter
        if working_dir.is_some() {
            base_conditions.push("s.working_dir = ?".to_string());
        }

        // Date filters - discrete dates take precedence over range
        // For discrete dates, we need to convert UTC to local time before comparing
        // timezone_offset is from JS getTimezoneOffset() which returns minutes BEHIND UTC
        // e.g., UTC+8 returns -480, so we negate it to get the adjustment
        if let Some(ref date_list) = dates {
            if !date_list.is_empty() {
                let date_placeholders: String =
                    date_list.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
                // Apply timezone offset to convert UTC to local time before extracting DATE
                if let Some(offset) = timezone_offset {
                    // Negate offset: getTimezoneOffset returns negative for UTC+ zones
                    // So UTC+8 (-480) becomes +480 minutes to add to UTC
                    let offset_minutes = -offset;
                    let sign = if offset_minutes >= 0 { "+" } else { "" };
                    base_conditions.push(format!(
                        "DATE(datetime(s.updated_at, '{}{} minutes')) IN ({})",
                        sign, offset_minutes, date_placeholders
                    ));
                } else {
                    // No timezone offset, use UTC directly
                    base_conditions.push(format!("DATE(s.updated_at) IN ({})", date_placeholders));
                }
            }
        } else {
            // Date range filters (only if discrete dates not provided)
            // Use datetime() function to properly compare dates regardless of format
            if date_from.is_some() {
                base_conditions.push("datetime(s.updated_at) >= datetime(?)".to_string());
            }
            if date_to.is_some() {
                base_conditions.push("datetime(s.updated_at) <= datetime(?)".to_string());
            }
        }

        let base_where_clause = base_conditions.join(" AND ");

        // Build paginated WHERE clause (with cursor)
        let paginated_where_clause = if before.is_some() {
            format!("{} AND s.updated_at < ?", base_where_clause)
        } else {
            base_where_clause.clone()
        };

        // Validate and build ORDER BY clause
        let valid_sort_fields = ["updated_at", "created_at", "message_count", "total_tokens"];
        let sort_field = if valid_sort_fields.contains(&sort_by.as_str()) {
            match sort_by.as_str() {
                "message_count" => "message_count".to_string(),
                "total_tokens" => "s.total_tokens".to_string(),
                field => format!("s.{}", field),
            }
        } else {
            "s.updated_at".to_string()
        };
        let order_direction = if sort_order.to_lowercase() == "asc" {
            "ASC"
        } else {
            "DESC"
        };

        // Query for paginated results
        let query = format!(
            r#"
            SELECT s.id, s.working_dir, s.name, s.description, s.user_set_name, s.session_type, s.created_at, s.updated_at, s.extension_data,
                   s.total_tokens, s.input_tokens, s.output_tokens,
                   s.accumulated_total_tokens, s.accumulated_input_tokens, s.accumulated_output_tokens,
                   s.schedule_id, s.recipe_json, s.user_recipe_values_json,
                   s.provider_name, s.model_config_json,
                   COUNT(m.id) as message_count
            FROM sessions s
            INNER JOIN messages m ON s.id = m.session_id
            WHERE {}
            GROUP BY s.id
            ORDER BY {} {}
            LIMIT ?
            "#,
            paginated_where_clause, sort_field, order_direction
        );

        let mut q = sqlx::query_as::<_, Session>(&query);

        // Bind session types
        for val in &bind_values {
            q = q.bind(val);
        }

        // Bind tag patterns if present
        // Use pattern %"tag"% to match exact tag in JSON array like ["tag1","tag2"]
        if let Some(ref tag_list) = tags {
            for tag in tag_list {
                q = q.bind(format!("%\"{}\"%", tag));
            }
        }

        // Bind working_dir if present
        if let Some(ref dir) = working_dir {
            q = q.bind(dir);
        }

        // Bind dates - discrete dates take precedence over range
        if let Some(ref date_list) = dates {
            for date in date_list {
                q = q.bind(date);
            }
        } else {
            // Bind date_from if present
            if let Some(ref from_time) = date_from {
                q = q.bind(from_time.to_rfc3339());
            }

            // Bind date_to if present
            if let Some(ref to_time) = date_to {
                q = q.bind(to_time.to_rfc3339());
            }
        }

        // Bind cursor if present
        if let Some(ref before_time) = before {
            q = q.bind(before_time.to_rfc3339());
        }

        // Bind limit
        q = q.bind(limit);

        let sessions = q.fetch_all(&self.pool).await?;

        // Query for total count (without cursor - shows total matching sessions)
        let count_query = format!(
            r#"
            SELECT COUNT(DISTINCT s.id)
            FROM sessions s
            INNER JOIN messages m ON s.id = m.session_id
            WHERE {}
            "#,
            base_where_clause
        );

        let mut count_q = sqlx::query_scalar::<_, i64>(&count_query);

        // Bind session types for count query
        for val in &bind_values {
            count_q = count_q.bind(val);
        }

        // Bind tag patterns if present (no cursor for count)
        // Use pattern %"tag"% to match exact tag in JSON array like ["tag1","tag2"]
        if let Some(ref tag_list) = tags {
            for tag in tag_list {
                count_q = count_q.bind(format!("%\"{}\"%", tag));
            }
        }

        // Bind working_dir for count query
        if let Some(ref dir) = working_dir {
            count_q = count_q.bind(dir);
        }

        // Bind dates for count query - discrete dates take precedence over range
        if let Some(ref date_list) = dates {
            for date in date_list {
                count_q = count_q.bind(date);
            }
        } else {
            // Bind date_from for count query
            if let Some(ref from_time) = date_from {
                count_q = count_q.bind(from_time.to_rfc3339());
            }

            // Bind date_to for count query
            if let Some(ref to_time) = date_to {
                count_q = count_q.bind(to_time.to_rfc3339());
            }
        }

        let total_count = count_q.fetch_one(&self.pool).await.unwrap_or(0);

        Ok((sessions, total_count))
    }

    async fn delete_session(&self, session_id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        let exists =
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?)")
                .bind(session_id)
                .fetch_one(&mut *tx)
                .await?;

        if !exists {
            return Err(anyhow::anyhow!("Session not found"));
        }

        sqlx::query("DELETE FROM messages WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM memory_edit_log WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM memory_snapshots WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM memory_facts WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM memory_candidates WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn get_insights(&self) -> Result<SessionInsights> {
        let row = sqlx::query_as::<_, (i64, Option<i64>)>(
            r#"
            SELECT COUNT(*) as total_sessions,
                   COALESCE(SUM(COALESCE(accumulated_total_tokens, total_tokens, 0)), 0) as total_tokens
            FROM sessions
            "#,
        )
            .fetch_one(&self.pool)
            .await?;

        Ok(SessionInsights {
            total_sessions: row.0 as usize,
            total_tokens: row.1.unwrap_or(0),
        })
    }

    async fn export_session(&self, id: &str) -> Result<String> {
        let session = self.get_session(id, true).await?;
        serde_json::to_string_pretty(&session).map_err(Into::into)
    }

    async fn import_session(&self, json: &str) -> Result<Session> {
        let import: Session = serde_json::from_str(json)?;

        let session = self
            .create_session(
                import.working_dir.clone(),
                import.name.clone(),
                import.session_type,
            )
            .await?;

        let mut builder = SessionUpdateBuilder::new(session.id.clone())
            .extension_data(import.extension_data)
            .total_tokens(import.total_tokens)
            .input_tokens(import.input_tokens)
            .output_tokens(import.output_tokens)
            .accumulated_total_tokens(import.accumulated_total_tokens)
            .accumulated_input_tokens(import.accumulated_input_tokens)
            .accumulated_output_tokens(import.accumulated_output_tokens)
            .schedule_id(import.schedule_id)
            .recipe(import.recipe)
            .user_recipe_values(import.user_recipe_values);

        if import.user_set_name {
            builder = builder.user_provided_name(import.name.clone());
        }

        self.apply_update(builder).await?;

        if let Some(conversation) = import.conversation {
            self.replace_conversation(&session.id, &conversation)
                .await?;
        }

        self.get_session(&session.id, true).await
    }

    async fn copy_session(&self, session_id: &str, new_name: String) -> Result<Session> {
        let original_session = self.get_session(session_id, true).await?;

        let new_session = self
            .create_session(
                original_session.working_dir.clone(),
                new_name,
                original_session.session_type,
            )
            .await?;

        let builder = SessionUpdateBuilder::new(new_session.id.clone())
            .extension_data(original_session.extension_data)
            .schedule_id(original_session.schedule_id)
            .recipe(original_session.recipe)
            .user_recipe_values(original_session.user_recipe_values);

        self.apply_update(builder).await?;

        if let Some(conversation) = original_session.conversation {
            self.replace_conversation(&new_session.id, &conversation)
                .await?;
        }

        self.get_session(&new_session.id, true).await
    }

    async fn truncate_conversation(&self, session_id: &str, timestamp: i64) -> Result<()> {
        sqlx::query("DELETE FROM messages WHERE session_id = ? AND created_timestamp >= ?")
            .bind(session_id)
            .bind(timestamp)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    fn memory_fact_from_tuple(row: MemoryFactRow) -> MemoryFact {
        MemoryFact {
            id: row.0,
            session_id: row.1,
            category: row.2,
            content: row.3,
            status: row.4.parse::<MemoryFactStatus>().unwrap_or_default(),
            pinned: row.5,
            source: row.6,
            confidence: normalize_memory_confidence(row.7),
            evidence_count: normalize_memory_evidence_count(row.8),
            last_validated_at: row.9,
            validation_command: normalize_validation_command(row.10),
            created_at: row.11,
            updated_at: row.12,
        }
    }

    async fn get_memory_fact_by_id(&self, session_id: &str, fact_id: &str) -> Result<MemoryFact> {
        let row = sqlx::query_as::<_, MemoryFactRow>(
            r#"
            SELECT id, session_id, category, content, status, pinned, source, confidence, evidence_count, last_validated_at, validation_command, created_at, updated_at
            FROM memory_facts
            WHERE session_id = ? AND id = ?
            "#,
        )
        .bind(session_id)
        .bind(fact_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Memory fact not found"))?;

        Ok(Self::memory_fact_from_tuple(row))
    }

    async fn append_memory_edit_log(
        &self,
        session_id: &str,
        fact_id: Option<&str>,
        action: &str,
        before_json: Option<&str>,
        after_json: Option<&str>,
        tx: Option<&mut sqlx::Transaction<'_, Sqlite>>,
    ) -> Result<()> {
        let query = sqlx::query(
            r#"
            INSERT INTO memory_edit_log (session_id, fact_id, action, before_json, after_json)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(session_id)
        .bind(fact_id)
        .bind(action)
        .bind(before_json)
        .bind(after_json);

        if let Some(tx) = tx {
            query.execute(&mut **tx).await?;
        } else {
            query.execute(&self.pool).await?;
        }

        Ok(())
    }

    async fn append_memory_candidates_in_tx(
        &self,
        session_id: &str,
        records: &[MemoryCandidateRecord],
        tx: &mut sqlx::Transaction<'_, Sqlite>,
    ) -> Result<()> {
        for record in records {
            let candidate_id = format!("memc_{}", Uuid::new_v4().simple());
            sqlx::query(
                r#"
                INSERT INTO memory_candidates (id, session_id, category, content, source, decision, reason)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(candidate_id)
            .bind(session_id)
            .bind(&record.category)
            .bind(&record.content)
            .bind(&record.source)
            .bind(&record.decision)
            .bind(&record.reason)
            .execute(&mut **tx)
            .await?;
        }

        sqlx::query(
            r#"
            DELETE FROM memory_candidates
            WHERE id IN (
                SELECT id
                FROM memory_candidates
                WHERE session_id = ?
                ORDER BY created_at DESC, id DESC
                LIMIT -1 OFFSET ?
            )
            "#,
        )
        .bind(session_id)
        .bind(MAX_MEMORY_CANDIDATES_PER_SESSION)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    async fn create_memory_snapshot_in_tx(
        &self,
        session_id: &str,
        reason: &str,
        tx: &mut sqlx::Transaction<'_, Sqlite>,
    ) -> Result<()> {
        let rows = sqlx::query_as::<_, MemoryFactRow>(
            r#"
            SELECT id, session_id, category, content, status, pinned, source, confidence, evidence_count, last_validated_at, validation_command, created_at, updated_at
            FROM memory_facts
            WHERE session_id = ?
            ORDER BY created_at ASC
            "#,
        )
        .bind(session_id)
        .fetch_all(&mut **tx)
        .await?;

        let facts: Vec<MemoryFact> = rows.into_iter().map(Self::memory_fact_from_tuple).collect();

        let facts_json = serde_json::to_string(&facts)?;
        let fact_count = facts.len() as i64;
        let reason = reason.trim();
        let reason = if reason.is_empty() {
            "snapshot"
        } else {
            reason
        };

        sqlx::query(
            r#"
            INSERT INTO memory_snapshots (session_id, reason, facts_json, fact_count)
            VALUES (?, ?, ?, ?)
            "#,
        )
        .bind(session_id)
        .bind(reason)
        .bind(facts_json)
        .bind(fact_count)
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            r#"
            DELETE FROM memory_snapshots
            WHERE id IN (
                SELECT id
                FROM memory_snapshots
                WHERE session_id = ?
                ORDER BY created_at DESC, id DESC
                LIMIT -1 OFFSET ?
            )
            "#,
        )
        .bind(session_id)
        .bind(MAX_MEMORY_SNAPSHOTS_PER_SESSION)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    async fn list_memory_facts(&self, session_id: &str) -> Result<Vec<MemoryFact>> {
        let rows = sqlx::query_as::<_, MemoryFactRow>(
            r#"
            SELECT id, session_id, category, content, status, pinned, source, confidence, evidence_count, last_validated_at, validation_command, created_at, updated_at
            FROM memory_facts
            WHERE session_id = ?
            ORDER BY pinned DESC, updated_at DESC, created_at DESC
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Self::memory_fact_from_tuple).collect())
    }

    async fn list_memory_candidates(
        &self,
        session_id: &str,
        decision: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<MemoryCandidate>> {
        let normalized_decision = decision
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty());
        let limit = i64::from(limit.unwrap_or(120).clamp(1, 500));

        let rows = if let Some(decision) = normalized_decision {
            sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    String,
                    String,
                    String,
                    String,
                    DateTime<Utc>,
                ),
            >(
                r#"
                SELECT id, session_id, category, content, source, decision, reason, created_at
                FROM memory_candidates
                WHERE session_id = ? AND decision = ?
                ORDER BY created_at DESC, id DESC
                LIMIT ?
                "#,
            )
            .bind(session_id)
            .bind(decision)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    String,
                    String,
                    String,
                    String,
                    DateTime<Utc>,
                ),
            >(
                r#"
                SELECT id, session_id, category, content, source, decision, reason, created_at
                FROM memory_candidates
                WHERE session_id = ?
                ORDER BY created_at DESC, id DESC
                LIMIT ?
                "#,
            )
            .bind(session_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows
            .into_iter()
            .map(|row| MemoryCandidate {
                id: row.0,
                session_id: row.1,
                category: row.2,
                content: row.3,
                source: row.4,
                decision: row.5,
                reason: row.6,
                created_at: row.7,
            })
            .collect())
    }

    async fn list_recent_cfpm_tool_gate_events(
        &self,
        session_id: &str,
        limit: Option<u32>,
    ) -> Result<Vec<CfpmToolGateEventRecord>> {
        let requested_limit = limit
            .unwrap_or(DEFAULT_CFPM_TOOL_GATE_EVENTS_LIMIT)
            .clamp(1, MAX_CFPM_TOOL_GATE_EVENTS_LIMIT);
        // Read a wider message window so we can skip non-gate inline notifications.
        let scan_limit = i64::from((requested_limit.saturating_mul(6)).max(40));

        let rows = sqlx::query_as::<_, (String, i64)>(
            r#"
            SELECT content_json, created_timestamp
            FROM messages
            WHERE session_id = ? AND role = 'assistant'
            ORDER BY timestamp DESC
            LIMIT ?
            "#,
        )
        .bind(session_id)
        .bind(scan_limit)
        .fetch_all(&self.pool)
        .await?;

        let mut events: Vec<CfpmToolGateEventRecord> = Vec::new();
        for (content_json, created_timestamp) in rows {
            let contents: Vec<MessageContent> = match serde_json::from_str(&content_json) {
                Ok(value) => value,
                Err(_) => continue,
            };

            for content in contents {
                let MessageContent::SystemNotification(notification) = content else {
                    continue;
                };
                if notification.notification_type != SystemNotificationType::InlineMessage {
                    continue;
                }

                let trimmed = notification.msg.trim();
                if !trimmed.starts_with(CFPM_TOOL_GATE_NOTIFICATION_PREFIX) {
                    continue;
                }

                let payload_json = trimmed
                    .trim_start_matches(CFPM_TOOL_GATE_NOTIFICATION_PREFIX)
                    .trim();
                let Ok(payload) = serde_json::from_str::<CfpmToolGatePayload>(payload_json) else {
                    continue;
                };

                if payload.tool.trim().is_empty()
                    || payload.target.trim().is_empty()
                    || payload.path.trim().is_empty()
                {
                    continue;
                }

                events.push(CfpmToolGateEventRecord {
                    action: if payload.action.trim().is_empty() {
                        "rewrite_known_folder_probe".to_string()
                    } else {
                        payload.action
                    },
                    tool: payload.tool,
                    target: payload.target,
                    path: payload.path,
                    original_command: payload.original_command,
                    rewritten_command: payload.rewritten_command,
                    verbosity: if payload.verbosity.trim().is_empty() {
                        "brief".to_string()
                    } else {
                        payload.verbosity
                    },
                    created_timestamp,
                });

                if events.len() >= requested_limit as usize {
                    return Ok(events);
                }
            }
        }

        Ok(events)
    }

    async fn create_memory_fact(
        &self,
        session_id: &str,
        draft: MemoryFactDraft,
    ) -> Result<MemoryFact> {
        let category = normalize_memory_category(&draft.category);
        let content = normalize_memory_content(&draft.content);
        if content.is_empty() {
            anyhow::bail!("Memory fact content cannot be empty");
        }
        let source = normalize_memory_source(&draft.source);
        let (confidence, evidence_count, last_validated_at, validation_command) =
            resolve_fact_metadata(
                &source,
                &category,
                draft.confidence,
                draft.evidence_count,
                draft.last_validated_at,
                draft.validation_command,
            );
        let fact_id = format!("mem_{}", Uuid::new_v4().simple());

        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"
            INSERT INTO memory_facts (
                id, session_id, category, content, status, pinned, source,
                confidence, evidence_count, last_validated_at, validation_command
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&fact_id)
        .bind(session_id)
        .bind(category)
        .bind(content)
        .bind(MemoryFactStatus::Active.to_string())
        .bind(draft.pinned)
        .bind(source)
        .bind(confidence)
        .bind(evidence_count)
        .bind(last_validated_at)
        .bind(validation_command)
        .execute(&mut *tx)
        .await?;

        let fact = sqlx::query_as::<_, MemoryFactRow>(
            r#"
            SELECT id, session_id, category, content, status, pinned, source, confidence, evidence_count, last_validated_at, validation_command, created_at, updated_at
            FROM memory_facts
            WHERE id = ? AND session_id = ?
            "#,
        )
        .bind(&fact_id)
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await
        .map(Self::memory_fact_from_tuple)?;

        let after_json = serde_json::to_string(&fact)?;
        self.append_memory_edit_log(
            session_id,
            Some(&fact.id),
            "create",
            None,
            Some(&after_json),
            Some(&mut tx),
        )
        .await?;

        tx.commit().await?;
        Ok(fact)
    }

    async fn update_memory_fact(
        &self,
        session_id: &str,
        fact_id: &str,
        patch: MemoryFactPatch,
    ) -> Result<MemoryFact> {
        let before = self.get_memory_fact_by_id(session_id, fact_id).await?;
        let mut category = before.category.clone();
        let mut content = before.content.clone();
        let mut status = before.status.clone();
        let mut pinned = before.pinned;
        let mut confidence = before.confidence;
        let mut evidence_count = before.evidence_count;
        let mut last_validated_at = before.last_validated_at;
        let mut validation_command = before.validation_command.clone();

        if let Some(next_category) = patch.category.as_deref() {
            category = normalize_memory_category(next_category);
        }
        if let Some(next_content) = patch.content.as_deref() {
            let normalized = normalize_memory_content(next_content);
            if normalized.is_empty() {
                anyhow::bail!("Memory fact content cannot be empty");
            }
            content = normalized;
        }
        if let Some(next_status) = patch.status {
            status = next_status;
        }
        if let Some(next_pinned) = patch.pinned {
            pinned = next_pinned;
        }

        let changed = category != before.category
            || content != before.content
            || status != before.status
            || pinned != before.pinned;
        if !changed {
            return Ok(before);
        }

        // If an auto-generated CFPM fact is edited manually, promote its source.
        let next_source = if before.source == MEMORY_SOURCE_CFPM_AUTO {
            MEMORY_SOURCE_USER.to_string()
        } else {
            before.source.clone()
        };
        if next_source == MEMORY_SOURCE_USER {
            confidence = confidence.max(DEFAULT_MEMORY_CONFIDENCE_USER);
        }
        if is_artifact_category(&category) || is_invalid_path_category(&category) {
            last_validated_at = Some(Utc::now());
            evidence_count = normalize_memory_evidence_count(evidence_count.saturating_add(1));
        }
        confidence = normalize_memory_confidence(confidence);
        validation_command = normalize_validation_command(validation_command);

        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"
            UPDATE memory_facts
            SET category = ?, content = ?, status = ?, pinned = ?, source = ?, confidence = ?, evidence_count = ?, last_validated_at = ?, validation_command = ?, updated_at = datetime('now')
            WHERE session_id = ? AND id = ?
            "#,
        )
        .bind(&category)
        .bind(&content)
        .bind(status.to_string())
        .bind(pinned)
        .bind(next_source)
        .bind(confidence)
        .bind(evidence_count)
        .bind(last_validated_at)
        .bind(validation_command)
        .bind(session_id)
        .bind(fact_id)
        .execute(&mut *tx)
        .await?;

        let after = sqlx::query_as::<_, MemoryFactRow>(
            r#"
            SELECT id, session_id, category, content, status, pinned, source, confidence, evidence_count, last_validated_at, validation_command, created_at, updated_at
            FROM memory_facts
            WHERE session_id = ? AND id = ?
            "#,
        )
        .bind(session_id)
        .bind(fact_id)
        .fetch_one(&mut *tx)
        .await
        .map(Self::memory_fact_from_tuple)?;

        let before_json = serde_json::to_string(&before)?;
        let after_json = serde_json::to_string(&after)?;
        self.append_memory_edit_log(
            session_id,
            Some(fact_id),
            "update",
            Some(&before_json),
            Some(&after_json),
            Some(&mut tx),
        )
        .await?;

        tx.commit().await?;
        Ok(after)
    }

    async fn rename_memory_paths(
        &self,
        session_id: &str,
        from_path: &str,
        to_path: &str,
    ) -> Result<u64> {
        let from_path = from_path.trim();
        let to_path = to_path.trim();
        if from_path.is_empty() || to_path.is_empty() || from_path == to_path {
            return Ok(0);
        }

        let mut tx = self.pool.begin().await?;
        let affected_rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                bool,
                String,
                f64,
                i64,
                Option<DateTime<Utc>>,
                Option<String>,
            ),
        >(
            r#"
            SELECT id, category, content, pinned, source, confidence, evidence_count, last_validated_at, validation_command
            FROM memory_facts
            WHERE session_id = ?
              AND status IN ('active', 'stale')
              AND instr(content, ?) > 0
            ORDER BY updated_at DESC, created_at DESC
            "#,
        )
        .bind(session_id)
        .bind(from_path)
        .fetch_all(&mut *tx)
        .await?;

        let mut inserted_count = 0_u64;
        let mut superseded_count = 0_u64;
        let mut skipped_count = 0_u64;
        let mut inserted_dedupe = HashSet::new();

        for (
            fact_id,
            category,
            content,
            pinned,
            source,
            confidence,
            evidence_count,
            last_validated_at,
            validation_command,
        ) in affected_rows
        {
            let replaced_content = normalize_memory_content(&content.replace(from_path, to_path));
            if replaced_content == normalize_memory_content(&content) || replaced_content.is_empty()
            {
                continue;
            }

            superseded_count += 1;
            sqlx::query(
                r#"
                UPDATE memory_facts
                SET status = ?, updated_at = datetime('now')
                WHERE session_id = ? AND id = ?
                "#,
            )
            .bind(MemoryFactStatus::Superseded.to_string())
            .bind(session_id)
            .bind(&fact_id)
            .execute(&mut *tx)
            .await?;

            let dedupe_key = format!(
                "{}::{}::{}",
                category,
                replaced_content.to_ascii_lowercase(),
                source
            );
            if !inserted_dedupe.insert(dedupe_key) {
                skipped_count += 1;
                continue;
            }

            let existing = sqlx::query_scalar::<_, i64>(
                r#"
                SELECT COUNT(*)
                FROM memory_facts
                WHERE session_id = ?
                  AND category = ?
                  AND lower(content) = lower(?)
                  AND source = ?
                  AND status != 'forgotten'
                "#,
            )
            .bind(session_id)
            .bind(&category)
            .bind(&replaced_content)
            .bind(&source)
            .fetch_one(&mut *tx)
            .await
            .unwrap_or(0);
            if existing > 0 {
                skipped_count += 1;
                continue;
            }

            let new_fact_id = format!("mem_{}", Uuid::new_v4().simple());
            let (confidence, evidence_count, last_validated_at, validation_command) =
                resolve_fact_metadata(
                    &source,
                    &category,
                    Some(confidence),
                    Some(evidence_count.saturating_add(1)),
                    last_validated_at,
                    validation_command,
                );
            sqlx::query(
                r#"
                INSERT INTO memory_facts (
                    id, session_id, category, content, status, pinned, source,
                    confidence, evidence_count, last_validated_at, validation_command
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(new_fact_id)
            .bind(session_id)
            .bind(&category)
            .bind(&replaced_content)
            .bind(MemoryFactStatus::Active.to_string())
            .bind(pinned)
            .bind(&source)
            .bind(confidence)
            .bind(evidence_count)
            .bind(last_validated_at)
            .bind(validation_command)
            .execute(&mut *tx)
            .await?;
            inserted_count += 1;
        }

        let after_json = serde_json::json!({
            "fromPath": from_path,
            "toPath": to_path,
            "rowsAffected": superseded_count,
            "insertedCount": inserted_count,
            "skippedCount": skipped_count,
        })
        .to_string();
        self.append_memory_edit_log(
            session_id,
            None,
            "rename_path",
            None,
            Some(&after_json),
            Some(&mut tx),
        )
        .await?;

        tx.commit().await?;
        Ok(superseded_count)
    }

    async fn list_memory_snapshots(&self, session_id: &str) -> Result<Vec<MemorySnapshotRecord>> {
        let rows = sqlx::query_as::<_, (i64, String, String, i64, DateTime<Utc>)>(
            r#"
            SELECT id, session_id, reason, fact_count, created_at
            FROM memory_snapshots
            WHERE session_id = ?
            ORDER BY created_at DESC, id DESC
            LIMIT 50
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| MemorySnapshotRecord {
                id: row.0,
                session_id: row.1,
                reason: row.2,
                fact_count: row.3,
                created_at: row.4,
            })
            .collect())
    }

    async fn rollback_memory_snapshot(&self, session_id: &str, snapshot_id: i64) -> Result<u64> {
        let snapshot_row = sqlx::query_as::<_, (String, String, i64)>(
            r#"
            SELECT facts_json, reason, fact_count
            FROM memory_snapshots
            WHERE session_id = ? AND id = ?
            "#,
        )
        .bind(session_id)
        .bind(snapshot_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Memory snapshot not found"))?;

        let facts: Vec<MemoryFact> = serde_json::from_str(&snapshot_row.0)?;
        let mut tx = self.pool.begin().await?;
        self.create_memory_snapshot_in_tx(
            session_id,
            &format!("rollback_backup_from_{}", snapshot_id),
            &mut tx,
        )
        .await?;

        sqlx::query("DELETE FROM memory_facts WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        for fact in &facts {
            let category = normalize_memory_category(&fact.category);
            let content = normalize_memory_content(&fact.content);
            if content.is_empty() {
                continue;
            }
            let source = normalize_memory_source(&fact.source);
            sqlx::query(
                r#"
                INSERT INTO memory_facts (
                    id, session_id, category, content, status, pinned, source,
                    confidence, evidence_count, last_validated_at, validation_command,
                    created_at, updated_at
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&fact.id)
            .bind(session_id)
            .bind(category)
            .bind(content)
            .bind(fact.status.to_string())
            .bind(fact.pinned)
            .bind(source)
            .bind(normalize_memory_confidence(fact.confidence))
            .bind(normalize_memory_evidence_count(fact.evidence_count))
            .bind(fact.last_validated_at)
            .bind(normalize_validation_command(
                fact.validation_command.clone(),
            ))
            .bind(fact.created_at)
            .bind(fact.updated_at)
            .execute(&mut *tx)
            .await?;
        }

        let after_json = serde_json::json!({
            "snapshotId": snapshot_id,
            "snapshotReason": snapshot_row.1,
            "snapshotFactCount": snapshot_row.2,
            "restoredFactCount": facts.len(),
        })
        .to_string();
        self.append_memory_edit_log(
            session_id,
            None,
            "rollback_snapshot",
            None,
            Some(&after_json),
            Some(&mut tx),
        )
        .await?;

        tx.commit().await?;
        Ok(facts.len() as u64)
    }

    async fn replace_cfpm_memory_facts(
        &self,
        session_id: &str,
        drafts: Vec<MemoryFactDraft>,
        reason: &str,
    ) -> Result<()> {
        let mut normalized_drafts = Vec::new();
        let mut candidate_records = Vec::new();
        let mut seen = HashSet::new();
        for draft in drafts {
            let category = normalize_memory_category(&draft.category);
            let content = normalize_memory_content(&draft.content);
            if content.is_empty() {
                candidate_records.push(MemoryCandidateRecord {
                    category,
                    content,
                    source: MEMORY_SOURCE_CFPM_AUTO.to_string(),
                    decision: "rejected".to_string(),
                    reason: "empty_content".to_string(),
                });
                continue;
            }

            if let Err(reason) = evaluate_cfpm_auto_candidate(&category, &content) {
                candidate_records.push(MemoryCandidateRecord {
                    category,
                    content,
                    source: MEMORY_SOURCE_CFPM_AUTO.to_string(),
                    decision: "rejected".to_string(),
                    reason: reason.to_string(),
                });
                continue;
            }

            let key = format!("{}::{}", category, content.to_ascii_lowercase());
            if !seen.insert(key) {
                candidate_records.push(MemoryCandidateRecord {
                    category,
                    content,
                    source: MEMORY_SOURCE_CFPM_AUTO.to_string(),
                    decision: "rejected".to_string(),
                    reason: "duplicate".to_string(),
                });
                continue;
            }

            let (confidence, evidence_count, last_validated_at, validation_command) =
                resolve_fact_metadata(
                    MEMORY_SOURCE_CFPM_AUTO,
                    &category,
                    draft.confidence,
                    draft.evidence_count,
                    draft.last_validated_at,
                    draft.validation_command,
                );
            normalized_drafts.push(MemoryFactDraft {
                category: category.clone(),
                content: content.clone(),
                source: MEMORY_SOURCE_CFPM_AUTO.to_string(),
                pinned: false,
                confidence: Some(confidence),
                evidence_count: Some(evidence_count),
                last_validated_at,
                validation_command,
            });
        }

        let invalid_paths = collect_invalid_path_canonicals_from_drafts(&normalized_drafts);
        let mut filtered_drafts = Vec::new();
        for draft in normalized_drafts {
            if artifact_conflicts_with_invalid_paths(
                &draft.category,
                &draft.content,
                &invalid_paths,
            ) {
                candidate_records.push(MemoryCandidateRecord {
                    category: draft.category,
                    content: draft.content,
                    source: MEMORY_SOURCE_CFPM_AUTO.to_string(),
                    decision: "rejected".to_string(),
                    reason: "artifact_marked_invalid".to_string(),
                });
                continue;
            }
            candidate_records.push(MemoryCandidateRecord {
                category: draft.category.clone(),
                content: draft.content.clone(),
                source: MEMORY_SOURCE_CFPM_AUTO.to_string(),
                decision: "accepted".to_string(),
                reason: "accepted".to_string(),
            });
            filtered_drafts.push(draft);
        }

        let mut tx = self.pool.begin().await?;
        self.create_memory_snapshot_in_tx(session_id, reason, &mut tx)
            .await?;
        self.append_memory_candidates_in_tx(session_id, &candidate_records, &mut tx)
            .await?;

        sqlx::query("DELETE FROM memory_facts WHERE session_id = ? AND source = ?")
            .bind(session_id)
            .bind(MEMORY_SOURCE_CFPM_AUTO)
            .execute(&mut *tx)
            .await?;

        for draft in filtered_drafts {
            let fact_id = format!("mem_{}", Uuid::new_v4().simple());
            sqlx::query(
                r#"
                INSERT INTO memory_facts (
                    id, session_id, category, content, status, pinned, source,
                    confidence, evidence_count, last_validated_at, validation_command
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(fact_id)
            .bind(session_id)
            .bind(draft.category)
            .bind(draft.content)
            .bind(MemoryFactStatus::Active.to_string())
            .bind(draft.pinned)
            .bind(MEMORY_SOURCE_CFPM_AUTO)
            .bind(draft.confidence.unwrap_or(DEFAULT_MEMORY_CONFIDENCE_CFPM))
            .bind(draft.evidence_count.unwrap_or(1))
            .bind(draft.last_validated_at)
            .bind(draft.validation_command)
            .execute(&mut *tx)
            .await?;
        }

        let rejected_reason_breakdown = collect_rejected_reason_breakdown(&candidate_records);

        let after_json = serde_json::json!({
            "source": MEMORY_SOURCE_CFPM_AUTO,
            "reason": reason,
            "factCount": sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM memory_facts WHERE session_id = ? AND source = ?"
            )
            .bind(session_id)
            .bind(MEMORY_SOURCE_CFPM_AUTO)
            .fetch_one(&mut *tx)
            .await
            .unwrap_or(0),
            "candidateAccepted": candidate_records
                .iter()
                .filter(|record| record.decision == "accepted")
                .count(),
            "candidateRejected": candidate_records
                .iter()
                .filter(|record| record.decision == "rejected")
                .count(),
            "rejectedReasonBreakdown": rejected_reason_breakdown.clone(),
        })
        .to_string();
        self.append_memory_edit_log(
            session_id,
            None,
            "replace_cfpm_auto",
            None,
            Some(&after_json),
            Some(&mut tx),
        )
        .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn merge_cfpm_memory_facts(
        &self,
        session_id: &str,
        drafts: Vec<MemoryFactDraft>,
        reason: &str,
    ) -> Result<CfpmRuntimeReport> {
        let mut incoming_candidates = Vec::new();
        let mut candidate_records = Vec::new();
        let mut incoming_dedupe = HashSet::new();
        for draft in drafts {
            let category = normalize_memory_category(&draft.category);
            let content = normalize_memory_content(&draft.content);
            if content.is_empty() {
                candidate_records.push(MemoryCandidateRecord {
                    category,
                    content,
                    source: MEMORY_SOURCE_CFPM_AUTO.to_string(),
                    decision: "rejected".to_string(),
                    reason: "empty_content".to_string(),
                });
                continue;
            }

            if let Err(reason) = evaluate_cfpm_auto_candidate(&category, &content) {
                candidate_records.push(MemoryCandidateRecord {
                    category,
                    content,
                    source: MEMORY_SOURCE_CFPM_AUTO.to_string(),
                    decision: "rejected".to_string(),
                    reason: reason.to_string(),
                });
                continue;
            }

            let key = format!("{}::{}", category, content.to_ascii_lowercase());
            if !incoming_dedupe.insert(key) {
                candidate_records.push(MemoryCandidateRecord {
                    category,
                    content,
                    source: MEMORY_SOURCE_CFPM_AUTO.to_string(),
                    decision: "rejected".to_string(),
                    reason: "duplicate".to_string(),
                });
                continue;
            }

            let (confidence, evidence_count, last_validated_at, validation_command) =
                resolve_fact_metadata(
                    MEMORY_SOURCE_CFPM_AUTO,
                    &category,
                    draft.confidence,
                    draft.evidence_count,
                    draft.last_validated_at,
                    draft.validation_command,
                );
            incoming_candidates.push(MemoryFactDraft {
                category: category.clone(),
                content: content.clone(),
                source: MEMORY_SOURCE_CFPM_AUTO.to_string(),
                pinned: draft.pinned,
                confidence: Some(confidence),
                evidence_count: Some(evidence_count),
                last_validated_at,
                validation_command,
            });
        }

        let existing_facts = sqlx::query_as::<_, MemoryFactRow>(
            r#"
            SELECT id, session_id, category, content, status, pinned, source, confidence, evidence_count, last_validated_at, validation_command, created_at, updated_at
            FROM memory_facts
            WHERE session_id = ? AND source = ?
            ORDER BY pinned DESC, updated_at DESC, created_at DESC
            "#,
        )
        .bind(session_id)
        .bind(MEMORY_SOURCE_CFPM_AUTO)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(Self::memory_fact_from_tuple)
        .collect::<Vec<_>>();

        let mut invalid_paths = collect_invalid_path_canonicals_from_memory_facts(&existing_facts);
        invalid_paths.extend(collect_invalid_path_canonicals_from_drafts(
            &incoming_candidates,
        ));

        let mut incoming = Vec::new();
        for draft in incoming_candidates {
            if artifact_conflicts_with_invalid_paths(
                &draft.category,
                &draft.content,
                &invalid_paths,
            ) {
                candidate_records.push(MemoryCandidateRecord {
                    category: draft.category,
                    content: draft.content,
                    source: MEMORY_SOURCE_CFPM_AUTO.to_string(),
                    decision: "rejected".to_string(),
                    reason: "artifact_marked_invalid".to_string(),
                });
                continue;
            }
            candidate_records.push(MemoryCandidateRecord {
                category: draft.category.clone(),
                content: draft.content.clone(),
                source: MEMORY_SOURCE_CFPM_AUTO.to_string(),
                decision: "accepted".to_string(),
                reason: "accepted".to_string(),
            });
            incoming.push(draft);
        }

        let accepted_count = candidate_records
            .iter()
            .filter(|record| record.decision == "accepted")
            .count() as u32;
        let rejected_count = candidate_records
            .iter()
            .filter(|record| record.decision == "rejected")
            .count() as u32;
        let rejected_reason_breakdown = collect_rejected_reason_breakdown(&candidate_records);

        if incoming.is_empty() {
            if !candidate_records.is_empty() {
                let mut tx = self.pool.begin().await?;
                self.append_memory_candidates_in_tx(session_id, &candidate_records, &mut tx)
                    .await?;
                tx.commit().await?;
            }
            let fact_count = self.count_active_cfpm_auto_facts(session_id).await? as u32;
            return Ok(CfpmRuntimeReport {
                reason: reason.to_string(),
                mode: "candidate_only".to_string(),
                accepted_count,
                rejected_count,
                rejected_reason_breakdown,
                pruned_count: 0,
                fact_count,
            });
        }

        let mut tx = self.pool.begin().await?;
        self.create_memory_snapshot_in_tx(session_id, reason, &mut tx)
            .await?;
        self.append_memory_candidates_in_tx(session_id, &candidate_records, &mut tx)
            .await?;

        let mut merged: Vec<(
            String,
            String,
            MemoryFactStatus,
            bool,
            f64,
            i64,
            Option<DateTime<Utc>>,
            Option<String>,
        )> = Vec::new();
        let mut dedupe: HashMap<String, usize> = HashMap::new();

        for fact in existing_facts {
            let category = normalize_memory_category(&fact.category);
            let content = normalize_memory_content(&fact.content);
            if content.is_empty() {
                continue;
            }
            if evaluate_cfpm_auto_candidate(&category, &content).is_err() {
                continue;
            }
            if artifact_conflicts_with_invalid_paths(&category, &content, &invalid_paths) {
                continue;
            }
            let key = format!("{}::{}", category, content.to_ascii_lowercase());
            if let std::collections::hash_map::Entry::Vacant(entry) = dedupe.entry(key) {
                let idx = merged.len();
                entry.insert(idx);
                merged.push((
                    category,
                    content,
                    fact.status,
                    fact.pinned,
                    normalize_memory_confidence(fact.confidence),
                    normalize_memory_evidence_count(fact.evidence_count),
                    fact.last_validated_at,
                    normalize_validation_command(fact.validation_command),
                ));
            }
            if merged.len() >= MAX_CFPM_AUTO_FACTS {
                break;
            }
        }

        for draft in incoming {
            let key = format!("{}::{}", draft.category, draft.content.to_ascii_lowercase());
            if let Some(existing_idx) = dedupe.get(&key).copied() {
                if let Some(existing) = merged.get_mut(existing_idx) {
                    let incoming_confidence =
                        normalize_memory_confidence(draft.confidence.unwrap_or(
                            default_confidence_for_fact(MEMORY_SOURCE_CFPM_AUTO, &draft.category),
                        ));
                    let incoming_evidence =
                        normalize_memory_evidence_count(draft.evidence_count.unwrap_or(1));
                    let total_evidence = normalize_memory_evidence_count(
                        existing.5.saturating_add(incoming_evidence),
                    );
                    let weighted_confidence = ((existing.4 * existing.5 as f64)
                        + (incoming_confidence * incoming_evidence as f64))
                        / total_evidence as f64;
                    existing.4 = normalize_memory_confidence(weighted_confidence);
                    existing.5 = total_evidence;
                    existing.2 = MemoryFactStatus::Active;
                    existing.3 = existing.3 || draft.pinned;
                    existing.6 =
                        merge_validation_timestamp(existing.6.clone(), draft.last_validated_at);
                    if draft
                        .validation_command
                        .as_ref()
                        .is_some_and(|cmd| !cmd.trim().is_empty())
                    {
                        existing.7 = normalize_validation_command(draft.validation_command);
                    }
                }
                continue;
            }

            if merged.len() >= MAX_CFPM_AUTO_FACTS {
                break;
            }
            let idx = merged.len();
            dedupe.insert(key, idx);
            merged.push((
                draft.category.clone(),
                draft.content.clone(),
                MemoryFactStatus::Active,
                draft.pinned,
                normalize_memory_confidence(draft.confidence.unwrap_or(
                    default_confidence_for_fact(MEMORY_SOURCE_CFPM_AUTO, &draft.category),
                )),
                normalize_memory_evidence_count(draft.evidence_count.unwrap_or(1)),
                draft.last_validated_at,
                normalize_validation_command(draft.validation_command),
            ));
        }

        sqlx::query("DELETE FROM memory_facts WHERE session_id = ? AND source = ?")
            .bind(session_id)
            .bind(MEMORY_SOURCE_CFPM_AUTO)
            .execute(&mut *tx)
            .await?;

        for (
            category,
            content,
            status,
            pinned,
            confidence,
            evidence_count,
            last_validated_at,
            validation_command,
        ) in &merged
        {
            let fact_id = format!("mem_{}", Uuid::new_v4().simple());
            sqlx::query(
                r#"
                INSERT INTO memory_facts (
                    id, session_id, category, content, status, pinned, source,
                    confidence, evidence_count, last_validated_at, validation_command
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(fact_id)
            .bind(session_id)
            .bind(category)
            .bind(content)
            .bind(status.to_string())
            .bind(*pinned)
            .bind(MEMORY_SOURCE_CFPM_AUTO)
            .bind(*confidence)
            .bind(*evidence_count)
            .bind(last_validated_at.clone())
            .bind(validation_command.clone())
            .execute(&mut *tx)
            .await?;
        }

        let after_json = serde_json::json!({
            "source": MEMORY_SOURCE_CFPM_AUTO,
            "reason": reason,
            "factCount": merged.len(),
            "mode": "merge",
            "candidateAccepted": accepted_count,
            "candidateRejected": rejected_count,
            "rejectedReasonBreakdown": rejected_reason_breakdown.clone(),
        })
        .to_string();
        self.append_memory_edit_log(
            session_id,
            None,
            "merge_cfpm_auto",
            None,
            Some(&after_json),
            Some(&mut tx),
        )
        .await?;

        tx.commit().await?;
        Ok(CfpmRuntimeReport {
            reason: reason.to_string(),
            mode: "merge".to_string(),
            accepted_count,
            rejected_count,
            rejected_reason_breakdown,
            pruned_count: 0,
            fact_count: merged.len() as u32,
        })
    }

    async fn prune_cfpm_auto_memory_facts(&self, session_id: &str, reason: &str) -> Result<u64> {
        let rows = sqlx::query_as::<_, (String, String, String)>(
            r#"
            SELECT id, category, content
            FROM memory_facts
            WHERE session_id = ? AND source = ? AND status = 'active'
            ORDER BY pinned DESC, updated_at DESC, created_at DESC
            "#,
        )
        .bind(session_id)
        .bind(MEMORY_SOURCE_CFPM_AUTO)
        .fetch_all(&self.pool)
        .await?;

        if rows.is_empty() {
            return Ok(0);
        }

        let mut invalid_paths = HashSet::new();
        for (_, category, content) in &rows {
            let normalized_category = normalize_memory_category(category);
            if !is_invalid_path_category(&normalized_category) {
                continue;
            }
            invalid_paths.extend(collect_canonical_paths_for_compare(content));
        }

        let mut keep_ids = HashSet::new();
        let mut dedupe = HashSet::new();

        for (fact_id, category, content) in &rows {
            let category = normalize_memory_category(category);
            let content = normalize_memory_content(content);
            if content.is_empty() {
                continue;
            }
            if evaluate_cfpm_auto_candidate(&category, &content).is_err() {
                continue;
            }
            if artifact_conflicts_with_invalid_paths(&category, &content, &invalid_paths) {
                continue;
            }

            let key = format!("{}::{}", category, content.to_ascii_lowercase());
            if !dedupe.insert(key) {
                continue;
            }

            keep_ids.insert(fact_id.clone());
            if keep_ids.len() >= MAX_CFPM_AUTO_FACTS {
                break;
            }
        }

        let remove_ids: Vec<String> = rows
            .into_iter()
            .filter_map(|(fact_id, _, _)| (!keep_ids.contains(&fact_id)).then_some(fact_id))
            .collect();
        if remove_ids.is_empty() {
            return Ok(0);
        }

        let mut tx = self.pool.begin().await?;
        self.create_memory_snapshot_in_tx(
            session_id,
            &format!("{}_prune_cfpm_auto", reason),
            &mut tx,
        )
        .await?;

        for fact_id in &remove_ids {
            sqlx::query("DELETE FROM memory_facts WHERE session_id = ? AND id = ?")
                .bind(session_id)
                .bind(fact_id)
                .execute(&mut *tx)
                .await?;
        }

        let after_json = serde_json::json!({
            "source": MEMORY_SOURCE_CFPM_AUTO,
            "reason": reason,
            "removedCount": remove_ids.len(),
            "keptCount": keep_ids.len(),
            "action": "prune_cfpm_auto",
        })
        .to_string();
        self.append_memory_edit_log(
            session_id,
            None,
            "prune_cfpm_auto",
            None,
            Some(&after_json),
            Some(&mut tx),
        )
        .await?;

        tx.commit().await?;
        Ok(remove_ids.len() as u64)
    }

    async fn count_active_cfpm_auto_facts(&self, session_id: &str) -> Result<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM memory_facts WHERE session_id = ? AND source = ? AND status = 'active'",
        )
        .bind(session_id)
        .bind(MEMORY_SOURCE_CFPM_AUTO)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        Ok(count)
    }

    async fn search_chat_history(
        &self,
        query: &str,
        limit: Option<usize>,
        after_date: Option<chrono::DateTime<chrono::Utc>>,
        before_date: Option<chrono::DateTime<chrono::Utc>>,
        exclude_session_id: Option<String>,
    ) -> Result<crate::session::chat_history_search::ChatRecallResults> {
        use crate::session::chat_history_search::ChatHistorySearch;

        ChatHistorySearch::new(
            &self.pool,
            query,
            limit,
            after_date,
            before_date,
            exclude_session_id,
        )
        .execute()
        .await
    }

    // ============ Shared Session Methods ============

    /// Create a new shared session
    #[allow(clippy::too_many_arguments)]
    pub async fn create_shared_session(
        &self,
        share_token: &str,
        name: &str,
        working_dir: &str,
        messages: &str,
        message_count: i32,
        total_tokens: Option<i32>,
        expires_at: Option<DateTime<Utc>>,
        password_hash: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO shared_sessions (
                share_token, name, working_dir, messages, message_count,
                total_tokens, expires_at, password_hash
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(share_token)
        .bind(name)
        .bind(working_dir)
        .bind(messages)
        .bind(message_count)
        .bind(total_tokens)
        .bind(expires_at)
        .bind(password_hash)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get a shared session by token
    pub async fn get_shared_session(&self, share_token: &str) -> Result<SharedSession> {
        let row = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                i32,
                Option<i32>,
                DateTime<Utc>,
                Option<DateTime<Utc>>,
                Option<String>,
            ),
        >(
            r#"
            SELECT share_token, name, working_dir, messages, message_count,
                   total_tokens, created_at, expires_at, password_hash
            FROM shared_sessions
            WHERE share_token = ?
            "#,
        )
        .bind(share_token)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Shared session not found"))?;

        Ok(SharedSession {
            share_token: row.0,
            name: row.1,
            working_dir: row.2,
            messages: row.3,
            message_count: row.4,
            total_tokens: row.5,
            created_at: row.6,
            expires_at: row.7,
            password_hash: row.8,
        })
    }

    /// Delete expired shared sessions
    pub async fn cleanup_expired_shares(&self) -> Result<u64> {
        let result = sqlx::query(
            r#"
            DELETE FROM shared_sessions
            WHERE expires_at IS NOT NULL AND expires_at < datetime('now')
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Delete a shared session by token
    pub async fn delete_shared_session(&self, share_token: &str) -> Result<()> {
        sqlx::query("DELETE FROM shared_sessions WHERE share_token = ?")
            .bind(share_token)
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::{Message, MessageContent};
    use rmcp::model::{AnnotateAble, RawContent};
    use tempfile::TempDir;

    const NUM_CONCURRENT_SESSIONS: i32 = 10;

    #[tokio::test]
    async fn test_concurrent_session_creation() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_sessions.db");

        let storage = Arc::new(SessionStorage::create(&db_path).await.unwrap());

        let mut handles = vec![];

        for i in 0..NUM_CONCURRENT_SESSIONS {
            let session_storage = Arc::clone(&storage);
            let handle = tokio::spawn(async move {
                let working_dir = PathBuf::from(format!("/tmp/test_{}", i));
                let description = format!("Test session {}", i);

                let session = session_storage
                    .create_session(working_dir.clone(), description, SessionType::User)
                    .await
                    .unwrap();

                session_storage
                    .add_message(
                        &session.id,
                        &Message {
                            id: None,
                            role: Role::User,
                            created: chrono::Utc::now().timestamp_millis(),
                            content: vec![MessageContent::text("hello world")],
                            metadata: Default::default(),
                        },
                    )
                    .await
                    .unwrap();

                session_storage
                    .add_message(
                        &session.id,
                        &Message {
                            id: None,
                            role: Role::Assistant,
                            created: chrono::Utc::now().timestamp_millis(),
                            content: vec![MessageContent::text("sup world?")],
                            metadata: Default::default(),
                        },
                    )
                    .await
                    .unwrap();

                session_storage
                    .apply_update(
                        SessionUpdateBuilder::new(session.id.clone())
                            .user_provided_name(format!("Updated session {}", i))
                            .total_tokens(Some(100 * i)),
                    )
                    .await
                    .unwrap();

                let updated = session_storage
                    .get_session(&session.id, true)
                    .await
                    .unwrap();
                assert_eq!(updated.message_count, 2);
                assert_eq!(updated.total_tokens, Some(100 * i));

                session.id
            });
            handles.push(handle);
        }

        let mut results = vec![];
        for handle in handles {
            results.push(handle.await.unwrap());
        }

        assert_eq!(results.len(), NUM_CONCURRENT_SESSIONS as usize);

        let unique_ids: std::collections::HashSet<_> = results.iter().collect();
        assert_eq!(unique_ids.len(), NUM_CONCURRENT_SESSIONS as usize);

        let sessions = storage.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), NUM_CONCURRENT_SESSIONS as usize);

        for session in &sessions {
            assert_eq!(session.message_count, 2);
            assert!(session.name.starts_with("Updated session"));
        }

        let insights = storage.get_insights().await.unwrap();
        assert_eq!(insights.total_sessions, NUM_CONCURRENT_SESSIONS as usize);
        let expected_tokens = 100 * NUM_CONCURRENT_SESSIONS * (NUM_CONCURRENT_SESSIONS - 1) / 2;
        assert_eq!(insights.total_tokens, expected_tokens as i64);
    }

    #[tokio::test]
    async fn test_export_import_roundtrip() {
        const DESCRIPTION: &str = "Original session";
        const TOTAL_TOKENS: i32 = 500;
        const INPUT_TOKENS: i32 = 300;
        const OUTPUT_TOKENS: i32 = 200;
        const ACCUMULATED_TOKENS: i32 = 1000;
        const USER_MESSAGE: &str = "test message";
        const ASSISTANT_MESSAGE: &str = "test response";

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_export.db");
        let storage = Arc::new(SessionStorage::create(&db_path).await.unwrap());

        let original = storage
            .create_session(
                PathBuf::from("/tmp/test"),
                DESCRIPTION.to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        storage
            .apply_update(
                SessionUpdateBuilder::new(original.id.clone())
                    .total_tokens(Some(TOTAL_TOKENS))
                    .input_tokens(Some(INPUT_TOKENS))
                    .output_tokens(Some(OUTPUT_TOKENS))
                    .accumulated_total_tokens(Some(ACCUMULATED_TOKENS)),
            )
            .await
            .unwrap();

        storage
            .add_message(
                &original.id,
                &Message {
                    id: None,
                    role: Role::User,
                    created: chrono::Utc::now().timestamp_millis(),
                    content: vec![MessageContent::text(USER_MESSAGE)],
                    metadata: Default::default(),
                },
            )
            .await
            .unwrap();

        storage
            .add_message(
                &original.id,
                &Message {
                    id: None,
                    role: Role::Assistant,
                    created: chrono::Utc::now().timestamp_millis(),
                    content: vec![MessageContent::text(ASSISTANT_MESSAGE)],
                    metadata: Default::default(),
                },
            )
            .await
            .unwrap();

        let exported = storage.export_session(&original.id).await.unwrap();
        let imported = storage.import_session(&exported).await.unwrap();

        assert_ne!(imported.id, original.id);
        assert_eq!(imported.name, DESCRIPTION);
        assert_eq!(imported.working_dir, PathBuf::from("/tmp/test"));
        assert_eq!(imported.total_tokens, Some(TOTAL_TOKENS));
        assert_eq!(imported.input_tokens, Some(INPUT_TOKENS));
        assert_eq!(imported.output_tokens, Some(OUTPUT_TOKENS));
        assert_eq!(imported.accumulated_total_tokens, Some(ACCUMULATED_TOKENS));
        assert_eq!(imported.message_count, 2);

        let conversation = imported.conversation.unwrap();
        assert_eq!(conversation.messages().len(), 2);
        assert_eq!(conversation.messages()[0].role, Role::User);
        assert_eq!(conversation.messages()[1].role, Role::Assistant);
    }

    #[tokio::test]
    async fn test_import_session_with_description_field() {
        const OLD_FORMAT_JSON: &str = r#"{
            "id": "20240101_1",
            "description": "Old format session",
            "user_set_name": true,
            "working_dir": "/tmp/test",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "extension_data": {},
            "message_count": 0
        }"#;

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_import.db");
        let storage = Arc::new(SessionStorage::create(&db_path).await.unwrap());

        let imported = storage.import_session(OLD_FORMAT_JSON).await.unwrap();

        assert_eq!(imported.name, "Old format session");
        assert!(imported.user_set_name);
        assert_eq!(imported.working_dir, PathBuf::from("/tmp/test"));
    }

    #[tokio::test]
    async fn test_memory_fact_crud_and_path_rename() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_memory_crud.db");
        let storage = Arc::new(SessionStorage::create(&db_path).await.unwrap());
        let session = storage
            .create_session(
                PathBuf::from("/tmp/test"),
                "memory".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let created = storage
            .create_memory_fact(
                &session.id,
                MemoryFactDraft::new("artifact", "saved at C:\\work\\old\\result.txt", "user"),
            )
            .await
            .unwrap();
        assert_eq!(created.status, MemoryFactStatus::Active);
        assert_eq!(created.source, MEMORY_SOURCE_USER);

        let updated = storage
            .update_memory_fact(
                &session.id,
                &created.id,
                MemoryFactPatch {
                    category: Some("artifact_path".to_string()),
                    content: None,
                    status: Some(MemoryFactStatus::Stale),
                    pinned: Some(true),
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.category, "artifact_path");
        assert_eq!(updated.status, MemoryFactStatus::Stale);
        assert!(updated.pinned);

        let affected = storage
            .rename_memory_paths(&session.id, "C:\\work\\old", "C:\\work\\new")
            .await
            .unwrap();
        assert_eq!(affected, 1);

        let listed = storage.list_memory_facts(&session.id).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|fact| {
            fact.status == MemoryFactStatus::Superseded
                && fact.content.contains("C:\\work\\old\\result.txt")
        }));
        assert!(listed.iter().any(|fact| {
            fact.status == MemoryFactStatus::Active
                && fact.content.contains("C:\\work\\new\\result.txt")
        }));
    }

    #[tokio::test]
    async fn test_replace_cfpm_memory_facts_and_rollback_snapshot() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_memory_cfpm.db");
        let storage = Arc::new(SessionStorage::create(&db_path).await.unwrap());
        let session = storage
            .create_session(
                PathBuf::from("/tmp/test"),
                "memory".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let manual = storage
            .create_memory_fact(
                &session.id,
                MemoryFactDraft::new("goal", "keep backend-first architecture", "user"),
            )
            .await
            .unwrap();

        storage
            .replace_cfpm_memory_facts(
                &session.id,
                vec![
                    MemoryFactDraft::new(
                        "verified_action",
                        "Executed command successfully: rg --files",
                        MEMORY_SOURCE_CFPM_AUTO,
                    ),
                    MemoryFactDraft::new(
                        "artifact",
                        "E:\\yw\\agiatme\\goose\\agime.exe",
                        MEMORY_SOURCE_CFPM_AUTO,
                    ),
                ],
                "auto_compaction",
            )
            .await
            .unwrap();

        let mut listed = storage.list_memory_facts(&session.id).await.unwrap();
        listed.sort_by(|a, b| a.category.cmp(&b.category));
        assert_eq!(listed.len(), 3);
        assert!(listed.iter().any(|f| f.id == manual.id));
        assert_eq!(
            listed
                .iter()
                .filter(|f| f.source == MEMORY_SOURCE_CFPM_AUTO)
                .count(),
            2
        );

        let auto_fact = listed
            .iter()
            .find(|f| f.source == MEMORY_SOURCE_CFPM_AUTO)
            .unwrap()
            .clone();
        let promoted = storage
            .update_memory_fact(
                &session.id,
                &auto_fact.id,
                MemoryFactPatch {
                    category: None,
                    content: Some("Executed command successfully: rg -n memory".to_string()),
                    status: None,
                    pinned: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(promoted.source, MEMORY_SOURCE_USER);

        storage
            .replace_cfpm_memory_facts(
                &session.id,
                vec![MemoryFactDraft::new(
                    "open_item",
                    "Implement memory panel UI",
                    MEMORY_SOURCE_CFPM_AUTO,
                )],
                "auto_compaction_2",
            )
            .await
            .unwrap();

        let listed_after_second = storage.list_memory_facts(&session.id).await.unwrap();
        assert!(listed_after_second.iter().any(|f| f.id == manual.id));
        assert!(listed_after_second.iter().any(|f| f.id == promoted.id));
        assert_eq!(
            listed_after_second
                .iter()
                .filter(|f| f.source == MEMORY_SOURCE_CFPM_AUTO)
                .count(),
            1
        );

        let snapshots = storage.list_memory_snapshots(&session.id).await.unwrap();
        assert!(!snapshots.is_empty());
        let snapshot_to_restore = snapshots.last().unwrap().id;
        let restored = storage
            .rollback_memory_snapshot(&session.id, snapshot_to_restore)
            .await
            .unwrap();
        assert!(restored >= 1);
    }

    #[test]
    fn test_parse_cfpm_memory_fact_drafts_ignores_date_only_artifacts() {
        let memory_text = r#"
[CFPM_MEMORY_V1]

Important artifacts/paths:
- 2024/7/19
- E:\yw\agiatme\goose\output\result.txt
"#;

        let drafts = parse_cfpm_memory_fact_drafts(memory_text);
        assert!(drafts
            .iter()
            .any(|draft| draft.category == "artifact" && draft.content.contains("result.txt")));
        assert!(!drafts
            .iter()
            .any(|draft| draft.category == "artifact" && draft.content == "2024/7/19"));
    }

    #[tokio::test]
    async fn test_runtime_memory_extraction_and_merge() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_memory_runtime.db");
        let storage = Arc::new(SessionStorage::create(&db_path).await.unwrap());
        let session = storage
            .create_session(
                PathBuf::from("/tmp/test"),
                "runtime-memory".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let turn_messages = vec![
            Message::user().with_text("我们需要保留输出文件路径"),
            Message::assistant().with_text("默认桌面路径不存在，尝试 C:\\Users\\jsjm\\Desktop"),
            Message::assistant()
                .with_text("已完成本轮处理，结果保存到 E:\\yw\\agiatme\\goose\\output\\result.txt"),
            Message::assistant().with_text("归档时间 2026/1/3"),
        ];
        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(!drafts.is_empty());
        assert!(drafts
            .iter()
            .any(|draft| draft.category == "artifact" && draft.content.contains("result.txt")));
        assert!(!drafts
            .iter()
            .any(|draft| draft.category == "artifact" && draft.content == "2026/1/3"));
        assert!(!drafts.iter().any(|draft| draft.category == "artifact"
            && draft.content.contains("C:\\Users\\jsjm\\Desktop")));

        storage
            .merge_cfpm_memory_facts(&session.id, drafts, "turn_checkpoint")
            .await
            .unwrap();

        let listed = storage.list_memory_facts(&session.id).await.unwrap();
        assert!(listed
            .iter()
            .any(|fact| fact.source == MEMORY_SOURCE_CFPM_AUTO));
        assert!(listed
            .iter()
            .any(|fact| fact.category == "artifact" && fact.content.contains("result.txt")));
        assert!(listed.iter().any(|fact| {
            fact.category == "invalid_path" && fact.content.contains("C:\\Users\\jsjm\\Desktop")
        }));
        assert!(!listed.iter().any(|fact| fact.content == "2026/1/3"));
        assert!(!listed
            .iter()
            .any(|fact| fact.category == "artifact"
                && fact.content.contains("C:\\Users\\jsjm\\Desktop")));
    }

    #[test]
    fn test_runtime_memory_extraction_accepts_explicit_path_line() {
        let turn_messages = vec![
            Message::assistant().with_text("C:\\Users\\jsjm\\OneDrive\\Desktop"),
            Message::assistant().with_text("下一步继续处理。"),
        ];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(drafts.iter().any(|draft| draft.category == "artifact"
            && draft.content == "C:\\Users\\jsjm\\OneDrive\\Desktop"));
    }

    #[test]
    fn test_runtime_memory_extraction_rejects_symbolic_path_candidates() {
        let turn_messages = vec![
            Message::assistant().with_text("使用 $env:USERPROFILE/Desktop 再次检查"),
            Message::assistant().with_text("确认路径变量可用"),
        ];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(!drafts.iter().any(|draft| {
            draft.category == "artifact"
                && draft
                    .content
                    .eq_ignore_ascii_case("$env:USERPROFILE/Desktop")
        }));
    }

    #[test]
    fn test_runtime_memory_extraction_normalizes_trailing_punctuation() {
        let turn_messages = vec![Message::assistant()
            .with_text("结果已保存到 \"C:\\Users\\jsjm\\OneDrive\\Desktop\\result.txt\".")];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(drafts.iter().any(|draft| {
            draft.category == "artifact"
                && draft.content == "C:\\Users\\jsjm\\OneDrive\\Desktop\\result.txt"
        }));
    }

    #[test]
    fn test_runtime_memory_extraction_ignores_failed_path_context_lines() {
        let turn_messages = vec![
            Message::assistant()
                .with_text("系统显示桌面路径是 C:\\Users\\jsjm\\Desktop，但访问不了。"),
            Message::assistant()
                .with_text("实际输出文件位于 C:\\Users\\jsjm\\OneDrive\\Desktop\\result.txt。"),
        ];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(!drafts.iter().any(|draft| {
            draft.category == "artifact" && draft.content == "C:\\Users\\jsjm\\Desktop"
        }));
        assert!(drafts.iter().any(|draft| {
            draft.category == "artifact"
                && draft.content == "C:\\Users\\jsjm\\OneDrive\\Desktop\\result.txt"
        }));
    }

    #[test]
    fn test_runtime_memory_extraction_ignores_private_note_and_failed_tool_lines() {
        let tool_output = "private note: output was 103 lines and we are only showing the most recent lines, remainder of lines in C:\\Users\\jsjm\\AppData\\Local\\Temp\\.tmpD50IIq do not show tmp file to user, that file can be searched if extra context needed to fulfill request. truncated output:\nGet-ChildItem : Cannot find path 'C:\\Users\\jsjm\\Desktop' because it does not exist.\nC:\\Users\\jsjm\\OneDrive\\Desktop";
        let turn_messages = vec![Message::user().with_tool_response(
            "tool_1",
            Ok(rmcp::model::CallToolResult {
                content: vec![RawContent::text(tool_output).no_annotation()],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        )];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(drafts.iter().any(|draft| {
            draft.category == "artifact" && draft.content == "C:\\Users\\jsjm\\OneDrive\\Desktop"
        }));
        assert!(!drafts
            .iter()
            .any(|draft| { draft.category == "artifact" && draft.content.contains(".tmpD50IIq") }));
        assert!(!drafts.iter().any(|draft| {
            draft.category == "artifact" && draft.content == "C:\\Users\\jsjm\\Desktop"
        }));
    }

    #[test]
    fn test_runtime_memory_extraction_ignores_temp_artifact_paths() {
        let turn_messages = vec![
            Message::assistant().with_text("C:\\Users\\jsjm\\AppData\\Local\\Temp\\.tmpD50IIq")
        ];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(!drafts.iter().any(|draft| {
            draft.category == "artifact"
                && draft
                    .content
                    .contains("C:\\Users\\jsjm\\AppData\\Local\\Temp")
        }));
    }

    #[test]
    fn test_runtime_memory_extraction_ignores_date_like_slash_tokens() {
        let turn_messages =
            vec![Message::assistant().with_text("- `goose` - 最近更新（2026/2/8）")];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(!drafts
            .iter()
            .any(|draft| draft.category == "artifact" && draft.content.contains("2026/2/8")));
    }

    #[test]
    fn test_runtime_memory_extraction_rejects_skill_catalog_verified_lines() {
        let turn_messages = vec![
            Message::assistant().with_text("| `canvas-design` | 创建视觉艺术（PNG/PDF） |"),
            Message::assistant().with_text("- `skill-creator` - 创建新技能的指南"),
        ];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(!drafts
            .iter()
            .any(|draft| draft.category == "verified_action"));
    }

    #[test]
    fn test_runtime_memory_extraction_rejects_todo_heading_noise() {
        let turn_messages =
            vec![Message::assistant().with_text("### 8. **TODO（任务管理）**\n- TODO 任务管理")];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(!drafts.iter().any(|draft| draft.category == "open_item"));
    }

    #[test]
    fn test_runtime_memory_extraction_ignores_unverified_assistant_path_guess() {
        let turn_messages = vec![Message::assistant()
            .with_text("好的！我已经知道你的桌面路径是 C:\\Users\\jsjm\\Desktop，让我查看一下：")];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(!drafts
            .iter()
            .any(|draft| draft.category == "artifact" && draft.content.contains("Desktop")));
    }

    #[test]
    fn test_runtime_memory_extraction_accepts_repeated_single_path_output_line() {
        let tool_output =
            "C:\\Users\\jsjm\\OneDrive\\Desktop\\result.txt C:\\Users\\jsjm\\OneDrive\\Desktop\\result.txt";
        let turn_messages = vec![Message::user().with_tool_response(
            "tool_1",
            Ok(rmcp::model::CallToolResult {
                content: vec![RawContent::text(tool_output).no_annotation()],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        )];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        let candidates = extract_candidate_paths_from_text(tool_output);
        assert!(
            !candidates.is_empty(),
            "expected path candidates from repeated output, got none"
        );
        assert!(
            drafts.iter().any(|draft| {
                draft.category == "artifact"
                    && draft.content == "C:\\Users\\jsjm\\OneDrive\\Desktop\\result.txt"
            }),
            "drafts: {:?}",
            drafts
        );
    }

    #[test]
    fn test_runtime_memory_extraction_rejects_missing_known_folder_path() {
        let root = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("target_cfpm_test")
            .join("cfpm_missing_known_folder");
        let missing_desktop = root.join("desktop");
        let _ = fs::remove_dir_all(&root);
        let missing_text = missing_desktop.to_string_lossy().to_string();
        let turn_messages = vec![Message::assistant().with_text(&missing_text)];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(!drafts
            .iter()
            .any(|draft| draft.category == "artifact" && draft.content == missing_text));
    }

    #[test]
    fn test_runtime_memory_extraction_records_invalid_path_from_failure_line() {
        let turn_messages = vec![
            Message::assistant().with_text(
                "Get-ChildItem : Cannot find path 'C:\\Users\\jsjm\\Desktop' because it does not exist.",
            ),
            Message::assistant()
                .with_text("实际输出文件位于 C:\\Users\\jsjm\\OneDrive\\Desktop\\result.txt。"),
        ];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(drafts.iter().any(|draft| {
            draft.category == "invalid_path" && draft.content == "C:\\Users\\jsjm\\Desktop"
        }));
        assert!(drafts.iter().any(|draft| {
            draft.category == "artifact"
                && draft.content == "C:\\Users\\jsjm\\OneDrive\\Desktop\\result.txt"
        }));
    }

    #[test]
    fn test_runtime_memory_extraction_records_invalid_path_from_system_path_specified_text() {
        let turn_messages = vec![Message::assistant()
            .with_text("The system cannot find the path specified: C:\\Users\\jsjm\\Desktop.")];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(drafts.iter().any(|draft| {
            draft.category == "invalid_path" && draft.content == "C:\\Users\\jsjm\\Desktop"
        }));
    }

    #[test]
    fn test_evaluate_cfpm_auto_candidate_rejects_artifact_with_trailing_sentence() {
        let result = evaluate_cfpm_auto_candidate(
            "artifact",
            "C:\\Users\\jsjm\\OneDrive\\Desktop`，包含以下文件",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_evaluate_cfpm_auto_candidate_rejects_verified_skill_catalog_line() {
        let result = evaluate_cfpm_auto_candidate(
            "verified_action",
            "| `canvas-design` | 创建视觉艺术（PNG/PDF） |",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_evaluate_cfpm_auto_candidate_rejects_open_item_heading_noise() {
        let result = evaluate_cfpm_auto_candidate("open_item", "### 8. **TODO（任务管理）**");
        assert!(result.is_err());
    }

    #[test]
    fn test_runtime_memory_extraction_attaches_validation_command_from_tool_request() {
        let tool_call = rmcp::model::CallToolRequestParam {
            name: "developer__shell_command".into(),
            arguments: Some(
                serde_json::json!({
                    "command": "Get-ChildItem \"$env:USERPROFILE/Desktop\""
                })
                .as_object()
                .expect("tool args object")
                .clone(),
            ),
        };
        let tool_output = "C:\\Users\\jsjm\\OneDrive\\Desktop";
        let turn_messages = vec![
            Message::assistant().with_tool_request("req_1", Ok(tool_call)),
            Message::user().with_tool_response(
                "req_1",
                Ok(rmcp::model::CallToolResult {
                    content: vec![RawContent::text(tool_output).no_annotation()],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
        ];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        let artifact = drafts
            .iter()
            .find(|draft| draft.category == "artifact" && draft.content == tool_output)
            .expect("artifact should be extracted");
        assert_eq!(
            artifact.validation_command.as_deref(),
            Some("Get-ChildItem \"$env:USERPROFILE/Desktop\"")
        );
    }

    #[test]
    fn test_runtime_memory_extraction_records_invalid_path_from_command_hint_on_error_response() {
        let tool_call = rmcp::model::CallToolRequestParam {
            name: "developer__shell_command".into(),
            arguments: Some(
                serde_json::json!({
                    "command": "Get-ChildItem 'C:\\Users\\jsjm\\Desktop'"
                })
                .as_object()
                .expect("tool args object")
                .clone(),
            ),
        };
        let turn_messages = vec![
            Message::assistant().with_tool_request("req_2", Ok(tool_call)),
            Message::user().with_tool_response(
                "req_2",
                Err(rmcp::model::ErrorData {
                    code: rmcp::model::ErrorCode::INTERNAL_ERROR,
                    message: std::borrow::Cow::from(
                        "Cannot find path because it does not exist.".to_string(),
                    ),
                    data: None,
                }),
            ),
        ];

        let drafts = extract_runtime_cfpm_memory_drafts(&turn_messages);
        assert!(drafts.iter().any(|draft| {
            draft.category == "invalid_path" && draft.content == "C:\\Users\\jsjm\\Desktop"
        }));
    }

    #[tokio::test]
    async fn test_merge_cfpm_memory_records_candidate_decisions() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_memory_candidates.db");
        let storage = Arc::new(SessionStorage::create(&db_path).await.unwrap());
        let session = storage
            .create_session(
                PathBuf::from("/tmp/test"),
                "candidate-memory".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let report = storage
            .merge_cfpm_memory_facts(
                &session.id,
                vec![
                    MemoryFactDraft::new("artifact", "2024/7/19", MEMORY_SOURCE_CFPM_AUTO),
                    MemoryFactDraft::new(
                        "verified_action",
                        "[stdout] running Get-ChildItem",
                        MEMORY_SOURCE_CFPM_AUTO,
                    ),
                    MemoryFactDraft::new(
                        "artifact",
                        "C:\\Users\\jsjm\\OneDrive\\Desktop",
                        MEMORY_SOURCE_CFPM_AUTO,
                    ),
                ],
                "candidate_gate_test",
            )
            .await
            .unwrap();
        assert_eq!(report.mode, "merge");
        assert!(report.accepted_count >= 1);
        assert!(report.rejected_count >= 2);
        assert!(!report.rejected_reason_breakdown.is_empty());

        let accepted_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM memory_candidates WHERE session_id = ? AND decision = 'accepted'",
        )
        .bind(&session.id)
        .fetch_one(&storage.pool)
        .await
        .unwrap_or(0);
        let rejected_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM memory_candidates WHERE session_id = ? AND decision = 'rejected'",
        )
        .bind(&session.id)
        .fetch_one(&storage.pool)
        .await
        .unwrap_or(0);

        assert!(accepted_count >= 1);
        assert!(rejected_count >= 2);

        let listed = storage.list_memory_facts(&session.id).await.unwrap();
        assert!(listed
            .iter()
            .any(|fact| fact.content.contains("C:\\Users\\jsjm\\OneDrive\\Desktop")));
        assert!(!listed.iter().any(|fact| fact.content == "2024/7/19"));
        assert!(!listed
            .iter()
            .any(|fact| fact.content.contains("[stdout] running")));
    }

    #[tokio::test]
    async fn test_merge_cfpm_memory_accumulates_evidence_for_existing_fact() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_memory_evidence_accumulate.db");
        let storage = Arc::new(SessionStorage::create(&db_path).await.unwrap());
        let session = storage
            .create_session(
                PathBuf::from("/tmp/test"),
                "candidate-evidence".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let draft = MemoryFactDraft::new(
            "artifact",
            "C:\\Users\\jsjm\\OneDrive\\Desktop",
            MEMORY_SOURCE_CFPM_AUTO,
        );
        storage
            .merge_cfpm_memory_facts(&session.id, vec![draft.clone()], "evidence_round_1")
            .await
            .unwrap();
        let first = storage
            .list_memory_facts(&session.id)
            .await
            .unwrap()
            .into_iter()
            .find(|fact| fact.category == "artifact")
            .expect("expected artifact fact after first merge");

        storage
            .merge_cfpm_memory_facts(&session.id, vec![draft], "evidence_round_2")
            .await
            .unwrap();
        let second = storage
            .list_memory_facts(&session.id)
            .await
            .unwrap()
            .into_iter()
            .find(|fact| fact.category == "artifact")
            .expect("expected artifact fact after second merge");

        assert!(second.evidence_count > first.evidence_count);
        assert!(second.confidence >= first.confidence);
    }

    #[tokio::test]
    async fn test_merge_cfpm_memory_accepts_invalid_path_facts() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_memory_invalid_path.db");
        let storage = Arc::new(SessionStorage::create(&db_path).await.unwrap());
        let session = storage
            .create_session(
                PathBuf::from("/tmp/test"),
                "candidate-invalid".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        storage
            .merge_cfpm_memory_facts(
                &session.id,
                vec![MemoryFactDraft::new(
                    "invalid_path",
                    "C:\\Users\\jsjm\\Desktop",
                    MEMORY_SOURCE_CFPM_AUTO,
                )],
                "invalid_path_round_1",
            )
            .await
            .unwrap();

        let listed = storage.list_memory_facts(&session.id).await.unwrap();
        assert!(listed
            .iter()
            .any(|fact| fact.category == "invalid_path" && fact.confidence >= 0.85));
    }

    #[tokio::test]
    async fn test_merge_cfpm_memory_rejects_artifact_when_same_path_is_invalid() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_memory_invalid_conflict.db");
        let storage = Arc::new(SessionStorage::create(&db_path).await.unwrap());
        let session = storage
            .create_session(
                PathBuf::from("/tmp/test"),
                "candidate-invalid-conflict".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let report = storage
            .merge_cfpm_memory_facts(
                &session.id,
                vec![
                    MemoryFactDraft::new(
                        "artifact",
                        "C:\\Users\\jsjm\\Desktop\\probe.txt",
                        MEMORY_SOURCE_CFPM_AUTO,
                    ),
                    MemoryFactDraft::new(
                        "invalid_path",
                        "C:\\Users\\jsjm\\Desktop\\probe.txt",
                        MEMORY_SOURCE_CFPM_AUTO,
                    ),
                ],
                "invalid_conflict_round_1",
            )
            .await
            .unwrap();

        assert_eq!(report.accepted_count, 1);
        assert_eq!(report.rejected_count, 1);
        assert!(report
            .rejected_reason_breakdown
            .iter()
            .any(|reason| reason.starts_with("artifact_marked_invalid=")));

        let listed = storage.list_memory_facts(&session.id).await.unwrap();
        assert!(!listed.iter().any(|fact| {
            fact.category == "artifact" && fact.content == "C:\\Users\\jsjm\\Desktop\\probe.txt"
        }));
        assert!(listed.iter().any(|fact| {
            fact.category == "invalid_path" && fact.content == "C:\\Users\\jsjm\\Desktop\\probe.txt"
        }));
    }

    #[tokio::test]
    async fn test_list_memory_candidates_with_filter() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_memory_candidates_list.db");
        let storage = Arc::new(SessionStorage::create(&db_path).await.unwrap());
        let session = storage
            .create_session(
                PathBuf::from("/tmp/test"),
                "candidate-list".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        storage
            .merge_cfpm_memory_facts(
                &session.id,
                vec![
                    MemoryFactDraft::new("artifact", "2024/7/19", MEMORY_SOURCE_CFPM_AUTO),
                    MemoryFactDraft::new(
                        "artifact",
                        "C:\\Users\\jsjm\\OneDrive\\Desktop",
                        MEMORY_SOURCE_CFPM_AUTO,
                    ),
                ],
                "candidate_list_test",
            )
            .await
            .unwrap();

        let accepted = storage
            .list_memory_candidates(&session.id, Some("accepted"), Some(50))
            .await
            .unwrap();
        let rejected = storage
            .list_memory_candidates(&session.id, Some("rejected"), Some(50))
            .await
            .unwrap();

        assert!(!accepted.is_empty());
        assert!(!rejected.is_empty());
        assert!(accepted.iter().all(|item| item.decision == "accepted"));
        assert!(rejected.iter().all(|item| item.decision == "rejected"));
    }

    #[tokio::test]
    async fn test_list_recent_cfpm_tool_gate_events_from_inline_notifications() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_memory_tool_gate_events.db");
        let storage = Arc::new(SessionStorage::create(&db_path).await.unwrap());
        let session = storage
            .create_session(
                PathBuf::from("/tmp/test"),
                "tool-gate-events".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        storage
            .add_message(
                &session.id,
                &Message::assistant().with_system_notification(
                    SystemNotificationType::InlineMessage,
                    "[CFPM_TOOL_GATE_V1] {\"version\":\"v1\",\"verbosity\":\"brief\",\"action\":\"rewrite_known_folder_probe\",\"tool\":\"developer__shell_command\",\"target\":\"desktop\",\"path\":\"C:\\\\Users\\\\jsjm\\\\OneDrive\\\\Desktop\",\"originalCommand\":\"Get-ChildItem \\\"$env:USERPROFILE/Desktop\\\"\",\"rewrittenCommand\":\"Get-ChildItem \\\"C:\\\\Users\\\\jsjm\\\\OneDrive\\\\Desktop\\\"\"}",
                ),
            )
            .await
            .unwrap();

        storage
            .add_message(
                &session.id,
                &Message::assistant().with_system_notification(
                    SystemNotificationType::InlineMessage,
                    "[CFPM_RUNTIME_V1] {\"version\":\"v1\",\"verbosity\":\"brief\"}",
                ),
            )
            .await
            .unwrap();

        let events = storage
            .list_recent_cfpm_tool_gate_events(&session.id, Some(10))
            .await
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tool, "developer__shell_command");
        assert_eq!(events[0].target, "desktop");
        assert_eq!(events[0].path, "C:\\Users\\jsjm\\OneDrive\\Desktop");
    }

    #[tokio::test]
    async fn test_prune_cfpm_auto_memory_facts_removes_date_noise() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_memory_prune_cfpm.db");
        let storage = Arc::new(SessionStorage::create(&db_path).await.unwrap());
        let session = storage
            .create_session(
                PathBuf::from("/tmp/test"),
                "prune-cfpm-memory".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let valid_path_id = format!("mem_{}", Uuid::new_v4().simple());
        let noisy_date_id = format!("mem_{}", Uuid::new_v4().simple());

        sqlx::query(
            r#"
            INSERT INTO memory_facts (id, session_id, category, content, status, pinned, source)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&valid_path_id)
        .bind(&session.id)
        .bind("artifact")
        .bind("C:\\Users\\jsjm\\OneDrive\\Desktop")
        .bind("active")
        .bind(false)
        .bind(MEMORY_SOURCE_CFPM_AUTO)
        .execute(&storage.pool)
        .await
        .unwrap();

        sqlx::query(
            r#"
            INSERT INTO memory_facts (id, session_id, category, content, status, pinned, source)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&noisy_date_id)
        .bind(&session.id)
        .bind("artifact")
        .bind("2024/7/19")
        .bind("active")
        .bind(false)
        .bind(MEMORY_SOURCE_CFPM_AUTO)
        .execute(&storage.pool)
        .await
        .unwrap();

        let removed = storage
            .prune_cfpm_auto_memory_facts(&session.id, "turn_checkpoint")
            .await
            .unwrap();
        assert_eq!(removed, 1);

        let listed = storage.list_memory_facts(&session.id).await.unwrap();
        assert!(listed
            .iter()
            .any(|fact| fact.content == "C:\\Users\\jsjm\\OneDrive\\Desktop"));
        assert!(!listed.iter().any(|fact| fact.content == "2024/7/19"));
    }

    #[tokio::test]
    async fn test_prune_cfpm_auto_memory_facts_removes_artifact_conflicting_with_invalid_path() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir
            .path()
            .join("test_memory_prune_invalid_conflict.db");
        let storage = Arc::new(SessionStorage::create(&db_path).await.unwrap());
        let session = storage
            .create_session(
                PathBuf::from("/tmp/test"),
                "prune-cfpm-invalid-conflict".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let artifact_id = format!("mem_{}", Uuid::new_v4().simple());
        let invalid_id = format!("mem_{}", Uuid::new_v4().simple());

        sqlx::query(
            r#"
            INSERT INTO memory_facts (id, session_id, category, content, status, pinned, source)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&artifact_id)
        .bind(&session.id)
        .bind("artifact")
        .bind("C:\\Users\\jsjm\\Desktop")
        .bind("active")
        .bind(false)
        .bind(MEMORY_SOURCE_CFPM_AUTO)
        .execute(&storage.pool)
        .await
        .unwrap();

        sqlx::query(
            r#"
            INSERT INTO memory_facts (id, session_id, category, content, status, pinned, source)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&invalid_id)
        .bind(&session.id)
        .bind("invalid_path")
        .bind("C:\\Users\\jsjm\\Desktop")
        .bind("active")
        .bind(false)
        .bind(MEMORY_SOURCE_CFPM_AUTO)
        .execute(&storage.pool)
        .await
        .unwrap();

        let removed = storage
            .prune_cfpm_auto_memory_facts(&session.id, "turn_checkpoint")
            .await
            .unwrap();
        assert_eq!(removed, 1);

        let listed = storage.list_memory_facts(&session.id).await.unwrap();
        assert!(!listed.iter().any(|fact| {
            fact.category == "artifact" && fact.content == "C:\\Users\\jsjm\\Desktop"
        }));
        assert!(listed.iter().any(|fact| {
            fact.category == "invalid_path" && fact.content == "C:\\Users\\jsjm\\Desktop"
        }));
    }

    #[test]
    fn test_evaluate_cfpm_auto_candidate_rejects_non_path_noise_like_ai_agent() {
        let decision = evaluate_cfpm_auto_candidate("artifact", "AI/Agent");
        assert!(decision.is_err());
    }

    #[test]
    fn test_evaluate_cfpm_auto_candidate_rejects_slash_command_tokens() {
        let decision = evaluate_cfpm_auto_candidate("artifact", "/think");
        assert!(decision.is_err());
    }

    #[test]
    fn test_evaluate_cfpm_auto_candidate_accepts_project_relative_source_path() {
        let decision = evaluate_cfpm_auto_candidate("artifact", "src/main.rs");
        assert!(decision.is_ok());
    }

    #[test]
    fn test_extract_runtime_cfpm_memory_drafts_does_not_capture_markdown_sentence_path_noise() {
        let messages = vec![Message::assistant().with_text(
            "成功找到了！你的桌面实际位置是 **`C:\\Users\\jsjm\\OneDrive\\Desktop`**（OneDrive 同步桌面）",
        )];

        let drafts = extract_runtime_cfpm_memory_drafts(&messages);
        assert!(drafts.iter().all(|draft| {
            !(draft.category == "artifact" && draft.content.ends_with("Desktop`"))
        }));
    }
}
