//! Task executor for running agent tasks (MongoDB version)
//!
//! This module provides the TaskExecutor which executes approved tasks
//! using the agime Provider abstraction layer for unified LLM access.

use agime::agents::extension::ExtensionInfo;
use agime::agents::final_output_tool::{
    FinalOutputTool, FINAL_OUTPUT_CONTINUATION_MESSAGE, FINAL_OUTPUT_TOOL_NAME,
};
use agime::agents::types::{
    RetryConfig, SuccessCheck, DEFAULT_ON_FAILURE_TIMEOUT_SECONDS, DEFAULT_RETRY_TIMEOUT_SECONDS,
};
use agime::context_mgmt::{
    compact_messages_with_strategy, ContextCompactionStrategy, DEFAULT_COMPACTION_THRESHOLD,
};
use agime::conversation::message::{Message, MessageContent};
use agime::conversation::{fix_conversation, Conversation};
use agime::prompt_template;
use agime::providers::base::{Provider, ProviderUsage};
use agime::providers::errors::ProviderError;
use agime::recipe::Response;
use agime::security::scanner::PromptInjectionScanner;
use agime::subprocess::configure_command_no_window;
use agime::token_counter::create_token_counter;
use agime_team::models::{
    AgentTask, ApiFormat, BuiltinExtension, CustomExtensionConfig, TaskResultType, TaskStatus,
    TeamAgent,
};
use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use chrono::{Local, Utc};
use futures::future::join_all;
use futures::StreamExt;
use mongodb::bson::{doc, Document};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::process::{Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::context_injector::DocumentContextInjector;
use super::extension_installer::{AutoInstallPolicy, ExtensionInstaller};
use super::extension_manager_client::{DynamicExtensionState, TeamExtensionManagerClient};
use super::mcp_connector::{ApiCaller, McpConnector};
use super::platform_runner::PlatformExtensionRunner;
use super::provider_factory;
use super::resource_access::is_runtime_resource_allowed;
use super::service_mongo::{AgentService, AgentTaskDoc, TeamAgentDoc};
use super::session_mongo::CreateSessionRequest;
use super::task_manager::{StreamEvent, TaskManager};

/// Build an HTTP client that respects system proxy settings.
/// On Windows, reads proxy from HTTPS_PROXY/HTTP_PROXY env vars,
/// and falls back to reading the Windows registry proxy settings.
pub(crate) fn build_http_client() -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(std::time::Duration::from_secs(120));

    // Check env vars first (reqwest reads these automatically),
    // but also check Windows registry as fallback
    if std::env::var("HTTPS_PROXY").is_err() && std::env::var("HTTP_PROXY").is_err() {
        if let Some(proxy_url) = detect_system_proxy() {
            tracing::info!("Using system proxy: {}", proxy_url);
            let proxy =
                reqwest::Proxy::all(&proxy_url).map_err(|e| anyhow!("Invalid proxy URL: {}", e))?;
            builder = builder.proxy(proxy);
        }
    }

    builder
        .build()
        .map_err(|e| anyhow!("Failed to build HTTP client: {}", e))
}

/// Detect system proxy on Windows from registry
#[cfg(target_os = "windows")]
pub(crate) fn detect_system_proxy() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let settings = hkcu
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
        .ok()?;
    let enabled: u32 = settings.get_value("ProxyEnable").ok()?;
    if enabled == 0 {
        return None;
    }
    let server: String = settings.get_value("ProxyServer").ok()?;
    if server.is_empty() {
        return None;
    }
    // Ensure it has a scheme
    if server.starts_with("http://")
        || server.starts_with("https://")
        || server.starts_with("socks5://")
    {
        Some(server)
    } else {
        Some(format!("http://{}", server))
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn detect_system_proxy() -> Option<String> {
    None
}

/// Max allowed tool timeout from env (2 hours).
const MAX_TOOL_TIMEOUT_SECS: u64 = 7200;

/// Max allowed turns from env to prevent runaway.
const MAX_UNIFIED_MAX_TURNS: usize = 5000;

/// Maximum characters for a single tool result before truncation
const MAX_TOOL_RESULT_CHARS: usize = 32_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TeamResourceMode {
    Explicit,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TeamSkillMode {
    Assigned,
    OnDemand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TeamAutoExtensionPolicy {
    ReviewedOnly,
    All,
}

#[derive(Debug, Clone)]
struct TeamRuntimeSettings {
    resource_mode: TeamResourceMode,
    skill_mode: TeamSkillMode,
    auto_extension_policy: TeamAutoExtensionPolicy,
    auto_install_extensions: AutoInstallPolicy,
    extension_cache_root: String,
    portal_base_url: String,
    workspace_root: String,
}

impl TeamRuntimeSettings {
    fn from_env() -> Self {
        let resource_mode = match std::env::var("TEAM_AGENT_RESOURCE_MODE")
            .unwrap_or_else(|_| "explicit".to_string())
            .to_lowercase()
            .as_str()
        {
            "auto" => TeamResourceMode::Auto,
            _ => TeamResourceMode::Explicit,
        };
        let skill_mode = match std::env::var("TEAM_AGENT_SKILL_MODE")
            .unwrap_or_else(|_| "assigned".to_string())
            .to_lowercase()
            .as_str()
        {
            "on_demand" | "ondemand" => TeamSkillMode::OnDemand,
            _ => TeamSkillMode::Assigned,
        };
        let auto_extension_policy = match std::env::var("TEAM_AGENT_AUTO_EXTENSION_POLICY")
            .unwrap_or_else(|_| "reviewed_only".to_string())
            .to_lowercase()
            .as_str()
        {
            "all" => TeamAutoExtensionPolicy::All,
            _ => TeamAutoExtensionPolicy::ReviewedOnly,
        };
        let auto_install_extensions = if std::env::var("TEAM_AGENT_AUTO_INSTALL_EXTENSIONS")
            .map(|s| s.eq_ignore_ascii_case("true"))
            .unwrap_or(true)
        {
            AutoInstallPolicy::Enabled
        } else {
            AutoInstallPolicy::Disabled
        };
        let extension_cache_root = std::env::var("TEAM_AGENT_EXTENSION_CACHE_ROOT")
            .unwrap_or_else(|_| "./data/runtime/extensions".to_string());
        let portal_base_url = std::env::var("PORTAL_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
        let workspace_root =
            std::env::var("WORKSPACE_ROOT").unwrap_or_else(|_| "./data/workspaces".to_string());

        Self {
            resource_mode,
            skill_mode,
            auto_extension_policy,
            auto_install_extensions,
            extension_cache_root,
            portal_base_url,
            workspace_root,
        }
    }
}

/// Mission-specific context injected into the system prompt when executing mission steps.
#[derive(Clone, serde::Deserialize)]
pub struct MissionPromptContext {
    pub goal: String,
    pub context: Option<String>,
    pub approval_policy: String,
    pub total_steps: usize,
    pub current_step: usize,
}

/// Context for rendering the system.md prompt template.
/// Mirrors the local agent's SystemPromptContext but simplified for team server use.
#[derive(Serialize)]
struct TeamSystemPromptContext {
    extensions: Vec<ExtensionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_selection_strategy: Option<String>,
    current_date_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    extension_tool_limits: Option<(usize, usize)>,
    agime_mode: String,
    is_autonomous: bool,
    enable_subagents: bool,
    max_extensions: usize,
    max_tools: usize,
}

/// Build the system prompt: core template + optional agent custom instructions.
///
/// The core system.md template is ALWAYS rendered as the base, ensuring identity,
/// behavioral rules, safety guardrails, and tool usage guidelines are never lost.
/// If the agent has a custom `system_prompt`, it is appended as `<agent_instructions>`
/// rather than replacing the core template.
fn build_system_prompt(
    extensions: &[ExtensionInfo],
    custom_prompt: Option<&str>,
    mission_context: Option<&MissionPromptContext>,
) -> String {
    let (mode, autonomous) = match mission_context {
        Some(mc) => ("mission".to_string(), mc.approval_policy == "auto"),
        None => ("chat".to_string(), false),
    };

    let context = TeamSystemPromptContext {
        extensions: extensions.to_vec(),
        tool_selection_strategy: None,
        current_date_time: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        extension_tool_limits: None,
        agime_mode: mode,
        is_autonomous: autonomous,
        enable_subagents: false,
        max_extensions: 5,
        max_tools: 50,
    };

    let mut prompt = match prompt_template::render_global_file("system.md", &context) {
        Ok(rendered) => rendered,
        Err(e) => {
            tracing::warn!("Failed to render system.md template: {}, using fallback", e);
            "You are a helpful AI assistant. Answer the user's questions accurately and concisely."
                .to_string()
        }
    };

    // Append agent-specific custom instructions (never replace core template)
    if let Some(custom) = custom_prompt {
        prompt.push_str("\n\n<agent_instructions>\n");
        prompt.push_str("The following are custom instructions configured for this agent. ");
        prompt.push_str("Follow these instructions while maintaining all core behavioral rules and safety guardrails above.\n\n");
        prompt.push_str(custom);
        prompt.push_str("\n</agent_instructions>");
    }

    // Append mission context when executing mission steps
    if let Some(mc) = mission_context {
        prompt.push_str("\n\n<mission_context>\n");
        prompt.push_str("You are executing a multi-step mission autonomously.\n\n");
        prompt.push_str(&format!("## Mission Goal\n{}\n", mc.goal));
        if let Some(ref ctx) = mc.context {
            prompt.push_str(&format!("\n## Additional Context\n{}\n", ctx));
        }
        prompt.push_str("\n## Execution Rules\n");
        prompt.push_str("- You are in AUTONOMOUS execution mode. Complete each step without asking questions.\n");
        prompt.push_str("- Focus on the current step. Do not skip ahead or revisit completed steps.\n");
        prompt.push_str("- If a step cannot be completed, explain what went wrong clearly.\n");
        prompt.push_str("- Verify your work before reporting completion.\n");
        prompt.push_str("- Be concise in your output — your response will be saved as step summary.\n");
        prompt.push_str(&format!(
            "\n## Progress\nStep {}/{} — Approval policy: {}\n",
            mc.current_step, mc.total_steps, mc.approval_policy
        ));
        prompt.push_str("</mission_context>");
    }

    prompt
}

/// Check if the agent has ExtensionManager enabled in its configuration.
fn agent_has_extension_manager_enabled(agent: &TeamAgent) -> bool {
    agent
        .enabled_extensions
        .iter()
        .any(|ext| ext.enabled && matches!(ext.extension, BuiltinExtension::ExtensionManager))
}

/// Convert enabled built-in extensions to CustomExtensionConfig entries.
/// Subprocess extensions (developer, memory, etc.) are started via `agime mcp <name>`.
/// Platform extensions (skills, todo, etc.) run in-process via PlatformExtensionRunner.
fn builtin_extensions_to_custom(agent: &TeamAgent) -> Vec<CustomExtensionConfig> {
    let agime_bin = find_agime_binary();
    let mut configs = Vec::new();

    for ext_config in &agent.enabled_extensions {
        if !ext_config.enabled {
            continue;
        }

        // Developer runs in-process via PlatformExtensionRunner
        if ext_config.extension == BuiltinExtension::Developer {
            continue;
        }

        // Only subprocess extensions can be started as MCP servers
        let mcp_name = match ext_config.extension.mcp_name() {
            Some(name) => name,
            None => {
                tracing::debug!(
                    "Skipping platform extension {:?} (not supported as subprocess)",
                    ext_config.extension
                );
                continue;
            }
        };

        if let Some(ref bin) = agime_bin {
            configs.push(CustomExtensionConfig {
                name: mcp_name.to_string(),
                ext_type: "stdio".to_string(),
                uri_or_cmd: bin.clone(),
                args: vec!["mcp".to_string(), mcp_name.to_string()],
                envs: HashMap::new(),
                enabled: true,
                source: None,
                source_extension_id: None,
            });
            tracing::info!(
                "Registered builtin extension '{}' as stdio MCP server",
                mcp_name
            );
        } else {
            tracing::warn!(
                "Cannot start builtin extension '{}': agime binary not found",
                mcp_name
            );
        }
    }

    configs
}

/// Find an extension config by name from the agent's full configuration
/// (including disabled extensions). Used to re-enable session extension overrides.
fn find_extension_config_by_name(agent: &TeamAgent, name: &str) -> Option<CustomExtensionConfig> {
    // Check custom extensions first (including disabled ones)
    if let Some(custom) = agent.custom_extensions.iter().find(|e| e.name == name) {
        let mut cfg = custom.clone();
        cfg.enabled = true;
        return Some(cfg);
    }

    // Check builtin extensions (use builtin_to_custom_config from extension_manager_client)
    // Match against both name() (snake_case API name) and mcp_name() (subprocess runtime name)
    for ext_config in &agent.enabled_extensions {
        let matches =
            ext_config.extension.name() == name || ext_config.extension.mcp_name() == Some(name);
        if matches {
            if let Some(cfg) =
                super::extension_manager_client::builtin_to_custom_config(&ext_config.extension)
            {
                return Some(cfg);
            }
        }
    }

    None
}

/// Find the agime binary path for starting subprocess MCP servers.
pub(super) fn find_agime_binary() -> Option<String> {
    // 1. Prefer current executable (agime-team-server now supports `mcp` subcommand)
    if let Ok(exe) = std::env::current_exe() {
        if exe.exists() {
            let is_supported_self = exe
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| matches!(s, "agime-team-server" | "agime" | "agime-cli"))
                .unwrap_or(false);
            if is_supported_self {
                return Some(exe.to_string_lossy().to_string());
            }
        }

        // Fallback: look for sibling agime/agime-cli binaries
        if let Some(dir) = exe.parent() {
            let agime_path = dir.join(if cfg!(windows) { "agime.exe" } else { "agime" });
            if agime_path.exists() {
                return Some(agime_path.to_string_lossy().to_string());
            }
            let cli_path = dir.join(if cfg!(windows) {
                "agime-cli.exe"
            } else {
                "agime-cli"
            });
            if cli_path.exists() {
                return Some(cli_path.to_string_lossy().to_string());
            }
        }
    }

    // 2. Try PATH
    if let Ok(output) = std::process::Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg("agime")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path.lines().next().unwrap_or(&path).to_string());
            }
        }
    }

    None
}

/// Load team shared extensions according to policy.
/// Returns empty Vec on failure (does not block task execution).
async fn load_team_shared_extensions(
    db: &MongoDb,
    team_id: &str,
    policy: TeamAutoExtensionPolicy,
    installer: &ExtensionInstaller,
) -> Vec<CustomExtensionConfig> {
    use agime_team::services::mongo::extension_service_mongo::ExtensionService;

    let ext_service = ExtensionService::new(db.clone());
    let extensions = match policy {
        TeamAutoExtensionPolicy::ReviewedOnly => ext_service.list_reviewed_for_team(team_id).await,
        TeamAutoExtensionPolicy::All => ext_service.list_active_for_team(team_id).await,
    };
    let extensions = match extensions {
        Ok(exts) => exts,
        Err(e) => {
            tracing::warn!(
                "Failed to load team shared extensions for team {}: {}",
                team_id,
                e
            );
            return Vec::new();
        }
    };

    let mut configs = Vec::new();
    for ext in extensions {
        if !is_runtime_resource_allowed(&ext.visibility, &ext.protection_level) {
            tracing::debug!(
                "Skipping team extension '{}' due to runtime visibility/protection policy (visibility={}, protection_level={})",
                ext.name,
                ext.visibility,
                ext.protection_level
            );
            continue;
        }

        match installer.resolve_team_extension(team_id, &ext).await {
            Ok(cfg) => {
                tracing::info!(
                    "Loaded team shared extension '{}' (type={}, policy={:?})",
                    ext.name,
                    ext.extension_type,
                    policy
                );
                configs.push(cfg);
            }
            Err(e) => {
                tracing::warn!(
                    "Skipping team extension '{}' due to runtime resolve error: {}",
                    ext.name,
                    e
                );
            }
        }
    }

    configs
}

/// Load content of skills assigned to an agent.
/// Returns Vec<(name, content)> for injection into system prompt.
/// Failures are logged and skipped (does not block task execution).
/// Maximum total size (in bytes) for injected skill content to prevent oversized system prompts.
const MAX_SKILLS_CONTENT_SIZE: usize = 50 * 1024; // 50 KB

async fn load_assigned_skills_content(
    db: &MongoDb,
    agent: &TeamAgent,
    allowed_skill_ids: Option<&HashSet<String>>,
) -> Vec<(String, String)> {
    use agime_team::services::mongo::skill_service_mongo::SkillService;

    let skill_service = SkillService::new(db.clone());
    let mut results = Vec::new();
    let mut total_size: usize = 0;

    for skill_config in &agent.assigned_skills {
        if !skill_config.enabled {
            continue;
        }
        if let Some(allowed) = allowed_skill_ids {
            if !allowed.contains(&skill_config.skill_id) {
                continue;
            }
        }

        match skill_service.get(&skill_config.skill_id).await {
            Ok(Some(skill)) => {
                // Prefer skill_md, fall back to content
                let content = skill.skill_md.or(skill.content).unwrap_or_default();
                if !content.is_empty() {
                    // Check total size limit
                    if total_size + content.len() > MAX_SKILLS_CONTENT_SIZE {
                        tracing::warn!(
                            "Skill content size limit reached ({} bytes), skipping remaining skills",
                            total_size
                        );
                        break;
                    }
                    total_size += content.len();
                    results.push((skill_config.name.clone(), content));
                }
            }
            Ok(None) => {
                tracing::warn!(
                    "Assigned skill '{}' (id={}) not found, skipping",
                    skill_config.name,
                    skill_config.skill_id
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to load assigned skill '{}': {}, skipping",
                    skill_config.name,
                    e
                );
            }
        }
    }

    if !results.is_empty() {
        tracing::info!(
            "Loaded {} assigned skills ({} bytes) for agent",
            results.len(),
            total_size
        );
    }

    results
}

/// Detects repeated identical tool calls to prevent infinite loops
struct RepetitionDetector {
    last_call: Option<(String, String)>, // (name, args_json)
    repeat_count: u32,
}

impl RepetitionDetector {
    fn new() -> Self {
        Self {
            last_call: None,
            repeat_count: 0,
        }
    }

    /// Check if a tool call is allowed. Returns false if repeated 3+ times consecutively.
    fn check(&mut self, name: &str, args: &serde_json::Value) -> bool {
        let args_json = serde_json::to_string(args).unwrap_or_default();
        let current = (name.to_string(), args_json);
        if self.last_call.as_ref() == Some(&current) {
            self.repeat_count += 1;
            self.repeat_count < 3
        } else {
            self.last_call = Some(current);
            self.repeat_count = 1;
            true
        }
    }
}

/// ApiCaller adapter that holds agent config for MCP Sampling
struct AgentApiCaller {
    api_url: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    api_format: ApiFormat,
}

impl ApiCaller for AgentApiCaller {
    fn call_llm<'a>(
        &'a self,
        system: &'a str,
        messages: Vec<serde_json::Value>,
        max_tokens: u32,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value>> + Send + 'a>>
    {
        Box::pin(async move {
            let client = build_http_client()?;
            match self.api_format {
                ApiFormat::Anthropic => {
                    self.call_anthropic(client, system, &messages, max_tokens)
                        .await
                }
                ApiFormat::OpenAI => {
                    self.call_openai(client, system, &messages, max_tokens)
                        .await
                }
                ApiFormat::Local => Err(anyhow!("Local API does not support MCP Sampling")),
            }
        })
    }
}

impl AgentApiCaller {
    async fn call_anthropic(
        &self,
        client: reqwest::Client,
        system: &str,
        messages: &[serde_json::Value],
        max_tokens: u32,
    ) -> Result<serde_json::Value> {
        let base_url = self
            .api_url
            .as_deref()
            .unwrap_or("https://api.anthropic.com");
        let model = self.model.as_deref().unwrap_or("claude-3-opus-20240229");
        let api_key = self
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow!("API key not configured"))?;

        let is_volcengine = base_url.contains("ark.cn-beijing.volces.com");

        let api_url = if base_url.ends_with("/messages") || base_url.ends_with("/v1/messages") {
            base_url.to_string()
        } else if base_url.ends_with("/v1") {
            format!("{}/messages", base_url)
        } else {
            format!("{}/v1/messages", base_url.trim_end_matches('/'))
        };

        let mut request = client
            .post(&api_url)
            .header("Content-Type", "application/json");

        if is_volcengine {
            request = request.header("Authorization", format!("Bearer {}", api_key));
        } else {
            request = request
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        }

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": messages,
        });
        if !system.is_empty() {
            body["system"] = serde_json::json!(system);
        }

        let response = request.json(&body).send().await?;
        if !response.status().is_success() {
            let error = response.text().await?;
            return Err(anyhow!("Anthropic API error: {}", error));
        }
        response
            .json()
            .await
            .map_err(|e| anyhow!("Parse error: {}", e))
    }

    async fn call_openai(
        &self,
        client: reqwest::Client,
        system: &str,
        messages: &[serde_json::Value],
        max_tokens: u32,
    ) -> Result<serde_json::Value> {
        let api_url = self
            .api_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1/chat/completions");
        let model = self.model.as_deref().unwrap_or("gpt-4");
        let api_key = self
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow!("API key not configured"))?;

        // Prepend system message if provided
        let mut all_messages = Vec::new();
        if !system.is_empty() {
            all_messages.push(serde_json::json!({"role": "system", "content": system}));
        }
        all_messages.extend(messages.iter().cloned());

        let body = serde_json::json!({
            "model": model,
            "messages": all_messages,
            "max_tokens": max_tokens,
        });

        let response = client
            .post(api_url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await?;
            return Err(anyhow!("OpenAI API error: {}", error));
        }
        response
            .json()
            .await
            .map_err(|e| anyhow!("Parse error: {}", e))
    }
}

/// Task executor for running agent tasks (MongoDB version)
pub struct TaskExecutor {
    db: Arc<MongoDb>,
    task_manager: Arc<TaskManager>,
    agent_service: Arc<AgentService>,
    security_scanner: PromptInjectionScanner,
    runtime_settings: TeamRuntimeSettings,
}

impl TaskExecutor {
    /// Create a new task executor
    pub fn new(db: Arc<MongoDb>, task_manager: Arc<TaskManager>) -> Self {
        let agent_service = Arc::new(AgentService::new(db.clone()));
        let runtime_settings = TeamRuntimeSettings::from_env();
        tracing::info!(
            "TaskExecutor runtime settings: resource_mode={:?}, skill_mode={:?}, auto_extension_policy={:?}, auto_install_extensions={:?}, cache_root={}, workspace_root={}",
            runtime_settings.resource_mode,
            runtime_settings.skill_mode,
            runtime_settings.auto_extension_policy,
            runtime_settings.auto_install_extensions,
            runtime_settings.extension_cache_root,
            runtime_settings.workspace_root,
        );
        Self {
            db,
            task_manager,
            agent_service,
            security_scanner: PromptInjectionScanner::new(),
            runtime_settings,
        }
    }

    fn tasks(&self) -> mongodb::Collection<AgentTaskDoc> {
        self.db.collection("agent_tasks")
    }

    fn agents(&self) -> mongodb::Collection<TeamAgentDoc> {
        self.db.collection("team_agents")
    }

    fn results(&self) -> mongodb::Collection<Document> {
        self.db.collection("agent_task_results")
    }

    /// Execute an approved task
    pub async fn execute_task(&self, task_id: &str, cancel_token: CancellationToken) -> Result<()> {
        // 1. Get task and agent info
        let task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| anyhow!("Task not found"))?;

        if task.status != TaskStatus::Approved {
            return Err(anyhow!("Task is not approved"));
        }

        let mut agent = self
            .get_agent(&task.agent_id)
            .await?
            .ok_or_else(|| anyhow!("Agent not found"))?;

        // Apply LLM overrides from task content (e.g. document analysis settings)
        if let Some(ov) = task.content.get("llm_overrides") {
            if let Some(u) = ov.get("api_url").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) { agent.api_url = Some(u.to_string()); }
            if let Some(k) = ov.get("api_key").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) { agent.api_key = Some(k.to_string()); }
            if let Some(m) = ov.get("model").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) { agent.model = Some(m.to_string()); }
            if let Some(f) = ov.get("api_format").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                if let Ok(fmt) = f.parse() { agent.api_format = fmt; }
            }
        }

        // Debug: Log API key status
        tracing::info!(
            "Executing task {} with agent {}, api_format={:?}, api_key_present={}",
            task_id,
            agent.id,
            agent.api_format,
            agent.api_key.is_some()
        );

        // 2. Update task status to running
        self.update_task_status(task_id, TaskStatus::Running)
            .await?;

        // Broadcast status change
        self.task_manager
            .broadcast(
                task_id,
                StreamEvent::Status {
                    status: "running".to_string(),
                },
            )
            .await;

        // 3. Execute the task (cancel_token passed into the loop for fine-grained checking)
        let result = self.run_task(task_id, &task, &agent, &cancel_token).await;

        // 4. Update task status based on result
        match result {
            Ok(_) => {
                self.update_task_status(task_id, TaskStatus::Completed)
                    .await?;
                self.task_manager
                    .broadcast(
                        task_id,
                        StreamEvent::Done {
                            status: "completed".to_string(),
                            error: None,
                        },
                    )
                    .await;
            }
            Err(e) => {
                let error_msg = e.to_string();
                // Don't propagate DB errors here — must always reach broadcast + complete below
                if let Err(db_err) = self.update_task_error(task_id, &error_msg).await {
                    tracing::error!("Failed to persist task error for {}: {}", task_id, db_err);
                }
                self.task_manager
                    .broadcast(
                        task_id,
                        StreamEvent::Done {
                            status: "failed".to_string(),
                            error: Some(error_msg),
                        },
                    )
                    .await;
            }
        }

        // 5. Complete task in manager
        self.task_manager.complete(task_id).await;

        Ok(())
    }

    /// Run the actual task execution with multi-turn agent loop
    async fn run_task(
        &self,
        task_id: &str,
        task: &AgentTask,
        agent: &TeamAgent,
        cancel_token: &CancellationToken,
    ) -> Result<()> {
        let user_messages = task
            .content
            .get("messages")
            .and_then(|m| m.as_array())
            .ok_or_else(|| anyhow!("Invalid task content: missing messages"))?;

        // Session management: load or create session
        let session_id = task
            .content
            .get("session_id")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());

        let (session, history_messages) = if let Some(ref sid) = session_id {
            // Load existing session
            match self.agent_service.get_session(sid).await {
                Ok(Some(sess)) => {
                    let msgs: Vec<Message> =
                        serde_json::from_str(&sess.messages_json).unwrap_or_default();
                    tracing::info!("Loaded session {} with {} messages", sid, msgs.len());
                    (Some(sess), msgs)
                }
                _ => {
                    tracing::warn!("Session {} not found, starting fresh", sid);
                    (None, Vec::new())
                }
            }
        } else {
            (None, Vec::new())
        };

        // Create session if none exists
        let session_id = if let Some(ref sess) = session {
            sess.session_id.clone()
        } else {
            let new_sess = self
                .agent_service
                .create_session(CreateSessionRequest {
                    team_id: task.team_id.clone(),
                    agent_id: task.agent_id.clone(),
                    user_id: task.submitter_id.clone(),
                    name: None,
                    attached_document_ids: Vec::new(),
                    extra_instructions: None,
                    allowed_extensions: None,
                    allowed_skill_ids: None,
                    retry_config: None,
                    max_turns: None,
                    tool_timeout_seconds: None,
                    max_portal_retry_rounds: None,
                    require_final_report: false,
                    portal_restricted: false,
                })
                .await
                .map_err(|e| anyhow!("Failed to create session: {}", e))?;
            let sid = new_sess.session_id.clone();
            tracing::info!("Created new session: {}", sid);
            sid
        };

        // Broadcast session_id to client
        self.task_manager
            .broadcast(
                task_id,
                StreamEvent::SessionId {
                    session_id: session_id.clone(),
                },
            )
            .await;

        // Get compaction strategy from task or default
        let compaction_strategy_name = task
            .content
            .get("compaction_strategy")
            .and_then(|s| s.as_str())
            .unwrap_or("cfpm_memory_v1")
            .to_string();

        // Build ApiCaller for MCP Sampling support
        let api_caller: Option<Arc<dyn ApiCaller>> = if agent.api_format != ApiFormat::Local {
            Some(Arc::new(AgentApiCaller {
                api_url: agent.api_url.clone(),
                api_key: agent.api_key.clone(),
                model: agent.model.clone(),
                api_format: agent.api_format,
            }))
        } else {
            None
        };

        let allowed_extension_names: Option<HashSet<String>> = session
            .as_ref()
            .and_then(|s| s.allowed_extensions.as_ref())
            .map(|items| {
                items
                    .iter()
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect::<HashSet<_>>()
            });
        let allowed_skill_ids: Option<HashSet<String>> = session
            .as_ref()
            .and_then(|s| s.allowed_skill_ids.as_ref())
            .map(|items| {
                items
                    .iter()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<HashSet<_>>()
            });

        let mut platform_enabled_extensions = agent.enabled_extensions.clone();
        if let Some(allowed) = &allowed_extension_names {
            platform_enabled_extensions.retain(|cfg| {
                let runtime_name = cfg
                    .extension
                    .mcp_name()
                    .unwrap_or_else(|| cfg.extension.name());
                if cfg.extension == BuiltinExtension::Skills
                    && self.runtime_settings.skill_mode == TeamSkillMode::OnDemand
                {
                    allowed.contains("skills") || allowed.contains("team_skills")
                } else {
                    allowed.contains(&runtime_name.to_lowercase())
                }
            });
        }

        // Connect to MCP extensions (builtin + custom + team shared)
        let mut all_extensions = builtin_extensions_to_custom(agent);
        all_extensions.extend(
            agent
                .custom_extensions
                .iter()
                .filter(|e| e.enabled)
                .cloned(),
        );

        // Merge team shared extensions (auto-discovery) when enabled.
        // Agent's own extensions take priority over team shared ones (skip duplicates by name).
        if self.runtime_settings.resource_mode == TeamResourceMode::Auto {
            let installer = ExtensionInstaller::new(
                self.db.clone(),
                self.runtime_settings.extension_cache_root.clone(),
                self.runtime_settings.auto_install_extensions,
            );
            let team_extensions = load_team_shared_extensions(
                &self.db,
                &task.team_id,
                self.runtime_settings.auto_extension_policy,
                &installer,
            )
            .await;
            if !team_extensions.is_empty() {
                let mut existing_names: HashSet<String> =
                    all_extensions.iter().map(|e| e.name.clone()).collect();
                for team_ext in team_extensions {
                    if !existing_names.contains(&team_ext.name) {
                        tracing::info!(
                            "Auto-discovered team extension '{}' for agent",
                            team_ext.name
                        );
                        existing_names.insert(team_ext.name.clone());
                        all_extensions.push(team_ext);
                    } else {
                        tracing::debug!(
                            "Skipping team extension '{}': already exists in agent config",
                            team_ext.name
                        );
                    }
                }
            }
        }

        // Apply session extension overrides (disabled/enabled)
        if let Some(ref sess) = session {
            if !sess.disabled_extensions.is_empty() {
                let disabled_set: HashSet<&str> = sess
                    .disabled_extensions
                    .iter()
                    .map(|s| s.as_str())
                    .collect();
                let before = all_extensions.len();
                all_extensions.retain(|e| !disabled_set.contains(e.name.as_str()));
                tracing::info!(
                    "Session extension overrides: disabled {} extensions ({} -> {})",
                    before - all_extensions.len(),
                    before,
                    all_extensions.len()
                );
            }
            // enabled_extensions: re-enable extensions from agent config that user
            // explicitly enabled during a previous message in this session
            if !sess.enabled_extensions.is_empty() {
                let existing_names: HashSet<String> =
                    all_extensions.iter().map(|e| e.name.clone()).collect();
                let mut added = 0usize;
                for enabled_name in &sess.enabled_extensions {
                    if existing_names.contains(enabled_name) {
                        continue; // already active
                    }
                    // Try to find in agent's disabled builtin extensions
                    if let Some(cfg) = find_extension_config_by_name(agent, enabled_name) {
                        tracing::info!(
                            "Re-enabling session extension override: '{}'",
                            enabled_name
                        );
                        all_extensions.push(cfg);
                        added += 1;
                    } else {
                        tracing::debug!(
                            "Session enabled extension '{}' not found in agent config, skipping",
                            enabled_name
                        );
                    }
                }
                if added > 0 {
                    tracing::info!(
                        "Session extension overrides: re-enabled {} extensions",
                        added
                    );
                }
            }
        }

        if let Some(allowed) = &allowed_extension_names {
            let before = all_extensions.len();
            all_extensions.retain(|ext| allowed.contains(&ext.name.to_lowercase()));
            tracing::info!(
                "Portal/session extension allowlist applied: {} -> {}",
                before,
                all_extensions.len()
            );
        }

        let workspace_path = task
            .content
            .get("workspace_path")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());

        let mcp = if !all_extensions.is_empty() {
            match McpConnector::connect(
                &all_extensions,
                api_caller.clone(),
                workspace_path.as_deref(),
            )
            .await
            {
                Ok(c) => {
                    tracing::info!("MCP connector ready, has_tools={}", c.has_tools());
                    Some(c)
                }
                Err(e) => {
                    tracing::warn!("Failed to connect MCP extensions: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Initialize platform extensions (Skills, Team, Todo, DocumentTools, PortalTools) in-process
        let platform = PlatformExtensionRunner::create(
            &platform_enabled_extensions,
            Some(self.db.clone()),
            Some(&task.team_id),
            Some(session_id.as_str()),
            None, // mission_id
            Some(&agent.id),
            self.runtime_settings.skill_mode == TeamSkillMode::OnDemand,
            workspace_path.as_deref(),
            Some(&self.runtime_settings.workspace_root),
            Some(&self.runtime_settings.portal_base_url),
            allowed_extension_names.as_ref(),
            allowed_skill_ids.as_ref(),
        )
        .await;
        if platform.has_tools() {
            tracing::info!(
                "Platform extensions ready: {:?}",
                platform.extension_names()
            );
        }

        let has_tools = mcp.as_ref().map_or(false, |m| m.has_tools()) || platform.has_tools();

        // Export attached documents once (used by both Local API and Provider paths)
        let doc_section = match (&session, &workspace_path) {
            (Some(sess), Some(wp)) if !sess.attached_document_ids.is_empty() => {
                let injector = DocumentContextInjector::new((*self.db).clone());
                let exported = injector
                    .export_to_workspace(&task.team_id, &sess.attached_document_ids, wp)
                    .await;
                let section = DocumentContextInjector::format_as_prompt_section(&exported);
                if section.is_empty() {
                    None
                } else {
                    Some(section)
                }
            }
            (Some(sess), None) if !sess.attached_document_ids.is_empty() => {
                tracing::warn!(
                    "Session has attached documents but no workspace_path, skipping export"
                );
                None
            }
            _ => None,
        };

        // Local API: always single-turn (no Provider abstraction)
        if agent.api_format == ApiFormat::Local {
            let ext_infos = self.collect_extension_infos(mcp.as_ref(), &platform);
            let mut local_msgs =
                self.build_messages_with_system_prompt(agent, user_messages, &ext_infos);

            // Inject document context into system message
            if let Some(ref ds) = doc_section {
                if let Some(first) = local_msgs.first_mut() {
                    if let Some(content) = first.get("content").and_then(|c| c.as_str()) {
                        let updated = format!("{}{}", content, ds);
                        first["content"] = serde_json::Value::String(updated);
                    }
                }
            }

            // Inject session extra_instructions into system message
            if let Some(ref sess) = session {
                if let Some(ref extra) = sess.extra_instructions {
                    if !extra.trim().is_empty() {
                        if let Some(first) = local_msgs.first_mut() {
                            if let Some(content) = first.get("content").and_then(|c| c.as_str()) {
                                let updated = format!(
                                    "{}\n\n<extra_instructions>\n{}\n</extra_instructions>",
                                    content, extra
                                );
                                first["content"] = serde_json::Value::String(updated);
                            }
                        }
                    }
                }
            }

            let response = match self.call_local_api(agent, &local_msgs).await {
                Ok(r) => r,
                Err(e) => {
                    if let Some(m) = mcp {
                        m.shutdown().await;
                    }
                    return Err(e);
                }
            };
            let text = response["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string();
            self.task_manager
                .broadcast(
                    task_id,
                    StreamEvent::Text {
                        content: text.clone(),
                    },
                )
                .await;
            self.save_task_result(task_id, TaskResultType::Message, &text)
                .await?;
            if let Some(m) = mcp {
                m.shutdown().await;
            }
            return Ok(());
        }

        // Create Provider via factory
        let provider = provider_factory::create_provider_for_agent(agent)?;

        // Extract mission context from task content (injected by execute_via_bridge)
        let mission_ctx: Option<MissionPromptContext> = task
            .content
            .get("mission_context")
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        // Build system prompt: core template + optional agent custom instructions
        let mut system_prompt = {
            let ext_infos = self.collect_extension_infos(mcp.as_ref(), &platform);
            let custom = agent
                .system_prompt
                .as_deref()
                .filter(|s| !s.trim().is_empty());
            build_system_prompt(&ext_infos, custom, mission_ctx.as_ref())
        };

        // In assigned mode, inject assigned team skills into the system prompt.
        if self.runtime_settings.skill_mode == TeamSkillMode::Assigned {
            let team_skills_content =
                load_assigned_skills_content(&self.db, agent, allowed_skill_ids.as_ref()).await;
            if !team_skills_content.is_empty() {
                system_prompt.push_str("\n\n<team-skills>\n");
                for (name, content) in &team_skills_content {
                    system_prompt.push_str(&format!("## Skill: {}\n{}\n\n", name, content));
                }
                system_prompt.push_str("</team-skills>");
            }
        }

        // Inject attached document context into system prompt
        if let Some(ref ds) = doc_section {
            system_prompt.push_str(ds);
        }

        // Inject session extra_instructions (e.g. portal project path)
        if let Some(ref sess) = session {
            if let Some(ref extra) = sess.extra_instructions {
                if !extra.trim().is_empty() {
                    system_prompt.push_str("\n\n<extra_instructions>\n");
                    system_prompt.push_str(extra);
                    system_prompt.push_str("\n</extra_instructions>");
                }
            }
        }

        let mut messages = self.build_provider_messages(user_messages);
        if !history_messages.is_empty() {
            let mut with_history = history_messages;
            with_history.extend(messages);
            messages = with_history;
        }

        // If no tools and no extension manager, single-turn via streaming
        let ext_manager_enabled = {
            let mut enabled = agent_has_extension_manager_enabled(agent);
            if let Some(ref sess) = session {
                if sess.portal_restricted {
                    enabled = false;
                }
            }
            if enabled {
                if let Some(allowed) = &allowed_extension_names {
                    enabled = allowed.contains("extension_manager");
                }
            }
            enabled
        };
        if !has_tools && !ext_manager_enabled {
            let (response_msg, usage) = self
                .call_provider_streaming(
                    task_id,
                    &provider,
                    &system_prompt,
                    &messages,
                    &[],
                    cancel_token,
                )
                .await?;
            let text = response_msg.as_concat_text();
            self.save_task_result(task_id, TaskResultType::Message, &text)
                .await?;

            // Save session
            messages.push(response_msg);
            self.save_session_state(&session_id, &messages, 0, 0).await;

            if let Some(m) = mcp {
                m.shutdown().await;
            }
            return Ok(());
        }

        // Wrap MCP + platform in shared state for dynamic extension management
        let dynamic_state = Arc::new(RwLock::new(DynamicExtensionState {
            mcp,
            platform,
            agent: agent.clone(),
            api_caller: api_caller.clone(),
        }));

        // Create extension manager client if enabled (with session persistence)
        let ext_manager = if ext_manager_enabled {
            tracing::info!("ExtensionManager enabled for this agent");
            Some(TeamExtensionManagerClient::with_session(
                dynamic_state.clone(),
                session_id.clone(),
                self.agent_service.clone(),
            ))
        } else {
            None
        };

        // Multi-turn agent loop with Provider
        let portal_restricted = session
            .as_ref()
            .map(|s| s.portal_restricted)
            .unwrap_or(false);
        let session_retry_config = session.as_ref().and_then(|s| s.retry_config.clone());
        let session_require_final_report = session
            .as_ref()
            .map(|s| s.require_final_report)
            .unwrap_or(false);
        let session_max_turns = session
            .as_ref()
            .and_then(|s| s.max_turns)
            .and_then(|v| (v > 0).then_some((v as usize).min(MAX_UNIFIED_MAX_TURNS)));
        let session_tool_timeout_secs = session
            .as_ref()
            .and_then(|s| s.tool_timeout_seconds)
            .filter(|v| *v > 0)
            .map(|v| v.min(MAX_TOOL_TIMEOUT_SECS));
        let session_max_portal_retry_rounds = session
            .as_ref()
            .and_then(|s| s.max_portal_retry_rounds)
            .and_then(|v| (v > 0).then_some(v as usize));
        let result = self
            .run_unified_loop(
                task_id,
                &provider,
                &system_prompt,
                messages,
                dynamic_state.clone(),
                ext_manager.as_ref(),
                cancel_token,
                &session_id,
                &compaction_strategy_name,
                portal_restricted,
                workspace_path.clone(),
                session_retry_config,
                session_require_final_report,
                session_max_turns,
                session_tool_timeout_secs,
                session_max_portal_retry_rounds,
            )
            .await;

        // Save extension state changes to session before shutdown
        if ext_manager_enabled {
            let state_read = dynamic_state.read().await;
            let mut active_names: Vec<String> = Vec::new();
            if let Some(ref mcp_conn) = state_read.mcp {
                active_names.extend(mcp_conn.extension_names());
            }
            active_names.extend(state_read.platform.extension_names());
            drop(state_read);

            let active_set: HashSet<String> = active_names.into_iter().collect();

            let overrides = super::runtime::compute_extension_overrides(agent, &active_set);

            if !overrides.disabled.is_empty() || !overrides.enabled.is_empty() {
                tracing::info!(
                    "Saving extension overrides to session {}: disabled={:?}, enabled={:?}",
                    session_id,
                    overrides.disabled,
                    overrides.enabled
                );
                if let Err(e) = self
                    .agent_service
                    .update_session_extensions(&session_id, &overrides.disabled, &overrides.enabled)
                    .await
                {
                    tracing::warn!("Failed to save extension overrides: {}", e);
                }
            }
        }

        // Shutdown MCP connections (take ownership via write lock to avoid leak
        // when Arc::try_unwrap fails due to lingering references)
        {
            let mcp = {
                let mut state = dynamic_state.write().await;
                state.mcp.take()
            };
            if let Some(m) = mcp {
                m.shutdown().await;
            }
        }

        result
    }

    /// Collect ExtensionInfo from both MCP and platform extensions.
    /// MCP extensions only have names (no instructions); platform extensions have real instructions.
    fn collect_extension_infos(
        &self,
        mcp: Option<&McpConnector>,
        platform: &PlatformExtensionRunner,
    ) -> Vec<ExtensionInfo> {
        let mut infos: Vec<ExtensionInfo> = mcp
            .map(|m| {
                m.extension_names()
                    .into_iter()
                    .map(|name| ExtensionInfo::new(&name, "", false))
                    .collect()
            })
            .unwrap_or_default();
        infos.extend(platform.extension_infos());
        // Sort alphabetically for prompt caching stability (same as local agent's prompt_manager)
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        infos
    }

    /// Build messages with system prompt prepended
    fn build_messages_with_system_prompt(
        &self,
        agent: &TeamAgent,
        user_messages: &[serde_json::Value],
        extensions: &[ExtensionInfo],
    ) -> Vec<serde_json::Value> {
        let mut messages = Vec::new();

        // Build system prompt: core template + optional agent custom instructions
        let custom = agent
            .system_prompt
            .as_deref()
            .filter(|s| !s.trim().is_empty());
        let prompt = build_system_prompt(extensions, custom, None);
        messages.push(serde_json::json!({
            "role": "system",
            "content": prompt
        }));

        // Add user messages
        messages.extend(user_messages.iter().cloned());

        messages
    }

    /// Build Provider-compatible Message list from raw JSON user messages
    fn build_provider_messages(&self, user_messages: &[serde_json::Value]) -> Vec<Message> {
        user_messages
            .iter()
            .map(|msg| {
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
                match role {
                    "assistant" => Message::assistant().with_text(content),
                    _ => Message::user().with_text(content),
                }
            })
            .collect()
    }

    /// Call local Ollama API (no tool calling support)
    async fn call_local_api(
        &self,
        agent: &TeamAgent,
        messages: &[serde_json::Value],
    ) -> Result<serde_json::Value> {
        let api_url = agent
            .api_url
            .as_deref()
            .unwrap_or("http://localhost:11434/api/chat");
        let model = agent.model.as_deref().unwrap_or("llama2");

        let client = build_http_client()?;
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
        Ok(result)
    }

    /// Convert tool content blocks to plain text (for SSE and truncation)
    fn tool_blocks_to_text(blocks: &[super::mcp_connector::ToolContentBlock]) -> String {
        blocks
            .iter()
            .map(|b| match b {
                super::mcp_connector::ToolContentBlock::Text(text) => text.clone(),
                super::mcp_connector::ToolContentBlock::Image { mime_type, .. } => {
                    format!("[Image: {}]", mime_type)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Truncate a string at a UTF-8 safe byte boundary.
    /// `max_bytes` is the maximum byte length; the result is trimmed back
    /// to the nearest char boundary if it falls mid-character.
    fn safe_truncate(s: &str, max_bytes: usize) -> &str {
        if s.len() <= max_bytes {
            return s;
        }
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }

    /// Truncate tool result text if it exceeds the maximum allowed length
    fn truncate_tool_result(text: String) -> String {
        if text.len() > MAX_TOOL_RESULT_CHARS {
            let total = text.len();
            let truncated = Self::safe_truncate(&text, MAX_TOOL_RESULT_CHARS);
            format!(
                "{}\n[truncated: showing first {} of {} bytes]",
                truncated,
                truncated.len(),
                total
            )
        } else {
            text
        }
    }

    /// Heuristic: whether this tool call likely changed files in workspace.
    fn tool_may_change_workspace(tool_name: &str) -> bool {
        let name = tool_name.to_lowercase();
        if name == "shell" || name == "text_editor" {
            return true;
        }
        [
            "write",
            "edit",
            "replace",
            "insert",
            "undo",
            "apply_patch",
            "mkdir",
            "touch",
            "rm",
            "cp",
            "mv",
        ]
        .iter()
        .any(|k| name.contains(k))
    }

    /// Return the latest user-authored text from the conversation buffer.
    fn latest_user_text(messages: &[Message]) -> String {
        messages
            .iter()
            .rev()
            .find(|m| m.role == rmcp::model::Role::User)
            .map(|m| m.as_concat_text())
            .unwrap_or_default()
    }

    /// Heuristic: whether user intent is coding/implementation for portal workspace.
    fn has_portal_coding_intent(user_text: &str) -> bool {
        let user_lower = user_text.to_lowercase();
        let coding_keywords = [
            "build", "create", "make", "implement", "update", "modify",
            "refactor", "fix", "html", "css", "javascript", "website",
            "代码", "页面", "网站", "修改", "创建", "实现", "修复", "重构",
        ];
        coding_keywords.iter().any(|k| user_lower.contains(k))
    }

    /// Heuristic: assistant reply looks like planning-only (no execution/output).
    fn assistant_looks_planning_only(assistant_text: &str) -> bool {
        let t = assistant_text.trim().to_lowercase();
        if t.is_empty() {
            return false;
        }
        // Only match when the response STARTS with planning phrases
        let planning_prefixes = [
            "let me", "i will", "i'll", "first,", "first ", "i need to",
            "先", "让我", "我先", "我来", "我需要", "我将",
        ];
        planning_prefixes.iter().any(|p| t.starts_with(p))
    }

    /// Heuristic: assistant explicitly claims completion/delivery.
    fn assistant_claims_completion(assistant_text: &str) -> bool {
        let assistant_lower = assistant_text.trim().to_lowercase();
        if assistant_lower.is_empty() {
            return false;
        }
        if Self::assistant_looks_planning_only(assistant_text) {
            return false;
        }
        let completion_keywords = [
            "done",
            "completed",
            "finished",
            "implemented",
            "created",
            "updated",
            "fixed",
            "ready",
            "已完成",
            "完成了",
            "已经完成",
            "已实现",
            "已创建",
            "已更新",
            "已修复",
            "可以访问",
            "可以测试",
        ];
        if completion_keywords
            .iter()
            .any(|k| assistant_lower.contains(k))
        {
            return true;
        }
        // Fallback: a substantive non-planning response likely indicates completion.
        assistant_lower.len() >= 120
    }

    /// Response schema used by `final_output` when portal sessions require
    /// a structured completion report.
    fn required_final_report_response() -> Response {
        Response {
            json_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "summary": {
                        "type": "string",
                        "description": "Short summary of what was implemented."
                    },
                    "changed_files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "Workspace-relative changed files."
                    },
                    "preview_url": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Preview URL that can be opened for verification."
                    },
                    "verification": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "Executed verification steps and outcomes."
                    }
                },
                "required": ["changed_files", "preview_url", "verification"],
                "additionalProperties": true
            })),
        }
    }

    fn retry_timeout_seconds(cfg: &RetryConfig) -> u64 {
        cfg.timeout_seconds.unwrap_or(DEFAULT_RETRY_TIMEOUT_SECONDS)
    }

    fn on_failure_timeout_seconds(cfg: &RetryConfig) -> u64 {
        cfg.on_failure_timeout_seconds
            .unwrap_or(DEFAULT_ON_FAILURE_TIMEOUT_SECONDS)
    }

    fn tool_timeout_seconds() -> Option<u64> {
        std::env::var("TEAM_AGENT_TOOL_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .map(|v| v.min(MAX_TOOL_TIMEOUT_SECS))
    }

    fn unified_max_turns() -> Option<usize> {
        std::env::var("TEAM_AGENT_MAX_TURNS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .map(|v| v.min(MAX_UNIFIED_MAX_TURNS))
    }

    async fn execute_shell_command_in_workspace(
        command: &str,
        timeout: Duration,
        workspace_path: Option<&str>,
    ) -> Result<Output> {
        let mut cmd = if cfg!(target_os = "windows") {
            let mut cmd = Command::new("cmd");
            cmd.args(["/C", command]);
            cmd
        } else {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", command]);
            cmd
        };
        if let Some(path) = workspace_path {
            cmd.current_dir(path);
        }
        configure_command_no_window(&mut cmd);
        let fut = async move {
            cmd.stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output()
                .await
                .map_err(anyhow::Error::from)
        };
        tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| anyhow!("Command timed out after {:?}", timeout))?
    }

    async fn run_success_checks(
        retry_config: &RetryConfig,
        workspace_path: Option<&str>,
    ) -> Result<()> {
        for check in &retry_config.checks {
            match check {
                SuccessCheck::Shell { command } => {
                    let timeout = Duration::from_secs(Self::retry_timeout_seconds(retry_config));
                    let output =
                        Self::execute_shell_command_in_workspace(command, timeout, workspace_path)
                            .await?;
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        return Err(anyhow!(
                            "success check failed: `{}` (status={:?}, stdout={}, stderr={})",
                            command,
                            output.status.code(),
                            stdout,
                            stderr
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    async fn run_on_failure_command(
        retry_config: &RetryConfig,
        workspace_path: Option<&str>,
    ) -> Result<()> {
        let Some(on_failure) = retry_config.on_failure.as_ref() else {
            return Ok(());
        };
        if on_failure.trim().is_empty() {
            return Ok(());
        }
        let timeout = Duration::from_secs(Self::on_failure_timeout_seconds(retry_config));
        let output =
            Self::execute_shell_command_in_workspace(on_failure, timeout, workspace_path).await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(anyhow!(
                "on_failure command failed: `{}` (status={:?}, stderr={})",
                on_failure,
                output.status.code(),
                stderr
            ));
        }
        Ok(())
    }

    /// In portal coding mode, decide whether to force a retry reminder when
    /// the model returned no tool calls.
    fn should_force_portal_tool_retry(latest_user_text: &str, assistant_text: &str) -> bool {
        let user_text = latest_user_text.trim();
        if user_text.is_empty() {
            return false;
        }

        let assistant_lower = assistant_text.trim().to_lowercase();
        let has_coding_intent = Self::has_portal_coding_intent(user_text);
        let looks_like_planning_only = Self::assistant_looks_planning_only(assistant_text);

        has_coding_intent && (looks_like_planning_only || assistant_lower.is_empty())
    }

    /// Heuristic: identify transient provider/runtime failures worth retrying.
    fn is_retryable_provider_error(err: &anyhow::Error) -> bool {
        if let Some(pe) = err.downcast_ref::<ProviderError>() {
            return matches!(
                pe,
                ProviderError::RateLimitExceeded { .. }
                    | ProviderError::ServerError(_)
                    | ProviderError::RequestFailed(_)
            );
        }
        Self::is_transient_error_text(&err.to_string())
    }

    /// Heuristic for transient text errors coming from wrapped anyhow contexts.
    fn is_transient_error_text(err_text: &str) -> bool {
        let t = err_text.to_lowercase();
        let keywords = [
            "timeout",
            "timed out",
            "temporar",
            "connection reset",
            "connection aborted",
            "connection refused",
            "connection closed",
            "network",
            "eof",
            "stream ended without producing a message",
            "service unavailable",
            "too many requests",
            "429",
            "502",
            "503",
            "504",
        ];
        keywords.iter().any(|k| t.contains(k))
    }

    /// Exponential backoff for provider retry, honoring provider-suggested rate-limit delay.
    fn provider_retry_delay(err: &anyhow::Error, attempt: usize) -> Duration {
        if let Some(ProviderError::RateLimitExceeded {
            retry_delay: Some(delay),
            ..
        }) = err.downcast_ref::<ProviderError>()
        {
            return *delay;
        }
        let shift = attempt.saturating_sub(1).min(4) as u32;
        let millis = (1000u64.saturating_mul(1u64 << shift)).min(15_000);
        Duration::from_millis(millis)
    }

    /// Unified multi-turn agent loop using Provider abstraction.
    /// Uses streaming output, session persistence, and context compaction.
    /// Tools are rebuilt each turn to reflect dynamic extension changes.
    async fn run_unified_loop(
        &self,
        task_id: &str,
        provider: &Arc<dyn Provider>,
        system_prompt: &str,
        initial_messages: Vec<Message>,
        dynamic_state: Arc<RwLock<DynamicExtensionState>>,
        ext_manager: Option<&TeamExtensionManagerClient>,
        cancel_token: &CancellationToken,
        session_id: &str,
        compaction_strategy_name: &str,
        portal_restricted: bool,
        workspace_path: Option<String>,
        retry_config: Option<RetryConfig>,
        require_final_report: bool,
        max_turns_override: Option<usize>,
        tool_timeout_secs_override: Option<u64>,
        max_portal_retry_rounds: Option<usize>,
    ) -> Result<()> {
        let max_turns = max_turns_override.or_else(Self::unified_max_turns);
        let tool_timeout_secs = tool_timeout_secs_override.or_else(Self::tool_timeout_seconds);
        let mut messages = initial_messages;
        let mut all_text = String::new();
        let mut repetition_detector = RepetitionDetector::new();
        let mut completed_due_to_max_turns = false;
        let mut portal_tool_retry_count: usize = 0;
        let mut provider_retry_count: usize = 0;
        const DEFAULT_MAX_PROVIDER_RETRIES: usize = 3;
        let max_provider_retries = retry_config
            .as_ref()
            .map(|cfg| cfg.max_retries.max(1) as usize)
            .unwrap_or(DEFAULT_MAX_PROVIDER_RETRIES)
            .min(8);
        let mut portal_successful_tool_calls: usize = 0;
        let mut previous_turn_had_tool_failure = false;
        let mut accumulated_input: i32 = 0;
        let mut accumulated_output: i32 = 0;
        let mut session_tokens: Option<i32> = None;
        /// Max recovery compaction attempts before giving up (same as local agent)
        const MAX_RECOVERY_COMPACTION_ATTEMPTS: i32 = 3;
        let mut recovery_compaction_attempts: i32 = 0;
        let mut effective_system_prompt = system_prompt.to_string();
        let mut final_output_tool = if require_final_report {
            let tool = FinalOutputTool::new(Self::required_final_report_response());
            effective_system_prompt.push_str("\n\n");
            effective_system_prompt.push_str(&tool.system_prompt());
            Some(tool)
        } else {
            None
        };

        let mut turn: usize = 0;
        loop {
            if let Some(limit) = max_turns {
                if turn >= limit {
                    completed_due_to_max_turns = true;
                    break;
                }
            }
            let turn = {
                let current = turn;
                turn = turn.saturating_add(1);
                current
            };
            // Check cancellation
            if cancel_token.is_cancelled() {
                tracing::info!("Unified loop cancelled at turn {}", turn + 1);
                break;
            }

            // Rebuild tools each turn to reflect dynamic extension changes
            let tools = {
                let state = dynamic_state.read().await;
                let mut t = state
                    .mcp
                    .as_ref()
                    .map(|m| m.tools_as_rmcp())
                    .unwrap_or_default();
                t.extend(state.platform.tools_as_rmcp());
                if ext_manager.is_some() {
                    t.extend(TeamExtensionManagerClient::tools_as_rmcp());
                }
                if let Some(ref final_tool) = final_output_tool {
                    t.push(final_tool.tool());
                }
                t
            }; // read lock released here

            // Context compaction check (skip first turn)
            if turn > 0 {
                if let Ok(true) = self
                    .check_compaction_needed(
                        provider,
                        &effective_system_prompt,
                        &messages,
                        &tools,
                        session_tokens,
                    )
                    .await
                {
                    let strategy = ContextCompactionStrategy::from_str(compaction_strategy_name);
                    let conversation = Conversation::new_unvalidated(messages.clone());
                    let before_tokens = session_tokens.unwrap_or(0) as usize;
                    match compact_messages_with_strategy(
                        provider.as_ref(),
                        &conversation,
                        false,
                        strategy,
                    )
                    .await
                    {
                        Ok((compacted, _usage)) => {
                            messages = compacted.messages().to_vec();
                            // Recount actual tokens after compaction (usage.total_tokens is often None)
                            let after_tokens = match create_token_counter().await {
                                Ok(counter) => counter.count_chat_tokens(
                                    &effective_system_prompt,
                                    &messages,
                                    &tools,
                                ),
                                Err(_) => 0,
                            };
                            session_tokens = Some(after_tokens as i32);
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::Compaction {
                                        strategy: compaction_strategy_name.to_string(),
                                        before_tokens,
                                        after_tokens,
                                    },
                                )
                                .await;
                            let _ = self
                                .agent_service
                                .increment_compaction_count(session_id, compaction_strategy_name)
                                .await;
                            tracing::info!(
                                "Compaction done: {} -> {} tokens",
                                before_tokens,
                                after_tokens
                            );
                        }
                        Err(e) => {
                            tracing::warn!("Compaction failed: {}, continuing", e);
                        }
                    }
                }
            }

            if let Some(limit) = max_turns {
                tracing::info!("Unified agent loop turn {}/{}", turn + 1, limit);
            } else {
                tracing::info!("Unified agent loop turn {}/unlimited", turn + 1);
            }
            self.task_manager
                .broadcast(
                    task_id,
                    StreamEvent::Turn {
                        current: turn + 1,
                        max: max_turns.unwrap_or(0),
                    },
                )
                .await;

            // Inject MOIM (Message of Immediate Memory) from platform extensions.
            // Inserted before the last assistant message (same as local agent in moim.rs).
            // Uses a temporary copy so the original messages list is not modified.
            let moim = {
                let state = dynamic_state.read().await;
                state.platform.collect_moim().await
            };
            let messages_for_llm = if let Some(moim) = moim {
                let mut tmp = messages.clone();
                let idx = tmp
                    .iter()
                    .rposition(|m| m.role == rmcp::model::Role::Assistant)
                    .unwrap_or(0);
                tmp.insert(idx, Message::user().with_text(moim));
                tmp
            } else {
                messages.clone()
            };

            // Fix conversation format before sending to LLM (same as local agent).
            // Ensures first/last messages are from user, merges consecutive same-role messages, etc.
            let conversation_for_llm = Conversation::new_unvalidated(messages_for_llm);
            let (fixed_conversation, fix_issues) = fix_conversation(conversation_for_llm);
            if !fix_issues.is_empty() {
                tracing::debug!("Conversation fixes applied: {:?}", fix_issues);
            }
            let messages_for_llm = fixed_conversation.messages().to_vec();

            // Call LLM via streaming Provider, with ContextLengthExceeded recovery
            let call_result = self
                .call_provider_streaming(
                    task_id,
                    provider,
                    &effective_system_prompt,
                    &messages_for_llm,
                    &tools,
                    cancel_token,
                )
                .await;

            let (response_msg, usage) = match call_result {
                Ok(result) => {
                    provider_retry_count = 0;
                    result
                }
                Err(e) => {
                    // Check if this is a ContextLengthExceeded error (recovery compaction)
                    if e.downcast_ref::<ProviderError>().map_or(false, |pe| {
                        matches!(pe, ProviderError::ContextLengthExceeded(_))
                    }) {
                        if recovery_compaction_attempts >= MAX_RECOVERY_COMPACTION_ATTEMPTS {
                            tracing::error!(
                                "Exceeded max recovery compaction attempts ({})",
                                MAX_RECOVERY_COMPACTION_ATTEMPTS
                            );
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::Text {
                                        content: format!(
                                            "Context limit exceeded after {} compaction attempts. Please start a new session.",
                                            MAX_RECOVERY_COMPACTION_ATTEMPTS
                                        ),
                                    },
                                )
                                .await;
                            // Save session state before breaking so context is not lost
                            self.save_session_state(
                                session_id,
                                &messages,
                                accumulated_input,
                                accumulated_output,
                            )
                            .await;
                            break;
                        }

                        recovery_compaction_attempts += 1;
                        tracing::info!(
                            "Recovery compaction attempt {}/{} due to ContextLengthExceeded",
                            recovery_compaction_attempts,
                            MAX_RECOVERY_COMPACTION_ATTEMPTS
                        );

                        // Perform recovery compaction
                        let strategy =
                            ContextCompactionStrategy::from_str(compaction_strategy_name);
                        let conversation = Conversation::new_unvalidated(messages.clone());
                        match compact_messages_with_strategy(
                            provider.as_ref(),
                            &conversation,
                            false,
                            strategy,
                        )
                        .await
                        {
                            Ok((compacted, _usage)) => {
                                messages = compacted.messages().to_vec();
                                // Recount actual tokens after recovery compaction
                                let after_tokens = match create_token_counter().await {
                                    Ok(counter) => counter.count_chat_tokens(
                                        &effective_system_prompt,
                                        &messages,
                                        &tools,
                                    ) as i32,
                                    Err(_) => 0,
                                };
                                session_tokens = Some(after_tokens);
                                self.task_manager
                                    .broadcast(
                                        task_id,
                                        StreamEvent::Compaction {
                                            strategy: compaction_strategy_name.to_string(),
                                            before_tokens: 0,
                                            after_tokens: after_tokens as usize,
                                        },
                                    )
                                    .await;
                                tracing::info!("Recovery compaction done, retrying LLM call");
                                continue; // Retry the turn
                            }
                            Err(compact_err) => {
                                tracing::error!("Recovery compaction failed: {}", compact_err);
                                return Err(anyhow!(
                                    "Context length exceeded and compaction failed: {}",
                                    compact_err
                                ));
                            }
                        }
                    }
                    if Self::is_retryable_provider_error(&e)
                        && provider_retry_count < max_provider_retries
                    {
                        provider_retry_count += 1;
                        let delay = Self::provider_retry_delay(&e, provider_retry_count);
                        tracing::warn!(
                            "Transient provider failure on turn {}, retrying ({}/{}), backoff {:?}: {}",
                            turn + 1,
                            provider_retry_count,
                            max_provider_retries,
                            delay,
                            e
                        );
                        self.task_manager
                            .broadcast(
                                task_id,
                                StreamEvent::Status {
                                    status: "llm_retry".to_string(),
                                },
                            )
                            .await;
                        tokio::select! {
                            _ = tokio::time::sleep(delay) => {},
                            _ = cancel_token.cancelled() => {
                                return Err(anyhow!("Task cancelled during llm retry backoff"));
                            }
                        }
                        continue;
                    }
                    // Not a ContextLengthExceeded error, propagate
                    return Err(e);
                }
            };

            // Accumulate token stats
            if let Some(ref u) = usage {
                accumulated_input += u.usage.input_tokens.unwrap_or(0);
                accumulated_output += u.usage.output_tokens.unwrap_or(0);
                session_tokens = Some(accumulated_input + accumulated_output);
            }

            // Extract text and tool requests from response
            // (Text was already streamed via call_provider_streaming)
            let mut tool_requests: Vec<(String, String, serde_json::Value)> = Vec::new();
            let latest_user_text = if portal_restricted {
                Self::latest_user_text(&messages)
            } else {
                String::new()
            };
            let assistant_text = response_msg.as_concat_text();
            for content in &response_msg.content {
                match content {
                    MessageContent::Text(tc) => {
                        if !tc.text.is_empty() {
                            all_text.push_str(&tc.text);
                        }
                    }
                    MessageContent::ToolRequest(req) => {
                        if let Ok(ref call) = req.tool_call {
                            let name = call.name.to_string();
                            let args = serde_json::to_value(&call.arguments)
                                .unwrap_or(serde_json::json!({}));
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::ToolCall {
                                        name: name.clone(),
                                        id: req.id.clone(),
                                    },
                                )
                                .await;
                            tool_requests.push((req.id.clone(), name, args));
                        }
                    }
                    MessageContent::Thinking(tc) => {
                        if !tc.thinking.is_empty() {
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::Thinking {
                                        content: tc.thinking.clone(),
                                    },
                                )
                                .await;
                        }
                    }
                    _ => {}
                }
            }

            // Append assistant message to conversation
            messages.push(response_msg);

            // If no tool calls, we're done
            if tool_requests.is_empty() {
                if portal_restricted {
                    let max_portal_tool_retries = max_portal_retry_rounds;
                    let has_coding_intent =
                        require_final_report || Self::has_portal_coding_intent(&latest_user_text);
                    let completion_claimed = Self::assistant_claims_completion(&assistant_text);
                    let final_output_collected = final_output_tool
                        .as_ref()
                        .and_then(|tool| tool.final_output.as_ref())
                        .is_some();
                    let has_success_checks = retry_config
                        .as_ref()
                        .map(|cfg| !cfg.checks.is_empty())
                        .unwrap_or(false);
                    let missing_execution = has_coding_intent && portal_successful_tool_calls == 0;
                    let missing_completion_signal = has_coding_intent
                        && !completion_claimed
                        && !(require_final_report && final_output_collected);
                    let missing_final_report =
                        has_coding_intent && require_final_report && !final_output_collected;
                    let missing_success_checks_config = has_coding_intent && !has_success_checks;
                    let mut success_check_failure: Option<String> = None;
                    if has_coding_intent
                        && completion_claimed
                        && !missing_execution
                        && !missing_final_report
                        && !missing_success_checks_config
                    {
                        if let Some(cfg) = retry_config.as_ref() {
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::Status {
                                        status: "completion_check".to_string(),
                                    },
                                )
                                .await;
                            if workspace_path.is_none() {
                                success_check_failure = Some(
                                    "workspace path missing for retry success checks".to_string(),
                                );
                            } else if let Err(e) =
                                Self::run_success_checks(cfg, workspace_path.as_deref()).await
                            {
                                success_check_failure = Some(e.to_string());
                            }
                        }
                    }
                    let should_force_retry =
                        Self::should_force_portal_tool_retry(&latest_user_text, &assistant_text)
                            || previous_turn_had_tool_failure
                            || missing_execution
                            || missing_completion_signal
                            || missing_final_report
                            || missing_success_checks_config
                            || success_check_failure.is_some();

                    if should_force_retry {
                        let can_retry = max_portal_tool_retries
                            .map(|max| portal_tool_retry_count < max)
                            .unwrap_or(true);
                        if can_retry {
                            portal_tool_retry_count = portal_tool_retry_count.saturating_add(1);
                            let attempt = portal_tool_retry_count;
                            let retry_limit_label = max_portal_tool_retries
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "unlimited".to_string());
                            let (failure_code, failure_reason) = if let Some(ref err) =
                                success_check_failure
                            {
                                (
                                    "success_checks_failed",
                                    format!("success checks failed: {}", err),
                                )
                            } else if missing_success_checks_config {
                                (
                                    "missing_success_checks_config",
                                    "retry.success_checks is required for portal coding tasks"
                                        .to_string(),
                                )
                            } else if missing_final_report {
                                (
                                    "missing_final_report",
                                    format!(
                                        "missing required final report (prefer final_output tool). {}",
                                        FINAL_OUTPUT_CONTINUATION_MESSAGE
                                    ),
                                )
                            } else if previous_turn_had_tool_failure {
                                (
                                    "previous_tool_failure",
                                    "previous tool execution failed".to_string(),
                                )
                            } else if missing_execution {
                                (
                                    "missing_execution",
                                    "no successful developer tool execution observed".to_string(),
                                )
                            } else if missing_completion_signal {
                                (
                                    "missing_completion_signal",
                                    "assistant did not explicitly report completion".to_string(),
                                )
                            } else {
                                (
                                    "no_actionable_output",
                                    "model produced no actionable output".to_string(),
                                )
                            };
                            tracing::warn!(
                                "Portal coding guard triggered: no tool call on turn {}, code={}, reason={}, injecting retry reminder ({}/{})",
                                turn + 1,
                                failure_code,
                                failure_reason,
                                attempt,
                                retry_limit_label
                            );
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::Status {
                                        status: "portal_tool_retry".to_string(),
                                    },
                                )
                                .await;

                            if let Some(cfg) = retry_config.as_ref() {
                                self.task_manager
                                    .broadcast(
                                        task_id,
                                        StreamEvent::Status {
                                            status: "on_failure".to_string(),
                                        },
                                    )
                                    .await;
                                if let Err(e) =
                                    Self::run_on_failure_command(cfg, workspace_path.as_deref())
                                        .await
                                {
                                    tracing::warn!("Portal on_failure command failed: {}", e);
                                }
                            }

                            let reminder = format!(
                                "Portal coding mode retry ({}/{}): {}. Continue implementation with concrete developer tool calls. \
If coding work is complete, provide a structured final report with: 1) changed files, 2) preview URL, 3) verification steps/results. {}",
                                attempt,
                                retry_limit_label,
                                failure_reason,
                                if missing_final_report {
                                    FINAL_OUTPUT_CONTINUATION_MESSAGE
                                } else {
                                    ""
                                }
                            );

                            // Agent-only reminder: user will not see this synthetic message.
                            messages.push(Message::user().with_text(reminder).agent_only());
                            continue;
                        }

                        let retry_limit = max_portal_tool_retries.unwrap_or(0);
                        let (reason_code, reason) = if let Some(err) = success_check_failure {
                            (
                                "success_checks_failed",
                                format!("success checks failed: {}", err),
                            )
                        } else if missing_success_checks_config {
                            (
                                "missing_success_checks_config",
                                "retry.success_checks is required for portal coding tasks"
                                    .to_string(),
                            )
                        } else if missing_final_report {
                            (
                                "missing_final_report",
                                "missing required final report".to_string(),
                            )
                        } else if previous_turn_had_tool_failure {
                            (
                                "previous_tool_failure",
                                "previous tool execution failed and the agent stopped without recovery"
                                    .to_string(),
                            )
                        } else if missing_execution {
                            (
                                "missing_execution",
                                "no successful developer tool execution was observed".to_string(),
                            )
                        } else if missing_completion_signal {
                            (
                                "missing_completion_signal",
                                "assistant did not provide an explicit completion result"
                                    .to_string(),
                            )
                        } else {
                            (
                                "no_actionable_output",
                                "model produced no actionable output".to_string(),
                            )
                        };
                        let structured_reason = serde_json::json!({
                            "type": "portal_task_incomplete",
                            "reason_code": reason_code,
                            "reason": reason,
                            "turn": turn + 1,
                            "max_retries": retry_limit,
                            "require_final_report": require_final_report,
                            "success_checks_required": true,
                            "success_checks_present": has_success_checks,
                            "successful_tool_calls": portal_successful_tool_calls
                        });
                        tracing::error!(
                            "Portal task marked incomplete after {} retries: {}",
                            retry_limit,
                            structured_reason
                        );
                        self.task_manager
                            .broadcast(
                                task_id,
                                StreamEvent::Status {
                                    status: "portal_incomplete".to_string(),
                                },
                            )
                            .await;
                        let warning = format!("[Portal task incomplete] {}", structured_reason);
                        all_text.push_str(&warning);
                        self.task_manager
                            .broadcast(task_id, StreamEvent::Text { content: warning })
                            .await;
                        return Err(anyhow!("Portal task incomplete: {}", structured_reason));
                    }
                }

                if require_final_report {
                    if let Some(final_output_text) = final_output_tool
                        .as_ref()
                        .and_then(|tool| tool.final_output.clone())
                    {
                        if assistant_text.trim().is_empty() {
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::Text {
                                        content: final_output_text.clone(),
                                    },
                                )
                                .await;
                            all_text.push_str(&final_output_text);
                            messages.push(Message::assistant().with_text(final_output_text));
                        }
                    }
                }

                tracing::info!("Unified loop ended: no tool calls at turn {}", turn + 1);
                break;
            }

            if portal_restricted {
                // Reset no-tool retry counter once model resumes actual tool execution.
                portal_tool_retry_count = 0;
            }

            // Track tool id -> name for richer runtime events (timeline/file updates)
            let tool_name_by_id: HashMap<String, String> = tool_requests
                .iter()
                .map(|(id, name, _)| (id.clone(), name.clone()))
                .collect();

            self.task_manager
                .broadcast(
                    task_id,
                    StreamEvent::Status {
                        status: "tool_execution".to_string(),
                    },
                )
                .await;

            // Check for repeated tool calls
            if cancel_token.is_cancelled() {
                return Err(anyhow!("Task cancelled during tool execution"));
            }

            let mut allowed = Vec::new();
            let mut denied: Vec<(String, String, String)> = Vec::new(); // (id, name, reason)
            for (id, name, args) in &tool_requests {
                if repetition_detector.check(name, args) {
                    allowed.push((id.clone(), name.clone(), args.clone()));
                } else {
                    tracing::warn!("Repeated tool call denied: {}", name);
                    denied.push((
                        id.clone(),
                        name.clone(),
                        "Tool call denied: repeated identical call detected (3 times). Try a different approach.".to_string(),
                    ));
                }
            }

            // Security scan: only check command-execution tools for dangerous patterns.
            // File-write tools (editors, file creators) contain code/markup that triggers
            // false positives on shell-oriented pattern rules.
            let mut security_allowed = Vec::new();
            let shell_keywords = ["shell", "bash", "cmd", "exec", "terminal", "run_command"];
            for (id, name, args) in allowed {
                let name_lower = name.to_lowercase();
                let is_shell_tool = shell_keywords.iter().any(|kw| name_lower.contains(kw));
                if !is_shell_tool {
                    security_allowed.push((id, name, args));
                    continue;
                }
                let tool_text = format!("Tool: {}\n{}", name, args);
                match self
                    .security_scanner
                    .scan_for_dangerous_patterns(&tool_text)
                    .await
                {
                    Ok(scan) if scan.is_malicious && scan.confidence >= 0.7 => {
                        tracing::warn!(
                            "Security: blocked tool '{}' (confidence={:.2}): {}",
                            name,
                            scan.confidence,
                            scan.explanation
                        );
                        let reason = format!(
                            "Tool call blocked by security scanner: {}",
                            scan.explanation
                        );
                        denied.push((id, name, reason));
                    }
                    _ => {
                        security_allowed.push((id, name, args));
                    }
                }
            }
            let allowed = security_allowed;

            // Split tool calls by execution mode.
            // - final_output is handled in-process (stateful, serial)
            // - ExtensionManager tools are serial (write lock needed)
            // - remaining tools run concurrently
            let mut final_output_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut ext_mgr_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut regular_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            for (id, name, args) in &allowed {
                if final_output_tool.is_some() && name == FINAL_OUTPUT_TOOL_NAME {
                    final_output_calls.push((id.clone(), name.clone(), args.clone()));
                } else if TeamExtensionManagerClient::can_handle(name) {
                    ext_mgr_calls.push((id.clone(), name.clone(), args.clone()));
                } else {
                    regular_calls.push((id.clone(), name.clone(), args.clone()));
                }
            }

            // Execute final_output calls serially first.
            let mut final_output_results: Vec<(
                String,
                u64,
                Result<Vec<super::mcp_connector::ToolContentBlock>, String>,
            )> = Vec::new();
            for (id, name, args) in &final_output_calls {
                let started_at = Instant::now();
                let result: Result<Vec<super::mcp_connector::ToolContentBlock>, String> =
                    if let Some(timeout_secs) = tool_timeout_secs {
                        match tokio::time::timeout(Duration::from_secs(timeout_secs), async {
                            let Some(tool) = final_output_tool.as_mut() else {
                                return Err("Final output tool is not enabled".to_string());
                            };
                            let tool_call = rmcp::model::CallToolRequestParam {
                                name: name.clone().into(),
                                arguments: args.as_object().cloned(),
                            };
                            let tool_result = tool.execute_tool_call(tool_call).await.result.await;
                            match tool_result {
                                Ok(call_result) => Ok(
                                    super::mcp_connector::McpConnector::extract_tool_result_blocks(
                                        &call_result,
                                    ),
                                ),
                                Err(e) => Err(format!("Error: {}", e)),
                            }
                        })
                        .await
                        {
                            Ok(outcome) => outcome,
                            Err(_) => Err(format!(
                                "Error: tool '{}' timed out after {}s",
                                name, timeout_secs
                            )),
                        }
                    } else {
                        match final_output_tool.as_mut() {
                            Some(tool) => {
                                let tool_call = rmcp::model::CallToolRequestParam {
                                    name: name.clone().into(),
                                    arguments: args.as_object().cloned(),
                                };
                                let tool_result =
                                    tool.execute_tool_call(tool_call).await.result.await;
                                match tool_result {
                                    Ok(call_result) => Ok(
                                        super::mcp_connector::McpConnector::extract_tool_result_blocks(
                                            &call_result,
                                        ),
                                    ),
                                    Err(e) => Err(format!("Error: {}", e)),
                                }
                            }
                            None => Err("Final output tool is not enabled".to_string()),
                        }
                    };
                let duration_ms = started_at.elapsed().as_millis() as u64;
                match result {
                    Ok(outcome) => {
                        final_output_results.push((id.clone(), duration_ms, Ok(outcome)))
                    }
                    Err(err) => {
                        tracing::warn!("{}", err);
                        final_output_results.push((id.clone(), duration_ms, Err(err)));
                    }
                }
            }

            // Execute ExtensionManager calls serially first (no lock held by caller)
            let mut ext_mgr_results: Vec<(
                String,
                u64,
                Result<Vec<super::mcp_connector::ToolContentBlock>, String>,
            )> = Vec::new();
            for (id, name, args) in &ext_mgr_calls {
                let started_at = Instant::now();
                if let Some(em) = ext_manager {
                    let result: Result<Vec<super::mcp_connector::ToolContentBlock>, String> =
                        if let Some(timeout_secs) = tool_timeout_secs {
                            match tokio::time::timeout(
                                Duration::from_secs(timeout_secs),
                                em.call_tool_rich(name, args.clone()),
                            )
                            .await
                            {
                                Ok(Ok(blocks)) => Ok(blocks),
                                Ok(Err(e)) => Err(format!("Error: {}", e)),
                                Err(_) => Err(format!(
                                    "Error: tool '{}' timed out after {}s",
                                    name, timeout_secs
                                )),
                            }
                        } else {
                            match em.call_tool_rich(name, args.clone()).await {
                                Ok(blocks) => Ok(blocks),
                                Err(e) => Err(format!("Error: {}", e)),
                            }
                        };
                    let duration_ms = started_at.elapsed().as_millis() as u64;
                    match result {
                        Ok(blocks) => ext_mgr_results.push((id.clone(), duration_ms, Ok(blocks))),
                        Err(err) => {
                            tracing::warn!("{}", err);
                            ext_mgr_results.push((id.clone(), duration_ms, Err(err)));
                        }
                    }
                } else {
                    let duration_ms = started_at.elapsed().as_millis() as u64;
                    ext_mgr_results.push((
                        id.clone(),
                        duration_ms,
                        Err("ExtensionManager not enabled".to_string()),
                    ));
                }
            }

            // Execute regular tools concurrently (with read lock)
            let ds = dynamic_state.clone();
            let futures: Vec<_> = regular_calls
                .iter()
                .map(|(id, name, args)| {
                    let id = id.clone();
                    let name = name.clone();
                    let args = args.clone();
                    let ds = ds.clone();
                    let tool_timeout_secs = tool_timeout_secs;
                    let ct = cancel_token.clone();
                    async move {
                        let started_at = Instant::now();
                        let result: Result<Vec<super::mcp_connector::ToolContentBlock>, String> =
                            if let Some(timeout_secs) = tool_timeout_secs {
                                match tokio::time::timeout(
                                    Duration::from_secs(timeout_secs),
                                    async {
                                        let state = ds.read().await;
                                        if state.platform.can_handle(&name) {
                                            state.platform.call_tool_rich(&name, args).await
                                        } else if let Some(ref m) = state.mcp {
                                            m.call_tool_rich(&name, args, ct.clone()).await
                                        } else {
                                            Err(anyhow!("No handler for tool: {}", name))
                                        }
                                    },
                                )
                                .await
                                {
                                    Ok(Ok(blocks)) => Ok(blocks),
                                    Ok(Err(e)) => Err(format!("Error: {}", e)),
                                    Err(_) => Err(format!(
                                        "Error: tool '{}' timed out after {}s",
                                        name, timeout_secs
                                    )),
                                }
                            } else {
                                let state = ds.read().await;
                                if state.platform.can_handle(&name) {
                                    state
                                        .platform
                                        .call_tool_rich(&name, args)
                                        .await
                                        .map_err(|e| format!("Error: {}", e))
                                } else if let Some(ref m) = state.mcp {
                                    m.call_tool_rich(&name, args, ct.clone())
                                        .await
                                        .map_err(|e| format!("Error: {}", e))
                                } else {
                                    Err(format!("Error: No handler for tool: {}", name))
                                }
                            };
                        let duration_ms = started_at.elapsed().as_millis() as u64;
                        match result {
                            Ok(blocks) => (id, duration_ms, Ok(blocks)),
                            Err(err) => {
                                tracing::warn!("{}", err);
                                (id, duration_ms, Err(err))
                            }
                        }
                    }
                })
                .collect();

            let regular_results = tokio::select! {
                res = join_all(futures) => res,
                _ = cancel_token.cancelled() => {
                    return Err(anyhow!("Task cancelled during tool execution"));
                }
            };

            // Merge final_output, ExtensionManager and regular results
            let mut results = ext_mgr_results;
            results.extend(final_output_results);
            results.extend(regular_results);

            // Build tool response message
            let mut tool_response_msg = Message::user();
            let mut this_turn_had_tool_failure = !denied.is_empty();

            // Add denied tool responses (repetition + security)
            for (id, name, reason) in &denied {
                self.task_manager
                    .broadcast(
                        task_id,
                        StreamEvent::ToolResult {
                            id: id.clone(),
                            success: false,
                            content: reason.clone(),
                            name: Some(name.clone()),
                            duration_ms: None,
                        },
                    )
                    .await;
                tool_response_msg = tool_response_msg.with_tool_response(
                    id.clone(),
                    Err(rmcp::ErrorData::new(
                        rmcp::model::ErrorCode::INTERNAL_ERROR,
                        reason.clone(),
                        None,
                    )),
                );
            }

            // Add actual tool results
            for (id, duration_ms, result) in results {
                match result {
                    Ok(blocks) => {
                        if portal_restricted {
                            let is_final_output = tool_name_by_id
                                .get(&id)
                                .map(|name| name == FINAL_OUTPUT_TOOL_NAME)
                                .unwrap_or(false);
                            if !is_final_output {
                                portal_successful_tool_calls += 1;
                            }
                        }
                        let text_repr = Self::tool_blocks_to_text(&blocks);
                        let sse_content = if text_repr.len() > 2000 {
                            format!("{}...", Self::safe_truncate(&text_repr, 2000))
                        } else {
                            text_repr.clone()
                        };
                        self.task_manager
                            .broadcast(
                                task_id,
                                StreamEvent::ToolResult {
                                    id: id.clone(),
                                    success: true,
                                    content: sse_content,
                                    name: tool_name_by_id.get(&id).cloned(),
                                    duration_ms: Some(duration_ms),
                                },
                            )
                            .await;

                        if let Some(tool_name) = tool_name_by_id.get(&id) {
                            if Self::tool_may_change_workspace(tool_name) {
                                self.task_manager
                                    .broadcast(
                                        task_id,
                                        StreamEvent::WorkspaceChanged {
                                            tool_name: tool_name.clone(),
                                        },
                                    )
                                    .await;
                            }
                        }

                        // Convert ToolContentBlocks to rmcp Content items
                        let content_items: Vec<rmcp::model::Content> = blocks
                            .iter()
                            .map(|b| match b {
                                super::mcp_connector::ToolContentBlock::Text(text) => {
                                    let truncated = Self::truncate_tool_result(text.clone());
                                    rmcp::model::Content::text(truncated)
                                }
                                super::mcp_connector::ToolContentBlock::Image {
                                    mime_type,
                                    data,
                                } => rmcp::model::Content::image(data.clone(), mime_type.clone()),
                            })
                            .collect();

                        let call_result = rmcp::model::CallToolResult {
                            content: content_items,
                            structured_content: None,
                            is_error: Some(false),
                            meta: None,
                        };
                        tool_response_msg =
                            tool_response_msg.with_tool_response(id, Ok(call_result));
                    }
                    Err(err_text) => {
                        this_turn_had_tool_failure = true;
                        let sse_content = if err_text.len() > 2000 {
                            format!("{}...", Self::safe_truncate(&err_text, 2000))
                        } else {
                            err_text.clone()
                        };
                        self.task_manager
                            .broadcast(
                                task_id,
                                StreamEvent::ToolResult {
                                    id: id.clone(),
                                    success: false,
                                    content: sse_content,
                                    name: tool_name_by_id.get(&id).cloned(),
                                    duration_ms: Some(duration_ms),
                                },
                            )
                            .await;

                        tool_response_msg = tool_response_msg.with_tool_response(
                            id,
                            Err(rmcp::ErrorData::new(
                                rmcp::model::ErrorCode::INTERNAL_ERROR,
                                Self::truncate_tool_result(err_text),
                                None,
                            )),
                        );
                    }
                }
            }
            previous_turn_had_tool_failure = this_turn_had_tool_failure;

            messages.push(tool_response_msg);

            self.task_manager
                .broadcast(
                    task_id,
                    StreamEvent::Status {
                        status: "running".to_string(),
                    },
                )
                .await;

            // Persist intermediate session state after each turn (crash recovery)
            // This ensures progress is not lost if the task is interrupted mid-execution.
            if turn % 3 == 2 {
                // Save every 3 turns to balance durability vs write overhead
                self.save_session_state(
                    session_id,
                    &messages,
                    accumulated_input,
                    accumulated_output,
                )
                .await;
            }
        }

        // Warn if configured max turns reached without natural completion.
        if completed_due_to_max_turns {
            if let Some(max_turns_limit) = max_turns {
                tracing::warn!(
                    "Unified loop reached max turns ({}) for task {}",
                    max_turns_limit,
                    task_id
                );
                let warning = format!(
                    "\n[Warning: Agent reached maximum turn limit ({}). Task may be incomplete.]",
                    max_turns_limit
                );
                all_text.push_str(&warning);
                self.task_manager
                    .broadcast(task_id, StreamEvent::Text { content: warning })
                    .await;
            }
        }

        // Save final accumulated text
        if !all_text.is_empty() {
            self.save_task_result(task_id, TaskResultType::Message, &all_text)
                .await?;
        }

        // Save session state
        self.save_session_state(session_id, &messages, accumulated_input, accumulated_output)
            .await;

        Ok(())
    }

    // ========================================
    // Streaming, compaction, and session helpers
    // ========================================

    /// Call Provider's streaming API, broadcast text deltas, return complete Message + Usage
    async fn call_provider_streaming(
        &self,
        task_id: &str,
        provider: &Arc<dyn Provider>,
        system_prompt: &str,
        messages: &[Message],
        tools: &[rmcp::model::Tool],
        cancel_token: &CancellationToken,
    ) -> Result<(Message, Option<ProviderUsage>)> {
        self.task_manager
            .broadcast(
                task_id,
                StreamEvent::Status {
                    status: "llm_call".to_string(),
                },
            )
            .await;

        // Try streaming first, fall back to complete() if not implemented
        let stream_result = provider.stream(system_prompt, messages, tools).await;

        let mut msg_stream = match stream_result {
            Ok(s) => s,
            Err(agime::providers::errors::ProviderError::NotImplemented(_)) => {
                // Fallback to non-streaming complete()
                let (msg, usage) = tokio::select! {
                    res = provider.complete(system_prompt, messages, tools) => {
                        res.map_err(anyhow::Error::from)?
                    }
                    _ = cancel_token.cancelled() => {
                        return Err(anyhow!("Task cancelled during API call"));
                    }
                };
                // Broadcast complete text at once
                let text = msg.as_concat_text();
                if !text.is_empty() {
                    self.task_manager
                        .broadcast(task_id, StreamEvent::Text { content: text })
                        .await;
                }
                return Ok((msg, Some(usage)));
            }
            Err(e) => return Err(anyhow::Error::from(e)),
        };

        // Consume the stream, accumulating deltas into a single message.
        // Provider streams yield incremental deltas (not accumulated messages),
        // so we must collect all text/thinking fragments and tool requests here.
        let mut accumulated_text = String::new();
        let mut accumulated_thinking = String::new();
        let mut thinking_signature = String::new();
        let mut tool_requests: Vec<MessageContent> = Vec::new();
        let mut final_usage: Option<ProviderUsage> = None;
        let mut got_any_message = false;

        let chunk_timeout_secs = std::env::var("TEAM_PROVIDER_CHUNK_TIMEOUT_SECS")
            .ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(10 * 60);
        let chunk_timeout = Duration::from_secs(chunk_timeout_secs);
        loop {
            tokio::select! {
                item = tokio::time::timeout(chunk_timeout, msg_stream.next()) => {
                    match item {
                        Err(_) => {
                            return Err(anyhow!("Provider stream timed out (no data for {} seconds)", chunk_timeout_secs));
                        }
                        Ok(item) => match item {
                        Some(Ok((msg_opt, usage_opt))) => {
                            if let Some(ref msg) = msg_opt {
                                got_any_message = true;
                                for part in &msg.content {
                                    match part {
                                        MessageContent::Text(tc) => {
                                            if !tc.text.is_empty() {
                                                accumulated_text.push_str(&tc.text);
                                                self.task_manager
                                                    .broadcast(
                                                        task_id,
                                                        StreamEvent::Text {
                                                            content: tc.text.clone(),
                                                        },
                                                    )
                                                    .await;
                                            }
                                        }
                                        MessageContent::Thinking(tc) => {
                                            if !tc.thinking.is_empty() {
                                                accumulated_thinking.push_str(&tc.thinking);
                                                thinking_signature = tc.signature.clone();
                                                self.task_manager
                                                    .broadcast(
                                                        task_id,
                                                        StreamEvent::Thinking {
                                                            content: tc.thinking.clone(),
                                                        },
                                                    )
                                                    .await;
                                            }
                                        }
                                        MessageContent::ToolRequest(_) => {
                                            // Tool requests are already accumulated
                                            // by the stream (yielded once complete)
                                            tool_requests.push(part.clone());
                                        }
                                        other => {
                                            // RedactedThinking, etc. — keep as-is
                                            tool_requests.push(other.clone());
                                        }
                                    }
                                }
                            }
                            if usage_opt.is_some() {
                                final_usage = usage_opt;
                            }
                        }
                        Some(Err(e)) => {
                            return Err(anyhow::Error::from(e));
                        }
                        None => break, // Stream ended
                    } // inner match (stream item)
                    } // outer match (timeout result)
                }
                _ = cancel_token.cancelled() => {
                    return Err(anyhow!("Task cancelled during streaming"));
                }
            }
        }

        if !got_any_message {
            return Err(anyhow!("Stream ended without producing a message"));
        }

        // Build the accumulated message with all collected content
        let mut content: Vec<MessageContent> = Vec::new();
        if !accumulated_thinking.is_empty() {
            content.push(MessageContent::thinking(
                accumulated_thinking,
                thinking_signature,
            ));
        }
        if !accumulated_text.is_empty() {
            content.push(MessageContent::text(accumulated_text));
        }
        content.extend(tool_requests);

        let message = Message::new(
            rmcp::model::Role::Assistant,
            chrono::Utc::now().timestamp(),
            content,
        );
        Ok((message, final_usage))
    }

    /// Check if context compaction is needed based on token count vs context limit
    async fn check_compaction_needed(
        &self,
        provider: &Arc<dyn Provider>,
        system_prompt: &str,
        messages: &[Message],
        tools: &[rmcp::model::Tool],
        cached_tokens: Option<i32>,
    ) -> Result<bool> {
        let context_limit = provider.get_model_config().context_limit();
        let threshold = DEFAULT_COMPACTION_THRESHOLD;

        let current_tokens = match cached_tokens {
            Some(t) if t > 0 => t as usize,
            _ => {
                let counter = create_token_counter()
                    .await
                    .map_err(|e| anyhow!("Token counter: {}", e))?;
                counter.count_chat_tokens(system_prompt, messages, tools)
            }
        };

        let ratio = current_tokens as f64 / context_limit as f64;
        tracing::debug!(
            "Compaction check: {}/{} = {:.2} (threshold: {:.2})",
            current_tokens,
            context_limit,
            ratio,
            threshold
        );
        Ok(ratio > threshold)
    }

    /// Save session state to MongoDB
    async fn save_session_state(
        &self,
        session_id: &str,
        messages: &[Message],
        input_tokens: i32,
        output_tokens: i32,
    ) {
        let messages_json = match serde_json::to_string(messages) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("Failed to serialize messages: {}", e);
                return;
            }
        };
        let msg_count = messages.len() as i32;
        let total = if input_tokens > 0 || output_tokens > 0 {
            Some(input_tokens + output_tokens)
        } else {
            None
        };

        if let Err(e) = self
            .agent_service
            .update_session_messages(
                session_id,
                &messages_json,
                msg_count,
                total,
                if input_tokens > 0 {
                    Some(input_tokens)
                } else {
                    None
                },
                if output_tokens > 0 {
                    Some(output_tokens)
                } else {
                    None
                },
            )
            .await
        {
            tracing::warn!("Failed to save session {}: {}", session_id, e);
        }
    }

    // ========================================
    // Database helper methods (MongoDB)
    // ========================================

    /// Get task by ID
    async fn get_task(&self, task_id: &str) -> Result<Option<AgentTask>> {
        let doc = self
            .tasks()
            .find_one(doc! { "task_id": task_id }, None)
            .await?;
        Ok(doc.map(|d| d.into()))
    }

    /// Get agent by ID (includes API key for execution)
    async fn get_agent(&self, agent_id: &str) -> Result<Option<TeamAgent>> {
        let doc = self
            .agents()
            .find_one(doc! { "agent_id": agent_id }, None)
            .await?;

        // Convert to TeamAgent but keep api_key for execution
        Ok(doc.map(|d| {
            let api_key = d.api_key.clone();
            let mut agent: TeamAgent = d.into();
            agent.api_key = api_key; // Restore API key for execution
            agent
        }))
    }

    /// Update task status with precondition check to prevent race conditions.
    /// Only transitions from valid prior states are allowed:
    ///   Approved → Running, Running → Completed/Failed
    /// Cancelled tasks are never overwritten.
    /// Returns `Err` if the transition was rejected (e.g. task already cancelled).
    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> Result<()> {
        let now = Utc::now();
        let mut set_doc = doc! { "status": status.to_string() };

        // Determine valid prior statuses for this transition
        let allowed_from = match status {
            TaskStatus::Running => vec!["approved"],
            TaskStatus::Completed | TaskStatus::Failed => vec!["running"],
            _ => vec![],
        };

        let filter = if allowed_from.is_empty() {
            doc! { "task_id": task_id }
        } else {
            doc! { "task_id": task_id, "status": { "$in": &allowed_from } }
        };

        if status == TaskStatus::Running {
            set_doc.insert("started_at", bson::DateTime::from_chrono(now));
        }

        if status == TaskStatus::Completed || status == TaskStatus::Failed {
            set_doc.insert("completed_at", bson::DateTime::from_chrono(now));
        }

        let result = self
            .tasks()
            .update_one(filter, doc! { "$set": set_doc }, None)
            .await?;

        if result.modified_count == 0 {
            tracing::warn!(
                "update_task_status: no update for task {} to {:?} (current status not in {:?})",
                task_id,
                status,
                allowed_from
            );
            return Err(anyhow!(
                "Task {} status transition to {:?} rejected (already cancelled or terminal)",
                task_id,
                status
            ));
        }

        Ok(())
    }

    /// Update task with error. Only updates if status is running or approved.
    async fn update_task_error(&self, task_id: &str, error: &str) -> Result<()> {
        let now = Utc::now();

        let result = self
            .tasks()
            .update_one(
                doc! { "task_id": task_id, "status": { "$in": ["running", "approved"] } },
                doc! { "$set": {
                    "status": "failed",
                    "error_message": error,
                    "completed_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.modified_count == 0 {
            tracing::warn!(
                "update_task_error: no update for task {} (already terminal?)",
                task_id
            );
        }

        Ok(())
    }

    /// Save task result
    async fn save_task_result(
        &self,
        task_id: &str,
        result_type: TaskResultType,
        content: &str,
    ) -> Result<()> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();

        let doc = doc! {
            "result_id": &id,
            "task_id": task_id,
            "result_type": result_type.to_string(),
            "content": doc! { "text": content },
            "created_at": bson::DateTime::from_chrono(now)
        };

        self.results().insert_one(doc, None).await?;

        Ok(())
    }
}
