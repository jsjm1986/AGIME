//! Agent Task model
//! Tasks are submitted by team members and executed by agents after admin approval

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Task status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    /// Task is pending approval
    Pending,
    /// Task has been approved, waiting to run
    Approved,
    /// Task has been rejected
    Rejected,
    /// Task is currently running
    Running,
    /// Task completed successfully
    Completed,
    /// Task failed
    Failed,
    /// Task was cancelled
    Cancelled,
}

impl Default for TaskStatus {
    fn default() -> Self {
        Self::Pending
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Approved => write!(f, "approved"),
            Self::Rejected => write!(f, "rejected"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("Invalid task status: {}", s)),
        }
    }
}

/// Task type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskType {
    /// Chat conversation task
    Chat,
    /// Execute a recipe
    Recipe,
    /// Execute a skill
    Skill,
}

impl Default for TaskType {
    fn default() -> Self {
        Self::Chat
    }
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chat => write!(f, "chat"),
            Self::Recipe => write!(f, "recipe"),
            Self::Skill => write!(f, "skill"),
        }
    }
}

impl std::str::FromStr for TaskType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "chat" => Ok(Self::Chat),
            "recipe" => Ok(Self::Recipe),
            "skill" => Ok(Self::Skill),
            _ => Err(format!("Invalid task type: {}", s)),
        }
    }
}

/// Agent Task entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: String,
    pub team_id: String,
    pub agent_id: String,
    /// User who submitted the task
    pub submitter_id: String,
    /// Admin who approved/rejected the task
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approver_id: Option<String>,
    pub task_type: TaskType,
    /// Task content as JSON
    pub content: serde_json::Value,
    pub status: TaskStatus,
    #[serde(default)]
    pub priority: i32,
    pub submitted_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl AgentTask {
    /// Create a new task
    pub fn new(
        team_id: String,
        agent_id: String,
        submitter_id: String,
        task_type: TaskType,
        content: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            team_id,
            agent_id,
            submitter_id,
            approver_id: None,
            task_type,
            content,
            status: TaskStatus::Pending,
            priority: 0,
            submitted_at: Utc::now(),
            approved_at: None,
            started_at: None,
            completed_at: None,
            error_message: None,
        }
    }

    /// Set priority
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
}

/// Request to submit a task
#[derive(Debug, Clone, Deserialize)]
pub struct SubmitTaskRequest {
    pub team_id: String,
    pub agent_id: String,
    pub task_type: TaskType,
    pub content: serde_json::Value,
    #[serde(default)]
    pub priority: i32,
}

/// Task list query parameters
#[derive(Debug, Clone, Deserialize)]
pub struct ListTasksQuery {
    pub team_id: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub status: Option<TaskStatus>,
    #[serde(default = "super::default_page")]
    pub page: u32,
    #[serde(default = "super::default_limit")]
    pub limit: u32,
}

/// Task result type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskResultType {
    /// AI message response
    Message,
    /// Tool call
    ToolCall,
    /// Error
    Error,
}

impl std::fmt::Display for TaskResultType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message => write!(f, "message"),
            Self::ToolCall => write!(f, "tool_call"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// Task execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub id: String,
    pub task_id: String,
    pub result_type: TaskResultType,
    pub content: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl TaskResult {
    /// Create a new task result
    pub fn new(task_id: String, result_type: TaskResultType, content: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            task_id,
            result_type,
            content,
            created_at: Utc::now(),
        }
    }
}
