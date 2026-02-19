//! Agent service layer for business logic

use agime_team::models::{
    AgentTask, AgentStatus, CreateAgentRequest, ListAgentsQuery,
    ListTasksQuery, PaginatedResponse, SubmitTaskRequest, TaskResult,
    TaskResultType, TaskStatus, TaskType, TeamAgent, UpdateAgentRequest,
};
use chrono::Utc;
use sqlx::SqlitePool;
use std::sync::Arc;
use uuid::Uuid;

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
}

/// Service error that includes both database and validation errors
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Validation error: {0}")]
    Validation(#[from] ValidationError),
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
        if !trimmed.is_empty() && !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
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

/// Agent service for managing team agents and tasks
pub struct AgentService {
    pool: Arc<SqlitePool>,
}

impl AgentService {
    /// Create a new agent service
    pub fn new(pool: Arc<SqlitePool>) -> Self {
        Self { pool }
    }

    // ========================================
    // Permission checks
    // ========================================

    /// Check if user is a member of the team
    pub async fn is_team_member(&self, user_id: &str, team_id: &str) -> Result<bool, sqlx::Error> {
        let result: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM team_members WHERE user_id = ? AND team_id = ? AND deleted = 0"
        )
        .bind(user_id)
        .bind(team_id)
        .fetch_optional(self.pool.as_ref())
        .await?;
        Ok(result.is_some())
    }

    /// Check if user is an admin of the team
    pub async fn is_team_admin(&self, user_id: &str, team_id: &str) -> Result<bool, sqlx::Error> {
        let result: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM team_members WHERE user_id = ? AND team_id = ? AND role IN ('admin', 'owner') AND deleted = 0"
        )
        .bind(user_id)
        .bind(team_id)
        .fetch_optional(self.pool.as_ref())
        .await?;
        Ok(result.is_some())
    }

    /// Get team_id for an agent
    pub async fn get_agent_team_id(&self, agent_id: &str) -> Result<Option<String>, sqlx::Error> {
        let result: Option<(String,)> = sqlx::query_as(
            "SELECT team_id FROM team_agents WHERE id = ?"
        )
        .bind(agent_id)
        .fetch_optional(self.pool.as_ref())
        .await?;
        Ok(result.map(|r| r.0))
    }

    /// Get team_id for a task
    pub async fn get_task_team_id(&self, task_id: &str) -> Result<Option<String>, sqlx::Error> {
        let result: Option<(String,)> = sqlx::query_as(
            "SELECT team_id FROM agent_tasks WHERE id = ?"
        )
        .bind(task_id)
        .fetch_optional(self.pool.as_ref())
        .await?;
        Ok(result.map(|r| r.0))
    }

    // ========================================
    // Agent CRUD operations
    // ========================================

    /// Create a new team agent with validation
    pub async fn create_agent(&self, req: CreateAgentRequest) -> Result<TeamAgent, ServiceError> {
        // Validate inputs
        validate_name(&req.name)?;
        validate_api_url(&req.api_url)?;
        validate_model(&req.model)?;

        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let api_format = req.api_format.as_deref().unwrap_or("openai");

        // Serialize extensions to JSON
        let enabled_extensions = req
            .enabled_extensions
            .as_ref()
            .map(|e| serde_json::to_string(e).unwrap_or_else(|_| "[]".to_string()))
            .unwrap_or_else(|| "[]".to_string());

        let custom_extensions = req
            .custom_extensions
            .as_ref()
            .map(|e| serde_json::to_string(e).unwrap_or_else(|_| "[]".to_string()))
            .unwrap_or_else(|| "[]".to_string());

        sqlx::query(
            r#"
            INSERT INTO team_agents (id, team_id, name, description, api_url, model, api_key, api_format, status, enabled_extensions, custom_extensions, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'idle', ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(&req.team_id)
        .bind(&req.name)
        .bind(&req.description)
        .bind(&req.api_url)
        .bind(&req.model)
        .bind(&req.api_key)
        .bind(api_format)
        .bind(&enabled_extensions)
        .bind(&custom_extensions)
        .bind(&now)
        .bind(&now)
        .execute(self.pool.as_ref())
        .await?;

        Ok(self.get_agent(&id).await?.ok_or(sqlx::Error::RowNotFound)?)
    }

    /// Get agent by ID
    pub async fn get_agent(&self, id: &str) -> Result<Option<TeamAgent>, sqlx::Error> {
        let row = sqlx::query_as::<_, AgentRow>(
            "SELECT * FROM team_agents WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool.as_ref())
        .await?;

        Ok(row.map(|r| r.into()))
    }

    /// List agents for a team
    pub async fn list_agents(&self, query: ListAgentsQuery) -> Result<PaginatedResponse<TeamAgent>, sqlx::Error> {
        // Limit max page size to prevent memory issues
        let limit = query.limit.min(100);
        let offset = (query.page.saturating_sub(1)) * limit;

        let total: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM team_agents WHERE team_id = ?",
        )
        .bind(&query.team_id)
        .fetch_one(self.pool.as_ref())
        .await?;

        let rows = sqlx::query_as::<_, AgentRow>(
            "SELECT * FROM team_agents WHERE team_id = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
        )
        .bind(&query.team_id)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(self.pool.as_ref())
        .await?;

        let items: Vec<TeamAgent> = rows.into_iter().map(|r| r.into()).collect();

        Ok(PaginatedResponse::new(items, total.0 as u64, query.page, query.limit))
    }

    /// Update an agent
    pub async fn update_agent(&self, id: &str, req: UpdateAgentRequest) -> Result<Option<TeamAgent>, sqlx::Error> {
        let now = Utc::now().to_rfc3339();

        let mut updates = vec!["updated_at = ?".to_string()];
        let mut values: Vec<String> = vec![now];

        if let Some(name) = req.name {
            updates.push("name = ?".to_string());
            values.push(name);
        }
        if let Some(desc) = req.description {
            updates.push("description = ?".to_string());
            values.push(desc);
        }
        if let Some(api_url) = req.api_url {
            updates.push("api_url = ?".to_string());
            values.push(api_url);
        }
        if let Some(model) = req.model {
            updates.push("model = ?".to_string());
            values.push(model);
        }
        if let Some(api_key) = req.api_key {
            updates.push("api_key = ?".to_string());
            values.push(api_key);
        }
        if let Some(api_format) = req.api_format {
            updates.push("api_format = ?".to_string());
            values.push(api_format);
        }
        if let Some(status) = req.status {
            updates.push("status = ?".to_string());
            values.push(status.to_string());
        }
        if let Some(ref enabled_extensions) = req.enabled_extensions {
            updates.push("enabled_extensions = ?".to_string());
            values.push(serde_json::to_string(enabled_extensions).unwrap_or_else(|_| "[]".to_string()));
        }
        if let Some(ref custom_extensions) = req.custom_extensions {
            updates.push("custom_extensions = ?".to_string());
            values.push(serde_json::to_string(custom_extensions).unwrap_or_else(|_| "[]".to_string()));
        }

        let sql = format!(
            "UPDATE team_agents SET {} WHERE id = ?",
            updates.join(", ")
        );

        let mut query = sqlx::query(&sql);
        for v in &values {
            query = query.bind(v);
        }
        query = query.bind(id);
        query.execute(self.pool.as_ref()).await?;

        self.get_agent(id).await
    }

    /// Delete an agent
    pub async fn delete_agent(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM team_agents WHERE id = ?")
            .bind(id)
            .execute(self.pool.as_ref())
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // ========================================
    // Task operations
    // ========================================

    /// Submit a new task with validation
    pub async fn submit_task(&self, submitter_id: &str, req: SubmitTaskRequest) -> Result<AgentTask, ServiceError> {
        // Validate priority
        validate_priority(req.priority)?;

        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let content = serde_json::to_string(&req.content).unwrap_or_default();

        sqlx::query(
            r#"
            INSERT INTO agent_tasks (id, team_id, agent_id, submitter_id, task_type, content, status, priority, submitted_at)
            VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?)
            "#,
        )
        .bind(&id)
        .bind(&req.team_id)
        .bind(&req.agent_id)
        .bind(submitter_id)
        .bind(req.task_type.to_string())
        .bind(&content)
        .bind(req.priority)
        .bind(&now)
        .execute(self.pool.as_ref())
        .await?;

        Ok(self.get_task(&id).await?.ok_or(sqlx::Error::RowNotFound)?)
    }

    /// Get task by ID
    pub async fn get_task(&self, id: &str) -> Result<Option<AgentTask>, sqlx::Error> {
        let row = sqlx::query_as::<_, TaskRow>(
            "SELECT * FROM agent_tasks WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool.as_ref())
        .await?;

        Ok(row.map(|r| r.into()))
    }

    /// List tasks for a team
    pub async fn list_tasks(&self, query: ListTasksQuery) -> Result<PaginatedResponse<AgentTask>, sqlx::Error> {
        // Limit max page size to prevent memory issues
        let limit = query.limit.min(100);
        let offset = (query.page.saturating_sub(1)) * limit;

        let mut where_clauses = vec!["team_id = ?".to_string()];
        let mut params: Vec<String> = vec![query.team_id.clone()];

        if let Some(agent_id) = &query.agent_id {
            where_clauses.push("agent_id = ?".to_string());
            params.push(agent_id.clone());
        }
        if let Some(status) = &query.status {
            where_clauses.push("status = ?".to_string());
            params.push(status.to_string());
        }

        let where_sql = where_clauses.join(" AND ");

        // Count query
        let count_sql = format!("SELECT COUNT(*) FROM agent_tasks WHERE {}", where_sql);
        let mut count_query = sqlx::query_as::<_, (i64,)>(&count_sql);
        for p in &params {
            count_query = count_query.bind(p);
        }
        let total = count_query.fetch_one(self.pool.as_ref()).await?;

        // Data query
        let data_sql = format!(
            "SELECT * FROM agent_tasks WHERE {} ORDER BY priority DESC, submitted_at DESC LIMIT ? OFFSET ?",
            where_sql
        );
        let mut data_query = sqlx::query_as::<_, TaskRow>(&data_sql);
        for p in &params {
            data_query = data_query.bind(p);
        }
        data_query = data_query.bind(limit as i64).bind(offset as i64);
        let rows = data_query.fetch_all(self.pool.as_ref()).await?;

        let items: Vec<AgentTask> = rows.into_iter().map(|r| r.into()).collect();
        Ok(PaginatedResponse::new(items, total.0 as u64, query.page, limit))
    }

    /// Approve a task (admin only)
    /// Returns None if task not found or not in pending status
    pub async fn approve_task(&self, task_id: &str, approver_id: &str) -> Result<Option<AgentTask>, sqlx::Error> {
        let now = Utc::now().to_rfc3339();

        let result = sqlx::query(
            "UPDATE agent_tasks SET status = 'approved', approver_id = ?, approved_at = ? WHERE id = ? AND status = 'pending'",
        )
        .bind(approver_id)
        .bind(&now)
        .bind(task_id)
        .execute(self.pool.as_ref())
        .await?;

        // Check if update was successful
        if result.rows_affected() == 0 {
            // Task not found or not in pending status
            return Ok(None);
        }

        self.get_task(task_id).await
    }

    /// Reject a task (admin only)
    /// Returns None if task not found or not in pending status
    pub async fn reject_task(&self, task_id: &str, approver_id: &str) -> Result<Option<AgentTask>, sqlx::Error> {
        let now = Utc::now().to_rfc3339();

        let result = sqlx::query(
            "UPDATE agent_tasks SET status = 'rejected', approver_id = ?, approved_at = ? WHERE id = ? AND status = 'pending'",
        )
        .bind(approver_id)
        .bind(&now)
        .bind(task_id)
        .execute(self.pool.as_ref())
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_task(task_id).await
    }

    /// Cancel a task (admin only)
    /// Returns None if task not found or not in cancellable status
    pub async fn cancel_task(&self, task_id: &str) -> Result<Option<AgentTask>, sqlx::Error> {
        let now = Utc::now().to_rfc3339();

        let result = sqlx::query(
            "UPDATE agent_tasks SET status = 'cancelled', completed_at = ? WHERE id = ? AND status IN ('pending', 'approved', 'running')",
        )
        .bind(&now)
        .bind(task_id)
        .execute(self.pool.as_ref())
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_task(task_id).await
    }

    /// Get task results
    pub async fn get_task_results(&self, task_id: &str) -> Result<Vec<TaskResult>, sqlx::Error> {
        let rows = sqlx::query_as::<_, TaskResultRow>(
            "SELECT * FROM agent_task_results WHERE task_id = ? ORDER BY created_at ASC",
        )
        .bind(task_id)
        .fetch_all(self.pool.as_ref())
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }
}

// ========================================
// Database row types
// ========================================

#[derive(sqlx::FromRow)]
struct AgentRow {
    id: String,
    team_id: String,
    name: String,
    description: Option<String>,
    api_url: Option<String>,
    model: Option<String>,
    api_key: Option<String>,
    api_format: Option<String>,
    status: String,
    last_error: Option<String>,
    created_at: String,
    updated_at: String,
    enabled_extensions: Option<String>,
    custom_extensions: Option<String>,
}

impl From<AgentRow> for TeamAgent {
    fn from(row: AgentRow) -> Self {
        use agime_team::models::{AgentExtensionConfig, ApiFormat, CustomExtensionConfig};

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
            api_key: None, // Don't expose API key in responses
            api_format: row.api_format
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(ApiFormat::OpenAI),
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
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        }
    }
}

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
        let parse_dt = |s: &str| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now())
        };

        Self {
            id: row.id,
            team_id: row.team_id,
            agent_id: row.agent_id,
            submitter_id: row.submitter_id,
            approver_id: row.approver_id,
            task_type: row.task_type.parse().unwrap_or(TaskType::Chat),
            content: serde_json::from_str(&row.content).unwrap_or(serde_json::Value::Null),
            status: row.status.parse().unwrap_or(TaskStatus::Pending),
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
struct TaskResultRow {
    id: String,
    task_id: String,
    result_type: String,
    content: String,
    created_at: String,
}

impl From<TaskResultRow> for TaskResult {
    fn from(row: TaskResultRow) -> Self {
        let result_type = match row.result_type.as_str() {
            "message" => TaskResultType::Message,
            "tool_call" => TaskResultType::ToolCall,
            _ => TaskResultType::Error,
        };

        Self {
            id: row.id,
            task_id: row.task_id,
            result_type,
            content: serde_json::from_str(&row.content)
                .unwrap_or(serde_json::Value::Null),
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        }
    }
}
