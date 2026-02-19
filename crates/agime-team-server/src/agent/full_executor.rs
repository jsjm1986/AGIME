//! Full Agent Executor with MCP and Skills support
//!
//! This module provides a complete Agent executor that integrates with
//! the agime Agent system, including MCP extensions and Skills loading.

use agime::agents::AgentEvent;
use agime::agents::extension::{ExtensionConfig, Envs};
use agime::agents::types::SessionConfig;
use agime::agents::Agent;
use agime::conversation::message::{Message, MessageContent};
use agime::providers::{self, base::Provider};
use agime::session::{SessionManager, SessionType};
use agime_team::models::{
    AgentTask, ApiFormat, CustomExtensionConfig, TaskResultType, TaskStatus, TeamAgent,
};
use anyhow::{anyhow, Result};
use chrono::Utc;
use futures::StreamExt;
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

use super::task_manager::{StreamEvent, TaskManager};

/// Global mutex to protect environment variable settings during provider creation
/// This prevents race conditions when multiple tasks run concurrently
static PROVIDER_ENV_LOCK: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));

/// Full Agent Executor with complete MCP and Skills support
pub struct FullAgentExecutor {
    pool: Arc<SqlitePool>,
    task_manager: Arc<TaskManager>,
}

impl FullAgentExecutor {
    /// Create a new full agent executor
    pub fn new(pool: Arc<SqlitePool>, task_manager: Arc<TaskManager>) -> Self {
        Self { pool, task_manager }
    }

    /// Execute an approved task using the full agime Agent system
    pub async fn execute_task(&self, task_id: &str) -> Result<()> {
        // 1. Get task and agent info
        let task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| anyhow!("Task not found"))?;

        if task.status != TaskStatus::Approved {
            return Err(anyhow!("Task is not approved"));
        }

        let team_agent = self
            .get_agent(&task.agent_id)
            .await?
            .ok_or_else(|| anyhow!("Agent not found"))?;

        // 2. Update task status to running
        self.update_task_status(task_id, TaskStatus::Running).await?;
        info!("Starting task execution: {}", task_id);

        // Broadcast status update
        self.task_manager.broadcast(task_id, StreamEvent::Status {
            status: "running".to_string(),
        }).await;

        // 3. Create and configure the Agent
        let result = self.run_with_agent(&task, &team_agent).await;

        // 4. Update task status based on result
        match result {
            Ok(_) => {
                self.update_task_status(task_id, TaskStatus::Completed)
                    .await?;
                info!("Task completed successfully: {}", task_id);

                // Broadcast done event
                self.task_manager.broadcast(task_id, StreamEvent::Done {
                    status: "completed".to_string(),
                    error: None,
                }).await;
            }
            Err(e) => {
                error!("Task failed: {} - {}", task_id, e);
                self.update_task_error(task_id, &e.to_string()).await?;

                // Broadcast error event
                self.task_manager.broadcast(task_id, StreamEvent::Done {
                    status: "failed".to_string(),
                    error: Some(e.to_string()),
                }).await;
            }
        }

        Ok(())
    }

    /// Run task with full Agent capabilities
    async fn run_with_agent(&self, task: &AgentTask, team_agent: &TeamAgent) -> Result<()> {
        // Create session in agime's database first (required for message storage)
        let working_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let session_name = format!("Team Task: {}", task.id);
        let session = SessionManager::create_session(
            working_dir,
            session_name,
            SessionType::SubAgent,
        ).await?;
        let session_id = session.id.clone();
        info!("Created session {} for task: {}", session_id, task.id);

        // Create the Agent
        let agent = Agent::new();

        // Set up the provider based on TeamAgent configuration
        let provider = self.create_provider(team_agent).await?;
        agent.update_provider(provider, &session_id).await?;

        // Load extensions based on agent configuration
        self.load_configured_extensions(&agent, team_agent).await?;

        // Load team skills
        self.load_team_skills(&agent, &task.team_id).await?;

        // Build user message from task content
        let user_message = self.build_user_message(task)?;

        // Create session config
        let session_config = SessionConfig {
            id: session_id.clone(),
            schedule_id: None,
            max_turns: Some(100), // Limit turns for safety
            retry_config: None,
        };

        // Create cancellation token
        let cancel_token = CancellationToken::new();

        // Execute the agent reply with tool loop
        let mut event_stream = agent
            .reply(user_message, session_config, Some(cancel_token))
            .await?;

        // Process events and save results (streaming)
        while let Some(event_result) = event_stream.next().await {
            match event_result {
                Ok(event) => {
                    self.handle_agent_event(&task.id, &event).await?;
                }
                Err(e) => {
                    warn!("Error in agent event stream: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Handle agent events and save results
    async fn handle_agent_event(&self, task_id: &str, event: &AgentEvent) -> Result<()> {
        match event {
            AgentEvent::Message(msg) => {
                let content = self.extract_message_content(msg);
                if !content.is_empty() {
                    // Broadcast text event for real-time streaming
                    self.task_manager.broadcast(task_id, StreamEvent::Text {
                        content: content.clone(),
                    }).await;

                    // Also save to database for persistence
                    self.save_task_result(task_id, TaskResultType::Message, &content)
                        .await?;
                }
            }
            AgentEvent::McpNotification((extension_name, notification)) => {
                info!(
                    "MCP notification from {}: {:?}",
                    extension_name, notification
                );
            }
            AgentEvent::ModelChange { model, mode } => {
                info!("Model changed to {} (mode: {})", model, mode);
            }
            AgentEvent::HistoryReplaced(_) => {
                info!("Conversation history was compacted");
            }
        }
        Ok(())
    }

    /// Create a provider based on TeamAgent configuration
    /// Uses a mutex to protect environment variable settings from concurrent access
    async fn create_provider(&self, agent: &TeamAgent) -> Result<Arc<dyn Provider>> {
        let model_name = agent.model.clone().unwrap_or_else(|| "gpt-4".to_string());

        // Get the API key from the database
        let api_key = self.get_agent_api_key(&agent.id).await?;

        info!(
            "Creating provider: format={:?}, model={}, has_api_key={}, api_url={:?}",
            agent.api_format, model_name, api_key.is_some(), agent.api_url
        );

        // Acquire lock to protect environment variable settings
        let _guard = PROVIDER_ENV_LOCK.lock().await;

        let result = match agent.api_format {
            ApiFormat::OpenAI => {
                if let Some(ref key) = api_key {
                    std::env::set_var("AGIME_OPENAI_API_KEY", key);
                } else {
                    warn!("No API key found in database for OpenAI agent");
                }
                if let Some(ref url) = agent.api_url {
                    std::env::set_var("AGIME_OPENAI_HOST", url);
                }
                providers::create_with_named_model("openai", &model_name).await
            }
            ApiFormat::Anthropic => {
                if let Some(ref key) = api_key {
                    std::env::set_var("AGIME_ANTHROPIC_API_KEY", key);
                } else {
                    warn!("No API key found in database for Anthropic agent");
                }
                if let Some(ref url) = agent.api_url {
                    std::env::set_var("AGIME_ANTHROPIC_HOST", url);
                }
                providers::create_with_named_model("anthropic", &model_name).await
            }
            ApiFormat::Local => {
                if let Some(ref url) = agent.api_url {
                    std::env::set_var("OLLAMA_HOST", url);
                }
                providers::create_with_named_model("ollama", &model_name).await
            }
        };

        // Clear sensitive environment variables after provider creation
        std::env::remove_var("AGIME_OPENAI_API_KEY");
        std::env::remove_var("AGIME_ANTHROPIC_API_KEY");

        result
    }

    /// Load extensions based on TeamAgent configuration
    async fn load_configured_extensions(&self, agent: &Agent, team_agent: &TeamAgent) -> Result<()> {
        // Load enabled builtin extensions
        for ext_config in &team_agent.enabled_extensions {
            if !ext_config.enabled {
                continue;
            }
            let ext = &ext_config.extension;
            if ext.is_platform() {
                self.add_platform_extension(agent, ext.name(), ext.description()).await;
            } else {
                self.add_builtin_extension(agent, ext.name(), ext.description()).await;
            }
        }

        // Load custom extensions
        for custom in &team_agent.custom_extensions {
            if !custom.enabled {
                continue;
            }
            self.add_custom_extension(agent, custom).await;
        }

        // Load TeamTools extension for Skills and MCP management via conversation
        self.add_team_tools_extension(agent, team_agent).await;

        Ok(())
    }

    /// Add a Platform extension (in-process)
    async fn add_platform_extension(&self, agent: &Agent, name: &str, desc: &str) {
        let config = ExtensionConfig::Platform {
            name: name.to_string(),
            description: desc.to_string(),
            bundled: Some(true),
            available_tools: vec![],
        };
        if let Err(e) = agent.add_extension(config).await {
            warn!("Failed to load {} extension: {}", name, e);
        }
    }

    /// Add a Builtin MCP server extension (subprocess)
    async fn add_builtin_extension(&self, agent: &Agent, name: &str, desc: &str) {
        let config = ExtensionConfig::Builtin {
            name: name.to_string(),
            description: desc.to_string(),
            display_name: Some(name.to_string()),
            timeout: Some(30000),
            bundled: Some(true),
            available_tools: vec![],
        };
        if let Err(e) = agent.add_extension(config).await {
            warn!("Failed to load {} builtin: {}", name, e);
        }
    }

    /// Add a custom extension (SSE or Stdio)
    async fn add_custom_extension(&self, agent: &Agent, custom: &CustomExtensionConfig) {
        let config = match custom.ext_type.as_str() {
            "sse" => ExtensionConfig::Sse {
                name: custom.name.clone(),
                description: String::new(),
                uri: custom.uri_or_cmd.clone(),
                envs: Envs::new(custom.envs.clone()),
                env_keys: vec![],
                timeout: Some(30000),
                bundled: Some(false),
                available_tools: vec![],
            },
            "stdio" => ExtensionConfig::Stdio {
                name: custom.name.clone(),
                description: String::new(),
                cmd: custom.uri_or_cmd.clone(),
                args: custom.args.clone(),
                envs: Envs::new(custom.envs.clone()),
                env_keys: vec![],
                timeout: Some(30000),
                bundled: Some(false),
                available_tools: vec![],
            },
            _ => {
                warn!("Unknown extension type: {}", custom.ext_type);
                return;
            }
        };
        if let Err(e) = agent.add_extension(config).await {
            warn!("Failed to load custom extension {}: {}", custom.name, e);
        }
    }

    /// Add TeamTools extension for managing Skills and MCP via conversation
    /// This extension allows the Agent to install/remove Skills and MCP extensions
    /// and saves them to the team database for sharing with team members
    async fn add_team_tools_extension(&self, agent: &Agent, team_agent: &TeamAgent) {
        // Get the current executable path for running the MCP server
        let exe_path = std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "agime-team-server".to_string());

        // Get database URL from environment or use default
        let db_url = std::env::var("AGIME_TEAM_DB_URL")
            .unwrap_or_else(|_| "sqlite://./data/team.db?mode=rwc".to_string());

        // Build environment variables for the TeamTools MCP server
        let mut envs = std::collections::HashMap::new();
        envs.insert("AGIME_TEAM_DB_URL".to_string(), db_url);
        envs.insert("AGIME_TEAM_ID".to_string(), team_agent.team_id.clone());
        envs.insert("AGIME_AGENT_ID".to_string(), team_agent.id.clone());
        envs.insert("AGIME_USER_ID".to_string(), "agent".to_string());

        let config = ExtensionConfig::Stdio {
            name: "teamtools".to_string(),
            description: "Team tools for managing Skills and MCP extensions. \
                Use install_skill to add new skills, list_skills to see available skills, \
                install_mcp to add MCP extensions, and list_mcps to see configured extensions."
                .to_string(),
            cmd: exe_path,
            args: vec!["mcp".to_string(), "teamtools".to_string()],
            envs: Envs::new(envs),
            env_keys: vec![],
            timeout: Some(60000),
            bundled: Some(true),
            available_tools: vec![],
        };

        if let Err(e) = agent.add_extension(config).await {
            warn!("Failed to load teamtools extension: {}", e);
        } else {
            info!("Loaded teamtools extension for team {}", team_agent.team_id);
        }
    }

    /// Load team skills and inject them into the agent's system prompt
    async fn load_team_skills(&self, agent: &Agent, team_id: &str) -> Result<()> {
        // Query team skills from database
        let skills = sqlx::query_as::<_, SkillRow>(
            "SELECT id, name, description, content FROM shared_skills \
             WHERE team_id = ? AND is_deleted = 0 \
             ORDER BY use_count DESC LIMIT 20"
        )
        .bind(team_id)
        .fetch_all(self.pool.as_ref())
        .await?;

        if skills.is_empty() {
            info!("No team skills found for team: {}", team_id);
            return Ok(());
        }

        // Build skills instruction
        let mut skills_instruction = String::from(
            "\n\n# Team Skills\n\n\
             The following skills are available from your team. \
             You can use them as reference or guidance:\n\n"
        );

        for skill in &skills {
            skills_instruction.push_str(&format!(
                "## Skill: {}\n\n{}\n\n---\n\n",
                skill.name,
                skill.content
            ));
        }

        // Inject skills into agent's system prompt
        agent.extend_system_prompt(skills_instruction).await;
        info!("Loaded {} team skills for team: {}", skills.len(), team_id);

        Ok(())
    }

    /// Build user message from task content
    fn build_user_message(&self, task: &AgentTask) -> Result<Message> {
        let messages = task
            .content
            .get("messages")
            .and_then(|m| m.as_array())
            .ok_or_else(|| anyhow!("Invalid task content: missing messages"))?;

        // Get the last user message
        let last_user_msg = messages
            .iter()
            .rev()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
            .ok_or_else(|| anyhow!("No user message found"))?;

        let content = last_user_msg
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("");

        Ok(Message::user().with_text(content))
    }

    /// Extract text content from a message
    fn extract_message_content(&self, msg: &Message) -> String {
        let mut content = String::new();
        for part in msg.content.iter() {
            match part {
                MessageContent::Text(text) => {
                    content.push_str(&text.text);
                }
                MessageContent::ToolRequest(req) => {
                    if let Ok(ref tc) = req.tool_call {
                        content.push_str(&format!("[Tool: {}]", tc.name));
                    }
                }
                MessageContent::ToolResponse(resp) => {
                    content.push_str(&format!("[Result: {}]", resp.id));
                }
                _ => {}
            }
        }
        content
    }

    // ========================================
    // Database helper methods
    // ========================================

    async fn get_task(&self, task_id: &str) -> Result<Option<AgentTask>> {
        let row = sqlx::query_as::<_, TaskRow>("SELECT * FROM agent_tasks WHERE id = ?")
            .bind(task_id)
            .fetch_optional(self.pool.as_ref())
            .await?;
        Ok(row.map(|r| r.into()))
    }

    async fn get_agent(&self, agent_id: &str) -> Result<Option<TeamAgent>> {
        let row = sqlx::query_as::<_, AgentRow>("SELECT * FROM team_agents WHERE id = ?")
            .bind(agent_id)
            .fetch_optional(self.pool.as_ref())
            .await?;
        Ok(row.map(|r| r.into()))
    }

    async fn get_agent_api_key(&self, agent_id: &str) -> Result<Option<String>> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT api_key FROM team_agents WHERE id = ?")
                .bind(agent_id)
                .fetch_optional(self.pool.as_ref())
                .await?;
        Ok(row.and_then(|r| r.0))
    }

    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let status_str = status.to_string();

        let (started_at, completed_at) = match status {
            TaskStatus::Running => (Some(now.clone()), None),
            TaskStatus::Completed | TaskStatus::Failed => (None, Some(now.clone())),
            _ => (None, None),
        };

        sqlx::query(
            "UPDATE agent_tasks SET status = ?, \
             started_at = COALESCE(?, started_at), \
             completed_at = COALESCE(?, completed_at) WHERE id = ?",
        )
        .bind(&status_str)
        .bind(&started_at)
        .bind(&completed_at)
        .bind(task_id)
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    async fn update_task_error(&self, task_id: &str, error: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE agent_tasks SET status = 'failed', \
             error_message = ?, completed_at = ? WHERE id = ?",
        )
        .bind(error)
        .bind(&now)
        .bind(task_id)
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    async fn save_task_result(
        &self,
        task_id: &str,
        result_type: TaskResultType,
        content: &str,
    ) -> Result<()> {
        // Check if task still exists before saving result
        // This prevents foreign key constraint failures if task was deleted during execution
        let task_exists: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM agent_tasks WHERE id = ?"
        )
        .bind(task_id)
        .fetch_optional(self.pool.as_ref())
        .await?;

        if task_exists.is_none() {
            warn!("Task {} no longer exists, skipping result save", task_id);
            return Ok(());
        }

        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let result_type_str = result_type.to_string();
        let content_json = serde_json::json!({ "text": content }).to_string();

        sqlx::query(
            "INSERT INTO agent_task_results \
             (id, task_id, result_type, content, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(task_id)
        .bind(&result_type_str)
        .bind(&content_json)
        .bind(&now)
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }
}

// ========================================
// Database row types
// ========================================

#[derive(sqlx::FromRow)]
struct TaskRow {
    id: String,
    team_id: String,
    agent_id: String,
    submitter_id: String,
    approver_id: Option<String>,
    task_type: String,
    content: String,
    status: String,
    priority: i32,
    submitted_at: String,
    approved_at: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    error_message: Option<String>,
}

impl From<TaskRow> for AgentTask {
    fn from(row: TaskRow) -> Self {
        use chrono::{DateTime, Utc};
        let parse_dt = |s: &str| -> DateTime<Utc> {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now())
        };
        Self {
            id: row.id,
            team_id: row.team_id,
            agent_id: row.agent_id,
            submitter_id: row.submitter_id,
            approver_id: row.approver_id,
            task_type: row.task_type.parse().unwrap_or_default(),
            content: serde_json::from_str(&row.content).unwrap_or_default(),
            status: row.status.parse().unwrap_or_default(),
            priority: row.priority,
            submitted_at: parse_dt(&row.submitted_at),
            approved_at: row.approved_at.map(|s| parse_dt(&s)),
            started_at: row.started_at.map(|s| parse_dt(&s)),
            completed_at: row.completed_at.map(|s| parse_dt(&s)),
            error_message: row.error_message,
        }
    }
}

#[derive(sqlx::FromRow)]
struct AgentRow {
    id: String,
    team_id: String,
    name: String,
    description: Option<String>,
    api_url: Option<String>,
    model: Option<String>,
    #[allow(dead_code)]
    api_key: Option<String>,
    api_format: String,
    status: String,
    last_error: Option<String>,
    created_at: String,
    updated_at: String,
    enabled_extensions: Option<String>,
    custom_extensions: Option<String>,
}

impl From<AgentRow> for TeamAgent {
    fn from(row: AgentRow) -> Self {
        use agime_team::models::{AgentExtensionConfig, AgentStatus, CustomExtensionConfig};
        use chrono::{DateTime, Utc};
        let parse_dt = |s: &str| -> DateTime<Utc> {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now())
        };

        // Parse enabled_extensions JSON
        let enabled_extensions: Vec<AgentExtensionConfig> = row
            .enabled_extensions
            .as_ref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        // Parse custom_extensions JSON
        let custom_extensions: Vec<CustomExtensionConfig> = row
            .custom_extensions
            .as_ref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        Self {
            id: row.id,
            team_id: row.team_id,
            name: row.name,
            description: row.description,
            system_prompt: None,
            api_url: row.api_url,
            model: row.model,
            api_key: None,
            api_format: row.api_format.parse().unwrap_or_default(),
            enabled_extensions,
            custom_extensions,
            status: row.status.parse().unwrap_or(AgentStatus::Idle),
            last_error: row.last_error,
            access_mode: Default::default(),
            allowed_groups: vec![],
            denied_groups: vec![],
            max_concurrent_tasks: 1,
            temperature: None,
            max_tokens: None,
            context_limit: None,
            created_at: parse_dt(&row.created_at),
            updated_at: parse_dt(&row.updated_at),
        }
    }
}

#[derive(sqlx::FromRow)]
struct SkillRow {
    #[allow(dead_code)]
    id: String,
    name: String,
    #[allow(dead_code)]
    description: Option<String>,
    content: String,
}
