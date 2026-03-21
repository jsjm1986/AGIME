//! Platform Extension Runner for in-process platform extensions
//!
//! Runs platform extensions (Skills, Team, Todo) directly in-process,
//! collecting tool definitions and dispatching tool calls.
//! Works alongside McpConnector (subprocess MCP) to provide a unified tool interface.

use agime::agents::extension::{PlatformExtensionContext, PLATFORM_EXTENSIONS};
use agime::agents::mcp_client::McpClientTrait;
use agime_team::db::MongoDb;
use agime_team::models::{AgentExtensionConfig, BuiltinExtension};
use anyhow::{anyhow, Result};
use rmcp::model::TaskSupport;
use std::collections::HashSet;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use super::avatar_governance_tools::{AvatarGovernanceRole, AvatarGovernanceToolsProvider};
use super::developer_tools::DeveloperToolsProvider;
use super::document_tools::DocumentToolsProvider;
use super::mcp_connector::{McpConnector, ToolContentBlock};
use super::mission_manager::MissionManager;
use super::mission_monitor_tools::MissionMonitorToolsProvider;
use super::mission_preflight_tools::MissionPreflightToolsProvider;
use super::portal_tools::PortalToolsProvider;
use super::skill_registry_tools::SkillRegistryToolsProvider;
use super::team_mcp_tools::TeamMcpToolsProvider;
use super::team_skill_tools::TeamSkillToolsProvider;

/// Entry for a single platform extension instance
struct PlatformExtensionEntry {
    name: String,
    instructions: String,
    has_resources: bool,
    client: Box<dyn McpClientTrait>,
    tools: Vec<PlatformToolDef>,
}

/// Tool definition within a platform extension
#[derive(Debug, Clone)]
struct PlatformToolDef {
    /// Prefixed name: "ext_name__tool_name"
    prefixed_name: String,
    /// Original tool name within the extension
    original_name: String,
    description: String,
    input_schema: serde_json::Value,
    execution: Option<rmcp::model::ToolExecution>,
}

/// Runs platform extensions in-process, providing tool listing and call dispatch.
pub struct PlatformExtensionRunner {
    extensions: Vec<PlatformExtensionEntry>,
}

impl PlatformExtensionRunner {
    fn is_blocked_platform_key(key: &str) -> bool {
        matches!(key, "team" | "chat_recall" | "extension_manager")
    }

    fn remap_legacy_tool_name<'a>(&self, tool_name: &'a str) -> &'a str {
        match tool_name {
            "team__team_list_installed" | "team_list_installed" => "team_skills__search",
            "team__team_load_skill" | "team_load_skill" => "team_skills__load",
            _ => tool_name,
        }
    }

    fn task_calls_enabled() -> bool {
        std::env::var("TEAM_MCP_ENABLE_TASK_CALLS")
            .ok()
            .map(|v| {
                let normalized = v.trim().to_ascii_lowercase();
                normalized == "1" || normalized == "true" || normalized == "yes"
            })
            .unwrap_or(true)
    }

    fn task_payload_for_execution(
        execution: Option<&rmcp::model::ToolExecution>,
    ) -> Result<Option<serde_json::Map<String, serde_json::Value>>> {
        let Some(task_support) = execution.and_then(|e| e.task_support) else {
            return Ok(None);
        };

        match task_support {
            TaskSupport::Forbidden => Ok(None),
            TaskSupport::Optional | TaskSupport::Required => {
                if Self::task_calls_enabled() {
                    Ok(Some(serde_json::Map::new()))
                } else if task_support == TaskSupport::Required {
                    Err(anyhow!(
                        "Tool requires task invocation, but TEAM_MCP_ENABLE_TASK_CALLS is disabled"
                    ))
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn normalize_document_access_mode(mode: Option<&str>) -> Option<String> {
        mode.map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase())
    }

    /// Create a new runner by instantiating enabled platform extensions.
    ///
    /// Supported extensions: Skills, Team, Todo, DocumentTools, PortalTools.
    /// ExtensionManager and ChatRecall are skipped (not applicable in team server context).
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        enabled_extensions: &[AgentExtensionConfig],
        db: Option<Arc<MongoDb>>,
        team_id: Option<&str>,
        actor_user_id: Option<&str>,
        session_source: Option<&str>,
        session_id: Option<&str>,
        mission_id: Option<&str>,
        agent_id: Option<&str>,
        _enable_team_skills_on_demand: bool,
        workspace_path: Option<&str>,
        workspace_root: Option<&str>,
        portal_base_url: Option<&str>,
        allowed_extension_names: Option<&HashSet<String>>,
        allowed_skill_ids: Option<&HashSet<String>>,
        attached_document_ids: Option<&[String]>,
        portal_restricted: bool,
        document_access_mode: Option<&str>,
        force_portal_tools: bool,
        mission_manager: Option<Arc<MissionManager>>,
    ) -> Self {
        let mut extensions = Vec::new();

        for ext_config in enabled_extensions {
            if !ext_config.enabled {
                continue;
            }

            let ext_name = ext_config.extension.name();
            let is_allowed = allowed_extension_names
                .map(|set| {
                    if ext_name == "skills" {
                        set.contains("skills") || set.contains("team_skills")
                    } else {
                        set.contains(ext_name)
                    }
                })
                .unwrap_or(true);
            if !is_allowed {
                continue;
            }

            // Team server always resolves Skills to the team-backed provider and
            // never falls back to the local filesystem-scanning extension.
            if ext_config.extension == BuiltinExtension::Skills {
                if let (Some(db), Some(tid)) = (&db, team_id) {
                    let provider = TeamSkillToolsProvider::new(
                        db.clone(),
                        tid.to_string(),
                        allowed_skill_ids.cloned(),
                    );
                    match Self::init_from_client("team_skills", Box::new(provider)).await {
                        Ok(entry) => {
                            tracing::info!(
                                "Platform extension 'team_skills' ready: {} tools",
                                entry.tools.len()
                            );
                            extensions.push(entry);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to init team_skills: {}", e);
                        }
                    }
                    continue;
                }
                tracing::warn!(
                    "Skipping skills extension because team-backed skills require db/team context"
                );
                continue;
            }

            // Handle Developer in-process — eliminates subprocess overhead
            if ext_config.extension == BuiltinExtension::Developer {
                if let Some(entry) = Self::try_init_developer(workspace_path).await {
                    extensions.push(entry);
                }
                continue;
            }

            // Handle DocumentTools specially — it needs db/team context
            if ext_config.extension == BuiltinExtension::DocumentTools {
                if allowed_extension_names
                    .map(|set| set.contains("document_tools"))
                    .unwrap_or(true)
                {
                    if let Some(entry) = Self::try_init_document_tools(
                        &db,
                        team_id,
                        session_id,
                        mission_id,
                        agent_id,
                        workspace_path,
                        attached_document_ids,
                        portal_restricted,
                        document_access_mode,
                    )
                    .await
                    {
                        extensions.push(entry);
                    }
                }
                continue;
            }

            if ext_config.extension == BuiltinExtension::SkillRegistry {
                if let Some(entry) =
                    Self::try_init_skill_registry(&db, team_id, actor_user_id, agent_id).await
                {
                    extensions.push(entry);
                }
                continue;
            }

            // ExtensionManager, ChatRecall, and the legacy Team collaboration
            // extension are not loaded in team server runtime. Team-backed
            // skills now live behind `team_skills`, and keeping the old `team`
            // tools exposed leads the model to call legacy `team__*` APIs such
            // as `team_list_installed`, which still depend on API-key style
            // auth paths.
            if matches!(
                ext_config.extension,
                BuiltinExtension::ExtensionManager
                    | BuiltinExtension::ChatRecall
                    | BuiltinExtension::Team
            ) {
                continue;
            }

            // Map BuiltinExtension enum to PLATFORM_EXTENSIONS key.
            let platform_key = match ext_config.extension {
                BuiltinExtension::Todo => Some("todo"),
                _ => None,
            };

            let key = match platform_key {
                Some(k) => k,
                None => continue,
            };

            match Self::init_one(key).await {
                Ok(entry) => {
                    tracing::info!(
                        "Platform extension '{}' ready: {} tools",
                        entry.name,
                        entry.tools.len()
                    );
                    extensions.push(entry);
                }
                Err(e) => {
                    tracing::warn!("Failed to init platform extension '{}': {}", key, e);
                }
            }
        }

        // Fallback: always load DocumentTools if db+team context available and not already loaded
        if allowed_extension_names
            .map(|set| set.contains("document_tools"))
            .unwrap_or(true)
            && !extensions.iter().any(|e| e.name == "document_tools")
        {
            if let Some(entry) = Self::try_init_document_tools(
                &db,
                team_id,
                session_id,
                mission_id,
                agent_id,
                workspace_path,
                attached_document_ids,
                portal_restricted,
                document_access_mode,
            )
            .await
            {
                tracing::info!(
                    "Platform extension 'document_tools' loaded as fallback: {} tools",
                    entry.tools.len()
                );
                extensions.push(entry);
            }
        }

        // Fallback: load PortalTools only when explicitly in allowed_extensions whitelist.
        // Unlike DocumentTools (always useful), PortalTools should only be available
        // to agents that are explicitly configured for portal management.
        let should_load_portal_tools = force_portal_tools
            || allowed_extension_names
                .map(|set| set.contains("portal_tools"))
                .unwrap_or(false);
        if should_load_portal_tools && !extensions.iter().any(|e| e.name == "portal_tools") {
            if let Some(entry) = Self::try_init_portal_tools(
                &db,
                team_id,
                agent_id,
                actor_user_id,
                session_source,
                portal_base_url,
                workspace_root,
            )
            .await
            {
                tracing::info!(
                    "Platform extension 'portal_tools' loaded as fallback: {} tools",
                    entry.tools.len()
                );
                extensions.push(entry);
            }
        }

        if !extensions.iter().any(|e| e.name == "avatar_governance") {
            if let Some(entry) =
                Self::try_init_avatar_governance(&db, team_id, agent_id, session_source, session_id)
                    .await
            {
                tracing::info!(
                    "Platform extension 'avatar_governance' loaded by session source: {} tools",
                    entry.tools.len()
                );
                extensions.push(entry);
            }
        }

        if !extensions.iter().any(|e| e.name == "team_mcp") {
            if let Some(entry) =
                Self::try_init_team_mcp(&db, team_id, actor_user_id, agent_id).await
            {
                tracing::info!(
                    "Platform extension 'team_mcp' loaded by team context: {} tools",
                    entry.tools.len()
                );
                extensions.push(entry);
            }
        }

        // Mission-only hard gate: always load mission preflight tool when mission context exists.
        // This tool is intentionally unavailable for normal chat sessions.
        if mission_id.is_some() && !extensions.iter().any(|e| e.name == "mission_preflight") {
            if let Some(entry) = Self::try_init_mission_preflight().await {
                tracing::info!(
                    "Platform extension 'mission_preflight' loaded for mission mode: {} tools",
                    entry.tools.len()
                );
                extensions.push(entry);
            }
        }

        if mission_id.is_some() && !extensions.iter().any(|e| e.name == "mission_monitor") {
            if let Some(entry) = Self::try_init_mission_monitor(
                &db,
                &mission_manager,
                team_id,
                actor_user_id,
                mission_id,
                workspace_root,
            )
            .await
            {
                tracing::info!(
                    "Platform extension 'mission_monitor' loaded for mission mode: {} tools",
                    entry.tools.len()
                );
                extensions.push(entry);
            }
        }

        Self { extensions }
    }

    /// Try to initialize DocumentTools if db+team context is available.
    /// Returns `None` if context is missing or initialization fails.
    #[allow(clippy::too_many_arguments)]
    async fn try_init_document_tools(
        db: &Option<Arc<MongoDb>>,
        team_id: Option<&str>,
        session_id: Option<&str>,
        mission_id: Option<&str>,
        agent_id: Option<&str>,
        workspace_path: Option<&str>,
        attached_document_ids: Option<&[String]>,
        portal_restricted: bool,
        document_access_mode: Option<&str>,
    ) -> Option<PlatformExtensionEntry> {
        let (db, tid) = match (db, team_id) {
            (Some(db), Some(tid)) => (db, tid),
            _ => return None,
        };
        let normalized_mode = Self::normalize_document_access_mode(document_access_mode);
        // Keep portal runtime safety by default, but allow full-scope document access
        // for explicitly full-access sessions (e.g. internal portal coding agent).
        let restrict_to_allowed_documents =
            portal_restricted && normalized_mode.as_deref() != Some("full");
        let provider = DocumentToolsProvider::new(
            db.clone(),
            tid.to_string(),
            session_id.map(String::from),
            mission_id.map(String::from),
            agent_id.map(String::from),
            workspace_path.map(String::from),
            attached_document_ids.map(|items| items.to_vec()),
            restrict_to_allowed_documents,
            document_access_mode.map(|s| s.to_string()),
        );
        match Self::init_from_client("document_tools", Box::new(provider)).await {
            Ok(entry) => {
                tracing::info!(
                    "Platform extension 'document_tools' ready: {} tools",
                    entry.tools.len()
                );
                Some(entry)
            }
            Err(e) => {
                tracing::warn!("Failed to init document_tools: {}", e);
                None
            }
        }
    }

    /// Try to initialize Developer extension in-process.
    async fn try_init_developer(workspace_path: Option<&str>) -> Option<PlatformExtensionEntry> {
        match DeveloperToolsProvider::new(workspace_path).await {
            Ok(provider) => match Self::init_from_client("developer", Box::new(provider)).await {
                Ok(entry) => {
                    tracing::info!(
                        "Platform extension 'developer' ready (in-process): {} tools",
                        entry.tools.len()
                    );
                    Some(entry)
                }
                Err(e) => {
                    tracing::warn!("Failed to init in-process developer: {}", e);
                    None
                }
            },
            Err(e) => {
                tracing::warn!("Failed to create in-process developer server: {}", e);
                None
            }
        }
    }

    /// Try to initialize PortalTools if db+team+base_url context is available.
    /// Returns `None` if context is missing or initialization fails.
    async fn try_init_portal_tools(
        db: &Option<Arc<MongoDb>>,
        team_id: Option<&str>,
        agent_id: Option<&str>,
        actor_user_id: Option<&str>,
        session_source: Option<&str>,
        base_url: Option<&str>,
        workspace_root: Option<&str>,
    ) -> Option<PlatformExtensionEntry> {
        let (db, tid) = match (db, team_id) {
            (Some(db), Some(tid)) => (db, tid),
            _ => return None,
        };
        let url = base_url.unwrap_or("http://127.0.0.1:8080");
        let ws_root = workspace_root.unwrap_or("./data/workspaces");
        let provider = PortalToolsProvider::new(
            db.clone(),
            tid.to_string(),
            agent_id.map(str::to_string),
            actor_user_id.map(str::to_string),
            session_source.map(str::to_string),
            url.to_string(),
            ws_root.to_string(),
        );
        match Self::init_from_client("portal_tools", Box::new(provider)).await {
            Ok(entry) => {
                tracing::info!(
                    "Platform extension 'portal_tools' ready: {} tools",
                    entry.tools.len()
                );
                Some(entry)
            }
            Err(e) => {
                tracing::warn!("Failed to init portal_tools: {}", e);
                None
            }
        }
    }

    async fn try_init_skill_registry(
        db: &Option<Arc<MongoDb>>,
        team_id: Option<&str>,
        actor_user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Option<PlatformExtensionEntry> {
        let (db, tid) = match (db, team_id) {
            (Some(db), Some(tid)) => (db, tid),
            _ => return None,
        };
        let actor_id = actor_user_id
            .or(agent_id)
            .unwrap_or("agent")
            .trim()
            .to_string();
        let provider = SkillRegistryToolsProvider::new(db.clone(), tid.to_string(), actor_id);
        match Self::init_from_client("skill_registry", Box::new(provider)).await {
            Ok(entry) => {
                tracing::info!(
                    "Platform extension 'skill_registry' ready: {} tools",
                    entry.tools.len()
                );
                Some(entry)
            }
            Err(e) => {
                tracing::warn!("Failed to init skill_registry: {}", e);
                None
            }
        }
    }

    async fn try_init_team_mcp(
        db: &Option<Arc<MongoDb>>,
        team_id: Option<&str>,
        actor_user_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Option<PlatformExtensionEntry> {
        let (db, tid, actor_id) = match (db, team_id, actor_user_id) {
            (Some(db), Some(tid), Some(actor_id)) if !actor_id.trim().is_empty() => {
                (db, tid, actor_id.trim())
            }
            _ => return None,
        };
        let provider = TeamMcpToolsProvider::new(
            db.clone(),
            tid.to_string(),
            actor_id.to_string(),
            agent_id.map(str::to_string),
        );
        match Self::init_from_client("team_mcp", Box::new(provider)).await {
            Ok(entry) => Some(entry),
            Err(e) => {
                tracing::warn!("Failed to init team_mcp: {}", e);
                None
            }
        }
    }

    /// Initialize mission preflight extension.
    /// Only used in mission/task execution mode.
    async fn try_init_mission_preflight() -> Option<PlatformExtensionEntry> {
        let provider = MissionPreflightToolsProvider::new();
        match Self::init_from_client("mission_preflight", Box::new(provider)).await {
            Ok(entry) => Some(entry),
            Err(e) => {
                tracing::warn!("Failed to init mission_preflight: {}", e);
                None
            }
        }
    }

    async fn try_init_mission_monitor(
        db: &Option<Arc<MongoDb>>,
        mission_manager: &Option<Arc<MissionManager>>,
        team_id: Option<&str>,
        actor_user_id: Option<&str>,
        mission_id: Option<&str>,
        workspace_root: Option<&str>,
    ) -> Option<PlatformExtensionEntry> {
        let (db, mission_manager, tid, mid) = match (db, mission_manager, team_id, mission_id) {
            (Some(db), Some(mission_manager), Some(tid), Some(mid)) => {
                (db, mission_manager, tid, mid)
            }
            _ => return None,
        };
        let provider = MissionMonitorToolsProvider::new(
            db.clone(),
            mission_manager.clone(),
            tid.to_string(),
            actor_user_id.map(str::to_string),
            Some(mid.to_string()),
            workspace_root.unwrap_or("./data/workspaces").to_string(),
        );
        match Self::init_from_client("mission_monitor", Box::new(provider)).await {
            Ok(entry) => Some(entry),
            Err(e) => {
                tracing::warn!("Failed to init mission_monitor: {}", e);
                None
            }
        }
    }

    async fn try_init_avatar_governance(
        db: &Option<Arc<MongoDb>>,
        team_id: Option<&str>,
        agent_id: Option<&str>,
        session_source: Option<&str>,
        session_id: Option<&str>,
    ) -> Option<PlatformExtensionEntry> {
        let (db, tid) = match (db, team_id) {
            (Some(db), Some(tid)) => (db, tid),
            _ => return None,
        };
        let role = match session_source.unwrap_or_default() {
            "portal" => AvatarGovernanceRole::Service,
            "portal_manager" | "portal_coding" => AvatarGovernanceRole::Manager,
            _ => return None,
        };
        let provider = AvatarGovernanceToolsProvider::new(
            db.clone(),
            tid.to_string(),
            role,
            agent_id.map(str::to_string),
            session_source.map(str::to_string),
            session_id.map(str::to_string),
        );
        match Self::init_from_client("avatar_governance", Box::new(provider)).await {
            Ok(entry) => Some(entry),
            Err(e) => {
                tracing::warn!("Failed to init avatar_governance: {}", e);
                None
            }
        }
    }

    /// Initialize a single platform extension by its key in PLATFORM_EXTENSIONS.
    async fn init_one(key: &str) -> Result<PlatformExtensionEntry> {
        if Self::is_blocked_platform_key(key) {
            return Err(anyhow!(
                "Platform extension '{}' is disabled in team server runtime",
                key
            ));
        }
        let def = PLATFORM_EXTENSIONS
            .get(key)
            .ok_or_else(|| anyhow!("Platform extension '{}' not found in registry", key))?;

        // Create context with no session/manager (team server doesn't have these)
        let context = PlatformExtensionContext {
            session_id: None,
            extension_manager: None,
            tool_route_manager: None,
        };

        // Instantiate via factory (catch_unwind guards against .unwrap() panics in factories)
        let client = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (def.client_factory)(context)
        }))
        .map_err(|_| {
            anyhow!(
                "Platform extension '{}' panicked during initialization",
                key
            )
        })?;

        Self::build_entry(def.name, client, String::new()).await
    }

    /// Initialize from an already-constructed McpClientTrait (for DocumentTools etc.)
    async fn init_from_client(
        name: &str,
        client: Box<dyn McpClientTrait>,
    ) -> Result<PlatformExtensionEntry> {
        let default_instructions = "Read, create, search and list team documents.".to_string();
        Self::build_entry(name, client, default_instructions).await
    }

    /// Shared logic: list tools from a client and assemble a PlatformExtensionEntry.
    ///
    /// `default_instructions` is used when the client's `get_info()` returns `None`.
    async fn build_entry(
        name: &str,
        client: Box<dyn McpClientTrait>,
        default_instructions: String,
    ) -> Result<PlatformExtensionEntry> {
        let cancel = CancellationToken::new();
        let list_result = client
            .list_tools(None, cancel)
            .await
            .map_err(|e| anyhow!("list_tools failed for '{}': {:?}", name, e))?;

        let ext_name = name.to_string();
        let safe_ext = ext_name.replace("__", "_");
        let tools: Vec<PlatformToolDef> = list_result
            .tools
            .iter()
            .map(|tool| {
                let original_name = tool.name.to_string();
                // Sanitize names to prevent split_once ambiguity (same as McpConnector)
                let safe_tool = original_name.replace("__", "_");
                let prefixed_name = format!("{}__{}", safe_ext, safe_tool);

                let description = tool
                    .description
                    .as_ref()
                    .map(|d| d.to_string())
                    .unwrap_or_default();

                let input_schema = serde_json::to_value(&tool.input_schema)
                    .unwrap_or(serde_json::json!({"type": "object"}));

                PlatformToolDef {
                    prefixed_name,
                    original_name,
                    description,
                    input_schema,
                    execution: tool.execution.clone(),
                }
            })
            .collect();

        // Extract instructions and resource capability from server info
        let (instructions, has_resources) = match client.get_info() {
            Some(info) => {
                let instr = info.instructions.clone().unwrap_or_default();
                let has_res = info.capabilities.resources.is_some();
                (instr, has_res)
            }
            None => (default_instructions, false),
        };

        Ok(PlatformExtensionEntry {
            name: ext_name,
            instructions,
            has_resources,
            client,
            tools,
        })
    }

    pub fn tools_as_rmcp(&self) -> Vec<rmcp::model::Tool> {
        self.extensions
            .iter()
            .flat_map(|ext| &ext.tools)
            .map(|tool| rmcp::model::Tool {
                name: tool.prefixed_name.clone().into(),
                title: None,
                description: Some(tool.description.clone().into()),
                input_schema: serde_json::from_value(tool.input_schema.clone()).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: tool.execution.clone(),
                icons: None,
                meta: None,
            })
            .collect()
    }

    /// Check if a tool name belongs to a platform extension.
    pub fn can_handle(&self, tool_name: &str) -> bool {
        self.find_tool(tool_name).is_some()
    }

    /// Look up a tool by its prefixed name, returning the extension and tool definition.
    fn find_tool(&self, tool_name: &str) -> Option<(&PlatformExtensionEntry, &PlatformToolDef)> {
        let tool_name = self.remap_legacy_tool_name(tool_name);
        for ext in &self.extensions {
            if let Some(tool) = ext.tools.iter().find(|t| t.prefixed_name == tool_name) {
                return Some((ext, tool));
            }
        }
        None
    }

    /// Execute a tool call, returning structured content blocks.
    pub async fn call_tool_rich(
        &self,
        tool_name: &str,
        input: serde_json::Value,
    ) -> Result<Vec<ToolContentBlock>> {
        let resolved_tool_name = self.remap_legacy_tool_name(tool_name);
        let (ext, tool) = self
            .find_tool(resolved_tool_name)
            .ok_or_else(|| anyhow!("Platform tool not found: {}", tool_name))?;

        if resolved_tool_name != tool_name {
            tracing::warn!(
                requested_tool = %tool_name,
                remapped_tool = %resolved_tool_name,
                "Remapped legacy team tool name to modern team-skills runtime tool"
            );
        }

        // Build arguments as JsonObject
        let arguments = match input {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            other => Some(serde_json::Map::from_iter([("input".to_string(), other)])),
        };
        let task = Self::task_payload_for_execution(tool.execution.as_ref())?;

        let cancel = CancellationToken::new();
        let call_result = ext
            .client
            .call_tool_with_task(&tool.original_name, arguments, task, cancel)
            .await
            .map_err(|e| anyhow!("Platform tool call failed: {:?}", e))?;

        // Convert CallToolResult content to ToolContentBlock
        Ok(Self::extract_blocks(&call_result))
    }

    /// Extract ToolContentBlocks from a CallToolResult.
    /// Delegates to McpConnector's shared implementation to avoid duplication.
    fn extract_blocks(result: &rmcp::model::CallToolResult) -> Vec<ToolContentBlock> {
        McpConnector::extract_tool_result_blocks(result)
    }

    /// Dynamically add a platform extension by its key at runtime.
    /// Returns the list of new tool names added.
    pub async fn add_extension(&mut self, key: &str) -> Result<Vec<String>> {
        if Self::is_blocked_platform_key(key) {
            return Err(anyhow!(
                "Platform extension '{}' is disabled in team server runtime",
                key
            ));
        }
        // Don't add if already loaded
        if self.extensions.iter().any(|ext| ext.name == key) {
            return Err(anyhow!("Platform extension '{}' is already loaded", key));
        }

        let entry = Self::init_one(key).await?;
        let tool_names: Vec<String> = entry
            .tools
            .iter()
            .map(|t| t.prefixed_name.clone())
            .collect();
        tracing::info!(
            "Dynamically added platform extension '{}': {} tools",
            entry.name,
            entry.tools.len()
        );
        self.extensions.push(entry);
        Ok(tool_names)
    }

    /// Dynamically remove a platform extension by name.
    /// Returns the list of tool names that were removed.
    pub fn remove_extension(&mut self, name: &str) -> Result<Vec<String>> {
        let idx = self
            .extensions
            .iter()
            .position(|ext| ext.name == name)
            .ok_or_else(|| anyhow!("Platform extension '{}' not found", name))?;

        let entry = self.extensions.remove(idx);
        let tool_names: Vec<String> = entry
            .tools
            .iter()
            .map(|t| t.prefixed_name.clone())
            .collect();
        tracing::info!("Dynamically removed platform extension '{}'", name);
        Ok(tool_names)
    }

    /// Check if a platform extension with the given name is currently loaded.
    pub fn has_extension(&self, name: &str) -> bool {
        self.extensions.iter().any(|ext| ext.name == name)
    }

    /// Check if any tools are available.
    pub fn has_tools(&self) -> bool {
        self.extensions.iter().any(|ext| !ext.tools.is_empty())
    }

    /// Get the names of all loaded platform extensions.
    pub fn extension_names(&self) -> Vec<String> {
        self.extensions.iter().map(|ext| ext.name.clone()).collect()
    }

    /// Get ExtensionInfo for each loaded extension (with real instructions).
    pub fn extension_infos(&self) -> Vec<agime::agents::extension::ExtensionInfo> {
        self.extensions
            .iter()
            .map(|ext| {
                agime::agents::extension::ExtensionInfo::new(
                    &ext.name,
                    &ext.instructions,
                    ext.has_resources,
                )
            })
            .collect()
    }

    /// Collect MOIM (Message of Immediate Memory) from all platform extensions.
    /// Always returns Some with at least a timestamp, matching local agent behavior.
    pub async fn collect_moim(&self) -> Option<String> {
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let mut content = format!("<info-msg>\nIt is currently {}\n", timestamp);

        for ext in &self.extensions {
            if let Some(moim) = ext.client.get_moim().await {
                if !moim.trim().is_empty() {
                    content.push('\n');
                    content.push_str(&moim);
                }
            }
        }

        content.push_str("\n</info-msg>");
        Some(content)
    }
}
