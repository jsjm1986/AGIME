//! Team MCP lifecycle tools.
//!
//! Formal MCP lifecycle on top of the team extension library + agent attachment chain.

use agime::agents::mcp_client::McpClientTrait;
use agime_team::db::MongoDb;
use agime_team::models::{BuiltinExtension, CustomExtensionConfig, ListAgentsQuery, TeamAgent};
use agime_team::services::mongo::ExtensionService;
use anyhow::{anyhow, Result};
use mongodb::bson::{self, doc, Bson, Document as BsonDocument};
use rmcp::model::*;
use rmcp::ServiceError;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::agent_prompt_composer::build_prompt_introspection_snapshot;
use super::capability_policy::{
    builtin_registry_entry, AgentRuntimePolicyResolver, ConfiguredBuiltinCapability,
};
use super::service_mongo::AgentService;

const MCP_TYPES: &[&str] = &["stdio", "sse", "streamable_http"];

#[derive(Debug, Clone)]
struct PlannedMcpInstall {
    name: Option<String>,
    transport: Option<String>,
    uri_or_cmd: Option<String>,
    args: Vec<String>,
    envs: HashMap<String, String>,
    description: Option<String>,
    source_url: Option<String>,
    shell_command: Option<String>,
    notes: Vec<String>,
}

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

fn build_extension_ref(
    extension_id: &str,
    name: &str,
    extension_class: &str,
    meta: &str,
) -> String {
    format!(
        "[[ext:{}|{}|{}|{}]]",
        extension_id, name, extension_class, meta
    )
}

fn build_scoped_extension_display_line_zh(
    extension_ref: &str,
    label: &str,
    transport: Option<&str>,
) -> String {
    match transport.filter(|value| !value.trim().is_empty()) {
        Some(value) => format!("{}（{}，{}）", extension_ref, label, value.to_uppercase()),
        None => format!("{}（{}）", extension_ref, label),
    }
}

fn build_scoped_extension_display_line_en(
    extension_ref: &str,
    label: &str,
    transport: Option<&str>,
) -> String {
    match transport.filter(|value| !value.trim().is_empty()) {
        Some(value) => format!("{} ({}, {})", extension_ref, label, value.to_uppercase()),
        None => format!("{} ({})", extension_ref, label),
    }
}

fn build_scoped_extension_plain_line_zh(
    name: &str,
    label: &str,
    transport: Option<&str>,
) -> String {
    match transport.filter(|value| !value.trim().is_empty()) {
        Some(value) => format!("{}（{}，{}）", name, label, value.to_uppercase()),
        None => format!("{}（{}）", name, label),
    }
}

fn build_scoped_extension_plain_line_en(
    name: &str,
    label: &str,
    transport: Option<&str>,
) -> String {
    match transport.filter(|value| !value.trim().is_empty()) {
        Some(value) => format!("{} ({}, {})", name, label, value.to_uppercase()),
        None => format!("{} ({})", name, label),
    }
}

fn build_extension_display_line_zh(extension_ref: &str, transport: &str) -> String {
    build_scoped_extension_display_line_zh(extension_ref, "团队库 MCP", Some(transport))
}

fn build_extension_display_line_en(extension_ref: &str, transport: &str) -> String {
    build_scoped_extension_display_line_en(extension_ref, "team library MCP", Some(transport))
}

fn build_extension_plain_line_zh(name: &str, transport: &str) -> String {
    build_scoped_extension_plain_line_zh(name, "团队库 MCP", Some(transport))
}

fn build_extension_plain_line_en(name: &str, transport: &str) -> String {
    build_scoped_extension_plain_line_en(name, "team library MCP", Some(transport))
}

fn build_runtime_section_markdown(title: &str, items: &[String], empty_label: &str) -> String {
    let mut buf = String::new();
    buf.push_str(title);
    buf.push('\n');
    if items.is_empty() {
        buf.push_str("- 暂无\n");
        if !empty_label.is_empty() {
            buf.clear();
            buf.push_str(title);
            buf.push('\n');
            buf.push_str("- ");
            buf.push_str(empty_label);
            buf.push('\n');
        }
        return buf;
    }
    for item in items {
        buf.push_str("- ");
        buf.push_str(item);
        buf.push('\n');
    }
    buf
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

fn parse_bool(args: &serde_json::Map<String, serde_json::Value>, key: &str, default: bool) -> bool {
    args.get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(default)
}

fn looks_like_http_url(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("http://") || trimmed.starts_with("https://")
}

fn infer_transport_from_input(uri_or_cmd: &str) -> Option<String> {
    let trimmed = uri_or_cmd.trim();
    if !looks_like_http_url(trimmed) {
        return Some("stdio".to_string());
    }

    let lowered = trimmed.to_ascii_lowercase();
    if lowered.contains("/sse") {
        Some("sse".to_string())
    } else if lowered.contains("/mcp") || lowered.contains("streamable") {
        Some("streamable_http".to_string())
    } else {
        None
    }
}

fn infer_name_from_package(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let without_version = if trimmed.starts_with('@') {
        match trimmed.rfind('@') {
            Some(idx) if idx > 0 && trimmed[..idx].contains('/') => &trimmed[..idx],
            _ => trimmed,
        }
    } else {
        trimmed.split('@').next().unwrap_or(trimmed)
    };

    let mut candidate = without_version
        .rsplit('/')
        .next()
        .unwrap_or(without_version)
        .trim_start_matches('@')
        .to_string();

    if candidate.eq_ignore_ascii_case("mcp") {
        if let Some(scope) = without_version
            .split('/')
            .next()
            .map(|value| value.trim_start_matches('@'))
            .filter(|value| !value.is_empty())
        {
            candidate = scope.to_string();
        }
    }

    candidate = candidate
        .trim_start_matches("mcp-")
        .trim_start_matches("server-")
        .trim_start_matches("mcp-server-")
        .to_string();

    if candidate.is_empty() {
        None
    } else {
        Some(candidate)
    }
}

fn infer_name_from_source_url(source_url: &str) -> Option<String> {
    let trimmed = source_url.trim().trim_end_matches('/');
    let last_segment = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty())
        .next_back()?;
    infer_name_from_package(last_segment)
}

fn infer_name_from_command(
    uri_or_cmd: &str,
    args: &[String],
    source_url: Option<&str>,
) -> Option<String> {
    if let Some(first_non_flag) = args.iter().find(|value| {
        let trimmed = value.trim();
        !trimmed.is_empty() && !trimmed.starts_with('-')
    }) {
        if let Some(name) = infer_name_from_package(first_non_flag) {
            return Some(name);
        }
    }

    let command = uri_or_cmd.trim();
    if !command.is_empty() && !looks_like_http_url(command) {
        if let Some(stem) = std::path::Path::new(command)
            .file_stem()
            .and_then(|value| value.to_str())
            .and_then(infer_name_from_package)
        {
            return Some(stem);
        }
    }

    source_url.and_then(infer_name_from_source_url)
}

fn parse_shell_command(shell_command: &str) -> Result<(String, Vec<String>)> {
    let parts = shlex::split(shell_command).ok_or_else(|| {
        anyhow!("Unable to parse shell_command; please provide a valid shell command")
    })?;
    let mut iter = parts.into_iter();
    let uri_or_cmd = iter
        .next()
        .ok_or_else(|| anyhow!("shell_command must contain an executable or URL"))?;
    Ok((uri_or_cmd, iter.collect()))
}

fn build_plan_from_args(
    args: &serde_json::Map<String, serde_json::Value>,
) -> Result<PlannedMcpInstall> {
    let source_url = parse_optional_string(args, "source_url");
    let shell_command = parse_optional_string(args, "shell_command");
    let envs = parse_string_map(args, "envs")?;
    let description = parse_optional_string(args, "description");

    let (uri_or_cmd, mut parsed_args, mut notes) =
        if let Some(shell_command_value) = shell_command.as_deref() {
            let (command, command_args) = parse_shell_command(shell_command_value)?;
            (
                Some(command),
                command_args,
                vec!["已从 shell_command 解析出 uri_or_cmd 与 args。".to_string()],
            )
        } else {
            (
                parse_optional_string(args, "uri_or_cmd"),
                parse_string_array(args, "args")?,
                Vec::new(),
            )
        };

    if shell_command.is_some() && args.contains_key("args") {
        let explicit_args = parse_string_array(args, "args")?;
        if !explicit_args.is_empty() {
            parsed_args = explicit_args;
            notes.push("显式提供的 args 覆盖了从 shell_command 解析出的参数。".to_string());
        }
    }

    let transport = match parse_optional_string(args, "type") {
        Some(value) => Some(normalize_mcp_transport(&value)?),
        None => uri_or_cmd
            .as_deref()
            .and_then(infer_transport_from_input)
            .or_else(|| source_url.as_deref().and_then(infer_transport_from_input)),
    };

    if args.get("type").is_none() {
        if let Some(inferred_transport) = transport.as_deref() {
            notes.push(format!(
                "未显式提供 type，已根据命令/地址推断为 {}。",
                inferred_transport
            ));
        }
    }

    let name = parse_optional_string(args, "name").or_else(|| {
        uri_or_cmd.as_deref().and_then(|command| {
            infer_name_from_command(command, &parsed_args, source_url.as_deref())
        })
    });

    if args.get("name").is_none() {
        if let Some(inferred_name) = name.as_deref() {
            notes.push(format!("未显式提供 name，已建议使用 '{}'.", inferred_name));
        }
    }

    Ok(PlannedMcpInstall {
        name,
        transport,
        uri_or_cmd,
        args: parsed_args,
        envs,
        description,
        source_url,
        shell_command,
        notes,
    })
}

fn validate_plan(plan: &mut PlannedMcpInstall) -> Vec<String> {
    let mut missing = Vec::new();

    if plan.uri_or_cmd.is_none() {
        if let Some(source_url) = plan
            .source_url
            .as_deref()
            .filter(|value| looks_like_http_url(value))
        {
            if let Some(inferred_transport) = infer_transport_from_input(source_url) {
                plan.uri_or_cmd = Some(source_url.to_string());
                plan.transport = Some(inferred_transport.clone());
                plan.notes.push(format!(
                    "未显式提供 uri_or_cmd，已尝试把 source_url 直接作为 {} MCP 入口。",
                    inferred_transport
                ));
            }
        }
    }

    if plan.name.is_none() {
        missing.push("name".to_string());
    }
    if plan.transport.is_none() {
        missing.push("type".to_string());
    }
    if plan.uri_or_cmd.is_none() {
        missing.push("uri_or_cmd".to_string());
    }

    if let (Some(transport), Some(uri_or_cmd)) =
        (plan.transport.as_deref(), plan.uri_or_cmd.as_deref())
    {
        match transport {
            "stdio" if looks_like_http_url(uri_or_cmd) => {
                plan.notes.push(
                    "当前 type=stdio，但 uri_or_cmd 看起来是 HTTP 地址；请确认这不是 SSE 或 streamable HTTP MCP。"
                        .to_string(),
                );
            }
            "sse" | "streamable_http" if !looks_like_http_url(uri_or_cmd) => {
                plan.notes.push(format!(
                    "当前 type={}，但 uri_or_cmd 看起来不是 HTTP 地址；请确认 transport 是否正确。",
                    transport
                ));
            }
            _ => {}
        }
    }

    if plan.source_url.is_some() && plan.shell_command.is_none() && plan.uri_or_cmd.is_none() {
        plan.notes.push(
            "source_url 目前只作为安装来源线索；若网页不是直接 MCP 入口，请先用现有网页阅读能力提取真实命令或服务地址。"
                .to_string(),
        );
    }

    missing
}

fn build_install_payload(
    plan: &PlannedMcpInstall,
    attach_agent_ids: &[String],
) -> Option<serde_json::Value> {
    Some(json!({
        "name": plan.name.as_ref()?,
        "type": plan.transport.as_ref()?,
        "uri_or_cmd": plan.uri_or_cmd.as_ref()?,
        "args": plan.args,
        "envs": plan.envs,
        "description": plan.description,
        "attach_agent_ids": attach_agent_ids,
    }))
}

pub struct TeamMcpToolsProvider {
    db: Arc<MongoDb>,
    team_id: String,
    actor_user_id: String,
    current_agent_id: Option<String>,
    info: InitializeResult,
}

impl TeamMcpToolsProvider {
    pub fn new(
        db: Arc<MongoDb>,
        team_id: String,
        actor_user_id: String,
        current_agent_id: Option<String>,
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
                name: "team_mcp".to_string(),
                title: Some("Team MCP".to_string()),
                version: "1.0.0".to_string(),
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Use these tools for the formal MCP lifecycle: search templates, inspect installed team MCP resources, inspect the current runtime capability scope of a team agent, read webpages/READMEs with existing tools if needed, normalize the install plan with plan_install_team_mcp, then formally install into the team extension library and optionally attach to specific agents. For update/remove requests, inspect the installed MCP list first so the exact target is confirmed before changing or removing anything. Never describe workspace-only clone/npm installs as system installation."
                    .to_string(),
            ),
        };
        Self {
            db,
            team_id,
            actor_user_id,
            current_agent_id,
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
            Err(anyhow!(
                "Current user is not allowed to manage team MCP resources"
            ))
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
            || template
                .description_zh
                .to_ascii_lowercase()
                .contains(&lowered)
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
            "display_line_en": build_extension_display_line_en(&ext_ref, &extension.extension_type),
            "plain_line_zh": build_extension_plain_line_zh(&extension.name, &extension.extension_type),
            "plain_line_en": build_extension_plain_line_en(&extension.name, &extension.extension_type),
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

    async fn list_all_team_agents(&self) -> Result<Vec<TeamAgent>> {
        let service = self.agent_service();
        let mut page = 1u32;
        let limit = 100u32;
        let mut items = Vec::new();

        loop {
            let response = service
                .list_agents(ListAgentsQuery {
                    team_id: self.team_id.clone(),
                    page,
                    limit,
                })
                .await
                .map_err(|e| anyhow!("Failed to list team agents: {}", e))?;
            let count = response.items.len();
            items.extend(response.items);
            if count < limit as usize {
                break;
            }
            page += 1;
        }

        Ok(items)
    }

    async fn resolve_agent_in_team(&self, agent_id_or_name: &str) -> Result<TeamAgent> {
        let needle = agent_id_or_name.trim();
        if needle.is_empty() {
            return Err(anyhow!("Missing required field 'agent_id_or_name'"));
        }

        let service = self.agent_service();
        if let Some(agent) = service.get_agent(needle).await? {
            if agent.team_id == self.team_id {
                return Ok(agent);
            }
        }

        let agents = self.list_all_team_agents().await?;
        agents
            .into_iter()
            .find(|agent| {
                agent.id == needle
                    || agent.name.eq_ignore_ascii_case(needle)
                    || agent
                        .model
                        .as_deref()
                        .map(|model| model.eq_ignore_ascii_case(needle))
                        .unwrap_or(false)
            })
            .ok_or_else(|| anyhow!("Team agent '{}' not found", needle))
    }

    fn is_current_agent_alias(value: &str) -> bool {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "" | "current"
                | "current_agent"
                | "current agent"
                | "this agent"
                | "this_agent"
                | "当前"
                | "当前agent"
                | "当前 agent"
                | "本agent"
                | "本 agent"
                | "本体"
        )
    }

    async fn resolve_current_agent(&self) -> Result<TeamAgent> {
        let current_agent_id = self.current_agent_id.as_deref().ok_or_else(|| {
            anyhow!("Current session agent is unavailable; please provide agent_id_or_name")
        })?;
        self.resolve_agent_in_team(current_agent_id).await
    }

    async fn resolve_runtime_target_agent(
        &self,
        requested: Option<&str>,
    ) -> Result<(TeamAgent, Option<String>, String)> {
        match requested.map(str::trim).filter(|value| !value.is_empty()) {
            None => Ok((
                self.resolve_current_agent().await?,
                None,
                "current_session_agent".to_string(),
            )),
            Some(value) if Self::is_current_agent_alias(value) => Ok((
                self.resolve_current_agent().await?,
                Some(value.to_string()),
                "current_session_agent_alias".to_string(),
            )),
            Some(value) => match self.resolve_agent_in_team(value).await {
                Ok(agent) => Ok((agent, Some(value.to_string()), "explicit_match".to_string())),
                Err(_) if self.current_agent_id.is_some() => Ok((
                    self.resolve_current_agent().await?,
                    Some(value.to_string()),
                    "fallback_to_current_session_agent".to_string(),
                )),
                Err(error) => Err(error),
            },
        }
    }

    fn builtin_display_name(extension: BuiltinExtension) -> &'static str {
        match extension {
            BuiltinExtension::Skills => "Skills",
            BuiltinExtension::SkillRegistry => "Skill Registry",
            BuiltinExtension::Tasks => "Tasks",
            BuiltinExtension::ExtensionManager => "Extension Manager",
            BuiltinExtension::Team => "Team",
            BuiltinExtension::ChatRecall => "Chat Recall",
            BuiltinExtension::DocumentTools => "Document Tools",
            BuiltinExtension::Developer => "Developer",
            BuiltinExtension::Memory => "Memory",
            BuiltinExtension::ComputerController => "Computer Controller",
            BuiltinExtension::AutoVisualiser => "Auto Visualiser",
            BuiltinExtension::Tutorial => "Tutorial",
        }
    }

    fn builtin_runtime_payload(extension: BuiltinExtension) -> serde_json::Value {
        let (runtime_status, runtime_kind, runtime_binding, note, available_in_team_runtime) =
            match extension {
                BuiltinExtension::Skills => (
                    "active",
                    "platform",
                    "team_skills",
                    "已在 team server 运行时重映射为 team_skills；用于当前团队技能查询与加载。",
                    true,
                ),
                BuiltinExtension::SkillRegistry => (
                    "active",
                    "platform",
                    "skill_registry",
                    "作为内置平台能力可直接提供 skills.sh / registry 查询与导入能力。",
                    true,
                ),
                BuiltinExtension::Tasks => (
                    "active",
                    "platform",
                    "tasks",
                    "作为内置平台能力注入当前运行时，用于结构化任务跟踪。",
                    true,
                ),
                BuiltinExtension::DocumentTools => (
                    "active",
                    "platform",
                    "document_tools",
                    "作为内置平台能力注入当前运行时。",
                    true,
                ),
                BuiltinExtension::Developer => (
                    "active",
                    "builtin_mcp",
                    "developer",
                    "作为内置 MCP 能力可读写文件并执行 shell 命令。",
                    true,
                ),
                BuiltinExtension::Memory => (
                    "active",
                    "builtin_mcp",
                    "memory",
                    "作为内置 MCP 能力注入当前运行时。",
                    true,
                ),
                BuiltinExtension::ComputerController => (
                    "active",
                    "builtin_mcp",
                    "computer_controller",
                    "作为内置 MCP 能力注入当前运行时；底层命令名为 computercontroller。",
                    true,
                ),
                BuiltinExtension::AutoVisualiser => (
                    "active",
                    "builtin_mcp",
                    "auto_visualiser",
                    "作为内置 MCP 能力注入当前运行时；底层命令名为 autovisualiser。",
                    true,
                ),
                BuiltinExtension::Tutorial => (
                    "active",
                    "builtin_mcp",
                    "tutorial",
                    "作为内置 MCP 能力注入当前运行时。",
                    true,
                ),
                BuiltinExtension::ExtensionManager => (
                    "blocked_legacy",
                    "legacy_platform",
                    "extension_manager",
                    "当前 team server 运行时会屏蔽这个遗留能力，不能把它当成当前可用工具。",
                    false,
                ),
                BuiltinExtension::Team => (
                    "blocked_legacy",
                    "legacy_platform",
                    "team",
                    "当前 team server 运行时会屏蔽旧 team 扩展链；不要把 team_list_installed 当成当前能力总览。",
                    false,
                ),
                BuiltinExtension::ChatRecall => (
                    "blocked_legacy",
                    "legacy_platform",
                    "chat_recall",
                    "当前 team server 运行时会屏蔽这个遗留能力，不应视为当前可直接调用的工具。",
                    false,
                ),
            };

        let display_name = Self::builtin_display_name(extension);
        let ext_ref = build_extension_ref(
            &format!("builtin:{}", runtime_binding),
            display_name,
            "builtin",
            runtime_binding,
        );
        let (display_line_zh, display_line_en, plain_line_zh, plain_line_en) = if runtime_status
            == "blocked_legacy"
        {
            (
                build_scoped_extension_display_line_zh(&ext_ref, "遗留能力，已屏蔽", None),
                build_scoped_extension_display_line_en(&ext_ref, "blocked legacy capability", None),
                build_scoped_extension_plain_line_zh(display_name, "遗留能力，已屏蔽", None),
                build_scoped_extension_plain_line_en(
                    display_name,
                    "blocked legacy capability",
                    None,
                ),
            )
        } else {
            (
                build_scoped_extension_display_line_zh(&ext_ref, "内置能力", None),
                build_scoped_extension_display_line_en(&ext_ref, "built-in capability", None),
                build_scoped_extension_plain_line_zh(display_name, "内置能力", None),
                build_scoped_extension_plain_line_en(display_name, "built-in capability", None),
            )
        };

        json!({
            "name": extension.name(),
            "display_name": display_name,
            "description": extension.description(),
            "is_platform": extension.is_platform(),
            "mcp_command_name": extension.mcp_name(),
            "ext_ref": ext_ref,
            "display_line_zh": display_line_zh,
            "display_line_en": display_line_en,
            "plain_line_zh": plain_line_zh,
            "plain_line_en": plain_line_en,
            "runtime_status": runtime_status,
            "runtime_kind": runtime_kind,
            "runtime_binding": runtime_binding,
            "available_in_current_team_runtime": available_in_team_runtime,
            "note": note,
        })
    }

    fn attached_extension_payload(extension: &CustomExtensionConfig) -> serde_json::Value {
        let source_kind = if extension.source.as_deref() == Some("team")
            || extension.source_extension_id.is_some()
        {
            "team"
        } else {
            "custom"
        };
        let extension_class = if source_kind == "team" {
            "mcp"
        } else {
            "custom"
        };
        let extension_id = if source_kind == "team" {
            extension
                .source_extension_id
                .as_deref()
                .map(|id| format!("team:{}", id))
                .unwrap_or_else(|| format!("team:{}", extension.name))
        } else {
            format!("custom:{}", extension.name)
        };
        let ext_ref = build_extension_ref(
            &extension_id,
            &extension.name,
            extension_class,
            &extension.ext_type,
        );
        let (display_line_zh, display_line_en, plain_line_zh, plain_line_en) =
            if source_kind == "team" {
                (
                    build_scoped_extension_display_line_zh(
                        &ext_ref,
                        "已挂载 MCP",
                        Some(&extension.ext_type),
                    ),
                    build_scoped_extension_display_line_en(
                        &ext_ref,
                        "attached MCP",
                        Some(&extension.ext_type),
                    ),
                    build_scoped_extension_plain_line_zh(
                        &extension.name,
                        "已挂载 MCP",
                        Some(&extension.ext_type),
                    ),
                    build_scoped_extension_plain_line_en(
                        &extension.name,
                        "attached MCP",
                        Some(&extension.ext_type),
                    ),
                )
            } else {
                (
                    build_scoped_extension_display_line_zh(
                        &ext_ref,
                        "已挂载自定义扩展",
                        Some(&extension.ext_type),
                    ),
                    build_scoped_extension_display_line_en(
                        &ext_ref,
                        "attached custom extension",
                        Some(&extension.ext_type),
                    ),
                    build_scoped_extension_plain_line_zh(
                        &extension.name,
                        "已挂载自定义扩展",
                        Some(&extension.ext_type),
                    ),
                    build_scoped_extension_plain_line_en(
                        &extension.name,
                        "attached custom extension",
                        Some(&extension.ext_type),
                    ),
                )
            };

        json!({
            "name": extension.name,
            "type": extension.ext_type,
            "uri_or_cmd": extension.uri_or_cmd,
            "args": extension.args,
            "enabled": extension.enabled,
            "source": extension.source,
            "source_extension_id": extension.source_extension_id,
            "source_kind": source_kind,
            "ext_ref": ext_ref,
            "display_line_zh": display_line_zh,
            "display_line_en": display_line_en,
            "plain_line_zh": plain_line_zh,
            "plain_line_en": plain_line_en,
        })
    }

    fn attached_team_extension_ref_payload(
        extension: &agime_team::models::AttachedTeamExtensionRef,
    ) -> serde_json::Value {
        let display_name = extension
            .display_name
            .as_deref()
            .or(extension.runtime_name.as_deref())
            .unwrap_or("Team MCP");
        let transport = extension.transport.as_deref().unwrap_or("mcp");
        let ext_ref = build_extension_ref(
            &format!("team:{}", extension.extension_id),
            display_name,
            "mcp",
            transport,
        );
        json!({
            "id": extension.extension_id,
            "name": display_name,
            "type": transport,
            "enabled": extension.enabled,
            "source_kind": "team",
            "ext_ref": ext_ref,
            "display_line_zh": build_scoped_extension_display_line_zh(&ext_ref, "已挂载 MCP", Some(transport)),
            "display_line_en": build_scoped_extension_display_line_en(&ext_ref, "attached MCP", Some(transport)),
            "plain_line_zh": build_scoped_extension_plain_line_zh(display_name, "已挂载 MCP", Some(transport)),
            "plain_line_en": build_scoped_extension_plain_line_en(display_name, "attached MCP", Some(transport)),
        })
    }

    fn builtin_runtime_payload_from_snapshot(
        capability: &ConfiguredBuiltinCapability,
    ) -> serde_json::Value {
        let registry = builtin_registry_entry(capability.extension);
        let primary_runtime_name = registry
            .runtime_names
            .first()
            .cloned()
            .unwrap_or_else(|| capability.extension.name().to_string());
        let note = match capability.extension {
            BuiltinExtension::Skills => {
                "已在 team server 运行时重映射为 team_skills；用于当前团队技能查询与加载。"
            }
            BuiltinExtension::SkillRegistry => {
                "作为内置平台能力可直接提供 skills.sh / registry 查询与导入能力。"
            }
            BuiltinExtension::Tasks | BuiltinExtension::DocumentTools => {
                "作为内置平台能力注入当前运行时。"
            }
            BuiltinExtension::Developer => {
                "当前由 team server 以内置能力方式提供文件编辑与 shell 能力。"
            }
            BuiltinExtension::Memory
            | BuiltinExtension::ComputerController
            | BuiltinExtension::AutoVisualiser
            | BuiltinExtension::Tutorial => "作为内置 MCP 能力注入当前运行时。",
            BuiltinExtension::ExtensionManager
            | BuiltinExtension::Team
            | BuiltinExtension::ChatRecall => {
                "该能力属于系统保留/注入能力，不作为普通可编辑扩展暴露。"
            }
        };
        json!({
            "name": capability.extension.name(),
            "display_name": registry.display_name,
            "runtime_status": if capability.enabled { "active" } else { "disabled" },
            "runtime_kind": match registry.kind {
                super::capability_policy::CapabilityKind::BuiltinPlatform => "platform",
                super::capability_policy::CapabilityKind::BuiltinMcp => "builtin_mcp",
                super::capability_policy::CapabilityKind::SystemReserved => "system_reserved",
                _ => "builtin",
            },
            "runtime_delivery": match registry.runtime_delivery {
                super::capability_policy::RuntimeDelivery::InProcess => "in_process",
                super::capability_policy::RuntimeDelivery::SubprocessMcp => "subprocess_mcp",
                super::capability_policy::RuntimeDelivery::SessionInjected => "session_injected",
            },
            "runtime_binding": primary_runtime_name,
            "note": note,
            "available_in_team_runtime": capability.enabled && registry.editable,
            "editable": registry.editable,
            "display_line_zh": build_scoped_extension_display_line_zh(
                &build_extension_ref(
                    &format!("builtin:{}", primary_runtime_name),
                    &registry.display_name,
                    "builtin",
                    &primary_runtime_name
                ),
                "内置能力",
                Some(&primary_runtime_name),
            ),
            "display_line_en": build_scoped_extension_display_line_en(
                &build_extension_ref(
                    &format!("builtin:{}", primary_runtime_name),
                    &registry.display_name,
                    "builtin",
                    &primary_runtime_name
                ),
                "built-in capability",
                Some(&primary_runtime_name),
            ),
            "plain_line_zh": build_scoped_extension_plain_line_zh(&registry.display_name, "内置能力", Some(&primary_runtime_name)),
            "plain_line_en": build_scoped_extension_plain_line_en(&registry.display_name, "built-in capability", Some(&primary_runtime_name)),
        })
    }

    async fn handle_inspect_runtime_capabilities(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<String> {
        let agent_id_or_name = parse_optional_string(args, "agent_id_or_name");
        let include_team_library = parse_bool(args, "include_team_library", true);
        let (agent, requested_target, resolution_mode) = self
            .resolve_runtime_target_agent(agent_id_or_name.as_deref())
            .await?;
        let runtime_snapshot = AgentRuntimePolicyResolver::resolve(&agent, None, None);
        let current_user_can_manage = self
            .agent_service()
            .is_team_admin(&self.actor_user_id, &self.team_id)
            .await
            .map_err(|e| anyhow!("Failed to verify team admin permission: {}", e))?;

        let enabled_builtin_capabilities = runtime_snapshot
            .extensions
            .builtin_capabilities
            .iter()
            .filter(|extension| extension.enabled && extension.registry.editable)
            .map(Self::builtin_runtime_payload_from_snapshot)
            .collect::<Vec<_>>();

        let attached_team_mcps = runtime_snapshot
            .extensions
            .attached_team_extensions
            .iter()
            .filter(|extension| extension.enabled)
            .map(Self::attached_team_extension_ref_payload)
            .collect::<Vec<_>>();

        let attached_custom_extensions = runtime_snapshot
            .extensions
            .custom_extensions
            .iter()
            .map(Self::attached_extension_payload)
            .collect::<Vec<_>>();

        let team_library_mcp = if include_team_library {
            let extensions = self
                .extension_service()
                .list_active_for_team(&self.team_id)
                .await?;
            let all_agents = self.list_all_team_agents().await?;
            extensions
                .into_iter()
                .filter(|extension| MCP_TYPES.contains(&extension.extension_type.as_str()))
                .map(|extension| {
                    let extension_id = extension.id.map(|id| id.to_hex()).unwrap_or_default();
                    let ext_ref = build_extension_ref(
                        &format!("team:{}", extension_id),
                        &extension.name,
                        "mcp",
                        &extension.extension_type,
                    );
                    let attached_count = all_agents
                        .iter()
                        .filter(|candidate| {
                            candidate
                                .attached_team_extensions
                                .iter()
                                .any(|custom| custom.enabled && custom.extension_id == extension_id)
                                || candidate.custom_extensions.iter().any(|custom| {
                                    custom.source_extension_id.as_deref()
                                        == Some(extension_id.as_str())
                                })
                        })
                        .count();
                    json!({
                        "id": extension_id,
                        "name": extension.name,
                        "transport": extension.extension_type,
                        "ext_ref": ext_ref,
                        "display_line_zh": build_extension_display_line_zh(&ext_ref, &extension.extension_type),
                        "display_line_en": build_extension_display_line_en(&ext_ref, &extension.extension_type),
                        "plain_line_zh": build_extension_plain_line_zh(&extension.name, &extension.extension_type),
                        "plain_line_en": build_extension_plain_line_en(&extension.name, &extension.extension_type),
                        "version": extension.version,
                        "attached_count": attached_count,
                        "attached_to_target_agent": runtime_snapshot
                            .extensions
                            .attached_team_extensions
                            .iter()
                            .any(|custom| {
                                custom.enabled && custom.extension_id == extension_id
                            }),
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let enabled_builtin_capability_count = enabled_builtin_capabilities.len();
        let attached_team_mcp_count = attached_team_mcps.len();
        let attached_custom_extension_count = attached_custom_extensions.len();
        let team_library_mcp_count = team_library_mcp.len();
        let tasks_enabled = runtime_snapshot
            .extensions
            .effective_allowed_extension_names
            .iter()
            .any(|name| name == "tasks");
        let prompt_introspection = build_prompt_introspection_snapshot(
            &runtime_snapshot,
            agent.model.as_deref().unwrap_or_default(),
            runtime_snapshot.delegation_policy.require_final_report,
        );
        let harness_capabilities = &prompt_introspection.harness_capabilities;
        let harness_runtime_capabilities = vec![
            json!({
                "name": "Tasks",
                "display_line_zh": format!("Tasks（任务板） · {}", if harness_capabilities.tasks_enabled { "当前会话可用" } else { "当前会话不可用" }),
                "display_line_en": format!("Tasks · {}", if harness_capabilities.tasks_enabled { "available in this session" } else { "disabled in this session" }),
                "plain_line_zh": format!("Tasks（任务板）：{}", if harness_capabilities.tasks_enabled { "当前会话可用" } else { "当前会话不可用" }),
                "plain_line_en": format!("Tasks: {}", if harness_capabilities.tasks_enabled { "available in this session" } else { "disabled in this session" }),
            }),
            json!({
                "name": "Plan Mode",
                "display_line_zh": format!("Plan Mode · {}", if harness_capabilities.plan_enabled { "允许" } else { "关闭" }),
                "display_line_en": format!("Plan Mode · {}", if harness_capabilities.plan_enabled { "enabled" } else { "disabled" }),
                "plain_line_zh": format!("Plan Mode：{}", if harness_capabilities.plan_enabled { "允许" } else { "关闭" }),
                "plain_line_en": format!("Plan Mode: {}", if harness_capabilities.plan_enabled { "enabled" } else { "disabled" }),
            }),
            json!({
                "name": "Subagent",
                "display_line_zh": format!("Subagent · {}", if harness_capabilities.subagent_enabled { "允许委派子任务" } else { "当前不可用" }),
                "display_line_en": format!("Subagent · {}", if harness_capabilities.subagent_enabled { "bounded delegation available" } else { "disabled in this session" }),
                "plain_line_zh": format!("Subagent：{}", if harness_capabilities.subagent_enabled { "允许委派子任务" } else { "当前不可用" }),
                "plain_line_en": format!("Subagent: {}", if harness_capabilities.subagent_enabled { "bounded delegation available" } else { "disabled in this session" }),
            }),
            json!({
                "name": "Swarm",
                "display_line_zh": format!("Swarm · {}", if harness_capabilities.swarm_enabled { "显式并行 worker 可用" } else { "显式 swarm 当前不可用" }),
                "display_line_en": format!("Swarm · {}", if harness_capabilities.swarm_enabled { "explicit multi-worker fan-out available" } else { "explicit swarm disabled in this session" }),
                "plain_line_zh": format!("Swarm：{}", if harness_capabilities.swarm_enabled { "显式并行 worker 可用" } else { "显式 swarm 当前不可用" }),
                "plain_line_en": format!("Swarm: {}", if harness_capabilities.swarm_enabled { "explicit multi-worker fan-out available" } else { "explicit swarm disabled in this session" }),
            }),
            json!({
                "name": "Worker Messaging",
                "display_line_zh": format!("Worker Messaging · {}", if harness_capabilities.worker_peer_messaging_enabled { "worker 间可直接互发消息" } else { "worker 互发消息未启用" }),
                "display_line_en": format!("Worker Messaging · {}", if harness_capabilities.worker_peer_messaging_enabled { "workers may directly message each other" } else { "worker peer messaging disabled" }),
                "plain_line_zh": format!("Worker Messaging：{}", if harness_capabilities.worker_peer_messaging_enabled { "worker 间可直接互发消息" } else { "worker 互发消息未启用" }),
                "plain_line_en": format!("Worker Messaging: {}", if harness_capabilities.worker_peer_messaging_enabled { "workers may directly message each other" } else { "worker peer messaging disabled" }),
            }),
            json!({
                "name": "Auto Swarm",
                "display_line_zh": format!("Auto Swarm · {}", if harness_capabilities.auto_swarm_enabled { "运行时可自动升级为 swarm" } else { "未启用" }),
                "display_line_en": format!("Auto Swarm · {}", if harness_capabilities.auto_swarm_enabled { "runtime may auto-upgrade suitable work" } else { "disabled" }),
                "plain_line_zh": format!("Auto Swarm：{}", if harness_capabilities.auto_swarm_enabled { "运行时可自动升级为 swarm" } else { "未启用" }),
                "plain_line_en": format!("Auto Swarm: {}", if harness_capabilities.auto_swarm_enabled { "runtime may auto-upgrade suitable work" } else { "disabled" }),
            }),
            json!({
                "name": "Validation Worker",
                "display_line_zh": format!("Validation Worker · {}", if harness_capabilities.validation_worker_enabled { "允许" } else { "关闭" }),
                "display_line_en": format!("Validation Worker · {}", if harness_capabilities.validation_worker_enabled { "enabled" } else { "disabled" }),
                "plain_line_zh": format!("Validation Worker：{}", if harness_capabilities.validation_worker_enabled { "允许" } else { "关闭" }),
                "plain_line_en": format!("Validation Worker: {}", if harness_capabilities.validation_worker_enabled { "enabled" } else { "disabled" }),
            }),
            json!({
                "name": "Approval Mode",
                "display_line_zh": format!("Approval Mode · {}", match harness_capabilities.approval_mode {
                    agime_team::models::ApprovalMode::LeaderOwned => "leader-owned",
                    agime_team::models::ApprovalMode::HeadlessFallback => "headless fallback",
                }),
                "display_line_en": format!("Approval Mode · {}", match harness_capabilities.approval_mode {
                    agime_team::models::ApprovalMode::LeaderOwned => "leader-owned",
                    agime_team::models::ApprovalMode::HeadlessFallback => "headless fallback",
                }),
                "plain_line_zh": format!("Approval Mode：{}", match harness_capabilities.approval_mode {
                    agime_team::models::ApprovalMode::LeaderOwned => "leader-owned",
                    agime_team::models::ApprovalMode::HeadlessFallback => "headless fallback",
                }),
                "plain_line_en": format!("Approval Mode: {}", match harness_capabilities.approval_mode {
                    agime_team::models::ApprovalMode::LeaderOwned => "leader-owned",
                    agime_team::models::ApprovalMode::HeadlessFallback => "headless fallback",
                }),
            }),
        ];

        let session_injected_management_tools = vec![json!({
            "name": "team_mcp",
            "display_name": "Team MCP",
            "ext_ref": build_extension_ref("builtin:team_mcp", "Team MCP", "builtin", "management"),
            "display_line_zh": build_scoped_extension_display_line_zh(
                &build_extension_ref("builtin:team_mcp", "Team MCP", "builtin", "management"),
                "管理工具",
                None,
            ),
            "display_line_en": build_scoped_extension_display_line_en(
                &build_extension_ref("builtin:team_mcp", "Team MCP", "builtin", "management"),
                "management tool",
                None,
            ),
            "plain_line_zh": build_scoped_extension_plain_line_zh("Team MCP", "管理工具", None),
            "plain_line_en": build_scoped_extension_plain_line_en("Team MCP", "management tool", None),
            "available_in_team_context_sessions": true,
            "requires_team_admin_for_mutations": true,
            "note": "这是团队上下文会话里自动注入的正式 MCP 管理工具；它本身不是团队扩展库里的一条 MCP 记录。"
        })];

        let section_specs = [
            (
                "管理工具",
                "Management Tools",
                &session_injected_management_tools,
            ),
            (
                "内置能力",
                "Built-in Capabilities",
                &enabled_builtin_capabilities,
            ),
            (
                "Harness 执行能力",
                "Harness Execution Powers",
                &harness_runtime_capabilities,
            ),
            ("已挂载 MCP", "Attached MCPs", &attached_team_mcps),
            (
                "已挂载自定义扩展",
                "Attached Custom Extensions",
                &attached_custom_extensions,
            ),
            ("团队库 MCP", "Team Library MCPs", &team_library_mcp),
        ];

        let render_ready_sections_zh = section_specs
            .iter()
            .map(|(title_zh, _, items)| {
                let display_lines = items
                    .iter()
                    .filter_map(|item| item.get("display_line_zh").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>();
                let plain_lines = items
                    .iter()
                    .filter_map(|item| item.get("plain_line_zh").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>();
                json!({
                    "title": title_zh,
                    "count": items.len(),
                    "display_lines": display_lines,
                    "plain_lines": plain_lines,
                })
            })
            .collect::<Vec<_>>();

        let render_ready_sections_en = section_specs
            .iter()
            .map(|(_, title_en, items)| {
                let display_lines = items
                    .iter()
                    .filter_map(|item| item.get("display_line_en").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>();
                let plain_lines = items
                    .iter()
                    .filter_map(|item| item.get("plain_line_en").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>();
                json!({
                    "title": title_en,
                    "count": items.len(),
                    "display_lines": display_lines,
                    "plain_lines": plain_lines,
                })
            })
            .collect::<Vec<_>>();

        let render_ready_markdown_zh = [
            build_runtime_section_markdown(
                "管理工具",
                &session_injected_management_tools
                    .iter()
                    .filter_map(|item| item.get("display_line_zh").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>(),
                "暂无",
            ),
            build_runtime_section_markdown(
                "内置能力",
                &enabled_builtin_capabilities
                    .iter()
                    .filter_map(|item| item.get("display_line_zh").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>(),
                "暂无",
            ),
            build_runtime_section_markdown(
                "Harness 执行能力",
                &harness_runtime_capabilities
                    .iter()
                    .filter_map(|item| item.get("display_line_zh").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>(),
                "暂无",
            ),
            build_runtime_section_markdown(
                "已挂载 MCP",
                &attached_team_mcps
                    .iter()
                    .filter_map(|item| item.get("display_line_zh").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>(),
                "暂无",
            ),
            build_runtime_section_markdown(
                "已挂载自定义扩展",
                &attached_custom_extensions
                    .iter()
                    .filter_map(|item| item.get("display_line_zh").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>(),
                "暂无",
            ),
            build_runtime_section_markdown(
                "团队库 MCP",
                &team_library_mcp
                    .iter()
                    .filter_map(|item| item.get("display_line_zh").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>(),
                "暂无",
            ),
        ]
        .join("\n");

        let render_ready_markdown_en = [
            build_runtime_section_markdown(
                "Management Tools",
                &session_injected_management_tools
                    .iter()
                    .filter_map(|item| item.get("display_line_en").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>(),
                "None",
            ),
            build_runtime_section_markdown(
                "Built-in Capabilities",
                &enabled_builtin_capabilities
                    .iter()
                    .filter_map(|item| item.get("display_line_en").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>(),
                "None",
            ),
            build_runtime_section_markdown(
                "Harness Execution Powers",
                &harness_runtime_capabilities
                    .iter()
                    .filter_map(|item| item.get("display_line_en").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>(),
                "None",
            ),
            build_runtime_section_markdown(
                "Attached MCPs",
                &attached_team_mcps
                    .iter()
                    .filter_map(|item| item.get("display_line_en").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>(),
                "None",
            ),
            build_runtime_section_markdown(
                "Attached Custom Extensions",
                &attached_custom_extensions
                    .iter()
                    .filter_map(|item| item.get("display_line_en").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>(),
                "None",
            ),
            build_runtime_section_markdown(
                "Team Library MCPs",
                &team_library_mcp
                    .iter()
                    .filter_map(|item| item.get("display_line_en").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>(),
                "None",
            ),
        ]
        .join("\n");

        Ok(serde_json::to_string_pretty(&json!({
            "target_agent": {
                "agent_id": agent.id,
                "name": agent.name,
                "model": agent.model,
                "team_id": agent.team_id,
                "skill_binding_mode": runtime_snapshot.skills.skill_binding_mode,
                "delegation_policy": runtime_snapshot.delegation_policy,
            },
            "prompt_snapshot_version": prompt_introspection.prompt_snapshot_version,
            "capability_snapshot": prompt_introspection.capability_snapshot,
            "delegation_snapshot": prompt_introspection.delegation_snapshot,
            "harness_capabilities": prompt_introspection.harness_capabilities,
            "tasks_enabled": prompt_introspection.tasks_enabled,
            "task_visibility_scope": if tasks_enabled { "capability_enabled" } else { "disabled" },
            "subagent_enabled": prompt_introspection.subagent_enabled,
            "swarm_enabled": prompt_introspection.swarm_enabled,
            "worker_peer_messaging_enabled": prompt_introspection.worker_peer_messaging_enabled,
            "validation_worker_enabled": prompt_introspection.validation_worker_enabled,
            "approval_mode": prompt_introspection.approval_mode,
            "resolution": {
                "requested_agent": requested_target,
                "current_session_agent_id": self.current_agent_id.clone(),
                "mode": resolution_mode,
            },
            "runtime_snapshot": runtime_snapshot,
            "current_user": {
                "actor_user_id": self.actor_user_id,
                "can_manage_team_mcp": current_user_can_manage,
            },
            "session_injected_management_tools": session_injected_management_tools,
            "enabled_builtin_capabilities": enabled_builtin_capabilities,
            "attached_team_mcps": attached_team_mcps,
            "attached_custom_extensions": attached_custom_extensions,
            "team_library_mcp": team_library_mcp,
            "render_ready_sections_zh": render_ready_sections_zh,
            "render_ready_sections_en": render_ready_sections_en,
            "render_ready_markdown_zh": render_ready_markdown_zh,
            "render_ready_markdown_en": render_ready_markdown_en,
            "summary": {
                "enabled_builtin_capability_count": enabled_builtin_capability_count,
                "attached_team_mcp_count": attached_team_mcp_count,
                "attached_custom_extension_count": attached_custom_extension_count,
                "team_library_mcp_count": team_library_mcp_count,
            },
            "guidance": "team_library_mcp 只代表团队扩展库里正式安装的 MCP 资源；它不等于当前 Agent 的全部可用能力。当前输出还额外包含 Harness 执行能力（如 tasks / subagent / swarm / validation worker）。若用户正在看能力清单，优先逐条使用 render_ready_sections_zh / render_ready_markdown_zh 里的 display_line_zh 原样输出；只有在正文解释、原因分析或泛化说明里，才改用 plain_line_zh。",
        }))?)
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
            "guidance": "从模板挑选后，可先调用 plan_install_team_mcp 校验安装计划，再调用 install_team_mcp 正式写入团队扩展库；若需要立即给某个 Agent 使用，再传 attach_agent_ids。"
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
        let extensions = ext_service.list_active_for_team(&self.team_id).await?;
        let all_agents = if include_attached {
            self.list_all_team_agents().await?
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
            "guidance": "这些结果只代表团队扩展库中的正式 MCP 资源，不等于当前 Agent 的全部运行时能力。若要查看某个 Agent 当前启用的内置能力、已挂载 team/custom MCP 与团队扩展库的区别，请调用 inspect_runtime_capabilities；若要给具体 Agent 使用，请调用 attach_team_mcp；若要删除正式资源，请调用 remove_team_mcp（默认会先摘除已挂载 Agent）。"
        }))?)
    }

    async fn handle_plan_install_team_mcp(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<String> {
        self.ensure_admin().await?;

        let attach_agent_ids = parse_string_array(args, "attach_agent_ids")?;
        let mut plan = build_plan_from_args(args)?;
        let missing_fields = validate_plan(&mut plan);
        let ready_to_install = missing_fields.is_empty();
        let install_payload = build_install_payload(&plan, &attach_agent_ids);

        Ok(serde_json::to_string_pretty(&json!({
            "ready_to_install": ready_to_install,
            "missing_fields": missing_fields,
            "normalized": {
                "name": plan.name,
                "type": plan.transport,
                "uri_or_cmd": plan.uri_or_cmd,
                "args": plan.args,
                "envs": plan.envs,
                "description": plan.description,
                "source_url": plan.source_url,
                "shell_command": plan.shell_command,
                "attach_agent_ids": attach_agent_ids,
            },
            "install_payload": install_payload,
            "notes": plan.notes,
            "guidance": if ready_to_install {
                "安装计划已齐备。下一步直接调用 install_team_mcp；若还需要立即给某个 Agent 使用，可保留 attach_agent_ids 或在安装后再调用 attach_team_mcp。"
            } else {
                "当前还不能正式安装。请先用现有网页阅读能力（如 developer / playwright）从网页、README、命令示例中补齐 missing_fields，再重新调用 plan_install_team_mcp 校验。"
            }
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
            "display_line_en": build_extension_display_line_en(&ext_ref, &created.extension_type),
            "plain_line_zh": build_extension_plain_line_zh(&created.name, &created.extension_type),
            "plain_line_en": build_extension_plain_line_en(&created.name, &created.extension_type),
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
        let ext_ref = build_extension_ref(
            &format!("team:{}", extension_id),
            &extension.name,
            "mcp",
            &extension.extension_type,
        );
        Ok(serde_json::to_string_pretty(&json!({
            "attached": true,
            "extension_id": extension_id,
            "ext_ref": ext_ref,
            "display_line_zh": build_scoped_extension_display_line_zh(&ext_ref, "已挂载 MCP", Some(&extension.extension_type)),
            "display_line_en": build_scoped_extension_display_line_en(&ext_ref, "attached MCP", Some(&extension.extension_type)),
            "plain_line_zh": build_scoped_extension_plain_line_zh(&extension.name, "已挂载 MCP", Some(&extension.extension_type)),
            "plain_line_en": build_scoped_extension_plain_line_en(&extension.name, "attached MCP", Some(&extension.extension_type)),
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
            "display_line_en": build_extension_display_line_en(&ext_ref, &updated.extension_type),
            "plain_line_zh": build_extension_plain_line_zh(&updated.name, &updated.extension_type),
            "plain_line_en": build_extension_plain_line_en(&updated.name, &updated.extension_type),
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
        let ext_ref = build_extension_ref(
            &format!("team:{}", extension_id),
            &extension.name,
            "mcp",
            &extension.extension_type,
        );
        Ok(serde_json::to_string_pretty(&json!({
            "removed": true,
            "extension_id": extension_id,
            "name": extension.name,
            "ext_ref": ext_ref,
            "display_line_zh": build_extension_display_line_zh(&ext_ref, &extension.extension_type),
            "display_line_en": build_extension_display_line_en(&ext_ref, &extension.extension_type),
            "plain_line_zh": build_extension_plain_line_zh(&extension.name, &extension.extension_type),
            "plain_line_en": build_extension_plain_line_en(&extension.name, &extension.extension_type),
            "detached_count": detached_agents.len(),
            "detached_agents": detached_agents,
            "guidance": if detach_attached {
                "正式 MCP 已从团队扩展库移除；此前挂载到 Agent 的副本也已一并摘除。如需确认结果，可重新调用 list_installed。"
            } else {
                "正式 MCP 已从团队扩展库移除。若还想确认剩余团队 MCP 资源，可重新调用 list_installed。"
            },
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
                name: "inspect_runtime_capabilities".into(),
                title: None,
                description: Some("Inspect the current runtime capability scope of one team agent. Separates enabled builtin capabilities, attached team/custom MCPs, session-injected Team MCP management tools, and formal MCP resources in the team library. Use this for questions like 'what can this agent use right now?'".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "agent_id_or_name": { "type": "string", "description": "Optional. Agent id, name, or model label within the current team. Defaults to the current session agent." },
                        "include_team_library": { "type": "boolean", "description": "Whether to include the formal team MCP library summary." }
                    }
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "plan_install_team_mcp".into(),
                title: None,
                description: Some("Validate and normalize a prospective MCP installation plan before formal installation. Use this after reading a webpage/README or extracting a shell command, then pass the returned install_payload into install_team_mcp.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "type": { "type": "string", "description": "stdio | sse | streamable_http" },
                        "uri_or_cmd": { "type": "string" },
                        "shell_command": { "type": "string", "description": "完整 shell 启动命令，如 `npx -y @playwright/mcp@latest --no-sandbox`。" },
                        "source_url": { "type": "string", "description": "来源网页/README/仓库地址；若它本身就是 MCP 入口，也会尝试直接推断。" },
                        "args": { "type": "array", "items": { "type": "string" } },
                        "envs": { "type": "object", "additionalProperties": { "type": "string" } },
                        "description": { "type": "string" },
                        "attach_agent_ids": { "type": "array", "items": { "type": "string" } }
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
                description: Some("Remove one formal team MCP resource from the team library. Call list_installed first to confirm the exact target; by default detach_attached=true so attached agent copies are removed before the library entry is deleted.".into()),
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
            "inspect_runtime_capabilities" => self.handle_inspect_runtime_capabilities(&args).await,
            "plan_install_team_mcp" => self.handle_plan_install_team_mcp(&args).await,
            "install_team_mcp" => self.handle_install_team_mcp(&args).await,
            "attach_team_mcp" => self.handle_attach_team_mcp(&args).await,
            "update_team_mcp" => self.handle_update_team_mcp(&args).await,
            "remove_team_mcp" => self.handle_remove_team_mcp(&args).await,
            _ => Err(anyhow!("Unknown tool: {}", name)),
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
