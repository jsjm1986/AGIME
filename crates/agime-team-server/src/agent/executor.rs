//! Task executor for running agent tasks with MCP support
//!
//! This module provides the TaskExecutor which executes approved tasks
//! using the agime provider system.

use agime_team::models::{AgentTask, ApiFormat, TaskResultType, TaskStatus, TeamAgent};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::sync::Arc;
use uuid::Uuid;

/// Task executor for running agent tasks
pub struct TaskExecutor {
    pool: Arc<SqlitePool>,
}

impl TaskExecutor {
    /// Create a new task executor
    pub fn new(pool: Arc<SqlitePool>) -> Self {
        Self { pool }
    }

    /// Execute an approved task
    pub async fn execute_task(&self, task_id: &str) -> Result<()> {
        // 1. Get task and agent info
        let task = self.get_task(task_id).await?
            .ok_or_else(|| anyhow!("Task not found"))?;

        if task.status != TaskStatus::Approved {
            return Err(anyhow!("Task is not approved"));
        }

        let agent = self.get_agent(&task.agent_id).await?
            .ok_or_else(|| anyhow!("Agent not found"))?;

        // 2. Update task status to running
        self.update_task_status(task_id, TaskStatus::Running).await?;

        // 3. Execute the task
        let result = self.run_task(&task, &agent).await;

        // 4. Update task status based on result
        match result {
            Ok(_) => {
                self.update_task_status(task_id, TaskStatus::Completed).await?;
            }
            Err(e) => {
                self.update_task_error(task_id, &e.to_string()).await?;
            }
        }

        Ok(())
    }

    /// Run the actual task execution
    async fn run_task(&self, task: &AgentTask, agent: &TeamAgent) -> Result<()> {
        // Get messages from content (content is already serde_json::Value)
        let messages = task.content.get("messages")
            .and_then(|m| m.as_array())
            .ok_or_else(|| anyhow!("Invalid task content: missing messages"))?;

        // Build request based on API format
        let response = match agent.api_format {
            ApiFormat::OpenAI => self.call_openai_api(agent, messages).await?,
            ApiFormat::Anthropic => self.call_anthropic_api(agent, messages).await?,
            ApiFormat::Local => self.call_local_api(agent, messages).await?,
        };

        // Save result
        self.save_task_result(&task.id, TaskResultType::Message, &response).await?;

        Ok(())
    }

    /// Call OpenAI-compatible API
    async fn call_openai_api(&self, agent: &TeamAgent, messages: &[serde_json::Value]) -> Result<String> {
        let api_url = agent.api_url.as_deref()
            .unwrap_or("https://api.openai.com/v1/chat/completions");
        let model = agent.model.as_deref().unwrap_or("gpt-4");
        let api_key = agent.api_key.as_deref()
            .ok_or_else(|| anyhow!("API key not configured"))?;

        let client = reqwest::Client::new();
        let response = client
            .post(api_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "model": model,
                "messages": messages,
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await?;
            return Err(anyhow!("API error: {}", error));
        }

        let result: serde_json::Value = response.json().await?;
        let content = result["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(content)
    }

    /// Call Anthropic API
    async fn call_anthropic_api(&self, agent: &TeamAgent, messages: &[serde_json::Value]) -> Result<String> {
        let api_url = agent.api_url.as_deref()
            .unwrap_or("https://api.anthropic.com/v1/messages");
        let model = agent.model.as_deref().unwrap_or("claude-3-opus-20240229");
        let api_key = agent.api_key.as_deref()
            .ok_or_else(|| anyhow!("API key not configured"))?;

        let client = reqwest::Client::new();
        let response = client
            .post(api_url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "model": model,
                "max_tokens": 4096,
                "messages": messages,
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await?;
            return Err(anyhow!("API error: {}", error));
        }

        let result: serde_json::Value = response.json().await?;
        let content = result["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(content)
    }

    /// Call local Ollama API
    async fn call_local_api(&self, agent: &TeamAgent, messages: &[serde_json::Value]) -> Result<String> {
        let api_url = agent.api_url.as_deref()
            .unwrap_or("http://localhost:11434/api/chat");
        let model = agent.model.as_deref().unwrap_or("llama2");

        let client = reqwest::Client::new();
        let response = client
            .post(api_url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": false,
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await?;
            return Err(anyhow!("API error: {}", error));
        }

        let result: serde_json::Value = response.json().await?;
        let content = result["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(content)
    }

    // ========================================
    // Database helper methods
    // ========================================

    /// Get task by ID
    async fn get_task(&self, task_id: &str) -> Result<Option<AgentTask>> {
        let row = sqlx::query_as::<_, TaskRow>(
            "SELECT * FROM agent_tasks WHERE id = ?",
        )
        .bind(task_id)
        .fetch_optional(self.pool.as_ref())
        .await?;

        Ok(row.map(|r| r.into()))
    }

    /// Get agent by ID
    async fn get_agent(&self, agent_id: &str) -> Result<Option<TeamAgent>> {
        let row = sqlx::query_as::<_, AgentRow>(
            "SELECT * FROM team_agents WHERE id = ?",
        )
        .bind(agent_id)
        .fetch_optional(self.pool.as_ref())
        .await?;

        Ok(row.map(|r| r.into()))
    }

    /// Update task status
    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let status_str = status.to_string();

        let started_at = if status == TaskStatus::Running {
            Some(now.clone())
        } else {
            None
        };

        let completed_at = if status == TaskStatus::Completed || status == TaskStatus::Failed {
            Some(now.clone())
        } else {
            None
        };

        sqlx::query(
            "UPDATE agent_tasks SET status = ?, started_at = COALESCE(?, started_at), completed_at = COALESCE(?, completed_at) WHERE id = ?",
        )
        .bind(&status_str)
        .bind(&started_at)
        .bind(&completed_at)
        .bind(task_id)
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    /// Update task with error
    async fn update_task_error(&self, task_id: &str, error: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "UPDATE agent_tasks SET status = 'failed', error_message = ?, completed_at = ? WHERE id = ?",
        )
        .bind(error)
        .bind(&now)
        .bind(task_id)
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    /// Save task result
    async fn save_task_result(&self, task_id: &str, result_type: TaskResultType, content: &str) -> Result<()> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let result_type_str = result_type.to_string();

        sqlx::query(
            "INSERT INTO agent_task_results (id, task_id, result_type, content, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(task_id)
        .bind(&result_type_str)
        .bind(content)
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
            content: serde_json::from_str(&row.content).unwrap_or(serde_json::Value::Null),
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
            api_key: row.api_key,
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
