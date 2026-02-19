//! Chat API routes (Phase 1 - Chat Track)
//!
//! These routes handle direct chat sessions that bypass the Task system.
//! Mounted at `/api/team/agent/chat`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, Sse},
        Json,
    },
    routing::{delete, get, post, put},
    Extension, Router,
};
use futures::stream::Stream;
use std::convert::Infallible;
use std::time::Duration;
use std::sync::Arc;

use crate::auth::middleware::UserContext;
use agime::agents::types::{RetryConfig, SuccessCheck};
use agime_team::models::BuiltinExtension;
use agime_team::MongoDb;

use super::chat_executor::ChatExecutor;
use super::chat_manager::ChatManager;
use super::service_mongo::AgentService;
use super::normalize_workspace_path;
use super::session_mongo::{
    CreateChatSessionRequest, SendChatMessageRequest, SendMessageResponse, SessionListItem,
    UserSessionListQuery,
};
use agime_team::services::mongo::PortalService;

type ChatState = (Arc<AgentService>, Arc<MongoDb>, Arc<ChatManager>, String);

#[derive(serde::Deserialize)]
struct StreamQuery {
    last_event_id: Option<u64>,
}

fn default_portal_retry_config() -> RetryConfig {
    let check_command = if cfg!(windows) {
        "if exist index.html (exit /b 0) else (echo index.html not found & exit /b 1)".to_string()
    } else {
        "[ -f index.html ]".to_string()
    };
    RetryConfig {
        max_retries: 6,
        checks: vec![SuccessCheck::Shell {
            command: check_command,
        }],
        on_failure: None,
        timeout_seconds: Some(180),
        on_failure_timeout_seconds: Some(300),
    }
}

/// Create chat router
pub fn chat_router(
    db: Arc<MongoDb>,
    chat_manager: Arc<ChatManager>,
    workspace_root: String,
) -> Router {
    let service = Arc::new(AgentService::new(db.clone()));

    Router::new()
        .route("/sessions", get(list_sessions))
        .route("/sessions", post(create_session))
        .route(
            "/sessions/portal-coding",
            post(create_portal_coding_session),
        )
        .route("/sessions/{id}", get(get_session))
        .route("/sessions/{id}", put(update_session))
        .route("/sessions/{id}", delete(delete_session))
        .route("/sessions/{id}/messages", post(send_message))
        .route("/sessions/{id}/stream", get(stream_chat))
        .route("/sessions/{id}/cancel", post(cancel_chat))
        .route("/sessions/{id}/archive", post(archive_session))
        // Phase 2: Document attachment
        .route(
            "/sessions/{id}/documents",
            get(list_attached_documents)
                .post(attach_documents)
                .delete(detach_documents),
        )
        .with_state((service, db, chat_manager, workspace_root))
}

/// GET /chat/sessions - List user's chat sessions
async fn list_sessions(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Query(mut query): Query<UserSessionListQuery>,
) -> Result<Json<Vec<SessionListItem>>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    // C1 fix: Always inject authenticated user_id to prevent data leakage
    query.user_id = Some(user.user_id.clone());

    service
        .list_user_sessions(query)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to list sessions: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

/// POST /chat/sessions - Create a new chat session
async fn create_session(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<CreateChatSessionRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Look up agent to get team_id
    let team_id = service
        .get_agent_team_id(&req.agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    let session = service
        .create_chat_session(
            &team_id,
            &req.agent_id,
            &user.user_id,
            req.attached_document_ids,
            req.extra_instructions,
            req.allowed_extensions,
            req.allowed_skill_ids,
            req.retry_config,
            req.max_turns,
            req.tool_timeout_seconds,
            req.max_portal_retry_rounds,
            req.require_final_report,
            req.portal_restricted,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to create chat session: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(serde_json::json!({
        "session_id": session.session_id,
        "agent_id": session.agent_id,
        "status": session.status,
    })))
}

#[derive(serde::Deserialize)]
struct CreatePortalCodingSessionRequest {
    team_id: String,
    portal_id: String,
    #[serde(default)]
    retry_config: Option<RetryConfig>,
    #[serde(default)]
    max_turns: Option<i32>,
    #[serde(default)]
    tool_timeout_seconds: Option<u64>,
    #[serde(default)]
    max_portal_retry_rounds: Option<u32>,
    #[serde(default)]
    require_final_report: Option<bool>,
}

/// POST /chat/sessions/portal-coding - Create a portal lab coding session with strict policy.
async fn create_portal_coding_session(
    State((service, db, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<CreatePortalCodingSessionRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    let portal_svc = PortalService::new((*db).clone());
    let portal = portal_svc
        .get(&req.team_id, &req.portal_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let portal_id = portal
        .id
        .map(|id| id.to_hex())
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    let agent_id = portal
        .coding_agent_id
        .clone()
        .or_else(|| portal.agent_id.clone())
        .or_else(|| portal.service_agent_id.clone())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let raw_project_path = portal.project_path.clone().ok_or(StatusCode::BAD_REQUEST)?;
    let project_path = normalize_workspace_path(&raw_project_path);
    let portal_slug = portal.slug.clone();

    if project_path != raw_project_path {
        if let Err(e) = portal_svc
            .set_project_path(&req.team_id, &portal_id, &project_path)
            .await
        {
            tracing::warn!(
                "Failed to normalize project_path for portal {}: {}",
                portal_id,
                e
            );
        }
    }

    // Ensure project directory exists; auto-create if missing
    if !std::path::Path::new(&project_path).exists() {
        tracing::warn!("Portal project_path missing, recreating: {}", project_path);
        if let Err(e) = std::fs::create_dir_all(&project_path) {
            tracing::error!("Failed to create project dir {}: {}", project_path, e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    // Ensure selected coding agent can actually run developer tools.
    let agent = service
        .get_agent(&agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let has_developer_builtin = agent
        .enabled_extensions
        .iter()
        .any(|ext| ext.enabled && ext.extension == BuiltinExtension::Developer);
    let has_developer_custom = agent
        .custom_extensions
        .iter()
        .any(|ext| ext.enabled && ext.name.trim().eq_ignore_ascii_case("developer"));
    if !has_developer_builtin && !has_developer_custom {
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut extra = String::new();
    if let Some(prompt) = portal.agent_system_prompt.clone() {
        let trimmed = prompt.trim();
        if !trimmed.is_empty() {
            extra.push_str(trimmed);
            extra.push_str("\n\n");
        }
    }
    extra.push_str(&format!("Portal 编程会话 | slug: {} | 目录: {}\n", portal_slug, project_path));
    extra.push_str("规则：\n");
    extra.push_str("1. 仅在项目目录下操作，text_editor 用相对路径（如 index.html），shell 工作目录已自动设置。\n");
    extra.push_str("2. 必须调用 developer 工具实际修改文件，不要只输出方案。\n");
    extra.push_str("3. 完成后汇报改动文件和预览地址。\n");
    extra.push_str("4. 不要执行 create_portal/publish_portal 等门户管理动作。\n");
    extra.push_str("5. _private/ 目录存放服务端数据（JSON key-value），前端通过 SDK 的 data API 访问，静态文件服务不会暴露此目录。\n");
    extra.push_str("\nPortal SDK（portal-sdk.js）前端可用 API：\n");
    extra.push_str("- sdk.chat: createSession() / sendMessage(sid, text) / subscribe(sid) → EventSource / cancel(sid) / listSessions()\n");
    extra.push_str("- sdk.docs: list() / get(docId) / getMeta(docId) / poll(docId, ms, cb) — 只读，仅绑定文档\n");
    extra.push_str("- sdk.data: list() / get(key) / set(key, value) — _private/ 下的 key-value 存储\n");
    extra.push_str("- sdk.config.get() / sdk.track(type, payload)\n");
    extra.push_str("用法: <script src=\"portal-sdk.js\"></script> → const sdk = new PortalSDK({ slug: '...' });\n");

    // Inject project directory context so the agent knows the current state
    let project_ctx = super::runtime::scan_project_context(&project_path, 8000);
    if !project_ctx.is_empty() {
        extra.push('\n');
        extra.push_str(&project_ctx);
        extra.push('\n');
    }

    let allowed_extensions = vec!["developer".to_string(), "document_tools".to_string()];
    let effective_retry_config = req
        .retry_config
        .clone()
        .unwrap_or_else(default_portal_retry_config);

    let session = service
        .create_chat_session(
            &req.team_id,
            &agent_id,
            &user.user_id,
            portal.bound_document_ids.clone(),
            Some(extra),
            Some(allowed_extensions.clone()),
            None,
            Some(effective_retry_config),
            req.max_turns,
            req.tool_timeout_seconds,
            req.max_portal_retry_rounds,
            req.require_final_report.unwrap_or(true),
            true,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to create portal coding session: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    service
        .set_session_workspace(&session.session_id, &project_path)
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to set workspace for portal coding session {}: {:?}",
                session.session_id,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    service
        .set_session_portal_context(&session.session_id, &portal_id, &portal_slug, None)
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to set portal context for portal coding session {}: {:?}",
                session.session_id,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(serde_json::json!({
        "session_id": session.session_id,
        "agent_id": session.agent_id,
        "status": session.status,
        "portal_restricted": true,
        "workspace_path": project_path,
        "allowed_extensions": allowed_extensions,
        "retry_config": session.retry_config,
        "max_turns": session.max_turns,
        "tool_timeout_seconds": session.tool_timeout_seconds,
        "max_portal_retry_rounds": session.max_portal_retry_rounds,
        "require_final_report": session.require_final_report,
    })))
}

/// GET /chat/sessions/{id} - Get session details with messages
async fn get_session(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Verify ownership
    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    // H4 fix: Convert bson::DateTime to ISO 8601 strings for frontend
    let mut json = serde_json::json!({
        "session_id": session.session_id,
        "team_id": session.team_id,
        "agent_id": session.agent_id,
        "user_id": session.user_id,
        "name": session.name,
        "status": session.status,
        "messages_json": session.messages_json,
        "message_count": session.message_count,
        "total_tokens": session.total_tokens,
        "input_tokens": session.input_tokens,
        "output_tokens": session.output_tokens,
        "compaction_count": session.compaction_count,
        "compaction_strategy": session.compaction_strategy,
        "disabled_extensions": session.disabled_extensions,
        "enabled_extensions": session.enabled_extensions,
        "created_at": session.created_at.to_chrono().to_rfc3339(),
        "updated_at": session.updated_at.to_chrono().to_rfc3339(),
        "title": session.title,
        "pinned": session.pinned,
        "last_message_preview": session.last_message_preview,
        "is_processing": session.is_processing,
        "workspace_path": session.workspace_path,
        "extra_instructions": session.extra_instructions,
        "allowed_extensions": session.allowed_extensions,
        "allowed_skill_ids": session.allowed_skill_ids,
        "retry_config": session.retry_config,
        "max_turns": session.max_turns,
        "tool_timeout_seconds": session.tool_timeout_seconds,
        "max_portal_retry_rounds": session.max_portal_retry_rounds,
        "require_final_report": session.require_final_report,
        "portal_restricted": session.portal_restricted,
        "portal_id": session.portal_id,
        "portal_slug": session.portal_slug,
        "visitor_id": session.visitor_id,
    });

    if let Some(lma) = session.last_message_at {
        json["last_message_at"] = serde_json::Value::String(lma.to_chrono().to_rfc3339());
    }

    Ok(Json(json))
}

/// PUT /chat/sessions/{id} - Update session (rename/pin)
#[derive(serde::Deserialize)]
struct UpdateSessionBody {
    title: Option<String>,
    pinned: Option<bool>,
}

async fn update_session(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Json(body): Json<UpdateSessionBody>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    if let Some(title) = &body.title {
        service
            .rename_session(&session_id, title)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(pinned) = body.pinned {
        service
            .pin_session(&session_id, pinned)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(StatusCode::OK)
}

/// POST /chat/sessions/{id}/messages - Send a message (triggers execution)
async fn send_message(
    State((service, db, chat_manager, ref workspace_root)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Json(req): Json<SendChatMessageRequest>,
) -> Result<Json<SendMessageResponse>, StatusCode> {
    // M7: Validate content is not empty or too long
    let content = req.content.trim().to_string();
    if content.is_empty() || content.len() > 100_000 {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Verify session exists and user owns it
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    // Register in ChatManager first (authoritative in-memory gate)
    let (cancel_token, _stream_tx) = match chat_manager.register(&session_id).await {
        Some(pair) => pair,
        None => return Err(StatusCode::CONFLICT),
    };

    // Then set MongoDB is_processing flag (secondary persistence)
    let claimed = service
        .try_start_processing(&session_id, &user.user_id)
        .await
        .map_err(|e| {
            tracing::error!("try_start_processing DB error for {}: {}", session_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        });
    match claimed {
        Ok(true) => {}
        _ => {
            // Rollback ChatManager registration
            chat_manager.unregister(&session_id).await;
            return Err(claimed.err().unwrap_or(StatusCode::CONFLICT));
        }
    }

    // Spawn background execution
    let executor = ChatExecutor::new(db.clone(), chat_manager.clone(), workspace_root.clone());
    let sid = session_id.clone();
    let agent_id = session.agent_id.clone();

    tokio::spawn(async move {
        if let Err(e) = executor
            .execute_chat(&sid, &agent_id, &content, cancel_token)
            .await
        {
            tracing::error!("Chat execution failed for session {}: {}", sid, e);
        }
    });

    Ok(Json(SendMessageResponse {
        session_id,
        streaming: true,
    }))
}

/// GET /chat/sessions/{id}/stream - SSE stream for chat events
async fn stream_chat(
    State((service, _, chat_manager, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(q): Query<StreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    // Verify ownership
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    let last_event_id = q.last_event_id.or_else(|| {
        headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
    });

    // Subscribe to chat events with buffered history for reconnect/late join.
    let (mut rx, history) = chat_manager
        .subscribe_with_history(&session_id, last_event_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let stream = async_stream::stream! {
        yield Ok(Event::default()
            .event("status")
            .data(serde_json::json!({
                "type": "Status",
                "status": "running"
            }).to_string()));

        for event in history {
            let is_done = event.event.is_done();
            let json = serde_json::to_string(&event.event).unwrap_or_default();
            let mut sse = Event::default().event(event.event.event_type()).data(json);
            if event.id > 0 {
                sse = sse.id(event.id.to_string());
            }
            yield Ok(sse);
            if is_done {
                return;
            }
        }

        let sse_lifetime_secs = std::env::var("TEAM_SSE_MAX_LIFETIME_SECS")
            .ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(2 * 60 * 60);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(sse_lifetime_secs);

        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(event)) => {
                    let is_done = event.event.is_done();
                    let json = serde_json::to_string(&event.event).unwrap_or_default();
                    let mut sse = Event::default().event(event.event.event_type()).data(json);
                    if event.id > 0 {
                        sse = sse.id(event.id.to_string());
                    }
                    yield Ok(sse);
                    if is_done {
                        break;
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Err(_) => {
                    tracing::info!("SSE stream deadline reached, closing for client reconnect");
                    break;
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    ))
}

/// POST /chat/sessions/{id}/cancel - Cancel active chat
async fn cancel_chat(
    State((service, _, chat_manager, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    let cancelled = chat_manager.cancel(&session_id).await;
    if cancelled {
        let _ = service.set_session_processing(&session_id, false).await;
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// POST /chat/sessions/{id}/archive - Archive session
async fn archive_session(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    // M8: Atomic archive — only succeeds if session is not processing
    let archived = service
        .archive_session_if_idle(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if archived {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::CONFLICT)
    }
}

/// DELETE /chat/sessions/{id} - Permanently delete session
async fn delete_session(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    // M8: Atomic delete — only succeeds if session is not processing
    let deleted = service
        .delete_session_if_idle(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if deleted {
        // P2: Best-effort workspace cleanup (after DB delete to avoid orphaned records)
        if let Err(e) = super::runtime::cleanup_workspace_dir(session.workspace_path.as_deref()) {
            tracing::warn!(
                "Failed to cleanup workspace for session {}: {}",
                session_id,
                e
            );
        }
        Ok(StatusCode::NO_CONTENT)
    } else {
        // Session was verified above but disappeared before delete — concurrent deletion
        Err(StatusCode::CONFLICT)
    }
}

// ── Phase 2: Document attachment routes ──

#[derive(serde::Deserialize)]
struct DocumentIdsBody {
    document_ids: Vec<String>,
}

/// POST /chat/sessions/{id}/documents - Attach documents
async fn attach_documents(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Json(body): Json<DocumentIdsBody>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .attach_documents_to_session(&session_id, &body.document_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

/// DELETE /chat/sessions/{id}/documents - Detach documents
async fn detach_documents(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Json(body): Json<DocumentIdsBody>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .detach_documents_from_session(&session_id, &body.document_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /chat/sessions/{id}/documents - List attached documents
async fn list_attached_documents(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(Json(session.attached_document_ids))
}
