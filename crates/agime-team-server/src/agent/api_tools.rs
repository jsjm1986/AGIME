use agime::agents::mcp_client::McpClientTrait;
use anyhow::{anyhow, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use rmcp::model::*;
use rmcp::ServiceError;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::executor_mongo::build_http_client;

pub struct ApiToolsProvider {
    info: InitializeResult,
}

impl ApiToolsProvider {
    pub fn new() -> Self {
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
                name: "api_tools".to_string(),
                title: Some("API Tools".to_string()),
                version: "1.0.0".to_string(),
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Use these tools for direct structured API/HTTP calls. Prefer http_request over browser or shell when validating external APIs."
                    .to_string(),
            ),
        };
        Self { info }
    }

    fn tool_definitions() -> Vec<Tool> {
        vec![Tool {
            name: "http_request".into(),
            title: None,
            description: Some(
                "Perform a direct HTTP request to an API endpoint. Prefer this tool over browser automation or shell/curl for API verification and execution."
                    .into(),
            ),
            input_schema: serde_json::from_value(json!({
                "type": "object",
                "properties": {
                    "method": {
                        "type": "string",
                        "description": "HTTP method, e.g. GET, POST, PUT, PATCH, DELETE",
                        "default": "GET"
                    },
                    "url": {
                        "type": "string",
                        "description": "Fully qualified URL"
                    },
                    "headers": {
                        "type": "object",
                        "description": "Optional request headers",
                        "additionalProperties": { "type": "string" }
                    },
                    "body": {
                        "type": "string",
                        "description": "Optional raw request body"
                    },
                    "timeout_sec": {
                        "type": "integer",
                        "description": "Optional request timeout in seconds (default 20)"
                    },
                    "follow_redirects": {
                        "type": "boolean",
                        "description": "Whether redirects should be followed (default true)"
                    },
                    "max_body_chars": {
                        "type": "integer",
                        "description": "Maximum response body characters to return (default 4000)"
                    }
                },
                "required": ["url"]
            }))
            .unwrap_or_default(),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }]
    }

    async fn handle_http_request(&self, args: &JsonObject) -> Result<String> {
        let url = args
            .get("url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("url is required"))?;
        let method = args
            .get("method")
            .and_then(|value| value.as_str())
            .unwrap_or("GET")
            .trim()
            .to_ascii_uppercase();
        let timeout_sec = args
            .get("timeout_sec")
            .and_then(|value| value.as_u64())
            .unwrap_or(20)
            .clamp(1, 120);
        let follow_redirects = args
            .get("follow_redirects")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let max_body_chars = args
            .get("max_body_chars")
            .and_then(|value| value.as_u64())
            .unwrap_or(4000)
            .clamp(128, 32_000) as usize;

        let mut builder =
            build_http_client()?.request(reqwest::Method::from_bytes(method.as_bytes())?, url);
        builder = builder.timeout(std::time::Duration::from_secs(timeout_sec));
        if !follow_redirects {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(timeout_sec))
                .build()?;
            builder = client.request(reqwest::Method::from_bytes(method.as_bytes())?, url);
        }

        let mut headers = HeaderMap::new();
        if let Some(map) = args.get("headers").and_then(|value| value.as_object()) {
            for (name, value) in map {
                let Some(value) = value.as_str() else {
                    continue;
                };
                let Ok(header_name) = HeaderName::from_bytes(name.as_bytes()) else {
                    continue;
                };
                let Ok(header_value) = HeaderValue::from_str(value) else {
                    continue;
                };
                headers.insert(header_name, header_value);
            }
        }
        if !headers.is_empty() {
            builder = builder.headers(headers);
        }
        if let Some(body) = args.get("body").and_then(|value| value.as_str()) {
            builder = builder.body(body.to_string());
        }

        let response = builder.send().await?;
        let status_code = response.status().as_u16();
        let response_headers = response.headers().clone();
        let body_text = response.text().await.unwrap_or_default();
        let content_type = response_headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string());
        let json_body = serde_json::from_str::<Value>(&body_text).ok();

        Ok(serde_json::to_string_pretty(&json!({
            "ok": status_code < 400,
            "request": {
                "method": method,
                "url": url,
            },
            "response": {
                "status_code": status_code,
                "content_type": content_type,
                "body_text": body_text.chars().take(max_body_chars).collect::<String>(),
                "json_body": json_body,
            }
        }))?)
    }
}

#[async_trait::async_trait]
impl McpClientTrait for ApiToolsProvider {
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
            "http_request" => self.handle_http_request(&args).await,
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
