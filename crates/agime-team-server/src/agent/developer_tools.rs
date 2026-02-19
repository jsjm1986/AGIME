//! In-process Developer extension bridge for team-server.
//!
//! Runs DeveloperServer in-process via tokio::io::duplex() transport,
//! eliminating subprocess startup overhead (~seconds per task).

use agime::agents::mcp_client::McpClientTrait;
use agime_mcp::developer::rmcp_developer::DeveloperServer;
use rmcp::model::*;
use rmcp::service::{RunningService, RoleClient};
use rmcp::{ClientHandler, ServiceError, ServiceExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

/// Minimal client handler for in-process MCP bridge (no sampling needed).
struct InProcessClientHandler;

impl ClientHandler for InProcessClientHandler {
    fn get_info(&self) -> ClientInfo {
        ClientInfo {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: Default::default(),
            client_info: Implementation {
                name: "agime-team-server-inproc".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                icons: None,
                title: None,
                website_url: None,
            },
        }
    }
}

/// In-process developer extension that implements McpClientTrait.
pub struct DeveloperToolsProvider {
    client: Mutex<RunningService<RoleClient, InProcessClientHandler>>,
    server_info: Option<InitializeResult>,
}

impl DeveloperToolsProvider {
    /// Create a new in-process developer extension.
    ///
    /// Spawns DeveloperServer on one end of a duplex channel and connects
    /// an MCP client on the other end — all within the same process.
    pub async fn new(workspace_path: Option<&str>) -> anyhow::Result<Self> {
        let mut server = DeveloperServer::new().extend_path_with_shell(true);

        // Set per-instance working directory (safe for multi-tenant)
        if let Some(wp) = workspace_path {
            server = server.working_dir(std::path::PathBuf::from(wp));
        }

        // Create in-memory duplex transport (64KB buffer)
        let (client_stream, server_stream) = tokio::io::duplex(65536);

        // Serve DeveloperServer on one end (background task)
        let (server_read, server_write) = tokio::io::split(server_stream);
        tokio::spawn(async move {
            match server.serve((server_read, server_write)).await {
                Ok(svc) => {
                    let _ = svc.waiting().await;
                }
                Err(e) => {
                    tracing::warn!("In-process developer server error: {}", e);
                }
            }
        });

        // Connect client on the other end
        let (client_read, client_write) = tokio::io::split(client_stream);
        let handler = InProcessClientHandler;
        let running: RunningService<RoleClient, InProcessClientHandler> =
            handler.serve((client_read, client_write)).await.map_err(|e| {
                anyhow::anyhow!("Failed to connect in-process developer client: {}", e)
            })?;
        let server_info = running.peer_info().cloned();

        Ok(Self {
            client: Mutex::new(running),
            server_info,
        })
    }

    async fn send_request(
        &self,
        request: ClientRequest,
        cancel_token: CancellationToken,
    ) -> Result<ServerResult, ServiceError> {
        let handle = self
            .client
            .lock()
            .await
            .send_cancellable_request(request, rmcp::service::PeerRequestOptions::no_options())
            .await?;

        let receiver = handle.rx;
        let timeout = Duration::from_secs(3600); // 1 hour for shell commands

        tokio::select! {
            result = receiver => {
                result.map_err(|_| ServiceError::TransportClosed)?
            }
            _ = tokio::time::sleep(timeout) => {
                Err(ServiceError::Timeout { timeout })
            }
            _ = cancel_token.cancelled() => {
                Err(ServiceError::Cancelled { reason: None })
            }
        }
    }
}

#[async_trait::async_trait]
impl McpClientTrait for DeveloperToolsProvider {
    async fn list_resources(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListResourcesResult, ServiceError> {
        Ok(ListResourcesResult {
            resources: vec![],
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        _uri: &str,
        _cancel_token: CancellationToken,
    ) -> Result<ReadResourceResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn list_tools(
        &self,
        cursor: Option<String>,
        cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, ServiceError> {
        let res = self
            .send_request(
                ClientRequest::ListToolsRequest(ListToolsRequest {
                    params: Some(PaginatedRequestParam { cursor }),
                    method: Default::default(),
                    extensions: Default::default(),
                }),
                cancel_token,
            )
            .await?;
        match res {
            ServerResult::ListToolsResult(r) => Ok(r),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        cancel_token: CancellationToken,
    ) -> Result<CallToolResult, ServiceError> {
        let res = self
            .send_request(
                ClientRequest::CallToolRequest(CallToolRequest {
                    params: CallToolRequestParam {
                        name: name.to_string().into(),
                        arguments,
                    },
                    method: Default::default(),
                    extensions: Default::default(),
                }),
                cancel_token,
            )
            .await?;
        match res {
            ServerResult::CallToolResult(r) => Ok(r),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn list_prompts(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListPromptsResult, ServiceError> {
        Ok(ListPromptsResult {
            prompts: vec![],
            next_cursor: None,
            meta: None,
        })
    }

    async fn get_prompt(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
        _cancel_token: CancellationToken,
    ) -> Result<GetPromptResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        let (_tx, rx) = mpsc::channel(1);
        rx
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        self.server_info.as_ref()
    }
}
