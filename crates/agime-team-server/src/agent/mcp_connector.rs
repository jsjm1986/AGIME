//! MCP Connector for agent tool calling
//!
//! Connects to MCP servers (stdio/SSE/StreamableHttp), collects tool definitions,
//! and executes tool calls. Independent of the agime crate.

use agime_team::models::CustomExtensionConfig;
use anyhow::{anyhow, Result};
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, CancelTaskParams, CancelTaskRequest,
    CancelledNotification, CancelledNotificationMethod, CancelledNotificationParam,
    ClientCapabilities, ClientInfo, ClientRequest, CreateElicitationRequestParams,
    CreateElicitationResult, CreateMessageRequestParams, CreateMessageResult, CreateTaskResult,
    ElicitationAction, GetTaskInfoParams, GetTaskInfoRequest, GetTaskResultParams,
    GetTaskResultRequest, Implementation, ListToolsRequest, PaginatedRequestParams,
    ProtocolVersion, RequestId, Role, SamplingMessage, SamplingMessageContent, ServerResult,
    TaskResult, TaskStatus, TaskSupport, Tool, ToolChoice, ToolChoiceMode, ToolExecution,
};
use rmcp::service::{PeerRequestOptions, RequestContext, RunningService};
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{ClientHandler, ErrorData, Peer, RoleClient, ServiceExt};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Tool definition collected from MCP servers
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    /// Prefixed name: "server_name__tool_name"
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    /// Server name for routing tool calls
    pub server_name: String,
    /// Original tool name within the MCP server
    pub original_name: String,
    /// Execution-related metadata from MCP tool definition
    pub execution: Option<ToolExecution>,
}

/// A content block returned from a tool call (supports multimodal)
#[derive(Debug, Clone)]
pub enum ToolContentBlock {
    Text(String),
    Image { mime_type: String, data: String }, // base64 data
}

#[derive(Debug, Clone)]
pub struct ToolTaskProgress {
    pub tool_name: String,
    pub server_name: String,
    pub task_id: String,
    pub status: String,
    pub status_message: Option<String>,
    pub poll_count: u32,
}

pub type ToolTaskProgressCallback = Arc<dyn Fn(ToolTaskProgress) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct ElicitationBridgeEvent {
    pub request_type: &'static str,
    pub message: String,
    pub elicitation_id: Option<String>,
    pub url: Option<String>,
}

pub type ElicitationBridgeCallback = Arc<dyn Fn(ElicitationBridgeEvent) + Send + Sync>;

/// A single MCP server connection
struct McpConnection {
    name: String,
    client: Mutex<RunningService<RoleClient, AgentClientHandler>>,
    tools: Vec<ToolDefinition>,
    tool_cache_ttl: Duration,
    tool_cache_expires_at: Instant,
    tool_list_changed_supported: bool,
    tool_list_changed_dirty: Arc<AtomicBool>,
}

/// Trait for making LLM API calls (used by MCP Sampling)
pub trait ApiCaller: Send + Sync {
    fn call_llm<'a>(
        &'a self,
        system: &'a str,
        messages: Vec<serde_json::Value>,
        max_tokens: u32,
        tools: Option<Vec<Tool>>,
        tool_choice: Option<ToolChoice>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value>> + Send + 'a>>;
}

/// Client handler that supports MCP Sampling via an ApiCaller
struct AgentClientHandler {
    api_caller: Option<Arc<dyn ApiCaller>>,
    tool_list_changed_dirty: Arc<AtomicBool>,
    elicitation_bridge: Option<ElicitationBridgeCallback>,
}

impl AgentClientHandler {
    fn elicitation_enabled() -> bool {
        std::env::var("TEAM_MCP_ENABLE_ELICITATION")
            .ok()
            .map(|v| {
                let normalized = v.trim().to_ascii_lowercase();
                normalized == "1" || normalized == "true" || normalized == "yes"
            })
            .unwrap_or(false)
    }

    fn elicitation_default_action() -> ElicitationAction {
        match std::env::var("TEAM_MCP_ELICITATION_DEFAULT_ACTION")
            .ok()
            .map(|v| v.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("accept") => ElicitationAction::Accept,
            Some("decline") => ElicitationAction::Decline,
            _ => ElicitationAction::Cancel,
        }
    }

    fn elicitation_default_content() -> Option<serde_json::Value> {
        let raw = std::env::var("TEAM_MCP_ELICITATION_DEFAULT_CONTENT_JSON").ok()?;
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    target: "agime::metrics",
                    event = "mcp_elicitation_default_content_invalid_json",
                    error = %e,
                    "TEAM_MCP_ELICITATION_DEFAULT_CONTENT_JSON is not valid JSON"
                );
                return None;
            }
        };
        match parsed {
            serde_json::Value::Object(_) => Some(parsed),
            _ => {
                warn!(
                    target: "agime::metrics",
                    event = "mcp_elicitation_default_content_not_object",
                    "TEAM_MCP_ELICITATION_DEFAULT_CONTENT_JSON must be a JSON object"
                );
                None
            }
        }
    }

    fn resolve_elicitation_policy(
        request: &CreateElicitationRequestParams,
    ) -> (ElicitationAction, Option<serde_json::Value>) {
        let mut action = Self::elicitation_default_action();
        let mut content = None;

        if matches!(action, ElicitationAction::Accept) {
            content = Self::elicitation_default_content();
            if matches!(
                request,
                CreateElicitationRequestParams::FormElicitationParams { .. }
            ) && content.is_none()
            {
                // Form elicitation accepts structured input; if no content is configured,
                // fall back to cancel to avoid returning an invalid empty accept payload.
                warn!(
                    target: "agime::metrics",
                    event = "mcp_elicitation_form_accept_without_content",
                    "Form elicitation default action 'accept' requires TEAM_MCP_ELICITATION_DEFAULT_CONTENT_JSON; falling back to cancel"
                );
                action = ElicitationAction::Cancel;
            }
        }

        (action, content)
    }

    fn resolve_sampling_tools(
        tools: Option<Vec<Tool>>,
        tool_choice: Option<ToolChoice>,
    ) -> std::result::Result<(Vec<Tool>, Option<ToolChoice>), ErrorData> {
        let mut resolved_tools = tools.unwrap_or_default();
        let resolved_choice = tool_choice;
        match resolved_choice.as_ref().and_then(|c| c.mode.clone()) {
            Some(ToolChoiceMode::None) => {
                resolved_tools.clear();
            }
            Some(ToolChoiceMode::Required) if resolved_tools.is_empty() => {
                return Err(ErrorData::new(
                    rmcp::model::ErrorCode::INVALID_PARAMS,
                    "tool_choice=required but no tools were provided",
                    None,
                ));
            }
            _ => {}
        }

        Ok((resolved_tools, resolved_choice))
    }

    fn validate_sampling_tool_choice_result(
        tool_choice: Option<&ToolChoice>,
        has_tool_use: bool,
    ) -> std::result::Result<(), ErrorData> {
        match tool_choice.and_then(|choice| choice.mode.as_ref()) {
            Some(ToolChoiceMode::Required) if !has_tool_use => Err(ErrorData::new(
                rmcp::model::ErrorCode::INVALID_PARAMS,
                "tool_choice=required but model returned no tool_use blocks",
                None,
            )),
            Some(ToolChoiceMode::None) if has_tool_use => Err(ErrorData::new(
                rmcp::model::ErrorCode::INVALID_PARAMS,
                "tool_choice=none but model returned tool_use blocks",
                None,
            )),
            _ => Ok(()),
        }
    }

    fn tool_choice_mode_label(choice: Option<&ToolChoice>) -> &'static str {
        match choice.and_then(|c| c.mode.as_ref()) {
            Some(ToolChoiceMode::Required) => "required",
            Some(ToolChoiceMode::None) => "none",
            Some(ToolChoiceMode::Auto) => "auto",
            None => "unset",
        }
    }

    fn sampling_content_to_api_block(item: &SamplingMessageContent) -> Option<serde_json::Value> {
        if let Some(text) = item.as_text() {
            return Some(serde_json::json!({
                "type": "text",
                "text": text.text.clone(),
            }));
        }
        if let Some(tool_use) = item.as_tool_use() {
            let mut block = serde_json::to_value(tool_use).ok()?;
            if block.get("type").is_none() {
                block["type"] = serde_json::json!("tool_use");
            }
            return Some(block);
        }
        if let Some(tool_result) = item.as_tool_result() {
            let mut block = serde_json::to_value(tool_result).ok()?;
            if block.get("type").is_none() {
                block["type"] = serde_json::json!("tool_result");
            }
            return Some(block);
        }
        None
    }

    fn sampling_messages_to_api_messages(messages: &[SamplingMessage]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|msg| {
                let role = match msg.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };

                let mut blocks: Vec<serde_json::Value> = msg
                    .content
                    .iter()
                    .filter_map(Self::sampling_content_to_api_block)
                    .collect();
                if blocks.is_empty() {
                    blocks.push(serde_json::json!({
                        "type": "text",
                        "text": "",
                    }));
                }

                serde_json::json!({
                    "role": role,
                    "content": blocks,
                })
            })
            .collect()
    }
}

impl ClientHandler for AgentClientHandler {
    fn get_info(&self) -> ClientInfo {
        let capabilities = if Self::elicitation_enabled() {
            ClientCapabilities::builder()
                .enable_sampling()
                .enable_sampling_tools()
                .enable_elicitation()
                .build()
        } else {
            ClientCapabilities::builder()
                .enable_sampling()
                .enable_sampling_tools()
                .build()
        };

        ClientInfo {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities,
            client_info: Implementation {
                name: "agime-team-server".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                description: None,
                icons: None,
                title: None,
                website_url: None,
            },
            meta: None,
        }
    }

    async fn create_message(
        &self,
        params: CreateMessageRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> std::result::Result<CreateMessageResult, ErrorData> {
        let api_caller = self.api_caller.as_ref().ok_or_else(|| {
            ErrorData::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                "MCP Sampling not supported: no API caller configured",
                None,
            )
        })?;

        // Convert sampling messages into structured blocks so tool_use/tool_result
        // are preserved and not collapsed into plain text.
        let messages = Self::sampling_messages_to_api_messages(&params.messages);

        let system = params.system_prompt.as_deref().unwrap_or("");
        let max_tokens = params.max_tokens;
        let (sampling_tools, sampling_tool_choice) =
            Self::resolve_sampling_tools(params.tools.clone(), params.tool_choice.clone())?;
        let tools_for_call = if sampling_tools.is_empty() {
            None
        } else {
            Some(sampling_tools)
        };
        let sampling_tools_count = tools_for_call.as_ref().map(|t| t.len()).unwrap_or(0);
        let sampling_tool_choice_mode =
            Self::tool_choice_mode_label(sampling_tool_choice.as_ref()).to_string();
        let started = Instant::now();
        tracing::debug!(
            target: "agime::metrics",
            event = "mcp_sampling_call_start",
            sampling_tools_count,
            sampling_tool_choice_mode = sampling_tool_choice_mode.as_str()
        );

        // Call the LLM via the ApiCaller
        let response = api_caller
            .call_llm(
                system,
                messages,
                max_tokens,
                tools_for_call,
                sampling_tool_choice.clone(),
            )
            .await
            .map_err(|e| {
                ErrorData::new(
                    rmcp::model::ErrorCode::INTERNAL_ERROR,
                    format!("LLM call failed: {}", e),
                    None,
                )
            })?;
        tracing::debug!(
            target: "agime::metrics",
            event = "mcp_sampling_call_done",
            sampling_tools_count,
            sampling_tool_choice_mode = sampling_tool_choice_mode.as_str(),
            latency_ms = started.elapsed().as_millis() as u64
        );

        // Convert response into MCP sampling content (text + tool_use where available).
        let mut content_items: Vec<SamplingMessageContent> = Vec::new();
        if let Some(content) = response["content"].as_array() {
            // Anthropic format
            for block in content {
                match block["type"].as_str() {
                    Some("text") => {
                        if let Some(text) = block["text"].as_str() {
                            content_items.push(SamplingMessageContent::text(text.to_string()));
                        }
                    }
                    Some("tool_use") => {
                        let id = block["id"].as_str().unwrap_or_default().to_string();
                        let name = block["name"].as_str().unwrap_or_default().to_string();
                        let input = block["input"].as_object().cloned().unwrap_or_default();
                        if !id.is_empty() && !name.is_empty() {
                            content_items.push(SamplingMessageContent::tool_use(id, name, input));
                        }
                    }
                    _ => {}
                }
            }
        } else {
            // OpenAI Chat Completions format
            if let Some(text) = response["choices"][0]["message"]["content"].as_str() {
                if !text.is_empty() {
                    content_items.push(SamplingMessageContent::text(text.to_string()));
                }
            }
            if let Some(tool_calls) = response["choices"][0]["message"]["tool_calls"].as_array() {
                for tool_call in tool_calls {
                    let id = tool_call["id"].as_str().unwrap_or_default().to_string();
                    let name = tool_call["function"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    let input = tool_call["function"]["arguments"]
                        .as_str()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                        .and_then(|v| v.as_object().cloned())
                        .unwrap_or_default();
                    if !id.is_empty() && !name.is_empty() {
                        content_items.push(SamplingMessageContent::tool_use(id, name, input));
                    }
                }
            }
        }

        if content_items.is_empty() {
            content_items.push(SamplingMessageContent::text(String::new()));
        }
        let has_tool_use = content_items.iter().any(|c| c.as_tool_use().is_some());
        Self::validate_sampling_tool_choice_result(sampling_tool_choice.as_ref(), has_tool_use)?;

        let model = response["model"].as_str().unwrap_or("unknown").to_string();

        Ok(CreateMessageResult {
            model,
            stop_reason: Some(if has_tool_use {
                CreateMessageResult::STOP_REASON_TOOL_USE.to_string()
            } else {
                CreateMessageResult::STOP_REASON_END_TURN.to_string()
            }),
            message: SamplingMessage {
                role: Role::Assistant,
                content: content_items.into(),
                meta: None,
            },
        })
    }

    async fn on_tool_list_changed(&self, _context: rmcp::service::NotificationContext<RoleClient>) {
        self.tool_list_changed_dirty.store(true, Ordering::Release);
        tracing::debug!(
            target: "agime::metrics",
            event = "mcp_tool_list_changed_notification",
            "MCP server reported tools/list_changed; tool cache marked dirty"
        );
    }

    async fn create_elicitation(
        &self,
        request: CreateElicitationRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> std::result::Result<CreateElicitationResult, rmcp::ErrorData> {
        let (action, content) = Self::resolve_elicitation_policy(&request);
        match request {
            CreateElicitationRequestParams::FormElicitationParams { message, .. } => {
                if let Some(cb) = &self.elicitation_bridge {
                    cb(ElicitationBridgeEvent {
                        request_type: "form",
                        message: message.to_string(),
                        elicitation_id: None,
                        url: None,
                    });
                }
                warn!(
                    target: "agime::metrics",
                    event = "mcp_elicitation_fallback",
                    request_type = "form",
                    action = ?action,
                    message = message.as_str(),
                    "MCP elicitation handled by configured policy fallback"
                );
            }
            CreateElicitationRequestParams::UrlElicitationParams {
                message,
                url,
                elicitation_id,
                ..
            } => {
                if let Some(cb) = &self.elicitation_bridge {
                    cb(ElicitationBridgeEvent {
                        request_type: "url",
                        message: message.to_string(),
                        elicitation_id: Some(elicitation_id.to_string()),
                        url: Some(url.to_string()),
                    });
                }
                warn!(
                    target: "agime::metrics",
                    event = "mcp_elicitation_fallback",
                    request_type = "url",
                    action = ?action,
                    message = message.as_str(),
                    url = url.as_str(),
                    elicitation_id = elicitation_id.as_str(),
                    "MCP URL elicitation handled by configured policy fallback"
                );
            }
        }

        Ok(CreateElicitationResult { action, content })
    }
}

/// MCP Connector that manages connections to multiple MCP servers
pub struct McpConnector {
    connections: Vec<McpConnection>,
    elicitation_bridge: Option<ElicitationBridgeCallback>,
}

impl McpConnector {
    /// Create an empty McpConnector with no connections.
    pub fn empty() -> Self {
        Self {
            connections: Vec::new(),
            elicitation_bridge: None,
        }
    }

    /// Connect to MCP servers defined in the agent's custom_extensions.
    /// Only connects to enabled extensions with supported types (sse, stdio).
    /// If `api_caller` is provided, MCP Sampling will be supported.
    /// If `workspace_path` is provided, stdio child processes will use it as CWD.
    pub async fn connect(
        extensions: &[CustomExtensionConfig],
        api_caller: Option<Arc<dyn ApiCaller>>,
        elicitation_bridge: Option<ElicitationBridgeCallback>,
        workspace_path: Option<&str>,
    ) -> Result<Self> {
        let mut connections = Vec::new();

        for ext in extensions {
            if !ext.enabled {
                info!("Skipping disabled extension: {}", ext.name);
                continue;
            }

            match Self::connect_one(
                ext,
                api_caller.clone(),
                elicitation_bridge.clone(),
                workspace_path,
            )
            .await
            {
                Ok(conn) => {
                    info!(
                        "Connected to MCP server '{}': {} tools available",
                        conn.name,
                        conn.tools.len()
                    );
                    connections.push(conn);
                }
                Err(e) => {
                    warn!("Failed to connect to MCP server '{}': {}", ext.name, e);
                    // Continue with other extensions rather than failing entirely
                }
            }
        }

        Ok(Self {
            connections,
            elicitation_bridge,
        })
    }

    /// Connect to a single MCP server (with 30s timeout)
    async fn connect_one(
        ext: &CustomExtensionConfig,
        api_caller: Option<Arc<dyn ApiCaller>>,
        elicitation_bridge: Option<ElicitationBridgeCallback>,
        workspace_path: Option<&str>,
    ) -> Result<McpConnection> {
        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            Self::connect_one_inner(ext, api_caller, elicitation_bridge, workspace_path),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(anyhow!(
                "MCP server '{}' connection timed out after 30s",
                ext.name
            )),
        }
    }

    /// Inner connection logic (called by connect_one with timeout)
    async fn connect_one_inner(
        ext: &CustomExtensionConfig,
        api_caller: Option<Arc<dyn ApiCaller>>,
        elicitation_bridge: Option<ElicitationBridgeCallback>,
        workspace_path: Option<&str>,
    ) -> Result<McpConnection> {
        let tool_list_changed_dirty = Arc::new(AtomicBool::new(false));
        let handler = AgentClientHandler {
            api_caller,
            tool_list_changed_dirty: tool_list_changed_dirty.clone(),
            elicitation_bridge,
        };
        let ext_type = ext.ext_type.to_lowercase();

        let running = match ext_type.as_str() {
            "sse" | "streamablehttp" | "streamable_http" => {
                Self::connect_http(handler, &ext.uri_or_cmd).await?
            }
            "stdio" => Self::connect_stdio(handler, ext, workspace_path).await?,
            other => {
                return Err(anyhow!("Unsupported extension type: {}", other));
            }
        };

        // List tools from the server
        let tools = Self::list_tools_from(&running, &ext.name).await?;
        let tool_list_changed_supported = running
            .peer_info()
            .and_then(|info| info.capabilities.tools.as_ref())
            .and_then(|tools| tools.list_changed)
            .unwrap_or(false);
        let tool_cache_ttl =
            Duration::from_secs(Self::tool_cache_ttl_seconds(tool_list_changed_supported));

        Ok(McpConnection {
            name: ext.name.clone(),
            client: Mutex::new(running),
            tools,
            tool_cache_ttl,
            tool_cache_expires_at: Instant::now() + tool_cache_ttl,
            tool_list_changed_supported,
            tool_list_changed_dirty,
        })
    }

    /// Connect via StreamableHttp transport (SSE-compatible).
    async fn connect_http(
        handler: AgentClientHandler,
        uri: &str,
    ) -> Result<RunningService<RoleClient, AgentClientHandler>> {
        let transport = StreamableHttpClientTransport::from_uri(uri.to_string());
        let running = handler
            .serve(transport)
            .await
            .map_err(|e| anyhow!("Failed to connect to HTTP MCP server: {}", e))?;
        Ok(running)
    }

    /// Connect via stdio (child process)
    async fn connect_stdio(
        handler: AgentClientHandler,
        ext: &CustomExtensionConfig,
        workspace_path: Option<&str>,
    ) -> Result<RunningService<RoleClient, AgentClientHandler>> {
        let mut cmd = tokio::process::Command::new(&ext.uri_or_cmd);
        cmd.args(&ext.args);

        // Set environment variables
        for (key, value) in &ext.envs {
            cmd.env(key, value);
        }

        // Set workspace directory as CWD for process isolation
        if let Some(wp) = workspace_path {
            cmd.current_dir(wp);
        }

        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // On Windows, prevent child process from creating a console window
        #[cfg(target_os = "windows")]
        {
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let transport = TokioChildProcess::new(cmd)?;
        let running = handler.serve(transport).await.map_err(|e| {
            anyhow!(
                "Failed to connect to stdio MCP server '{}': {}",
                ext.uri_or_cmd,
                e
            )
        })?;
        Ok(running)
    }

    /// List tools from a connected MCP server
    async fn list_tools_from(
        running: &RunningService<RoleClient, AgentClientHandler>,
        server_name: &str,
    ) -> Result<Vec<ToolDefinition>> {
        let mut all_tools = Vec::new();
        let mut cursor: Option<String> = None;
        const MAX_PAGES: usize = 50;

        for _page in 0..MAX_PAGES {
            let request = ClientRequest::ListToolsRequest(ListToolsRequest {
                params: Some(PaginatedRequestParams {
                    meta: None,
                    cursor: cursor.clone(),
                }),
                method: Default::default(),
                extensions: Default::default(),
            });

            let result = running
                .send_request(request)
                .await
                .map_err(|e| anyhow!("Failed to list tools: {}", e))?;

            let tools_result = match result {
                ServerResult::ListToolsResult(r) => r,
                _ => return Err(anyhow!("Unexpected response for list_tools")),
            };

            for tool in &tools_result.tools {
                // Sanitize server_name: replace __ with _ to prevent split_once ambiguity
                let safe_server_name = server_name.replace("__", "_");
                let safe_tool_name = tool.name.to_string().replace("__", "_");
                let prefixed_name = format!("{}__{}", safe_server_name, safe_tool_name);
                let description = tool
                    .description
                    .as_ref()
                    .map(|d| d.to_string())
                    .unwrap_or_default();

                // Convert input_schema to serde_json::Value
                let input_schema = serde_json::to_value(&tool.input_schema)
                    .unwrap_or(serde_json::json!({"type": "object"}));

                all_tools.push(ToolDefinition {
                    name: prefixed_name,
                    description,
                    input_schema,
                    server_name: server_name.to_string(),
                    original_name: tool.name.to_string(),
                    execution: tool.execution.clone(),
                });
            }

            // Handle pagination
            match tools_result.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        if cursor.is_some() {
            warn!(
                "Tool listing for '{}' reached max pages ({}), some tools may be missing",
                server_name, MAX_PAGES
            );
        }

        Ok(all_tools)
    }

    /// Get all tool definitions as native rmcp::model::Tool (for Provider trait)
    pub fn tools_as_rmcp(&self) -> Vec<rmcp::model::Tool> {
        self.connections
            .iter()
            .flat_map(|conn| &conn.tools)
            .map(|tool| rmcp::model::Tool {
                name: tool.name.clone().into(),
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

    /// Get all tool definitions formatted for the Anthropic API
    pub fn tools_for_anthropic_api(&self) -> Vec<serde_json::Value> {
        self.connections
            .iter()
            .flat_map(|conn| &conn.tools)
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                })
            })
            .collect()
    }

    /// Get all tool definitions formatted for the OpenAI API (function calling)
    pub fn tools_for_openai_api(&self) -> Vec<serde_json::Value> {
        self.connections
            .iter()
            .flat_map(|conn| &conn.tools)
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                    }
                })
            })
            .collect()
    }

    /// Check if any tools are available
    pub fn has_tools(&self) -> bool {
        self.connections.iter().any(|c| !c.tools.is_empty())
    }

    fn tool_cache_ttl_seconds(tool_list_changed_supported: bool) -> u64 {
        const DEFAULT_TOOL_CACHE_TTL_SECS: u64 = 5;
        const DEFAULT_TOOL_CACHE_TTL_LIST_CHANGED_SECS: u64 = 300;
        let env_name = if tool_list_changed_supported {
            "AGIME_EXTENSION_TOOL_CACHE_TTL_LIST_CHANGED_SECS"
        } else {
            "AGIME_EXTENSION_TOOL_CACHE_TTL_SECS"
        };
        let default = if tool_list_changed_supported {
            DEFAULT_TOOL_CACHE_TTL_LIST_CHANGED_SECS
        } else {
            DEFAULT_TOOL_CACHE_TTL_SECS
        };
        std::env::var(env_name)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(default)
    }

    fn task_timeout_secs() -> u64 {
        std::env::var("MCP_TASK_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .or_else(|| {
                std::env::var("TEAM_MCP_TOOL_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .filter(|v| *v > 0)
            })
            .unwrap_or(600)
    }

    fn connection_tools_refresh_needed(conn: &McpConnection, now: Instant) -> bool {
        let list_changed = conn.tool_list_changed_dirty.load(Ordering::Acquire);
        let expired = now >= conn.tool_cache_expires_at;
        list_changed || expired
    }

    pub async fn refresh_tools_if_stale(&mut self) {
        let now = Instant::now();
        for conn in &mut self.connections {
            if !Self::connection_tools_refresh_needed(conn, now) {
                continue;
            }
            let list_changed = conn.tool_list_changed_dirty.swap(false, Ordering::AcqRel);
            let refresh_reason = if list_changed {
                "list_changed"
            } else {
                "ttl_expired"
            };

            let client = conn.client.lock().await;
            match Self::list_tools_from(&client, &conn.name).await {
                Ok(fresh_tools) => {
                    let refreshed_count = fresh_tools.len();
                    conn.tools = fresh_tools;
                    conn.tool_cache_expires_at = Instant::now() + conn.tool_cache_ttl;
                    tracing::debug!(
                        target: "agime::metrics",
                        event = "mcp_tools_cache_refreshed",
                        extension = conn.name.as_str(),
                        reason = refresh_reason,
                        tool_count = refreshed_count,
                        tool_list_changed_supported = conn.tool_list_changed_supported
                    );
                }
                Err(e) => {
                    if list_changed {
                        conn.tool_list_changed_dirty.store(true, Ordering::Release);
                    }
                    conn.tool_cache_expires_at = Instant::now() + Duration::from_secs(5);
                    warn!(
                        "Failed to refresh tools for extension '{}': {} (keeping stale list)",
                        conn.name, e
                    );
                }
            }
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

    fn task_payload_for_tool(
        tool: Option<&ToolDefinition>,
    ) -> Result<Option<serde_json::Map<String, serde_json::Value>>> {
        Self::task_payload_for_tool_with_gate(tool, Self::task_calls_enabled())
    }

    fn task_payload_for_tool_with_gate(
        tool: Option<&ToolDefinition>,
        task_calls_enabled: bool,
    ) -> Result<Option<serde_json::Map<String, serde_json::Value>>> {
        let Some(task_support) = tool
            .and_then(|t| t.execution.as_ref())
            .and_then(|e| e.task_support)
        else {
            return Ok(None);
        };

        match task_support {
            TaskSupport::Forbidden => Ok(None),
            TaskSupport::Optional => {
                if task_calls_enabled {
                    Ok(Some(serde_json::Map::new()))
                } else {
                    Ok(None)
                }
            }
            TaskSupport::Required => {
                if task_calls_enabled {
                    Ok(Some(serde_json::Map::new()))
                } else {
                    Err(anyhow!(
                        "Tool requires task invocation, but TEAM_MCP_ENABLE_TASK_CALLS is disabled"
                    ))
                }
            }
        }
    }

    /// Get the names of all connected extensions
    pub fn extension_names(&self) -> Vec<String> {
        self.connections.iter().map(|c| c.name.clone()).collect()
    }

    /// Route a tool call to the appropriate MCP server and return the raw result.
    /// Supports cancellation: on timeout or external cancel, sends a cancel notification
    /// to the MCP subprocess (matching local agime behavior).
    async fn send_tool_call(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        progress_cb: Option<ToolTaskProgressCallback>,
        cancel_token: CancellationToken,
    ) -> Result<CallToolResult> {
        // Parse "server_name__tool_name" format
        let (server_name, original_name) = tool_name
            .split_once("__")
            .ok_or_else(|| anyhow!("Invalid tool name format: {}", tool_name))?;

        let conn = self
            .connections
            .iter()
            .find(|c| c.name == server_name)
            .ok_or_else(|| anyhow!("MCP server not found: {}", server_name))?;
        let tool_def = conn.tools.iter().find(|t| t.name == tool_name);
        let task = Self::task_payload_for_tool(tool_def)?;
        let task_enabled = task.is_some();
        let task_support = tool_def
            .and_then(|t| t.execution.as_ref())
            .and_then(|e| e.task_support)
            .map(|v| format!("{:?}", v))
            .unwrap_or_else(|| "None".to_string());
        let started = Instant::now();
        tracing::debug!(
            target: "agime::metrics",
            event = "mcp_tool_call_start",
            tool_name,
            server_name,
            task_enabled,
            task_support = task_support.as_str()
        );

        let arguments = match input {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            other => Some(serde_json::Map::from_iter([("input".to_string(), other)])),
        };

        let request = ClientRequest::CallToolRequest(CallToolRequest {
            params: CallToolRequestParams {
                meta: None,
                name: original_name.to_string().into(),
                arguments,
                task,
            },
            method: Default::default(),
            extensions: Default::default(),
        });

        let timeout_secs = Self::task_timeout_secs();
        let timeout = std::time::Duration::from_secs(timeout_secs);
        let deadline = started + timeout;
        let result = Self::send_request_with_timeout(conn, request, &cancel_token, timeout)
            .await
            .map_err(|e| anyhow!("Tool call failed: {}", e))?;

        match result {
            ServerResult::CallToolResult(call_result) => {
                tracing::debug!(
                    target: "agime::metrics",
                    event = "mcp_tool_call_done",
                    tool_name,
                    server_name,
                    task_enabled,
                    task_support = task_support.as_str(),
                    is_error = call_result.is_error,
                    latency_ms = started.elapsed().as_millis() as u64
                );
                Ok(call_result)
            }
            ServerResult::CreateTaskResult(task_result) => {
                tracing::info!(
                    target: "agime::metrics",
                    event = "mcp_tool_task_created",
                    tool_name,
                    server_name,
                    task_id = task_result.task.task_id.as_str(),
                    task_status = format!("{:?}", task_result.task.status).as_str(),
                    task_support = task_support.as_str()
                );

                let call_result = self
                    .wait_for_task_result(
                        conn,
                        tool_name,
                        server_name,
                        task_result,
                        progress_cb,
                        &cancel_token,
                        deadline,
                        timeout_secs,
                    )
                    .await?;

                tracing::debug!(
                    target: "agime::metrics",
                    event = "mcp_tool_call_done_via_task",
                    tool_name,
                    server_name,
                    task_enabled,
                    task_support = task_support.as_str(),
                    latency_ms = started.elapsed().as_millis() as u64
                );
                Ok(call_result)
            }
            _ => Err(anyhow!("Unexpected response for call_tool")),
        }
    }

    async fn send_request_with_timeout(
        conn: &McpConnection,
        request: ClientRequest,
        cancel_token: &CancellationToken,
        timeout: std::time::Duration,
    ) -> Result<ServerResult> {
        let client = conn.client.lock().await;
        let handle = client
            .send_cancellable_request(request, PeerRequestOptions::no_options())
            .await
            .map_err(|e| anyhow!("Failed to send MCP request: {}", e))?;
        drop(client);

        let receiver = handle.rx;
        let peer = handle.peer;
        let request_id = handle.id;

        tokio::select! {
            res = receiver => {
                res.map_err(|_| anyhow!("MCP transport closed"))?
                    .map_err(|e| anyhow!("{}", e))
            }
            _ = tokio::time::sleep(timeout) => {
                let _ = send_cancel_notification(&peer, request_id, "timed out").await;
                Err(anyhow!("MCP request timed out after {}ms", timeout.as_millis()))
            }
            _ = cancel_token.cancelled() => {
                let _ = send_cancel_notification(&peer, request_id, "operation cancelled").await;
                Err(anyhow!("MCP request cancelled"))
            }
        }
    }

    fn poll_interval_for_task(task: &rmcp::model::Task) -> std::time::Duration {
        let ms = task.poll_interval.unwrap_or(1000).clamp(200, 10_000);
        std::time::Duration::from_millis(ms)
    }

    fn remaining_until(deadline: Instant) -> Option<std::time::Duration> {
        deadline.checked_duration_since(Instant::now())
    }

    async fn best_effort_cancel_task(conn: &McpConnection, task_id: &str) {
        let request = ClientRequest::CancelTaskRequest(CancelTaskRequest {
            params: CancelTaskParams {
                meta: None,
                task_id: task_id.to_string(),
            },
            method: Default::default(),
            extensions: Default::default(),
        });
        let token = CancellationToken::new();
        let _ = Self::send_request_with_timeout(
            conn,
            request,
            &token,
            std::time::Duration::from_secs(5),
        )
        .await;
    }

    fn task_result_to_call_result(
        task_id: &str,
        task_result: TaskResult,
    ) -> Result<CallToolResult> {
        let content_type = task_result.content_type.clone();
        let summary = task_result.summary.clone();
        let value = task_result.value;
        let envelope = serde_json::json!({
            "task_id": task_id,
            "content_type": content_type,
            "summary": summary,
            "value": value.clone(),
        });

        let text = if content_type.starts_with("text/") {
            value.as_str().map(|s| s.to_string()).unwrap_or_else(|| {
                serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| envelope.to_string())
            })
        } else {
            serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| envelope.to_string())
        };

        let mut call_result = CallToolResult::success(vec![rmcp::model::Content::text(text)]);
        if let Some(map) = value.as_object() {
            call_result.structured_content = Some(serde_json::Value::Object(map.clone()));
        }
        Ok(call_result)
    }

    fn task_status_label(status: &TaskStatus) -> &'static str {
        match status {
            TaskStatus::Working => "working",
            TaskStatus::InputRequired => "input_required",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    fn emit_task_progress(
        progress_cb: Option<&ToolTaskProgressCallback>,
        tool_name: &str,
        server_name: &str,
        task_id: &str,
        status: &TaskStatus,
        status_message: Option<String>,
        poll_count: u32,
    ) {
        if let Some(cb) = progress_cb {
            cb(ToolTaskProgress {
                tool_name: tool_name.to_string(),
                server_name: server_name.to_string(),
                task_id: task_id.to_string(),
                status: Self::task_status_label(status).to_string(),
                status_message,
                poll_count,
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn wait_for_task_result(
        &self,
        conn: &McpConnection,
        tool_name: &str,
        server_name: &str,
        task_result: CreateTaskResult,
        progress_cb: Option<ToolTaskProgressCallback>,
        cancel_token: &CancellationToken,
        deadline: Instant,
        timeout_secs: u64,
    ) -> Result<CallToolResult> {
        let mut task = task_result.task;
        let task_id = task.task_id.clone();
        let mut poll_count: u32 = 0;
        let mut last_progress_key = Some((
            Self::task_status_label(&task.status).to_string(),
            task.status_message.clone(),
        ));

        Self::emit_task_progress(
            progress_cb.as_ref(),
            tool_name,
            server_name,
            &task_id,
            &task.status,
            task.status_message.clone(),
            poll_count,
        );

        loop {
            match &task.status {
                TaskStatus::Completed => {
                    let remaining = Self::remaining_until(deadline).ok_or_else(|| {
                        anyhow!(
                            "Tool call '{}' timed out after {}s while waiting task result",
                            tool_name,
                            timeout_secs
                        )
                    })?;

                    let result_request =
                        ClientRequest::GetTaskResultRequest(GetTaskResultRequest {
                            params: GetTaskResultParams {
                                meta: None,
                                task_id: task_id.clone(),
                            },
                            method: Default::default(),
                            extensions: Default::default(),
                        });
                    let result = Self::send_request_with_timeout(
                        conn,
                        result_request,
                        cancel_token,
                        remaining,
                    )
                    .await
                    .map_err(|e| anyhow!("Task {} result retrieval failed: {}", task_id, e))?;
                    return match result {
                        ServerResult::TaskResult(task_result) => {
                            Self::task_result_to_call_result(&task_id, task_result)
                        }
                        _ => Err(anyhow!(
                            "Unexpected response for tasks/result of task {}",
                            task_id
                        )),
                    };
                }
                TaskStatus::Failed => {
                    return Err(anyhow!(
                        "Tool task {} failed{}",
                        task_id,
                        task.status_message
                            .as_ref()
                            .map(|m| format!(": {}", m))
                            .unwrap_or_default()
                    ));
                }
                TaskStatus::Cancelled => {
                    return Err(anyhow!(
                        "Tool task {} was cancelled{}",
                        task_id,
                        task.status_message
                            .as_ref()
                            .map(|m| format!(": {}", m))
                            .unwrap_or_default()
                    ));
                }
                TaskStatus::InputRequired => {
                    Self::emit_task_progress(
                        progress_cb.as_ref(),
                        tool_name,
                        server_name,
                        &task_id,
                        &task.status,
                        task.status_message.clone(),
                        poll_count,
                    );
                    return Err(anyhow!(
                        "Tool task {} requires additional input{}",
                        task_id,
                        task.status_message
                            .as_ref()
                            .map(|m| format!(": {}", m))
                            .unwrap_or_default()
                    ));
                }
                TaskStatus::Working => {}
            }

            let remaining = match Self::remaining_until(deadline) {
                Some(r) if !r.is_zero() => r,
                _ => {
                    tracing::warn!(
                        target: "agime::metrics",
                        event = "mcp_tool_task_timeout",
                        tool_name,
                        server_name,
                        task_id = task_id.as_str(),
                        timeout_secs
                    );
                    Self::best_effort_cancel_task(conn, &task_id).await;
                    return Err(anyhow!(
                        "Tool call '{}' timed out after {}s while polling task {}",
                        tool_name,
                        timeout_secs,
                        task_id
                    ));
                }
            };

            let info_request = ClientRequest::GetTaskInfoRequest(GetTaskInfoRequest {
                params: GetTaskInfoParams {
                    meta: None,
                    task_id: task_id.clone(),
                },
                method: Default::default(),
                extensions: Default::default(),
            });

            let info_res = Self::send_request_with_timeout(
                conn,
                info_request,
                cancel_token,
                remaining.min(std::time::Duration::from_secs(30)),
            )
            .await
            .map_err(|e| anyhow!("Task {} polling failed: {}", task_id, e))?;

            match info_res {
                ServerResult::GetTaskInfoResult(info) => {
                    if let Some(next_task) = info.task {
                        task = next_task;
                        poll_count = poll_count.saturating_add(1);
                        let next_key = (
                            Self::task_status_label(&task.status).to_string(),
                            task.status_message.clone(),
                        );
                        let should_emit = last_progress_key
                            .as_ref()
                            .map(|prev| prev != &next_key)
                            .unwrap_or(true)
                            || poll_count.is_multiple_of(5);
                        if should_emit {
                            Self::emit_task_progress(
                                progress_cb.as_ref(),
                                tool_name,
                                server_name,
                                &task_id,
                                &task.status,
                                task.status_message.clone(),
                                poll_count,
                            );
                            last_progress_key = Some(next_key);
                        }
                    } else {
                        tracing::debug!(
                            target: "agime::metrics",
                            event = "mcp_tool_task_info_empty",
                            tool_name,
                            server_name,
                            task_id = task_id.as_str()
                        );
                    }
                }
                _ => {
                    return Err(anyhow!(
                        "Unexpected response for tasks/get of task {}",
                        task_id
                    ));
                }
            }

            let wait_for = Self::poll_interval_for_task(&task)
                .min(Self::remaining_until(deadline).unwrap_or_default());
            if wait_for.is_zero() {
                continue;
            }
            tokio::select! {
                _ = tokio::time::sleep(wait_for) => {}
                _ = cancel_token.cancelled() => {
                    tracing::warn!(
                        target: "agime::metrics",
                        event = "mcp_tool_task_cancelled",
                        tool_name,
                        server_name,
                        task_id = task_id.as_str()
                    );
                    Self::best_effort_cancel_task(conn, &task_id).await;
                    return Err(anyhow!("Tool call '{}' cancelled", tool_name));
                }
            }
        }
    }

    /// Execute a tool call by prefixed name, returns the result as a string
    pub async fn call_tool(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        cancel_token: CancellationToken,
    ) -> Result<String> {
        let call_result = self
            .send_tool_call(tool_name, input, None, cancel_token)
            .await?;
        Self::extract_tool_result_text(&call_result)
    }

    /// Extract text content from a CallToolResult
    fn extract_tool_result_text(result: &CallToolResult) -> Result<String> {
        let mut texts = Vec::new();
        for content in &result.content {
            // Content is an enum, extract text from it
            let text = match content.raw {
                rmcp::model::RawContent::Text(ref t) => t.text.clone(),
                rmcp::model::RawContent::Image(ref img) => {
                    format!("[Image: {}]", img.mime_type)
                }
                rmcp::model::RawContent::Audio(ref audio) => {
                    format!("[Audio: {}]", audio.mime_type)
                }
                rmcp::model::RawContent::Resource(ref res) => match &res.resource {
                    rmcp::model::ResourceContents::TextResourceContents { uri, text, .. } => {
                        if text.is_empty() {
                            format!("[Resource: {}]", uri)
                        } else {
                            text.clone()
                        }
                    }
                    rmcp::model::ResourceContents::BlobResourceContents { uri, .. } => {
                        format!("[Blob Resource: {}]", uri)
                    }
                },
                rmcp::model::RawContent::ResourceLink(ref link) => {
                    format!("[ResourceLink: {}]", link.uri)
                }
            };
            texts.push(text);
        }
        Ok(texts.join("\n"))
    }

    /// Extract structured content blocks from a CallToolResult (supports multimodal).
    /// Public so other modules (e.g. PlatformExtensionRunner) can reuse this logic.
    pub fn extract_tool_result_blocks(result: &CallToolResult) -> Vec<ToolContentBlock> {
        let mut blocks = Vec::new();
        for content in &result.content {
            match content.raw {
                rmcp::model::RawContent::Text(ref t) => {
                    blocks.push(ToolContentBlock::Text(t.text.clone()));
                }
                rmcp::model::RawContent::Image(ref img) => {
                    blocks.push(ToolContentBlock::Image {
                        mime_type: img.mime_type.clone(),
                        data: img.data.clone(),
                    });
                }
                _ => {
                    // For other types, fall back to text representation
                    let text = match content.raw {
                        rmcp::model::RawContent::Audio(ref audio) => {
                            format!("[Audio: {}]", audio.mime_type)
                        }
                        rmcp::model::RawContent::Resource(ref res) => match &res.resource {
                            rmcp::model::ResourceContents::TextResourceContents {
                                uri,
                                text,
                                ..
                            } => {
                                if text.is_empty() {
                                    format!("[Resource: {}]", uri)
                                } else {
                                    text.clone()
                                }
                            }
                            rmcp::model::ResourceContents::BlobResourceContents { uri, .. } => {
                                format!("[Blob Resource: {}]", uri)
                            }
                        },
                        rmcp::model::RawContent::ResourceLink(ref link) => {
                            format!("[ResourceLink: {}]", link.uri)
                        }
                        _ => String::new(),
                    };
                    if !text.is_empty() {
                        blocks.push(ToolContentBlock::Text(text));
                    }
                }
            }
        }
        blocks
    }

    /// Execute a tool call, returns structured content blocks (supports multimodal)
    pub async fn call_tool_rich(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        cancel_token: CancellationToken,
    ) -> Result<Vec<ToolContentBlock>> {
        let call_result = self
            .send_tool_call(tool_name, input, None, cancel_token)
            .await?;
        Ok(Self::extract_tool_result_blocks(&call_result))
    }

    pub async fn call_tool_rich_with_progress(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        progress_cb: Option<ToolTaskProgressCallback>,
        cancel_token: CancellationToken,
    ) -> Result<Vec<ToolContentBlock>> {
        let call_result = self
            .send_tool_call(tool_name, input, progress_cb, cancel_token)
            .await?;
        Ok(Self::extract_tool_result_blocks(&call_result))
    }

    /// Dynamically add a new MCP extension at runtime.
    /// Connects to the server and returns the list of new tool names added.
    pub async fn add_extension(
        &mut self,
        ext: &CustomExtensionConfig,
        api_caller: Option<Arc<dyn ApiCaller>>,
    ) -> Result<Vec<String>> {
        // Don't add if already connected
        if self.connections.iter().any(|c| c.name == ext.name) {
            return Err(anyhow!("Extension '{}' is already connected", ext.name));
        }

        let conn =
            Self::connect_one(ext, api_caller, self.elicitation_bridge.clone(), None).await?;
        let tool_names: Vec<String> = conn.tools.iter().map(|t| t.name.clone()).collect();
        info!(
            "Dynamically added MCP extension '{}': {} tools",
            conn.name,
            conn.tools.len()
        );
        self.connections.push(conn);
        Ok(tool_names)
    }

    /// Dynamically remove an MCP extension by name.
    /// Disconnects the server and returns the list of tool names that were removed.
    pub async fn remove_extension(&mut self, name: &str) -> Result<Vec<String>> {
        let idx = self
            .connections
            .iter()
            .position(|c| c.name == name)
            .ok_or_else(|| anyhow!("Extension '{}' not found", name))?;

        let conn = self.connections.remove(idx);
        let tool_names: Vec<String> = conn.tools.iter().map(|t| t.name.clone()).collect();

        // Disconnect the MCP server
        let client = conn.client.into_inner();
        if let Err(e) = client.cancel().await {
            warn!("Error disconnecting MCP server '{}': {}", name, e);
        } else {
            info!("Dynamically removed MCP extension '{}'", name);
        }

        Ok(tool_names)
    }

    /// Check if an extension with the given name is currently connected.
    pub fn has_extension(&self, name: &str) -> bool {
        self.connections.iter().any(|c| c.name == name)
    }

    /// Shutdown all MCP connections
    pub async fn shutdown(mut self) {
        for conn in std::mem::take(&mut self.connections) {
            let name = conn.name.clone();
            let mut client = conn.client.into_inner();
            match tokio::time::timeout(std::time::Duration::from_secs(2), client.close()).await {
                Ok(Ok(_)) => {
                    info!("MCP server '{}' disconnected", name);
                }
                Ok(Err(e)) => {
                    warn!(
                        "Graceful close failed for MCP server '{}': {}, falling back to cancel",
                        name, e
                    );
                    if let Err(cancel_err) = client.cancel().await {
                        error!(
                            "Error cancelling MCP server '{}' after close failure: {}",
                            name, cancel_err
                        );
                    }
                }
                Err(_) => {
                    warn!(
                        "Graceful close timed out for MCP server '{}', falling back to cancel",
                        name
                    );
                    if let Err(cancel_err) = client.cancel().await {
                        error!(
                            "Error cancelling MCP server '{}' after close timeout: {}",
                            name, cancel_err
                        );
                    }
                }
            }
        }
    }
}

impl Drop for McpConnector {
    fn drop(&mut self) {
        let connections = std::mem::take(&mut self.connections);
        if connections.is_empty() {
            return;
        }
        let count = connections.len();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                for conn in connections {
                    let name = conn.name.clone();
                    let mut client = conn.client.into_inner();
                    match tokio::time::timeout(std::time::Duration::from_secs(2), client.close())
                        .await
                    {
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => {
                            warn!(
                                "Drop: graceful close failed for MCP server '{}': {}, cancelling",
                                name, e
                            );
                            if let Err(cancel_err) = client.cancel().await {
                                error!(
                                    "Drop: error cancelling MCP server '{}' after close failure: {}",
                                    name, cancel_err
                                );
                            }
                        }
                        Err(_) => {
                            warn!(
                                "Drop: graceful close timed out for MCP server '{}', cancelling",
                                name
                            );
                            if let Err(cancel_err) = client.cancel().await {
                                error!(
                                    "Drop: error cancelling MCP server '{}' after close timeout: {}",
                                    name, cancel_err
                                );
                            }
                        }
                    }
                }
            });
        } else {
            warn!(
                "McpConnector dropped outside tokio runtime, {} MCP connections may leak",
                count
            );
        }
    }
}

/// Send a cancellation notification to an MCP peer (matching local agime behavior).
async fn send_cancel_notification(
    peer: &Peer<RoleClient>,
    request_id: RequestId,
    reason: &str,
) -> Result<()> {
    peer.send_notification(
        CancelledNotification {
            params: CancelledNotificationParam {
                request_id,
                reason: Some(reason.to_owned()),
            },
            method: CancelledNotificationMethod,
            extensions: Default::default(),
        }
        .into(),
    )
    .await
    .map_err(|e| anyhow!("Failed to send cancel notification: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn tool_with_task_support(task_support: TaskSupport) -> ToolDefinition {
        ToolDefinition {
            name: "ext__tool".to_string(),
            description: String::new(),
            input_schema: serde_json::json!({"type":"object"}),
            server_name: "ext".to_string(),
            original_name: "tool".to_string(),
            execution: Some(ToolExecution {
                task_support: Some(task_support),
            }),
        }
    }

    #[test]
    fn sampling_required_without_tools_rejected() {
        let err = AgentClientHandler::resolve_sampling_tools(
            None,
            Some(ToolChoice {
                mode: Some(ToolChoiceMode::Required),
            }),
        )
        .expect_err("required mode without tools must fail");
        assert!(err.message.contains("tool_choice=required"));
    }

    #[test]
    fn sampling_none_clears_tools() {
        let tool = Tool {
            name: "x".into(),
            title: None,
            description: None,
            input_schema: Default::default(),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        };
        let (tools, _) = AgentClientHandler::resolve_sampling_tools(
            Some(vec![tool]),
            Some(ToolChoice {
                mode: Some(ToolChoiceMode::None),
            }),
        )
        .expect("none mode should be valid");
        assert!(tools.is_empty());
    }

    #[test]
    fn sampling_required_rejects_non_tool_response() {
        let result = AgentClientHandler::validate_sampling_tool_choice_result(
            Some(&ToolChoice {
                mode: Some(ToolChoiceMode::Required),
            }),
            false,
        );
        assert!(result.is_err());
    }

    #[test]
    fn sampling_none_rejects_tool_response() {
        let result = AgentClientHandler::validate_sampling_tool_choice_result(
            Some(&ToolChoice {
                mode: Some(ToolChoiceMode::None),
            }),
            true,
        );
        assert!(result.is_err());
    }

    #[test]
    fn task_payload_gate_obeys_support_level() {
        let required = tool_with_task_support(TaskSupport::Required);
        let optional = tool_with_task_support(TaskSupport::Optional);
        let forbidden = tool_with_task_support(TaskSupport::Forbidden);

        assert!(
            McpConnector::task_payload_for_tool_with_gate(Some(&required), true)
                .expect("required+enabled should pass")
                .is_some()
        );
        assert!(
            McpConnector::task_payload_for_tool_with_gate(Some(&optional), true)
                .expect("optional+enabled should pass")
                .is_some()
        );
        assert!(
            McpConnector::task_payload_for_tool_with_gate(Some(&forbidden), true)
                .expect("forbidden should pass")
                .is_none()
        );
        assert!(
            McpConnector::task_payload_for_tool_with_gate(Some(&optional), false)
                .expect("optional+disabled should pass")
                .is_none()
        );
        assert!(
            McpConnector::task_payload_for_tool_with_gate(Some(&required), false).is_err(),
            "required+disabled should fail"
        );
    }

    #[test]
    fn sampling_messages_preserve_structured_tool_blocks() {
        let msg = SamplingMessage::new_multiple(
            Role::Assistant,
            vec![
                SamplingMessageContent::text("hello"),
                SamplingMessageContent::tool_use(
                    "call_1",
                    "search_web",
                    serde_json::Map::from_iter([(
                        "q".to_string(),
                        serde_json::Value::String("rmcp".to_string()),
                    )]),
                ),
                SamplingMessageContent::tool_result("call_1", Vec::new()),
            ],
        );

        let encoded = AgentClientHandler::sampling_messages_to_api_messages(&[msg]);
        let content = encoded[0]["content"]
            .as_array()
            .expect("content should be structured array");
        assert_eq!(content.len(), 3);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "tool_use");
        assert_eq!(content[2]["type"], "tool_result");
    }

    #[test]
    fn task_result_conversion_keeps_structured_payload() {
        let task_result = TaskResult {
            content_type: "application/json".to_string(),
            value: serde_json::json!({"ok": true, "count": 2}),
            summary: Some("done".to_string()),
        };
        let converted =
            McpConnector::task_result_to_call_result("task_1", task_result).expect("convert");
        assert_eq!(converted.is_error, Some(false));
        assert!(converted.structured_content.is_some());
        let text = converted
            .content
            .iter()
            .find_map(|c| c.as_text().map(|t| t.text.clone()))
            .unwrap_or_default();
        assert!(text.contains("\"task_id\": \"task_1\""));
        assert!(text.contains("\"content_type\": \"application/json\""));
    }

    #[test]
    fn task_result_conversion_plain_text_returns_text_content() {
        let task_result = TaskResult {
            content_type: "text/plain".to_string(),
            value: serde_json::json!("hello from task"),
            summary: None,
        };
        let converted =
            McpConnector::task_result_to_call_result("task_2", task_result).expect("convert");
        let text = converted
            .content
            .iter()
            .find_map(|c| c.as_text().map(|t| t.text.clone()))
            .unwrap_or_default();
        assert_eq!(text, "hello from task");
    }

    #[test]
    fn elicitation_default_action_is_cancel_when_unset() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::remove_var("TEAM_MCP_ELICITATION_DEFAULT_ACTION");
        assert_eq!(
            AgentClientHandler::elicitation_default_action(),
            ElicitationAction::Cancel
        );
    }

    #[test]
    fn elicitation_default_action_supports_decline() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::set_var("TEAM_MCP_ELICITATION_DEFAULT_ACTION", "decline");
        assert_eq!(
            AgentClientHandler::elicitation_default_action(),
            ElicitationAction::Decline
        );
        std::env::remove_var("TEAM_MCP_ELICITATION_DEFAULT_ACTION");
    }

    #[test]
    fn elicitation_default_action_supports_accept() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::set_var("TEAM_MCP_ELICITATION_DEFAULT_ACTION", "accept");
        assert_eq!(
            AgentClientHandler::elicitation_default_action(),
            ElicitationAction::Accept
        );
        std::env::remove_var("TEAM_MCP_ELICITATION_DEFAULT_ACTION");
    }

    #[test]
    fn form_elicitation_accept_without_content_falls_back_to_cancel() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::set_var("TEAM_MCP_ELICITATION_DEFAULT_ACTION", "accept");
        std::env::remove_var("TEAM_MCP_ELICITATION_DEFAULT_CONTENT_JSON");
        let req = CreateElicitationRequestParams::FormElicitationParams {
            meta: None,
            message: "m".into(),
            requested_schema: serde_json::from_value(serde_json::json!({
                "type": "object",
                "properties": {
                    "answer": { "type": "string" }
                },
                "required": ["answer"]
            }))
            .expect("schema"),
        };
        let (action, content) = AgentClientHandler::resolve_elicitation_policy(&req);
        assert_eq!(action, ElicitationAction::Cancel);
        assert!(content.is_none());
        std::env::remove_var("TEAM_MCP_ELICITATION_DEFAULT_ACTION");
    }

    #[test]
    fn form_elicitation_accept_with_content_is_kept() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::set_var("TEAM_MCP_ELICITATION_DEFAULT_ACTION", "accept");
        std::env::set_var(
            "TEAM_MCP_ELICITATION_DEFAULT_CONTENT_JSON",
            r#"{"answer":"ok"}"#,
        );
        let req = CreateElicitationRequestParams::FormElicitationParams {
            meta: None,
            message: "m".into(),
            requested_schema: serde_json::from_value(serde_json::json!({
                "type": "object",
                "properties": {
                    "answer": { "type": "string" }
                },
                "required": ["answer"]
            }))
            .expect("schema"),
        };
        let (action, content) = AgentClientHandler::resolve_elicitation_policy(&req);
        assert_eq!(action, ElicitationAction::Accept);
        assert_eq!(
            content
                .as_ref()
                .and_then(|v| v.get("answer"))
                .and_then(|v| v.as_str()),
            Some("ok")
        );
        std::env::remove_var("TEAM_MCP_ELICITATION_DEFAULT_ACTION");
        std::env::remove_var("TEAM_MCP_ELICITATION_DEFAULT_CONTENT_JSON");
    }
}
