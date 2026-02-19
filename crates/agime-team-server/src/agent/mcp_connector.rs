//! MCP Connector for agent tool calling
//!
//! Connects to MCP servers (stdio/SSE/StreamableHttp), collects tool definitions,
//! and executes tool calls. Independent of the agime crate.

use agime_team::models::CustomExtensionConfig;
use anyhow::{anyhow, Result};
use rmcp::model::{
    CallToolRequestParam, CallToolResult, CancelledNotification, CancelledNotificationMethod,
    CancelledNotificationParam, ClientInfo, ClientRequest, Content, CreateMessageRequestParam,
    CreateMessageResult, Implementation, ListToolsRequest, PaginatedRequestParam, ProtocolVersion,
    RequestId, Role, SamplingMessage, ServerResult,
};
use rmcp::service::{PeerRequestOptions, RequestContext, RunningService, ServiceRole};
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{ClientHandler, ErrorData, Peer, RoleClient, ServiceError, ServiceExt};
use std::process::Stdio;
use std::sync::Arc;
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
}

/// A content block returned from a tool call (supports multimodal)
#[derive(Debug, Clone)]
pub enum ToolContentBlock {
    Text(String),
    Image { mime_type: String, data: String }, // base64 data
}

/// A single MCP server connection
struct McpConnection {
    name: String,
    client: Mutex<RunningService<RoleClient, AgentClientHandler>>,
    tools: Vec<ToolDefinition>,
}

/// Trait for making LLM API calls (used by MCP Sampling)
pub trait ApiCaller: Send + Sync {
    fn call_llm<'a>(
        &'a self,
        system: &'a str,
        messages: Vec<serde_json::Value>,
        max_tokens: u32,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value>> + Send + 'a>>;
}

/// Client handler that supports MCP Sampling via an ApiCaller
struct AgentClientHandler {
    api_caller: Option<Arc<dyn ApiCaller>>,
}

impl ClientHandler for AgentClientHandler {
    fn get_info(&self) -> ClientInfo {
        ClientInfo {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: Default::default(),
            client_info: Implementation {
                name: "agime-team-server".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                icons: None,
                title: None,
                website_url: None,
            },
        }
    }

    async fn create_message(
        &self,
        params: CreateMessageRequestParam,
        _context: RequestContext<RoleClient>,
    ) -> std::result::Result<CreateMessageResult, ErrorData> {
        let api_caller = self.api_caller.as_ref().ok_or_else(|| {
            ErrorData::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                "MCP Sampling not supported: no API caller configured",
                None,
            )
        })?;

        // Convert SamplingMessages to JSON messages for the API
        let messages: Vec<serde_json::Value> = params
            .messages
            .iter()
            .map(|msg| {
                let role = match msg.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                let text = msg
                    .content
                    .raw
                    .as_text()
                    .map(|t| t.text.clone())
                    .unwrap_or_default();
                serde_json::json!({ "role": role, "content": text })
            })
            .collect();

        let system = params.system_prompt.as_deref().unwrap_or("");
        let max_tokens = params.max_tokens;

        // Call the LLM via the ApiCaller
        let response = api_caller
            .call_llm(system, messages, max_tokens)
            .await
            .map_err(|e| {
                ErrorData::new(
                    rmcp::model::ErrorCode::INTERNAL_ERROR,
                    format!("LLM call failed: {}", e),
                    None,
                )
            })?;

        // Extract text from response (try Anthropic format, then OpenAI format)
        let text = if let Some(content) = response["content"].as_array() {
            // Anthropic format
            content
                .iter()
                .filter_map(|b| {
                    if b["type"].as_str() == Some("text") {
                        b["text"].as_str()
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        } else if let Some(text) = response["choices"][0]["message"]["content"].as_str() {
            // OpenAI format
            text.to_string()
        } else {
            String::new()
        };

        let model = response["model"].as_str().unwrap_or("unknown").to_string();

        Ok(CreateMessageResult {
            model,
            stop_reason: Some("endTurn".to_string()),
            message: SamplingMessage {
                role: Role::Assistant,
                content: Content::text(text),
            },
        })
    }
}

/// MCP Connector that manages connections to multiple MCP servers
pub struct McpConnector {
    connections: Vec<McpConnection>,
}

impl McpConnector {
    /// Create an empty McpConnector with no connections.
    pub fn empty() -> Self {
        Self {
            connections: Vec::new(),
        }
    }

    /// Connect to MCP servers defined in the agent's custom_extensions.
    /// Only connects to enabled extensions with supported types (sse, stdio).
    /// If `api_caller` is provided, MCP Sampling will be supported.
    /// If `workspace_path` is provided, stdio child processes will use it as CWD.
    pub async fn connect(
        extensions: &[CustomExtensionConfig],
        api_caller: Option<Arc<dyn ApiCaller>>,
        workspace_path: Option<&str>,
    ) -> Result<Self> {
        let mut connections = Vec::new();

        for ext in extensions {
            if !ext.enabled {
                info!("Skipping disabled extension: {}", ext.name);
                continue;
            }

            match Self::connect_one(ext, api_caller.clone(), workspace_path).await {
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

        Ok(Self { connections })
    }

    /// Connect to a single MCP server (with 30s timeout)
    async fn connect_one(
        ext: &CustomExtensionConfig,
        api_caller: Option<Arc<dyn ApiCaller>>,
        workspace_path: Option<&str>,
    ) -> Result<McpConnection> {
        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            Self::connect_one_inner(ext, api_caller, workspace_path),
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
        workspace_path: Option<&str>,
    ) -> Result<McpConnection> {
        let handler = AgentClientHandler { api_caller };
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

        Ok(McpConnection {
            name: ext.name.clone(),
            client: Mutex::new(running),
            tools,
        })
    }

    /// Connect via StreamableHttp (also handles SSE in rmcp 0.12.0)
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
            use std::os::windows::process::CommandExt;
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
                params: Some(PaginatedRequestParam {
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

        let arguments = match input {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            other => Some(serde_json::Map::from_iter([("input".to_string(), other)])),
        };

        let request = ClientRequest::CallToolRequest(rmcp::model::CallToolRequest {
            params: CallToolRequestParam {
                name: original_name.to_string().into(),
                arguments,
            },
            method: Default::default(),
            extensions: Default::default(),
        });

        let timeout_secs = std::env::var("TEAM_MCP_TOOL_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(300);
        let timeout = std::time::Duration::from_secs(timeout_secs);

        // Send cancellable request — returns a handle we can select on
        let client = conn.client.lock().await;
        let handle = client
            .send_cancellable_request(request, PeerRequestOptions::no_options())
            .await
            .map_err(|e| anyhow!("Failed to send tool call: {}", e))?;

        let receiver = handle.rx;
        let peer = handle.peer;
        let request_id = handle.id;
        // Drop the lock before awaiting the response
        drop(client);

        let result = tokio::select! {
            res = receiver => {
                res.map_err(|_| anyhow!("MCP transport closed"))?
                   .map_err(|e| anyhow!("Tool call failed: {}", e))?
            }
            _ = tokio::time::sleep(timeout) => {
                let _ = send_cancel_notification(&peer, request_id, "timed out").await;
                return Err(anyhow!("Tool call '{}' timed out after {}s", tool_name, timeout_secs));
            }
            _ = cancel_token.cancelled() => {
                let _ = send_cancel_notification(&peer, request_id, "operation cancelled").await;
                return Err(anyhow!("Tool call '{}' cancelled", tool_name));
            }
        };

        match result {
            ServerResult::CallToolResult(call_result) => Ok(call_result),
            _ => Err(anyhow!("Unexpected response for call_tool")),
        }
    }

    /// Execute a tool call by prefixed name, returns the result as a string
    pub async fn call_tool(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        cancel_token: CancellationToken,
    ) -> Result<String> {
        let call_result = self.send_tool_call(tool_name, input, cancel_token).await?;
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
        let call_result = self.send_tool_call(tool_name, input, cancel_token).await?;
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

        let conn = Self::connect_one(ext, api_caller, None).await?;
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
            let client = conn.client.into_inner();
            if let Err(e) = client.cancel().await {
                error!("Error shutting down MCP server '{}': {}", conn.name, e);
            } else {
                info!("MCP server '{}' disconnected", conn.name);
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
                    let client = conn.client.into_inner();
                    if let Err(e) = client.cancel().await {
                        error!("Drop: error shutting down MCP server '{}': {}", name, e);
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
