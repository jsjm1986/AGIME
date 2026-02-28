//! Team Extension Manager Client
//!
//! Provides dynamic extension management for team server agents.
//! Allows agents to search, enable, and disable extensions during a conversation.
//! Adapted from the local ExtensionManager but uses shared state instead of
//! Weak<ExtensionManager> + Weak<ToolRouterIndexManager>.

use agime_team::models::{BuiltinExtension, CustomExtensionConfig, TeamAgent};
use agime_team::services::mongo::extension_service_mongo::ExtensionService;
use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use super::mcp_connector::{ApiCaller, McpConnector, ToolContentBlock};
use super::platform_runner::PlatformExtensionRunner;
use super::service_mongo::AgentService;

/// Shared mutable state for dynamic extension management.
/// Wrapped in Arc<RwLock<>> to allow the extension manager to modify
/// MCP connections and platform extensions at runtime.
pub struct DynamicExtensionState {
    pub mcp: Option<McpConnector>,
    pub platform: PlatformExtensionRunner,
    pub agent: TeamAgent,
    pub api_caller: Option<Arc<dyn ApiCaller>>,
}

impl DynamicExtensionState {
    /// Collect names of all currently active extensions (MCP + platform).
    pub fn active_extension_names(&self) -> HashSet<String> {
        let mut names = HashSet::new();
        if let Some(ref mcp) = self.mcp {
            names.extend(mcp.extension_names());
        }
        names.extend(self.platform.extension_names());
        names
    }
}

/// Tool name prefix for extension manager tools
const TOOL_PREFIX: &str = "extensionmanager__";

/// Team Extension Manager Client that provides tools for dynamic extension management.
///
/// Tools:
/// - `extensionmanager__search_available_extensions` — list extensions that can be enabled/disabled
/// - `extensionmanager__search_team_extensions` — list team shared extensions that can be installed
/// - `extensionmanager__install_team_extension` — install a team shared extension to current agent
/// - `extensionmanager__manage_extensions` — enable or disable an extension
/// - `extensionmanager__list_resources` — list resources from extensions
/// - `extensionmanager__read_resource` — read a specific resource
pub struct TeamExtensionManagerClient {
    state: Arc<RwLock<DynamicExtensionState>>,
    session_id: Option<String>,
    agent_service: Option<Arc<AgentService>>,
    db: Option<MongoDb>,
}

impl TeamExtensionManagerClient {
    pub fn new(state: Arc<RwLock<DynamicExtensionState>>) -> Self {
        Self {
            state,
            session_id: None,
            agent_service: None,
            db: None,
        }
    }

    /// Create with session persistence support
    pub fn with_session(
        state: Arc<RwLock<DynamicExtensionState>>,
        session_id: String,
        agent_service: Arc<AgentService>,
        db: MongoDb,
    ) -> Self {
        Self {
            state,
            session_id: Some(session_id),
            agent_service: Some(agent_service),
            db: Some(db),
        }
    }

    /// Check if a tool name belongs to the extension manager.
    pub fn can_handle(tool_name: &str) -> bool {
        tool_name.starts_with(TOOL_PREFIX)
    }

    /// Return extension manager tool definitions as rmcp::model::Tool.
    pub fn tools_as_rmcp() -> Vec<rmcp::model::Tool> {
        vec![
            Self::tool_search_available(),
            Self::tool_search_team_extensions(),
            Self::tool_install_team_extension(),
            Self::tool_manage_extensions(),
            Self::tool_list_resources(),
            Self::tool_read_resource(),
        ]
    }

    fn tool_search_available() -> rmcp::model::Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        });
        rmcp::model::Tool {
            name: format!("{}search_available_extensions", TOOL_PREFIX).into(),
            title: None,
            description: Some("List all extensions that can be enabled or disabled. Shows currently active extensions and available extensions that are not yet active.".into()),
            input_schema: serde_json::from_value(schema).expect("valid schema"),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }
    }

    fn tool_manage_extensions() -> rmcp::model::Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["enable", "disable"],
                    "description": "Whether to enable or disable the extension"
                },
                "extension_name": {
                    "type": "string",
                    "description": "Name of the extension to enable or disable"
                }
            },
            "required": ["action", "extension_name"],
            "additionalProperties": false
        });
        rmcp::model::Tool {
            name: format!("{}manage_extensions", TOOL_PREFIX).into(),
            title: None,
            description: Some("Enable or disable an extension. Enabling adds its tools to the conversation; disabling removes them.".into()),
            input_schema: serde_json::from_value(schema).expect("valid schema"),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }
    }

    fn tool_search_team_extensions() -> rmcp::model::Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Optional fuzzy query by extension name/description"
                },
                "reviewed_only": {
                    "type": "boolean",
                    "description": "Whether to show only security-reviewed extensions (default true)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum result count (default 20, max 100)",
                    "minimum": 1,
                    "maximum": 100
                }
            },
            "additionalProperties": false
        });
        rmcp::model::Tool {
            name: format!("{}search_team_extensions", TOOL_PREFIX).into(),
            title: None,
            description: Some("List installable team shared extensions. Returns extension id and name for installation.".into()),
            input_schema: serde_json::from_value(schema).expect("valid schema"),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }
    }

    fn tool_install_team_extension() -> rmcp::model::Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "extension_id": {
                    "type": "string",
                    "description": "Extension id from search_team_extensions results"
                },
                "extension_name": {
                    "type": "string",
                    "description": "Extension name (used when extension_id is not provided)"
                },
                "auto_enable": {
                    "type": "boolean",
                    "description": "Whether to immediately enable after install (default true)"
                }
            },
            "additionalProperties": false
        });
        rmcp::model::Tool {
            name: format!("{}install_team_extension", TOOL_PREFIX).into(),
            title: None,
            description: Some("Install a team shared extension to this agent, optionally enabling it immediately.".into()),
            input_schema: serde_json::from_value(schema).expect("valid schema"),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }
    }

    fn tool_list_resources() -> rmcp::model::Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "extension_name": {
                    "type": "string",
                    "description": "Name of the extension to list resources from (optional, lists all if omitted)"
                }
            },
            "additionalProperties": false
        });
        rmcp::model::Tool {
            name: format!("{}list_resources", TOOL_PREFIX).into(),
            title: None,
            description: Some("List available resources from active extensions.".into()),
            input_schema: serde_json::from_value(schema).expect("valid schema"),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }
    }

    fn tool_read_resource() -> rmcp::model::Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "uri": {
                    "type": "string",
                    "description": "URI of the resource to read"
                }
            },
            "required": ["uri"],
            "additionalProperties": false
        });
        rmcp::model::Tool {
            name: format!("{}read_resource", TOOL_PREFIX).into(),
            title: None,
            description: Some("Read a specific resource by URI from an active extension.".into()),
            input_schema: serde_json::from_value(schema).expect("valid schema"),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }
    }

    /// Dispatch a tool call to the appropriate handler.
    /// IMPORTANT: This method acquires write locks internally for manage_extensions,
    /// so callers must NOT hold any lock on the shared state.
    pub async fn call_tool_rich(&self, name: &str, args: Value) -> Result<Vec<ToolContentBlock>> {
        let tool_name = name
            .strip_prefix(TOOL_PREFIX)
            .ok_or_else(|| anyhow!("Not an extension manager tool: {}", name))?;

        match tool_name {
            "search_available_extensions" => self.handle_search().await,
            "search_team_extensions" => self.handle_search_team_extensions(args).await,
            "install_team_extension" => self.handle_install_team_extension(args).await,
            "manage_extensions" => self.handle_manage(args).await,
            "list_resources" => self.handle_list_resources(args).await,
            "read_resource" => self.handle_read_resource(args).await,
            _ => Err(anyhow!("Unknown extension manager tool: {}", tool_name)),
        }
    }

    /// Handle search_available_extensions: list active and available extensions.
    async fn handle_search(&self) -> Result<Vec<ToolContentBlock>> {
        let state = self.state.read().await;

        // Collect currently active extension names
        let mut active = state.active_extension_names();
        // TeamExtensionManagerClient provides extension_manager tools directly (not via platform_runner),
        // so add it to the active list to reflect that its functionality IS running.
        active.insert("extension_manager".to_string());

        // Collect all configured extensions from agent
        let all_configured = collect_all_extension_names(&state.agent);

        // Find available (configured but not active)
        let team_skills_active = active.contains("team_skills");
        let available: Vec<&String> = all_configured
            .iter()
            .filter(|name| !active.contains(name.as_str()))
            // Hide "skills" when "team_skills" (MongoDB-backed) is active
            .filter(|name| !(team_skills_active && name.as_str() == "skills"))
            // ChatRecall is not applicable in team server context
            .filter(|name| name.as_str() != "chat_recall")
            .collect();

        let mut output = String::new();
        output.push_str("## Active Extensions\n");
        if active.is_empty() {
            output.push_str("  (none)\n");
        } else {
            for name in &active {
                output.push_str(&format!("  - {} ✓\n", name));
            }
        }

        output.push_str("\n## Available Extensions (can be enabled)\n");
        if available.is_empty() {
            output.push_str("  (none — all configured extensions are active)\n");
        } else {
            for name in &available {
                let ext_type = classify_extension(&state.agent, name);
                match ext_type {
                    ExtensionType::BinaryNotFound(_) => {
                        output.push_str(&format!(
                            "  - {} ⚠ (requires 'agime' binary, not found)\n",
                            name
                        ));
                    }
                    _ => {
                        output.push_str(&format!("  - {}\n", name));
                    }
                }
            }
        }

        Ok(vec![ToolContentBlock::Text(output)])
    }

    async fn handle_search_team_extensions(&self, args: Value) -> Result<Vec<ToolContentBlock>> {
        let db = self
            .db
            .clone()
            .ok_or_else(|| anyhow!("search_team_extensions is unavailable: missing DB handle"))?;

        let query = args["query"].as_str().unwrap_or("").trim().to_string();
        let reviewed_only = args["reviewed_only"].as_bool().unwrap_or(true);
        let limit = args["limit"].as_u64().unwrap_or(20).clamp(1, 100) as usize;

        let team_id = { self.state.read().await.agent.team_id.clone() };
        let service = ExtensionService::new(db);
        let mut exts = if reviewed_only {
            service
                .list_reviewed_for_team(&team_id)
                .await
                .map_err(|e| anyhow!("Failed to load reviewed team extensions: {}", e))?
        } else {
            service
                .list_active_for_team(&team_id)
                .await
                .map_err(|e| anyhow!("Failed to load team extensions: {}", e))?
        };

        if !query.is_empty() {
            let normalized_query = normalize_ext_name(&query);
            let query_lower = query.to_lowercase();
            exts.retain(|e| {
                let name_hit = normalize_ext_name(&e.name).contains(&normalized_query);
                let desc_hit = e
                    .description
                    .as_deref()
                    .map(|d| d.to_lowercase().contains(&query_lower))
                    .unwrap_or(false);
                name_hit || desc_hit
            });
        }

        exts.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        let mut output = String::new();
        output.push_str("## Team Shared Extensions\n");
        output.push_str(&format!(
            "team_id: {} | reviewed_only: {}\n\n",
            team_id, reviewed_only
        ));
        if exts.is_empty() {
            output.push_str("  (none)\n");
            return Ok(vec![ToolContentBlock::Text(output)]);
        }

        for ext in exts.into_iter().take(limit) {
            let id = ext.id.map(|oid| oid.to_hex()).unwrap_or_default();
            let review = if ext.security_reviewed {
                "reviewed"
            } else {
                "pending_review"
            };
            output.push_str(&format!(
                "- {} (id: {}) [{}] type={} use_count={}\n",
                ext.name, id, review, ext.extension_type, ext.use_count
            ));
            if let Some(desc) = ext.description.as_deref().filter(|s| !s.trim().is_empty()) {
                output.push_str(&format!("  description: {}\n", desc.replace('\n', " ")));
            }
        }
        output.push_str(
            "\nUse extensionmanager__install_team_extension with extension_id (recommended).",
        );

        Ok(vec![ToolContentBlock::Text(output)])
    }

    async fn handle_install_team_extension(&self, args: Value) -> Result<Vec<ToolContentBlock>> {
        let db = self
            .db
            .clone()
            .ok_or_else(|| anyhow!("install_team_extension is unavailable: missing DB handle"))?;
        let agent_service = self.agent_service.as_ref().cloned().ok_or_else(|| {
            anyhow!("install_team_extension is unavailable: missing agent service")
        })?;

        let requested_id = args["extension_id"]
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let requested_name = args["extension_name"]
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let auto_enable = args["auto_enable"].as_bool().unwrap_or(true);

        if requested_id.is_none() && requested_name.is_none() {
            return Err(anyhow!(
                "Missing extension identifier: provide extension_id or extension_name"
            ));
        }

        let (team_id, agent_id) = {
            let state = self.state.read().await;
            (state.agent.team_id.clone(), state.agent.id.clone())
        };

        let ext_service = ExtensionService::new(db);

        let (extension_id, extension_name) = if let Some(id) = requested_id {
            let ext = ext_service
                .get(&id)
                .await
                .map_err(|e| anyhow!("Failed to load extension '{}': {}", id, e))?
                .ok_or_else(|| anyhow!("Extension '{}' not found", id))?;
            if ext.team_id.to_hex() != team_id {
                return Err(anyhow!(
                    "Extension '{}' does not belong to current team",
                    id
                ));
            }
            (id, ext.name)
        } else {
            let query_name = requested_name.clone().unwrap_or_default();
            let normalized_query = normalize_ext_name(&query_name);
            let mut exts = ext_service
                .list_active_for_team(&team_id)
                .await
                .map_err(|e| anyhow!("Failed to list team extensions: {}", e))?;
            let found = exts
                .iter()
                .find(|e| normalize_ext_name(&e.name) == normalized_query)
                .cloned()
                .or_else(|| {
                    exts.iter()
                        .find(|e| normalize_ext_name(&e.name).contains(&normalized_query))
                        .cloned()
                })
                .ok_or_else(|| anyhow!("No team extension matched '{}'", query_name))?;
            (
                found.id.map(|oid| oid.to_hex()).unwrap_or_default(),
                found.name.clone(),
            )
        };

        let mut already_installed = false;
        let updated_agent = match agent_service
            .add_team_extension_to_agent(&agent_id, &extension_id, &team_id)
            .await
        {
            Ok(agent) => agent,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("already exists") {
                    already_installed = true;
                    agent_service.get_agent(&agent_id).await.map_err(|db_err| {
                        anyhow!("Failed to reload agent after duplicate install: {}", db_err)
                    })?
                } else {
                    return Err(anyhow!(
                        "Failed to install team extension '{}': {}",
                        extension_name,
                        msg
                    ));
                }
            }
        };

        if let Some(agent) = updated_agent {
            let mut state = self.state.write().await;
            state.agent = agent;
        }

        let mut results = Vec::new();
        if already_installed {
            results.push(ToolContentBlock::Text(format!(
                "Extension '{}' is already installed on this agent.",
                extension_name
            )));
        } else {
            results.push(ToolContentBlock::Text(format!(
                "Installed team extension '{}' (id={}) to this agent.",
                extension_name, extension_id
            )));
        }

        if auto_enable {
            let mut enable_output = self.enable_extension(&extension_name).await?;
            results.append(&mut enable_output);
            self.sync_session_extensions().await;
        }

        Ok(results)
    }

    /// Handle manage_extensions: enable or disable an extension.
    /// Acquires a WRITE lock on the shared state.
    async fn handle_manage(&self, args: Value) -> Result<Vec<ToolContentBlock>> {
        let action = args["action"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing 'action' parameter"))?;
        let ext_name = args["extension_name"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing 'extension_name' parameter"))?;

        let result = match action {
            "enable" => self.enable_extension(ext_name).await,
            "disable" => self.disable_extension(ext_name).await,
            _ => Err(anyhow!(
                "Invalid action '{}'. Use 'enable' or 'disable'.",
                action
            )),
        };

        // Sync extension state to session after successful manage operation
        if result.is_ok() {
            self.sync_session_extensions().await;
        }

        result
    }

    /// Sync current extension state to session document for persistence across messages.
    ///
    /// Note: The DB write happens after releasing the read lock. This is acceptable because
    /// agent tool calls are sequential within a session, so concurrent modifications to the
    /// same session's extension state cannot occur during normal operation.
    async fn sync_session_extensions(&self) {
        let (session_id, agent_service) = match (&self.session_id, &self.agent_service) {
            (Some(sid), Some(svc)) => (sid.clone(), svc.clone()),
            _ => return, // No session persistence configured
        };

        // Compute disabled/enabled within a single read lock scope to get a consistent snapshot.
        // We must release the lock before the async DB call to avoid holding it across await.
        let (disabled, enabled) = {
            let state = self.state.read().await;
            let active = state.active_extension_names();
            let overrides = super::runtime::compute_extension_overrides(&state.agent, &active);
            (overrides.disabled, overrides.enabled)
        };

        info!(
            "Syncing extension overrides to session {}: disabled={:?}, enabled={:?}",
            session_id, disabled, enabled
        );

        if let Err(e) = agent_service
            .update_session_extensions(&session_id, &disabled, &enabled)
            .await
        {
            tracing::warn!("Failed to sync extension overrides to session: {}", e);
        }
    }

    /// Enable an extension by name.
    async fn enable_extension(&self, ext_name: &str) -> Result<Vec<ToolContentBlock>> {
        let mut state = self.state.write().await;

        // Block enabling local "skills" when "team_skills" (MongoDB-backed) is already active.
        // The local skills extension reads from filesystem directories which don't exist on the server.
        if normalize_ext_name(ext_name) == "skills" && state.platform.has_extension("team_skills") {
            return Ok(vec![ToolContentBlock::Text(
                "The 'team_skills' extension is already active, which provides MongoDB-backed skill \
                 management (search and load tools). The local 'skills' extension is not available \
                 in team server context. Use team_skills to search and load skills."
                    .to_string(),
            )]);
        }

        // Determine if it's a platform extension or MCP extension
        let ext_type = classify_extension(&state.agent, ext_name);

        match ext_type {
            ExtensionType::Platform(key) => {
                if state.platform.has_extension(&key) {
                    return Ok(vec![ToolContentBlock::Text(format!(
                        "Extension '{}' is already active.",
                        key
                    ))]);
                }
                match state.platform.add_extension(&key).await {
                    Ok(tools) => {
                        info!("Enabled platform extension '{}': {:?}", key, tools);
                        Ok(vec![ToolContentBlock::Text(format!(
                            "Enabled extension '{}'. New tools available: {}",
                            key,
                            tools.join(", ")
                        ))])
                    }
                    Err(e) => Ok(vec![ToolContentBlock::Text(format!(
                        "Failed to enable extension '{}': {}",
                        key, e
                    ))]),
                }
            }
            ExtensionType::Mcp(config) => {
                let already = state
                    .mcp
                    .as_ref()
                    .map_or(false, |m| m.has_extension(&config.name));
                if already {
                    return Ok(vec![ToolContentBlock::Text(format!(
                        "Extension '{}' is already active.",
                        config.name
                    ))]);
                }
                let ext_display = config.name.clone();
                let api_caller = state.api_caller.clone();
                let mcp = state.mcp.get_or_insert_with(|| McpConnector::empty());
                match mcp.add_extension(&config, api_caller).await {
                    Ok(tools) => {
                        info!("Enabled MCP extension '{}': {:?}", ext_display, tools);
                        Ok(vec![ToolContentBlock::Text(format!(
                            "Enabled extension '{}'. New tools available: {}",
                            ext_display,
                            tools.join(", ")
                        ))])
                    }
                    Err(e) => Ok(vec![ToolContentBlock::Text(format!(
                        "Failed to enable extension '{}': {}",
                        ext_display, e
                    ))]),
                }
            }
            ExtensionType::BinaryNotFound(name) => Ok(vec![ToolContentBlock::Text(format!(
                "Extension '{}' is configured but cannot be started: the 'agime' binary was not found. \
                 Please ensure 'agime' is built and available next to the team server binary or in PATH.",
                name
            ))]),
            ExtensionType::Unknown => Ok(vec![ToolContentBlock::Text(format!(
                "Extension '{}' is not configured for this agent.",
                ext_name
            ))]),
        }
    }

    /// Disable an extension by name (supports fuzzy name matching).
    async fn disable_extension(&self, ext_name: &str) -> Result<Vec<ToolContentBlock>> {
        let mut state = self.state.write().await;

        // Resolve canonical name via normalized matching across platform + MCP
        let canonical = {
            let platform_names = state.platform.extension_names();
            let mcp_names = state
                .mcp
                .as_ref()
                .map(|m| m.extension_names())
                .unwrap_or_default();
            let all_active: Vec<String> = platform_names
                .into_iter()
                .chain(mcp_names.into_iter())
                .collect();
            find_matching_ext_name(&all_active, ext_name).map(|s| s.to_string())
        };

        let resolved = match canonical {
            Some(name) => name,
            None => {
                return Ok(vec![ToolContentBlock::Text(format!(
                    "Extension '{}' is not currently active.",
                    ext_name
                ))]);
            }
        };

        // Try platform first
        if state.platform.has_extension(&resolved) {
            match state.platform.remove_extension(&resolved) {
                Ok(tools) => {
                    info!("Disabled platform extension '{}': {:?}", resolved, tools);
                    return Ok(vec![ToolContentBlock::Text(format!(
                        "Disabled extension '{}'. Removed tools: {}",
                        resolved,
                        tools.join(", ")
                    ))]);
                }
                Err(e) => {
                    return Ok(vec![ToolContentBlock::Text(format!(
                        "Failed to disable extension '{}': {}",
                        resolved, e
                    ))]);
                }
            }
        }

        // Try MCP
        if let Some(ref mut mcp) = state.mcp {
            if mcp.has_extension(&resolved) {
                match mcp.remove_extension(&resolved).await {
                    Ok(tools) => {
                        info!("Disabled MCP extension '{}': {:?}", resolved, tools);
                        return Ok(vec![ToolContentBlock::Text(format!(
                            "Disabled extension '{}'. Removed tools: {}",
                            resolved,
                            tools.join(", ")
                        ))]);
                    }
                    Err(e) => {
                        return Ok(vec![ToolContentBlock::Text(format!(
                            "Failed to disable extension '{}': {}",
                            resolved, e
                        ))]);
                    }
                }
            }
        }

        Ok(vec![ToolContentBlock::Text(format!(
            "Extension '{}' is not currently active.",
            ext_name
        ))])
    }

    /// Handle list_resources: list resources from active extensions.
    async fn handle_list_resources(&self, args: Value) -> Result<Vec<ToolContentBlock>> {
        let filter_name = args["extension_name"].as_str();
        let state = self.state.read().await;

        let mut output = String::from("## Extension Resources\n\n");
        output.push_str("Note: Resource listing is limited in team server context. ");
        output.push_str("Active extensions with potential resources:\n\n");

        // List platform extensions
        for name in state.platform.extension_names() {
            if let Some(filter) = filter_name {
                if name != filter {
                    continue;
                }
            }
            output.push_str(&format!("- {} (platform)\n", name));
        }

        // List MCP extensions
        if let Some(ref mcp) = state.mcp {
            for name in mcp.extension_names() {
                if let Some(filter) = filter_name {
                    if name != filter {
                        continue;
                    }
                }
                output.push_str(&format!("- {} (mcp)\n", name));
            }
        }

        Ok(vec![ToolContentBlock::Text(output)])
    }

    /// Handle read_resource: read a specific resource by URI.
    async fn handle_read_resource(&self, args: Value) -> Result<Vec<ToolContentBlock>> {
        let uri = args["uri"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing 'uri' parameter"))?;

        Ok(vec![ToolContentBlock::Text(format!(
            "Resource reading is not yet supported in team server context. URI: {}",
            uri
        ))])
    }
}

// ========================================
// Helper types and functions
// ========================================

/// Classification of an extension type for enable/disable routing.
enum ExtensionType {
    /// Platform extension (in-process), with the registry key
    Platform(String),
    /// MCP extension (subprocess), with the config
    Mcp(CustomExtensionConfig),
    /// Extension is configured but the agime binary needed to run it was not found
    BinaryNotFound(String),
    /// Unknown extension
    Unknown,
}

/// Normalize an extension name for fuzzy matching.
/// Strips underscores, hyphens, spaces and lowercases to handle LLM name variations
/// like "auto_visualiser", "AutoVisualiser", "auto-visualiser" → "autovisualiser".
fn normalize_ext_name(name: &str) -> String {
    name.to_lowercase().replace(['_', '-', ' '], "")
}

/// Find the actual extension name that matches the given (possibly non-normalized) name.
/// Returns the canonical name if found, or None.
fn find_matching_ext_name<'a>(candidates: &'a [String], query: &str) -> Option<&'a str> {
    let normalized_query = normalize_ext_name(query);
    // Exact match first
    if let Some(name) = candidates.iter().find(|n| **n == query) {
        return Some(name.as_str());
    }
    // Normalized match
    candidates
        .iter()
        .find(|n| normalize_ext_name(n) == normalized_query)
        .map(|s| s.as_str())
}

/// Classify an extension name to determine how to enable it.
fn classify_extension(agent: &TeamAgent, ext_name: &str) -> ExtensionType {
    let normalized = normalize_ext_name(ext_name);

    // Check builtin platform extensions
    for ext_config in &agent.enabled_extensions {
        let name = ext_config.extension.name();
        if name == ext_name || normalize_ext_name(name) == normalized {
            if ext_config.extension.is_platform() {
                // Use original snake_case name as the platform registry key,
                // matching PlatformExtensionRunner keys: "skills", "todo", "team", "document_tools"
                return ExtensionType::Platform(name.to_string());
            } else {
                // Builtin MCP extension — build config from agime binary
                if let Some(config) = builtin_to_custom_config(&ext_config.extension) {
                    return ExtensionType::Mcp(config);
                }
                // Binary not found — extension is configured but cannot be started
                return ExtensionType::BinaryNotFound(name.to_string());
            }
        }
    }

    // Check custom extensions
    for custom in &agent.custom_extensions {
        if custom.name == ext_name || normalize_ext_name(&custom.name) == normalized {
            return ExtensionType::Mcp(custom.clone());
        }
    }

    ExtensionType::Unknown
}

/// Collect all extension names configured for an agent (both builtin and custom).
/// Returns RUNTIME names: mcp_name() for subprocess extensions, name() for platform extensions.
/// This ensures the names match what PlatformExtensionRunner and McpConnector actually register.
fn collect_all_extension_names(agent: &TeamAgent) -> Vec<String> {
    let mut names = Vec::new();

    for ext_config in &agent.enabled_extensions {
        if ext_config.enabled {
            // MCP subprocess extensions use mcp_name() at runtime
            if let Some(mcp) = ext_config.extension.mcp_name() {
                names.push(mcp.to_string());
            } else {
                names.push(ext_config.extension.name().to_string());
            }
        }
    }

    for custom in &agent.custom_extensions {
        if custom.enabled {
            names.push(custom.name.clone());
        }
    }

    names
}

/// Convert a builtin extension to a CustomExtensionConfig for MCP subprocess startup.
pub(super) fn builtin_to_custom_config(ext: &BuiltinExtension) -> Option<CustomExtensionConfig> {
    let mcp_name = ext.mcp_name()?;

    let bin = super::executor_mongo::find_agime_binary()?;
    Some(CustomExtensionConfig {
        name: mcp_name.to_string(),
        ext_type: "stdio".to_string(),
        uri_or_cmd: bin,
        args: vec!["mcp".to_string(), mcp_name.to_string()],
        envs: HashMap::new(),
        enabled: true,
        source: None,
        source_extension_id: None,
    })
}
