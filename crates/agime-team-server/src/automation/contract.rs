use std::collections::{BTreeMap, HashMap};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use shlex::split as shlex_split;

use super::models::{AutomationIntegrationDoc, DraftStatus, IntegrationAuthType};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedHttpRequest {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedHttpResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_body: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedHttpAction {
    pub verified: bool,
    pub risk_level: String,
    pub command_kind: String,
    pub source_tool: String,
    pub source_call_id: String,
    pub request: VerifiedHttpRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<VerifiedHttpResponse>,
}

#[derive(Debug, Clone)]
pub struct BuilderSyncPayload {
    pub status: DraftStatus,
    pub probe_report: Value,
    pub candidate_plan: Value,
    pub session_preview: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecutionContract {
    #[serde(default)]
    pub native_execution_ready: bool,
    #[serde(default)]
    pub verified_http_actions: Vec<VerifiedHttpAction>,
}

pub fn extract_last_assistant_text(messages_json: &str) -> Option<String> {
    let msgs: Vec<Value> = serde_json::from_str(messages_json).ok()?;
    let msg = msgs
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))?;
    let content = msg.get("content")?;

    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }

    content.as_array().and_then(|arr| {
        arr.iter()
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
            .find(|text| !text.trim().is_empty())
            .map(|text| text.to_string())
    })
}

pub fn derive_builder_sync_payload(
    session_id: &str,
    messages_json: &str,
    summary: Option<&str>,
    final_status: &str,
    current_status: DraftStatus,
    integrations: &[AutomationIntegrationDoc],
) -> BuilderSyncPayload {
    let assistant_text = extract_last_assistant_text(messages_json).unwrap_or_default();
    let verified_http_actions = extract_verified_http_actions(messages_json, integrations);
    let structured_summary = build_verified_actions_summary(&verified_http_actions);
    let verified_http_actions_json =
        serde_json::to_value(&verified_http_actions).unwrap_or_else(|_| json!([]));
    let native_execution_ready = !verified_http_actions.is_empty();
    let verified_integration_ids = verified_http_actions
        .iter()
        .filter_map(|item| item.request.integration_id.clone())
        .collect::<Vec<_>>();

    let status = if final_status == "failed" {
        DraftStatus::Failed
    } else if native_execution_ready || !assistant_text.trim().is_empty() {
        DraftStatus::Ready
    } else {
        current_status
    };

    let probe_report = json!({
        "session_id": session_id,
        "status": final_status,
        "summary": if !structured_summary.is_empty() { structured_summary.clone() } else { summary.unwrap_or_default().to_string() },
        "assistant_excerpt": assistant_text.chars().take(4000).collect::<String>(),
        "source": "builder_chat",
        "native_execution_ready": native_execution_ready,
        "verified_http_actions": verified_http_actions_json.clone(),
    });

    let candidate_plan = json!({
        "recommended_path": if !structured_summary.is_empty() { structured_summary.clone() } else { assistant_text.clone() },
        "builder_notes": assistant_text.chars().take(3000).collect::<String>(),
        "source": "builder_chat",
        "verified_http_actions": verified_http_actions_json,
        "native_execution_ready": native_execution_ready,
        "verified_integration_ids": verified_integration_ids,
        "execution_mode": if native_execution_ready { "native_http_preferred" } else { "agent_chat_fallback" },
    });

    BuilderSyncPayload {
        status,
        probe_report,
        candidate_plan,
        session_preview: if !structured_summary.is_empty() {
            structured_summary.chars().take(400).collect()
        } else {
            assistant_text.chars().take(400).collect()
        },
    }
}

fn build_verified_actions_summary(actions: &[VerifiedHttpAction]) -> String {
    if actions.is_empty() {
        return String::new();
    }
    let mut lines = vec!["已验证调用路径：".to_string()];
    for (index, action) in actions.iter().enumerate() {
        let status = action
            .response
            .as_ref()
            .and_then(|response| response.status_code)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        lines.push(format!(
            "{}. {} {} -> {}",
            index + 1,
            action.request.method,
            action.request.url,
            status
        ));
    }
    lines.join("\n")
}

pub fn extract_verified_http_actions(
    messages_json: &str,
    integrations: &[AutomationIntegrationDoc],
) -> Vec<VerifiedHttpAction> {
    let msgs: Vec<Value> = match serde_json::from_str(messages_json) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };

    let mut request_order = Vec::new();
    let mut requests: HashMap<String, (String, String)> = HashMap::new();
    let mut responses: HashMap<String, String> = HashMap::new();

    for msg in msgs {
        let Some(items) = msg.get("content").and_then(|value| value.as_array()) else {
            continue;
        };
        for item in items {
            match item.get("type").and_then(|value| value.as_str()) {
                Some("toolRequest") => {
                    let Some(call_id) = item.get("id").and_then(|value| value.as_str()) else {
                        continue;
                    };
                    let Some(tool_name) = item
                        .get("toolCall")
                        .and_then(|value| value.get("value"))
                        .and_then(|value| value.get("name"))
                        .and_then(|value| value.as_str())
                    else {
                        continue;
                    };
                    let extracted = item
                        .get("toolCall")
                        .and_then(|value| value.get("value"))
                        .and_then(|value| value.get("arguments"))
                        .map(|value| {
                            if tool_name == "api_tools__http_request" {
                                serde_json::to_string(value).unwrap_or_default()
                            } else {
                                value
                                    .get("command")
                                    .or_else(|| value.get("url"))
                                    .and_then(|inner| inner.as_str())
                                    .unwrap_or_default()
                                    .to_string()
                            }
                        })
                        .unwrap_or_default();
                    request_order.push(call_id.to_string());
                    requests.insert(call_id.to_string(), (tool_name.to_string(), extracted));
                }
                Some("toolResponse") => {
                    let Some(call_id) = item.get("id").and_then(|value| value.as_str()) else {
                        continue;
                    };
                    let text = tool_response_text(item);
                    if !text.trim().is_empty() {
                        responses.insert(call_id.to_string(), text);
                    }
                }
                _ => {}
            }
        }
    }

    request_order
        .into_iter()
        .filter_map(|call_id| {
            let (tool_name, command) = requests.get(&call_id)?.clone();
            let (request, command_kind) = if tool_name == "developer__shell" {
                parse_http_request_from_command(&command, integrations)?
            } else if tool_name == "api_tools__http_request" {
                parse_api_tools_request(&command, integrations)?
            } else if tool_name == "playwright__browser_navigate" {
                parse_playwright_navigate_request(&command, integrations)?
            } else {
                return None;
            };
            let response = responses.get(&call_id).and_then(|text| {
                if tool_name == "api_tools__http_request" {
                    parse_api_tools_response(text)
                } else if tool_name == "playwright__browser_navigate" {
                    parse_playwright_navigation_response(text)
                } else {
                    parse_http_response(text)
                }
            });
            Some(VerifiedHttpAction {
                verified: response.is_some(),
                risk_level: risk_level_for_method(&request.method).to_string(),
                command_kind,
                source_tool: tool_name,
                source_call_id: call_id,
                request,
                response,
            })
        })
        .collect()
}

pub fn parse_execution_contract(value: &Value) -> Option<ExecutionContract> {
    serde_json::from_value(value.clone()).ok()
}

fn parse_http_request_from_command(
    command: &str,
    integrations: &[AutomationIntegrationDoc],
) -> Option<(VerifiedHttpRequest, String)> {
    parse_curl_request(command, integrations)
        .map(|request| (request, "curl".to_string()))
        .or_else(|| parse_python_request(command, integrations).map(|request| (request, "python".to_string())))
}

fn parse_curl_request(
    command: &str,
    integrations: &[AutomationIntegrationDoc],
) -> Option<VerifiedHttpRequest> {
    let mut tokens = shlex_split(command)?;
    if tokens.first()?.as_str() != "curl" {
        return None;
    }
    if let Some(pipe_index) = tokens.iter().position(|item| item == "|") {
        tokens.truncate(pipe_index);
    }

    let mut method: Option<String> = None;
    let mut headers = BTreeMap::new();
    let mut body: Option<String> = None;
    let mut url: Option<String> = None;

    let mut index = 1usize;
    while index < tokens.len() {
        let token = &tokens[index];
        match token.as_str() {
            "-X" | "--request" => {
                if let Some(value) = tokens.get(index + 1) {
                    method = Some(value.to_uppercase());
                    index += 2;
                    continue;
                }
            }
            "-H" | "--header" => {
                if let Some(value) = tokens.get(index + 1) {
                    if let Some((name, header_value)) = parse_header(value) {
                        headers.insert(name, header_value);
                    }
                    index += 2;
                    continue;
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" => {
                if let Some(value) = tokens.get(index + 1) {
                    body = Some(value.clone());
                    index += 2;
                    continue;
                }
            }
            _ => {}
        }
        if url.is_none() && looks_like_http_url(token) {
            url = Some(token.clone());
        }
        index += 1;
    }

    let url = url?;
    let method = method.unwrap_or_else(|| {
        if body.is_some() {
            "POST".to_string()
        } else {
            "GET".to_string()
        }
    });

    Some(VerifiedHttpRequest {
        method,
        url: url.clone(),
        headers,
        body,
        integration_id: match_integration_id(&url, integrations),
    })
}

fn parse_python_request(
    command: &str,
    integrations: &[AutomationIntegrationDoc],
) -> Option<VerifiedHttpRequest> {
    if !command.contains("urllib.request") && !command.contains("requests.") {
        return None;
    }

    let url = first_http_literal(command)?;
    let method = if command.contains("requests.post(") || command.contains("method='POST'") {
        "POST"
    } else if command.contains("requests.put(") || command.contains("method='PUT'") {
        "PUT"
    } else if command.contains("requests.patch(") || command.contains("method='PATCH'") {
        "PATCH"
    } else if command.contains("requests.delete(") || command.contains("method='DELETE'") {
        "DELETE"
    } else {
        "GET"
    }
    .to_string();

    let mut headers = BTreeMap::new();
    if let Some(raw_headers) = extract_between(command, "headers={", "}") {
        for fragment in raw_headers.split(',') {
            if let Some((name, value)) = parse_header_fragment(fragment) {
                headers.insert(name, value);
            }
        }
    }

    Some(VerifiedHttpRequest {
        method,
        url: url.clone(),
        headers,
        body: None,
        integration_id: match_integration_id(&url, integrations),
    })
}

fn parse_playwright_navigate_request(
    url: &str,
    integrations: &[AutomationIntegrationDoc],
) -> Option<(VerifiedHttpRequest, String)> {
    if !looks_like_http_url(url) {
        return None;
    }
    Some((
        VerifiedHttpRequest {
            method: "GET".to_string(),
            url: url.to_string(),
            headers: BTreeMap::new(),
            body: None,
            integration_id: match_integration_id(url, integrations),
        },
        "playwright_navigate".to_string(),
    ))
}

fn parse_api_tools_request(
    raw: &str,
    integrations: &[AutomationIntegrationDoc],
) -> Option<(VerifiedHttpRequest, String)> {
    let value = serde_json::from_str::<Value>(raw).ok()?;
    let url = value.get("url")?.as_str()?;
    let method = value
        .get("method")
        .and_then(|item| item.as_str())
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let headers = value
        .get("headers")
        .and_then(|item| item.as_object())
        .map(|items| {
            items
                .iter()
                .filter_map(|(key, value)| value.as_str().map(|v| (key.clone(), v.to_string())))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let body = value
        .get("body")
        .and_then(|item| item.as_str())
        .map(|item| item.to_string());
    Some((
        VerifiedHttpRequest {
            method,
            url: url.to_string(),
            headers,
            body,
            integration_id: match_integration_id(url, integrations),
        },
        "api_tools".to_string(),
    ))
}

fn parse_http_response(text: &str) -> Option<VerifiedHttpResponse> {
    let normalized = text.replace("\r\n", "\n");
    let mut status_code = None;
    let mut content_type = None;
    let mut body_text: Option<String> = None;

    let blocks = normalized.split("\n\n").collect::<Vec<_>>();
    let head = *blocks.first()?;
    for line in head.lines() {
        if line.starts_with("HTTP/") {
            let parts = line.split_whitespace().collect::<Vec<_>>();
            status_code = parts.get(1).and_then(|value| value.parse::<u16>().ok());
        } else if line.to_ascii_lowercase().starts_with("content-type:") {
            content_type = Some(
                line.split_once(':')
                    .map(|(_, value)| value.trim().to_string())
                    .unwrap_or_default(),
            );
        }
    }

    if blocks.len() > 1 {
        let tail = blocks[1..].join("\n\n");
        let trimmed = tail.trim();
        if !trimmed.is_empty() {
            body_text = Some(trimmed.chars().take(4000).collect::<String>());
        }
    }

    let json_body = body_text
        .as_ref()
        .and_then(|body| serde_json::from_str::<Value>(body).ok());

    if status_code.is_none() && body_text.is_none() {
        return None;
    }

    Some(VerifiedHttpResponse {
        status_code,
        content_type,
        body_text,
        json_body,
    })
}

fn parse_playwright_navigation_response(text: &str) -> Option<VerifiedHttpResponse> {
    let snapshot_marker = "### Snapshot";
    let snapshot_block = text
        .split(snapshot_marker)
        .nth(1)
        .unwrap_or("")
        .chars()
        .take(4000)
        .collect::<String>();
    let page_marker = "- Page URL:";
    let body_text = if let Some(start) = text.find(page_marker) {
        let tail = &text[start..];
        let extracted = tail.chars().take(4000).collect::<String>();
        Some(extracted)
    } else if !snapshot_block.trim().is_empty() {
        Some(snapshot_block.clone())
    } else {
        None
    };

    let json_body = text
        .split('"')
        .find_map(|part| serde_json::from_str::<Value>(part).ok());

    if body_text.is_none() && json_body.is_none() {
        return None;
    }

    Some(VerifiedHttpResponse {
        status_code: None,
        content_type: Some(
            if json_body.is_some() {
                "application/json".to_string()
            } else {
                "text/plain".to_string()
            },
        ),
        body_text,
        json_body,
    })
}

fn parse_api_tools_response(text: &str) -> Option<VerifiedHttpResponse> {
    let value = serde_json::from_str::<Value>(text).ok()?;
    let response = value.get("response").cloned().unwrap_or_else(|| json!({}));
    Some(VerifiedHttpResponse {
        status_code: response
            .get("status_code")
            .and_then(|item| item.as_u64())
            .map(|value| value as u16),
        content_type: response
            .get("content_type")
            .and_then(|item| item.as_str())
            .map(|item| item.to_string()),
        body_text: response
            .get("body_text")
            .and_then(|item| item.as_str())
            .map(|item| item.to_string()),
        json_body: response.get("json_body").cloned(),
    })
}

fn tool_response_text(item: &Value) -> String {
    let Some(content) = item
        .get("toolResult")
        .and_then(|value| value.get("value"))
        .and_then(|value| value.get("content"))
        .and_then(|value| value.as_array())
    else {
        return String::new();
    };

    content
        .iter()
        .filter_map(|entry| entry.get("text").and_then(|value| value.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn match_integration_id(url: &str, integrations: &[AutomationIntegrationDoc]) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    integrations
        .iter()
        .filter_map(|integration| {
            let base_url = integration.base_url.as_ref()?;
            let parsed_base = Url::parse(base_url).ok()?;
            let parsed_base_str = parsed_base.as_str().trim_end_matches('/').to_string();
            let parsed_url_str = parsed.as_str().trim_end_matches('/').to_string();
            if parsed_url_str.starts_with(&parsed_base_str) {
                Some((parsed_base_str.len(), integration.integration_id.clone()))
            } else {
                None
            }
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, integration_id)| integration_id)
}

fn risk_level_for_method(method: &str) -> &'static str {
    match method.to_ascii_uppercase().as_str() {
        "GET" | "HEAD" | "OPTIONS" => "low",
        "POST" | "PUT" | "PATCH" | "DELETE" => "high",
        _ => "medium",
    }
}

fn looks_like_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn first_http_literal(text: &str) -> Option<String> {
    let start = text.find("http://").or_else(|| text.find("https://"))?;
    let tail = &text[start..];
    let end = tail
        .find(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | ')' | ']' | '}'))
        .unwrap_or(tail.len());
    Some(tail[..end].trim_end_matches(',').to_string())
}

fn extract_between<'a>(text: &'a str, start_marker: &str, end_marker: &str) -> Option<&'a str> {
    let start = text.find(start_marker)? + start_marker.len();
    let tail = &text[start..];
    let end = tail.find(end_marker)?;
    Some(&tail[..end])
}

fn parse_header(value: &str) -> Option<(String, String)> {
    let (name, header_value) = value.split_once(':')?;
    Some((name.trim().to_string(), header_value.trim().to_string()))
}

fn parse_header_fragment(value: &str) -> Option<(String, String)> {
    let trimmed = value.trim();
    let (name, header_value) = trimmed.split_once(':')?;
    Some((
        name.trim_matches(|c| c == '\'' || c == '"' || c == ' ').to_string(),
        header_value
            .trim_matches(|c| c == '\'' || c == '"' || c == ' ')
            .to_string(),
    ))
}

pub fn auth_headers_for_integration(integration: &AutomationIntegrationDoc) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    match integration.auth_type {
        IntegrationAuthType::None => {}
        IntegrationAuthType::Bearer => {
            if let Some(token) = lookup_auth_value(
                &integration.auth_config,
                &["token", "bearer_token", "access_token", "value"],
            ) {
                headers.insert("Authorization".to_string(), format!("Bearer {}", token));
            }
        }
        IntegrationAuthType::Header => {
            if let (Some(name), Some(value)) = (
                lookup_auth_value(&integration.auth_config, &["header_name", "name", "key"]),
                lookup_auth_value(&integration.auth_config, &["header_value", "value", "token"]),
            ) {
                headers.insert(name, value);
            }
        }
        IntegrationAuthType::Basic => {
            if let (Some(username), Some(password)) = (
                lookup_auth_value(&integration.auth_config, &["username", "user"]),
                lookup_auth_value(&integration.auth_config, &["password", "pass"]),
            ) {
                let encoded = STANDARD.encode(format!("{}:{}", username, password));
                headers.insert("Authorization".to_string(), format!("Basic {}", encoded));
            }
        }
        IntegrationAuthType::ApiKey => {
            if let (Some(name), Some(value)) = (
                lookup_auth_value(&integration.auth_config, &["header_name", "name", "key_name"]),
                lookup_auth_value(&integration.auth_config, &["key", "value", "token", "api_key"]),
            ) {
                headers.insert(name, value);
            }
        }
    }
    headers
}

fn lookup_auth_value(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|inner| inner.as_str())
            .map(|inner| inner.trim().to_string())
            .filter(|inner| !inner.is_empty())
    })
}
