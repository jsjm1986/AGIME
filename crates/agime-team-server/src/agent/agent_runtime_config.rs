use agime_team::models::{
    AgentExtensionConfig, ApiFormat, AttachedTeamExtensionRef, BuiltinExtension,
    CustomExtensionConfig, TeamAgent,
};
use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::extension_installer::{AutoInstallPolicy, ExtensionInstaller};
use super::mcp_connector::ApiCaller;

/// Build an HTTP client that respects system proxy settings.
pub(crate) fn build_http_client() -> Result<reqwest::Client> {
    let mut builder = apply_reqwest_tls_backend(reqwest::Client::builder())
        .timeout(std::time::Duration::from_secs(120));

    if std::env::var("HTTPS_PROXY").is_err() && std::env::var("HTTP_PROXY").is_err() {
        if let Some(proxy_url) = detect_system_proxy() {
            tracing::info!("Using system proxy: {}", proxy_url);
            let proxy =
                reqwest::Proxy::all(&proxy_url).map_err(|e| anyhow!("Invalid proxy URL: {}", e))?;
            builder = builder.proxy(proxy);
        }
    }

    builder
        .build()
        .map_err(|e| anyhow!("Failed to build HTTP client: {}", e))
}

fn apply_reqwest_tls_backend(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    #[cfg(feature = "tls-native")]
    {
        builder.use_native_tls()
    }
    #[cfg(not(feature = "tls-native"))]
    {
        builder.use_rustls_tls()
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn detect_system_proxy() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let settings = hkcu
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
        .ok()?;
    let enabled: u32 = settings.get_value("ProxyEnable").ok()?;
    if enabled == 0 {
        return None;
    }
    let server: String = settings.get_value("ProxyServer").ok()?;
    if server.is_empty() {
        return None;
    }
    if server.starts_with("http://")
        || server.starts_with("https://")
        || server.starts_with("socks5://")
    {
        Some(server)
    } else {
        Some(format!("http://{}", server))
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn detect_system_proxy() -> Option<String> {
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TeamSkillMode {
    Assigned,
    OnDemand,
}

#[derive(Debug, Clone)]
pub(crate) struct TeamRuntimeSettings {
    pub(crate) skill_mode: TeamSkillMode,
    pub(crate) auto_install_extensions: AutoInstallPolicy,
    pub(crate) extension_cache_root: String,
    pub(crate) portal_base_url: String,
    pub(crate) workspace_root: String,
}

impl TeamRuntimeSettings {
    pub(crate) fn from_env() -> Self {
        let skill_mode = match std::env::var("TEAM_AGENT_SKILL_MODE")
            .unwrap_or_else(|_| "on_demand".to_string())
            .to_lowercase()
            .as_str()
        {
            "on_demand" | "ondemand" => TeamSkillMode::OnDemand,
            _ => TeamSkillMode::Assigned,
        };
        let auto_install_extensions = if std::env::var("TEAM_AGENT_AUTO_INSTALL_EXTENSIONS")
            .map(|s| s.eq_ignore_ascii_case("true"))
            .unwrap_or(true)
        {
            AutoInstallPolicy::Enabled
        } else {
            AutoInstallPolicy::Disabled
        };
        let extension_cache_root = std::env::var("TEAM_AGENT_EXTENSION_CACHE_ROOT")
            .unwrap_or_else(|_| "./data/runtime/extensions".to_string());
        let portal_base_url = std::env::var("PORTAL_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
        let workspace_root =
            std::env::var("WORKSPACE_ROOT").unwrap_or_else(|_| "./data/workspaces".to_string());

        Self {
            skill_mode,
            auto_install_extensions,
            extension_cache_root,
            portal_base_url,
            workspace_root,
        }
    }
}

pub(crate) fn agent_has_extension_manager_enabled(agent: &TeamAgent) -> bool {
    agent
        .enabled_extensions
        .iter()
        .any(|ext| ext.enabled && matches!(ext.extension, BuiltinExtension::ExtensionManager))
}

pub(crate) fn builtin_extension_configs_to_custom(
    enabled_extensions: &[AgentExtensionConfig],
) -> Vec<CustomExtensionConfig> {
    let agime_bin = find_agime_binary();
    let mut configs = Vec::new();

    for ext_config in enabled_extensions {
        if !ext_config.enabled {
            continue;
        }
        if ext_config.extension == BuiltinExtension::Developer {
            continue;
        }
        let Some(mcp_name) = ext_config.extension.mcp_name() else {
            tracing::debug!(
                "Skipping platform extension {:?} (not supported as subprocess)",
                ext_config.extension
            );
            continue;
        };

        if let Some(ref bin) = agime_bin {
            configs.push(CustomExtensionConfig {
                name: mcp_name.to_string(),
                ext_type: "stdio".to_string(),
                uri_or_cmd: bin.clone(),
                args: vec!["mcp".to_string(), mcp_name.to_string()],
                envs: HashMap::new(),
                enabled: true,
                source: None,
                source_extension_id: None,
            });
        } else {
            tracing::warn!(
                "Cannot start builtin extension '{}': agime binary not found",
                mcp_name
            );
        }
    }

    configs
}

fn extension_name_key(value: &str) -> String {
    value.to_ascii_lowercase().replace(['_', '-', ' '], "")
}

pub(crate) fn extension_allowed_by_name(extension_name: &str, allowed: &HashSet<String>) -> bool {
    let extension_key = extension_name_key(extension_name);
    allowed.iter().any(|item| {
        item.eq_ignore_ascii_case(extension_name) || extension_name_key(item) == extension_key
    })
}

pub(crate) fn find_extension_config_by_name(
    agent: &TeamAgent,
    name: &str,
) -> Option<CustomExtensionConfig> {
    if let Some(custom) = agent.custom_extensions.iter().find(|e| e.name == name) {
        let mut cfg = custom.clone();
        cfg.enabled = true;
        return Some(cfg);
    }

    for ext_config in &agent.enabled_extensions {
        let matches =
            ext_config.extension.name() == name || ext_config.extension.mcp_name() == Some(name);
        if matches {
            if let Some(cfg) =
                super::extension_manager_client::builtin_to_custom_config(&ext_config.extension)
            {
                return Some(cfg);
            }
        }
    }

    None
}

pub(super) fn find_agime_binary() -> Option<String> {
    if let Ok(exe) = std::env::current_exe() {
        if exe.exists() {
            let is_supported_self = exe
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| matches!(s, "agime-team-server" | "agime" | "agime-cli"))
                .unwrap_or(false);
            if is_supported_self {
                return Some(exe.to_string_lossy().to_string());
            }
        }

        if let Some(dir) = exe.parent() {
            let agime_path = dir.join(if cfg!(windows) { "agime.exe" } else { "agime" });
            if agime_path.exists() {
                return Some(agime_path.to_string_lossy().to_string());
            }
            let cli_path = dir.join(if cfg!(windows) {
                "agime-cli.exe"
            } else {
                "agime-cli"
            });
            if cli_path.exists() {
                return Some(cli_path.to_string_lossy().to_string());
            }
        }
    }

    if let Ok(output) = std::process::Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg("agime")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path.lines().next().unwrap_or(&path).to_string());
            }
        }
    }

    None
}

pub(crate) async fn resolve_agent_custom_extensions(
    db: &MongoDb,
    team_id: &str,
    custom_extensions: &[CustomExtensionConfig],
    installer: &ExtensionInstaller,
) -> Vec<CustomExtensionConfig> {
    use agime_team::services::mongo::extension_service_mongo::ExtensionService;

    let ext_service = ExtensionService::new(db.clone());
    let mut resolved = Vec::new();

    for ext in custom_extensions.iter().filter(|e| e.enabled) {
        let is_team_source =
            ext.source.as_deref() == Some("team") || ext.source_extension_id.is_some();
        if !is_team_source {
            resolved.push(ext.clone());
            continue;
        }

        let source_id = match ext.source_extension_id.as_deref() {
            Some(id) if !id.trim().is_empty() => id,
            _ => {
                tracing::warn!(
                    "Team-sourced extension '{}' has no source_extension_id, using stored config as-is",
                    ext.name
                );
                resolved.push(ext.clone());
                continue;
            }
        };

        let shared = match ext_service.get(source_id).await {
            Ok(Some(doc)) => doc,
            Ok(None) => {
                tracing::warn!(
                    "Source extension '{}' not found for '{}', using stored config as-is",
                    source_id,
                    ext.name
                );
                resolved.push(ext.clone());
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to load source extension '{}' for '{}': {}; using stored config as-is",
                    source_id,
                    ext.name,
                    e
                );
                resolved.push(ext.clone());
                continue;
            }
        };

        match installer.resolve_team_extension(team_id, &shared).await {
            Ok(cfg) => resolved.push(cfg),
            Err(e) => {
                tracing::warn!(
                    "Failed to resolve team extension '{}' ({}): {}; using stored config as-is",
                    ext.name,
                    source_id,
                    e
                );
                resolved.push(ext.clone());
            }
        }
    }

    resolved
}

pub(crate) async fn resolve_agent_attached_team_extensions(
    db: &MongoDb,
    team_id: &str,
    attached_extensions: &[AttachedTeamExtensionRef],
    legacy_team_extensions: &[CustomExtensionConfig],
    installer: &ExtensionInstaller,
) -> Vec<CustomExtensionConfig> {
    use agime_team::services::mongo::extension_service_mongo::ExtensionService;

    let ext_service = ExtensionService::new(db.clone());
    let mut resolved = Vec::new();
    let mut seen_extension_ids = HashSet::new();

    for reference in attached_extensions.iter().filter(|item| item.enabled) {
        let extension_id = reference.extension_id.trim();
        if extension_id.is_empty() || !seen_extension_ids.insert(extension_id.to_string()) {
            continue;
        }

        match ext_service.get(extension_id).await {
            Ok(Some(doc)) => match installer.resolve_team_extension(team_id, &doc).await {
                Ok(config) => resolved.push(config),
                Err(error) => tracing::warn!(
                    "Failed to resolve attached team extension '{}' ({}): {}",
                    reference.display_name.as_deref().unwrap_or(extension_id),
                    extension_id,
                    error
                ),
            },
            Ok(None) => tracing::warn!(
                "Attached team extension '{}' ({}) no longer exists",
                reference.display_name.as_deref().unwrap_or(extension_id),
                extension_id
            ),
            Err(error) => tracing::warn!(
                "Failed to load attached team extension '{}' ({}): {}",
                reference.display_name.as_deref().unwrap_or(extension_id),
                extension_id,
                error
            ),
        }
    }

    for legacy in legacy_team_extensions
        .iter()
        .filter(|extension| extension.enabled)
    {
        let Some(extension_id) = legacy.source_extension_id.as_deref() else {
            resolved.push(legacy.clone());
            continue;
        };
        if seen_extension_ids.contains(extension_id) {
            continue;
        }
        seen_extension_ids.insert(extension_id.to_string());
        match ext_service.get(extension_id).await {
            Ok(Some(doc)) => match installer.resolve_team_extension(team_id, &doc).await {
                Ok(config) => resolved.push(config),
                Err(error) => {
                    tracing::warn!(
                        "Failed to resolve legacy team extension '{}' ({}): {}; using stored config as-is",
                        legacy.name,
                        extension_id,
                        error
                    );
                    resolved.push(legacy.clone());
                }
            },
            Ok(None) | Err(_) => resolved.push(legacy.clone()),
        }
    }

    resolved
}

pub(crate) struct AgentApiCaller {
    api_url: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    api_format: ApiFormat,
}

impl ApiCaller for AgentApiCaller {
    fn call_llm<'a>(
        &'a self,
        system: &'a str,
        messages: Vec<serde_json::Value>,
        max_tokens: u32,
        tools: Option<Vec<rmcp::model::Tool>>,
        tool_choice: Option<rmcp::model::ToolChoice>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value>> + Send + 'a>>
    {
        Box::pin(async move {
            let client = build_http_client()?;
            match self.api_format {
                ApiFormat::Anthropic => {
                    self.call_anthropic(
                        client,
                        system,
                        &messages,
                        max_tokens,
                        tools.as_deref(),
                        tool_choice.as_ref(),
                    )
                    .await
                }
                ApiFormat::OpenAI | ApiFormat::LiteLLM => {
                    self.call_openai(
                        client,
                        system,
                        &messages,
                        max_tokens,
                        tools.as_deref(),
                        tool_choice.as_ref(),
                    )
                    .await
                }
                ApiFormat::Local => Err(anyhow!("Local API does not support MCP Sampling")),
            }
        })
    }
}

pub(crate) fn build_api_caller(agent: &TeamAgent) -> Option<Arc<dyn ApiCaller>> {
    if agent.api_format == ApiFormat::Local {
        return None;
    }
    Some(Arc::new(AgentApiCaller {
        api_url: agent.api_url.clone(),
        api_key: agent.api_key.clone(),
        model: agent.model.clone(),
        api_format: agent.api_format,
    }))
}

impl AgentApiCaller {
    fn anthropic_tools_payload(
        tools: Option<&[rmcp::model::Tool]>,
    ) -> Option<Vec<serde_json::Value>> {
        let tools = tools?;
        if tools.is_empty() {
            return None;
        }
        Some(
            tools
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "name": tool.name,
                        "description": tool.description.clone().unwrap_or_default(),
                        "input_schema": tool.input_schema,
                    })
                })
                .collect(),
        )
    }

    fn openai_tools_payload(tools: Option<&[rmcp::model::Tool]>) -> Option<Vec<serde_json::Value>> {
        let tools = tools?;
        if tools.is_empty() {
            return None;
        }
        Some(
            tools
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description.clone().unwrap_or_default(),
                            "parameters": tool.input_schema,
                        }
                    })
                })
                .collect(),
        )
    }

    fn anthropic_tool_choice_payload(
        tool_choice: Option<&rmcp::model::ToolChoice>,
    ) -> Option<serde_json::Value> {
        match tool_choice.and_then(|c| c.mode.clone()) {
            Some(rmcp::model::ToolChoiceMode::Required) => Some(serde_json::json!({"type": "any"})),
            Some(rmcp::model::ToolChoiceMode::Auto) => Some(serde_json::json!({"type": "auto"})),
            Some(rmcp::model::ToolChoiceMode::None) => None,
            None => None,
        }
    }

    fn openai_tool_choice_payload(
        tool_choice: Option<&rmcp::model::ToolChoice>,
    ) -> Option<serde_json::Value> {
        match tool_choice.and_then(|c| c.mode.clone()) {
            Some(rmcp::model::ToolChoiceMode::Required) => Some(serde_json::json!("required")),
            Some(rmcp::model::ToolChoiceMode::Auto) => Some(serde_json::json!("auto")),
            Some(rmcp::model::ToolChoiceMode::None) => Some(serde_json::json!("none")),
            None => None,
        }
    }

    fn normalize_anthropic_content_blocks(content: &serde_json::Value) -> Vec<serde_json::Value> {
        fn text_block(text: String) -> serde_json::Value {
            serde_json::json!({
                "type": "text",
                "text": text,
            })
        }

        let mut blocks = Vec::new();
        if let Some(items) = content.as_array() {
            for item in items {
                let block_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                match block_type {
                    "text" => {
                        let text = item
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        blocks.push(text_block(text));
                    }
                    "tool_use" => {
                        let id = item
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let name = item
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let input = item
                            .get("input")
                            .and_then(|v| v.as_object().cloned())
                            .unwrap_or_default();
                        if !id.is_empty() && !name.is_empty() {
                            blocks.push(serde_json::json!({
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": input,
                            }));
                        }
                    }
                    "tool_result" => {
                        let tool_use_id = item
                            .get("tool_use_id")
                            .and_then(|v| v.as_str())
                            .or_else(|| item.get("toolUseId").and_then(|v| v.as_str()))
                            .unwrap_or_default()
                            .to_string();
                        if tool_use_id.is_empty() {
                            continue;
                        }
                        let content_value = item
                            .get("content")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!([]));
                        let is_error = item
                            .get("is_error")
                            .and_then(|v| v.as_bool())
                            .or_else(|| item.get("isError").and_then(|v| v.as_bool()))
                            .unwrap_or(false);
                        blocks.push(serde_json::json!({
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": content_value,
                            "is_error": is_error,
                        }));
                    }
                    _ => {
                        if let Some(text) = item.as_str() {
                            blocks.push(text_block(text.to_string()));
                        } else {
                            blocks.push(text_block(item.to_string()));
                        }
                    }
                }
            }
        } else if let Some(text) = content.as_str() {
            blocks.push(text_block(text.to_string()));
        } else if !content.is_null() {
            blocks.push(text_block(content.to_string()));
        }

        if blocks.is_empty() {
            blocks.push(text_block(String::new()));
        }
        blocks
    }

    fn normalize_anthropic_messages(messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|msg| {
                let role = msg
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("user")
                    .to_string();
                let content = msg.get("content").unwrap_or(&serde_json::Value::Null);
                serde_json::json!({
                    "role": role,
                    "content": Self::normalize_anthropic_content_blocks(content),
                })
            })
            .collect()
    }

    fn openai_tool_result_text(content: &serde_json::Value) -> String {
        if let Some(s) = content.as_str() {
            return s.to_string();
        }
        if let Some(items) = content.as_array() {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                    parts.push(text.to_string());
                } else if let Some(text) = item.as_str() {
                    parts.push(text.to_string());
                }
            }
            return parts.join("\n");
        }
        if content.is_null() {
            return String::new();
        }
        content.to_string()
    }

    fn flush_openai_text_message(
        out: &mut Vec<serde_json::Value>,
        role: &str,
        text_buf: &mut String,
    ) -> bool {
        if text_buf.is_empty() {
            return false;
        }
        out.push(serde_json::json!({
            "role": role,
            "content": text_buf.clone(),
        }));
        text_buf.clear();
        true
    }

    fn normalize_openai_messages(messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for msg in messages {
            let role = msg
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string();
            let Some(content) = msg.get("content") else {
                continue;
            };

            if let Some(text) = content.as_str() {
                out.push(serde_json::json!({
                    "role": role,
                    "content": text,
                }));
                continue;
            }

            let Some(items) = content.as_array() else {
                out.push(serde_json::json!({
                    "role": role,
                    "content": content.to_string(),
                }));
                continue;
            };

            let mut text_buf = String::new();
            let mut emitted = false;

            for item in items {
                let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                match item_type {
                    "text" => {
                        if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                            if !text_buf.is_empty() {
                                text_buf.push('\n');
                            }
                            text_buf.push_str(text);
                        }
                    }
                    "tool_use" => {
                        if Self::flush_openai_text_message(&mut out, &role, &mut text_buf) {
                            emitted = true;
                        }
                        let id = item
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let name = item
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        if id.is_empty() || name.is_empty() {
                            continue;
                        }
                        let args_value = item
                            .get("input")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({}));
                        let arguments = if let Some(s) = args_value.as_str() {
                            s.to_string()
                        } else {
                            serde_json::to_string(&args_value).unwrap_or_else(|_| "{}".to_string())
                        };
                        out.push(serde_json::json!({
                            "role": "assistant",
                            "content": serde_json::Value::Null,
                            "tool_calls": [{
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": arguments,
                                }
                            }],
                        }));
                        emitted = true;
                    }
                    "tool_result" => {
                        if Self::flush_openai_text_message(&mut out, &role, &mut text_buf) {
                            emitted = true;
                        }
                        let tool_call_id = item
                            .get("toolUseId")
                            .and_then(|v| v.as_str())
                            .or_else(|| item.get("tool_use_id").and_then(|v| v.as_str()))
                            .unwrap_or_default()
                            .to_string();
                        if tool_call_id.is_empty() {
                            continue;
                        }
                        let result_text = Self::openai_tool_result_text(
                            item.get("content").unwrap_or(&serde_json::Value::Null),
                        );
                        out.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": tool_call_id,
                            "content": result_text,
                        }));
                        emitted = true;
                    }
                    _ => {
                        if let Some(text) = item.as_str() {
                            if !text_buf.is_empty() {
                                text_buf.push('\n');
                            }
                            text_buf.push_str(text);
                        }
                    }
                }
            }
            if Self::flush_openai_text_message(&mut out, &role, &mut text_buf) {
                emitted = true;
            }
            if !emitted {
                out.push(serde_json::json!({
                    "role": role,
                    "content": "",
                }));
            }
        }
        out
    }

    async fn call_anthropic(
        &self,
        client: reqwest::Client,
        system: &str,
        messages: &[serde_json::Value],
        max_tokens: u32,
        tools: Option<&[rmcp::model::Tool]>,
        tool_choice: Option<&rmcp::model::ToolChoice>,
    ) -> Result<serde_json::Value> {
        let base_url = self
            .api_url
            .as_deref()
            .unwrap_or("https://api.anthropic.com");
        let model = self.model.as_deref().unwrap_or("claude-3-opus-20240229");
        let api_key = self
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow!("API key not configured"))?;

        let is_volcengine = base_url.contains("ark.cn-beijing.volces.com");
        let api_url = if base_url.ends_with("/messages") || base_url.ends_with("/v1/messages") {
            base_url.to_string()
        } else if base_url.ends_with("/v1") {
            format!("{}/messages", base_url)
        } else {
            format!("{}/v1/messages", base_url.trim_end_matches('/'))
        };

        let mut request = client
            .post(&api_url)
            .header("Content-Type", "application/json");

        if is_volcengine {
            request = request.header("Authorization", format!("Bearer {}", api_key));
        } else {
            request = request
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        }

        let normalized_messages = Self::normalize_anthropic_messages(messages);
        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": normalized_messages,
        });
        if !system.is_empty() {
            body["system"] = serde_json::json!(system);
        }
        if let Some(tool_defs) = Self::anthropic_tools_payload(tools) {
            body["tools"] = serde_json::json!(tool_defs);
        }
        if let Some(choice) = Self::anthropic_tool_choice_payload(tool_choice) {
            body["tool_choice"] = choice;
        }

        let response = request.json(&body).send().await?;
        if !response.status().is_success() {
            let error = response.text().await?;
            return Err(anyhow!("Anthropic API error: {}", error));
        }
        response
            .json()
            .await
            .map_err(|e| anyhow!("Parse error: {}", e))
    }

    async fn call_openai(
        &self,
        client: reqwest::Client,
        system: &str,
        messages: &[serde_json::Value],
        max_tokens: u32,
        tools: Option<&[rmcp::model::Tool]>,
        tool_choice: Option<&rmcp::model::ToolChoice>,
    ) -> Result<serde_json::Value> {
        let api_url = self
            .api_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1/chat/completions");
        let model = self.model.as_deref().unwrap_or("gpt-4");
        let api_key = self
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow!("API key not configured"))?;

        let mut all_messages = Vec::new();
        if !system.is_empty() {
            all_messages.push(serde_json::json!({"role": "system", "content": system}));
        }
        all_messages.extend(Self::normalize_openai_messages(messages));

        let mut body = serde_json::json!({
            "model": model,
            "messages": all_messages,
            "max_tokens": max_tokens,
        });
        if let Some(tool_defs) = Self::openai_tools_payload(tools) {
            body["tools"] = serde_json::json!(tool_defs);
        }
        if let Some(choice) = Self::openai_tool_choice_payload(tool_choice) {
            body["tool_choice"] = choice;
        }

        let response = client
            .post(api_url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await?;
            return Err(anyhow!("OpenAI API error: {}", error));
        }
        response
            .json()
            .await
            .map_err(|e| anyhow!("Parse error: {}", e))
    }
}

pub struct ExtensionOverrides {
    pub disabled: Vec<String>,
    pub enabled: Vec<String>,
}

pub fn compute_extension_overrides(
    agent: &TeamAgent,
    active: &HashSet<String>,
) -> ExtensionOverrides {
    let default_names: HashSet<String> = agent
        .enabled_extensions
        .iter()
        .filter(|e| e.enabled)
        .filter(|e| {
            !matches!(
                e.extension,
                BuiltinExtension::ExtensionManager
                    | BuiltinExtension::ChatRecall
                    | BuiltinExtension::Team
                    | BuiltinExtension::DocumentTools
            )
        })
        .map(|e| {
            if let Some(mcp) = e.extension.mcp_name() {
                return mcp.to_string();
            }
            if e.extension == BuiltinExtension::Skills && active.contains("team_skills") {
                return "team_skills".to_string();
            }
            e.extension.name().to_string()
        })
        .chain(
            agent
                .custom_extensions
                .iter()
                .filter(|e| e.enabled)
                .map(|e| e.name.clone()),
        )
        .collect();

    let disabled = default_names.difference(active).cloned().collect();
    let enabled = active.difference(&default_names).cloned().collect();

    ExtensionOverrides { disabled, enabled }
}
