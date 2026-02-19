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
use std::collections::HashSet;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use super::developer_tools::DeveloperToolsProvider;
use super::document_tools::DocumentToolsProvider;
use super::mcp_connector::{McpConnector, ToolContentBlock};
use super::portal_tools::PortalToolsProvider;
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
}

/// Runs platform extensions in-process, providing tool listing and call dispatch.
pub struct PlatformExtensionRunner {
    extensions: Vec<PlatformExtensionEntry>,
}

impl PlatformExtensionRunner {
    /// Create a new runner by instantiating enabled platform extensions.
    ///
    /// Supported extensions: Skills, Team, Todo, DocumentTools, PortalTools.
    /// ExtensionManager and ChatRecall are skipped (not applicable in team server context).
    pub async fn create(
        enabled_extensions: &[AgentExtensionConfig],
        db: Option<Arc<MongoDb>>,
        team_id: Option<&str>,
        session_id: Option<&str>,
        mission_id: Option<&str>,
        agent_id: Option<&str>,
        enable_team_skills_on_demand: bool,
        workspace_path: Option<&str>,
        workspace_root: Option<&str>,
        portal_base_url: Option<&str>,
        allowed_extension_names: Option<&HashSet<String>>,
        allowed_skill_ids: Option<&HashSet<String>>,
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

            // In on-demand mode, replace local "skills" extension with team-backed tools.
            if ext_config.extension == BuiltinExtension::Skills && enable_team_skills_on_demand {
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
                    "team_skills on-demand requested but db/team context missing; falling back to local skills extension"
                );
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
                    )
                    .await
                    {
                        extensions.push(entry);
                    }
                }
                continue;
            }

            // ExtensionManager and ChatRecall are not loaded in team server runtime.
            if matches!(
                ext_config.extension,
                BuiltinExtension::ExtensionManager | BuiltinExtension::ChatRecall
            ) {
                continue;
            }

            // Map BuiltinExtension enum to PLATFORM_EXTENSIONS key.
            let platform_key = match ext_config.extension {
                BuiltinExtension::Skills => Some("skills"),
                BuiltinExtension::Todo => Some("todo"),
                BuiltinExtension::Team => Some("team"),
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
        if allowed_extension_names
            .map(|set| set.contains("portal_tools"))
            .unwrap_or(false)
            && !extensions.iter().any(|e| e.name == "portal_tools")
        {
            if let Some(entry) =
                Self::try_init_portal_tools(&db, team_id, portal_base_url, workspace_root).await
            {
                tracing::info!(
                    "Platform extension 'portal_tools' loaded as fallback: {} tools",
                    entry.tools.len()
                );
                extensions.push(entry);
            }
        }

        Self { extensions }
    }

    /// Try to initialize DocumentTools if db+team context is available.
    /// Returns `None` if context is missing or initialization fails.
    async fn try_init_document_tools(
        db: &Option<Arc<MongoDb>>,
        team_id: Option<&str>,
        session_id: Option<&str>,
        mission_id: Option<&str>,
        agent_id: Option<&str>,
        workspace_path: Option<&str>,
    ) -> Option<PlatformExtensionEntry> {
        let (db, tid) = match (db, team_id) {
            (Some(db), Some(tid)) => (db, tid),
            _ => return None,
        };
        let provider = DocumentToolsProvider::new(
            db.clone(),
            tid.to_string(),
            session_id.map(String::from),
            mission_id.map(String::from),
            agent_id.map(String::from),
            workspace_path.map(String::from),
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
    async fn try_init_developer(
        workspace_path: Option<&str>,
    ) -> Option<PlatformExtensionEntry> {
        match DeveloperToolsProvider::new(workspace_path).await {
            Ok(provider) => {
                match Self::init_from_client("developer", Box::new(provider)).await {
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
                }
            }
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

    /// Initialize a single platform extension by its key in PLATFORM_EXTENSIONS.
    async fn init_one(key: &str) -> Result<PlatformExtensionEntry> {
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
        let (ext, tool) = self
            .find_tool(tool_name)
            .ok_or_else(|| anyhow!("Platform tool not found: {}", tool_name))?;

        // Build arguments as JsonObject
        let arguments = match input {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            other => Some(serde_json::Map::from_iter([("input".to_string(), other)])),
        };

        let cancel = CancellationToken::new();
        let call_result = ext
            .client
            .call_tool(&tool.original_name, arguments, cancel)
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
