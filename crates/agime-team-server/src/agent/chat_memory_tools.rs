use std::sync::Arc;

use agime::agents::mcp_client::McpClientTrait;
use agime_team::db::MongoDb;
use anyhow::{anyhow, Result};
use rmcp::model::*;
use rmcp::ServiceError;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::chat_memory::{
    render_memory_update_notice, sanitize_memory_patch, ChatMemoryService,
    UpdateUserChatMemoryRequest,
};
use super::service_mongo::AgentService;

pub struct ChatMemoryToolsProvider {
    db: Arc<MongoDb>,
    team_id: String,
    user_id: String,
    session_id: Option<String>,
    info: InitializeResult,
}

impl ChatMemoryToolsProvider {
    pub fn new(
        db: Arc<MongoDb>,
        team_id: String,
        user_id: String,
        session_id: Option<String>,
    ) -> Self {
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
                name: "chat_memory".to_string(),
                title: Some("Chat Memory".to_string()),
                version: "1.0.0".to_string(),
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Use get_memory to inspect current team-scoped user relationship memory. Use save_memory only when the user explicitly provides stable relationship information or clearly wants it remembered. Use propose_memory_update when you think some stable relationship information would be valuable to remember but the user has not explicitly asked to save it yet. If key relationship information is missing, ask naturally first and do not guess."
                    .to_string(),
            ),
        };
        Self {
            db,
            team_id,
            user_id,
            session_id,
            info,
        }
    }

    fn tool_definitions() -> Vec<Tool> {
        vec![
            Tool {
                name: "get_memory".into(),
                title: None,
                description: Some(
                    "Read current team-scoped relationship memory for the active user. Use this before relying on name, role, current focus, or collaboration preferences.".into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {}
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "save_memory".into(),
                title: None,
                description: Some(
                    "Save stable relationship memory for the active user in the current team. Use only when the user explicitly gives durable information or asks you to remember it. Do not guess.".into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "preferred_address": { "type": ["string", "null"] },
                        "role_hint": { "type": ["string", "null"] },
                        "current_focus": { "type": ["string", "null"] },
                        "collaboration_preference": { "type": ["string", "null"] },
                        "notes": { "type": ["string", "null"] }
                    }
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "propose_memory_update".into(),
                title: None,
                description: Some(
                    "Create a pending memory suggestion for the active user in the current team. Use when some stable relationship info may be worth remembering, but the user has not explicitly asked to save it yet. Do not guess.".into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "preferred_address": { "type": ["string", "null"] },
                        "role_hint": { "type": ["string", "null"] },
                        "current_focus": { "type": ["string", "null"] },
                        "collaboration_preference": { "type": ["string", "null"] },
                        "notes": { "type": ["string", "null"] },
                        "source_message": { "type": "string" },
                        "reason": { "type": "string" }
                    },
                    "required": ["reason"]
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

    async fn handle_get_memory(&self) -> Result<String> {
        let service = ChatMemoryService::new(self.db.clone());
        let payload = service.get_memory(&self.team_id, &self.user_id).await?;
        Ok(
            serde_json::to_string_pretty(&json!({
                "memory": payload.map(|doc| super::chat_memory::UserChatMemoryResponse::from(doc))
            }))?,
        )
    }

    async fn handle_save_memory(&self, arguments: &JsonObject) -> Result<String> {
        fn optional_string_arg(
            arguments: &JsonObject,
            key: &str,
        ) -> Option<Option<String>> {
            match arguments.get(key) {
                Some(value) if value.is_null() => Some(None),
                Some(value) => value.as_str().map(|text| Some(text.to_string())),
                None => None,
            }
        }

        let payload = UpdateUserChatMemoryRequest {
            preferred_address: optional_string_arg(arguments, "preferred_address"),
            role_hint: optional_string_arg(arguments, "role_hint"),
            current_focus: optional_string_arg(arguments, "current_focus"),
            collaboration_preference: optional_string_arg(
                arguments,
                "collaboration_preference",
            ),
            notes: optional_string_arg(arguments, "notes"),
            session_id: self.session_id.clone(),
        };
        let (patch, _) = sanitize_memory_patch(payload);
        if patch.preferred_address.is_none()
            && patch.role_hint.is_none()
            && patch.current_focus.is_none()
            && patch.collaboration_preference.is_none()
            && patch.notes.is_none()
        {
            return Err(anyhow!("No memory fields provided"));
        }
        let service = ChatMemoryService::new(self.db.clone());
        let memory = service
            .upsert_memory(&self.team_id, &self.user_id, patch, &self.user_id)
            .await?;

        if let Some(session_id) = self.session_id.clone() {
            let _ = AgentService::new(self.db.clone())
                .append_hidden_session_notice(&session_id, &render_memory_update_notice(&memory))
                .await;
        }

        Ok(
            serde_json::to_string_pretty(&json!({
                "memory": super::chat_memory::UserChatMemoryResponse::from(memory)
            }))?,
        )
    }

    async fn handle_propose_memory_update(&self, arguments: &JsonObject) -> Result<String> {
        fn optional_string_arg(
            arguments: &JsonObject,
            key: &str,
        ) -> Option<Option<String>> {
            match arguments.get(key) {
                Some(value) if value.is_null() => Some(None),
                Some(value) => value.as_str().map(|text| Some(text.to_string())),
                None => None,
            }
        }

        let reason = arguments
            .get("reason")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("reason is required"))?;
        let source_message = arguments
            .get("source_message")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(reason);
        let payload = UpdateUserChatMemoryRequest {
            preferred_address: optional_string_arg(arguments, "preferred_address"),
            role_hint: optional_string_arg(arguments, "role_hint"),
            current_focus: optional_string_arg(arguments, "current_focus"),
            collaboration_preference: optional_string_arg(
                arguments,
                "collaboration_preference",
            ),
            notes: optional_string_arg(arguments, "notes"),
            session_id: self.session_id.clone(),
        };
        let (patch, _) = sanitize_memory_patch(payload);
        if patch.preferred_address.is_none()
            && patch.role_hint.is_none()
            && patch.current_focus.is_none()
            && patch.collaboration_preference.is_none()
            && patch.notes.is_none()
        {
            return Err(anyhow!("No memory fields provided"));
        }

        let existing = ChatMemoryService::new(self.db.clone())
            .get_memory(&self.team_id, &self.user_id)
            .await?;
        let patch_fields = super::chat_memory::sanitize_memory_fields(
            super::chat_memory::UserChatMemoryFields {
                preferred_address: patch.preferred_address.flatten(),
                role_hint: patch.role_hint.flatten(),
                current_focus: patch.current_focus.flatten(),
                collaboration_preference: patch.collaboration_preference.flatten(),
                notes: patch.notes.flatten(),
            },
        );
        let suggestion = ChatMemoryService::new(self.db.clone())
            .create_suggestion(
                &self.team_id,
                &self.user_id,
                self.session_id.as_deref().unwrap_or(""),
                source_message,
                patch_fields,
                reason,
            )
            .await?;

        Ok(
            serde_json::to_string_pretty(&json!({
                "suggestion": super::chat_memory::UserChatMemorySuggestionResponse::from(suggestion),
                "memory": existing.map(super::chat_memory::UserChatMemoryResponse::from)
            }))?,
        )
    }
}

#[async_trait::async_trait]
impl McpClientTrait for ChatMemoryToolsProvider {
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
            "get_memory" => self.handle_get_memory().await,
            "save_memory" => self.handle_save_memory(&args).await,
            "propose_memory_update" => self.handle_propose_memory_update(&args).await,
            _ => Err(anyhow!("Unknown tool: {}", name)),
        };

        match result {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
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
