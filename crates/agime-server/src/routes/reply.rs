use crate::state::AppState;
use agime::agents::{AgentEvent, SessionConfig};
use agime::conversation::message::{Message, MessageContent, SystemNotificationType, TokenState};
use agime::conversation::Conversation;
use agime::session::SessionManager;
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{self, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use bytes::Bytes;
use futures::{stream::StreamExt, Stream};
use rmcp::model::ServerNotification;
use serde::{Deserialize, Serialize};
use std::{
    convert::Infallible,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;

fn track_tool_telemetry(content: &MessageContent, all_messages: &[Message]) {
    match content {
        MessageContent::ToolRequest(tool_request) => {
            if let Ok(tool_call) = &tool_request.tool_call {
                tracing::info!(monotonic_counter.goose.tool_calls = 1,
                    tool_name = %tool_call.name,
                    "Tool call started"
                );
            }
        }
        MessageContent::ToolResponse(tool_response) => {
            let tool_name = all_messages
                .iter()
                .rev()
                .find_map(|msg| {
                    msg.content.iter().find_map(|c| {
                        if let MessageContent::ToolRequest(req) = c {
                            if req.id == tool_response.id {
                                if let Ok(tool_call) = &req.tool_call {
                                    Some(tool_call.name.clone())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or_else(|| "unknown".to_string().into());

            let success = tool_response.tool_result.is_ok();
            let result_status = if success { "success" } else { "error" };

            tracing::info!(
                counter.goose.tool_completions = 1,
                tool_name = %tool_name,
                result = %result_status,
                "Tool call completed"
            );
        }
        _ => {}
    }
}

/// True when any message in the turn carries image content the model would be
/// asked to read.
fn conversation_has_image(messages: &[Message]) -> bool {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .any(|c| matches!(c, MessageContent::Image(_)))
}

/// User-visible inline notice emitted when a turn contains an image but the
/// active model is not configured for multimodal input. The shared provider
/// formatter silently down-converts the image to a text placeholder (so the
/// LLM request stays well-formed), which historically left the user with no
/// signal that their image was dropped. This surfaces that explicitly. Wording
/// is aligned with `agime::providers::utils::multimodal_unsupported_message`.
fn image_dropped_notice() -> String {
    "注意：当前所选模型未开启多模态（图片）支持，本次发送的图片不会被模型读取，已自动转为文字占位符。\
如需让模型理解图片，请切换到支持视觉（vision）的模型，或在「模型设置」中开启该模型的多模态支持。\n\n\
Note: the selected model does not have multimodal (image) support enabled, so the image you \
sent will not be read by the model and has been converted to a text placeholder. To let the \
model understand images, switch to a vision-capable model or enable multimodal support for \
this model in Model Settings."
        .to_string()
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ChatRequest {
    messages: Vec<Message>,
    session_id: String,
    recipe_name: Option<String>,
    recipe_version: Option<String>,
}

pub struct SseResponse {
    rx: ReceiverStream<String>,
}

impl SseResponse {
    fn new(rx: ReceiverStream<String>) -> Self {
        Self { rx }
    }
}

impl Stream for SseResponse {
    type Item = Result<Bytes, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.rx)
            .poll_next(cx)
            .map(|opt| opt.map(|s| Ok(Bytes::from(s))))
    }
}

impl IntoResponse for SseResponse {
    fn into_response(self) -> axum::response::Response {
        let stream = self;
        let body = axum::body::Body::from_stream(stream);

        http::Response::builder()
            .header("Content-Type", "text/event-stream")
            .header("Cache-Control", "no-cache")
            .header("Connection", "keep-alive")
            .body(body)
            .unwrap()
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(tag = "type")]
pub enum MessageEvent {
    Message {
        message: Message,
        token_state: TokenState,
    },
    Error {
        error: String,
    },
    Finish {
        reason: String,
        token_state: TokenState,
    },
    ModelChange {
        model: String,
        mode: String,
    },
    Notification {
        request_id: String,
        #[schema(value_type = Object)]
        message: ServerNotification,
    },
    UpdateConversation {
        conversation: Conversation,
    },
    /// Harness control envelope mirroring an `AgentEvent` as a sequenced
    /// `HarnessControlMessage`. Only emitted when the
    /// `desktop_harness_host` cargo feature is enabled.
    #[cfg(feature = "desktop_harness_host")]
    HarnessControl {
        #[schema(value_type = Object)]
        envelope: agime::agents::HarnessControlEnvelope,
    },
    Ping,
}

async fn get_token_state(session_id: &str) -> TokenState {
    SessionManager::get_session(session_id, false)
        .await
        .map(|session| TokenState {
            input_tokens: session.input_tokens.unwrap_or(0),
            output_tokens: session.output_tokens.unwrap_or(0),
            total_tokens: session.total_tokens.unwrap_or(0),
            accumulated_input_tokens: session.accumulated_input_tokens.unwrap_or(0),
            accumulated_output_tokens: session.accumulated_output_tokens.unwrap_or(0),
            accumulated_total_tokens: session.accumulated_total_tokens.unwrap_or(0),
        })
        .inspect_err(|e| {
            tracing::warn!(
                "Failed to fetch session token state for {}: {}",
                session_id,
                e
            );
        })
        .unwrap_or_default()
}

async fn stream_event(
    event: MessageEvent,
    tx: &mpsc::Sender<String>,
    cancel_token: &CancellationToken,
) {
    let json = serde_json::to_string(&event).unwrap_or_else(|e| {
        format!(
            r#"{{"type":"Error","error":"Failed to serialize event: {}"}}"#,
            e
        )
    });

    if tx.send(format!("data: {}\n\n", json)).await.is_err() {
        tracing::info!("client hung up");
        cancel_token.cancel();
    }
}

#[allow(clippy::too_many_lines)]
#[utoipa::path(
    post,
    path = "/reply",
    request_body = ChatRequest,
    responses(
        (status = 200, description = "Streaming response initiated",
         body = MessageEvent,
         content_type = "text/event-stream"),
        (status = 424, description = "Agent not initialized"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn reply(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatRequest>,
) -> Result<SseResponse, StatusCode> {
    let session_start = std::time::Instant::now();

    tracing::info!(
        counter.goose.session_starts = 1,
        session_type = "app",
        interface = "ui",
        "[PERF] /reply request received"
    );

    let session_id = request.session_id.clone();

    if let Some(recipe_name) = request.recipe_name.clone() {
        if state.mark_recipe_run_if_absent(&session_id).await {
            let recipe_version = request
                .recipe_version
                .clone()
                .unwrap_or_else(|| "unknown".to_string());

            tracing::info!(
                counter.goose.recipe_runs = 1,
                recipe_name = %recipe_name,
                recipe_version = %recipe_version,
                session_type = "app",
                interface = "ui",
                "Recipe execution started"
            );
        }
    }

    let (tx, rx) = mpsc::channel(100);
    let stream = ReceiverStream::new(rx);
    let cancel_token = CancellationToken::new();

    let messages = Conversation::new_unvalidated(request.messages);

    let task_cancel = cancel_token.clone();
    let task_tx = tx.clone();

    drop(tokio::spawn(async move {
        let agent = match state.get_agent(session_id.clone()).await {
            Ok(agent) => agent,
            Err(e) => {
                tracing::error!("Failed to get session agent: {}", e);
                let _ = stream_event(
                    MessageEvent::Error {
                        error: format!("Failed to get session agent: {}", e),
                    },
                    &task_tx,
                    &task_cancel,
                )
                .await;
                return;
            }
        };

        // Phase 4: execution-slot admission control (desktop-harness-host
        // only). Mirrors team-server `wait_for_execution_slot`. The desktop
        // default is `max_slots = 1` per agent, matching today's
        // single-turn-per-agent behavior; raise via
        // `DesktopExecutionSlotProvider::with_max_slots` for power users.
        // The `_slot_guard` releases the slot on every drop path (success,
        // panic, cancellation), so the slot is always returned.
        #[cfg(feature = "desktop_harness_host")]
        let _slot_guard = {
            use crate::host_admission::wait_for_execution_slot;
            if let Err(e) =
                wait_for_execution_slot(state.execution_slots.as_ref(), &session_id, &task_cancel)
                    .await
            {
                tracing::warn!(
                    "execution slot acquisition failed for {}: {}",
                    session_id,
                    e
                );
                let _ = stream_event(
                    MessageEvent::Error {
                        error: format!("Execution queue unavailable: {}", e),
                    },
                    &task_tx,
                    &task_cancel,
                )
                .await;
                return;
            }
            ExecutionSlotGuard {
                provider: state.execution_slots.clone(),
                agent_id: session_id.clone(),
            }
        };

        let _session = match SessionManager::get_session(&session_id, false).await {
            Ok(metadata) => metadata,
            Err(e) => {
                tracing::error!("Failed to read session for {}: {}", session_id, e);
                let _ = stream_event(
                    MessageEvent::Error {
                        error: format!("Failed to read session: {}", e),
                    },
                    &task_tx,
                    &cancel_token,
                )
                .await;
                return;
            }
        };

        let session_config = SessionConfig {
            id: session_id.clone(),
            schedule_id: None,
            max_turns: None,
            retry_config: None,
        };

        // Workspace execution boundary: when the session has a registered
        // `WorkspaceExecutionContext` (set via `/agent/session_override`),
        // append the rendered boundary instruction to the agent's system
        // prompt. This mirrors the team-server harness behavior of letting
        // the model see the workspace root, run dir, and write-allowed roots.
        //
        // The same override entry also carries optional provider replacement
        // and prompt-overlay text — apply them here, before `agent.reply()`,
        // so the chat turn observes the override.
        //
        // Lifecycle:
        // - `clear_host_override_extras` is called *unconditionally* so any
        //   host-level overlays pushed by a prior turn (with a now-different
        //   override) do not leak through. The session-static
        //   `system_prompt_extras` (desktop_prompt, recipe overlay, hints)
        //   live in a separate channel and are NOT touched here.
        // - `consume_session_override_for_turn` takes a snapshot AND drains
        //   the per-turn `turn_system_instruction` field, so it is
        //   single-shot.
        // - The provider is only re-applied if its `HostProviderConfig`
        //   changed since the last time it was applied for this session
        //   (see `AppState::record_applied_provider`), avoiding a
        //   `SessionManager::update_session` write and a router-tool
        //   selector rebuild on every chat turn.
        agent.clear_host_override_extras().await;
        if let Some(override_value) = state.consume_session_override_for_turn(&session_id).await {
            if let Some(cfg) = override_value.provider_config.as_ref() {
                if state.record_applied_provider(&session_id, cfg).await {
                    match crate::host_provider::create_provider_for_config(cfg) {
                        Ok(provider) => {
                            if let Err(e) = agent.update_provider(provider, &session_id).await {
                                tracing::warn!(
                                    "session_override: failed to replace provider for {}: {}",
                                    session_id,
                                    e
                                );
                                // Roll back the fingerprint so the next turn
                                // gets another chance to apply this config.
                                state.clear_applied_provider_fingerprint(&session_id).await;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "session_override: failed to build provider config for {}: {}",
                                session_id,
                                e
                            );
                            state.clear_applied_provider_fingerprint(&session_id).await;
                        }
                    }
                }
            }

            let runtime_overlay_owned = override_value.runtime_overlay_text.clone().or_else(|| {
                override_value
                    .capability_snapshot
                    .as_ref()
                    .map(crate::host_prompt::build_runtime_capability_snapshot_overlay)
            });
            let harness_overlay_owned = override_value
                .harness_delegation_overlay_text
                .clone()
                .or_else(|| {
                    override_value
                        .harness_overlay
                        .as_ref()
                        .map(crate::host_prompt::build_harness_delegation_overlay_text)
                });
            let surface_overlay_owned = override_value
                .surface_contract_overlay_text
                .clone()
                .or_else(|| {
                    override_value
                        .surface_contract
                        .as_ref()
                        .map(crate::host_prompt::build_surface_contract_overlay_text)
                });

            let overlay_input = crate::host_prompt::AgentPromptComposerInput {
                extensions: &[],
                custom_prompt: None,
                session_extra_instructions: override_value.extra_instructions.as_deref(),
                prompt_profile_overlay: override_value.prompt_profile_overlay.as_deref(),
                turn_system_instruction: override_value.turn_system_instruction.as_deref(),
                runtime_overlay_text: runtime_overlay_owned.as_deref(),
                harness_delegation_overlay_text: harness_overlay_owned.as_deref(),
                surface_contract_overlay_text: surface_overlay_owned.as_deref(),
                model_name: "",
            };
            if let Some(text) = crate::host_prompt::compose_session_overlays_only(overlay_input) {
                agent.extend_host_override_extras(text).await;
            }

            if let Some(text) = crate::host_workspace::merge_workspace_execution_instruction(
                None,
                override_value.workspace_context.as_ref(),
            ) {
                agent.extend_host_override_extras(text).await;
            }

            // When the override carried an explicit workspace context, cache
            // it on the session so the completion-mirror persist path can
            // reuse the same `run_id` advertised in the prompt.
            #[cfg(feature = "desktop_harness_host")]
            if let Some(ws) = override_value.workspace_context.as_ref() {
                let mut cache = state.workspace_execution_contexts.lock().await;
                cache.insert(session_id.clone(), ws.clone());
            }
        }

        #[cfg(feature = "desktop_harness_host")]
        {
            let needs_fallback = state
                .session_override(&session_id)
                .await
                .map(|o| o.workspace_context.is_none())
                .unwrap_or(true);
            if needs_fallback {
                let run_id = uuid::Uuid::new_v4().to_string();
                match state
                    .workspace_execution_context_for_session(&session_id, &run_id)
                    .await
                {
                    Ok(ctx) => {
                        if let Some(text) =
                            crate::host_workspace::merge_workspace_execution_instruction(
                                None,
                                Some(&ctx),
                            )
                        {
                            agent.extend_host_override_extras(text).await;
                        }
                    }
                    Err(err) => {
                        tracing::debug!(
                            "workspace context fallback skipped for session {}: {}",
                            session_id,
                            err
                        );
                    }
                }
            }
        }

        let user_message = match messages.last() {
            Some(msg) => msg,
            _ => {
                let _ = stream_event(
                    MessageEvent::Error {
                        error: "Reply started with empty messages".to_string(),
                    },
                    &task_tx,
                    &task_cancel,
                )
                .await;
                return;
            }
        };

        // Harness task handle, populated only on the desktop_harness_host
        // path, so the idle-timeout branch can abort a stalled core turn and
        // release its provider connection. Unused on the fallback build.
        #[cfg(feature = "desktop_harness_host")]
        let mut harness_handle: Option<tokio::task::JoinHandle<()>> = None;

        let stream_result = {
            #[cfg(feature = "desktop_harness_host")]
            {
                let host = crate::desktop_harness_host::DesktopHarnessHost::new(state.clone());
                let (control_tx, mut control_rx) =
                    mpsc::channel::<agime::agents::HarnessControlEnvelope>(64);

                // Forward harness control envelopes to the SSE stream as
                // `MessageEvent::HarnessControl` frames. Best-effort, never
                // blocks the agent stream; ends when the agent stream ends or
                // the cancel token fires.
                let mirror_tx = task_tx.clone();
                let mirror_cancel = task_cancel.clone();
                let mirror_state = state.clone();
                let mirror_session_id = session_id.clone();
                drop(tokio::spawn(async move {
                    while let Some(envelope) = control_rx.recv().await {
                        if mirror_cancel.is_cancelled() {
                            break;
                        }
                        // Intercept completion-report events to persist
                        // produced/accepted artifacts into the session
                        // workspace manifest. Best-effort: any failure is
                        // logged at debug level and never aborts the SSE
                        // stream. Persistence is dispatched to a detached
                        // task so blocking IO never delays the next mirror
                        // frame.
                        let pending_artifact_meta =
                            if let agime::agents::HarnessControlMessage::Completion(
                                agime::agents::CompletionControlEvent::StructuredPublished {
                                    metadata: Some(meta),
                                    ..
                                },
                            ) = &envelope.payload
                            {
                                Some(meta.clone())
                            } else {
                                None
                            };

                        stream_event(
                            MessageEvent::HarnessControl { envelope },
                            &mirror_tx,
                            &mirror_cancel,
                        )
                        .await;

                        if let Some(meta) = pending_artifact_meta {
                            let persist_state = mirror_state.clone();
                            let persist_session = mirror_session_id.clone();
                            drop(tokio::spawn(async move {
                                record_completion_report_artifacts(
                                    &persist_state,
                                    &persist_session,
                                    &meta,
                                )
                                .await;
                            }));
                        }
                    }
                }));

                let mirrored = host
                    .execute_chat_host_with_mirror_handle(
                        agent.clone(),
                        user_message.clone(),
                        session_config,
                        Some(task_cancel.clone()),
                        control_tx,
                    )
                    .await;
                // Keep `stream_result` the same `Result<BoxStream>` shape as the
                // fallback arm; stash the harness JoinHandle separately so the
                // idle-timeout path below can abort a stalled core turn.
                match mirrored {
                    Ok((stream, handle)) => {
                        harness_handle = Some(handle);
                        Ok(stream)
                    }
                    Err(e) => Err(e),
                }
            }
            #[cfg(not(feature = "desktop_harness_host"))]
            {
                agent
                    .reply(
                        user_message.clone(),
                        session_config,
                        Some(task_cancel.clone()),
                    )
                    .await
            }
        };

        let mut stream = match stream_result {
            Ok(stream) => {
                tracing::info!(
                    "[PERF] agent.reply() stream ready, elapsed: {:?}",
                    session_start.elapsed()
                );
                stream
            }
            Err(e) => {
                tracing::error!("Failed to start reply stream: {:?}", e);
                stream_event(
                    MessageEvent::Error {
                        error: e.to_string(),
                    },
                    &task_tx,
                    &cancel_token,
                )
                .await;
                return;
            }
        };

        // If this turn carries an image but the active model is not multimodal,
        // the shared provider formatter will silently swap the image for a text
        // placeholder. Emit a single user-visible inline notice so the drop is
        // not invisible. This only adds an SSE event on the desktop reply path;
        // it does not touch the shared formatter signatures.
        if conversation_has_image(messages.messages()) {
            let supports_multimodal = agent
                .provider()
                .await
                .map(|p| p.get_model_config().supports_multimodal)
                .unwrap_or(true);
            if !supports_multimodal {
                let notice = Message::assistant().with_system_notification(
                    SystemNotificationType::InlineMessage,
                    image_dropped_notice(),
                );
                let token_state = get_token_state(&session_id).await;
                stream_event(
                    MessageEvent::Message {
                        message: notice,
                        token_state,
                    },
                    &tx,
                    &cancel_token,
                )
                .await;
            }
        }

        let mut all_messages = messages.clone();
        let mut first_message_sent = false;

        // Desktop SSE idle watchdog: if the core harness streams no event for
        // `idle_ceiling`, abort the turn and surface an error instead of
        // spinning forever (a provider can accept a tool-bearing streaming
        // request and then return nothing). Reset on every event so long
        // answers and long local tool runs are never killed. `0s` disables it.
        #[cfg(feature = "desktop_harness_host")]
        let idle_ceiling =
            Duration::from_secs(crate::desktop_harness_host::stream_idle_timeout_secs_from_env());
        #[cfg(not(feature = "desktop_harness_host"))]
        let idle_ceiling = Duration::from_secs(0);
        let mut last_activity = std::time::Instant::now();
        // Set when we break due to an error we already surfaced, so the normal
        // post-loop `Finish` event is suppressed.
        let mut errored = false;

        let mut heartbeat_interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                _ = task_cancel.cancelled() => {
                    tracing::info!("Agent task cancelled");
                    break;
                }
                _ = heartbeat_interval.tick() => {
                    stream_event(MessageEvent::Ping, &tx, &cancel_token).await;
                }
                response = timeout(Duration::from_millis(500), stream.next()) => {
                    // Any `Ok(_)` means the upstream future resolved (event,
                    // stream error, or clean end) — i.e. the core harness is
                    // alive — so reset the idle watchdog. Only `Err(_)` (the
                    // 500ms poll timeout) leaves it ticking.
                    if response.is_ok() {
                        last_activity = std::time::Instant::now();
                    }
                    match response {
                        Ok(Some(Ok(AgentEvent::Message(message)))) => {
                            if !first_message_sent {
                                tracing::info!("[PERF] first SSE message event, elapsed: {:?}", session_start.elapsed());
                                first_message_sent = true;
                            }
                            for content in &message.content {
                                track_tool_telemetry(content, all_messages.messages());
                            }

                            all_messages.push(message.clone());

                            let token_state = get_token_state(&session_id).await;

                            stream_event(MessageEvent::Message { message, token_state }, &tx, &cancel_token).await;
                        }
                        Ok(Some(Ok(AgentEvent::HistoryReplaced(new_messages)))) => {
                            all_messages = new_messages.clone();
                            stream_event(MessageEvent::UpdateConversation {conversation: new_messages}, &tx, &cancel_token).await;

                        }
                        Ok(Some(Ok(AgentEvent::ModelChange { model, mode }))) => {
                            stream_event(MessageEvent::ModelChange { model, mode }, &tx, &cancel_token).await;
                        }
                        Ok(Some(Ok(AgentEvent::ToolTransportRequest(_)))) => {}
                        Ok(Some(Ok(AgentEvent::McpNotification((request_id, n))))) => {
                            stream_event(MessageEvent::Notification{
                                request_id: request_id.clone(),
                                message: n,
                            }, &tx, &cancel_token).await;
                        }

                        Ok(Some(Err(e))) => {
                            tracing::error!("Error processing message: {}", e);
                            stream_event(
                                MessageEvent::Error {
                                    error: e.to_string(),
                                },
                                &tx,
                                &cancel_token,
                            ).await;
                            break;
                        }
                        Ok(None) => {
                            break;
                        }
                        Err(_) => {
                            if tx.is_closed() {
                                break;
                            }
                            if !idle_ceiling.is_zero() && last_activity.elapsed() >= idle_ceiling {
                                tracing::warn!(
                                    "desktop SSE idle timeout after {:?}; aborting stalled turn",
                                    idle_ceiling
                                );
                                stream_event(
                                    MessageEvent::Error {
                                        error: "The model stopped responding (stream idle timeout). The request was aborted — please try again.".to_string(),
                                    },
                                    &tx,
                                    &cancel_token,
                                )
                                .await;
                                errored = true;
                                // Signal the core turn to cancel, then forcibly
                                // abort its background task so the hung future
                                // (parked on the provider SSE body) is dropped
                                // and its connection released.
                                task_cancel.cancel();
                                #[cfg(feature = "desktop_harness_host")]
                                if let Some(handle) = harness_handle.as_ref() {
                                    handle.abort();
                                }
                                break;
                            }
                            continue;
                        }
                    }
                }
            }
        }

        let session_duration = session_start.elapsed();

        if let Ok(session) = SessionManager::get_session(&session_id, true).await {
            let total_tokens = session.total_tokens.unwrap_or(0);
            tracing::info!(
                counter.goose.session_completions = 1,
                session_type = "app",
                interface = "ui",
                exit_type = "normal",
                duration_ms = session_duration.as_millis() as u64,
                total_tokens = total_tokens,
                message_count = session.message_count,
                "Session completed"
            );

            tracing::info!(
                counter.goose.session_duration_ms = session_duration.as_millis() as u64,
                session_type = "app",
                interface = "ui",
                "Session duration"
            );

            if total_tokens > 0 {
                tracing::info!(
                    counter.goose.session_tokens = total_tokens,
                    session_type = "app",
                    interface = "ui",
                    "Session tokens"
                );
            }
        } else {
            tracing::info!(
                counter.goose.session_completions = 1,
                session_type = "app",
                interface = "ui",
                exit_type = "normal",
                duration_ms = session_duration.as_millis() as u64,
                total_tokens = 0u64,
                message_count = all_messages.len(),
                "Session completed"
            );

            tracing::info!(
                counter.goose.session_duration_ms = session_duration.as_millis() as u64,
                session_type = "app",
                interface = "ui",
                "Session duration"
            );
        }

        let final_token_state = get_token_state(&session_id).await;

        // Skip the normal Finish event if we already surfaced an error (e.g.
        // idle timeout); the client treats Error as terminal, and a trailing
        // Finish would be redundant/confusing.
        if !errored {
            let _ = stream_event(
                MessageEvent::Finish {
                    reason: "stop".to_string(),
                    token_state: final_token_state,
                },
                &task_tx,
                &cancel_token,
            )
            .await;
        }
    }));
    Ok(SseResponse::new(stream))
}

/// RAII guard that releases the per-agent execution slot back to the desktop
/// admission provider when the chat reply task finishes — including panics
/// and cancellations. The release is intentionally fire-and-forget on a
/// detached tokio task because `Drop` cannot be async.
#[cfg(feature = "desktop_harness_host")]
struct ExecutionSlotGuard {
    provider: Arc<crate::host_admission::DesktopExecutionSlotProvider>,
    agent_id: String,
}

#[cfg(feature = "desktop_harness_host")]
impl Drop for ExecutionSlotGuard {
    fn drop(&mut self) {
        use crate::host_admission::ExecutionSlotProvider;
        let provider = self.provider.clone();
        let agent_id = std::mem::take(&mut self.agent_id);
        tokio::spawn(async move {
            if let Err(e) = provider.release_execution_slot(&agent_id).await {
                tracing::warn!("execution slot release failed for {}: {}", agent_id, e);
            }
        });
    }
}

#[cfg(feature = "desktop_harness_host")]
async fn record_completion_report_artifacts(
    state: &Arc<AppState>,
    session_id: &str,
    metadata: &serde_json::Value,
) {
    use crate::host_helpers::normalize_adapter_execution_host_completion_report;
    use agime::agents::parse_execution_host_completion_report;

    // The metadata Value may arrive in three shapes:
    //   1. Value::String containing a JSON-encoded report (team-server style)
    //   2. Value::Object — the report itself, structured in-band
    //   3. Anything else — give up.
    let report_opt = match metadata {
        serde_json::Value::String(text) => parse_execution_host_completion_report(Some(text)),
        serde_json::Value::Object(_) => {
            serde_json::from_value::<agime::agents::ExecutionHostCompletionReport>(metadata.clone())
                .ok()
                .or_else(|| parse_execution_host_completion_report(Some(&metadata.to_string())))
        }
        _ => None,
    };
    let Some(report) = report_opt else {
        tracing::debug!(
            "completion metadata for session {} is not a parseable host report",
            session_id
        );
        return;
    };
    let report = normalize_adapter_execution_host_completion_report(report, None, "chat");

    if report.produced_artifacts.is_empty() && report.accepted_artifacts.is_empty() {
        return;
    }

    let binding = match state.ensure_chat_workspace_binding(session_id).await {
        Ok(b) => b,
        Err(err) => {
            tracing::debug!(
                "skip artifact recording for {}: workspace binding error: {}",
                session_id,
                err
            );
            return;
        }
    };
    let svc = state.workspace_service().await;
    if let Err(err) = svc.record_completion_artifacts(
        &binding,
        &report.produced_artifacts,
        &report.accepted_artifacts,
    ) {
        tracing::debug!(
            "record_completion_artifacts failed for session {}: {}",
            session_id,
            err
        );
        return;
    }
    if let Err(err) = svc.reconcile_manifest_artifacts(&binding) {
        tracing::debug!(
            "reconcile_manifest_artifacts failed for session {}: {}",
            session_id,
            err
        );
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/reply",
            post(reply).layer(DefaultBodyLimit::max(50 * 1024 * 1024)),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod integration_tests {
        use super::*;
        use agime::conversation::message::Message;
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;

        #[tokio::test(flavor = "multi_thread")]
        async fn test_reply_endpoint() {
            let state = AppState::new().await.unwrap();

            let app = routes(state);

            let request = Request::builder()
                .uri("/reply")
                .method("POST")
                .header("content-type", "application/json")
                .header("x-secret-key", "test-secret")
                .body(Body::from(
                    serde_json::to_string(&ChatRequest {
                        messages: vec![Message::user().with_text("test message")],
                        session_id: "test-session".to_string(),
                        recipe_name: None,
                        recipe_version: None,
                    })
                    .unwrap(),
                ))
                .unwrap();

            let response = app.oneshot(request).await.unwrap();

            assert_eq!(response.status(), StatusCode::OK);
        }
    }
}
