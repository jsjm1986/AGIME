use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use agime::agents::mcp_client::McpClientTrait;
use agime_team::db::MongoDb;
use agime_team::services::mongo::DocumentService;
use anyhow::{anyhow, Result};
use mime_guess::MimeGuess;
use rmcp::model::*;
use rmcp::ServiceError;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::chat_channels::ChatWorkspaceFileBlock;
use super::service_mongo::AgentService;

pub struct ChatDeliveryToolsProvider {
    db: Arc<MongoDb>,
    team_id: String,
    session_id: String,
    info: InitializeResult,
}

impl ChatDeliveryToolsProvider {
    pub fn new(db: Arc<MongoDb>, team_id: String, session_id: String) -> Self {
        let info = InitializeResult {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {
                    list_changed: Some(false),
                }),
                resources: None,
                prompts: None,
                completions: None,
                experimental: None,
                logging: None,
                extensions: None,
                tasks: None,
            },
            server_info: Implementation {
                name: "chat_delivery".to_string(),
                title: Some("Chat Delivery".to_string()),
                version: "1.0.0".to_string(),
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Use attach_workspace_file_to_message after you have created or exported a file inside the current conversation workspace and the user expects that file to be delivered in the current assistant reply. If the user wants to download or preview an existing team document, prefer attach_document_to_message so the document is materialized and attached in one step. Prefer attaching the file instead of merely saying it was saved to the workspace. Do not invent URLs manually."
                    .to_string(),
            ),
        };
        Self {
            db,
            team_id,
            session_id,
            info,
        }
    }

    fn tool_definitions() -> Vec<Tool> {
        vec![
            Tool {
                name: "attach_workspace_file_to_message".into(),
                title: None,
                description: Some(
                    "Attach a workspace file to the current assistant reply so the direct chat reply can show download and preview actions. Only use files that already exist inside the current conversation workspace."
                        .into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Relative file path inside the current workspace, e.g. artifacts/report.md" },
                        "label": { "type": "string", "description": "Optional user-facing file label" }
                    },
                    "required": ["path"]
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "attach_document_to_message".into(),
                title: None,
                description: Some(
                    "Materialize an existing team document into the current conversation workspace and attach it to the current assistant reply so the user can preview or download it immediately."
                        .into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "doc_id": { "type": "string", "description": "Team document ID to deliver" },
                        "label": { "type": "string", "description": "Optional user-facing file label" },
                        "output_name": { "type": "string", "description": "Optional target file name inside the workspace" }
                    },
                    "required": ["doc_id"]
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
        ]
    }

    fn normalize_relative_path(path: &str) -> Result<String> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("path is required"));
        }
        let raw = Path::new(trimmed);
        if raw.is_absolute() {
            return Err(anyhow!("path must be a relative workspace path"));
        }

        let mut normalized = PathBuf::new();
        for component in raw.components() {
            match component {
                Component::Normal(part) => normalized.push(part),
                Component::CurDir => {}
                Component::ParentDir => {
                    return Err(anyhow!("path must stay inside the workspace"));
                }
                _ => return Err(anyhow!("path must be a relative workspace path")),
            }
        }

        let normalized = normalized.to_string_lossy().replace('\\', "/");
        if normalized.is_empty() {
            return Err(anyhow!("path is required"));
        }
        Ok(normalized)
    }

    fn preview_supported(path: &str, content_type: &str) -> bool {
        let lowered_path = path.to_ascii_lowercase();
        content_type.starts_with("text/")
            || matches!(
                content_type,
                "application/json"
                    | "application/pdf"
                    | "application/msword"
                    | "application/vnd.ms-excel"
                    | "application/vnd.ms-powerpoint"
                    | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                    | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                    | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                    | "image/svg+xml"
            )
            || content_type.starts_with("image/")
            || content_type.starts_with("audio/")
            || content_type.starts_with("video/")
            || lowered_path.ends_with(".csv")
            || lowered_path.ends_with(".json")
            || lowered_path.ends_with(".md")
            || lowered_path.ends_with(".txt")
            || lowered_path.ends_with(".html")
            || lowered_path.ends_with(".htm")
            || lowered_path.ends_with(".svg")
            || lowered_path.ends_with(".doc")
            || lowered_path.ends_with(".docx")
            || lowered_path.ends_with(".xls")
            || lowered_path.ends_with(".xlsx")
            || lowered_path.ends_with(".ppt")
            || lowered_path.ends_with(".pptx")
    }

    fn sanitize_output_name(name: &str) -> String {
        let sanitized = name
            .trim()
            .chars()
            .map(|ch| match ch {
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                _ => ch,
            })
            .collect::<String>();
        let trimmed = sanitized.trim_matches('.').trim();
        if trimmed.is_empty() {
            "document.bin".to_string()
        } else {
            trimmed.to_string()
        }
    }

    async fn handle_attach_workspace_file_to_message(
        &self,
        arguments: &JsonObject,
    ) -> Result<String> {
        let path = arguments
            .get("path")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("path is required"))?;
        let label = arguments
            .get("label")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());

        let service = AgentService::new(self.db.clone());
        let session = service
            .get_session(&self.session_id)
            .await?
            .ok_or_else(|| anyhow!("session not found"))?;

        if session.team_id != self.team_id {
            return Err(anyhow!("session does not belong to the active team"));
        }
        if !session.session_source.eq_ignore_ascii_case("chat") {
            return Err(anyhow!(
                "this tool is only available inside direct agent chat"
            ));
        }
        let workspace_root = session
            .workspace_path
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("workspace has not been established for this session yet"))?;

        let relative_path = Self::normalize_relative_path(path)?;
        let absolute_path = Path::new(workspace_root).join(&relative_path);
        if !absolute_path.is_file() {
            return Err(anyhow!("workspace file does not exist: {}", relative_path));
        }

        let metadata = std::fs::metadata(&absolute_path)?;
        let content_type = MimeGuess::from_path(&absolute_path)
            .first_raw()
            .unwrap_or("application/octet-stream")
            .to_string();
        let block = ChatWorkspaceFileBlock {
            block_type: "workspace_file".to_string(),
            path: relative_path.clone(),
            label: label.unwrap_or_else(|| {
                Path::new(&relative_path)
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_else(|| relative_path.clone())
            }),
            content_type: Some(content_type.clone()),
            size_bytes: Some(metadata.len() as i64),
            preview_supported: Self::preview_supported(&relative_path, &content_type),
        };
        service
            .queue_pending_message_workspace_file(&self.session_id, &block)
            .await?;

        Ok(serde_json::to_string_pretty(&json!({
            "attached": true,
            "file": block,
        }))?)
    }

    async fn handle_attach_document_to_message(&self, arguments: &JsonObject) -> Result<String> {
        let doc_id = arguments
            .get("doc_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("doc_id is required"))?;
        let label = arguments
            .get("label")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let requested_output_name = arguments
            .get("output_name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let service = AgentService::new(self.db.clone());
        let session = service
            .get_session(&self.session_id)
            .await?
            .ok_or_else(|| anyhow!("session not found"))?;
        if session.team_id != self.team_id {
            return Err(anyhow!("session does not belong to the active team"));
        }
        if !session.session_source.eq_ignore_ascii_case("chat") {
            return Err(anyhow!(
                "this tool is only available inside direct agent chat"
            ));
        }
        let workspace_root = session
            .workspace_path
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("workspace has not been established for this session yet"))?;

        let document_service = DocumentService::new((*self.db).clone());
        let metadata = document_service.get_metadata(&self.team_id, doc_id).await?;
        let (content, source_name, content_type) =
            document_service.download(&self.team_id, doc_id).await?;
        let file_name = Self::sanitize_output_name(requested_output_name.unwrap_or(&source_name));
        let relative_path = format!("artifacts/{}", file_name);
        let absolute_path = Path::new(workspace_root).join(&relative_path);
        if let Some(parent) = absolute_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&absolute_path, content)?;
        let stored_metadata = fs::metadata(&absolute_path)?;
        let final_content_type = if content_type.trim().is_empty() {
            MimeGuess::from_path(&absolute_path)
                .first_raw()
                .unwrap_or("application/octet-stream")
                .to_string()
        } else {
            content_type
        };

        let block = ChatWorkspaceFileBlock {
            block_type: "workspace_file".to_string(),
            path: relative_path.clone(),
            label: label.unwrap_or_else(|| metadata.name.clone()),
            content_type: Some(final_content_type.clone()),
            size_bytes: Some(stored_metadata.len() as i64),
            preview_supported: Self::preview_supported(&relative_path, &final_content_type),
        };
        service
            .queue_pending_message_workspace_file(&self.session_id, &block)
            .await?;

        Ok(serde_json::to_string_pretty(&json!({
            "attached": true,
            "document_id": doc_id,
            "document_name": metadata.name,
            "file": block,
        }))?)
    }
}

#[async_trait::async_trait]
impl McpClientTrait for ChatDeliveryToolsProvider {
    async fn list_resources(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListResourcesResult, ServiceError> {
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
    ) -> std::result::Result<ReadResourceResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn list_tools(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListToolsResult, ServiceError> {
        Ok(ListToolsResult {
            tools: Self::tool_definitions(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<CallToolResult, ServiceError> {
        let args = arguments.unwrap_or_default();
        let result = match name {
            "attach_workspace_file_to_message" => {
                self.handle_attach_workspace_file_to_message(&args).await
            }
            "attach_document_to_message" => self.handle_attach_document_to_message(&args).await,
            _ => Err(anyhow!("unknown tool: {}", name)),
        };

        match result {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(
                error.to_string(),
            )])),
        }
    }

    async fn list_tasks(
        &self,
        _cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListTasksResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn get_task_info(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<GetTaskInfoResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn get_task_result(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<TaskResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn cancel_task(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<(), ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn list_prompts(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListPromptsResult, ServiceError> {
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
    ) -> std::result::Result<GetPromptResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        let (_tx, rx) = mpsc::channel(1);
        rx
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}
