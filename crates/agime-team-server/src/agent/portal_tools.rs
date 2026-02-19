//! Portal Tools — platform extension for Agent to create/manage/publish portals
//!
//! Implements McpClientTrait so it can be loaded by PlatformExtensionRunner.
//! Agent uses Developer extension (text_editor + shell) for file editing.

use agime::agents::mcp_client::McpClientTrait;
use agime_team::db::MongoDb;
use agime_team::models::mongo::{CreatePortalRequest, PortalDetail};
use agime_team::services::mongo::PortalService;
use anyhow::Result;
use rmcp::model::*;
use rmcp::ServiceError;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Provider of portal tools for agents
pub struct PortalToolsProvider {
    db: Arc<MongoDb>,
    team_id: String,
    /// Server base URL for generating public portal URLs (e.g. "http://192.168.1.100:8080")
    base_url: String,
    /// Workspace root for creating project folders
    workspace_root: String,
}

impl PortalToolsProvider {
    pub fn new(
        db: Arc<MongoDb>,
        team_id: String,
        base_url: String,
        workspace_root: String,
    ) -> Self {
        Self {
            db,
            team_id,
            base_url,
            workspace_root,
        }
    }

    fn service(&self) -> PortalService {
        PortalService::new((*self.db).clone())
    }

    fn tool_definitions() -> Vec<Tool> {
        vec![
            Tool {
                name: "create_portal".into(),
                title: None,
                description: Some("Create a new Portal with a project folder. Returns the portal ID, public URL, and project_path. Use Developer extension (text_editor, shell) to build the project in the project_path directory.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Portal name" },
                        "description": { "type": "string", "description": "Portal description" },
                        "slug": { "type": "string", "description": "URL slug (auto-generated if omitted)" },
                        "agent_enabled": { "type": "boolean", "description": "Enable embedded agent chat" },
                        "coding_agent_id": { "type": "string", "description": "Agent ID used for Portal laboratory coding sessions" },
                        "service_agent_id": { "type": "string", "description": "Agent ID used for public visitor chat sessions" },
                        "agent_id": { "type": "string", "description": "Legacy single-agent field. If provided, both coding/service agent may fallback to this value." },
                        "agent_system_prompt": { "type": "string", "description": "System prompt for the embedded agent" },
                        "agent_welcome_message": { "type": "string", "description": "Welcome message shown in chat widget" },
                        "bound_document_ids": {
                            "type": "array", "items": { "type": "string" },
                            "description": "Document IDs to bind as agent context"
                        },
                        "allowed_extensions": {
                            "type": "array", "items": { "type": "string" },
                            "description": "Optional runtime extension allowlist for visitor sessions"
                        },
                        "allowed_skill_ids": {
                            "type": "array", "items": { "type": "string" },
                            "description": "Optional skill id allowlist for visitor sessions"
                        }
                    },
                    "required": ["name"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "publish_portal".into(),
                title: None,
                description: Some("Publish or unpublish a portal. Published portals are accessible at /p/{slug}.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "portal_id": { "type": "string", "description": "Portal ID" },
                        "publish": { "type": "boolean", "description": "true to publish, false to unpublish" }
                    },
                    "required": ["portal_id", "publish"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "list_portals".into(),
                title: None,
                description: Some("List all portals for the team.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "page": { "type": "integer", "description": "Page number" },
                        "limit": { "type": "integer", "description": "Items per page" }
                    }
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                icons: None,
                meta: None,
            },
        ]
    }
}

#[async_trait::async_trait]
impl McpClientTrait for PortalToolsProvider {
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
            "create_portal" => self.handle_create_portal(&args).await,
            "publish_portal" => self.handle_publish_portal(&args).await,
            "list_portals" => self.handle_list_portals(&args).await,
            _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
        };

        match result {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
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
        None
    }
}

// ── Tool handler implementations ──

impl PortalToolsProvider {
    /// Verify portal belongs to this team
    async fn verify_portal_ownership(&self, portal_id: &str) -> Result<()> {
        let svc = self.service();
        svc.get(&self.team_id, portal_id).await?;
        Ok(())
    }

    fn parse_optional_string_list(args: &JsonObject, key: &str) -> Option<Vec<String>> {
        args.get(key).and_then(|v| {
            v.as_array().map(|arr| {
                let mut out = Vec::<String>::new();
                for item in arr {
                    if let Some(s) = item.as_str() {
                        let trimmed = s.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if !out.iter().any(|v| v == trimmed) {
                            out.push(trimmed.to_string());
                        }
                    }
                }
                out
            })
        })
    }

    async fn handle_create_portal(&self, args: &JsonObject) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        let req = CreatePortalRequest {
            name: name.to_string(),
            slug: args
                .get("slug")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            description: args
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            output_form: None,
            agent_enabled: args.get("agent_enabled").and_then(|v| v.as_bool()),
            coding_agent_id: args
                .get("coding_agent_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            service_agent_id: args
                .get("service_agent_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            agent_id: args
                .get("agent_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            agent_system_prompt: args
                .get("agent_system_prompt")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            agent_welcome_message: args
                .get("agent_welcome_message")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            bound_document_ids: Self::parse_optional_string_list(args, "bound_document_ids"),
            allowed_extensions: Self::parse_optional_string_list(args, "allowed_extensions"),
            allowed_skill_ids: Self::parse_optional_string_list(args, "allowed_skill_ids"),
            tags: None,
            settings: None,
        };

        let svc = self.service();
        let portal = svc.create(&self.team_id, "agent", req).await?;
        let detail = PortalDetail::from(portal);

        let project_path = svc
            .initialize_project_folder(
                &self.team_id,
                &detail.id,
                &detail.slug,
                &detail.name,
                &self.workspace_root,
            )
            .await?;

        Ok(serde_json::to_string_pretty(&json!({
            "id": detail.id,
            "slug": detail.slug,
            "name": detail.name,
            "status": detail.status,
            "projectPath": project_path,
            "codingAgentId": detail.coding_agent_id,
            "serviceAgentId": detail.service_agent_id,
            "allowedExtensions": detail.allowed_extensions,
            "allowedSkillIds": detail.allowed_skill_ids,
            "publicUrl": format!("{}/p/{}", self.base_url, detail.slug),
            "message": format!("Portal created. Project folder: {}. Use text_editor and shell tools to build your project. API client scaffold: portal-agent-client.js", project_path),
        }))?)
    }

    async fn handle_publish_portal(&self, args: &JsonObject) -> Result<String> {
        let portal_id = args
            .get("portal_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("portal_id is required"))?;
        self.verify_portal_ownership(portal_id).await?;
        let publish = args
            .get("publish")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let svc = self.service();
        let portal = if publish {
            svc.publish(&self.team_id, portal_id).await?
        } else {
            svc.unpublish(&self.team_id, portal_id).await?
        };

        let detail = PortalDetail::from(portal);
        Ok(serde_json::to_string_pretty(&json!({
            "id": detail.id,
            "slug": detail.slug,
            "status": detail.status,
            "publicUrl": if publish { format!("{}/p/{}", self.base_url, detail.slug) } else { String::new() },
            "message": if publish { "Portal published" } else { "Portal unpublished" },
        }))?)
    }

    async fn handle_list_portals(&self, args: &JsonObject) -> Result<String> {
        let page = args.get("page").and_then(|v| v.as_u64()).unwrap_or(1);
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);

        let svc = self.service();
        let result = svc.list(&self.team_id, page, limit).await?;

        Ok(serde_json::to_string_pretty(&json!({
            "items": result.items.iter().map(|p| json!({
                "id": p.id,
                "slug": p.slug,
                "name": p.name,
                "status": p.status,
                "agentEnabled": p.agent_enabled,
                "projectPath": p.project_path,
                "allowedExtensions": p.allowed_extensions,
                "allowedSkillIds": p.allowed_skill_ids,
                "publicUrl": format!("{}/p/{}", self.base_url, p.slug),
            })).collect::<Vec<_>>(),
            "total": result.total,
            "page": result.page,
        }))?)
    }
}
