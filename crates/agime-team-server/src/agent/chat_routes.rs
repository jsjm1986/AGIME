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
use std::sync::Arc;
use std::time::Duration;

use crate::auth::middleware::UserContext;
use agime::agents::types::{RetryConfig, SuccessCheck};
use agime_team::models::{BuiltinExtension, ListAgentsQuery, TeamAgent};
use agime_team::MongoDb;

use super::chat_executor::ChatExecutor;
use super::chat_manager::ChatManager;
use super::normalize_workspace_path;
use super::prompt_profiles::{
    build_portal_coding_overlay, build_portal_manager_overlay, PortalCodingProfileInput,
};
use super::service_mongo::AgentService;
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

#[derive(serde::Deserialize)]
struct EventListQuery {
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    after_event_id: Option<u64>,
    #[serde(default)]
    before_event_id: Option<u64>,
    #[serde(default)]
    order: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
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
        .route(
            "/sessions/portal-manager",
            post(create_portal_manager_session),
        )
        .route("/sessions/{id}", get(get_session))
        .route("/sessions/{id}", put(update_session))
        .route("/sessions/{id}", delete(delete_session))
        .route("/sessions/{id}/messages", post(send_message))
        .route("/sessions/{id}/stream", get(stream_chat))
        .route("/sessions/{id}/events", get(list_session_events))
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
    State((service, db, _, _)): State<ChatState>,
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

    // Enforce agent group-based access control
    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new((*db).clone())
            .get_user_group_ids(&team_id, &user.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let has_agent_access = service
        .check_agent_access(&req.agent_id, &user.user_id, &user_group_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !has_agent_access {
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
            req.document_access_mode,
            None,
            None,
            None,
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

#[derive(serde::Deserialize)]
struct CreatePortalManagerSessionRequest {
    team_id: String,
    #[serde(default)]
    manager_agent_id: Option<String>,
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

fn has_manager_tooling(agent: &TeamAgent) -> bool {
    let builtin = agent.enabled_extensions.iter().any(|ext| {
        ext.enabled
            && matches!(
                ext.extension,
                BuiltinExtension::Developer
                    | BuiltinExtension::ExtensionManager
                    | BuiltinExtension::Team
            )
    });
    let custom = agent.custom_extensions.iter().any(|ext| {
        ext.enabled
            && matches!(
                ext.name.trim().to_ascii_lowercase().as_str(),
                "developer" | "portal_tools" | "extension_manager" | "team"
            )
    });
    builtin || custom
}

async fn resolve_manager_agent_id(
    service: &AgentService,
    team_id: &str,
    manager_agent_id: Option<&str>,
) -> Result<String, StatusCode> {
    let requested = manager_agent_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    if let Some(agent_id) = requested {
        let agent = service
            .get_agent(&agent_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        if agent.team_id != team_id {
            return Err(StatusCode::FORBIDDEN);
        }
        return Ok(agent_id);
    }

    let agents = service
        .list_agents(ListAgentsQuery {
            team_id: team_id.to_string(),
            page: 1,
            limit: 100,
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if agents.items.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    if let Some(agent) = agents.items.iter().find(|agent| has_manager_tooling(agent)) {
        return Ok(agent.id.clone());
    }

    Ok(agents.items[0].id.clone())
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
    let is_admin = service
        .is_team_admin(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_admin {
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

    // Enforce agent group-based access control
    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new((*db).clone())
            .get_user_group_ids(&req.team_id, &user.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let has_agent_access = service
        .check_agent_access(&agent_id, &user.user_id, &user_group_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !has_agent_access {
        return Err(StatusCode::FORBIDDEN);
    }

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

    // Inject project directory context so the agent knows the current state
    let project_ctx = super::runtime::scan_project_context(&project_path, 8000);
    let portal_policy_overlay = portal.agent_system_prompt.clone();
    let extra = build_portal_coding_overlay(PortalCodingProfileInput {
        portal_slug: &portal_slug,
        project_path: &project_path,
        portal_policy_overlay: portal_policy_overlay.as_deref(),
        project_context: if project_ctx.trim().is_empty() {
            None
        } else {
            Some(project_ctx.as_str())
        },
    });

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
            None,
            None,
            Some(effective_retry_config),
            req.max_turns,
            req.tool_timeout_seconds,
            req.max_portal_retry_rounds,
            req.require_final_report.unwrap_or(false),
            false,
            Some("full".to_string()),
            Some("portal_coding".to_string()),
            None,
            Some(true),
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
        .set_session_portal_context(
            &session.session_id,
            &portal_id,
            &portal_slug,
            None,
            Some("full"),
            false,
        )
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
        "portal_restricted": false,
        "workspace_path": project_path,
        "allowed_extensions": serde_json::Value::Null,
        "retry_config": session.retry_config,
        "max_turns": session.max_turns,
        "tool_timeout_seconds": session.tool_timeout_seconds,
        "max_portal_retry_rounds": session.max_portal_retry_rounds,
        "require_final_report": session.require_final_report,
    })))
}

/// POST /chat/sessions/portal-manager - Create team-level portal manager session.
/// This session is used to create/configure digital avatars before any portal exists.
async fn create_portal_manager_session(
    State((service, db, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<CreatePortalManagerSessionRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }
    let is_admin = service
        .is_team_admin(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let manager_agent_id =
        resolve_manager_agent_id(&service, &req.team_id, req.manager_agent_id.as_deref()).await?;

    // Enforce agent group-based access control
    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new((*db).clone())
            .get_user_group_ids(&req.team_id, &user.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let has_agent_access = service
        .check_agent_access(&manager_agent_id, &user.user_id, &user_group_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !has_agent_access {
        return Err(StatusCode::FORBIDDEN);
    }

    let extra = build_portal_manager_overlay();

    let effective_retry_config = req
        .retry_config
        .clone()
        .unwrap_or_else(default_portal_retry_config);

    let session = service
        .create_chat_session(
            &req.team_id,
            &manager_agent_id,
            &user.user_id,
            Vec::new(),
            Some(extra),
            None,
            None,
            Some(effective_retry_config),
            req.max_turns,
            req.tool_timeout_seconds,
            req.max_portal_retry_rounds,
            req.require_final_report.unwrap_or(false),
            false,
            Some("full".to_string()),
            Some("portal_manager".to_string()),
            None,
            Some(true),
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to create portal manager session: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(serde_json::json!({
        "session_id": session.session_id,
        "agent_id": session.agent_id,
        "status": session.status,
        "portal_restricted": false,
        "allowed_extensions": serde_json::Value::Null,
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
        "document_access_mode": session.document_access_mode,
        "portal_id": session.portal_id,
        "portal_slug": session.portal_slug,
        "visitor_id": session.visitor_id,
        "session_source": session.session_source,
        "source_mission_id": session.source_mission_id,
        "hidden_from_chat_list": session.hidden_from_chat_list,
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

fn fix_bson_dates(val: &mut serde_json::Value) {
    match val {
        serde_json::Value::Object(map) => {
            if map.len() == 1 && map.contains_key("$date") {
                if let Some(date_val) = map.get("$date") {
                    if let Some(date_obj) = date_val.as_object() {
                        if let Some(ms) = date_obj.get("$numberLong").and_then(|v| v.as_str()) {
                            if let Ok(ts) = ms.parse::<i64>() {
                                if let Some(dt) =
                                    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts)
                                {
                                    *val = serde_json::Value::String(dt.to_rfc3339());
                                    return;
                                }
                            }
                        }
                    }
                    if let Some(s) = date_val.as_str() {
                        *val = serde_json::Value::String(s.to_string());
                        return;
                    }
                }
            }
            for v in map.values_mut() {
                fix_bson_dates(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                fix_bson_dates(v);
            }
        }
        _ => {}
    }
}

/// GET /chat/sessions/{id}/events - List persisted runtime events.
async fn list_session_events(
    State((service, _, chat_manager, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Query(q): Query<EventListQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        let is_admin = service
            .is_team_admin(&user.user_id, &session.team_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if !is_admin {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    let limit = q.limit.unwrap_or(500).clamp(1, 2000);
    let descending = q
        .order
        .as_deref()
        .map(str::trim)
        .map(|v| v.eq_ignore_ascii_case("desc"))
        .unwrap_or(false);
    let explicit_run_id = q.run_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let selected_run_id: Option<String> = match explicit_run_id {
        Some(rid)
            if rid.eq_ignore_ascii_case("__all__")
                || rid.eq_ignore_ascii_case("all")
                || rid == "*" =>
        {
            None
        }
        Some(rid) => Some(rid.to_string()),
        None => chat_manager.active_run_id(&session_id).await,
    };

    let events = service
        .list_chat_events(
            &session_id,
            selected_run_id.as_deref(),
            q.after_event_id,
            q.before_event_id,
            limit,
            descending,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to list chat events for {}: {:?}", session_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let mut value = serde_json::to_value(events).unwrap_or_default();
    fix_bson_dates(&mut value);
    Ok(Json(value))
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
