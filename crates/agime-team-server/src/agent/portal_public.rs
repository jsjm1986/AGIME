//! Portal public routes — unauthenticated endpoints for serving published portals
//!
//! Mounted BEFORE auth middleware so external visitors can access published pages.
//! Phase 3: Real Agent chat via SSE + Markdown page rendering.

use agime_team::db::MongoDb;
use agime_team::models::mongo::{InteractionType, Portal, PortalInteraction, PortalStatus};
use agime_team::services::mongo::{DocumentService, PortalService};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    routing::{get, post, put},
    Json, Router,
};
use chrono::Utc;
use futures::stream::Stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::path::{Component, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::chat_executor::ChatExecutor;
use super::chat_manager::ChatManager;
use super::service_mongo::AgentService;
use super::normalize_workspace_path;

/// State for portal public routes
#[derive(Clone)]
pub struct PortalPublicState {
    pub db: Arc<MongoDb>,
    pub chat_manager: Arc<ChatManager>,
    pub workspace_root: String,
}

pub fn portal_public_routes(
    db: Arc<MongoDb>,
    chat_manager: Arc<ChatManager>,
    workspace_root: String,
) -> Router {
    let state = PortalPublicState {
        db,
        chat_manager,
        workspace_root,
    };
    Router::new()
        .route("/p/{slug}", get(serve_portal_index))
        .route("/p/{slug}/{*path}", get(serve_portal_page))
        .route("/p/{slug}/api/interact", post(log_interaction))
        .route("/p/{slug}/api/config", get(portal_config))
        // Phase 3: Public chat routes
        .route("/p/{slug}/api/chat/session", post(create_visitor_session))
        .route("/p/{slug}/api/chat/message", post(send_visitor_message))
        .route(
            "/p/{slug}/api/chat/stream/{session_id}",
            get(stream_visitor_chat),
        )
        // Data API (key-value storage in _private/)
        .route("/p/{slug}/api/data", get(list_data_keys))
        .route("/p/{slug}/api/data/{key}", get(get_data).put(set_data))
        // Document bridge API (read-only, bound documents only)
        .route("/p/{slug}/api/docs", get(list_bound_documents))
        .route("/p/{slug}/api/docs/{doc_id}", get(get_bound_document))
        .route("/p/{slug}/api/docs/{doc_id}/meta", get(get_bound_document_meta))
        // Chat enhancements
        .route("/p/{slug}/api/chat/cancel", post(cancel_visitor_chat))
        .route("/p/{slug}/api/chat/sessions", get(list_visitor_sessions))
        .with_state(state)
}

/// Self-contained chat widget with real Agent SSE streaming
fn render_chat_widget(slug: &str, welcome_message: Option<&str>) -> String {
    let welcome = welcome_message.unwrap_or("Hi! How can I help you?");
    format!(
        r##"<div id="portal-chat-widget">
<style>
#pcw-btn{{position:fixed;bottom:20px;right:20px;width:56px;height:56px;border-radius:50%;background:#2563eb;color:#fff;border:none;cursor:pointer;font-size:24px;box-shadow:0 4px 12px rgba(0,0,0,.15);z-index:9999;display:flex;align-items:center;justify-content:center}}
#pcw-panel{{position:fixed;bottom:88px;right:20px;width:380px;max-height:520px;background:#fff;border-radius:12px;box-shadow:0 8px 30px rgba(0,0,0,.12);z-index:9999;display:none;flex-direction:column;overflow:hidden}}
#pcw-header{{background:#2563eb;color:#fff;padding:14px 16px;font-weight:600;display:flex;justify-content:space-between;align-items:center}}
#pcw-header button{{background:none;border:none;color:#fff;cursor:pointer;font-size:18px}}
#pcw-messages{{flex:1;overflow-y:auto;padding:12px;min-height:300px;max-height:380px}}
.pcw-msg{{margin:6px 0;padding:8px 12px;border-radius:8px;max-width:85%;word-wrap:break-word;font-size:14px;line-height:1.5;white-space:pre-wrap}}
.pcw-msg.bot{{background:#f0f4ff;align-self:flex-start}}
.pcw-msg.user{{background:#2563eb;color:#fff;margin-left:auto}}
.pcw-msg.thinking{{background:#fef3c7;font-style:italic;font-size:12px}}
.pcw-typing{{margin:6px 0;padding:8px 12px;color:#6b7280;font-size:13px}}
#pcw-input-row{{display:flex;border-top:1px solid #e5e7eb;padding:8px}}
#pcw-input{{flex:1;border:none;outline:none;padding:8px 12px;font-size:14px}}
#pcw-send{{background:#2563eb;color:#fff;border:none;padding:8px 16px;cursor:pointer;font-weight:600;border-radius:6px}}
#pcw-send:disabled{{opacity:0.5;cursor:not-allowed}}
</style>
<button id="pcw-btn" onclick="pcwToggle()" aria-label="Chat">💬</button>
<div id="pcw-panel">
  <div id="pcw-header"><span>Chat</span><button onclick="pcwToggle()">✕</button></div>
  <div id="pcw-messages" style="display:flex;flex-direction:column"></div>
  <div id="pcw-input-row">
    <input id="pcw-input" placeholder="Type a message..." onkeydown="if(event.key==='Enter'&&!event.shiftKey)pcwSend()">
    <button id="pcw-send" onclick="pcwSend()">Send</button>
  </div>
</div>
<script>
(function(){{
  var SLUG="{slug}";
  var vid=localStorage.getItem('pcw_vid');
  if(!vid){{vid='v_'+Math.random().toString(36).substr(2,9);localStorage.setItem('pcw_vid',vid)}}
  var sessionId=sessionStorage.getItem('pcw_sid_'+SLUG)||'';
  var msgs=JSON.parse(sessionStorage.getItem('pcw_msgs_'+SLUG)||'[]');
  var panel=document.getElementById('pcw-panel');
  var msgBox=document.getElementById('pcw-messages');
  var sendBtn=document.getElementById('pcw-send');
  var busy=false;
  var evtSource=null;
  var currentBotEl=null;
  var currentBotText='';
  var lastEventId='';

  function saveMsgs(){{sessionStorage.setItem('pcw_msgs_'+SLUG,JSON.stringify(msgs))}}

  function render(){{
    msgBox.innerHTML='';
    if(msgs.length===0){{
      var w=document.createElement('div');w.className='pcw-msg bot';w.textContent={welcome_js};
      msgBox.appendChild(w);
    }}
    msgs.forEach(function(m){{
      var d=document.createElement('div');
      d.className='pcw-msg '+(m.role==='user'?'user':'bot');
      d.textContent=m.content;msgBox.appendChild(d);
    }});
    msgBox.scrollTop=msgBox.scrollHeight;
  }}
  render();

  function setBusy(b){{
    busy=b;
    sendBtn.disabled=b;
    document.getElementById('pcw-input').disabled=b;
  }}

  function addTyping(){{
    var el=document.createElement('div');el.className='pcw-typing';el.id='pcw-typing';
    el.textContent='Thinking...';msgBox.appendChild(el);msgBox.scrollTop=msgBox.scrollHeight;
  }}
  function removeTyping(){{var el=document.getElementById('pcw-typing');if(el)el.remove()}}

  function ensureSession(cb){{
    if(sessionId)return cb();
    fetch('/p/'+SLUG+'/api/chat/session',{{
      method:'POST',headers:{{'Content-Type':'application/json'}},
      body:JSON.stringify({{visitor_id:vid}})
    }}).then(function(r){{
      if(!r.ok)throw new Error('session error '+r.status);
      return r.json();
    }}).then(function(d){{
      sessionId=d.session_id;
      sessionStorage.setItem('pcw_sid_'+SLUG,sessionId);
      cb();
    }}).catch(function(e){{
      console.error('Chat session error:',e);
      setBusy(false);
    }});
  }}

  function connectSSE(){{
    if(evtSource)evtSource.close();
    var streamUrl='/p/'+SLUG+'/api/chat/stream/'+sessionId;
    var queryParts=['visitor_id='+encodeURIComponent(vid)];
    if(lastEventId)queryParts.push('last_event_id=' + encodeURIComponent(lastEventId));
    if(queryParts.length>0)streamUrl += '?' + queryParts.join('&');
    evtSource=new EventSource(streamUrl);
    currentBotEl=null;currentBotText='';

    evtSource.addEventListener('text',function(e){{
      if(e.lastEventId)lastEventId=e.lastEventId;
      removeTyping();
      var data=JSON.parse(e.data);
      if(!currentBotEl){{
        currentBotEl=document.createElement('div');
        currentBotEl.className='pcw-msg bot';
        msgBox.appendChild(currentBotEl);
      }}
      currentBotText+=data.content;
      currentBotEl.textContent=currentBotText;
      msgBox.scrollTop=msgBox.scrollHeight;
    }});

    evtSource.addEventListener('thinking',function(e){{
      if(e.lastEventId)lastEventId=e.lastEventId;
      removeTyping();addTyping();
    }});

    evtSource.addEventListener('done',function(e){{
      if(e.lastEventId)lastEventId=e.lastEventId;
      removeTyping();
      evtSource.close();evtSource=null;
      if(currentBotText){{
        msgs.push({{role:'bot',content:currentBotText}});
        saveMsgs();
      }}
      currentBotEl=null;currentBotText='';
      setBusy(false);
    }});

    evtSource.onerror=function(){{
      evtSource.close();evtSource=null;
      removeTyping();
      if(!currentBotText){{
        var errEl=document.createElement('div');errEl.className='pcw-msg bot';
        errEl.textContent='Connection lost. Please try again.';
        msgBox.appendChild(errEl);
      }} else {{
        msgs.push({{role:'bot',content:currentBotText}});saveMsgs();
      }}
      currentBotEl=null;currentBotText='';
      setBusy(false);
    }};
  }}

  window.pcwToggle=function(){{
    var vis=panel.style.display==='flex';
    panel.style.display=vis?'none':'flex';
  }};

  window.pcwSend=function(){{
    if(busy)return;
    var inp=document.getElementById('pcw-input');
    var text=inp.value.trim();if(!text)return;
    msgs.push({{role:'user',content:text}});saveMsgs();
    inp.value='';render();
    setBusy(true);addTyping();

    function postMessage(retried){{
      fetch('/p/'+SLUG+'/api/chat/message',{{
        method:'POST',headers:{{'Content-Type':'application/json'}},
        body:JSON.stringify({{session_id:sessionId,visitor_id:vid,content:text}})
      }}).then(function(r){{
        if(!r.ok){{
          if(!retried && (r.status===403 || r.status===404)){{
            sessionId='';
            sessionStorage.removeItem('pcw_sid_'+SLUG);
            ensureSession(function(){{ postMessage(true); }});
            return null;
          }}
          throw new Error('send error '+r.status);
        }}
        return r.json();
      }}).then(function(d){{
        if(d && d.streaming)connectSSE();
      }}).catch(function(e){{
        console.error('Send error:',e);
        removeTyping();
        var errEl=document.createElement('div');errEl.className='pcw-msg bot';
        errEl.textContent='Failed to send message. Please try again.';
        msgBox.appendChild(errEl);
        setBusy(false);
      }});
    }}

    ensureSession(function(){{ postMessage(false); }});

    // Also log interaction (fire and forget)
    fetch('/p/'+SLUG+'/api/interact',{{
      method:'POST',headers:{{'Content-Type':'application/json'}},
      body:JSON.stringify({{visitorId:vid,interactionType:'chat_message',data:{{message:text}}}})
    }}).catch(function(){{}});
  }};
}})();
</script>
</div>"##,
        slug = slug,
        welcome_js = serde_json::to_string(welcome).unwrap_or_else(|_| "\"Hi!\"".to_string()),
    )
}

/// Extract a stable visitor identifier from request headers.
/// Priority: X-Visitor-Id header > hashed IP from X-Forwarded-For > "anonymous"
fn extract_visitor_id(headers: &HeaderMap) -> String {
    if let Some(vid) = headers.get("x-visitor-id").and_then(|v| v.to_str().ok()) {
        if !vid.is_empty() && vid.len() <= 64 {
            return vid.to_string();
        }
    }
    if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let ip = forwarded.split(',').next().unwrap_or("").trim();
        if !ip.is_empty() {
            return format!(
                "ip_{:x}",
                ip.bytes()
                    .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64))
            );
        }
    }
    "anonymous".to_string()
}

/// Validate and normalize visitor id from public APIs.
/// Allows ASCII letters/digits/underscore/hyphen and max length 64.
fn normalize_visitor_id(input: &str) -> Option<String> {
    let id = input.trim();
    if id.is_empty() || id.len() > 64 {
        return None;
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    Some(id.to_string())
}

fn normalize_agent_id(input: Option<&str>) -> Option<String> {
    input
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn resolve_service_agent_id(portal: &Portal) -> Option<String> {
    normalize_agent_id(portal.service_agent_id.as_deref())
        .or_else(|| normalize_agent_id(portal.agent_id.as_deref()))
        .or_else(|| normalize_agent_id(portal.coding_agent_id.as_deref()))
}

// ---------------------------------------------------------------------------
// Filesystem serving helpers
// ---------------------------------------------------------------------------

/// Sanitize a URL path to prevent directory traversal attacks.
/// Only keeps `Component::Normal` segments.
fn sanitize_path(raw: &str) -> Option<std::path::PathBuf> {
    let path = std::path::Path::new(raw);
    let mut clean = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(seg) => clean.push(seg),
            _ => return None, // reject .., /, prefix etc.
        }
    }
    if clean.as_os_str().is_empty() {
        return None;
    }
    Some(clean)
}

fn normalize_optional_string_list(input: Option<Vec<String>>) -> Option<Vec<String>> {
    let input = input?;
    let mut out = Vec::<String>::new();
    for item in input {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out.iter().any(|v| v == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    Some(out)
}

/// Serve a file from the portal's project folder.
/// Returns (body_bytes, content_type). Injects chat widget into HTML files.
fn serve_from_filesystem(
    project_path: &str,
    relative_path: &str,
    portal: &Portal,
) -> Result<(Vec<u8>, String), (StatusCode, String)> {
    let base = std::path::Path::new(project_path);

    // Determine the file path
    let file_path = if relative_path.is_empty() || relative_path == "index" {
        base.join("index.html")
    } else if let Some(sanitized) = sanitize_path(relative_path) {
        // Block access to _private/ directory (case-insensitive for Windows)
        if sanitized.to_string_lossy().to_ascii_lowercase().starts_with("_private") {
            return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
        }
        let candidate = base.join(&sanitized);
        if candidate.is_dir() {
            candidate.join("index.html")
        } else if candidate.exists() {
            candidate
        } else {
            // SPA fallback: try root index.html for paths without extensions
            let has_ext = sanitized.extension().map_or(false, |e| !e.is_empty());
            if !has_ext {
                base.join("index.html")
            } else {
                return Err((StatusCode::NOT_FOUND, "File not found".to_string()));
            }
        }
    } else {
        return Err((StatusCode::BAD_REQUEST, "Invalid path".to_string()));
    };

    if !file_path.exists() {
        return Err((StatusCode::NOT_FOUND, "File not found".to_string()));
    }

    // Security: ensure resolved path is within project_path
    let canonical_base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let canonical_file = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.clone());
    if !canonical_file.starts_with(&canonical_base) {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    let body = std::fs::read(&file_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Read error: {}", e),
        )
    })?;

    let mime = mime_guess::from_path(&file_path)
        .first_or_octet_stream()
        .to_string();

    // Inject chat widget into HTML files
    if mime.starts_with("text/html") {
        let html = String::from_utf8_lossy(&body);
        if portal.agent_enabled && resolve_service_agent_id(portal).is_some() {
            let widget = render_chat_widget(&portal.slug, portal.agent_welcome_message.as_deref());
            // Insert before </body> if present, otherwise append
            let injected = if let Some(pos) = html.rfind("</body>") {
                format!("{}{}{}", &html[..pos], widget, &html[pos..])
            } else {
                format!("{}{}", html, widget)
            };
            return Ok((injected.into_bytes(), mime));
        }
    }

    Ok((body, mime))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn serve_portal_index(
    State(state): State<PortalPublicState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .get_by_slug(&slug)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".to_string()))?;

    let portal_id = portal.id.unwrap_or_default();

    // Filesystem-first: if portal has project_path, serve from filesystem (even when draft)
    if let Some(ref project_path) = portal.project_path {
        let (body, content_type) = serve_from_filesystem(project_path, "", &portal)?;

        // Log page view (fire and forget)
        let db_clone = state.db.clone();
        let team_id = portal.team_id;
        let vid = extract_visitor_id(&headers);
        tokio::spawn(async move {
            let svc = PortalService::new((*db_clone).clone());
            let _ = svc
                .log_interaction(PortalInteraction {
                    id: None,
                    portal_id,
                    team_id,
                    visitor_id: vid,
                    interaction_type: InteractionType::PageView,
                    page_path: Some("index".to_string()),
                    data: serde_json::Value::Null,
                    created_at: Utc::now(),
                })
                .await;
        });

        return Ok(([(header::CONTENT_TYPE, content_type)], body));
    }

    Err((
        StatusCode::NOT_FOUND,
        "Legacy MongoDB portals are no longer supported. Please recreate this portal.".to_string(),
    ))
}

async fn serve_portal_page(
    State(state): State<PortalPublicState>,
    headers: HeaderMap,
    Path((slug, path)): Path<(String, String)>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Intercept API paths — they should not be served as pages
    if path.starts_with("api/") {
        return Err((StatusCode::NOT_FOUND, "Not found".to_string()));
    }

    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .get_by_slug(&slug)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".to_string()))?;

    let portal_id = portal.id.unwrap_or_default();

    // Filesystem-first: if portal has project_path, serve from filesystem (even when draft)
    if let Some(ref project_path) = portal.project_path {
        let (body, content_type) = serve_from_filesystem(project_path, &path, &portal)?;

        // Log page view (fire and forget)
        let db_clone = state.db.clone();
        let team_id = portal.team_id;
        let path_clone = path.clone();
        let vid = extract_visitor_id(&headers);
        tokio::spawn(async move {
            let svc = PortalService::new((*db_clone).clone());
            let _ = svc
                .log_interaction(PortalInteraction {
                    id: None,
                    portal_id,
                    team_id,
                    visitor_id: vid,
                    interaction_type: InteractionType::PageView,
                    page_path: Some(path_clone),
                    data: serde_json::Value::Null,
                    created_at: Utc::now(),
                })
                .await;
        });

        return Ok(([(header::CONTENT_TYPE, content_type)], body));
    }

    Err((
        StatusCode::NOT_FOUND,
        "Legacy MongoDB portals are no longer supported. Please recreate this portal.".to_string(),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InteractRequest {
    visitor_id: String,
    interaction_type: InteractionType,
    page_path: Option<String>,
    #[serde(default)]
    data: serde_json::Value,
}

async fn log_interaction(
    State(state): State<PortalPublicState>,
    Path(slug): Path<String>,
    Json(req): Json<InteractRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .get_by_slug(&slug)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".to_string()))?;

    // Allow draft portals with project_path (filesystem-based)
    if portal.status != PortalStatus::Published && portal.project_path.is_none() {
        return Err((StatusCode::NOT_FOUND, "Portal not published".to_string()));
    }

    svc.log_interaction(PortalInteraction {
        id: None,
        portal_id: portal.id.unwrap_or_default(),
        team_id: portal.team_id,
        visitor_id: req.visitor_id,
        interaction_type: req.interaction_type,
        page_path: req.page_path,
        data: req.data,
        created_at: Utc::now(),
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

async fn portal_config(
    State(state): State<PortalPublicState>,
    Path(slug): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .get_by_slug(&slug)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".to_string()))?;

    // Allow draft portals with project_path (filesystem-based)
    if portal.status != PortalStatus::Published && portal.project_path.is_none() {
        return Err((StatusCode::NOT_FOUND, "Portal not published".to_string()));
    }

    Ok(Json(serde_json::json!({
        "apiVersion": "v1",
        "name": portal.name,
        "agentEnabled": portal.agent_enabled && resolve_service_agent_id(&portal).is_some(),
        "agentWelcomeMessage": portal.agent_welcome_message,
        "chatApi": {
            "sessionPath": format!("/p/{}/api/chat/session", slug),
            "messagePath": format!("/p/{}/api/chat/message", slug),
            "streamPathTemplate": format!("/p/{}/api/chat/stream/{{session_id}}", slug),
        }
    })))
}

// ---------------------------------------------------------------------------
// Phase 3: Public chat handlers (unauthenticated, visitor-based)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreateVisitorSessionRequest {
    visitor_id: String,
}

/// POST /p/{slug}/api/chat/session — Create or retrieve a visitor chat session
async fn create_visitor_session(
    State(state): State<PortalPublicState>,
    Path(slug): Path<String>,
    Json(req): Json<CreateVisitorSessionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let visitor_id = normalize_visitor_id(&req.visitor_id)
        .ok_or((StatusCode::BAD_REQUEST, "Invalid visitor_id".into()))?;

    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .get_by_slug(&slug)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".into()))?;

    // Allow draft portals with project_path (filesystem-based)
    if portal.status != PortalStatus::Published && portal.project_path.is_none() {
        return Err((StatusCode::NOT_FOUND, "Portal not published".into()));
    }
    if !portal.agent_enabled {
        return Err((StatusCode::BAD_REQUEST, "Agent not enabled".into()));
    }
    let agent_id = resolve_service_agent_id(&portal)
        .ok_or((StatusCode::BAD_REQUEST, "No agent configured".into()))?;
    let portal_id = portal.id.map(|id| id.to_hex()).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Portal id missing".into(),
    ))?;
    let synthetic_user_id = format!("portal_visitor_{}", visitor_id);
    let agent_svc = AgentService::new(state.db.clone());
    let normalized_project_path = portal
        .project_path
        .as_ref()
        .map(|p| normalize_workspace_path(p));
    if let (Some(old), Some(new)) = (
        portal.project_path.as_ref(),
        normalized_project_path.as_ref(),
    ) {
        if old != new {
            if let Err(e) = svc
                .set_project_path(&portal.team_id.to_hex(), &portal_id, new)
                .await
            {
                tracing::warn!(
                    "Failed to normalize portal project_path for {}: {}",
                    portal_id,
                    e
                );
            }
        }
    }

    // Create a new session with extra_instructions
    let mut extra_instructions_parts: Vec<String> = Vec::new();
    if let Some(ref prompt) = portal.agent_system_prompt {
        if !prompt.trim().is_empty() {
            extra_instructions_parts.push(prompt.clone());
        }
    }
    if let Some(ref project_path) = normalized_project_path {
        extra_instructions_parts.push(format!(
            "你的项目工作目录是: {}\n请在此目录下进行所有文件操作。",
            project_path
        ));
        let project_ctx = super::runtime::scan_project_context(project_path, 6000);
        if !project_ctx.is_empty() {
            extra_instructions_parts.push(project_ctx);
        }
    }
    let extra_instructions = if extra_instructions_parts.is_empty() {
        None
    } else {
        Some(extra_instructions_parts.join("\n\n"))
    };

    let allowed_extensions = normalize_optional_string_list(portal.allowed_extensions.clone());
    let allowed_skill_ids = normalize_optional_string_list(portal.allowed_skill_ids.clone());

    // Reuse only a session already bound to this exact portal.
    if let Ok(Some(session)) = agent_svc
        .find_active_portal_session(&synthetic_user_id, &agent_id, &portal_id)
        .await
    {
        if let Err(e) = agent_svc
            .sync_portal_session_policy(
                &session.session_id,
                portal.bound_document_ids.clone(),
                extra_instructions.clone(),
                allowed_extensions.clone(),
                allowed_skill_ids.clone(),
                None,
                false,
            )
            .await
        {
            tracing::warn!(
                "Failed to sync existing portal visitor session {} policy: {}",
                session.session_id,
                e
            );
        }
        if let Err(e) = agent_svc
            .set_session_portal_context(
                &session.session_id,
                &portal_id,
                &portal.slug,
                Some(&visitor_id),
            )
            .await
        {
            tracing::warn!(
                "Failed to refresh portal context for existing session {}: {}",
                session.session_id,
                e
            );
        }
        if let Some(ref project_path) = normalized_project_path {
            if session.workspace_path.as_deref() != Some(project_path.as_str()) {
                if let Err(e) = agent_svc
                    .set_session_workspace(&session.session_id, project_path)
                    .await
                {
                    tracing::warn!(
                        "Failed to refresh workspace for existing session {}: {}",
                        session.session_id,
                        e
                    );
                }
            }
        }
        return Ok(Json(serde_json::json!({
            "session_id": session.session_id,
            "existing": true,
        })));
    }

    let session = agent_svc
        .create_chat_session(
            &portal.team_id.to_hex(),
            &agent_id,
            &synthetic_user_id,
            portal.bound_document_ids.clone(),
            extra_instructions,
            allowed_extensions,
            allowed_skill_ids,
            None,
            None,
            None,
            None,
            false,
            true,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    agent_svc
        .set_session_portal_context(
            &session.session_id,
            &portal_id,
            &portal.slug,
            Some(&visitor_id),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if let Some(ref project_path) = normalized_project_path {
        agent_svc
            .set_session_workspace(&session.session_id, project_path)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(Json(serde_json::json!({
        "session_id": session.session_id,
        "existing": false,
    })))
}

#[derive(Deserialize)]
struct SendVisitorMessageRequest {
    session_id: String,
    visitor_id: String,
    content: String,
}

#[derive(Deserialize)]
struct StreamQuery {
    last_event_id: Option<u64>,
    visitor_id: Option<String>,
}

/// POST /p/{slug}/api/chat/message — Send a visitor message (triggers Agent execution)
async fn send_visitor_message(
    State(state): State<PortalPublicState>,
    Path(slug): Path<String>,
    Json(req): Json<SendVisitorMessageRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let visitor_id = normalize_visitor_id(&req.visitor_id)
        .ok_or((StatusCode::BAD_REQUEST, "Invalid visitor_id".into()))?;
    let content = req.content.trim().to_string();
    if content.is_empty() || content.len() > 100_000 {
        return Err((StatusCode::BAD_REQUEST, "Invalid message".into()));
    }

    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .get_by_slug(&slug)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".into()))?;
    let portal_id = portal.id.map(|id| id.to_hex()).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Portal id missing".into(),
    ))?;

    // Allow draft portals with project_path (filesystem-based)
    let is_accessible = portal.status == PortalStatus::Published || portal.project_path.is_some();
    if !is_accessible || !portal.agent_enabled {
        return Err((StatusCode::BAD_REQUEST, "Chat not available".into()));
    }

    let synthetic_user_id = format!("portal_visitor_{}", visitor_id);
    let agent_svc = AgentService::new(state.db.clone());

    // Verify session exists and belongs to this visitor
    let session = agent_svc
        .get_session(&req.session_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into()))?
        .ok_or((StatusCode::NOT_FOUND, "Session not found".into()))?;

    if session.user_id != synthetic_user_id {
        return Err((StatusCode::FORBIDDEN, "Session mismatch".into()));
    }
    if session.portal_id.as_deref() != Some(portal_id.as_str()) {
        return Err((StatusCode::FORBIDDEN, "Portal session mismatch".into()));
    }
    if session.portal_slug.as_deref() != Some(slug.as_str()) {
        return Err((StatusCode::FORBIDDEN, "Portal slug mismatch".into()));
    }
    if session.visitor_id.as_deref() != Some(visitor_id.as_str()) {
        return Err((StatusCode::FORBIDDEN, "Visitor mismatch".into()));
    }

    // Register in ChatManager first (authoritative in-memory gate)
    let (cancel_token, _stream_tx) = match state.chat_manager.register(&req.session_id).await {
        Some(pair) => pair,
        None => {
            return Err((StatusCode::CONFLICT, "Already active".into()));
        }
    };

    // Then set MongoDB is_processing flag (secondary persistence)
    let claimed = agent_svc
        .try_start_processing(&req.session_id, &synthetic_user_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error".to_string()));
    match claimed {
        Ok(true) => {}
        _ => {
            // Rollback ChatManager registration
            state.chat_manager.unregister(&req.session_id).await;
            return Err(claimed.err().unwrap_or((StatusCode::CONFLICT, "Already processing".into())));
        }
    }

    // Spawn background execution
    let executor = ChatExecutor::new(
        state.db.clone(),
        state.chat_manager.clone(),
        state.workspace_root.clone(),
    );
    let sid = req.session_id.clone();
    let agent_id = session.agent_id.clone();

    tokio::spawn(async move {
        if let Err(e) = executor
            .execute_chat(&sid, &agent_id, &content, cancel_token)
            .await
        {
            tracing::error!("Portal chat execution failed for session {}: {}", sid, e);
        }
    });

    Ok(Json(serde_json::json!({
        "session_id": req.session_id,
        "streaming": true,
    })))
}

/// GET /p/{slug}/api/chat/stream/{session_id} — SSE stream for visitor chat
async fn stream_visitor_chat(
    State(state): State<PortalPublicState>,
    headers: HeaderMap,
    Path((slug, session_id)): Path<(String, String)>,
    Query(q): Query<StreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    let visitor_id = q
        .visitor_id
        .as_deref()
        .and_then(normalize_visitor_id)
        .or_else(|| {
            headers
                .get("x-visitor-id")
                .and_then(|v| v.to_str().ok())
                .and_then(normalize_visitor_id)
        })
        .ok_or((StatusCode::BAD_REQUEST, "visitor_id is required".into()))?;

    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .get_by_slug(&slug)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".into()))?;
    let portal_id = portal.id.map(|id| id.to_hex()).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Portal id missing".into(),
    ))?;

    // Verify session exists and belongs to a portal visitor
    let agent_svc = AgentService::new(state.db.clone());
    let session = agent_svc
        .get_session(&session_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into()))?
        .ok_or((StatusCode::NOT_FOUND, "Session not found".into()))?;

    if !session.user_id.starts_with("portal_visitor_") {
        return Err((StatusCode::FORBIDDEN, "Not a portal session".into()));
    }
    if session.user_id != format!("portal_visitor_{}", visitor_id) {
        return Err((StatusCode::FORBIDDEN, "Session mismatch".into()));
    }
    if session.portal_id.as_deref() != Some(portal_id.as_str()) {
        return Err((StatusCode::FORBIDDEN, "Portal session mismatch".into()));
    }
    if session.portal_slug.as_deref() != Some(slug.as_str()) {
        return Err((StatusCode::FORBIDDEN, "Portal slug mismatch".into()));
    }
    if session.visitor_id.as_deref() != Some(visitor_id.as_str()) {
        return Err((StatusCode::FORBIDDEN, "Visitor mismatch".into()));
    }

    let last_event_id = q.last_event_id.or_else(|| {
        headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
    });

    // Subscribe to chat events with buffered history for reconnect/late join.
    let (mut rx, history) = state
        .chat_manager
        .subscribe_with_history(&session_id, last_event_id)
        .await
        .ok_or((StatusCode::NOT_FOUND, "No active stream".into()))?;

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
            .interval(Duration::from_secs(15))
            .text("ping"),
    ))
}

// ---------------------------------------------------------------------------
// Data API — key-value storage in _private/ directory
// ---------------------------------------------------------------------------

fn validate_data_key(key: &str) -> Result<(), (StatusCode, String)> {
    if key.is_empty() || key.len() > 64 {
        return Err((StatusCode::BAD_REQUEST, "Invalid key length".into()));
    }
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err((StatusCode::BAD_REQUEST, "Key must be [a-zA-Z0-9_-]".into()));
    }
    Ok(())
}

fn get_private_dir(portal: &Portal) -> Result<std::path::PathBuf, (StatusCode, String)> {
    let project_path = portal.project_path.as_deref()
        .ok_or((StatusCode::NOT_FOUND, "No project path".into()))?;
    Ok(std::path::Path::new(project_path).join("_private"))
}

/// GET /p/{slug}/api/data — list data keys
async fn list_data_keys(
    State(state): State<PortalPublicState>,
    Path(slug): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let svc = PortalService::new((*state.db).clone());
    let portal = svc.get_by_slug(&slug).await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".into()))?;
    if portal.status != PortalStatus::Published && portal.project_path.is_none() {
        return Err((StatusCode::NOT_FOUND, "Portal not published".into()));
    }
    let dir = get_private_dir(&portal)?;
    let mut keys = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(key) = name.strip_suffix(".json") {
                    keys.push(key.to_string());
                }
            }
        }
    }
    Ok(Json(serde_json::json!({ "keys": keys })))
}

/// GET /p/{slug}/api/data/{key}
async fn get_data(
    State(state): State<PortalPublicState>,
    Path((slug, key)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    validate_data_key(&key)?;
    let svc = PortalService::new((*state.db).clone());
    let portal = svc.get_by_slug(&slug).await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".into()))?;
    if portal.status != PortalStatus::Published && portal.project_path.is_none() {
        return Err((StatusCode::NOT_FOUND, "Portal not published".into()));
    }
    let file = get_private_dir(&portal)?.join(format!("{}.json", key));
    let data = std::fs::read_to_string(&file)
        .map_err(|_| (StatusCode::NOT_FOUND, "Key not found".into()))?;
    let value: serde_json::Value = serde_json::from_str(&data)
        .unwrap_or(serde_json::Value::String(data));
    Ok(Json(value))
}

/// PUT /p/{slug}/api/data/{key}
async fn set_data(
    State(state): State<PortalPublicState>,
    Path((slug, key)): Path<(String, String)>,
    Json(value): Json<serde_json::Value>,
) -> Result<StatusCode, (StatusCode, String)> {
    validate_data_key(&key)?;
    let svc = PortalService::new((*state.db).clone());
    let portal = svc.get_by_slug(&slug).await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".into()))?;
    if portal.status != PortalStatus::Published && portal.project_path.is_none() {
        return Err((StatusCode::NOT_FOUND, "Portal not published".into()));
    }
    let dir = get_private_dir(&portal)?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let file = dir.join(format!("{}.json", key));
    let json = serde_json::to_string_pretty(&value)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if json.len() > 10 * 1024 * 1024 {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "Value exceeds 10MB limit".into()));
    }
    std::fs::write(&file, json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Document bridge API — read-only access to bound documents
// ---------------------------------------------------------------------------

/// GET /p/{slug}/api/docs — list bound documents metadata
async fn list_bound_documents(
    State(state): State<PortalPublicState>,
    Path(slug): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let svc = PortalService::new((*state.db).clone());
    let portal = svc.get_by_slug(&slug).await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".into()))?;
    if portal.status != PortalStatus::Published && portal.project_path.is_none() {
        return Err((StatusCode::NOT_FOUND, "Portal not published".into()));
    }
    let doc_svc = DocumentService::new((*state.db).clone());
    let team_id = portal.team_id.to_hex();
    let mut docs = Vec::new();
    for doc_id in &portal.bound_document_ids {
        if let Ok(meta) = doc_svc.get_metadata(&team_id, doc_id).await {
            docs.push(serde_json::json!({
                "id": meta.id, "name": meta.name,
                "mime_type": meta.mime_type, "file_size": meta.file_size,
            }));
        }
    }
    Ok(Json(serde_json::json!({ "documents": docs })))
}

/// GET /p/{slug}/api/docs/{doc_id} — get bound document content
async fn get_bound_document(
    State(state): State<PortalPublicState>,
    Path((slug, doc_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let svc = PortalService::new((*state.db).clone());
    let portal = svc.get_by_slug(&slug).await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".into()))?;
    if portal.status != PortalStatus::Published && portal.project_path.is_none() {
        return Err((StatusCode::NOT_FOUND, "Portal not published".into()));
    }
    if !portal.bound_document_ids.iter().any(|id| id == &doc_id) {
        return Err((StatusCode::FORBIDDEN, "Document not bound".into()));
    }
    let doc_svc = DocumentService::new((*state.db).clone());
    let team_id = portal.team_id.to_hex();
    let (text, mime_type, total_size) = doc_svc
        .get_text_content_chunked(&team_id, &doc_id, None, None).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({
        "text": text, "mime_type": mime_type, "total_size": total_size,
    })))
}

/// GET /p/{slug}/api/docs/{doc_id}/meta — get bound document metadata
async fn get_bound_document_meta(
    State(state): State<PortalPublicState>,
    Path((slug, doc_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let svc = PortalService::new((*state.db).clone());
    let portal = svc.get_by_slug(&slug).await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".into()))?;
    if portal.status != PortalStatus::Published && portal.project_path.is_none() {
        return Err((StatusCode::NOT_FOUND, "Portal not published".into()));
    }
    if !portal.bound_document_ids.iter().any(|id| id == &doc_id) {
        return Err((StatusCode::FORBIDDEN, "Document not bound".into()));
    }
    let doc_svc = DocumentService::new((*state.db).clone());
    let team_id = portal.team_id.to_hex();
    let meta = doc_svc.get_metadata(&team_id, &doc_id).await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(serde_json::json!({
        "id": meta.id, "name": meta.name,
        "mime_type": meta.mime_type, "file_size": meta.file_size,
        "updated_at": meta.updated_at.to_rfc3339(),
    })))
}

// ---------------------------------------------------------------------------
// Chat enhancements
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CancelRequest {
    session_id: String,
    visitor_id: String,
}

/// POST /p/{slug}/api/chat/cancel — cancel active chat execution
async fn cancel_visitor_chat(
    State(state): State<PortalPublicState>,
    Path(slug): Path<String>,
    Json(req): Json<CancelRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let visitor_id = normalize_visitor_id(&req.visitor_id)
        .ok_or((StatusCode::BAD_REQUEST, "Invalid visitor_id".into()))?;
    let synthetic_user_id = format!("portal_visitor_{}", visitor_id);

    let agent_svc = AgentService::new(state.db.clone());
    let session = agent_svc.get_session(&req.session_id).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into()))?
        .ok_or((StatusCode::NOT_FOUND, "Session not found".into()))?;

    if session.user_id != synthetic_user_id {
        return Err((StatusCode::FORBIDDEN, "Session mismatch".into()));
    }

    state.chat_manager.cancel(&req.session_id).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ListSessionsQuery {
    visitor_id: String,
}

/// GET /p/{slug}/api/chat/sessions — list visitor's chat sessions
async fn list_visitor_sessions(
    State(state): State<PortalPublicState>,
    Path(slug): Path<String>,
    Query(q): Query<ListSessionsQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let visitor_id = normalize_visitor_id(&q.visitor_id)
        .ok_or((StatusCode::BAD_REQUEST, "Invalid visitor_id".into()))?;

    let svc = PortalService::new((*state.db).clone());
    let portal = svc.get_by_slug(&slug).await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".into()))?;

    let synthetic_user_id = format!("portal_visitor_{}", visitor_id);
    let agent_svc = AgentService::new(state.db.clone());
    let sessions = agent_svc
        .list_user_sessions(super::session_mongo::UserSessionListQuery {
            team_id: portal.team_id.to_hex(),
            user_id: Some(synthetic_user_id),
            agent_id: None,
            status: None,
            page: 1,
            limit: 20,
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<_> = sessions
        .into_iter()
        .map(|s| serde_json::json!({
            "session_id": s.session_id,
            "title": s.title,
            "created_at": s.created_at,
            "last_message_at": s.last_message_at,
            "message_count": s.message_count,
        }))
        .collect();

    Ok(Json(serde_json::json!({ "sessions": items })))
}
