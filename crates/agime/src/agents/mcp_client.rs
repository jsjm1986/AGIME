use crate::action_required_manager::ActionRequiredManager;
use crate::agents::types::SharedProvider;
use crate::config::env_compat::{get_env_compat_or, get_env_compat_parsed_or};
use crate::providers::base::CompletionOptions;
use crate::session_context::SESSION_ID_HEADER;
use rmcp::model::{
    CreateElicitationRequestParams, CreateElicitationResult, ElicitationAction, ErrorCode,
    JsonObject, SamplingMessageContent, ToolChoiceMode,
};
/// MCP client implementation for AGIME
use rmcp::{
    model::{
        CallToolRequest, CallToolRequestParams, CallToolResult, CancelTaskParams,
        CancelTaskRequest, CancelledNotification, CancelledNotificationMethod,
        CancelledNotificationParam, ClientCapabilities, ClientInfo, ClientRequest,
        CreateMessageRequestParams, CreateMessageResult, CreateTaskResult, GetPromptRequest,
        GetPromptRequestParams, GetPromptResult, GetTaskInfoParams, GetTaskInfoRequest,
        GetTaskInfoResult, GetTaskResultParams, GetTaskResultRequest, Implementation,
        InitializeResult, ListPromptsRequest, ListPromptsResult, ListResourcesRequest,
        ListResourcesResult, ListTasksRequest, ListTasksResult, ListToolsRequest, ListToolsResult,
        LoggingMessageNotification, LoggingMessageNotificationMethod, PaginatedRequestParams,
        ProgressNotification, ProgressNotificationMethod, PromptListChangedNotification,
        PromptListChangedNotificationMethod, ProtocolVersion, ReadResourceRequest,
        ReadResourceRequestParams, ReadResourceResult, RequestId, ResourceListChangedNotification,
        ResourceListChangedNotificationMethod, SamplingMessage, ServerNotification, ServerResult,
        TaskResult, TaskStatus, Tool, ToolListChangedNotification,
        ToolListChangedNotificationMethod,
    },
    service::{
        ClientInitializeError, PeerRequestOptions, RequestContext, RequestHandle, RunningService,
        ServiceRole,
    },
    transport::IntoTransport,
    ClientHandler, ErrorData, Peer, RoleClient, ServiceError, ServiceExt,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::{
    mpsc::{self, Sender},
    Mutex,
};
use tokio_util::sync::CancellationToken;

pub type BoxError = Box<dyn std::error::Error + Sync + Send>;

pub type Error = rmcp::ServiceError;

#[async_trait::async_trait]
pub trait McpClientTrait: Send + Sync {
    async fn list_resources(
        &self,
        next_cursor: Option<String>,
        cancel_token: CancellationToken,
    ) -> Result<ListResourcesResult, Error>;

    async fn read_resource(
        &self,
        uri: &str,
        cancel_token: CancellationToken,
    ) -> Result<ReadResourceResult, Error>;

    async fn list_tools(
        &self,
        next_cursor: Option<String>,
        cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error>;

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error>;

    async fn call_tool_with_task(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        task: Option<JsonObject>,
        cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let _ = task;
        self.call_tool(name, arguments, cancel_token).await
    }

    async fn list_tasks(
        &self,
        cursor: Option<String>,
        cancel_token: CancellationToken,
    ) -> Result<ListTasksResult, Error>;

    async fn get_task_info(
        &self,
        task_id: &str,
        cancel_token: CancellationToken,
    ) -> Result<GetTaskInfoResult, Error>;

    async fn get_task_result(
        &self,
        task_id: &str,
        cancel_token: CancellationToken,
    ) -> Result<TaskResult, Error>;

    async fn cancel_task(
        &self,
        task_id: &str,
        cancel_token: CancellationToken,
    ) -> Result<(), Error>;

    async fn list_prompts(
        &self,
        next_cursor: Option<String>,
        cancel_token: CancellationToken,
    ) -> Result<ListPromptsResult, Error>;

    async fn get_prompt(
        &self,
        name: &str,
        arguments: Value,
        cancel_token: CancellationToken,
    ) -> Result<GetPromptResult, Error>;

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification>;

    fn get_info(&self) -> Option<&InitializeResult>;

    async fn get_moim(&self) -> Option<String> {
        None
    }
}

pub struct GooseClient {
    notification_handlers: Arc<Mutex<Vec<Sender<ServerNotification>>>>,
    provider: SharedProvider,
    tool_list_changed: Arc<AtomicBool>,
}

impl GooseClient {
    pub fn new(
        handlers: Arc<Mutex<Vec<Sender<ServerNotification>>>>,
        provider: SharedProvider,
        tool_list_changed: Arc<AtomicBool>,
    ) -> Self {
        GooseClient {
            notification_handlers: handlers,
            provider,
            tool_list_changed,
        }
    }

    fn resolve_sampling_tools(
        tools: Option<Vec<Tool>>,
        tool_choice_mode: Option<ToolChoiceMode>,
    ) -> Result<Vec<Tool>, ErrorData> {
        let mut resolved = tools.unwrap_or_default();
        match tool_choice_mode {
            Some(ToolChoiceMode::None) => {
                resolved.clear();
            }
            Some(ToolChoiceMode::Required) if resolved.is_empty() => {
                return Err(ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "tool_choice=required but no tools were provided",
                    None,
                ));
            }
            _ => {}
        }
        Ok(resolved)
    }

    fn validate_sampling_tool_choice_result(
        tool_choice_mode: Option<ToolChoiceMode>,
        has_tool_use: bool,
    ) -> Result<(), ErrorData> {
        match tool_choice_mode {
            Some(ToolChoiceMode::Required) if !has_tool_use => Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                "tool_choice=required but model returned no tool_use blocks",
                None,
            )),
            Some(ToolChoiceMode::None) if has_tool_use => Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                "tool_choice=none but model returned tool_use blocks",
                None,
            )),
            _ => Ok(()),
        }
    }

    fn decode_sampling_tool_use(
        tool_use: &rmcp::model::ToolUseContent,
    ) -> Result<(String, String, JsonObject), ErrorData> {
        let value = serde_json::to_value(tool_use).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to serialize tool_use content: {}", e),
                None,
            )
        })?;

        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let name = value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let input = value
            .get("input")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        if name.is_empty() {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                "tool_use.name is required",
                None,
            ));
        }

        Ok((id, name, input))
    }

    fn decode_sampling_tool_result(
        tool_result: &rmcp::model::ToolResultContent,
    ) -> Result<(String, Vec<rmcp::model::Content>, Option<bool>), ErrorData> {
        let value = serde_json::to_value(tool_result).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to serialize tool_result content: {}", e),
                None,
            )
        })?;

        let tool_use_id = value
            .get("toolUseId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let content = value
            .get("content")
            .cloned()
            .and_then(|v| serde_json::from_value::<Vec<rmcp::model::Content>>(v).ok())
            .unwrap_or_default();
        let is_error = value.get("isError").and_then(|v| v.as_bool());

        if tool_use_id.is_empty() {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                "tool_result.toolUseId is required",
                None,
            ));
        }

        Ok((tool_use_id, content, is_error))
    }
}

impl ClientHandler for GooseClient {
    async fn on_progress(
        &self,
        params: rmcp::model::ProgressNotificationParam,
        context: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
        self.notification_handlers
            .lock()
            .await
            .iter()
            .for_each(|handler| {
                let _ = handler.try_send(ServerNotification::ProgressNotification(
                    ProgressNotification {
                        params: params.clone(),
                        method: ProgressNotificationMethod,
                        extensions: context.extensions.clone(),
                    },
                ));
            });
    }

    async fn on_logging_message(
        &self,
        params: rmcp::model::LoggingMessageNotificationParam,
        context: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
        self.notification_handlers
            .lock()
            .await
            .iter()
            .for_each(|handler| {
                let _ = handler.try_send(ServerNotification::LoggingMessageNotification(
                    LoggingMessageNotification {
                        params: params.clone(),
                        method: LoggingMessageNotificationMethod,
                        extensions: context.extensions.clone(),
                    },
                ));
            });
    }

    async fn on_resource_list_changed(
        &self,
        context: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
        self.notification_handlers
            .lock()
            .await
            .iter()
            .for_each(|handler| {
                let _ = handler.try_send(ServerNotification::ResourceListChangedNotification(
                    ResourceListChangedNotification {
                        method: ResourceListChangedNotificationMethod,
                        extensions: context.extensions.clone(),
                    },
                ));
            });
    }

    async fn on_tool_list_changed(
        &self,
        context: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
        self.tool_list_changed.store(true, Ordering::Release);
        self.notification_handlers
            .lock()
            .await
            .iter()
            .for_each(|handler| {
                let _ = handler.try_send(ServerNotification::ToolListChangedNotification(
                    ToolListChangedNotification {
                        method: ToolListChangedNotificationMethod,
                        extensions: context.extensions.clone(),
                    },
                ));
            });
    }

    async fn on_prompt_list_changed(
        &self,
        context: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
        self.notification_handlers
            .lock()
            .await
            .iter()
            .for_each(|handler| {
                let _ = handler.try_send(ServerNotification::PromptListChangedNotification(
                    PromptListChangedNotification {
                        method: PromptListChangedNotificationMethod,
                        extensions: context.extensions.clone(),
                    },
                ));
            });
    }

    #[allow(clippy::too_many_lines)]
    async fn create_message(
        &self,
        params: CreateMessageRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<CreateMessageResult, ErrorData> {
        let provider = self
            .provider
            .lock()
            .await
            .as_ref()
            .ok_or(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "Could not use provider",
                None,
            ))?
            .clone();

        let mut provider_ready_messages = Vec::new();
        for msg in &params.messages {
            let mut contents = Vec::new();
            for item in msg.content.iter() {
                if let Some(text) = item.as_text() {
                    contents.push(crate::conversation::message::MessageContent::text(
                        text.text.clone(),
                    ));
                    continue;
                }
                if let Some(tool_use) = item.as_tool_use() {
                    let (id, name, input) = Self::decode_sampling_tool_use(tool_use)?;
                    contents.push(crate::conversation::message::MessageContent::tool_request(
                        id,
                        Ok(CallToolRequestParams {
                            name: name.into(),
                            arguments: Some(input),
                            meta: None,
                            task: None,
                        }),
                    ));
                    continue;
                }
                if let Some(tool_result) = item.as_tool_result() {
                    let (tool_use_id, content, is_error) =
                        Self::decode_sampling_tool_result(tool_result)?;
                    contents.push(crate::conversation::message::MessageContent::tool_response(
                        tool_use_id,
                        Ok(CallToolResult {
                            content,
                            structured_content: None,
                            is_error,
                            meta: None,
                        }),
                    ));
                }
            }

            if contents.is_empty() {
                contents.push(crate::conversation::message::MessageContent::text(""));
            }

            provider_ready_messages.push(crate::conversation::message::Message::new(
                msg.role.clone(),
                chrono::Utc::now().timestamp(),
                contents,
            ));
        }

        let system_prompt = params
            .system_prompt
            .as_deref()
            .unwrap_or("You are a general-purpose AI agent called AGIME");
        let tool_choice_mode = params
            .tool_choice
            .as_ref()
            .and_then(|choice| choice.mode.clone());
        let resolved_tools =
            Self::resolve_sampling_tools(params.tools.clone(), tool_choice_mode.clone())?;

        let (response, usage) = provider
            .complete_with_options(
                system_prompt,
                &provider_ready_messages,
                &resolved_tools,
                CompletionOptions {
                    tool_choice_mode: tool_choice_mode.clone(),
                },
            )
            .await
            .map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "Unexpected error while completing the prompt",
                    Some(Value::from(e.to_string())),
                )
            })?;

        let mut sampling_content = Vec::new();
        for content in &response.content {
            match content {
                crate::conversation::message::MessageContent::Text(text) => {
                    sampling_content.push(SamplingMessageContent::text(text.text.clone()));
                }
                crate::conversation::message::MessageContent::ToolRequest(req) => {
                    if let Ok(tool_call) = &req.tool_call {
                        sampling_content.push(SamplingMessageContent::tool_use(
                            req.id.clone(),
                            tool_call.name.to_string(),
                            tool_call.arguments.clone().unwrap_or_default(),
                        ));
                    }
                }
                crate::conversation::message::MessageContent::ToolResponse(res) => {
                    if let Ok(tool_result) = &res.tool_result {
                        sampling_content.push(SamplingMessageContent::tool_result(
                            res.id.clone(),
                            tool_result.content.clone(),
                        ));
                    }
                }
                _ => {}
            }
        }

        if sampling_content.is_empty() {
            sampling_content.push(SamplingMessageContent::text(""));
        }
        let has_tool_use = sampling_content.iter().any(|c| c.as_tool_use().is_some());
        Self::validate_sampling_tool_choice_result(tool_choice_mode, has_tool_use)?;

        Ok(CreateMessageResult {
            model: usage.model,
            stop_reason: Some(if has_tool_use {
                CreateMessageResult::STOP_REASON_TOOL_USE.to_string()
            } else {
                CreateMessageResult::STOP_REASON_END_TURN.to_string()
            }),
            message: SamplingMessage {
                role: response.role,
                content: sampling_content.into(),
                meta: None,
            },
        })
    }

    async fn create_elicitation(
        &self,
        request: CreateElicitationRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<CreateElicitationResult, ErrorData> {
        match request {
            CreateElicitationRequestParams::FormElicitationParams {
                message,
                requested_schema,
                ..
            } => {
                let schema_value = serde_json::to_value(&requested_schema).map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to serialize elicitation schema: {}", e),
                        None,
                    )
                })?;

                ActionRequiredManager::global()
                    .request_and_wait(message.clone(), schema_value, Duration::from_secs(300))
                    .await
                    .map(|user_data| CreateElicitationResult {
                        action: ElicitationAction::Accept,
                        content: Some(user_data),
                    })
                    .map_err(|e| {
                        ErrorData::new(
                            ErrorCode::INTERNAL_ERROR,
                            format!("Elicitation request timed out or failed: {}", e),
                            None,
                        )
                    })
            }
            CreateElicitationRequestParams::UrlElicitationParams {
                message,
                url,
                elicitation_id,
                ..
            } => {
                let schema = json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["accept", "decline", "cancel"],
                            "description": "Choose accept after completing the URL flow."
                        }
                    },
                    "required": ["action"]
                });

                let user_data = ActionRequiredManager::global()
                    .request_and_wait(
                        format!(
                            "{message}\n\nURL: {url}\nElicitation ID: {elicitation_id}\n\nComplete the URL flow, then choose action."
                        ),
                        schema,
                        Duration::from_secs(300),
                    )
                    .await
                    .map_err(|e| {
                        ErrorData::new(
                            ErrorCode::INTERNAL_ERROR,
                            format!("URL elicitation request timed out or failed: {}", e),
                            None,
                        )
                    })?;

                let action = match user_data
                    .get("action")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_ascii_lowercase())
                    .as_deref()
                {
                    Some("accept") => ElicitationAction::Accept,
                    Some("decline") => ElicitationAction::Decline,
                    Some("cancel") => ElicitationAction::Cancel,
                    _ => ElicitationAction::Cancel,
                };

                Ok(CreateElicitationResult {
                    action,
                    content: None,
                })
            }
        }
    }

    fn get_info(&self) -> ClientInfo {
        ClientInfo {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ClientCapabilities::builder()
                .enable_sampling()
                .enable_sampling_tools()
                .enable_elicitation()
                .build(),
            client_info: Implementation {
                name: "agime".to_string(),
                version: get_env_compat_or("MCP_CLIENT_VERSION", env!("CARGO_PKG_VERSION")),
                description: None,
                icons: None,
                title: None,
                website_url: None,
            },
            meta: None,
        }
    }
}

/// The MCP client is the interface for MCP operations.
pub struct McpClient {
    client: Mutex<RunningService<RoleClient, GooseClient>>,
    notification_subscribers: Arc<Mutex<Vec<mpsc::Sender<ServerNotification>>>>,
    server_info: Option<InitializeResult>,
    timeout: std::time::Duration,
    tools_cache: Mutex<HashMap<String, (Instant, ListToolsResult)>>,
    tools_cache_ttl: Duration,
    tools_cache_dirty: Arc<AtomicBool>,
}

impl McpClient {
    fn task_timeout() -> Duration {
        Duration::from_secs(get_env_compat_parsed_or("MCP_TASK_TIMEOUT_SECS", 600u64))
    }

    fn poll_interval_for_task(task: &rmcp::model::Task) -> Duration {
        let ms = task.poll_interval.unwrap_or(1000).clamp(200, 10_000);
        Duration::from_millis(ms)
    }

    fn task_result_to_call_result(task_id: &str, task_result: TaskResult) -> CallToolResult {
        let content_type = task_result.content_type.clone();
        let summary = task_result.summary.clone();
        let value = task_result.value;
        let envelope = serde_json::json!({
            "task_id": task_id,
            "content_type": content_type,
            "summary": summary,
            "value": value.clone(),
        });

        let text = if content_type.starts_with("text/") {
            value.as_str().map(|s| s.to_string()).unwrap_or_else(|| {
                serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| envelope.to_string())
            })
        } else {
            serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| envelope.to_string())
        };

        let mut call_result = CallToolResult::success(vec![rmcp::model::Content::text(text)]);
        if let Some(map) = value.as_object() {
            call_result.structured_content = Some(serde_json::Value::Object(map.clone()));
        }
        call_result
    }

    fn task_status_error_result(
        task_id: &str,
        status: &str,
        status_message: Option<&str>,
    ) -> CallToolResult {
        let message = match status_message {
            Some(m) if !m.trim().is_empty() => format!("Task {} {}: {}", task_id, status, m),
            _ => format!("Task {} {}", task_id, status),
        };
        CallToolResult::error(vec![rmcp::model::Content::text(message)])
    }

    async fn wait_for_task_completion(
        &self,
        task_result: CreateTaskResult,
        cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let timeout = Self::task_timeout();
        let deadline = Instant::now() + timeout;
        let mut task = task_result.task;
        let task_id = task.task_id.clone();

        loop {
            match task.status {
                TaskStatus::Completed => {
                    let result = self
                        .send_request(
                            ClientRequest::GetTaskResultRequest(GetTaskResultRequest {
                                params: GetTaskResultParams {
                                    meta: None,
                                    task_id: task_id.clone(),
                                },
                                method: Default::default(),
                                extensions: inject_session_into_extensions(Default::default()),
                            }),
                            cancel_token.clone(),
                        )
                        .await?;

                    return match result {
                        ServerResult::TaskResult(task_result) => {
                            Ok(Self::task_result_to_call_result(&task_id, task_result))
                        }
                        _ => Err(ServiceError::UnexpectedResponse),
                    };
                }
                TaskStatus::Failed => {
                    return Ok(Self::task_status_error_result(
                        &task_id,
                        "failed",
                        task.status_message.as_deref(),
                    ));
                }
                TaskStatus::Cancelled => {
                    return Ok(Self::task_status_error_result(
                        &task_id,
                        "cancelled",
                        task.status_message.as_deref(),
                    ));
                }
                TaskStatus::InputRequired => {
                    return Ok(Self::task_status_error_result(
                        &task_id,
                        "requires input",
                        task.status_message.as_deref(),
                    ));
                }
                TaskStatus::Working => {}
            }

            if Instant::now() >= deadline {
                let _ = self.cancel_task(&task_id, CancellationToken::new()).await;
                return Err(ServiceError::Timeout { timeout });
            }

            let info = self
                .send_request(
                    ClientRequest::GetTaskInfoRequest(GetTaskInfoRequest {
                        params: GetTaskInfoParams {
                            meta: None,
                            task_id: task_id.clone(),
                        },
                        method: Default::default(),
                        extensions: inject_session_into_extensions(Default::default()),
                    }),
                    cancel_token.clone(),
                )
                .await?;

            match info {
                ServerResult::GetTaskInfoResult(info) => {
                    if let Some(next_task) = info.task {
                        task = next_task;
                    }
                }
                _ => return Err(ServiceError::UnexpectedResponse),
            }

            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_default();
            let sleep_for = Self::poll_interval_for_task(&task).min(remaining);
            if sleep_for.is_zero() {
                continue;
            }

            tokio::select! {
                _ = tokio::time::sleep(sleep_for) => {}
                _ = cancel_token.cancelled() => {
                    let _ = self.cancel_task(&task_id, CancellationToken::new()).await;
                    return Err(ServiceError::Cancelled { reason: Some("operation cancelled".to_string()) });
                }
            }
        }
    }

    pub async fn connect<T, E, A>(
        transport: T,
        timeout: std::time::Duration,
        provider: SharedProvider,
    ) -> Result<Self, ClientInitializeError>
    where
        T: IntoTransport<RoleClient, E, A>,
        E: std::error::Error + From<std::io::Error> + Send + Sync + 'static,
    {
        let notification_subscribers =
            Arc::new(Mutex::new(Vec::<mpsc::Sender<ServerNotification>>::new()));
        let tools_cache_dirty = Arc::new(AtomicBool::new(false));
        let tools_cache_ttl_secs = get_env_compat_parsed_or("MCP_LIST_TOOLS_CACHE_TTL_SECS", 15u64);
        let tools_cache_ttl = Duration::from_secs(tools_cache_ttl_secs);

        let client = GooseClient::new(
            notification_subscribers.clone(),
            provider,
            tools_cache_dirty.clone(),
        );
        let client: rmcp::service::RunningService<rmcp::RoleClient, GooseClient> =
            client.serve(transport).await?;
        let server_info = client.peer_info().cloned();

        Ok(Self {
            client: Mutex::new(client),
            notification_subscribers,
            server_info,
            timeout,
            tools_cache: Mutex::new(HashMap::new()),
            tools_cache_ttl,
            tools_cache_dirty,
        })
    }

    async fn send_request(
        &self,
        request: ClientRequest,
        cancel_token: CancellationToken,
    ) -> Result<ServerResult, Error> {
        let handle = self
            .client
            .lock()
            .await
            .send_cancellable_request(request, PeerRequestOptions::no_options())
            .await?;

        await_response(handle, self.timeout, &cancel_token).await
    }
}

async fn await_response(
    handle: RequestHandle<RoleClient>,
    timeout: Duration,
    cancel_token: &CancellationToken,
) -> Result<<RoleClient as ServiceRole>::PeerResp, ServiceError> {
    let receiver = handle.rx;
    let peer = handle.peer;
    let request_id = handle.id;
    tokio::select! {
        result = receiver => {
            result.map_err(|_e| ServiceError::TransportClosed)?
        }
        _ = tokio::time::sleep(timeout) => {
            send_cancel_message(&peer, request_id, Some("timed out".to_owned())).await?;
            Err(ServiceError::Timeout{timeout})
        }
        _ = cancel_token.cancelled() => {
            send_cancel_message(&peer, request_id, Some("operation cancelled".to_owned())).await?;
            Err(ServiceError::Cancelled { reason: None })
        }
    }
}

async fn send_cancel_message(
    peer: &Peer<RoleClient>,
    request_id: RequestId,
    reason: Option<String>,
) -> Result<(), ServiceError> {
    peer.send_notification(
        CancelledNotification {
            params: CancelledNotificationParam { request_id, reason },
            method: CancelledNotificationMethod,
            extensions: Default::default(),
        }
        .into(),
    )
    .await
}

#[async_trait::async_trait]
impl McpClientTrait for McpClient {
    fn get_info(&self) -> Option<&InitializeResult> {
        self.server_info.as_ref()
    }

    async fn list_resources(
        &self,
        cursor: Option<String>,
        cancel_token: CancellationToken,
    ) -> Result<ListResourcesResult, Error> {
        let res = self
            .send_request(
                ClientRequest::ListResourcesRequest(ListResourcesRequest {
                    params: Some(PaginatedRequestParams { cursor, meta: None }),
                    method: Default::default(),
                    extensions: inject_session_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::ListResourcesResult(result) => Ok(result),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn read_resource(
        &self,
        uri: &str,
        cancel_token: CancellationToken,
    ) -> Result<ReadResourceResult, Error> {
        let res = self
            .send_request(
                ClientRequest::ReadResourceRequest(ReadResourceRequest {
                    params: ReadResourceRequestParams {
                        uri: uri.to_string(),
                        meta: None,
                    },
                    method: Default::default(),
                    extensions: inject_session_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::ReadResourceResult(result) => Ok(result),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn list_tools(
        &self,
        cursor: Option<String>,
        cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        if self.tools_cache_dirty.swap(false, Ordering::AcqRel) {
            self.tools_cache.lock().await.clear();
        }

        let cache_key = cursor.clone().unwrap_or_else(|| "__root__".to_string());
        if !self.tools_cache_ttl.is_zero() {
            if let Some((cached_at, cached_result)) =
                self.tools_cache.lock().await.get(&cache_key).cloned()
            {
                if cached_at.elapsed() <= self.tools_cache_ttl {
                    return Ok(cached_result);
                }
            }
        }

        let res = self
            .send_request(
                ClientRequest::ListToolsRequest(ListToolsRequest {
                    params: Some(PaginatedRequestParams { cursor, meta: None }),
                    method: Default::default(),
                    extensions: inject_session_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::ListToolsResult(result) => {
                if !self.tools_cache_ttl.is_zero() {
                    let mut cache = self.tools_cache.lock().await;
                    cache.insert(cache_key, (Instant::now(), result.clone()));
                    if cache.len() > 128 {
                        cache.retain(|_, (cached_at, _)| {
                            cached_at.elapsed() <= self.tools_cache_ttl
                        });
                    }
                }
                Ok(result)
            }
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        self.call_tool_with_task(name, arguments, None, cancel_token)
            .await
    }

    async fn call_tool_with_task(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        task: Option<JsonObject>,
        cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let res = self
            .send_request(
                ClientRequest::CallToolRequest(CallToolRequest {
                    params: CallToolRequestParams {
                        name: name.to_string().into(),
                        arguments,
                        meta: None,
                        task,
                    },
                    method: Default::default(),
                    extensions: inject_session_into_extensions(Default::default()),
                }),
                cancel_token.clone(),
            )
            .await?;

        match res {
            ServerResult::CallToolResult(result) => Ok(result),
            ServerResult::CreateTaskResult(task_result) => {
                self.wait_for_task_completion(task_result, cancel_token)
                    .await
            }
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn list_tasks(
        &self,
        cursor: Option<String>,
        cancel_token: CancellationToken,
    ) -> Result<ListTasksResult, Error> {
        let res = self
            .send_request(
                ClientRequest::ListTasksRequest(ListTasksRequest {
                    params: Some(PaginatedRequestParams { cursor, meta: None }),
                    method: Default::default(),
                    extensions: inject_session_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::ListTasksResult(result) => Ok(result),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn get_task_info(
        &self,
        task_id: &str,
        cancel_token: CancellationToken,
    ) -> Result<GetTaskInfoResult, Error> {
        let res = self
            .send_request(
                ClientRequest::GetTaskInfoRequest(GetTaskInfoRequest {
                    params: GetTaskInfoParams {
                        meta: None,
                        task_id: task_id.to_string(),
                    },
                    method: Default::default(),
                    extensions: inject_session_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::GetTaskInfoResult(result) => Ok(result),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn get_task_result(
        &self,
        task_id: &str,
        cancel_token: CancellationToken,
    ) -> Result<TaskResult, Error> {
        let res = self
            .send_request(
                ClientRequest::GetTaskResultRequest(GetTaskResultRequest {
                    params: GetTaskResultParams {
                        meta: None,
                        task_id: task_id.to_string(),
                    },
                    method: Default::default(),
                    extensions: inject_session_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::TaskResult(result) => Ok(result),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn cancel_task(
        &self,
        task_id: &str,
        cancel_token: CancellationToken,
    ) -> Result<(), Error> {
        let res = self
            .send_request(
                ClientRequest::CancelTaskRequest(CancelTaskRequest {
                    params: CancelTaskParams {
                        meta: None,
                        task_id: task_id.to_string(),
                    },
                    method: Default::default(),
                    extensions: inject_session_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::EmptyResult(_) => Ok(()),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn list_prompts(
        &self,
        cursor: Option<String>,
        cancel_token: CancellationToken,
    ) -> Result<ListPromptsResult, Error> {
        let res = self
            .send_request(
                ClientRequest::ListPromptsRequest(ListPromptsRequest {
                    params: Some(PaginatedRequestParams { cursor, meta: None }),
                    method: Default::default(),
                    extensions: inject_session_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::ListPromptsResult(result) => Ok(result),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn get_prompt(
        &self,
        name: &str,
        arguments: Value,
        cancel_token: CancellationToken,
    ) -> Result<GetPromptResult, Error> {
        let arguments = match arguments {
            Value::Object(map) => Some(map),
            _ => None,
        };
        let res = self
            .send_request(
                ClientRequest::GetPromptRequest(GetPromptRequest {
                    params: GetPromptRequestParams {
                        name: name.to_string(),
                        arguments,
                        meta: None,
                    },
                    method: Default::default(),
                    extensions: inject_session_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::GetPromptResult(result) => Ok(result),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        let (tx, rx) = mpsc::channel(64);
        self.notification_subscribers.lock().await.push(tx);
        rx
    }
}

/// Replaces session ID, case-insensitively, in Extensions._meta.
fn inject_session_into_extensions(
    mut extensions: rmcp::model::Extensions,
) -> rmcp::model::Extensions {
    use rmcp::model::Meta;

    if let Some(session_id) = crate::session_context::current_session_id() {
        let mut meta_map = extensions
            .get::<Meta>()
            .map(|meta| meta.0.clone())
            .unwrap_or_default();

        // JsonObject is case-sensitive, so we use retain for case-insensitive removal
        meta_map.retain(|k, _| !k.eq_ignore_ascii_case(SESSION_ID_HEADER));

        meta_map.insert(SESSION_ID_HEADER.to_string(), Value::String(session_id));

        extensions.insert(Meta(meta_map));
    }

    extensions
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::Meta;
    use std::sync::Arc;

    fn dummy_tool() -> rmcp::model::Tool {
        rmcp::model::Tool {
            name: "dummy".into(),
            title: None,
            description: Some("dummy".into()),
            input_schema: Arc::new(serde_json::Map::new()),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }
    }

    #[tokio::test]
    async fn test_session_id_in_mcp_meta() {
        use serde_json::json;

        let session_id = "test-session-789";
        crate::session_context::with_session_id(Some(session_id.to_string()), async {
            let extensions = inject_session_into_extensions(Default::default());
            let meta = extensions.get::<Meta>().unwrap();

            assert_eq!(
                &meta.0,
                json!({
                    SESSION_ID_HEADER: session_id
                })
                .as_object()
                .unwrap()
            );
        })
        .await;
    }

    #[tokio::test]
    async fn test_no_session_id_in_mcp_when_absent() {
        let extensions = inject_session_into_extensions(Default::default());
        let meta = extensions.get::<Meta>();

        assert!(meta.is_none());
    }

    #[tokio::test]
    async fn test_all_mcp_operations_include_session() {
        use serde_json::json;

        let session_id = "consistent-session-id";
        crate::session_context::with_session_id(Some(session_id.to_string()), async {
            let ext1 = inject_session_into_extensions(Default::default());
            let ext2 = inject_session_into_extensions(Default::default());
            let ext3 = inject_session_into_extensions(Default::default());

            for ext in [&ext1, &ext2, &ext3] {
                assert_eq!(
                    &ext.get::<Meta>().unwrap().0,
                    json!({
                        SESSION_ID_HEADER: session_id
                    })
                    .as_object()
                    .unwrap()
                );
            }
        })
        .await;
    }

    #[tokio::test]
    async fn test_session_id_case_insensitive_replacement() {
        use rmcp::model::{Extensions, Meta};
        use serde_json::{from_value, json};

        let session_id = "new-session-id";
        crate::session_context::with_session_id(Some(session_id.to_string()), async {
            let mut extensions = Extensions::new();
            extensions.insert(
                from_value::<Meta>(json!({
                    "GOOSE-SESSION-ID": "old-session-1",
                    "Goose-Session-Id": "old-session-2",
                    "other-key": "preserve-me"
                }))
                .unwrap(),
            );

            let extensions = inject_session_into_extensions(extensions);
            let meta = extensions.get::<Meta>().unwrap();

            assert_eq!(
                &meta.0,
                json!({
                    SESSION_ID_HEADER: session_id,
                    "other-key": "preserve-me"
                })
                .as_object()
                .unwrap()
            );
        })
        .await;
    }

    #[test]
    fn test_resolve_sampling_tools_required_without_tools_is_error() {
        let result = GooseClient::resolve_sampling_tools(None, Some(ToolChoiceMode::Required));
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_sampling_tools_none_mode_clears_tools() {
        let result = GooseClient::resolve_sampling_tools(
            Some(vec![dummy_tool()]),
            Some(ToolChoiceMode::None),
        )
        .expect("tool resolution should succeed");
        assert!(result.is_empty());
    }

    #[test]
    fn test_validate_sampling_tool_choice_required_needs_tool_use() {
        let result = GooseClient::validate_sampling_tool_choice_result(
            Some(ToolChoiceMode::Required),
            false,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_sampling_tool_choice_none_rejects_tool_use() {
        let result =
            GooseClient::validate_sampling_tool_choice_result(Some(ToolChoiceMode::None), true);
        assert!(result.is_err());
    }

    #[test]
    fn test_task_result_to_call_result_keeps_structured_payload() {
        let task_result = TaskResult {
            content_type: "application/json".to_string(),
            value: serde_json::json!({"ok": true, "count": 2}),
            summary: Some("done".to_string()),
        };
        let converted = McpClient::task_result_to_call_result("task_1", task_result);
        assert_eq!(converted.is_error, Some(false));
        assert!(converted.structured_content.is_some());
        let text = converted
            .content
            .iter()
            .find_map(|c| c.as_text().map(|t| t.text.clone()))
            .unwrap_or_default();
        assert!(text.contains("\"task_id\": \"task_1\""));
        assert!(text.contains("\"content_type\": \"application/json\""));
    }

    #[test]
    fn test_task_status_error_result_marks_error() {
        let result =
            McpClient::task_status_error_result("task_9", "failed", Some("permission denied"));
        assert_eq!(result.is_error, Some(true));
        let text = result
            .content
            .iter()
            .find_map(|c| c.as_text().map(|t| t.text.clone()))
            .unwrap_or_default();
        assert!(text.contains("task_9"));
        assert!(text.contains("failed"));
        assert!(text.contains("permission denied"));
    }
}
