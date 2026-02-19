//! Team Skill Tools provider for cloud agent on-demand skill access.
//!
//! Exposes in-process tools to search and load team shared skills at runtime.

use agime::agents::mcp_client::McpClientTrait;
use agime_team::db::MongoDb;
use agime_team::services::mongo::SkillService;
use anyhow::{anyhow, Result};
use rmcp::model::*;
use rmcp::ServiceError;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::resource_access::is_runtime_resource_allowed;

const MAX_SKILL_CONTENT_BYTES: usize = 128 * 1024;
const DEFAULT_SEARCH_LIMIT: u64 = 20;
const MAX_SEARCH_LIMIT: u64 = 100;

pub struct TeamSkillToolsProvider {
    db: Arc<MongoDb>,
    team_id: String,
    allowed_skill_ids: Option<HashSet<String>>,
    info: InitializeResult,
}

impl TeamSkillToolsProvider {
    pub fn new(
        db: Arc<MongoDb>,
        team_id: String,
        allowed_skill_ids: Option<HashSet<String>>,
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
            },
            server_info: Implementation {
                name: "team_skills".to_string(),
                title: Some("Team Skills".to_string()),
                version: "1.0.0".to_string(),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Use search/load to retrieve team-shared skills on demand without injecting full skill content into system prompt."
                    .to_string(),
            ),
        };
        Self {
            db,
            team_id,
            allowed_skill_ids,
            info,
        }
    }

    fn service(&self) -> SkillService {
        SkillService::new((*self.db).clone())
    }

    fn is_skill_allowed(&self, skill_id: &str) -> bool {
        self.allowed_skill_ids
            .as_ref()
            .map(|set| set.contains(skill_id))
            .unwrap_or(true)
    }

    fn tool_definitions() -> Vec<Tool> {
        vec![
            Tool {
                name: "search".into(),
                title: None,
                description: Some(
                    "Search team shared skills by name/description and return short metadata."
                        .into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query (optional)" },
                        "limit": { "type": "integer", "description": "Max results (default 20, max 100)" }
                    }
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "load".into(),
                title: None,
                description: Some("Load full content of a team shared skill by skill_id.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "skill_id": { "type": "string", "description": "Skill ObjectId hex string" }
                    },
                    "required": ["skill_id"]
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                icons: None,
                meta: None,
            },
        ]
    }
}

#[async_trait::async_trait]
impl McpClientTrait for TeamSkillToolsProvider {
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
            "search" => self.handle_search(&args).await,
            "load" => self.handle_load(&args).await,
            _ => Err(anyhow!("Unknown tool: {}", name)),
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
        Some(&self.info)
    }
}

impl TeamSkillToolsProvider {
    async fn handle_search(&self, args: &JsonObject) -> Result<String> {
        let query = args.get("query").and_then(|v| v.as_str());
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .min(MAX_SEARCH_LIMIT);

        let svc = self.service();
        let result = svc
            .list(
                &self.team_id,
                Some(1),
                Some(limit),
                query,
                Some("use_count"),
            )
            .await?;

        let eligible: Vec<_> = result
            .items
            .into_iter()
            .filter(|s| is_runtime_resource_allowed(&s.visibility, &s.protection_level))
            .filter(|s| self.is_skill_allowed(&s.id))
            .collect();

        let items: Vec<serde_json::Value> = eligible
            .into_iter()
            .map(|s| {
                json!({
                    "id": s.id,
                    "name": s.name,
                    "description": s.description,
                    "version": s.version,
                    "tags": s.tags,
                    "visibility": s.visibility,
                    "protection_level": s.protection_level,
                    "use_count": s.use_count,
                })
            })
            .collect();

        Ok(json!({
            "team_id": self.team_id,
            "total": items.len(),
            "skills": items,
        })
        .to_string())
    }

    async fn handle_load(&self, args: &JsonObject) -> Result<String> {
        let skill_id = args
            .get("skill_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("skill_id is required"))?;

        if !self.is_skill_allowed(skill_id) {
            return Err(anyhow!(
                "Skill '{}' is not allowed in this portal session",
                skill_id
            ));
        }

        let svc = self.service();
        let skill = svc
            .get(skill_id)
            .await?
            .ok_or_else(|| anyhow!("Skill not found"))?;
        if skill.team_id.to_hex() != self.team_id {
            return Err(anyhow!("Skill does not belong to this team"));
        }
        if !is_runtime_resource_allowed(&skill.visibility, &skill.protection_level) {
            return Err(anyhow!(
                "Skill is not available to runtime due to visibility/protection policy"
            ));
        }

        let mut content = skill
            .skill_md
            .clone()
            .or(skill.content.clone())
            .unwrap_or_default();
        let mut truncated = false;
        if content.len() > MAX_SKILL_CONTENT_BYTES {
            content = content
                .chars()
                .take(MAX_SKILL_CONTENT_BYTES)
                .collect::<String>();
            truncated = true;
        }

        let files: Vec<serde_json::Value> = skill
            .files
            .iter()
            .map(|f| json!({ "path": f.path }))
            .collect();

        // Best-effort statistics update.
        let _ = svc.increment_use_count(skill_id).await;

        Ok(json!({
            "id": skill.id.map(|id| id.to_hex()).unwrap_or_default(),
            "name": skill.name.clone(),
            "description": skill.description.clone(),
            "version": skill.version.clone(),
            "storage_type": match skill.storage_type {
                agime_team::models::mongo::SkillStorageType::Inline => "inline",
                agime_team::models::mongo::SkillStorageType::Package => "package",
            },
            "content": content,
            "truncated": truncated,
            "files": files,
            "manifest": skill.manifest.clone(),
            "metadata": skill.metadata.clone(),
            "tags": skill.tags.clone(),
        })
        .to_string())
    }
}
