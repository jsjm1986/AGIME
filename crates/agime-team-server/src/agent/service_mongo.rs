//! Agent service layer for business logic (MongoDB version)

use super::mission_mongo::{
    resolve_execution_profile, AttemptRecord, CreateMissionRequest, GoalNode, GoalStatus,
    ListMissionsQuery, MissionArtifactDoc, MissionDoc, MissionEventDoc, MissionListItem,
    MissionStatus, MissionStep, ProgressSignal, RuntimeContract, RuntimeContractVerification,
    StepStatus,
};
use super::normalize_workspace_path;
use super::session_mongo::{
    AgentSessionDoc, CreateSessionRequest, SessionListItem, SessionListQuery, UserSessionListQuery,
};
use super::task_manager::StreamEvent;
use agime::agents::types::RetryConfig;
use agime_team::models::{
    AgentExtensionConfig, AgentSkillConfig, AgentStatus, AgentTask, ApiFormat, BuiltinExtension,
    CreateAgentRequest, CustomExtensionConfig, ListAgentsQuery, ListTasksQuery, PaginatedResponse,
    SubmitTaskRequest, TaskResult, TaskResultType, TaskStatus, TaskType, TeamAgent,
    UpdateAgentRequest,
};
use agime_team::MongoDb;
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};
use serde::{Deserialize, Serialize};
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
    InvalidName,
    #[error("Invalid API URL format")]
    InvalidApiUrl,
    #[error("Model name must be 1-100 characters")]
    InvalidModel,
    #[error("Priority must be between 0 and 100")]
    InvalidPriority,
    #[error("Invalid extension config: missing uri_or_cmd (or legacy uriOrCmd/command)")]
    InvalidExtensionConfig,
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

/// Validate agent name
fn validate_name(name: &str) -> Result<(), ValidationError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 100 {
        return Err(ValidationError::InvalidName);
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
            return Err(ValidationError::InvalidApiUrl);
        }
    }
    Ok(())
}

/// Validate model name
fn validate_model(model: &Option<String>) -> Result<(), ValidationError> {
    if let Some(ref m) = model {
        let trimmed = m.trim();
        if trimmed.len() > 100 {
            return Err(ValidationError::InvalidModel);
        }
    }
    Ok(())
}

/// Validate priority
fn validate_priority(priority: i32) -> Result<(), ValidationError> {
    if priority < 0 || priority > 100 {
        return Err(ValidationError::InvalidPriority);
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn custom_extension_bson_keeps_envs_and_type_field() {
        let mut envs = HashMap::new();
        envs.insert("API_TOKEN".to_string(), "secret-token".to_string());

        let ext = CustomExtensionConfig {
            name: "demo_mcp".to_string(),
            ext_type: "stdio".to_string(),
            uri_or_cmd: "demo-cmd".to_string(),
            args: vec!["--foo".to_string()],
            envs,
            enabled: true,
            source: Some("team".to_string()),
            source_extension_id: Some("ext-id".to_string()),
        };

        let doc = custom_extension_to_bson_document(&ext);
        let env_doc = doc.get_document("envs").expect("envs must exist");
        assert_eq!(doc.get_str("type").unwrap_or_default(), "stdio");
        assert_eq!(
            env_doc.get_str("API_TOKEN").unwrap_or_default(),
            "secret-token"
        );
    }
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
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_tasks: u32,
    /// LLM temperature (0.0 - 1.0)
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub temperature: Option<f32>,
    /// Maximum output tokens per LLM call
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_tokens: Option<i32>,
    /// Context window limit override
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_limit: Option<usize>,
    /// Skills assigned from team shared skills
    #[serde(default)]
    pub assigned_skills: Vec<AgentSkillConfig>,
    /// Auto-approve chat tasks (skip manual approval for chat messages)
    #[serde(default = "default_auto_approve_chat")]
    pub auto_approve_chat: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

fn default_max_concurrent() -> u32 {
    1
}

fn default_auto_approve_chat() -> bool {
    true
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
            enabled_extensions: doc.enabled_extensions,
            custom_extensions: doc.custom_extensions,
            status: doc.status.parse().unwrap_or(AgentStatus::Idle),
            last_error: doc.last_error,
            allowed_groups: doc.allowed_groups,
            max_concurrent_tasks: doc.max_concurrent_tasks,
            temperature: doc.temperature,
            max_tokens: doc.max_tokens,
            context_limit: doc.context_limit,
            assigned_skills: doc.assigned_skills,
            auto_approve_chat: doc.auto_approve_chat,
            created_at: doc.created_at,
            updated_at: doc.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTaskDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub task_id: String,
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
            "mission" | "portal" | "portal_coding" | "system" | "chat" => v,
            _ => {
                if portal_restricted {
                    "portal".to_string()
                } else {
                    "chat".to_string()
                }
            }
        }
    }

    /// M12: Ensure MongoDB indexes for agent_sessions collection (chat track)
    pub async fn ensure_chat_indexes(&self) {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        let indexes = vec![
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

        if let Err(e) = self.sessions().create_indexes(indexes, None).await {
            tracing::warn!("Failed to create chat session indexes: {}", e);
        } else {
            tracing::info!("Chat session indexes ensured");
        }
    }

    fn agents(&self) -> mongodb::Collection<TeamAgentDoc> {
        self.db.collection("team_agents")
    }

    fn tasks(&self) -> mongodb::Collection<AgentTaskDoc> {
        self.db.collection("agent_tasks")
    }

    fn results(&self) -> mongodb::Collection<TaskResultDoc> {
        self.db.collection("agent_task_results")
    }

    fn sessions(&self) -> mongodb::Collection<AgentSessionDoc> {
        self.db.collection("agent_sessions")
    }

    fn teams(&self) -> mongodb::Collection<Document> {
        self.db.collection("teams")
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
            enabled_extensions: req.enabled_extensions.unwrap_or_else(|| {
                BuiltinExtension::defaults()
                    .into_iter()
                    .map(|ext| AgentExtensionConfig {
                        extension: ext,
                        enabled: true,
                    })
                    .collect()
            }),
            custom_extensions: req.custom_extensions.unwrap_or_default(),
            allowed_groups: req.allowed_groups.unwrap_or_default(),
            max_concurrent_tasks: req.max_concurrent_tasks.unwrap_or(1),
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            context_limit: req.context_limit,
            assigned_skills: req.assigned_skills.unwrap_or_default(),
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
        Ok(doc.map(|d| d.into()))
    }

    /// Get agent with API key preserved (for internal server-side use only, never expose to API).
    pub async fn get_agent_with_key(
        &self,
        id: &str,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let doc = self
            .agents()
            .find_one(doc! { "agent_id": id }, None)
            .await?;
        Ok(doc.map(agent_doc_with_key))
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
                    "api_key": { "$exists": true, "$nin": [null, ""] }
                },
                options,
            )
            .await?;
        Ok(doc.map(agent_doc_with_key))
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
        let items: Vec<TeamAgent> = docs.into_iter().map(|d| d.into()).collect();

        Ok(PaginatedResponse::new(
            items,
            total,
            query.page,
            query.limit,
        ))
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
            let ext_bson = mongodb::bson::to_bson(extensions).unwrap_or(bson::Bson::Array(vec![]));
            set_doc.insert("enabled_extensions", ext_bson);
        }
        if let Some(ref custom_ext) = req.custom_extensions {
            let ext_bson = custom_extensions_to_bson(custom_ext);
            set_doc.insert("custom_extensions", ext_bson);
        }
        if let Some(ref allowed_groups) = req.allowed_groups {
            let bson_val =
                mongodb::bson::to_bson(allowed_groups).unwrap_or(bson::Bson::Array(vec![]));
            set_doc.insert("allowed_groups", bson_val);
        }
        if let Some(max_concurrent) = req.max_concurrent_tasks {
            set_doc.insert("max_concurrent_tasks", max_concurrent as i32);
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
        if let Some(ref assigned_skills) = req.assigned_skills {
            let skills_bson =
                mongodb::bson::to_bson(assigned_skills).unwrap_or(bson::Bson::Array(vec![]));
            set_doc.insert("assigned_skills", skills_bson);
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

        let doc = AgentTaskDoc {
            id: None,
            task_id: id.clone(),
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

        if result.modified_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
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
                    "status": { "$in": ["pending", "approved", "running"] }
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
                    "status": { "$in": ["running", "approved"] }
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

    /// Add a team shared extension to an agent's custom_extensions.
    /// Converts the SharedExtension from the extensions collection into a CustomExtensionConfig
    /// and appends it to the agent's custom_extensions array (with deduplication by name).
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

        // 3. Check for duplicate name
        if agent.custom_extensions.iter().any(|e| e.name == ext.name) {
            return Err(ServiceError::Internal(format!(
                "Extension '{}' already exists in this agent",
                ext.name
            )));
        }

        // 4. Convert SharedExtension -> CustomExtensionConfig
        let uri_or_cmd = ext
            .config
            .get_str("uri_or_cmd")
            .or_else(|_| ext.config.get_str("uriOrCmd"))
            .or_else(|_| ext.config.get_str("command"))
            .unwrap_or_default()
            .to_string();

        if uri_or_cmd.trim().is_empty() {
            return Err(ServiceError::Validation(
                ValidationError::InvalidExtensionConfig,
            ));
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

        let new_ext = CustomExtensionConfig {
            name: ext.name,
            ext_type: ext.extension_type,
            uri_or_cmd,
            args,
            envs,
            enabled: true,
            source: Some("team".to_string()),
            source_extension_id: Some(ext_id_hex),
        };

        // 5. Append to custom_extensions via $push
        let ext_bson = Bson::Document(custom_extension_to_bson_document(&new_ext));

        let now = Utc::now();
        self.agents()
            .update_one(
                doc! { "agent_id": agent_id },
                doc! {
                    "$push": { "custom_extensions": ext_bson },
                    "$set": { "updated_at": bson::DateTime::from_chrono(now) }
                },
                None,
            )
            .await?;

        Ok(self.get_agent(agent_id).await?)
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
            .list(team_id, Some(1), Some(200), None, None)
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
        let session_id = Uuid::new_v4().to_string();
        let now = bson::DateTime::now();
        let session_source =
            Self::normalize_session_source(req.session_source.clone(), req.portal_restricted);
        let hidden_from_chat_list = req.hidden_from_chat_list.unwrap_or_else(|| {
            req.portal_restricted
                || session_source == "mission"
                || session_source == "system"
                || session_source == "portal_coding"
        });

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
            compaction_count: 0,
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
            attached_document_ids: req.attached_document_ids,
            workspace_path: None,
            extra_instructions: req.extra_instructions,
            allowed_extensions: req.allowed_extensions,
            allowed_skill_ids: req.allowed_skill_ids,
            retry_config: req.retry_config,
            max_turns: req.max_turns,
            tool_timeout_seconds: req.tool_timeout_seconds,
            max_portal_retry_rounds: req.max_portal_retry_rounds,
            require_final_report: req.require_final_report,
            portal_restricted: req.portal_restricted,
            document_access_mode: req.document_access_mode,
            portal_id: None,
            portal_slug: None,
            visitor_id: None,
            session_source,
            source_mission_id: req.source_mission_id,
            hidden_from_chat_list,
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

        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set },
                None,
            )
            .await?;
        Ok(())
    }

    /// Increment compaction count.
    pub async fn increment_compaction_count(
        &self,
        session_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$inc": { "compaction_count": 1 },
                    "$set": {
                        "updated_at": now,
                    }
                },
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

    // ========== Chat Track Methods (Phase 1) ==========

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

        // 2) Mission-bound sessions from mission docs -> mission source + hidden + source_mission_id.
        let mission_opts = mongodb::options::FindOptions::builder()
            .projection(doc! { "mission_id": 1, "session_id": 1, "status": 1 })
            .build();
        // Use raw bson documents here to tolerate legacy mission rows that miss
        // strongly-typed fields (e.g. team_id) and keep migration best-effort.
        let missions_raw = self.db.collection::<bson::Document>("agent_missions");
        let mut mission_cursor = missions_raw.find(doc! {}, mission_opts).await?;
        while let Some(m) = mission_cursor.try_next().await? {
            let mission_id = m.get_str("mission_id").ok().map(|s| s.to_string());
            let session_id = m.get_str("session_id").ok().map(|s| s.to_string());
            if let (Some(mission_id), Some(session_id)) = (mission_id, session_id) {
                let mission_status = m.get_str("status").ok().unwrap_or_default();
                let mut set_doc = doc! {
                    "session_source": "mission",
                    "source_mission_id": mission_id,
                    "hidden_from_chat_list": true,
                };
                if matches!(mission_status, "completed" | "cancelled") {
                    set_doc.insert("status", "archived");
                }
                let _ = self
                    .sessions()
                    .update_one(
                        doc! { "session_id": &session_id },
                        doc! { "$set": set_doc },
                        None,
                    )
                    .await?;
            }
        }

        // 3) Remaining sessions with missing source -> default chat + visible.
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
        session_source: Option<String>,
        source_mission_id: Option<String>,
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
            session_source,
            source_mission_id,
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

        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set },
                None,
            )
            .await?;
        Ok(())
    }

    // ═══════════════════════════════════════════════════════
    // Mission Track (Phase 2) - Collection accessors
    // ═══════════════════════════════════════════════════════

    fn missions(&self) -> mongodb::Collection<MissionDoc> {
        self.db.collection("agent_missions")
    }

    fn artifacts(&self) -> mongodb::Collection<MissionArtifactDoc> {
        self.db.collection("agent_mission_artifacts")
    }

    fn mission_events(&self) -> mongodb::Collection<MissionEventDoc> {
        self.db.collection("agent_mission_events")
    }

    // ─── Mission CRUD ────────────────────────────────────

    pub async fn create_mission(
        &self,
        req: &CreateMissionRequest,
        team_id: &str,
        creator_id: &str,
    ) -> Result<MissionDoc, mongodb::error::Error> {
        let now = bson::DateTime::now();
        let step_timeout_seconds = req
            .step_timeout_seconds
            .filter(|v| *v > 0)
            .map(|v| v.min(7200));
        let step_max_retries = req.step_max_retries.filter(|v| *v > 0).map(|v| v.min(8));
        let mission = MissionDoc {
            id: None,
            mission_id: Uuid::new_v4().to_string(),
            team_id: team_id.to_string(),
            agent_id: req.agent_id.clone(),
            creator_id: creator_id.to_string(),
            goal: req.goal.clone(),
            context: req.context.clone(),
            status: MissionStatus::Draft,
            approval_policy: req.approval_policy.clone().unwrap_or_default(),
            steps: vec![],
            current_step: None,
            session_id: None,
            source_chat_session_id: req.source_chat_session_id.clone(),
            token_budget: req.token_budget.unwrap_or(0),
            total_tokens_used: 0,
            priority: req.priority.unwrap_or(0),
            step_timeout_seconds,
            step_max_retries,
            plan_version: 1,
            execution_mode: req.execution_mode.clone().unwrap_or_default(),
            execution_profile: req.execution_profile.clone().unwrap_or_default(),
            goal_tree: None,
            current_goal_id: None,
            total_pivots: 0,
            total_abandoned: 0,
            error_message: None,
            final_summary: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
            attached_document_ids: req.attached_document_ids.clone(),
            workspace_path: None,
            current_run_id: None,
        };
        self.missions().insert_one(&mission, None).await?;
        Ok(mission)
    }

    pub async fn get_mission(
        &self,
        mission_id: &str,
    ) -> Result<Option<MissionDoc>, mongodb::error::Error> {
        self.missions()
            .find_one(doc! { "mission_id": mission_id }, None)
            .await
    }

    /// Attach documents to a mission
    pub async fn attach_documents_to_mission(
        &self,
        mission_id: &str,
        document_ids: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$addToSet": { "attached_document_ids": { "$each": document_ids } },
                    "$set": { "updated_at": now },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Detach documents from a mission
    pub async fn detach_documents_from_mission(
        &self,
        mission_id: &str,
        document_ids: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$pullAll": { "attached_document_ids": document_ids },
                    "$set": { "updated_at": now },
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn list_missions(
        &self,
        query: ListMissionsQuery,
    ) -> Result<Vec<MissionListItem>, mongodb::error::Error> {
        let mut filter = doc! { "team_id": &query.team_id };
        if let Some(ref aid) = query.agent_id {
            filter.insert("agent_id", aid);
        }
        if let Some(ref s) = query.status {
            filter.insert("status", s);
        }

        let skip = ((query.page.max(1) - 1) * query.limit) as u64;
        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .skip(skip)
            .limit(query.limit as i64)
            .build();

        let cursor = self.missions().find(filter, opts).await?;
        let missions: Vec<MissionDoc> = cursor.try_collect().await?;

        // Batch-fetch agent names to avoid N+1 queries
        let agent_ids: Vec<&str> = missions.iter().map(|m| m.agent_id.as_str()).collect();
        let agent_names = self.batch_get_agent_names(&agent_ids).await;

        let mut items = Vec::with_capacity(missions.len());
        for m in missions {
            let agent_name = agent_names.get(&m.agent_id).cloned().unwrap_or_default();

            let completed_steps = m
                .steps
                .iter()
                .filter(|s| s.status == StepStatus::Completed)
                .count();

            let goal_count = m
                .goal_tree
                .as_ref()
                .map(|g| {
                    g.iter()
                        .filter(|n| n.status != GoalStatus::Abandoned)
                        .count()
                })
                .unwrap_or(0);
            let completed_goals = m
                .goal_tree
                .as_ref()
                .map(|g| {
                    g.iter()
                        .filter(|n| n.status == GoalStatus::Completed)
                        .count()
                })
                .unwrap_or(0);

            let resolved_execution_profile = resolve_execution_profile(&m);
            items.push(MissionListItem {
                mission_id: m.mission_id,
                agent_id: m.agent_id,
                agent_name,
                goal: m.goal,
                status: m.status,
                approval_policy: m.approval_policy,
                step_count: m.steps.len(),
                completed_steps,
                current_step: m.current_step,
                total_tokens_used: m.total_tokens_used,
                created_at: m.created_at.to_chrono().to_rfc3339(),
                updated_at: m.updated_at.to_chrono().to_rfc3339(),
                execution_mode: m.execution_mode.clone(),
                execution_profile: m.execution_profile.clone(),
                resolved_execution_profile,
                goal_count,
                completed_goals,
                pivots: m.total_pivots,
                attached_doc_count: m.attached_document_ids.len(),
            });
        }
        Ok(items)
    }

    pub async fn delete_mission(&self, mission_id: &str) -> Result<bool, mongodb::error::Error> {
        // Only Draft, Cancelled, or Failed missions can be deleted
        let result = self
            .missions()
            .delete_one(
                doc! {
                    "mission_id": mission_id,
                    "status": { "$in": ["draft", "cancelled", "failed"] }
                },
                None,
            )
            .await?;
        Ok(result.deleted_count > 0)
    }

    // ─── Mission Status Management ───────────────────────

    /// Update mission status with atomic precondition to prevent race conditions.
    /// Returns an error when the transition precondition is not satisfied.
    pub async fn update_mission_status(
        &self,
        mission_id: &str,
        status: &MissionStatus,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        let should_set_started_at = if matches!(status, MissionStatus::Running) {
            self.get_mission(mission_id)
                .await?
                .map(|m| m.started_at.is_none())
                .unwrap_or(true)
        } else {
            false
        };
        let status_bson = bson::to_bson(status)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        let mut set = doc! {
            "status": status_bson,
            "updated_at": now,
        };
        // Set timestamps for terminal states
        match status {
            MissionStatus::Running | MissionStatus::Planning => {
                // Clear terminal timestamp when mission re-enters active states.
                set.insert("completed_at", bson::Bson::Null);
                if should_set_started_at {
                    set.insert("started_at", now);
                }
                // Stamp server instance for orphaned mission recovery
                if let Ok(iid) = std::env::var("TEAM_SERVER_INSTANCE_ID") {
                    set.insert("server_instance_id", iid);
                }
            }
            MissionStatus::Completed | MissionStatus::Failed | MissionStatus::Cancelled => {
                set.insert("completed_at", now);
            }
            _ => {}
        }

        // Atomic precondition: only allow valid state transitions
        let allowed_from: Vec<&str> = match status {
            MissionStatus::Planning => vec!["draft", "planned"],
            MissionStatus::Planned => vec!["planning", "paused", "failed"],
            MissionStatus::Running => vec!["draft", "planned", "planning", "paused", "failed"],
            MissionStatus::Paused => vec!["running", "planning"],
            MissionStatus::Completed => vec!["running"],
            MissionStatus::Failed => vec!["running", "planning", "paused", "planned"],
            MissionStatus::Cancelled => vec!["draft", "planned", "running", "paused", "planning"],
            _ => vec![],
        };

        let filter = if allowed_from.is_empty() {
            doc! { "mission_id": mission_id }
        } else {
            let bson_arr: Vec<bson::Bson> = allowed_from
                .iter()
                .map(|s| bson::Bson::String(s.to_string()))
                .collect();
            doc! { "mission_id": mission_id, "status": { "$in": bson_arr } }
        };

        let result = self
            .missions()
            .update_one(filter, doc! { "$set": set }, None)
            .await?;

        if result.modified_count == 0 {
            if let Some(current) = self.get_mission(mission_id).await? {
                if current.status == *status {
                    // Idempotent transition request; treat as success.
                    return Ok(());
                }
                return Err(mongodb::error::Error::custom(format!(
                    "mission status transition rejected: mission_id={}, from={:?}, to={:?}",
                    mission_id, current.status, status
                )));
            }
            tracing::warn!(
                "update_mission_status: no update for mission {} to {:?} (precondition failed)",
                mission_id,
                status
            );
            return Err(mongodb::error::Error::custom(format!(
                "mission not found or transition rejected: mission_id={}, to={:?}",
                mission_id, status
            )));
        }
        Ok(())
    }

    pub async fn set_mission_session(
        &self,
        mission_id: &str,
        session_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "session_id": session_id,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_mission_current_run(
        &self,
        mission_id: &str,
        run_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "current_run_id": run_id,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_mission_workspace(
        &self,
        mission_id: &str,
        path: &str,
    ) -> Result<(), mongodb::error::Error> {
        let normalized = normalize_workspace_path(path);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "workspace_path": normalized,
                    "updated_at": bson::DateTime::now(),
                }},
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
                doc! { "$set": {
                    "workspace_path": normalized,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Bind an existing session to a portal context.
    pub async fn set_session_portal_context(
        &self,
        session_id: &str,
        portal_id: &str,
        portal_slug: &str,
        visitor_id: Option<&str>,
        document_access_mode: Option<&str>,
        portal_restricted: bool,
    ) -> Result<(), mongodb::error::Error> {
        let mut set_doc = doc! {
            "portal_restricted": portal_restricted,
            "portal_id": portal_id,
            "portal_slug": portal_slug,
            "updated_at": bson::DateTime::now(),
        };
        match document_access_mode {
            Some(mode) if !mode.trim().is_empty() => {
                set_doc.insert("document_access_mode", mode.trim());
            }
            _ => {
                set_doc.insert("document_access_mode", bson::Bson::Null);
            }
        }
        match visitor_id {
            Some(v) if !v.trim().is_empty() => {
                set_doc.insert("visitor_id", v.trim());
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

    /// Sync portal runtime policy for an existing session so portal config changes
    /// (documents, allowlists, prompt constraints) take effect immediately.
    pub async fn sync_portal_session_policy(
        &self,
        session_id: &str,
        attached_document_ids: Vec<String>,
        extra_instructions: Option<String>,
        allowed_extensions: Option<Vec<String>>,
        allowed_skill_ids: Option<Vec<String>>,
        retry_config: Option<RetryConfig>,
        require_final_report: bool,
        document_access_mode: Option<String>,
    ) -> Result<(), mongodb::error::Error> {
        let mut set_doc = doc! {
            "attached_document_ids": attached_document_ids,
            "portal_restricted": true,
            "require_final_report": require_final_report,
            "updated_at": bson::DateTime::now(),
        };
        match document_access_mode {
            Some(v) if !v.trim().is_empty() => {
                set_doc.insert("document_access_mode", v);
            }
            _ => {
                set_doc.insert("document_access_mode", bson::Bson::Null);
            }
        }

        match extra_instructions {
            Some(v) if !v.trim().is_empty() => {
                set_doc.insert("extra_instructions", v);
            }
            _ => {
                set_doc.insert("extra_instructions", bson::Bson::Null);
            }
        }
        match allowed_extensions {
            Some(v) => {
                set_doc.insert(
                    "allowed_extensions",
                    mongodb::bson::to_bson(&v).unwrap_or(bson::Bson::Array(vec![])),
                );
            }
            None => {
                set_doc.insert("allowed_extensions", bson::Bson::Null);
            }
        }
        match allowed_skill_ids {
            Some(v) => {
                set_doc.insert(
                    "allowed_skill_ids",
                    mongodb::bson::to_bson(&v).unwrap_or(bson::Bson::Array(vec![])),
                );
            }
            None => {
                set_doc.insert("allowed_skill_ids", bson::Bson::Null);
            }
        }
        match retry_config {
            Some(v) => {
                set_doc.insert(
                    "retry_config",
                    mongodb::bson::to_bson(&v).unwrap_or(bson::Bson::Null),
                );
            }
            None => {
                set_doc.insert("retry_config", bson::Bson::Null);
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

    pub async fn save_mission_plan(
        &self,
        mission_id: &str,
        steps: Vec<MissionStep>,
    ) -> Result<(), mongodb::error::Error> {
        let steps_bson = bson::to_bson(&steps).unwrap_or_default();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "steps": steps_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_mission_error(
        &self,
        mission_id: &str,
        error: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "error_message": error,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn clear_mission_error(&self, mission_id: &str) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "error_message": bson::Bson::Null,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_mission_final_summary(
        &self,
        mission_id: &str,
        summary: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "final_summary": summary,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn add_mission_tokens(
        &self,
        mission_id: &str,
        tokens_used: i32,
    ) -> Result<(), mongodb::error::Error> {
        if tokens_used <= 0 {
            return Ok(());
        }
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$inc": { "total_tokens_used": tokens_used as i64 },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    // ─── Step Management ─────────────────────────────────

    pub async fn update_step_status(
        &self,
        mission_id: &str,
        step_index: u32,
        status: &StepStatus,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.status", step_index);
        let status_bson = bson::to_bson(status)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        let now = bson::DateTime::now();
        let mut set_doc = doc! {
            &field: status_bson,
            "updated_at": now,
        };
        if matches!(status, StepStatus::Running) {
            let started_field = format!("steps.{}.started_at", step_index);
            set_doc.insert(started_field, now);
        }
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn complete_step(
        &self,
        mission_id: &str,
        step_index: u32,
        tokens_used: i32,
    ) -> Result<(), mongodb::error::Error> {
        let status_field = format!("steps.{}.status", step_index);
        let completed_field = format!("steps.{}.completed_at", step_index);
        let tokens_field = format!("steps.{}.tokens_used", step_index);
        let now = bson::DateTime::now();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        &status_field: "completed",
                        &completed_field: now,
                        &tokens_field: tokens_used,
                        "updated_at": now,
                    },
                    "$inc": { "total_tokens_used": tokens_used as i64 },
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn fail_step(
        &self,
        mission_id: &str,
        step_index: u32,
        error: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_field = format!("steps.{}.status", step_index);
        let error_field = format!("steps.{}.error_message", step_index);
        let completed_field = format!("steps.{}.completed_at", step_index);
        let now = bson::DateTime::now();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &status_field: "failed",
                    &error_field: error,
                    &completed_field: now,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Save the structured output summary for a completed step.
    pub async fn set_step_output_summary(
        &self,
        mission_id: &str,
        step_index: u32,
        summary: &str,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.output_summary", step_index);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &field: summary,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Persist step runtime contract extracted from mission_preflight.
    pub async fn set_step_runtime_contract(
        &self,
        mission_id: &str,
        step_index: u32,
        contract: &RuntimeContract,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.runtime_contract", step_index);
        let contract_bson = bson::to_bson(contract)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &field: contract_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Persist step contract verification result.
    pub async fn set_step_contract_verification(
        &self,
        mission_id: &str,
        step_index: u32,
        verification: &RuntimeContractVerification,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.contract_verification", step_index);
        let verify_bson = bson::to_bson(verification)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &field: verify_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_step_tool_calls(
        &self,
        mission_id: &str,
        step_index: u32,
        tool_calls: &[super::mission_mongo::ToolCallRecord],
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.tool_calls", step_index);
        let bson_arr = bson::to_bson(tool_calls).unwrap_or(bson::Bson::Array(vec![]));
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": { &field: bson_arr } },
                None,
            )
            .await?;
        Ok(())
    }

    /// Increment the retry count for a step.
    pub async fn increment_step_retry(
        &self,
        mission_id: &str,
        step_index: u32,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.retry_count", step_index);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$inc": { &field: 1 },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Reset a step to pending so mission resume can retry from this step.
    pub async fn reset_step_for_retry(
        &self,
        mission_id: &str,
        step_index: u32,
    ) -> Result<(), mongodb::error::Error> {
        let status_field = format!("steps.{}.status", step_index);
        let error_field = format!("steps.{}.error_message", step_index);
        let started_field = format!("steps.{}.started_at", step_index);
        let completed_field = format!("steps.{}.completed_at", step_index);
        let summary_field = format!("steps.{}.output_summary", step_index);
        let tool_calls_field = format!("steps.{}.tool_calls", step_index);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        &status_field: "pending",
                        "updated_at": bson::DateTime::now(),
                    },
                    "$unset": {
                        &error_field: "",
                        &started_field: "",
                        &completed_field: "",
                        &summary_field: "",
                        &tool_calls_field: "",
                    },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Replace remaining steps after a re-plan, incrementing plan_version.
    pub async fn replan_remaining_steps(
        &self,
        mission_id: &str,
        all_steps: Vec<MissionStep>,
    ) -> Result<(), mongodb::error::Error> {
        let steps_bson = bson::to_bson(&all_steps)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        "steps": steps_bson,
                        "updated_at": bson::DateTime::now(),
                    },
                    "$inc": { "plan_version": 1 },
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn approve_step(
        &self,
        mission_id: &str,
        step_index: u32,
        approver_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_field = format!("steps.{}.status", step_index);
        let approver_field = format!("steps.{}.approved_by", step_index);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &status_field: "pending",
                    &approver_field: approver_id,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn advance_mission_step(
        &self,
        mission_id: &str,
        next_step: u32,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "current_step": next_step as i32,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    // ─── Artifact Management ─────────────────────────────

    pub async fn save_artifact(
        &self,
        artifact: &MissionArtifactDoc,
    ) -> Result<(), mongodb::error::Error> {
        // Upsert by mission_id + file_path to deduplicate across steps/goals
        if let Some(ref fp) = artifact.file_path {
            let filter = doc! {
                "mission_id": &artifact.mission_id,
                "file_path": fp,
            };
            let mut replacement = artifact.clone();
            replacement.id = None;
            let opts = mongodb::options::ReplaceOptions::builder()
                .upsert(true)
                .build();
            self.artifacts()
                .replace_one(filter, replacement, opts)
                .await?;
        } else {
            self.artifacts().insert_one(artifact, None).await?;
        }
        Ok(())
    }

    pub async fn list_mission_artifacts(
        &self,
        mission_id: &str,
    ) -> Result<Vec<MissionArtifactDoc>, mongodb::error::Error> {
        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "step_index": 1, "created_at": 1 })
            .build();
        let cursor = self
            .artifacts()
            .find(doc! { "mission_id": mission_id }, opts)
            .await?;
        cursor.try_collect().await
    }

    pub async fn get_artifact(
        &self,
        artifact_id: &str,
    ) -> Result<Option<MissionArtifactDoc>, mongodb::error::Error> {
        self.artifacts()
            .find_one(doc! { "artifact_id": artifact_id }, None)
            .await
    }

    pub async fn set_artifact_document_link(
        &self,
        artifact_id: &str,
        document_id: &str,
        document_status: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.artifacts()
            .update_one(
                doc! { "artifact_id": artifact_id },
                doc! {
                    "$set": {
                        "archived_document_id": document_id,
                        "archived_document_status": document_status,
                        "archived_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Persist mission runtime stream events for replay/analysis.
    pub async fn save_mission_stream_events(
        &self,
        items: &[(String, String, u64, StreamEvent)],
    ) -> Result<(), mongodb::error::Error> {
        if items.is_empty() {
            return Ok(());
        }

        let docs: Vec<MissionEventDoc> = items
            .iter()
            .map(|(mission_id, run_id, event_id, event)| MissionEventDoc {
                id: None,
                mission_id: mission_id.clone(),
                run_id: Some(run_id.clone()),
                event_id: (*event_id).try_into().unwrap_or(i64::MAX),
                event_type: event.event_type().to_string(),
                payload: serde_json::to_value(event).unwrap_or_else(|_| serde_json::json!({})),
                created_at: bson::DateTime::now(),
            })
            .collect();

        self.mission_events().insert_many(docs, None).await?;
        Ok(())
    }

    pub async fn list_mission_events(
        &self,
        mission_id: &str,
        run_id: Option<&str>,
        after_event_id: Option<u64>,
        limit: u32,
    ) -> Result<Vec<MissionEventDoc>, mongodb::error::Error> {
        let clamped_limit = limit.clamp(1, 2000);
        let mut filter = doc! { "mission_id": mission_id };
        if let Some(run) = run_id {
            filter.insert("run_id", run);
        }
        if let Some(after) = after_event_id {
            filter.insert("event_id", doc! { "$gt": after as i64 });
        }

        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "event_id": 1, "created_at": 1 })
            .limit(clamped_limit as i64)
            .build();
        let mut events: Vec<MissionEventDoc> = self
            .mission_events()
            .find(filter.clone(), opts.clone())
            .await?
            .try_collect()
            .await?;

        // Handle run switches gracefully:
        // if caller sends a stale `after_event_id` from a previous run,
        // restart from beginning of current run instead of returning empty forever.
        if events.is_empty() {
            if let (Some(run), Some(after)) = (run_id, after_event_id) {
                let max_opts = mongodb::options::FindOneOptions::builder()
                    .sort(doc! { "event_id": -1, "created_at": -1 })
                    .build();
                let max_doc = self
                    .mission_events()
                    .find_one(doc! { "mission_id": mission_id, "run_id": run }, max_opts)
                    .await?;
                if let Some(latest) = max_doc {
                    if latest.event_id < after as i64 {
                        let restart_filter = doc! { "mission_id": mission_id, "run_id": run };
                        events = self
                            .mission_events()
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

    // ─── Goal Tree Management (AGE) ─────────────────────

    /// Save initial goal tree for a mission.
    pub async fn save_goal_tree(
        &self,
        mission_id: &str,
        goals: Vec<GoalNode>,
    ) -> Result<(), mongodb::error::Error> {
        let goals_bson = bson::to_bson(&goals)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree": goals_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Update a goal's status using arrayFilters.
    /// Automatically sets completed_at when status is Completed or Abandoned.
    pub async fn update_goal_status(
        &self,
        mission_id: &str,
        goal_id: &str,
        status: &GoalStatus,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(status)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let now = bson::DateTime::now();
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();

        let mut set_doc = doc! {
            "goal_tree.$[elem].status": status_bson,
            "updated_at": now,
        };

        // Set completed_at for terminal statuses
        if matches!(
            status,
            GoalStatus::Completed | GoalStatus::Abandoned | GoalStatus::Failed
        ) {
            set_doc.insert("goal_tree.$[elem].completed_at", now);
        }

        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": set_doc },
                opts,
            )
            .await?;
        Ok(())
    }

    /// Reset a goal to pending so adaptive resume can retry it.
    pub async fn reset_goal_for_retry(
        &self,
        mission_id: &str,
        goal_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        "goal_tree.$[elem].status": "pending",
                        "updated_at": bson::DateTime::now(),
                    },
                    "$unset": {
                        "goal_tree.$[elem].completed_at": "",
                    },
                },
                opts,
            )
            .await?;
        Ok(())
    }

    /// Append an AttemptRecord to a goal's attempts array.
    pub async fn push_goal_attempt(
        &self,
        mission_id: &str,
        goal_id: &str,
        attempt: &AttemptRecord,
    ) -> Result<(), mongodb::error::Error> {
        let attempt_bson = bson::to_bson(attempt)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$push": { "goal_tree.$[elem].attempts": attempt_bson },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                opts,
            )
            .await?;
        Ok(())
    }

    /// Update the signal of the last attempt for a goal.
    pub async fn update_last_attempt_signal(
        &self,
        mission_id: &str,
        goal_id: &str,
        signal: &ProgressSignal,
    ) -> Result<(), mongodb::error::Error> {
        // Read current goal to find the last attempt index
        let mission = self.get_mission(mission_id).await?;
        if let Some(mission) = mission {
            if let Some(goals) = &mission.goal_tree {
                if let Some(goal) = goals.iter().find(|g| g.goal_id == goal_id) {
                    if !goal.attempts.is_empty() {
                        let last_idx = goal.attempts.len() - 1;
                        let signal_bson = bson::to_bson(signal).map_err(|e| {
                            mongodb::error::Error::custom(format!("BSON error: {}", e))
                        })?;
                        let field = format!("goal_tree.$[elem].attempts.{}.signal", last_idx);
                        let opts = mongodb::options::UpdateOptions::builder()
                            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
                            .build();
                        self.missions()
                            .update_one(
                                doc! { "mission_id": mission_id },
                                doc! { "$set": { &field: signal_bson, "updated_at": bson::DateTime::now() } },
                                opts,
                            )
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Set a goal's output_summary.
    pub async fn set_goal_output_summary(
        &self,
        mission_id: &str,
        goal_id: &str,
        summary: &str,
    ) -> Result<(), mongodb::error::Error> {
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].output_summary": summary,
                    "updated_at": bson::DateTime::now(),
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Persist goal runtime contract extracted from mission_preflight.
    pub async fn set_goal_runtime_contract(
        &self,
        mission_id: &str,
        goal_id: &str,
        contract: &RuntimeContract,
    ) -> Result<(), mongodb::error::Error> {
        let contract_bson = bson::to_bson(contract)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].runtime_contract": contract_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Persist goal contract verification result.
    pub async fn set_goal_contract_verification(
        &self,
        mission_id: &str,
        goal_id: &str,
        verification: &RuntimeContractVerification,
    ) -> Result<(), mongodb::error::Error> {
        let verify_bson = bson::to_bson(verification)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].contract_verification": verify_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Set a goal's pivot_reason.
    pub async fn set_goal_pivot(
        &self,
        mission_id: &str,
        goal_id: &str,
        reason: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(&GoalStatus::Pivoting)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].pivot_reason": reason,
                    "goal_tree.$[elem].status": status_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Update current_goal_id on the mission.
    pub async fn advance_mission_goal(
        &self,
        mission_id: &str,
        goal_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "current_goal_id": goal_id,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Atomically set goal pivot status + increment total_pivots counter.
    pub async fn pivot_goal_atomic(
        &self,
        mission_id: &str,
        goal_id: &str,
        reason: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(&GoalStatus::Pivoting)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let now = bson::DateTime::now();
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        "goal_tree.$[elem].pivot_reason": reason,
                        "goal_tree.$[elem].status": status_bson,
                        "updated_at": now,
                    },
                    "$inc": { "total_pivots": 1 },
                },
                opts,
            )
            .await?;
        Ok(())
    }

    /// Atomically abandon goal + increment total_abandoned counter.
    pub async fn abandon_goal_atomic(
        &self,
        mission_id: &str,
        goal_id: &str,
        reason: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(&GoalStatus::Abandoned)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let now = bson::DateTime::now();
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        "goal_tree.$[elem].status": status_bson,
                        "goal_tree.$[elem].pivot_reason": reason,
                        "goal_tree.$[elem].completed_at": now,
                        "updated_at": now,
                    },
                    "$inc": { "total_abandoned": 1 },
                },
                opts,
            )
            .await?;
        Ok(())
    }

    /// Insert child goals into the goal_tree array.
    pub async fn insert_child_goals(
        &self,
        mission_id: &str,
        new_goals: Vec<GoalNode>,
    ) -> Result<(), mongodb::error::Error> {
        let goals_bson = bson::to_bson(&new_goals)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$push": { "goal_tree": { "$each": goals_bson } },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Mark a goal as Abandoned with reason and timestamp.
    pub async fn abandon_goal(
        &self,
        mission_id: &str,
        goal_id: &str,
        reason: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(&GoalStatus::Abandoned)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let now = bson::DateTime::now();
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].status": status_bson,
                    "goal_tree.$[elem].pivot_reason": reason,
                    "goal_tree.$[elem].completed_at": now,
                    "updated_at": now,
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Reset orphaned Running/Planning missions to Failed on server startup.
    /// Recover all stale live missions regardless of historical instance id.
    pub async fn recover_orphaned_missions(
        &self,
        _instance_id: &str,
    ) -> Result<u64, mongodb::error::Error> {
        let now = bson::DateTime::now();
        let filter = doc! {
            "status": { "$in": ["running", "planning"] },
        };
        let result = self
            .missions()
            .update_many(
                filter,
                doc! { "$set": {
                    "status": "failed",
                    "updated_at": now,
                    "completed_at": now,
                    "error_message": "Server restarted while mission was in progress",
                }},
                None,
            )
            .await?;
        Ok(result.modified_count)
    }

    // ─── Mission Indexes ─────────────────────────────────

    pub async fn ensure_mission_indexes(&self) {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        let mission_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "status": 1, "created_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "agent_id": 1, "status": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "creator_id": 1, "status": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        ];

        if let Err(e) = self.missions().create_indexes(mission_indexes, None).await {
            tracing::warn!("Failed to create mission indexes: {}", e);
        }

        let artifact_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "step_index": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "artifact_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        ];

        if let Err(e) = self
            .artifacts()
            .create_indexes(artifact_indexes, None)
            .await
        {
            tracing::warn!("Failed to create artifact indexes: {}", e);
        }

        let event_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "event_id": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "run_id": 1, "event_id": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "run_id": { "$exists": true } })
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "created_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "run_id": 1, "created_at": 1 })
                .build(),
        ];

        if let Err(e) = self
            .mission_events()
            .create_indexes(event_indexes, None)
            .await
        {
            tracing::warn!("Failed to create mission event indexes: {}", e);
        }
    }
}
