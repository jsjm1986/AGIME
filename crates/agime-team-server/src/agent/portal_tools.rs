//! Portal Tools — platform extension for Agent to create/manage/publish portals
//!
//! Implements McpClientTrait so it can be loaded by PlatformExtensionRunner.
//! Agent uses Developer extension (text_editor + shell) for file editing.

use agime::agents::mcp_client::McpClientTrait;
use agime_team::db::MongoDb;
use agime_team::models::mongo::{
    CreatePortalRequest, PortalDetail, PortalDocumentAccessMode, UpdatePortalRequest,
};
use agime_team::models::{BuiltinExtension, TeamAgent, UpdateAgentRequest};
use agime_team::services::mongo::document_service_mongo::DocumentService;
use agime_team::services::mongo::extension_service_mongo::ExtensionService;
use agime_team::services::mongo::skill_service_mongo::SkillService;
use agime_team::services::mongo::PortalService;
use anyhow::Result;
use rmcp::model::*;
use rmcp::ServiceError;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::service_mongo::AgentService;

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

    fn tool_definitions(&self) -> Vec<Tool> {
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
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "publish_portal".into(),
                title: None,
                description: Some(format!("Publish or unpublish a portal. Published portals are accessible at {}/p/{{slug}}. Always show the full publicUrl to the user.", self.base_url).into()),
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
                execution: None,
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
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "get_portal_service_capability_profile".into(),
                title: None,
                description: Some("Get current service-agent capability profile for a portal, including effective extensions/skills/doc permissions and editable configuration surface.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "portal_id": { "type": "string", "description": "Portal ID" }
                    },
                    "required": ["portal_id"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "configure_portal_service_agent".into(),
                title: None,
                description: Some("Configure portal runtime policy and service-agent capability in one place. Supports setting service agent, doc access mode, allowlists, welcome message, service-agent system prompt, and adding team skills/extensions to service agent.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "portal_id": { "type": "string", "description": "Portal ID" },
                        "service_agent_id": { "type": "string", "description": "Optional: set service agent id for this portal" },
                        "clear_service_agent": { "type": "boolean", "description": "Optional: clear service agent binding" },
                        "document_access_mode": {
                            "type": "string",
                            "enum": ["read_only", "co_edit_draft", "controlled_write"],
                            "description": "Portal document access mode for visitor sessions"
                        },
                        "bound_document_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Replace portal bound document IDs with this full list"
                        },
                        "add_bound_document_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Add document IDs into portal bound documents"
                        },
                        "remove_bound_document_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Remove document IDs from portal bound documents"
                        },
                        "clear_bound_document_ids": {
                            "type": "boolean",
                            "description": "Clear all portal bound documents"
                        },
                        "allowed_extensions": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Portal visitor runtime extension allowlist (runtime names). Empty array = no extension allowlist."
                        },
                        "clear_allowed_extensions": {
                            "type": "boolean",
                            "description": "Clear extension allowlist (set to unrestricted)"
                        },
                        "allowed_skill_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Portal visitor runtime skill allowlist (skill IDs). Empty array = no skill allowlist."
                        },
                        "clear_allowed_skill_ids": {
                            "type": "boolean",
                            "description": "Clear skill allowlist (set to unrestricted)"
                        },
                        "agent_welcome_message": { "type": "string", "description": "Portal visitor chat welcome message" },
                        "clear_agent_welcome_message": { "type": "boolean", "description": "Clear portal welcome message" },
                        "service_agent_system_prompt": { "type": "string", "description": "Update service agent system prompt (agent-level)" },
                        "clear_service_agent_system_prompt": { "type": "boolean", "description": "Clear service agent system prompt (sets empty string)" },
                        "show_chat_widget": { "type": "boolean", "description": "Show default chat widget on portal page" },
                        "add_team_skill_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Team shared skill IDs to assign to service agent"
                        },
                        "add_team_extension_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Team shared extension IDs to add into service agent custom extensions"
                        }
                    },
                    "required": ["portal_id"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
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
            tools: self.tool_definitions(),
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
            "get_portal_service_capability_profile" => {
                self.handle_get_portal_service_capability_profile(&args)
                    .await
            }
            "configure_portal_service_agent" => {
                self.handle_configure_portal_service_agent(&args).await
            }
            _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
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
        None
    }
}

// ── Tool handler implementations ──

impl PortalToolsProvider {
    fn resolve_effective_service_agent_id(portal: &PortalDetail) -> Option<String> {
        portal
            .service_agent_id
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                portal
                    .agent_id
                    .as_ref()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| {
                portal
                    .coding_agent_id
                    .as_ref()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
    }

    fn normalize_list(items: Option<Vec<String>>) -> Vec<String> {
        let mut out = Vec::new();
        for item in items.unwrap_or_default() {
            let v = item.trim();
            if v.is_empty() {
                continue;
            }
            if !out.iter().any(|x| x == v) {
                out.push(v.to_string());
            }
        }
        out
    }

    fn parse_document_access_mode(raw: Option<&str>) -> Result<Option<PortalDocumentAccessMode>> {
        let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let mode = match raw.to_ascii_lowercase().as_str() {
            "read_only" | "readonly" | "read-only" => PortalDocumentAccessMode::ReadOnly,
            "co_edit_draft" | "co-edit-draft" | "coeditdraft" => {
                PortalDocumentAccessMode::CoEditDraft
            }
            "controlled_write" | "controlled-write" | "controlledwrite" => {
                PortalDocumentAccessMode::ControlledWrite
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "Invalid document_access_mode '{}'. Use read_only | co_edit_draft | controlled_write",
                    raw
                ))
            }
        };
        Ok(Some(mode))
    }

    fn collect_service_agent_extension_capabilities(agent: &TeamAgent) -> Vec<String> {
        let mut set: HashSet<String> = HashSet::new();
        for cfg in &agent.enabled_extensions {
            if !cfg.enabled {
                continue;
            }
            set.insert(cfg.extension.name().to_lowercase());
            if let Some(mcp) = cfg.extension.mcp_name() {
                set.insert(mcp.to_lowercase());
            }
            if cfg.extension == BuiltinExtension::Skills {
                set.insert("team_skills".to_string());
            }
        }
        for cfg in &agent.custom_extensions {
            if !cfg.enabled {
                continue;
            }
            let name = cfg.name.trim().to_lowercase();
            if !name.is_empty() {
                set.insert(name);
            }
        }
        // Runtime fallback extensions in team-server
        set.insert("document_tools".to_string());
        set.insert("portal_tools".to_string());

        let mut out: Vec<String> = set.into_iter().collect();
        out.sort();
        out
    }

    fn collect_service_agent_skill_capabilities(agent: &TeamAgent) -> Vec<String> {
        let mut out = Vec::new();
        for skill in &agent.assigned_skills {
            if !skill.enabled {
                continue;
            }
            let id = skill.skill_id.trim();
            if id.is_empty() {
                continue;
            }
            if !out.iter().any(|s| s == id) {
                out.push(id.to_string());
            }
        }
        out.sort();
        out
    }

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

    fn get_bool_arg(args: &JsonObject, keys: &[&str]) -> Option<bool> {
        for key in keys {
            if let Some(v) = args.get(*key).and_then(|v| v.as_bool()) {
                return Some(v);
            }
        }
        None
    }

    fn get_str_arg<'a>(args: &'a JsonObject, keys: &[&str]) -> Option<&'a str> {
        for key in keys {
            if let Some(v) = args.get(*key).and_then(|v| v.as_str()) {
                return Some(v);
            }
        }
        None
    }

    fn parse_optional_string_list_any(args: &JsonObject, keys: &[&str]) -> Option<Vec<String>> {
        for key in keys {
            if args.contains_key(*key) {
                return Self::parse_optional_string_list(args, key);
            }
        }
        None
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
            document_access_mode: None,
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
            "message": format!("Portal created. Public URL: {}/p/{}. Project folder: {}. Use text_editor and shell tools to build your project. API client scaffold: portal-agent-client.js", self.base_url, detail.slug, project_path),
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
            "message": if publish { format!("Portal published. Visit: {}/p/{}", self.base_url, detail.slug) } else { "Portal unpublished".to_string() },
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

    async fn handle_get_portal_service_capability_profile(
        &self,
        args: &JsonObject,
    ) -> Result<String> {
        let portal_id = args
            .get("portal_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("portal_id is required"))?;
        self.verify_portal_ownership(portal_id).await?;

        let portal_svc = self.service();
        let portal = PortalDetail::from(portal_svc.get(&self.team_id, portal_id).await?);

        let agent_svc = AgentService::new(self.db.clone());
        let service_agent_id = Self::resolve_effective_service_agent_id(&portal);
        let service_agent = match service_agent_id.as_deref() {
            Some(agent_id) => agent_svc.get_agent(agent_id).await?,
            None => None,
        };

        let service_extensions = service_agent
            .as_ref()
            .map(Self::collect_service_agent_extension_capabilities)
            .unwrap_or_default();
        let service_skills = service_agent
            .as_ref()
            .map(Self::collect_service_agent_skill_capabilities)
            .unwrap_or_default();

        let allow_ext = Self::normalize_list(portal.allowed_extensions.clone());
        let allow_skill = Self::normalize_list(portal.allowed_skill_ids.clone());

        let effective_ext = if allow_ext.is_empty() {
            service_extensions.clone()
        } else {
            service_extensions
                .iter()
                .filter(|name| allow_ext.iter().any(|x| x.eq_ignore_ascii_case(name)))
                .cloned()
                .collect::<Vec<_>>()
        };
        let effective_skill = if allow_skill.is_empty() {
            service_skills.clone()
        } else {
            service_skills
                .iter()
                .filter(|id| allow_skill.iter().any(|x| x == *id))
                .cloned()
                .collect::<Vec<_>>()
        };

        let skill_service = SkillService::new((*self.db).clone());
        let ext_service = ExtensionService::new((*self.db).clone());
        let team_skills = skill_service
            .list(&self.team_id, Some(1), Some(200), None, None)
            .await?;
        let team_extensions = ext_service
            .list(&self.team_id, Some(1), Some(200), None, None)
            .await?;

        let show_chat_widget = portal
            .settings
            .get("showChatWidget")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let document_service = DocumentService::new((*self.db).clone());
        let team_documents = document_service
            .list(&self.team_id, None)
            .await
            .unwrap_or_default();
        let mut bound_document_details: Vec<serde_json::Value> = Vec::new();
        for doc_id in &portal.bound_document_ids {
            match document_service.get_metadata(&self.team_id, doc_id).await {
                Ok(doc) => bound_document_details.push(json!({
                    "id": doc.id,
                    "name": doc.name,
                    "displayName": doc.display_name,
                    "mimeType": doc.mime_type,
                    "fileSize": doc.file_size,
                    "folderPath": doc.folder_path,
                })),
                Err(_) => bound_document_details.push(json!({
                    "id": doc_id,
                    "missing": true
                })),
            }
        }

        Ok(serde_json::to_string_pretty(&json!({
            "portal": {
                "id": portal.id,
                "slug": portal.slug,
                "name": portal.name,
                "agentEnabled": portal.agent_enabled,
                "serviceAgentId": service_agent_id,
                "codingAgentId": portal.coding_agent_id,
                "documentAccessMode": portal.document_access_mode,
                "boundDocumentIds": portal.bound_document_ids,
                "boundDocumentDetails": bound_document_details,
                "agentWelcomeMessage": portal.agent_welcome_message,
                "showChatWidget": show_chat_widget,
            },
            "serviceAgent": service_agent.as_ref().map(|a| json!({
                "id": a.id,
                "name": a.name,
                "description": a.description,
                "model": a.model,
                "apiFormat": a.api_format,
                "systemPromptConfigured": a.system_prompt.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false),
                "enabledBuiltinExtensions": a.enabled_extensions.iter().filter(|e| e.enabled).map(|e| e.extension.name()).collect::<Vec<_>>(),
                "enabledCustomExtensions": a.custom_extensions.iter().filter(|e| e.enabled).map(|e| e.name.clone()).collect::<Vec<_>>(),
                "enabledSkillIds": a.assigned_skills.iter().filter(|s| s.enabled).map(|s| s.skill_id.clone()).collect::<Vec<_>>(),
                "enabledSkillNames": a.assigned_skills.iter().filter(|s| s.enabled).map(|s| s.name.clone()).collect::<Vec<_>>(),
            })),
            "capabilityPolicy": {
                "allowlistExtensions": allow_ext,
                "allowlistSkillIds": allow_skill,
                "serviceAgentExtensions": service_extensions,
                "serviceAgentSkillIds": service_skills,
                "effectiveExtensions": effective_ext,
                "effectiveSkillIds": effective_skill,
            },
            "catalog": {
                "teamSkills": team_skills.items.iter().map(|s| json!({
                    "id": s.id,
                    "name": s.name,
                    "description": s.description,
                    "version": s.version,
                })).collect::<Vec<_>>(),
                "teamExtensions": team_extensions.items.iter().map(|e| json!({
                    "id": e.id,
                    "name": e.name,
                    "description": e.description,
                    "type": e.extension_type,
                    "securityReviewed": e.security_reviewed,
                    "version": e.version,
                })).collect::<Vec<_>>(),
                "teamDocuments": team_documents.iter().map(|d| json!({
                    "id": d.id,
                    "name": d.name,
                    "displayName": d.display_name,
                    "mimeType": d.mime_type,
                    "fileSize": d.file_size,
                    "folderPath": d.folder_path,
                })).collect::<Vec<_>>(),
            },
            "usage": {
                "nextStep": "Call configure_portal_service_agent to update service capability/policy (and bound documents) before redesigning UI."
            }
        }))?)
    }

    async fn handle_configure_portal_service_agent(&self, args: &JsonObject) -> Result<String> {
        let portal_id = args
            .get("portal_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("portal_id is required"))?;
        self.verify_portal_ownership(portal_id).await?;

        let portal_svc = self.service();
        let current = PortalDetail::from(portal_svc.get(&self.team_id, portal_id).await?);
        let agent_svc = AgentService::new(self.db.clone());

        let mut service_agent_override =
            Self::get_str_arg(args, &["service_agent_id", "serviceAgentId"])
                .map(|s| s.trim().to_string());
        if Self::get_bool_arg(args, &["clear_service_agent", "clearServiceAgent"]).unwrap_or(false)
        {
            service_agent_override = Some(String::new());
        }

        let effective_service_agent_id = match service_agent_override.as_ref() {
            Some(v) if v.is_empty() => None,
            Some(v) => Some(v.clone()),
            None => Self::resolve_effective_service_agent_id(&current),
        };

        let add_skill_ids =
            Self::parse_optional_string_list_any(args, &["add_team_skill_ids", "addTeamSkillIds"]);
        let add_extension_ids = Self::parse_optional_string_list_any(
            args,
            &["add_team_extension_ids", "addTeamExtensionIds"],
        );
        let mut add_skill_results: Vec<serde_json::Value> = Vec::new();
        let mut add_extension_results: Vec<serde_json::Value> = Vec::new();

        let has_skills_to_add = add_skill_ids.as_ref().is_some_and(|v| !v.is_empty());
        let has_extensions_to_add = add_extension_ids.as_ref().is_some_and(|v| !v.is_empty());
        if has_skills_to_add || has_extensions_to_add {
            let target_agent = effective_service_agent_id.as_deref().ok_or_else(|| {
                anyhow::anyhow!("No effective service agent. Set service_agent_id first.")
            })?;

            if let Some(ids) = add_skill_ids {
                for skill_id in ids {
                    match agent_svc
                        .add_team_skill_to_agent(target_agent, &skill_id, &self.team_id)
                        .await
                    {
                        Ok(_) => add_skill_results.push(json!({
                            "skill_id": skill_id,
                            "ok": true
                        })),
                        Err(e) => add_skill_results.push(json!({
                            "skill_id": skill_id,
                            "ok": false,
                            "error": e.to_string()
                        })),
                    }
                }
            }

            if let Some(ids) = add_extension_ids {
                for ext_id in ids {
                    match agent_svc
                        .add_team_extension_to_agent(target_agent, &ext_id, &self.team_id)
                        .await
                    {
                        Ok(_) => add_extension_results.push(json!({
                            "extension_id": ext_id,
                            "ok": true
                        })),
                        Err(e) => add_extension_results.push(json!({
                            "extension_id": ext_id,
                            "ok": false,
                            "error": e.to_string()
                        })),
                    }
                }
            }
        }

        if args.contains_key("service_agent_system_prompt")
            || args.contains_key("serviceAgentSystemPrompt")
            || Self::get_bool_arg(
                args,
                &[
                    "clear_service_agent_system_prompt",
                    "clearServiceAgentSystemPrompt",
                ],
            )
            .unwrap_or(false)
        {
            let target_agent = effective_service_agent_id.as_deref().ok_or_else(|| {
                anyhow::anyhow!("No effective service agent. Set service_agent_id first.")
            })?;
            let prompt_value = if Self::get_bool_arg(
                args,
                &[
                    "clear_service_agent_system_prompt",
                    "clearServiceAgentSystemPrompt",
                ],
            )
            .unwrap_or(false)
            {
                String::new()
            } else {
                Self::get_str_arg(
                    args,
                    &["service_agent_system_prompt", "serviceAgentSystemPrompt"],
                )
                .map(|s| s.to_string())
                .unwrap_or_default()
            };
            let _ = agent_svc
                .update_agent(
                    target_agent,
                    UpdateAgentRequest {
                        name: None,
                        description: None,
                        avatar: None,
                        system_prompt: Some(prompt_value),
                        api_url: None,
                        model: None,
                        api_key: None,
                        api_format: None,
                        status: None,
                        enabled_extensions: None,
                        custom_extensions: None,
                        allowed_groups: None,
                        max_concurrent_tasks: None,
                        temperature: None,
                        max_tokens: None,
                        context_limit: None,
                        assigned_skills: None,
                        auto_approve_chat: None,
                    },
                )
                .await?;
        }

        let service_agent_id_update = service_agent_override.map(|agent_id| {
            let trimmed = agent_id.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        let document_access_mode = Self::parse_document_access_mode(Self::get_str_arg(
            args,
            &["document_access_mode", "documentAccessMode"],
        ))?;
        let allowed_extensions = if Self::get_bool_arg(
            args,
            &["clear_allowed_extensions", "clearAllowedExtensions"],
        )
        .unwrap_or(false)
        {
            Some(Vec::new())
        } else {
            Self::parse_optional_string_list_any(args, &["allowed_extensions", "allowedExtensions"])
        };
        let allowed_skill_ids =
            if Self::get_bool_arg(args, &["clear_allowed_skill_ids", "clearAllowedSkillIds"])
                .unwrap_or(false)
            {
                Some(Vec::new())
            } else {
                Self::parse_optional_string_list_any(
                    args,
                    &["allowed_skill_ids", "allowedSkillIds"],
                )
            };
        let clear_bound_document_ids =
            Self::get_bool_arg(args, &["clear_bound_document_ids", "clearBoundDocumentIds"])
                .unwrap_or(false);
        let replace_bound_document_ids =
            Self::parse_optional_string_list_any(args, &["bound_document_ids", "boundDocumentIds"]);
        let add_bound_document_ids = Self::parse_optional_string_list_any(
            args,
            &["add_bound_document_ids", "addBoundDocumentIds"],
        );
        let remove_bound_document_ids = Self::parse_optional_string_list_any(
            args,
            &["remove_bound_document_ids", "removeBoundDocumentIds"],
        );
        let should_update_bound_document_ids = clear_bound_document_ids
            || replace_bound_document_ids.is_some()
            || add_bound_document_ids.is_some()
            || remove_bound_document_ids.is_some();

        let bound_document_ids = if should_update_bound_document_ids {
            let mut next = if clear_bound_document_ids {
                Vec::new()
            } else if let Some(list) = replace_bound_document_ids.clone() {
                list
            } else {
                current.bound_document_ids.clone()
            };

            if let Some(adds) = add_bound_document_ids.clone() {
                for doc_id in adds {
                    if !next.iter().any(|v| v == &doc_id) {
                        next.push(doc_id);
                    }
                }
            }

            if let Some(removes) = remove_bound_document_ids.clone() {
                next.retain(|id| !removes.iter().any(|rm| rm == id));
            }

            let document_service = DocumentService::new((*self.db).clone());
            let mut invalid_ids: Vec<String> = Vec::new();
            for doc_id in &next {
                if document_service
                    .get_metadata(&self.team_id, doc_id)
                    .await
                    .is_err()
                {
                    invalid_ids.push(doc_id.clone());
                }
            }
            if !invalid_ids.is_empty() {
                return Err(anyhow::anyhow!(
                    "Invalid bound_document_ids: {}. Use real document IDs from get_portal_service_capability_profile.catalog.teamDocuments",
                    invalid_ids.join(", ")
                ));
            }
            Some(next)
        } else {
            None
        };

        let agent_welcome_message = if Self::get_bool_arg(
            args,
            &["clear_agent_welcome_message", "clearAgentWelcomeMessage"],
        )
        .unwrap_or(false)
        {
            Some(None)
        } else if args.contains_key("agent_welcome_message")
            || args.contains_key("agentWelcomeMessage")
        {
            Some(
                Self::get_str_arg(args, &["agent_welcome_message", "agentWelcomeMessage"])
                    .map(|s| s.to_string()),
            )
        } else {
            None
        };

        let settings =
            if args.contains_key("show_chat_widget") || args.contains_key("showChatWidget") {
                let mut settings = match &current.settings {
                    serde_json::Value::Object(obj) => obj.clone(),
                    _ => serde_json::Map::new(),
                };
                if let Some(v) = Self::get_bool_arg(args, &["show_chat_widget", "showChatWidget"]) {
                    settings.insert("showChatWidget".to_string(), serde_json::Value::Bool(v));
                }
                Some(serde_json::Value::Object(settings))
            } else {
                None
            };

        let updated = portal_svc
            .update(
                &self.team_id,
                portal_id,
                UpdatePortalRequest {
                    name: None,
                    slug: None,
                    description: None,
                    output_form: None,
                    agent_enabled: None,
                    coding_agent_id: None,
                    service_agent_id: service_agent_id_update,
                    agent_id: None,
                    agent_system_prompt: None,
                    agent_welcome_message,
                    bound_document_ids,
                    allowed_extensions,
                    allowed_skill_ids,
                    document_access_mode,
                    tags: None,
                    settings,
                },
            )
            .await?;
        let updated = PortalDetail::from(updated);

        let mut profile_args = JsonObject::new();
        profile_args.insert(
            "portal_id".to_string(),
            serde_json::Value::String(portal_id.to_string()),
        );
        let refreshed_profile = serde_json::from_str::<serde_json::Value>(
            &self
                .handle_get_portal_service_capability_profile(&profile_args)
                .await?,
        )
        .unwrap_or_else(|_| json!({}));

        Ok(serde_json::to_string_pretty(&json!({
            "ok": true,
            "portalId": portal_id,
            "updated": {
                "serviceAgentId": updated.service_agent_id,
                "documentAccessMode": updated.document_access_mode,
                "boundDocumentIds": updated.bound_document_ids,
                "allowedExtensions": updated.allowed_extensions,
                "allowedSkillIds": updated.allowed_skill_ids,
                "agentWelcomeMessage": updated.agent_welcome_message,
                "settings": updated.settings,
            },
            "agentMutationResults": {
                "skills": add_skill_results,
                "extensions": add_extension_results,
            },
            "profile": refreshed_profile,
        }))?)
    }
}
