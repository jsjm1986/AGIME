//! Team MCP lifecycle tools.
//!
//! Formal MCP lifecycle on top of the team extension library + agent attachment chain.

use agime::agents::mcp_client::McpClientTrait;
use agime_team::db::MongoDb;
use agime_team::models::{ListAgentsQuery, TeamAgent};
use agime_team::services::mongo::ExtensionService;
use anyhow::{anyhow, Result};
use mongodb::bson::{self, doc, Bson, Document as BsonDocument};
use rmcp::model::*;
use rmcp::ServiceError;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::service_mongo::AgentService;

const MCP_TYPES: &[&str] = &["stdio", "sse", "streamable_http"];

#[derive(Clone)]
struct McpTemplatePreset {
    id: &'static str,
    title_zh: &'static str,
    title_en: &'static str,
    description_zh: &'static str,
    category: &'static str,
    transport: &'static str,
    default_name: &'static str,
    uri_or_cmd: &'static str,
    args: &'static [&'static str],
    envs: &'static [(&'static str, &'static str)],
    note_zh: &'static str,
    keywords: &'static [&'static str],
}

const MCP_TEMPLATES: &[McpTemplatePreset] = &[
    McpTemplatePreset {
        id: "filesystem",
        title_zh: "文件系统",
        title_en: "Filesystem",
        description_zh: "把某个受控目录暴露给 Agent 的官方文件系统 MCP。",
        category: "filesystem",
        transport: "stdio",
        default_name: "filesystem",
        uri_or_cmd: "npx",
        args: &[
            "-y",
            "@modelcontextprotocol/server-filesystem",
            "/path/to/allowed/root",
        ],
        envs: &[],
        note_zh: "请把最后一个参数替换成真实可访问目录。",
        keywords: &["filesystem", "files", "directory", "stdio"],
    },
    McpTemplatePreset {
        id: "playwright",
        title_zh: "浏览器",
        title_en: "Playwright",
        description_zh: "浏览器自动化 MCP，适合页面检查、表单操作和回归验证。",
        category: "browser",
        transport: "stdio",
        default_name: "playwright",
        uri_or_cmd: "npx",
        args: &["-y", "@playwright/mcp@latest"],
        envs: &[("PLAYWRIGHT_HEADLESS", "1")],
        note_zh: "默认按无头模式预填，需要图形化浏览器时再调整环境变量。",
        keywords: &["playwright", "browser", "automation", "web"],
    },
    McpTemplatePreset {
        id: "remote-sse",
        title_zh: "远程 SSE",
        title_en: "Remote SSE",
        description_zh: "适合已经在外部运行的 SSE MCP 服务，只需填写访问地址。",
        category: "remote",
        transport: "sse",
        default_name: "remote-sse",
        uri_or_cmd: "http://127.0.0.1:8931/sse",
        args: &[],
        envs: &[],
        note_zh: "确认服务端已经启动，并且当前 Agent 所在环境可访问这个地址。",
        keywords: &["remote", "sse", "service", "endpoint"],
    },
    McpTemplatePreset {
        id: "streamable-http",
        title_zh: "Streamable HTTP",
        title_en: "Streamable HTTP",
        description_zh: "适合通过 HTTP 暴露的 MCP 服务，便于统一部署和网关接入。",
        category: "remote",
        transport: "streamable_http",
        default_name: "streamable-http",
        uri_or_cmd: "http://127.0.0.1:8931/mcp",
        args: &[],
        envs: &[],
        note_zh: "先确认服务端支持真正的 MCP HTTP 入口，必要时补充鉴权变量。",
        keywords: &["http", "streamable", "remote", "mcp"],
    },
    McpTemplatePreset {
        id: "local-script",
        title_zh: "本地脚本",
        title_en: "Local Script",
        description_zh: "适合你已经有 Python、Node 或其他本地脚本型 MCP 服务时快速接入。",
        category: "script",
        transport: "stdio",
        default_name: "local-script",
        uri_or_cmd: "python",
        args: &["server.py"],
        envs: &[],
        note_zh: "先确认命令能在服务器环境中直接运行，再补充启动参数。",
        keywords: &["local", "script", "python", "node", "stdio"],
    },
];

fn build_extension_ref(extension_id: &str, name: &str, extension_class: &str, meta: &str) -> String {
    format!("[[ext:{}|{}|{}|{}]]", extension_id, name, extension_class, meta)
}

fn build_extension_display_line_zh(extension_ref: &str, transport: &str) -> String {
    format!("{}（团队 MCP，{}）", extension_ref, transport.to_uppercase())
}

fn build_extension_plain_line_zh(name: &str, transport: &str) -> String {
    format!("{}（团队 MCP，{}）", name, transport.to_uppercase())
}

fn normalize_mcp_transport(raw: &str) -> Result<String> {
    let normalized = raw
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .replace(' ', "_");
    if MCP_TYPES.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err(anyhow!(
            "Invalid MCP transport '{}'. Use one of: stdio | sse | streamable_http",
            raw
        ))
    }
}

fn parse_string_array(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Vec<String>> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| anyhow!("'{}' must be an array of strings", key))?;
    array
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
                .ok_or_else(|| anyhow!("'{}' must be an array of strings", key))
        })
        .collect()
}

fn parse_string_map(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<std::collections::HashMap<String, String>> {
    let Some(value) = args.get(key) else {
        return Ok(std::collections::HashMap::new());
    };
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("'{}' must be an object of string values", key))?;
    object
        .iter()
        .map(|(map_key, map_value)| {
            map_value
                .as_str()
                .map(|value| (map_key.trim().to_string(), value.trim().to_string()))
                .filter(|(trimmed_key, _)| !trimmed_key.is_empty())
                .ok_or_else(|| anyhow!("'{}' must be an object of string values", key))
        })
        .collect()
}

fn parse_required_string(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .ok_or_else(|| anyhow!("Missing required field '{}'", key))
}

fn parse_optional_string(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

fn parse_bool(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: bool,
) -> bool {
    args.get(key).and_then(|value| value.as_bool()).unwrap_or(default)
}

pub struct TeamMcpToolsProvider {
    db: Arc<MongoDb>,
    team_id: String,
    actor_user_id: String,
    info: InitializeResult,
}

impl TeamMcpToolsProvider {
    pub fn new(db: Arc<MongoDb>, team_id: String, actor_user_id: String) -> Self {
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
                name: "team_mcp".to_string(),
                title: Some("Team MCP".to_string()),
                version: "1.0.0".to_string(),
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Use these tools for the formal MCP lifecycle: search templates, list installed team MCP resources, install into the team extension library, then optionally attach to specific agents. Never describe workspace-only clone/npm installs as system installation."
                    .to_string(),
            ),
        };
        Self {
            db,
            team_id,
            actor_user_id,
            info,
        }
    }

    fn extension_service(&self) -> ExtensionService {
        ExtensionService::new((*self.db).clone())
    }

    fn agent_service(&self) -> AgentService {
        AgentService::new(self.db.clone())
    }

    async fn ensure_admin(&self) -> Result<()> {
        let service = self.agent_service();
        let allowed = service
            .is_team_admin(&self.actor_user_id, &self.team_id)
            .await
            .map_err(|e| anyhow!("Failed to verify team admin permission: {}", e))?;
        if allowed {
            Ok(())
        } else {
            Err(anyhow!("Current user is not allowed to manage team MCP resources"))
        }
    }

    async fn resolve_extension(
        &self,
        extension_id_or_name: &str,
    ) -> Result<agime_team::models::mongo::Extension> {
        let ext_service = self.extension_service();
        if let Ok(Some(extension)) = ext_service.get(extension_id_or_name).await {
            if extension.team_id.to_hex() == self.team_id {
                return Ok(extension);
            }
        }

        let list = ext_service.list_active_for_team(&self.team_id).await?;
        list.into_iter()
            .find(|extension| extension.name.eq_ignore_ascii_case(extension_id_or_name))
            .ok_or_else(|| anyhow!("Team MCP extension '{}' not found", extension_id_or_name))
    }

    fn template_matches(template: &McpTemplatePreset, query: &str, category: Option<&str>) -> bool {
        if let Some(category) = category {
            if !category.trim().is_empty() && !template.category.eq_ignore_ascii_case(category) {
                return false;
            }
        }
        if query.trim().is_empty() {
            return true;
        }
        let lowered = query.trim().to_ascii_lowercase();
        template.id.to_ascii_lowercase().contains(&lowered)
            || template.title_zh.to_ascii_lowercase().contains(&lowered)
            || template.title_en.to_ascii_lowercase().contains(&lowered)
            || template.description_zh.to_ascii_lowercase().contains(&lowered)
            || template
                .keywords
                .iter()
                .any(|keyword| keyword.to_ascii_lowercase().contains(&lowered))
    }

    fn template_payload(template: &McpTemplatePreset) -> serde_json::Value {
        json!({
            "id": template.id,
            "title_zh": template.title_zh,
            "title_en": template.title_en,
            "description_zh": template.description_zh,
            "category": template.category,
            "transport": template.transport,
            "default_name": template.default_name,
            "uri_or_cmd": template.uri_or_cmd,
            "args": template.args,
            "envs": template.envs.iter().map(|(key, value)| json!({ "key": key, "value": value })).collect::<Vec<_>>(),
            "note_zh": template.note_zh,
            "keywords": template.keywords,
        })
    }

    fn extension_payload(
        extension: &agime_team::models::mongo::Extension,
        attached_agents: &[TeamAgent],
    ) -> serde_json::Value {
        let extension_id = extension.id.map(|id| id.to_hex()).unwrap_or_default();
        let ext_ref = build_extension_ref(
            &format!("team:{}", extension_id),
            &extension.name,
            "mcp",
            &extension.extension_type,
        );
        json!({
            "id": extension_id,
            "name": extension.name,
            "description": extension.description,
            "transport": extension.extension_type,
            "ext_ref": ext_ref,
            "display_line_zh": build_extension_display_line_zh(&ext_ref, &extension.extension_type),
            "plain_line_zh": build_extension_plain_line_zh(&extension.name, &extension.extension_type),
            "version": extension.version,
            "config": bson::from_document::<serde_json::Value>(extension.config.clone()).unwrap_or_else(|_| json!({})),
            "tags": extension.tags,
            "attached_agents": attached_agents.iter().map(|agent| json!({
                "agent_id": agent.id,
                "name": agent.name,
                "agent_role": agent.agent_role,
                "agent_domain": agent.agent_domain,
            })).collect::<Vec<_>>(),
            "attached_count": attached_agents.len(),
        })
    }

    async fn handle_list_templates(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<String> {
        let query = parse_optional_string(args, "query").unwrap_or_default();
        let category = parse_optional_string(args, "category");
        let items = MCP_TEMPLATES
            .iter()
            .filter(|template| Self::template_matches(template, &query, category.as_deref()))
            .map(Self::template_payload)
            .collect::<Vec<_>>();
        Ok(serde_json::to_string_pretty(&json!({
            "query": query,
            "category": category,
            "count": items.len(),
            "templates": items,
            "guidance": "从模板挑选后，可调用 install_team_mcp 正式写入团队扩展库；若需要立即给某个 Agent 使用，再传 attach_agent_ids。"
        }))?)
    }

    async fn handle_list_installed(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<String> {
        let query = parse_optional_string(args, "query")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let include_attached = parse_bool(args, "include_attached_agents", true);
        let ext_service = self.extension_service();
        let agent_service = self.agent_service();
        let extensions = ext_service.list_active_for_team(&self.team_id).await?;
        let all_agents = if include_attached {
            agent_service
                .list_agents(ListAgentsQuery {
                    team_id: self.team_id.clone(),
                    page: 1,
                    limit: 100,
                })
                .await
                .map_err(|e| anyhow!("Failed to list team agents: {}", e))?
                .items
        } else {
            Vec::new()
        };

        let items = extensions
            .into_iter()
            .filter(|extension| MCP_TYPES.contains(&extension.extension_type.as_str()))
            .filter(|extension| {
                if query.is_empty() {
                    true
                } else {
                    extension.name.to_ascii_lowercase().contains(&query)
                        || extension
                            .description
                            .as_deref()
                            .unwrap_or_default()
                            .to_ascii_lowercase()
                            .contains(&query)
                }
            })
            .map(|extension| {
                let source_id = extension.id.map(|value| value.to_hex()).unwrap_or_default();
                let attached_agents = if include_attached {
                    all_agents
                        .iter()
                        .filter(|agent| {
                            agent.custom_extensions.iter().any(|custom| {
                                custom.source_extension_id.as_deref() == Some(source_id.as_str())
                            })
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                Self::extension_payload(&extension, &attached_agents)
            })
            .collect::<Vec<_>>();

        Ok(serde_json::to_string_pretty(&json!({
            "count": items.len(),
            "items": items,
            "guidance": "这些结果代表团队扩展库中的正式 MCP 资源。若要给具体 Agent 使用，请调用 attach_team_mcp；若要删除正式资源，请调用 remove_team_mcp。"
        }))?)
    }

    async fn handle_install_team_mcp(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<String> {
        self.ensure_admin().await?;

        let name = parse_required_string(args, "name")?;
        let transport = normalize_mcp_transport(&parse_required_string(args, "type")?)?;
        let uri_or_cmd = parse_required_string(args, "uri_or_cmd")?;
        let arguments = parse_string_array(args, "args")?;
        let envs = parse_string_map(args, "envs")?;
        let description = parse_optional_string(args, "description");
        let visibility =
            parse_optional_string(args, "visibility").unwrap_or_else(|| "team".to_string());
        let attach_agent_ids = parse_string_array(args, "attach_agent_ids")?;

        let mut tags = parse_string_array(args, "tags")?;
        if !tags.iter().any(|tag| tag.eq_ignore_ascii_case("mcp")) {
            tags.push("mcp".to_string());
        }
        if !tags.iter().any(|tag| tag.eq_ignore_ascii_case(&transport)) {
            tags.push(transport.clone());
        }

        let config = doc! {
            "uri_or_cmd": uri_or_cmd.clone(),
            "args": arguments.clone(),
            "envs": bson::to_bson(&envs).unwrap_or(Bson::Document(BsonDocument::new())),
        };

        let ext_service = self.extension_service();
        let created = ext_service
            .create(
                &self.team_id,
                &self.actor_user_id,
                &name,
                &transport,
                config,
                description.clone(),
                Some(tags.clone()),
                Some(visibility.clone()),
            )
            .await?;

        let extension_id = created.id.map(|id| id.to_hex()).unwrap_or_default();
        let agent_service = self.agent_service();
        let mut attached = Vec::new();
        for agent_id in &attach_agent_ids {
            let attached_agent = agent_service
                .add_team_extension_to_agent(agent_id, &extension_id, &self.team_id)
                .await
                .map_err(|e| anyhow!("Failed to attach MCP to agent {}: {}", agent_id, e))?
                .ok_or_else(|| anyhow!("Agent '{}' not found", agent_id))?;
            attached.push(json!({
                "agent_id": attached_agent.id,
                "name": attached_agent.name,
            }));
        }

        let ext_ref = build_extension_ref(
            &format!("team:{}", extension_id),
            &created.name,
            "mcp",
            &created.extension_type,
        );
        Ok(serde_json::to_string_pretty(&json!({
            "installed": true,
            "extension_id": extension_id,
            "ext_ref": ext_ref,
            "display_line_zh": build_extension_display_line_zh(&ext_ref, &created.extension_type),
            "plain_line_zh": build_extension_plain_line_zh(&created.name, &created.extension_type),
            "attach_count": attached.len(),
            "attached_agents": attached,
            "next_steps": if attach_agent_ids.is_empty() {
                "团队扩展库已创建。若要让具体 Agent 使用，请再调用 attach_team_mcp。"
            } else {
                "团队扩展库已创建，并已挂载到指定 Agent。"
            }
        }))?)
    }

    async fn handle_attach_team_mcp(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<String> {
        self.ensure_admin().await?;
        let extension_id_or_name = parse_required_string(args, "extension_id_or_name")?;
        let agent_ids = parse_string_array(args, "agent_ids")?;
        if agent_ids.is_empty() {
            return Err(anyhow!("Missing required field 'agent_ids'"));
        }
        let extension = self.resolve_extension(&extension_id_or_name).await?;
        let extension_id = extension.id.map(|id| id.to_hex()).unwrap_or_default();
        let agent_service = self.agent_service();
        let mut attached = Vec::new();
        for agent_id in &agent_ids {
            let attached_agent = agent_service
                .add_team_extension_to_agent(agent_id, &extension_id, &self.team_id)
                .await
                .map_err(|e| anyhow!("Failed to attach MCP to agent {}: {}", agent_id, e))?
                .ok_or_else(|| anyhow!("Agent '{}' not found", agent_id))?;
            attached.push(json!({
                "agent_id": attached_agent.id,
                "name": attached_agent.name,
            }));
        }
        Ok(serde_json::to_string_pretty(&json!({
            "attached": true,
            "extension_id": extension_id,
            "attached_count": attached.len(),
            "attached_agents": attached,
        }))?)
    }

    async fn handle_update_team_mcp(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<String> {
        self.ensure_admin().await?;
        let extension_id_or_name = parse_required_string(args, "extension_id_or_name")?;
        let current = self.resolve_extension(&extension_id_or_name).await?;
        let current_id = current.id.map(|id| id.to_hex()).unwrap_or_default();

        let next_name = parse_optional_string(args, "name");
        let next_description = if args.contains_key("description") {
            Some(parse_optional_string(args, "description").unwrap_or_default())
        } else {
            None
        };
        let sync_attached = parse_bool(args, "sync_attached", true);
        let has_config_change = args.contains_key("type")
            || args.contains_key("uri_or_cmd")
            || args.contains_key("args")
            || args.contains_key("envs");

        let next_config = if has_config_change {
            let transport = args
                .get("type")
                .and_then(|value| value.as_str())
                .map(normalize_mcp_transport)
                .transpose()?
                .unwrap_or_else(|| current.extension_type.clone());
            let uri_or_cmd = args
                .get("uri_or_cmd")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
                .or_else(|| {
                    current
                        .config
                        .get_str("uri_or_cmd")
                        .or_else(|_| current.config.get_str("uriOrCmd"))
                        .or_else(|_| current.config.get_str("command"))
                        .ok()
                        .map(|value| value.to_string())
                })
                .ok_or_else(|| anyhow!("MCP config requires uri_or_cmd"))?;
            let next_args = if args.contains_key("args") {
                parse_string_array(args, "args")?
            } else {
                current
                    .config
                    .get_array("args")
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| value.as_str().map(|item| item.to_string()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            };
            let next_envs = if args.contains_key("envs") {
                parse_string_map(args, "envs")?
            } else {
                current
                    .config
                    .get_document("envs")
                    .map(|doc| {
                        doc.iter()
                            .filter_map(|(key, value)| {
                                value.as_str().map(|item| (key.clone(), item.to_string()))
                            })
                            .collect::<std::collections::HashMap<_, _>>()
                    })
                    .unwrap_or_default()
            };
            Some(doc! {
                "uri_or_cmd": uri_or_cmd,
                "args": next_args,
                "envs": bson::to_bson(&next_envs).unwrap_or(Bson::Document(BsonDocument::new())),
                "transport": transport,
            })
        } else {
            None
        };

        let ext_service = self.extension_service();
        let updated = ext_service
            .update(
                &current_id,
                next_name.clone(),
                args.get("type")
                    .and_then(|value| value.as_str())
                    .map(normalize_mcp_transport)
                    .transpose()?,
                next_description.clone(),
                next_config,
            )
            .await?;

        let synced_agents = if sync_attached {
            self.agent_service()
                .sync_team_extension_to_attached_agents(&self.team_id, &current_id)
                .await
                .map_err(|e| anyhow!("Failed to sync attached agents: {}", e))?
                .into_iter()
                .map(|agent| json!({ "agent_id": agent.id, "name": agent.name }))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let ext_ref = build_extension_ref(
            &format!("team:{}", current_id),
            &updated.name,
            "mcp",
            &updated.extension_type,
        );
        Ok(serde_json::to_string_pretty(&json!({
            "updated": true,
            "extension_id": current_id,
            "ext_ref": ext_ref,
            "display_line_zh": build_extension_display_line_zh(&ext_ref, &updated.extension_type),
            "plain_line_zh": build_extension_plain_line_zh(&updated.name, &updated.extension_type),
            "synced_attached_agents": synced_agents,
            "sync_count": synced_agents.len(),
        }))?)
    }

    async fn handle_remove_team_mcp(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<String> {
        self.ensure_admin().await?;
        let extension_id_or_name = parse_required_string(args, "extension_id_or_name")?;
        let detach_attached = parse_bool(args, "detach_attached", true);
        let extension = self.resolve_extension(&extension_id_or_name).await?;
        let extension_id = extension.id.map(|id| id.to_hex()).unwrap_or_default();

        let detached_agents = if detach_attached {
            self.agent_service()
                .detach_team_extension_from_attached_agents(&self.team_id, &extension_id)
                .await
                .map_err(|e| anyhow!("Failed to detach attached agents: {}", e))?
                .into_iter()
                .map(|agent| json!({ "agent_id": agent.id, "name": agent.name }))
                .collect::<Vec<_>>()
        } else {
            let attached = self
                .agent_service()
                .list_agents_attached_to_team_extension(&self.team_id, &extension_id)
                .await
                .map_err(|e| anyhow!("Failed to inspect attached agents: {}", e))?;
            if !attached.is_empty() {
                return Err(anyhow!(
                    "Extension is still attached to {} agent(s); rerun with detach_attached=true",
                    attached.len()
                ));
            }
            Vec::new()
        };

        self.extension_service().delete(&extension_id).await?;
        Ok(serde_json::to_string_pretty(&json!({
            "removed": true,
            "extension_id": extension_id,
            "name": extension.name,
            "detached_count": detached_agents.len(),
            "detached_agents": detached_agents,
        }))?)
    }

    fn tool_definitions() -> Vec<Tool> {
        vec![
            Tool {
                name: "list_templates".into(),
                title: None,
                description: Some("List built-in MCP installation templates for formal team installation planning.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "category": { "type": "string", "description": "filesystem | browser | remote | script" }
                    }
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "list_installed".into(),
                title: None,
                description: Some("List formal MCP resources already stored in the team extension library, optionally with attached agent usage.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "include_attached_agents": { "type": "boolean" }
                    }
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "install_team_mcp".into(),
                title: None,
                description: Some("Formally install a custom MCP into the team extension library, then optionally attach it to one or more agents. This is the preferred installation chain.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "type": { "type": "string", "description": "stdio | sse | streamable_http" },
                        "uri_or_cmd": { "type": "string" },
                        "args": { "type": "array", "items": { "type": "string" } },
                        "envs": { "type": "object", "additionalProperties": { "type": "string" } },
                        "description": { "type": "string" },
                        "tags": { "type": "array", "items": { "type": "string" } },
                        "visibility": { "type": "string" },
                        "attach_agent_ids": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["name", "type", "uri_or_cmd"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "attach_team_mcp".into(),
                title: None,
                description: Some("Attach an already installed team MCP resource to one or more agents.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "extension_id_or_name": { "type": "string" },
                        "agent_ids": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["extension_id_or_name", "agent_ids"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "update_team_mcp".into(),
                title: None,
                description: Some("Update one formal team MCP resource and optionally sync the updated config into all attached agent copies.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "extension_id_or_name": { "type": "string" },
                        "name": { "type": "string" },
                        "description": { "type": "string" },
                        "type": { "type": "string", "description": "stdio | sse | streamable_http" },
                        "uri_or_cmd": { "type": "string" },
                        "args": { "type": "array", "items": { "type": "string" } },
                        "envs": { "type": "object", "additionalProperties": { "type": "string" } },
                        "sync_attached": { "type": "boolean" }
                    },
                    "required": ["extension_id_or_name"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "remove_team_mcp".into(),
                title: None,
                description: Some("Remove one formal team MCP resource from the team library, optionally detaching all attached agent copies first.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "extension_id_or_name": { "type": "string" },
                        "detach_attached": { "type": "boolean" }
                    },
                    "required": ["extension_id_or_name"]
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
impl McpClientTrait for TeamMcpToolsProvider {
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
            "list_templates" => self.handle_list_templates(&args).await,
            "list_installed" => self.handle_list_installed(&args).await,
            "install_team_mcp" => self.handle_install_team_mcp(&args).await,
            "attach_team_mcp" => self.handle_attach_team_mcp(&args).await,
            "update_team_mcp" => self.handle_update_team_mcp(&args).await,
            "remove_team_mcp" => self.handle_remove_team_mcp(&args).await,
            _ => Err(anyhow!("Unknown tool: {}", name)),
        };

        match result {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(error.to_string())])),
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
