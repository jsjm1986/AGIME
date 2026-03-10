//! Team Agent model
//! Agents belong to teams and execute tasks submitted by members

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Agent status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    /// Agent is idle, ready to accept tasks
    #[default]
    Idle,
    /// Agent is currently running a task
    Running,
    /// Agent is paused
    Paused,
    /// Agent encountered an error
    Error,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Running => write!(f, "running"),
            Self::Paused => write!(f, "paused"),
            Self::Error => write!(f, "error"),
        }
    }
}

impl std::str::FromStr for AgentStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "idle" => Ok(Self::Idle),
            "running" => Ok(Self::Running),
            "paused" => Ok(Self::Paused),
            "error" => Ok(Self::Error),
            _ => Err(format!("Invalid agent status: {}", s)),
        }
    }
}

/// API format compatibility mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ApiFormat {
    #[default]
    OpenAI,
    Anthropic,
    Local,
}

impl std::fmt::Display for ApiFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenAI => write!(f, "openai"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::Local => write!(f, "local"),
        }
    }
}

impl std::str::FromStr for ApiFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(Self::OpenAI),
            "anthropic" => Ok(Self::Anthropic),
            "local" => Ok(Self::Local),
            _ => Err(format!("Invalid api format: {}", s)),
        }
    }
}

/// Built-in extension type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinExtension {
    // Platform extensions (in-process)
    Skills,
    Todo,
    ExtensionManager,
    Team,
    ChatRecall,
    DocumentTools,
    // Builtin MCP servers (subprocess)
    Developer,
    Memory,
    ComputerController,
    AutoVisualiser,
    Tutorial,
}

impl BuiltinExtension {
    /// Get all available extensions
    pub fn all() -> Vec<Self> {
        vec![
            Self::Skills,
            Self::Todo,
            Self::ExtensionManager,
            Self::Team,
            Self::ChatRecall,
            Self::DocumentTools,
            Self::Developer,
            Self::Memory,
            Self::ComputerController,
            Self::AutoVisualiser,
            Self::Tutorial,
        ]
    }

    /// Get default enabled extensions
    pub fn defaults() -> Vec<Self> {
        vec![
            Self::Skills,
            Self::Todo,
            Self::Developer,
            Self::ExtensionManager,
            Self::DocumentTools,
        ]
    }

    /// Get extension name (snake_case, consistent with serde serialization)
    pub fn name(&self) -> &'static str {
        match self {
            Self::Skills => "skills",
            Self::Todo => "todo",
            Self::ExtensionManager => "extension_manager",
            Self::Team => "team",
            Self::ChatRecall => "chat_recall",
            Self::DocumentTools => "document_tools",
            Self::Developer => "developer",
            Self::Memory => "memory",
            Self::ComputerController => "computer_controller",
            Self::AutoVisualiser => "auto_visualiser",
            Self::Tutorial => "tutorial",
        }
    }

    /// Get MCP subprocess command name (used for `agime mcp <name>`)
    /// These are the names registered in the agime binary's MCP subcommand.
    pub fn mcp_name(&self) -> Option<&'static str> {
        match self {
            Self::Developer => Some("developer"),
            Self::Memory => Some("memory"),
            Self::ComputerController => Some("computercontroller"),
            Self::AutoVisualiser => Some("autovisualiser"),
            Self::Tutorial => Some("tutorial"),
            _ => None, // Platform extensions don't use subprocess
        }
    }

    /// Get extension description
    pub fn description(&self) -> &'static str {
        match self {
            Self::Skills => "Load and use skills",
            Self::Todo => "Task tracking",
            Self::ExtensionManager => "Extension management",
            Self::Team => "Team collaboration",
            Self::ChatRecall => "Conversation memory",
            Self::DocumentTools => "Read, create, search and list team documents",
            Self::Developer => "File editing and shell commands",
            Self::Memory => "Knowledge base",
            Self::ComputerController => "Computer control",
            Self::AutoVisualiser => "Auto visualization",
            Self::Tutorial => "Tutorials",
        }
    }

    /// Check if this is a platform extension (in-process)
    pub fn is_platform(&self) -> bool {
        matches!(
            self,
            Self::Skills
                | Self::Todo
                | Self::ExtensionManager
                | Self::Team
                | Self::ChatRecall
                | Self::DocumentTools
        )
    }
}

/// Agent extension configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExtensionConfig {
    pub extension: BuiltinExtension,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Custom extension configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomExtensionConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub ext_type: String,
    pub uri_or_cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing)]
    pub envs: std::collections::HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Source of this extension: "team" for team shared extensions, None for manually added
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The original shared extension ID (for team-sourced extensions)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_extension_id: Option<String>,
}

/// Skill assigned to an agent from team shared skills
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkillConfig {
    /// MongoDB ObjectId (hex) of the shared skill
    pub skill_id: String,
    /// Skill name (denormalized for display)
    pub name: String,
    /// Description (denormalized)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether this skill is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Version at assignment time
    #[serde(default)]
    pub version: String,
}

/// Team Agent entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamAgent {
    pub id: String,
    pub team_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Preset icon avatar identifier (e.g. "bot", "brain", "rocket")
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub avatar: Option<String>,
    /// System prompt that defines the agent's behavior and personality
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// API endpoint URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    /// Model name (e.g., "claude-3-opus", "gpt-4")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// API key (stored encrypted, never returned in responses)
    #[serde(skip_serializing, default)]
    pub api_key: Option<String>,
    /// API format compatibility mode
    #[serde(default)]
    pub api_format: ApiFormat,
    /// Enabled extensions (JSON array of extension configs)
    #[serde(default)]
    pub enabled_extensions: Vec<AgentExtensionConfig>,
    /// Custom extensions added by user
    #[serde(default)]
    pub custom_extensions: Vec<CustomExtensionConfig>,
    /// Agent domain: general / digital_avatar / ecosystem_portal
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_domain: Option<String>,
    /// Dedicated role inside a domain: manager / service
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_role: Option<String>,
    /// Owning manager agent for dedicated service agents
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub owner_manager_agent_id: Option<String>,
    /// Original template agent this agent was provisioned from
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub template_source_agent_id: Option<String>,
    pub status: AgentStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Group IDs allowed to use this agent (empty = all members can use)
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    /// Max concurrent tasks for this agent
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_tasks: u32,
    /// LLM temperature (0.0 - 1.0). None uses provider default.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub temperature: Option<f32>,
    /// Maximum output tokens per LLM call. None uses provider default.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_tokens: Option<i32>,
    /// Context window limit override. None uses model default.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_limit: Option<usize>,
    /// Skills assigned from team shared skills
    #[serde(default)]
    pub assigned_skills: Vec<AgentSkillConfig>,
    /// Auto-approve chat tasks (skip manual approval for chat messages).
    /// SECURITY: Only team admins/owners can modify this field via update_agent API.
    /// When true, chat-type tasks are approved immediately upon submission.
    #[serde(default = "default_auto_approve_chat")]
    pub auto_approve_chat: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_max_concurrent() -> u32 {
    1
}

fn default_auto_approve_chat() -> bool {
    true
}

impl TeamAgent {
    /// Create a new team agent
    pub fn new(team_id: String, name: String) -> Self {
        let now = Utc::now();
        // Default enabled extensions
        let enabled_extensions = BuiltinExtension::defaults()
            .into_iter()
            .map(|ext| AgentExtensionConfig {
                extension: ext,
                enabled: true,
            })
            .collect();
        Self {
            id: Uuid::new_v4().to_string(),
            team_id,
            name,
            description: None,
            avatar: None,
            system_prompt: None,
            api_url: None,
            model: None,
            api_key: None,
            api_format: ApiFormat::default(),
            enabled_extensions,
            custom_extensions: vec![],
            agent_domain: None,
            agent_role: None,
            owner_manager_agent_id: None,
            template_source_agent_id: None,
            status: AgentStatus::Idle,
            last_error: None,
            allowed_groups: vec![],
            max_concurrent_tasks: 1,
            temperature: None,
            max_tokens: None,
            context_limit: None,
            assigned_skills: vec![],
            auto_approve_chat: true,
            created_at: now,
            updated_at: now,
        }
    }

    /// Set description
    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }

    /// Set API configuration
    pub fn with_api(mut self, api_url: String, model: String, api_format: ApiFormat) -> Self {
        self.api_url = Some(api_url);
        self.model = Some(model);
        self.api_format = api_format;
        self
    }
}

/// Request to create a team agent
#[derive(Debug, Clone, Deserialize)]
pub struct CreateAgentRequest {
    pub team_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub api_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_format: Option<String>,
    #[serde(default)]
    pub enabled_extensions: Option<Vec<AgentExtensionConfig>>,
    #[serde(default)]
    pub custom_extensions: Option<Vec<CustomExtensionConfig>>,
    #[serde(default)]
    pub agent_domain: Option<String>,
    #[serde(default)]
    pub agent_role: Option<String>,
    #[serde(default)]
    pub owner_manager_agent_id: Option<String>,
    #[serde(default)]
    pub template_source_agent_id: Option<String>,
    #[serde(default)]
    pub allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    pub max_concurrent_tasks: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<i32>,
    #[serde(default)]
    pub context_limit: Option<usize>,
    #[serde(default)]
    pub assigned_skills: Option<Vec<AgentSkillConfig>>,
}

/// Request to update a team agent
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateAgentRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub api_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_format: Option<String>,
    #[serde(default)]
    pub status: Option<AgentStatus>,
    #[serde(default)]
    pub enabled_extensions: Option<Vec<AgentExtensionConfig>>,
    #[serde(default)]
    pub custom_extensions: Option<Vec<CustomExtensionConfig>>,
    #[serde(default)]
    pub agent_domain: Option<String>,
    #[serde(default)]
    pub agent_role: Option<String>,
    #[serde(default)]
    pub owner_manager_agent_id: Option<String>,
    #[serde(default)]
    pub template_source_agent_id: Option<String>,
    #[serde(default)]
    pub allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    pub max_concurrent_tasks: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<i32>,
    #[serde(default)]
    pub context_limit: Option<usize>,
    #[serde(default)]
    pub assigned_skills: Option<Vec<AgentSkillConfig>>,
    #[serde(default)]
    pub auto_approve_chat: Option<bool>,
}

/// Agent list query parameters
#[derive(Debug, Clone, Deserialize)]
pub struct ListAgentsQuery {
    pub team_id: String,
    #[serde(default = "super::default_page")]
    pub page: u32,
    #[serde(default = "super::default_limit")]
    pub limit: u32,
}
