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
    SkillRegistry,
    #[serde(rename = "tasks", alias = "todo")]
    Tasks,
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
            Self::SkillRegistry,
            Self::Tasks,
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
            Self::Tasks,
            Self::Developer,
            Self::ExtensionManager,
            Self::DocumentTools,
        ]
    }

    /// Get extension name (snake_case, consistent with serde serialization)
    pub fn name(&self) -> &'static str {
        match self {
            Self::Skills => "skills",
            Self::SkillRegistry => "skill_registry",
            Self::Tasks => "tasks",
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
            Self::SkillRegistry => "Discover and import remote skills",
            Self::Tasks => "Structured task tracking",
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
                | Self::SkillRegistry
                | Self::Tasks
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

/// Skill binding semantics for an agent runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillBindingMode {
    AssignedOnly,
    #[default]
    Hybrid,
    OnDemandOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    #[default]
    LeaderOwned,
    HeadlessFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DelegationPolicy {
    #[serde(default = "default_true")]
    pub allow_plan: bool,
    #[serde(default = "default_true")]
    pub allow_subagent: bool,
    #[serde(default = "default_true")]
    pub allow_swarm: bool,
    #[serde(default = "default_true")]
    pub allow_worker_messaging: bool,
    #[serde(default = "default_true")]
    pub allow_auto_swarm: bool,
    #[serde(default = "default_true")]
    pub allow_validation_worker: bool,
    #[serde(default)]
    pub approval_mode: ApprovalMode,
    #[serde(default = "default_subagent_depth")]
    pub max_subagent_depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism_budget: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swarm_budget: Option<u32>,
    #[serde(default)]
    pub require_final_report: bool,
}

impl Default for DelegationPolicy {
    fn default() -> Self {
        Self {
            allow_plan: true,
            allow_subagent: true,
            allow_swarm: true,
            allow_worker_messaging: true,
            allow_auto_swarm: true,
            allow_validation_worker: true,
            approval_mode: ApprovalMode::LeaderOwned,
            max_subagent_depth: default_subagent_depth(),
            parallelism_budget: None,
            swarm_budget: None,
            require_final_report: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DelegationPolicyOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_plan: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_subagent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_swarm: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_worker_messaging: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_auto_swarm: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_validation_worker: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_mode: Option<ApprovalMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_subagent_depth: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism_budget: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swarm_budget: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_final_report: Option<bool>,
}

impl DelegationPolicy {
    pub fn apply_override(&self, override_policy: Option<&DelegationPolicyOverride>) -> Self {
        let Some(override_policy) = override_policy else {
            return self.clone();
        };

        let allow_plan = override_policy.allow_plan.unwrap_or(self.allow_plan);
        let allow_subagent = override_policy
            .allow_subagent
            .unwrap_or(self.allow_subagent);
        let allow_swarm = override_policy.allow_swarm.unwrap_or(self.allow_swarm);
        let allow_worker_messaging = override_policy
            .allow_worker_messaging
            .unwrap_or(self.allow_worker_messaging)
            && allow_swarm;
        let allow_auto_swarm = override_policy
            .allow_auto_swarm
            .unwrap_or(self.allow_auto_swarm)
            && allow_swarm;
        let allow_validation_worker = override_policy
            .allow_validation_worker
            .unwrap_or(self.allow_validation_worker);
        let approval_mode = override_policy.approval_mode.unwrap_or(self.approval_mode);
        let max_subagent_depth = override_policy
            .max_subagent_depth
            .unwrap_or(self.max_subagent_depth);
        let parallelism_budget =
            narrow_optional_u32(self.parallelism_budget, override_policy.parallelism_budget);
        let swarm_budget = narrow_optional_u32(self.swarm_budget, override_policy.swarm_budget);
        let require_final_report = override_policy
            .require_final_report
            .unwrap_or(self.require_final_report);

        Self {
            allow_plan,
            allow_subagent,
            allow_swarm,
            allow_worker_messaging,
            allow_auto_swarm,
            allow_validation_worker,
            approval_mode,
            max_subagent_depth,
            parallelism_budget,
            swarm_budget,
            require_final_report,
        }
    }
}

fn narrow_optional_u32(base: Option<u32>, override_value: Option<u32>) -> Option<u32> {
    match (base, override_value) {
        (Some(base), Some(override_value)) => Some(base.min(override_value)),
        (Some(base), None) => Some(base),
        (None, Some(override_value)) => Some(override_value),
        (None, None) => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachedTeamExtensionRef {
    pub extension_id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
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
    /// Whether think/reasoning mode should be enabled for this agent by default.
    /// Unsupported models automatically fall back to normal mode.
    #[serde(default = "default_thinking_enabled")]
    pub thinking_enabled: bool,
    /// Skills assigned from team shared skills
    #[serde(default)]
    pub assigned_skills: Vec<AgentSkillConfig>,
    /// Skill exposure semantics for runtime use.
    #[serde(default)]
    pub skill_binding_mode: SkillBindingMode,
    /// Agent-level Harness delegation policy.
    #[serde(default)]
    pub delegation_policy: DelegationPolicy,
    /// References to team shared extensions attached to this agent.
    #[serde(default)]
    pub attached_team_extensions: Vec<AttachedTeamExtensionRef>,
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

fn default_thinking_enabled() -> bool {
    true
}

fn default_auto_approve_chat() -> bool {
    true
}

fn default_subagent_depth() -> u32 {
    1
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
            thinking_enabled: true,
            assigned_skills: vec![],
            skill_binding_mode: SkillBindingMode::Hybrid,
            delegation_policy: DelegationPolicy::default(),
            attached_team_extensions: vec![],
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

#[cfg(test)]
mod tests {
    use super::BuiltinExtension;

    #[test]
    fn builtin_extension_accepts_legacy_todo_alias() {
        let parsed: BuiltinExtension = serde_json::from_str("\"todo\"").expect("parse legacy");
        assert_eq!(parsed, BuiltinExtension::Tasks);
        assert_eq!(
            serde_json::to_string(&parsed).expect("serialize"),
            "\"tasks\""
        );
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
    pub thinking_enabled: Option<bool>,
    #[serde(default)]
    pub assigned_skills: Option<Vec<AgentSkillConfig>>,
    #[serde(default)]
    pub skill_binding_mode: Option<SkillBindingMode>,
    #[serde(default)]
    pub delegation_policy: Option<DelegationPolicy>,
    #[serde(default)]
    pub attached_team_extensions: Option<Vec<AttachedTeamExtensionRef>>,
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
    pub thinking_enabled: Option<bool>,
    #[serde(default)]
    pub assigned_skills: Option<Vec<AgentSkillConfig>>,
    #[serde(default)]
    pub skill_binding_mode: Option<SkillBindingMode>,
    #[serde(default)]
    pub delegation_policy: Option<DelegationPolicy>,
    #[serde(default)]
    pub attached_team_extensions: Option<Vec<AttachedTeamExtensionRef>>,
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
