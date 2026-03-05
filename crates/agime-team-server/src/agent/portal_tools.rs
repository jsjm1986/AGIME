//! Portal Tools — platform extension for Agent to create/manage/publish portals
//!
//! Implements McpClientTrait so it can be loaded by PlatformExtensionRunner.
//! Agent uses Developer extension (text_editor + shell) for file editing.

use agime::agents::mcp_client::McpClientTrait;
use agime_team::db::MongoDb;
use agime_team::models::mongo::{
    CreatePortalRequest, PortalDetail, PortalDocumentAccessMode, PortalDomain, UpdatePortalRequest,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PortalDomainKind {
    Ecosystem,
    Avatar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PortalManagementScope {
    Unscoped,
    AvatarOnly,
}

/// Provider of portal tools for agents
pub struct PortalToolsProvider {
    db: Arc<MongoDb>,
    team_id: String,
    /// Current runtime/session agent id (the manager agent for `portal_manager` scope).
    owner_agent_id: Option<String>,
    /// Authenticated actor user ID that initiated this runtime.
    /// Mutating portal tools require this actor to be team admin/owner.
    actor_user_id: Option<String>,
    /// Server base URL for generating public portal URLs (e.g. "http://192.168.1.100:8080")
    base_url: String,
    /// Workspace root for creating project folders
    workspace_root: String,
    management_scope: PortalManagementScope,
}

impl PortalToolsProvider {
    fn resolve_management_scope(session_source: Option<&str>) -> PortalManagementScope {
        let source = session_source
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase());
        match source.as_deref() {
            Some("portal_manager") => PortalManagementScope::AvatarOnly,
            _ => PortalManagementScope::Unscoped,
        }
    }

    pub fn new(
        db: Arc<MongoDb>,
        team_id: String,
        owner_agent_id: Option<String>,
        actor_user_id: Option<String>,
        session_source: Option<String>,
        base_url: String,
        workspace_root: String,
    ) -> Self {
        let management_scope = Self::resolve_management_scope(session_source.as_deref());
        Self {
            db,
            team_id,
            owner_agent_id,
            actor_user_id,
            base_url,
            workspace_root,
            management_scope,
        }
    }

    fn service(&self) -> PortalService {
        PortalService::new((*self.db).clone())
    }

    fn tool_definitions(&self) -> Vec<Tool> {
        let mut tools = vec![
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
                        "output_form": { "type": "string", "enum": ["website", "widget", "agent_only"], "description": "Portal output form" },
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
                        },
                        "document_access_mode": {
                            "type": "string",
                            "enum": ["read_only", "co_edit_draft", "controlled_write"],
                            "description": "Portal document access mode for visitor sessions"
                        },
                        "tags": {
                            "type": "array", "items": { "type": "string" },
                            "description": "Portal tags"
                        },
                        "settings": {
                            "type": "object",
                            "description": "Portal settings JSON object"
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
                name: "create_digital_avatar".into(),
                title: None,
                description: Some("Create a digital-avatar portal in one step (manager/service agents, governance settings, tags, document policy).".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Avatar name" },
                        "slug": { "type": "string", "description": "URL slug (auto-generated if omitted)" },
                        "description": { "type": "string", "description": "Avatar description" },
                        "manager_agent_id": { "type": "string", "description": "Manager/coding agent id" },
                        "service_agent_id": { "type": "string", "description": "Service agent id (fallback to manager_agent_id if omitted)" },
                        "avatar_type": { "type": "string", "enum": ["external_service", "internal_worker"], "description": "Avatar type" },
                        "run_mode": { "type": "string", "enum": ["on_demand", "scheduled", "event_driven"], "description": "Avatar run mode" },
                        "document_access_mode": {
                            "type": "string",
                            "enum": ["read_only", "co_edit_draft", "controlled_write"],
                            "description": "Document access mode"
                        },
                        "manager_approval_mode": { "type": "string", "description": "Governance decision mode, default manager_decides" },
                        "optimization_mode": { "type": "string", "description": "Optimization mode, default dual_loop" },
                        "bound_document_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Bound document IDs"
                        },
                        "allowed_extensions": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional visitor extension allowlist"
                        },
                        "allowed_skill_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional visitor skill allowlist"
                        },
                        "agent_system_prompt": { "type": "string", "description": "Service agent system prompt" },
                        "agent_welcome_message": { "type": "string", "description": "Service agent welcome message" },
                        "settings": { "type": "object", "description": "Additional settings patch" },
                        "tags": { "type": "array", "items": { "type": "string" }, "description": "Additional tags" }
                    },
                    "required": ["name", "manager_agent_id"]
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
                description: Some("Configure portal runtime policy in one place. Supports setting coding/service agent, doc access mode, bound documents, allowlists, tags/settings patch, welcome message, and replace/append service-agent system prompt, plus adding team skills/extensions to service agent.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "portal_id": { "type": "string", "description": "Portal ID" },
                        "service_agent_id": { "type": "string", "description": "Optional: set service agent id for this portal" },
                        "clear_service_agent": { "type": "boolean", "description": "Optional: clear service agent binding" },
                        "coding_agent_id": { "type": "string", "description": "Optional: set coding manager agent id for this portal" },
                        "clear_coding_agent": { "type": "boolean", "description": "Optional: clear coding manager agent binding" },
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
                        "service_agent_system_prompt": { "type": "string", "description": "Replace service agent system prompt (agent-level)" },
                        "append_service_agent_system_prompt": { "type": "string", "description": "Append text to existing service agent system prompt (additive overlay, non-destructive)." },
                        "clear_service_agent_system_prompt": { "type": "boolean", "description": "Clear service agent system prompt (sets empty string)" },
                        "show_chat_widget": { "type": "boolean", "description": "Show default chat widget on portal page" },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Replace all tags with this full list"
                        },
                        "add_tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Add tags"
                        },
                        "remove_tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Remove tags"
                        },
                        "clear_tags": { "type": "boolean", "description": "Clear all tags" },
                        "settings_patch": {
                            "type": "object",
                            "description": "Merge patch into portal settings. Supports auto-governance config at settings.digitalAvatarGovernanceConfig.autoProposalTriggerCount (1-10). Recommended baseline: 3 (aggressive), 5 (balanced), 7 (conservative). Example: {\"digitalAvatarGovernanceConfig\":{\"autoProposalTriggerCount\":5}}"
                        },
                        "clear_settings": {
                            "type": "boolean",
                            "description": "Reset settings to empty object before applying patch"
                        },
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
        ];
        if self.management_scope == PortalManagementScope::AvatarOnly {
            tools.retain(|tool| tool.name.as_ref() != "create_portal");
        }
        tools
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
            "create_digital_avatar" => self.handle_create_digital_avatar(&args).await,
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
    fn domain_label(domain: PortalDomainKind) -> &'static str {
        match domain {
            PortalDomainKind::Avatar => "avatar",
            PortalDomainKind::Ecosystem => "ecosystem",
        }
    }

    fn detect_domain_from_tags(tags: &[String]) -> PortalDomainKind {
        let is_avatar = tags.iter().any(|tag| {
            let v = tag.trim().to_ascii_lowercase();
            v == "digital-avatar" || v.starts_with("avatar:") || v == "domain:avatar"
        });
        if is_avatar {
            PortalDomainKind::Avatar
        } else {
            PortalDomainKind::Ecosystem
        }
    }

    fn detect_domain_from_settings(settings: &serde_json::Value) -> Option<PortalDomainKind> {
        let raw = settings
            .get("domain")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase())?;
        match raw.as_str() {
            "avatar" => Some(PortalDomainKind::Avatar),
            "ecosystem" => Some(PortalDomainKind::Ecosystem),
            _ => None,
        }
    }

    fn detect_domain_from_tags_and_settings(
        tags: &[String],
        settings: &serde_json::Value,
    ) -> PortalDomainKind {
        let by_tags = Self::detect_domain_from_tags(tags);
        if by_tags == PortalDomainKind::Avatar {
            return by_tags;
        }
        Self::detect_domain_from_settings(settings).unwrap_or(by_tags)
    }

    fn normalize_domain_tags(tags: &mut Vec<String>, desired: PortalDomainKind) {
        tags.retain(|tag| {
            let lower = tag.trim().to_ascii_lowercase();
            match desired {
                PortalDomainKind::Avatar => lower != "domain:ecosystem",
                PortalDomainKind::Ecosystem => {
                    lower != "digital-avatar"
                        && !lower.starts_with("avatar:")
                        && lower != "domain:avatar"
                }
            }
        });

        match desired {
            PortalDomainKind::Avatar => {
                if !tags
                    .iter()
                    .any(|tag| tag.trim().eq_ignore_ascii_case("digital-avatar"))
                {
                    tags.push("digital-avatar".to_string());
                }
                if !tags
                    .iter()
                    .any(|tag| tag.trim().eq_ignore_ascii_case("domain:avatar"))
                {
                    tags.push("domain:avatar".to_string());
                }
            }
            PortalDomainKind::Ecosystem => {
                if !tags
                    .iter()
                    .any(|tag| tag.trim().eq_ignore_ascii_case("domain:ecosystem"))
                {
                    tags.push("domain:ecosystem".to_string());
                }
            }
        }
    }

    fn detect_domain_from_portal(portal: &PortalDetail) -> PortalDomainKind {
        if let Some(domain) = portal.domain {
            return match domain {
                PortalDomain::Avatar => PortalDomainKind::Avatar,
                PortalDomain::Ecosystem => PortalDomainKind::Ecosystem,
            };
        }
        Self::detect_domain_from_tags_and_settings(&portal.tags, &portal.settings)
    }

    fn scope_label(scope: PortalManagementScope) -> &'static str {
        match scope {
            PortalManagementScope::Unscoped => "unscoped",
            PortalManagementScope::AvatarOnly => "avatar_only",
        }
    }

    fn scope_manager_agent_id(&self) -> Option<&str> {
        self.owner_agent_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
    }

    fn resolve_portal_manager_agent_id(portal: &PortalDetail) -> Option<&str> {
        portal
            .coding_agent_id
            .as_deref()
            .or(portal.agent_id.as_deref())
            .map(str::trim)
            .filter(|id| !id.is_empty())
    }

    fn ensure_scope_manager_binding(&self, manager_agent_id: &str, action: &str) -> Result<()> {
        if self.management_scope != PortalManagementScope::AvatarOnly {
            return Ok(());
        }
        let scope_manager = self.scope_manager_agent_id().ok_or_else(|| {
            anyhow::anyhow!(
                "portal_tools scope violation: action '{}' requires manager scope agent context",
                action
            )
        })?;
        if scope_manager == manager_agent_id.trim() {
            return Ok(());
        }
        Err(anyhow::anyhow!(
            "portal_tools scope violation: action '{}' can only target manager agent '{}' in current session scope",
            action,
            scope_manager
        ))
    }

    fn ensure_scope_allows_domain(&self, domain: PortalDomainKind, action: &str) -> Result<()> {
        let allowed = match self.management_scope {
            PortalManagementScope::Unscoped => true,
            PortalManagementScope::AvatarOnly => domain == PortalDomainKind::Avatar,
        };
        if allowed {
            return Ok(());
        }
        Err(anyhow::anyhow!(
            "portal_tools scope violation: action '{}' is not allowed for domain '{}' in scope '{}'",
            action,
            Self::domain_label(domain),
            Self::scope_label(self.management_scope),
        ))
    }

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

    fn parse_output_form(
        raw: Option<&str>,
    ) -> Result<Option<agime_team::models::mongo::PortalOutputForm>> {
        let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let form = match raw.to_ascii_lowercase().as_str() {
            "website" => agime_team::models::mongo::PortalOutputForm::Website,
            "widget" => agime_team::models::mongo::PortalOutputForm::Widget,
            "agent_only" | "agent-only" | "agentonly" => {
                agime_team::models::mongo::PortalOutputForm::AgentOnly
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "Invalid output_form '{}'. Use website | widget | agent_only",
                    raw
                ))
            }
        };
        Ok(Some(form))
    }

    fn get_json_object_arg(
        args: &JsonObject,
        keys: &[&str],
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        for key in keys {
            if let Some(obj) = args.get(*key).and_then(|v| v.as_object()).cloned() {
                return Some(obj);
            }
        }
        None
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

    /// Verify portal belongs to this team and matches current management scope.
    async fn get_portal_checked(&self, portal_id: &str, action: &str) -> Result<PortalDetail> {
        let svc = self.service();
        let portal = PortalDetail::from(svc.get(&self.team_id, portal_id).await?);
        let domain = Self::detect_domain_from_portal(&portal);
        self.ensure_scope_allows_domain(domain, action)?;
        if self.management_scope == PortalManagementScope::AvatarOnly {
            let portal_manager_id =
                Self::resolve_portal_manager_agent_id(&portal).ok_or_else(|| {
                    anyhow::anyhow!(
                        "portal_tools scope violation: portal '{}' has no manager agent binding",
                        portal_id
                    )
                })?;
            self.ensure_scope_manager_binding(portal_manager_id, action)?;
        }
        Ok(portal)
    }

    async fn require_admin_mutation(&self, tool_name: &str) -> Result<()> {
        let actor_user_id = self.actor_user_id.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "portal_tools mutation denied: missing actor user context for {}",
                tool_name
            )
        })?;
        let agent_service = AgentService::new(self.db.clone());
        let is_admin = agent_service
            .is_team_admin(actor_user_id, &self.team_id)
            .await?;
        if !is_admin {
            return Err(anyhow::anyhow!(
                "portal_tools mutation denied: {} requires team admin/owner role",
                tool_name
            ));
        }
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
        self.require_admin_mutation("create_portal").await?;
        self.ensure_scope_allows_domain(PortalDomainKind::Ecosystem, "create_portal")?;
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        let output_form =
            Self::parse_output_form(Self::get_str_arg(args, &["output_form", "outputForm"]))?;
        let document_access_mode = Self::parse_document_access_mode(Self::get_str_arg(
            args,
            &["document_access_mode", "documentAccessMode"],
        ))?;
        let mut tags = Self::parse_optional_string_list_any(args, &["tags"]).unwrap_or_default();
        Self::normalize_domain_tags(&mut tags, PortalDomainKind::Ecosystem);
        let mut settings = Self::get_json_object_arg(args, &["settings"]).unwrap_or_default();
        settings
            .entry("domain".to_string())
            .or_insert_with(|| serde_json::Value::String("ecosystem".to_string()));

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
            output_form,
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
            document_access_mode,
            tags: Some(tags),
            settings: Some(serde_json::Value::Object(settings)),
        };

        let svc = self.service();
        let portal = svc.create(&self.team_id, "agent", req).await?;

        let project_path = svc
            .initialize_project_folder(&self.team_id, &portal, &self.workspace_root)
            .await?;
        let detail = PortalDetail::from(portal);

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

    async fn handle_create_digital_avatar(&self, args: &JsonObject) -> Result<String> {
        self.require_admin_mutation("create_digital_avatar").await?;
        self.ensure_scope_allows_domain(PortalDomainKind::Avatar, "create_digital_avatar")?;
        let name = Self::get_str_arg(args, &["name"])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;
        let manager_agent_id = Self::get_str_arg(args, &["manager_agent_id", "managerAgentId"])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("manager_agent_id is required"))?;
        self.ensure_scope_manager_binding(manager_agent_id, "create_digital_avatar")?;
        let service_agent_id = Self::get_str_arg(args, &["service_agent_id", "serviceAgentId"])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(manager_agent_id);

        let avatar_type = Self::get_str_arg(args, &["avatar_type", "avatarType"])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("external_service")
            .to_string();
        let avatar_type_tag = if avatar_type.eq_ignore_ascii_case("internal_worker") {
            "avatar:internal"
        } else {
            "avatar:external"
        };
        let run_mode = Self::get_str_arg(args, &["run_mode", "runMode"])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("on_demand")
            .to_string();
        let manager_approval_mode =
            Self::get_str_arg(args, &["manager_approval_mode", "managerApprovalMode"])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("manager_decides")
                .to_string();
        let optimization_mode = Self::get_str_arg(args, &["optimization_mode", "optimizationMode"])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("dual_loop")
            .to_string();

        let mut tags = vec!["digital-avatar".to_string(), avatar_type_tag.to_string()];
        let manager_tag = format!("manager:{}", manager_agent_id);
        tags.push(manager_tag);
        if let Some(extra_tags) = Self::parse_optional_string_list_any(args, &["tags"]) {
            for tag in extra_tags {
                if !tags.iter().any(|v| v == &tag) {
                    tags.push(tag);
                }
            }
        }
        Self::normalize_domain_tags(&mut tags, PortalDomainKind::Avatar);

        let mut settings = Self::get_json_object_arg(args, &["settings"]).unwrap_or_default();
        settings.insert(
            "avatarType".to_string(),
            serde_json::Value::String(avatar_type.clone()),
        );
        settings.insert(
            "runMode".to_string(),
            serde_json::Value::String(run_mode.clone()),
        );
        settings.insert(
            "managerApprovalMode".to_string(),
            serde_json::Value::String(manager_approval_mode.clone()),
        );
        settings.insert(
            "managerAgentId".to_string(),
            serde_json::Value::String(manager_agent_id.to_string()),
        );
        settings.insert(
            "managerGroupId".to_string(),
            serde_json::Value::String(manager_agent_id.to_string()),
        );
        settings.insert(
            "serviceRuntimeAgentId".to_string(),
            serde_json::Value::String(service_agent_id.to_string()),
        );
        settings.insert(
            "optimizationMode".to_string(),
            serde_json::Value::String(optimization_mode.clone()),
        );
        settings.insert(
            "domain".to_string(),
            serde_json::Value::String("avatar".to_string()),
        );

        let output_form = Self::parse_output_form(Some("agent_only"))?;
        let document_access_mode = Self::parse_document_access_mode(Self::get_str_arg(
            args,
            &["document_access_mode", "documentAccessMode"],
        ))?;

        let req = CreatePortalRequest {
            name: name.to_string(),
            slug: Self::get_str_arg(args, &["slug"]).map(|s| s.to_string()),
            description: Self::get_str_arg(args, &["description"]).map(|s| s.to_string()),
            output_form,
            agent_enabled: Some(true),
            coding_agent_id: Some(manager_agent_id.to_string()),
            service_agent_id: Some(service_agent_id.to_string()),
            agent_id: None,
            agent_system_prompt: Self::get_str_arg(
                args,
                &["agent_system_prompt", "agentSystemPrompt"],
            )
            .map(|s| s.to_string()),
            agent_welcome_message: Self::get_str_arg(
                args,
                &["agent_welcome_message", "agentWelcomeMessage"],
            )
            .map(|s| s.to_string()),
            bound_document_ids: Self::parse_optional_string_list_any(
                args,
                &["bound_document_ids", "boundDocumentIds"],
            ),
            allowed_extensions: Self::parse_optional_string_list_any(
                args,
                &["allowed_extensions", "allowedExtensions"],
            ),
            allowed_skill_ids: Self::parse_optional_string_list_any(
                args,
                &["allowed_skill_ids", "allowedSkillIds"],
            ),
            document_access_mode,
            tags: Some(tags.clone()),
            settings: Some(serde_json::Value::Object(settings.clone())),
        };

        let svc = self.service();
        let portal = svc.create(&self.team_id, "agent", req).await?;
        let project_path = svc
            .initialize_project_folder(&self.team_id, &portal, &self.workspace_root)
            .await?;
        let detail = PortalDetail::from(portal);

        Ok(serde_json::to_string_pretty(&json!({
            "ok": true,
            "avatar": {
                "id": detail.id,
                "slug": detail.slug,
                "name": detail.name,
                "status": detail.status,
                "projectPath": project_path,
                "codingAgentId": detail.coding_agent_id,
                "serviceAgentId": detail.service_agent_id,
                "documentAccessMode": detail.document_access_mode,
                "tags": detail.tags,
                "settings": detail.settings,
                "publicUrl": format!("{}/p/{}", self.base_url, detail.slug),
            },
            "message": "Digital avatar created. Next: call configure_portal_service_agent for capability alignment, then re-read profile for verification.",
        }))?)
    }

    async fn handle_publish_portal(&self, args: &JsonObject) -> Result<String> {
        self.require_admin_mutation("publish_portal").await?;
        let portal_id = args
            .get("portal_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("portal_id is required"))?;
        self.get_portal_checked(portal_id, "publish_portal").await?;
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
        let domain_filter = match self.management_scope {
            PortalManagementScope::AvatarOnly => Some("avatar"),
            PortalManagementScope::Unscoped => None,
        };

        let svc = self.service();
        let result = if self.management_scope == PortalManagementScope::AvatarOnly {
            // In avatar-only scope, pull a larger window then apply manager-scope filter + paging.
            svc.list(&self.team_id, 1, 2000, domain_filter).await?
        } else {
            svc.list(&self.team_id, page, limit, domain_filter).await?
        };
        let scope_manager = self.scope_manager_agent_id().map(str::to_string);
        let scoped_items_all: Vec<_> = result
            .items
            .iter()
            .filter(|portal| {
                if self.management_scope != PortalManagementScope::AvatarOnly {
                    return true;
                }
                let Some(scope_manager_id) = scope_manager.as_deref() else {
                    return false;
                };
                let portal_manager_id = portal
                    .coding_agent_id
                    .as_deref()
                    .or(portal.agent_id.as_deref())
                    .map(str::trim)
                    .filter(|id| !id.is_empty());
                portal_manager_id == Some(scope_manager_id)
            })
            .collect();
        let scoped_items: Vec<_> = if self.management_scope == PortalManagementScope::AvatarOnly {
            let start = page.saturating_sub(1).saturating_mul(limit) as usize;
            let end = std::cmp::min(start.saturating_add(limit as usize), scoped_items_all.len());
            if start >= scoped_items_all.len() {
                Vec::new()
            } else {
                scoped_items_all[start..end].to_vec()
            }
        } else {
            scoped_items_all.clone()
        };

        Ok(serde_json::to_string_pretty(&json!({
            "items": scoped_items.iter().map(|p| json!({
                "id": p.id,
                "slug": p.slug,
                "name": p.name,
                "domain": match p.domain {
                    Some(PortalDomain::Avatar) => "avatar",
                    Some(PortalDomain::Ecosystem) => "ecosystem",
                    None => {
                        if Self::detect_domain_from_tags(&p.tags) == PortalDomainKind::Avatar {
                            "avatar"
                        } else {
                            "ecosystem"
                        }
                    }
                },
                "status": p.status,
                "agentEnabled": p.agent_enabled,
                "projectPath": p.project_path,
                "allowedExtensions": p.allowed_extensions,
                "allowedSkillIds": p.allowed_skill_ids,
                "publicUrl": format!("{}/p/{}", self.base_url, p.slug),
            })).collect::<Vec<_>>(),
            "total": if self.management_scope == PortalManagementScope::AvatarOnly {
                scoped_items_all.len() as u64
            } else {
                result.total
            },
            "page": if self.management_scope == PortalManagementScope::AvatarOnly {
                page
            } else {
                result.page
            },
            "scope": Self::scope_label(self.management_scope),
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
        let portal = self
            .get_portal_checked(portal_id, "get_portal_service_capability_profile")
            .await?;

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
        self.require_admin_mutation("configure_portal_service_agent")
            .await?;
        let portal_id = args
            .get("portal_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("portal_id is required"))?;
        self.get_portal_checked(portal_id, "configure_portal_service_agent")
            .await?;

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
        let mut coding_agent_override =
            Self::get_str_arg(args, &["coding_agent_id", "codingAgentId"])
                .map(|s| s.trim().to_string());
        if Self::get_bool_arg(args, &["clear_coding_agent", "clearCodingAgent"]).unwrap_or(false) {
            coding_agent_override = Some(String::new());
        }
        if self.management_scope == PortalManagementScope::AvatarOnly {
            if let Some(ref candidate) = coding_agent_override {
                if candidate.trim().is_empty() {
                    return Err(anyhow::anyhow!(
                        "configure_portal_service_agent cannot clear coding/manager agent in avatar scope"
                    ));
                }
                self.ensure_scope_manager_binding(candidate, "configure_portal_service_agent")?;
            }
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

        let clear_service_prompt = Self::get_bool_arg(
            args,
            &[
                "clear_service_agent_system_prompt",
                "clearServiceAgentSystemPrompt",
            ],
        )
        .unwrap_or(false);
        let has_replace_service_prompt = args.contains_key("service_agent_system_prompt")
            || args.contains_key("serviceAgentSystemPrompt");
        let has_append_service_prompt = args.contains_key("append_service_agent_system_prompt")
            || args.contains_key("appendServiceAgentSystemPrompt");
        if has_replace_service_prompt || has_append_service_prompt || clear_service_prompt {
            let target_agent = effective_service_agent_id.as_deref().ok_or_else(|| {
                anyhow::anyhow!("No effective service agent. Set service_agent_id first.")
            })?;
            let replace_prompt = Self::get_str_arg(
                args,
                &["service_agent_system_prompt", "serviceAgentSystemPrompt"],
            )
            .map(|s| s.to_string())
            .unwrap_or_default();
            let append_prompt = Self::get_str_arg(
                args,
                &[
                    "append_service_agent_system_prompt",
                    "appendServiceAgentSystemPrompt",
                ],
            )
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
            let prompt_value = if clear_service_prompt {
                String::new()
            } else {
                let mut next = if has_replace_service_prompt {
                    replace_prompt
                } else {
                    agent_svc
                        .get_agent(target_agent)
                        .await?
                        .and_then(|agent| agent.system_prompt)
                        .unwrap_or_default()
                };
                if has_append_service_prompt && !append_prompt.is_empty() {
                    if next.trim().is_empty() {
                        next = append_prompt;
                    } else {
                        next.push_str("\n\n");
                        next.push_str(&append_prompt);
                    }
                }
                next
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
        let coding_agent_id_update = coding_agent_override.map(|agent_id| {
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

        let clear_tags = Self::get_bool_arg(args, &["clear_tags", "clearTags"]).unwrap_or(false);
        let replace_tags = Self::parse_optional_string_list_any(args, &["tags"]);
        let add_tags = Self::parse_optional_string_list_any(args, &["add_tags", "addTags"]);
        let remove_tags =
            Self::parse_optional_string_list_any(args, &["remove_tags", "removeTags"]);
        let should_update_tags =
            clear_tags || replace_tags.is_some() || add_tags.is_some() || remove_tags.is_some();
        let mut tags = if should_update_tags {
            let mut next = if clear_tags {
                Vec::new()
            } else if let Some(replace) = replace_tags.clone() {
                replace
            } else {
                current.tags.clone()
            };
            if let Some(adds) = add_tags.clone() {
                for tag in adds {
                    if !next.iter().any(|v| v == &tag) {
                        next.push(tag);
                    }
                }
            }
            if let Some(removes) = remove_tags.clone() {
                next.retain(|tag| !removes.iter().any(|remove| remove == tag));
            }
            Some(next)
        } else {
            None
        };

        let has_settings_patch =
            args.contains_key("settings_patch") || args.contains_key("settingsPatch");
        let clear_settings =
            Self::get_bool_arg(args, &["clear_settings", "clearSettings"]).unwrap_or(false);
        let mut merged_settings = if clear_settings {
            serde_json::Map::new()
        } else {
            match &current.settings {
                serde_json::Value::Object(obj) => obj.clone(),
                _ => serde_json::Map::new(),
            }
        };
        if let Some(patch) = Self::get_json_object_arg(args, &["settings_patch", "settingsPatch"]) {
            for (key, value) in patch {
                merged_settings.insert(key, value);
            }
        }
        if let Some(v) = Self::get_bool_arg(args, &["show_chat_widget", "showChatWidget"]) {
            merged_settings.insert("showChatWidget".to_string(), serde_json::Value::Bool(v));
        }
        let mut settings = if clear_settings
            || has_settings_patch
            || args.contains_key("show_chat_widget")
            || args.contains_key("showChatWidget")
        {
            Some(serde_json::Value::Object(merged_settings))
        } else {
            None
        };

        // Domain is structural metadata, not a service-agent capability.
        // Keep domain immutable for this tool and normalize tags/settings.
        let current_domain = Self::detect_domain_from_portal(&current);
        if let Some(next_tags) = tags.as_mut() {
            Self::normalize_domain_tags(next_tags, current_domain);
        }
        if let Some(raw_settings) = settings.take() {
            let mut next_settings = match raw_settings {
                serde_json::Value::Object(obj) => obj,
                _ => serde_json::Map::new(),
            };
            next_settings.insert(
                "domain".to_string(),
                serde_json::Value::String(Self::domain_label(current_domain).to_string()),
            );
            settings = Some(serde_json::Value::Object(next_settings));
        }
        if self.management_scope == PortalManagementScope::AvatarOnly {
            let next_tags = tags.get_or_insert_with(|| current.tags.clone());
            Self::normalize_domain_tags(next_tags, PortalDomainKind::Avatar);
            let mut next_settings = match settings.take() {
                Some(serde_json::Value::Object(obj)) => obj,
                Some(_) | None => match &current.settings {
                    serde_json::Value::Object(obj) => obj.clone(),
                    _ => serde_json::Map::new(),
                },
            };
            next_settings.insert(
                "domain".to_string(),
                serde_json::Value::String("avatar".to_string()),
            );
            settings = Some(serde_json::Value::Object(next_settings));
        }

        let updated_domain = Self::detect_domain_from_tags_and_settings(
            tags.as_deref().unwrap_or(&current.tags),
            settings.as_ref().unwrap_or(&current.settings),
        );
        if updated_domain != current_domain {
            return Err(anyhow::anyhow!(
                "configure_portal_service_agent cannot change portal domain from '{}' to '{}'. Create a new portal in the target domain and migrate configuration.",
                Self::domain_label(current_domain),
                Self::domain_label(updated_domain),
            ));
        }
        self.ensure_scope_allows_domain(updated_domain, "configure_portal_service_agent")?;

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
                    coding_agent_id: coding_agent_id_update,
                    service_agent_id: service_agent_id_update,
                    agent_id: None,
                    agent_system_prompt: None,
                    agent_welcome_message,
                    bound_document_ids,
                    allowed_extensions,
                    allowed_skill_ids,
                    document_access_mode,
                    tags,
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
                "codingAgentId": updated.coding_agent_id,
                "serviceAgentId": updated.service_agent_id,
                "documentAccessMode": updated.document_access_mode,
                "boundDocumentIds": updated.bound_document_ids,
                "allowedExtensions": updated.allowed_extensions,
                "allowedSkillIds": updated.allowed_skill_ids,
                "tags": updated.tags,
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
