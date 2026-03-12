//! Portal public routes — unauthenticated endpoints for serving published portals
//!
//! Mounted BEFORE auth middleware so external visitors can access published pages.
//! Phase 3: Real Agent chat via SSE + Markdown page rendering.

use agime_team::db::MongoDb;
use agime_team::models::mongo::{
    ExternalUser, InteractionType, Portal, PortalEffectivePublicConfig, PortalInteraction,
};
use agime_team::services::mongo::{
    DocumentService, DocumentVersionService, ExternalUserService, FolderService, PortalService,
    TeamService,
};
use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, Sse},
        IntoResponse, Redirect,
    },
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use futures::stream::Stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::path::Component;
use std::sync::Arc;
use std::time::Duration;

use super::chat_executor::ChatExecutor;
use super::chat_manager::ChatManager;
use super::normalize_workspace_path;
use super::service_mongo::AgentService;

const EXTERNAL_USER_SESSION_COOKIE_NAME: &str = "agime_portal_user_session";

/// State for portal public routes
#[derive(Clone)]
pub struct PortalPublicState {
    pub db: Arc<MongoDb>,
    pub chat_manager: Arc<ChatManager>,
    pub workspace_root: String,
    pub secure_cookies: bool,
}

pub fn portal_public_routes(
    db: Arc<MongoDb>,
    chat_manager: Arc<ChatManager>,
    workspace_root: String,
    secure_cookies: bool,
) -> Router {
    let state = PortalPublicState {
        db,
        chat_manager,
        workspace_root,
        secure_cookies,
    };
    Router::new()
        .route("/p/{slug}", get(|Path(slug): Path<String>| async move {
            Redirect::permanent(&format!("/p/{slug}/"))
        }))
        .route("/p/{slug}/", get(serve_portal_index))
        .route("/p/{slug}/{*path}", get(serve_portal_page))
        .route("/p/{slug}/api/interact", post(log_interaction))
        .route("/p/{slug}/api/config", get(portal_config))
        .route("/p/{slug}/api/auth/register", post(register_public_user))
        .route("/p/{slug}/api/auth/login", post(login_public_user))
        .route("/p/{slug}/api/auth/logout", post(logout_public_user))
        .route("/p/{slug}/api/auth/session", get(get_public_user_session))
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
        .route(
            "/p/{slug}/api/docs/{doc_id}/meta",
            get(get_bound_document_meta),
        )
        .route(
            "/p/{slug}/api/user-docs",
            get(list_visitor_user_documents).post(upload_visitor_user_document),
        )
        .route(
            "/p/{slug}/api/user-docs/{doc_id}",
            get(get_visitor_user_document),
        )
        .route(
            "/p/{slug}/api/user-docs/{doc_id}/meta",
            get(get_visitor_user_document_meta),
        )
        // Chat enhancements
        .route("/p/{slug}/api/chat/cancel", post(cancel_visitor_chat))
        .route("/p/{slug}/api/chat/sessions", get(list_visitor_sessions))
        .with_state(state)
}

/// Sanitize slug for safe interpolation into JS string literals.
fn sanitize_slug_for_js(slug: &str) -> String {
    slug.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

fn extract_client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next().map(str::trim))
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn extract_user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn build_external_user_session_cookie(session_id: &str, secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000{}",
        EXTERNAL_USER_SESSION_COOKIE_NAME, session_id, secure_flag
    )
}

fn build_external_user_clear_cookie(secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}",
        EXTERNAL_USER_SESSION_COOKIE_NAME, secure_flag
    )
}

fn cookie_from_headers(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookie_str| {
            cookie_str.split(';').find_map(|cookie| {
                let cookie = cookie.trim();
                cookie
                    .strip_prefix(&format!("{name}="))
                    .map(ToString::to_string)
            })
        })
}

fn external_user_session_id_from_headers(headers: &HeaderMap) -> Option<String> {
    cookie_from_headers(headers, EXTERNAL_USER_SESSION_COOKIE_NAME)
}

/// Built-in Portal SDK served at /p/{slug}/portal-sdk.js
fn render_portal_sdk(slug: &str) -> String {
    let slug = sanitize_slug_for_js(slug);
    format!(
        r##"(function(global){{
"use strict";
var BASE="/p/{slug}/api";
var VID_KEY="pcw_vid";
function createVid(){{if(global.crypto&&typeof global.crypto.getRandomValues==="function"){{var bytes=new Uint8Array(16);global.crypto.getRandomValues(bytes);return "v_"+Array.prototype.map.call(bytes,function(b){{return b.toString(16).padStart(2,"0")}}).join("")}}return "v_"+Math.random().toString(36).slice(2,14)}}
function vid(){{var v=localStorage.getItem(VID_KEY);if(!v){{v=createVid();localStorage.setItem(VID_KEY,v)}}return v}}
function post(path,body){{return fetch(BASE+path,{{method:"POST",credentials:"same-origin",headers:{{"Content-Type":"application/json"}},body:JSON.stringify(body)}}).then(function(r){{if(!r.ok)throw new Error(r.status+" "+r.statusText);return r.json()}})}}
function postWithVisitor(path,body){{return fetch(BASE+path,{{method:"POST",credentials:"same-origin",headers:{{"Content-Type":"application/json","x-visitor-id":vid()}},body:JSON.stringify(body)}}).then(function(r){{if(!r.ok)throw new Error(r.status+" "+r.statusText);return r.json()}})}}
function get(path){{return fetch(BASE+path,{{credentials:"same-origin"}}).then(function(r){{if(!r.ok)throw new Error(r.status+" "+r.statusText);return r.json()}})}}
function getWithVisitor(path){{return fetch(BASE+path,{{credentials:"same-origin",headers:{{"x-visitor-id":vid()}}}}).then(function(r){{if(!r.ok)throw new Error(r.status+" "+r.statusText);return r.json()}})}}
function postForm(path,form){{return fetch(BASE+path,{{method:"POST",credentials:"same-origin",headers:{{"x-visitor-id":vid()}},body:form}}).then(function(r){{if(!r.ok)throw new Error(r.status+" "+r.statusText);return r.json()}})}}

function PortalSDK(opts){{
  this.slug=opts&&opts.slug||"{slug}";
  var sidKey="pcw_sid_"+this.slug+"_"+vid();
  var historyKey="pcw_msgs_"+this.slug+"_"+vid();
  function safeParseArray(raw){{try{{var v=JSON.parse(raw||"[]");return Array.isArray(v)?v:[]}}catch(_e){{return []}}}}
  function loadHistory(){{return safeParseArray(localStorage.getItem(historyKey))}}
  function saveHistory(items){{var arr=Array.isArray(items)?items:[];if(arr.length>200)arr=arr.slice(arr.length-200);localStorage.setItem(historyKey,JSON.stringify(arr));return arr}}
  function appendHistory(item){{var arr=loadHistory();arr.push(item);return saveHistory(arr)}}
  function saveSessionId(sid){{if(sid)localStorage.setItem(sidKey,sid)}}
  function clearSessionId(){{localStorage.removeItem(sidKey)}}
  function currentSessionId(){{return localStorage.getItem(sidKey)||""}}
  function mapStatus(raw){{
    var s=String(raw||"").toLowerCase();
    if(!s || s==="running") return "processing";
    if(s.indexOf("llm")>=0) return "calling_model";
    if(s.indexOf("tool")>=0) return "running_tool";
    if(s.indexOf("compaction")>=0) return "compacting_context";
    return s;
  }}
  this.chat={{
    getVisitorId:function(){{return vid()}},
    mapStatus:mapStatus,
    getLocalSessionId:function(){{return currentSessionId()}},
    clearLocalSession:function(){{clearSessionId()}},
    getLocalHistory:function(){{return loadHistory()}},
    clearLocalHistory:function(){{saveHistory([])}},
    appendLocalHistory:function(item){{return appendHistory(item||{{}})}},
    createSession:function(){{return post("/chat/session",{{visitor_id:vid()}}).then(function(d){{if(d&&d.session_id)saveSessionId(d.session_id);return d;}})}},
    createOrResumeSession:function(){{var sid=currentSessionId();if(sid)return Promise.resolve({{session_id:sid,existing:true}});return post("/chat/session",{{visitor_id:vid()}}).then(function(d){{if(d&&d.session_id)saveSessionId(d.session_id);return d;}})}},
    sendMessage:function(sid,text){{
      var id=sid||currentSessionId();
      var content=(text==null?"":String(text));
      if(!id) return Promise.reject(new Error("No active session"));
      appendHistory({{role:"user",content:content,ts:Date.now(),session_id:id}});
      return post("/chat/message",{{session_id:id,visitor_id:vid(),content:content}});
    }},
    subscribe:function(sid,lastEventId){{
      var id=sid||currentSessionId();
      if(!id) throw new Error("No active session");
      var q=["visitor_id="+encodeURIComponent(vid())];
      if(lastEventId) q.push("last_event_id="+encodeURIComponent(lastEventId));
      return new EventSource(BASE+"/chat/stream/"+id+"?"+q.join("&"));
    }},
    cancel:function(sid){{
      var id=sid||currentSessionId();
      if(!id) return Promise.resolve();
      return post("/chat/cancel",{{session_id:id,visitor_id:vid()}});
    }},
    listSessions:function(){{return get("/chat/sessions?visitor_id="+encodeURIComponent(vid()))}},
    sendAndStream:function(text,handlers){{
      var h=handlers||{{}};
      var content=(text==null?"":String(text)).trim();
      if(!content) return Promise.reject(new Error("Message is empty"));
      var sessionId="";
      var assistantText="";
      var lastEventId="";
      var streamRef=null;
      return this.createOrResumeSession()
        .then(function(s){{sessionId=s.session_id;return post("/chat/message",{{session_id:sessionId,visitor_id:vid(),content:content}})}})
        .then(function(resp){{
          appendHistory({{role:"user",content:content,ts:Date.now(),session_id:sessionId}});
          if(!resp||!resp.streaming) return {{session_id:sessionId,close:function(){{}}}};
          streamRef=new EventSource(BASE+"/chat/stream/"+sessionId+"?visitor_id="+encodeURIComponent(vid()));
          function parseData(evt){{try{{return JSON.parse(evt.data||"{{}}")}}catch(_e){{return {{}}}}}}
          function emit(kind,data,evt){{if(h.onEvent)h.onEvent(kind,data,evt)}}
          streamRef.onopen=function(){{emit("connection",{{status:"open"}},null)}};
          streamRef.addEventListener("text",function(evt){{if(evt.lastEventId)lastEventId=evt.lastEventId;var d=parseData(evt);assistantText+=String(d.content||"");if(h.onTextDelta)h.onTextDelta(String(d.content||""),d,evt);emit("text",d,evt)}});
          streamRef.addEventListener("thinking",function(evt){{if(evt.lastEventId)lastEventId=evt.lastEventId;emit("thinking",parseData(evt),evt)}});
          streamRef.addEventListener("status",function(evt){{if(evt.lastEventId)lastEventId=evt.lastEventId;var d=parseData(evt);d.mapped_status=mapStatus(d.status);emit("status",d,evt)}});
          streamRef.addEventListener("toolcall",function(evt){{if(evt.lastEventId)lastEventId=evt.lastEventId;emit("toolcall",parseData(evt),evt)}});
          streamRef.addEventListener("toolresult",function(evt){{if(evt.lastEventId)lastEventId=evt.lastEventId;emit("toolresult",parseData(evt),evt)}});
          streamRef.addEventListener("turn",function(evt){{if(evt.lastEventId)lastEventId=evt.lastEventId;emit("turn",parseData(evt),evt)}});
          streamRef.addEventListener("compaction",function(evt){{if(evt.lastEventId)lastEventId=evt.lastEventId;emit("compaction",parseData(evt),evt)}});
          streamRef.addEventListener("workspace_changed",function(evt){{if(evt.lastEventId)lastEventId=evt.lastEventId;emit("workspace_changed",parseData(evt),evt)}});
          streamRef.addEventListener("done",function(evt){{
            if(evt.lastEventId)lastEventId=evt.lastEventId;
            var d=parseData(evt);
            if(assistantText) appendHistory({{role:"bot",content:assistantText,ts:Date.now(),session_id:sessionId}});
            emit("done",d,evt);
            if(h.onDone) h.onDone(d,{{session_id:sessionId,last_event_id:lastEventId}});
            streamRef.close();
          }});
          streamRef.onerror=function(err){{
            if(assistantText) appendHistory({{role:"bot",content:assistantText,ts:Date.now(),session_id:sessionId}});
            if(h.onError) h.onError(err,{{session_id:sessionId,last_event_id:lastEventId}});
            if(streamRef) streamRef.close();
          }};
          return {{session_id:sessionId,close:function(){{if(streamRef)streamRef.close();}}}};
        }}).catch(function(err){{if(h.onError)h.onError(err,{{session_id:sessionId,last_event_id:lastEventId}});throw err;}});
    }}
  }};
  this.docs={{
    list:function(){{return get("/docs")}},
    get:function(id){{return get("/docs/"+encodeURIComponent(id))}},
    getMeta:function(id){{return get("/docs/"+encodeURIComponent(id)+"/meta")}},
    poll:function(id,ms,cb){{var t=setInterval(function(){{get("/docs/"+encodeURIComponent(id)).then(cb).catch(function(){{}})}},ms||3000);return function(){{clearInterval(t)}}}}
  }};
  this.userDocs={{
    list:function(){{return getWithVisitor("/user-docs")}},
    get:function(id){{return getWithVisitor("/user-docs/"+encodeURIComponent(id))}},
    getMeta:function(id){{return getWithVisitor("/user-docs/"+encodeURIComponent(id)+"/meta")}},
    upload:function(file){{
      if(!file) return Promise.reject(new Error("No file"));
      var form=new FormData();
      form.append("file",file,file.name||"upload");
      return postForm("/user-docs",form);
    }}
  }};
  this.auth={{
    session:function(){{return get("/auth/session").catch(function(){{return null;}})}},
    register:function(payload){{return postWithVisitor("/auth/register",payload||{{}})}},
    login:function(payload){{return postWithVisitor("/auth/login",payload||{{}})}},
    logout:function(){{return post("/auth/logout",{{}})}}
  }};
  this.data={{
    list:function(){{return get("/data")}},
    get:function(key){{return get("/data/"+encodeURIComponent(key))}},
    set:function(key,value){{return fetch(BASE+"/data/"+encodeURIComponent(key),{{method:"PUT",credentials:"same-origin",headers:{{"Content-Type":"application/json","x-visitor-id":vid()}},body:JSON.stringify(value)}}).then(function(r){{if(!r.ok)throw new Error(r.status+" "+r.statusText)}})}}
  }};
  this.config={{get:function(){{return get("/config")}}}};
  this.track=function(type,payload,pagePath){{var body={{visitorId:vid(),interactionType:type,data:payload||{{}}}};if(pagePath)body.pagePath=pagePath;return post("/interact",body).catch(function(){{}})}}
}}

global.PortalSDK=PortalSDK;
}})(typeof window!=="undefined"?window:this);
"##,
        slug = slug
    )
}

/// Self-contained chat widget with real Agent SSE streaming
fn render_chat_widget(portal: &Portal) -> String {
    let slug = sanitize_slug_for_js(&portal.slug);
    let welcome = portal
        .agent_welcome_message
        .as_deref()
        .unwrap_or("Hi! How can I help you?");
    let header_title = if PortalService::is_digital_avatar_portal(portal) {
        "与数字分身对话"
    } else {
        "Chat"
    };
    let input_placeholder = if PortalService::is_digital_avatar_portal(portal) {
        "请输入你的目标、问题或需要处理的事项..."
    } else {
        "Type a message..."
    };
    format!(
        r##"<div id="portal-chat-widget">
<style>
#pcw-btn{{position:fixed;bottom:20px;right:20px;width:56px;height:56px;border-radius:50%;background:#2563eb;color:#fff;border:none;cursor:pointer;font-size:24px;box-shadow:0 4px 12px rgba(0,0,0,.15);z-index:9999;display:flex;align-items:center;justify-content:center}}
#pcw-panel{{position:fixed;bottom:88px;right:20px;width:380px;max-height:520px;background:#fff;border-radius:12px;box-shadow:0 8px 30px rgba(0,0,0,.12);z-index:9999;display:none;flex-direction:column;overflow:hidden}}
#pcw-header{{background:#2563eb;color:#fff;padding:14px 16px;font-weight:600;display:flex;justify-content:space-between;align-items:center}}
#pcw-header button{{background:none;border:none;color:#fff;cursor:pointer;font-size:18px}}
#pcw-status{{display:none;align-items:center;justify-content:space-between;gap:8px;padding:8px 12px;background:#f8fafc;border-top:1px solid #e5e7eb;border-bottom:1px solid #e5e7eb;font-size:12px;color:#475569}}
.pcw-status-main{{display:flex;align-items:center;gap:8px;min-width:0;flex:1}}
.pcw-status-dot{{width:8px;height:8px;border-radius:999px;background:#2563eb;animation:pcwPulse 1.2s ease-in-out infinite}}
.pcw-status-text{{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}}
.pcw-status-elapsed{{font-variant-numeric:tabular-nums;color:#64748b;flex-shrink:0}}
@keyframes pcwPulse{{0%,100%{{opacity:.35}}50%{{opacity:1}}}}
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
  <div id="pcw-header"><span>{header_title}</span><button onclick="pcwToggle()">✕</button></div>
  <div id="pcw-status">
    <div class="pcw-status-main">
      <span class="pcw-status-dot"></span>
      <span id="pcw-status-text" class="pcw-status-text">处理中...</span>
    </div>
    <span id="pcw-status-elapsed" class="pcw-status-elapsed">0s</span>
  </div>
  <div id="pcw-messages" style="display:flex;flex-direction:column"></div>
  <div id="pcw-input-row">
    <input id="pcw-input" placeholder="{input_placeholder}" onkeydown="if(event.key==='Enter'&&!event.shiftKey)pcwSend()">
    <button id="pcw-send" onclick="pcwSend()">Send</button>
  </div>
</div>
<script>
(function(){{
  var SLUG="{slug}";
  var vid=localStorage.getItem('pcw_vid');
  if(!vid){{vid='v_'+Array.from(crypto.getRandomValues(new Uint8Array(9)),function(b){{return b.toString(36)}}).join('').substring(0,12);localStorage.setItem('pcw_vid',vid)}}
  var sidKey='pcw_sid_'+SLUG+'_'+vid;
  var msgsKey='pcw_msgs_'+SLUG+'_'+vid;
  var legacySidKey='pcw_sid_'+SLUG;
  var legacyMsgsKey='pcw_msgs_'+SLUG;
  function safeParseArray(raw){{try{{var v=JSON.parse(raw||'[]');return Array.isArray(v)?v:[]}}catch(_e){{return []}}}}
  function migrateLegacyStorage(){{
    var legacySid=localStorage.getItem(legacySidKey)||sessionStorage.getItem(legacySidKey)||'';
    if(!localStorage.getItem(sidKey) && legacySid) localStorage.setItem(sidKey, legacySid);
    var legacyMsgsRaw=localStorage.getItem(legacyMsgsKey)||sessionStorage.getItem(legacyMsgsKey)||'';
    if(!localStorage.getItem(msgsKey) && legacyMsgsRaw) localStorage.setItem(msgsKey, legacyMsgsRaw);
    sessionStorage.removeItem(legacySidKey);
    sessionStorage.removeItem(legacyMsgsKey);
  }}
  migrateLegacyStorage();
  var sessionId=localStorage.getItem(sidKey)||'';
  var msgs=safeParseArray(localStorage.getItem(msgsKey));
  var panel=document.getElementById('pcw-panel');
  var msgBox=document.getElementById('pcw-messages');
  var sendBtn=document.getElementById('pcw-send');
  var inputEl=document.getElementById('pcw-input');
  var statusBar=document.getElementById('pcw-status');
  var statusTextEl=document.getElementById('pcw-status-text');
  var statusElapsedEl=document.getElementById('pcw-status-elapsed');
  var busy=false;
  var evtSource=null;
  var currentBotEl=null;
  var currentBotText='';
  var lastEventId='';
  var statusTimer=null;
  var heartbeatTimer=null;
  var startedAt=0;
  var lastActivityAt=0;

  function saveSessionId(id){{sessionId=id||'';if(sessionId)localStorage.setItem(sidKey,sessionId);else localStorage.removeItem(sidKey)}}
  function clearSessionId(){{sessionId='';localStorage.removeItem(sidKey);lastEventId=''}}
  function saveMsgs(){{if(msgs.length>200)msgs=msgs.slice(msgs.length-200);localStorage.setItem(msgsKey,JSON.stringify(msgs))}}
  function setStatusVisible(show){{if(statusBar)statusBar.style.display=show?'flex':'none'}}
  function clearStatusTimers(){{
    if(statusTimer){{clearInterval(statusTimer);statusTimer=null}}
    if(heartbeatTimer){{clearInterval(heartbeatTimer);heartbeatTimer=null}}
  }}
  function formatElapsed(sec){{return String(sec<0?0:sec)+'s'}}
  function touchStatus(text){{
    lastActivityAt=Date.now();
    if(statusTextEl && text) statusTextEl.textContent=text;
  }}
  function startStatus(text){{
    startedAt=Date.now();
    touchStatus(text||'请求已发送，等待代理执行...');
    setStatusVisible(true);
    if(statusElapsedEl) statusElapsedEl.textContent='0s';
    clearStatusTimers();
    statusTimer=setInterval(function(){{
      if(!busy || !statusElapsedEl) return;
      statusElapsedEl.textContent=formatElapsed(Math.floor((Date.now()-startedAt)/1000));
    }},1000);
    heartbeatTimer=setInterval(function(){{
      if(!busy) return;
      if(Date.now()-lastActivityAt>12000) touchStatus('仍在处理中，请稍候...');
    }},3000);
  }}
  function stopStatus(finalText){{
    clearStatusTimers();
    if(finalText) touchStatus(finalText);
    if(statusElapsedEl && startedAt>0) statusElapsedEl.textContent=formatElapsed(Math.floor((Date.now()-startedAt)/1000));
    setTimeout(function(){{if(!busy)setStatusVisible(false)}}, finalText?1200:0);
  }}
  function mapStatus(raw){{
    var s=String(raw||'').toLowerCase();
    if(!s || s==='running') return '处理中...';
    if(s.indexOf('llm')>=0) return '正在调用模型...';
    if(s.indexOf('portal_tool_retry')>=0) return '正在重试工具链路...';
    if(s.indexOf('tool')>=0) return '正在执行工具...';
    if(s.indexOf('compaction')>=0) return '正在整理上下文...';
    return String(raw);
  }}
  function safeParseJson(raw){{try{{return JSON.parse(raw||'{{}}')}}catch(_e){{return {{}}}}}}

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
    inputEl.disabled=b;
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
      saveSessionId(d.session_id);
      cb();
    }}).catch(function(e){{
      console.error('Chat session error:',e);
      setBusy(false);
      stopStatus('会话创建失败');
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
    evtSource.onopen=function(){{touchStatus('已连接，正在执行...')}};

    evtSource.addEventListener('status',function(e){{
      if(e.lastEventId)lastEventId=e.lastEventId;
      var d=safeParseJson(e.data);
      touchStatus(mapStatus(d.status));
    }});

    evtSource.addEventListener('toolcall',function(e){{
      if(e.lastEventId)lastEventId=e.lastEventId;
      var d=safeParseJson(e.data);
      touchStatus('调用工具：'+String(d.name||'tool'));
    }});

    evtSource.addEventListener('toolresult',function(e){{
      if(e.lastEventId)lastEventId=e.lastEventId;
      var d=safeParseJson(e.data);
      var toolName=String(d.name||'tool');
      touchStatus((d.success===false?'工具失败：':'工具完成：')+toolName);
    }});

    evtSource.addEventListener('turn',function(e){{
      if(e.lastEventId)lastEventId=e.lastEventId;
      var d=safeParseJson(e.data);
      if(d.current&&d.max) touchStatus('执行轮次 '+d.current+'/'+d.max);
    }});

    evtSource.addEventListener('compaction',function(e){{
      if(e.lastEventId)lastEventId=e.lastEventId;
      touchStatus('正在整理上下文...');
    }});

    evtSource.addEventListener('text',function(e){{
      if(e.lastEventId)lastEventId=e.lastEventId;
      removeTyping();
      var data=safeParseJson(e.data);
      if(!currentBotEl){{
        currentBotEl=document.createElement('div');
        currentBotEl.className='pcw-msg bot';
        msgBox.appendChild(currentBotEl);
      }}
      currentBotText+=data.content;
      currentBotEl.textContent=currentBotText;
      msgBox.scrollTop=msgBox.scrollHeight;
      touchStatus('正在生成回复...');
    }});

    evtSource.addEventListener('thinking',function(e){{
      if(e.lastEventId)lastEventId=e.lastEventId;
      removeTyping();addTyping();
      touchStatus('思考中...');
    }});

    evtSource.addEventListener('done',function(e){{
      if(e.lastEventId)lastEventId=e.lastEventId;
      var data=safeParseJson(e.data);
      removeTyping();
      evtSource.close();evtSource=null;
      if(currentBotText){{
        msgs.push({{role:'bot',content:currentBotText,ts:Date.now()}});
        saveMsgs();
      }}
      if(data && data.error){{
        msgs.push({{role:'bot',content:'⚠ '+String(data.error),ts:Date.now()}});
        saveMsgs();
        render();
        stopStatus('执行失败');
      }} else {{
        stopStatus('执行完成');
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
        msgs.push({{role:'bot',content:currentBotText,ts:Date.now()}});
        saveMsgs();
      }}
      currentBotEl=null;currentBotText='';
      setBusy(false);
      stopStatus('连接中断');
    }};
  }}

  window.pcwToggle=function(){{
    var vis=panel.style.display==='flex';
    panel.style.display=vis?'none':'flex';
  }};

  window.pcwSend=function(){{
    if(busy)return;
    var inp=inputEl;
    var text=inp.value.trim();if(!text)return;
    msgs.push({{role:'user',content:text,ts:Date.now()}});saveMsgs();
    inp.value='';render();
    setBusy(true);addTyping();startStatus('请求已发送，等待代理执行...');

    function postMessage(retried){{
      fetch('/p/'+SLUG+'/api/chat/message',{{
        method:'POST',headers:{{'Content-Type':'application/json'}},
        body:JSON.stringify({{session_id:sessionId,visitor_id:vid,content:text}})
      }}).then(function(r){{
        if(!r.ok){{
          if(!retried && (r.status===403 || r.status===404)){{
            clearSessionId();
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
        stopStatus('发送失败');
      }});
    }}

    ensureSession(function(){{ postMessage(false); }});

    // Also log interaction (fire and forget)
    fetch('/p/'+SLUG+'/api/interact',{{
      method:'POST',headers:{{'Content-Type':'application/json'}},
      body:JSON.stringify({{visitorId:vid,interactionType:'chat_message',data:{{message:text}}}})
    }}).catch(function(){{}});
  }};
  window.addEventListener('beforeunload',function(){{if(evtSource)evtSource.close()}});
}})();
</script>
</div>"##,
        slug = slug,
        header_title = header_title,
        input_placeholder = input_placeholder,
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
    // L-3: Use a fixed salt to prevent rainbow-table reversal of IP hashes
    if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let ip = forwarded.split(',').next().unwrap_or("").trim();
        if !ip.is_empty() {
            const SALT: u64 = 0xa3f7_c291_e5b8_4d06;
            return format!(
                "ip_{:x}",
                ip.bytes()
                    .fold(SALT, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64))
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

fn synthetic_visitor_user_id(visitor_id: &str) -> String {
    format!("portal_visitor_{visitor_id}")
}

fn portal_user_upload_root() -> &'static str {
    "/用户上传文档"
}

fn portal_user_upload_folder(slug: &str) -> String {
    format!("{}/{}", portal_user_upload_root(), slug)
}

fn guess_mime_from_extension(filename: &str) -> Option<&'static str> {
    let ext = filename.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "md" | "markdown" => Some("text/markdown"),
        "txt" => Some("text/plain"),
        "json" => Some("application/json"),
        "yaml" | "yml" => Some("application/x-yaml"),
        "xml" => Some("application/xml"),
        "html" | "htm" => Some("text/html"),
        "csv" => Some("text/csv"),
        "pdf" => Some("application/pdf"),
        "doc" => Some("application/msword"),
        "docx" => Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        "xls" => Some("application/vnd.ms-excel"),
        "xlsx" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "ppt" => Some("application/vnd.ms-powerpoint"),
        "pptx" => Some("application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

fn is_allowed_public_upload_mime(mime_type: &str) -> bool {
    let mime = mime_type.trim().to_ascii_lowercase();
    if mime.is_empty() {
        return false;
    }
    mime.starts_with("text/")
        || mime.starts_with("image/")
        || matches!(
            mime.as_str(),
            "application/pdf"
                | "application/json"
                | "application/xml"
                | "application/x-yaml"
                | "application/msword"
                | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                | "application/vnd.ms-excel"
                | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                | "application/vnd.ms-powerpoint"
                | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        )
}

fn visitor_id_from_header(headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    let raw = headers
        .get("x-visitor-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    normalize_visitor_id(raw).ok_or((StatusCode::BAD_REQUEST, "Invalid visitor_id".into()))
}

#[derive(Clone)]
struct ResolvedPublicIdentity {
    user_id: String,
    visitor_id: Option<String>,
}

async fn resolve_external_user_from_headers(
    state: &PortalPublicState,
    team_id: &str,
    headers: &HeaderMap,
) -> Result<Option<ExternalUser>, (StatusCode, String)> {
    let Some(session_id) = external_user_session_id_from_headers(headers) else {
        return Ok(None);
    };
    ExternalUserService::new(state.db.clone())
        .validate_session(team_id, &session_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn resolve_public_identity(
    state: &PortalPublicState,
    team_id: &str,
    headers: &HeaderMap,
    explicit_visitor_id: Option<&str>,
) -> Result<ResolvedPublicIdentity, (StatusCode, String)> {
    if let Some(user) = resolve_external_user_from_headers(state, team_id, headers).await? {
        let visitor_id = explicit_visitor_id
            .and_then(normalize_visitor_id)
            .or_else(|| {
                headers
                    .get("x-visitor-id")
                    .and_then(|value| value.to_str().ok())
                    .and_then(normalize_visitor_id)
            });
        return Ok(ResolvedPublicIdentity {
            user_id: user.external_user_id.clone(),
            visitor_id,
        });
    }

    let visitor_id = explicit_visitor_id
        .and_then(normalize_visitor_id)
        .or_else(|| {
            headers
                .get("x-visitor-id")
                .and_then(|value| value.to_str().ok())
                .and_then(normalize_visitor_id)
        })
        .ok_or((StatusCode::BAD_REQUEST, "Invalid visitor_id".into()))?;
    Ok(ResolvedPublicIdentity {
        user_id: synthetic_visitor_user_id(&visitor_id),
        visitor_id: Some(visitor_id),
    })
}

async fn list_visitor_upload_sources(
    doc_svc: &DocumentService,
    team_id: &str,
    portal_slug: &str,
    public_user_id: &str,
) -> Result<Vec<agime_team::models::mongo::DocumentSummary>, (StatusCode, String)> {
    doc_svc
        .list_by_uploader_in_folder(
            team_id,
            public_user_id,
            &portal_user_upload_folder(portal_slug),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn list_visitor_related_ai_documents(
    doc_svc: &DocumentService,
    team_id: &str,
    source_docs: &[agime_team::models::mongo::DocumentSummary],
) -> Result<Vec<agime_team::models::mongo::DocumentSummary>, (StatusCode, String)> {
    let source_ids = source_docs
        .iter()
        .map(|doc| doc.id.clone())
        .collect::<Vec<_>>();
    if source_ids.is_empty() {
        return Ok(Vec::new());
    }
    doc_svc
        .list_related_ai_documents(team_id, &source_ids, Some(1), Some(500))
        .await
        .map(|result| result.items)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn build_visitor_user_doc_payload(
    doc_svc: &DocumentService,
    team_id: &str,
    portal_slug: &str,
    public_user_id: &str,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let source_docs =
        list_visitor_upload_sources(doc_svc, team_id, portal_slug, public_user_id).await?;
    let related_ai_docs = list_visitor_related_ai_documents(doc_svc, team_id, &source_docs).await?;
    let documents = source_docs
        .iter()
        .map(|doc| {
            let related = related_ai_docs
                .iter()
                .filter(|candidate| candidate.source_document_ids.iter().any(|id| id == &doc.id))
                .map(|candidate| {
                    serde_json::json!({
                        "id": candidate.id,
                        "name": candidate.name,
                        "display_name": candidate.display_name,
                        "mime_type": candidate.mime_type,
                        "file_size": candidate.file_size,
                        "origin": candidate.origin,
                        "status": candidate.status,
                        "category": candidate.category,
                        "source_document_ids": candidate.source_document_ids,
                        "source_session_id": candidate.source_session_id,
                        "source_mission_id": candidate.source_mission_id,
                        "lineage_description": candidate.lineage_description,
                        "updated_at": candidate.updated_at.to_rfc3339(),
                        "created_at": candidate.created_at.to_rfc3339(),
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "id": doc.id,
                "name": doc.name,
                "display_name": doc.display_name,
                "mime_type": doc.mime_type,
                "file_size": doc.file_size,
                "folder_path": doc.folder_path,
                "origin": doc.origin,
                "status": doc.status,
                "category": doc.category,
                "updated_at": doc.updated_at.to_rfc3339(),
                "created_at": doc.created_at.to_rfc3339(),
                "related_ai_documents": related,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "documents": documents }))
}

async fn resolve_visitor_accessible_document(
    doc_svc: &DocumentService,
    team_id: &str,
    portal_slug: &str,
    public_user_id: &str,
    doc_id: &str,
) -> Result<Option<agime_team::models::mongo::DocumentSummary>, (StatusCode, String)> {
    if let Some(meta) = doc_svc
        .get_by_uploader_in_folder(
            team_id,
            public_user_id,
            &portal_user_upload_folder(portal_slug),
            doc_id,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    {
        return Ok(Some(meta));
    }
    let source_docs =
        list_visitor_upload_sources(doc_svc, team_id, portal_slug, public_user_id).await?;
    let related_ai_docs = list_visitor_related_ai_documents(doc_svc, team_id, &source_docs).await?;
    Ok(related_ai_docs.into_iter().find(|doc| doc.id == doc_id))
}

async fn load_public_portal(
    state: &PortalPublicState,
    slug: &str,
) -> Result<(Portal, PortalEffectivePublicConfig), (StatusCode, String)> {
    let svc = PortalService::new((*state.db).clone());
    let portal = svc
        .get_by_slug(slug)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Portal not found".to_string()))?;
    let effective = svc
        .resolve_effective_public_config(&portal)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !effective.public_access_enabled {
        return Err((StatusCode::NOT_FOUND, "Portal not published".into()));
    }
    Ok((portal, effective))
}

async fn resolve_portal_team_name(state: &PortalPublicState, portal: &Portal) -> Option<String> {
    let team_id = portal.team_id.to_hex();
    TeamService::new((*state.db).clone())
        .get(&team_id)
        .await
        .ok()
        .flatten()
        .map(|team| team.name)
        .filter(|value| !value.trim().is_empty())
}

/// Validate that a session belongs to the given visitor and portal.
fn validate_session_ownership(
    session: &super::session_mongo::AgentSessionDoc,
    expected_user_id: &str,
    portal_id: &str,
    slug: &str,
    visitor_id: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    if session.user_id != expected_user_id {
        return Err((StatusCode::FORBIDDEN, "Session mismatch".into()));
    }
    if session.portal_id.as_deref() != Some(portal_id) {
        return Err((StatusCode::FORBIDDEN, "Portal session mismatch".into()));
    }
    if session.portal_slug.as_deref() != Some(slug) {
        return Err((StatusCode::FORBIDDEN, "Portal slug mismatch".into()));
    }
    if expected_user_id.starts_with("portal_visitor_")
        && session.visitor_id.as_deref() != visitor_id
    {
        return Err((StatusCode::FORBIDDEN, "Visitor mismatch".into()));
    }
    Ok(())
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
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Serve a file from the portal's project folder.
/// Returns (body_bytes, content_type). Injects chat widget into HTML files.
async fn serve_from_filesystem(
    project_path: &str,
    relative_path: &str,
    portal: &Portal,
    effective: &PortalEffectivePublicConfig,
    team_name: Option<&str>,
) -> Result<(Vec<u8>, String), (StatusCode, String)> {
    let base = std::path::Path::new(project_path);

    // Determine the file path
    let file_path = if relative_path.is_empty() || relative_path == "index" {
        base.join("index.html")
    } else if let Some(sanitized) = sanitize_path(relative_path) {
        // Block access to _private/ directory (case-insensitive for Windows)
        if sanitized
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("_private")
        {
            return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
        }
        let candidate = base.join(&sanitized);
        let is_dir = tokio::fs::metadata(&candidate)
            .await
            .map(|m| m.is_dir())
            .unwrap_or(false);
        if is_dir {
            candidate.join("index.html")
        } else if tokio::fs::metadata(&candidate).await.is_ok() {
            candidate
        } else {
            // SPA fallback: try root index.html for paths without extensions
            let has_ext = sanitized.extension().is_some_and(|e| !e.is_empty());
            if !has_ext {
                base.join("index.html")
            } else {
                return Err((StatusCode::NOT_FOUND, "File not found".to_string()));
            }
        }
    } else {
        return Err((StatusCode::BAD_REQUEST, "Invalid path".to_string()));
    };

    if tokio::fs::metadata(&file_path).await.is_err() {
        return Err((StatusCode::NOT_FOUND, "File not found".to_string()));
    }

    // M-2: Security: ensure resolved path is within project_path (fail on canonicalize error)
    let canonical_base = tokio::fs::canonicalize(base).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Base path error: {}", e),
        )
    })?;
    let canonical_file = tokio::fs::canonicalize(&file_path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("File path error: {}", e),
        )
    })?;
    if !canonical_file.starts_with(&canonical_base) {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    let body = tokio::fs::read(&file_path).await.map_err(|e| {
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
        let html = String::from_utf8_lossy(&body).to_string();
        let is_index = relative_path.is_empty() || relative_path == "index";
        let should_upgrade_avatar_default = is_index
            && PortalService::is_digital_avatar_portal(portal)
            && (html.contains("This portal is ready for development.")
                || PortalService::is_generated_digital_avatar_index_html(&html));
        let mut rendered_html = if should_upgrade_avatar_default {
            PortalService::render_digital_avatar_index_html_with_effective(
                portal, effective, team_name,
            )
        } else {
            html
        };
        let is_generated_avatar_default =
            PortalService::is_generated_digital_avatar_index_html(&rendered_html);

        if effective.chat_enabled
            && effective.show_chat_widget
            && !should_upgrade_avatar_default
            && !is_generated_avatar_default
        {
            let widget = render_chat_widget(portal);
            // Insert before </body> if present, otherwise append
            rendered_html = if let Some(pos) = rendered_html.rfind("</body>") {
                format!(
                    "{}{}{}",
                    &rendered_html[..pos],
                    widget,
                    &rendered_html[pos..]
                )
            } else {
                format!("{}{}", rendered_html, widget)
            };
        }
        if should_upgrade_avatar_default {
            tracing::info!(
                "Auto-upgraded legacy default avatar index template for slug={}",
                portal.slug
            );
        }
        return Ok((rendered_html.into_bytes(), mime));
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
    let (portal, effective) = load_public_portal(&state, &slug).await?;
    let team_name = resolve_portal_team_name(&state, &portal).await;

    let portal_id = portal.id.unwrap_or_default();

    // Filesystem-first: published portals may be served from filesystem.
    if let Some(ref project_path) = portal.project_path {
        let (body, content_type) =
            serve_from_filesystem(project_path, "", &portal, &effective, team_name.as_deref())
                .await?;

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

        return Ok((
            [
                (header::CONTENT_TYPE, content_type),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
                (header::X_FRAME_OPTIONS, "SAMEORIGIN".to_string()),
            ],
            body,
        ));
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

    let (portal, effective) = load_public_portal(&state, &slug).await?;
    let team_name = resolve_portal_team_name(&state, &portal).await;

    // Serve built-in Portal SDK JS for published portals only.
    if path == "portal-sdk.js" {
        return Ok((
            [
                (
                    header::CONTENT_TYPE,
                    "application/javascript; charset=utf-8".to_string(),
                ),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
                (header::X_FRAME_OPTIONS, "SAMEORIGIN".to_string()),
            ],
            render_portal_sdk(&slug).into_bytes(),
        ));
    }

    let portal_id = portal.id.unwrap_or_default();

    // Filesystem-first: published portals may be served from filesystem.
    if let Some(ref project_path) = portal.project_path {
        let (body, content_type) = serve_from_filesystem(
            project_path,
            &path,
            &portal,
            &effective,
            team_name.as_deref(),
        )
        .await?;

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

        return Ok((
            [
                (header::CONTENT_TYPE, content_type),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
                (header::X_FRAME_OPTIONS, "SAMEORIGIN".to_string()),
            ],
            body,
        ));
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
    let (portal, _) = load_public_portal(&state, &slug).await?;
    let svc = PortalService::new((*state.db).clone());

    // M-3: normalize visitor_id like other chat endpoints
    let visitor_id = normalize_visitor_id(&req.visitor_id)
        .ok_or((StatusCode::BAD_REQUEST, "Invalid visitor_id".into()))?;

    svc.log_interaction(PortalInteraction {
        id: None,
        portal_id: portal.id.unwrap_or_default(),
        team_id: portal.team_id,
        visitor_id,
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
    let (portal, effective) = load_public_portal(&state, &slug).await?;
    let effective_public_config = effective.clone();
    let effective_allowed_extensions = effective.effective_allowed_extensions.clone();
    let effective_allowed_skill_ids = effective.effective_allowed_skill_ids.clone();
    let effective_allowed_skill_names = effective.effective_allowed_skill_names.clone();

    Ok(Json(serde_json::json!({
        "apiVersion": "v1",
        "name": portal.name,
        "portalKind": if PortalService::is_digital_avatar_portal(&portal) { "digital_avatar" } else { "ecosystem_portal" },
        "agentEnabled": effective.chat_enabled,
        "showChatWidget": effective.show_chat_widget,
        "documentAccessMode": PortalService::document_access_mode_key(effective.effective_document_access_mode),
        "agentWelcomeMessage": portal.agent_welcome_message,
        "effectivePublicConfig": effective_public_config,
        "avatarProfile": if PortalService::is_digital_avatar_portal(&portal) {
            serde_json::json!({
                "avatarTypeLabel": PortalService::resolve_avatar_type_label(&portal),
                "runModeLabel": PortalService::resolve_run_mode_label(&portal),
                "documentAccessLabel": PortalService::resolve_doc_mode_label_for_mode(effective.effective_document_access_mode),
                "boundDocumentCount": portal.bound_document_ids.len(),
                "allowedExtensions": effective_allowed_extensions,
                "allowedSkillIds": effective_allowed_skill_ids,
                "allowedSkillNames": effective_allowed_skill_names,
                "extensionsInherited": effective.extensions_inherited,
                "skillsInherited": effective.skills_inherited,
                "exposure": effective.exposure,
            })
        } else {
            serde_json::Value::Null
        },
        "chatApi": {
            "sessionPath": format!("/p/{}/api/chat/session", slug),
            "messagePath": format!("/p/{}/api/chat/message", slug),
            "streamPathTemplate": format!("/p/{}/api/chat/stream/{{session_id}}", slug),
        },
        "userDocsApi": {
            "listPath": format!("/p/{}/api/user-docs", slug),
            "contentPathTemplate": format!("/p/{}/api/user-docs/{{doc_id}}", slug),
            "metaPathTemplate": format!("/p/{}/api/user-docs/{{doc_id}}/meta", slug),
            "identityMode": "external_user_or_anonymous_browser_local",
        },
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicRegisterRequest {
    username: String,
    password: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    phone: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicLoginRequest {
    username: String,
    password: String,
}

async fn register_public_user(
    State(state): State<PortalPublicState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(req): Json<PublicRegisterRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (portal, _) = load_public_portal(&state, &slug).await?;
    let visitor_id = headers
        .get("x-visitor-id")
        .and_then(|value| value.to_str().ok())
        .and_then(normalize_visitor_id);
    let ip_address = extract_client_ip(&headers);
    let user_agent = extract_user_agent(&headers);
    let svc = ExternalUserService::new(state.db.clone());
    let user = svc
        .register(
            &portal.team_id.to_hex(),
            &req.username,
            &req.password,
            req.display_name,
            req.phone,
            visitor_id.as_deref(),
            ip_address.as_deref(),
            user_agent.as_deref(),
        )
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let session = svc
        .create_session(
            &portal.team_id.to_hex(),
            &user.external_user_id,
            visitor_id.as_deref(),
            ip_address.as_deref(),
            user_agent.as_deref(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((
        [(
            header::SET_COOKIE,
            build_external_user_session_cookie(&session.session_id, state.secure_cookies),
        )],
        Json(serde_json::json!({
            "authenticated": true,
            "user": user.to_public_profile(),
        })),
    ))
}

async fn login_public_user(
    State(state): State<PortalPublicState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(req): Json<PublicLoginRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (portal, _) = load_public_portal(&state, &slug).await?;
    let visitor_id = headers
        .get("x-visitor-id")
        .and_then(|value| value.to_str().ok())
        .and_then(normalize_visitor_id);
    let ip_address = extract_client_ip(&headers);
    let user_agent = extract_user_agent(&headers);
    let svc = ExternalUserService::new(state.db.clone());
    let user = svc
        .authenticate(
            &portal.team_id.to_hex(),
            &req.username,
            &req.password,
            visitor_id.as_deref(),
            ip_address.as_deref(),
            user_agent.as_deref(),
        )
        .await
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
    let session = svc
        .create_session(
            &portal.team_id.to_hex(),
            &user.external_user_id,
            visitor_id.as_deref(),
            ip_address.as_deref(),
            user_agent.as_deref(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((
        [(
            header::SET_COOKIE,
            build_external_user_session_cookie(&session.session_id, state.secure_cookies),
        )],
        Json(serde_json::json!({
            "authenticated": true,
            "user": user.to_public_profile(),
        })),
    ))
}

async fn logout_public_user(
    State(state): State<PortalPublicState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (_portal, _) = load_public_portal(&state, &slug).await?;
    if let Some(session_id) = external_user_session_id_from_headers(&headers) {
        let svc = ExternalUserService::new(state.db.clone());
        svc.delete_session(&session_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    Ok((
        [(
            header::SET_COOKIE,
            build_external_user_clear_cookie(state.secure_cookies),
        )],
        Json(serde_json::json!({
            "authenticated": false,
            "user": serde_json::Value::Null,
        })),
    ))
}

async fn get_public_user_session(
    State(state): State<PortalPublicState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (portal, _) = load_public_portal(&state, &slug).await?;
    if let Some(user) =
        resolve_external_user_from_headers(&state, &portal.team_id.to_hex(), &headers).await?
    {
        return Ok(Json(serde_json::json!({
            "authenticated": true,
            "user": user.to_public_profile(),
        })));
    }

    Ok(Json(serde_json::json!({
        "authenticated": false,
        "user": serde_json::Value::Null,
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
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(req): Json<CreateVisitorSessionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let svc = PortalService::new((*state.db).clone());
    let (portal, effective) = load_public_portal(&state, &slug).await?;
    if !effective.chat_enabled {
        return Err((StatusCode::BAD_REQUEST, "Agent not enabled".into()));
    }
    let agent_id = PortalService::resolve_service_agent_id(&portal)
        .ok_or((StatusCode::BAD_REQUEST, "No agent configured".into()))?;
    let portal_id = portal.id.map(|id| id.to_hex()).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Portal id missing".into(),
    ))?;
    let identity = resolve_public_identity(
        &state,
        &portal.team_id.to_hex(),
        &headers,
        Some(&req.visitor_id),
    )
    .await?;
    let agent_svc = AgentService::new(state.db.clone());
    let doc_svc = DocumentService::new((*state.db).clone());
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
    // Inject runtime environment info so the agent knows its OS and shell.
    // This mirrors what the local developer extension provides via get_info().
    {
        let os = std::env::consts::OS;
        let (shell, shell_syntax_hint) = if cfg!(target_os = "windows") {
            ("cmd.exe", "使用 cmd.exe 语法（如 dir, type, copy）。不要使用 PowerShell 语法（如 Get-Content, ls）或 bash 语法（如 cat, grep）。文件路径可用正斜杠或反斜杠。注意：cmd.exe 默认编码可能不是 UTF-8，写入文件时优先使用 text_editor 而非 echo 重定向。")
        } else {
            ("sh", "使用标准 POSIX shell 语法。")
        };
        extra_instructions_parts.push(format!(
            "运行环境:\n\
             - 操作系统: {os}\n\
             - Shell工具使用的解释器: {shell}\n\
             - {shell_syntax_hint}\n\
             - 在进行文档读写前，先调用 document_session_policy 工具确认本会话文档权限与可访问范围，再执行 read/create/update。"
        ));
    }

    if let Some(ref project_path) = normalized_project_path {
        extra_instructions_parts.push(format!(
            "你的项目工作目录是: {}\n\
             重要规则:\n\
             1. 只在此目录下操作文件，禁止访问父目录或其他目录。\n\
             2. 绑定的文档(bound_documents)已自动注入到你的上下文中，前端不需要手动拉取文档内容拼接到消息里。\n\
             3. 使用 text_editor 编辑文件时用相对路径（如 index.html）。\n\
             4. 完成修改后告知用户改动了哪些文件。\n\
             5. 向用户提供完整的预览地址时，使用完整URL而非相对路径。",
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

    let allowed_extensions =
        normalize_optional_string_list(Some(effective.effective_allowed_extensions.clone()));
    let allowed_skill_ids =
        normalize_optional_string_list(Some(effective.effective_allowed_skill_ids.clone()));
    let document_access_mode =
        PortalService::document_access_mode_key(effective.effective_document_access_mode)
            .to_string();
    let mut attached_document_ids = portal.bound_document_ids.clone();
    let visitor_uploads = list_visitor_upload_sources(
        &doc_svc,
        &portal.team_id.to_hex(),
        &portal.slug,
        &identity.user_id,
    )
    .await?;
    for doc in visitor_uploads {
        if !attached_document_ids.iter().any(|id| id == &doc.id) {
            attached_document_ids.push(doc.id);
        }
    }

    // Reuse only a session already bound to this exact portal.
    if let Ok(Some(session)) = agent_svc
        .find_active_portal_session(&identity.user_id, &agent_id, &portal_id)
        .await
    {
        if let Err(e) = agent_svc
            .sync_portal_session_policy(
                &session.session_id,
                attached_document_ids.clone(),
                extra_instructions.clone(),
                allowed_extensions.clone(),
                allowed_skill_ids.clone(),
                None,
                false,
                Some(document_access_mode.clone()),
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
                identity.visitor_id.as_deref(),
                Some(document_access_mode.as_str()),
                true,
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
            &identity.user_id,
            attached_document_ids,
            extra_instructions,
            allowed_extensions,
            allowed_skill_ids,
            None,
            None,
            None,
            None,
            false,
            true,
            Some(document_access_mode.clone()),
            Some("portal".to_string()),
            None,
            Some(true),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    agent_svc
        .set_session_portal_context(
            &session.session_id,
            &portal_id,
            &portal.slug,
            identity.visitor_id.as_deref(),
            Some(document_access_mode.as_str()),
            true,
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
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(req): Json<SendVisitorMessageRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let content = req.content.trim().to_string();
    if content.is_empty() || content.len() > 100_000 {
        return Err((StatusCode::BAD_REQUEST, "Invalid message".into()));
    }

    let (portal, effective) = load_public_portal(&state, &slug).await?;
    let portal_id = portal.id.map(|id| id.to_hex()).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Portal id missing".into(),
    ))?;

    if !effective.chat_enabled {
        return Err((StatusCode::BAD_REQUEST, "Chat not available".into()));
    }

    let identity = resolve_public_identity(
        &state,
        &portal.team_id.to_hex(),
        &headers,
        Some(&req.visitor_id),
    )
    .await?;
    let agent_svc = AgentService::new(state.db.clone());

    // Verify session exists and belongs to this visitor
    let session = agent_svc
        .get_session(&req.session_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into()))?
        .ok_or((StatusCode::NOT_FOUND, "Session not found".into()))?;

    validate_session_ownership(
        &session,
        &identity.user_id,
        &portal_id,
        &slug,
        identity.visitor_id.as_deref(),
    )?;

    // Register in ChatManager first (authoritative in-memory gate)
    let (cancel_token, _stream_tx) = match state.chat_manager.register(&req.session_id).await {
        Some(pair) => pair,
        None => {
            return Err((StatusCode::CONFLICT, "Already active".into()));
        }
    };

    // Then set MongoDB is_processing flag (secondary persistence)
    let claimed = agent_svc
        .try_start_processing(&req.session_id, &identity.user_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error".to_string()));
    match claimed {
        Ok(true) => {}
        _ => {
            // Rollback ChatManager registration
            state.chat_manager.unregister(&req.session_id).await;
            return Err(claimed
                .err()
                .unwrap_or((StatusCode::CONFLICT, "Already processing".into())));
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
        });

    let (portal, _) = load_public_portal(&state, &slug).await?;
    let portal_id = portal.id.map(|id| id.to_hex()).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Portal id missing".into(),
    ))?;
    let identity = resolve_public_identity(
        &state,
        &portal.team_id.to_hex(),
        &headers,
        visitor_id.as_deref(),
    )
    .await?;

    // Verify session exists and belongs to a portal visitor
    let agent_svc = AgentService::new(state.db.clone());
    let session = agent_svc
        .get_session(&session_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into()))?
        .ok_or((StatusCode::NOT_FOUND, "Session not found".into()))?;

    validate_session_ownership(
        &session,
        &identity.user_id,
        &portal_id,
        &slug,
        identity.visitor_id.as_deref(),
    )?;

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
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                    tracing::warn!("SSE subscriber lagged, skipped {} events", n);
                    continue;
                }
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
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err((StatusCode::BAD_REQUEST, "Key must be [a-zA-Z0-9_-]".into()));
    }
    Ok(())
}

fn get_private_dir(portal: &Portal) -> Result<std::path::PathBuf, (StatusCode, String)> {
    let project_path = portal
        .project_path
        .as_deref()
        .ok_or((StatusCode::NOT_FOUND, "No project path".into()))?;
    Ok(std::path::Path::new(project_path).join("_private"))
}

/// GET /p/{slug}/api/data — list data keys
async fn list_data_keys(
    State(state): State<PortalPublicState>,
    Path(slug): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (portal, _) = load_public_portal(&state, &slug).await?;
    let dir = get_private_dir(&portal)?;
    let mut keys = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
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
    let (portal, _) = load_public_portal(&state, &slug).await?;
    let file = get_private_dir(&portal)?.join(format!("{}.json", key));
    let data = tokio::fs::read_to_string(&file)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Key not found".into()))?;
    let value: serde_json::Value =
        serde_json::from_str(&data).unwrap_or(serde_json::Value::String(data));
    Ok(Json(value))
}

/// PUT /p/{slug}/api/data/{key}
async fn set_data(
    State(state): State<PortalPublicState>,
    headers: HeaderMap,
    Path((slug, key)): Path<(String, String)>,
    Json(value): Json<serde_json::Value>,
) -> Result<StatusCode, (StatusCode, String)> {
    validate_data_key(&key)?;
    // H-2: Require visitor identity for writes
    let _visitor_id = headers
        .get("x-visitor-id")
        .and_then(|v| v.to_str().ok())
        .and_then(normalize_visitor_id)
        .ok_or((
            StatusCode::UNAUTHORIZED,
            "x-visitor-id header required".into(),
        ))?;
    // M-5: Check serialized size before any I/O
    let json = serde_json::to_string_pretty(&value)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if json.len() > 10 * 1024 * 1024 {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "Value exceeds 10MB limit".into(),
        ));
    }
    let (portal, _) = load_public_portal(&state, &slug).await?;
    let dir = get_private_dir(&portal)?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let file = dir.join(format!("{}.json", key));
    tokio::fs::write(&file, json)
        .await
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
    let (portal, _) = load_public_portal(&state, &slug).await?;
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
    let (portal, _) = load_public_portal(&state, &slug).await?;
    if !portal.bound_document_ids.iter().any(|id| id == &doc_id) {
        return Err((StatusCode::FORBIDDEN, "Document not bound".into()));
    }
    let doc_svc = DocumentService::new((*state.db).clone());
    let team_id = portal.team_id.to_hex();
    let (text, mime_type, total_size) = doc_svc
        .get_text_content_chunked(&team_id, &doc_id, None, None)
        .await
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
    let (portal, _) = load_public_portal(&state, &slug).await?;
    if !portal.bound_document_ids.iter().any(|id| id == &doc_id) {
        return Err((StatusCode::FORBIDDEN, "Document not bound".into()));
    }
    let doc_svc = DocumentService::new((*state.db).clone());
    let team_id = portal.team_id.to_hex();
    let meta = doc_svc
        .get_metadata(&team_id, &doc_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(serde_json::json!({
        "id": meta.id, "name": meta.name,
        "mime_type": meta.mime_type, "file_size": meta.file_size,
        "updated_at": meta.updated_at.to_rfc3339(),
    })))
}

/// GET /p/{slug}/api/user-docs — list anonymous visitor uploads plus related AI drafts
async fn list_visitor_user_documents(
    State(state): State<PortalPublicState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (portal, _) = load_public_portal(&state, &slug).await?;
    let identity =
        resolve_public_identity(&state, &portal.team_id.to_hex(), &headers, None).await?;
    let doc_svc = DocumentService::new((*state.db).clone());
    let payload = build_visitor_user_doc_payload(
        &doc_svc,
        &portal.team_id.to_hex(),
        &portal.slug,
        &identity.user_id,
    )
    .await?;
    Ok(Json(payload))
}

/// POST /p/{slug}/api/user-docs — upload an anonymous visitor document
#[allow(clippy::too_many_lines)]
async fn upload_visitor_user_document(
    State(state): State<PortalPublicState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    const MAX_VISITOR_UPLOAD_SIZE: usize = 25 * 1024 * 1024;
    let (portal, _) = load_public_portal(&state, &slug).await?;
    let identity =
        resolve_public_identity(&state, &portal.team_id.to_hex(), &headers, None).await?;
    let portal_id = portal.id.map(|id| id.to_hex()).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Portal id missing".into(),
    ))?;
    let team_id = portal.team_id.to_hex();
    let mut file_name = String::new();
    let mut file_data = Vec::new();
    let mut mime_type = "application/octet-stream".to_string();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    {
        if field.name().unwrap_or("") != "file" {
            continue;
        }
        file_name = field.file_name().unwrap_or("upload").to_string();
        if let Some(ct) = field.content_type() {
            mime_type = ct.to_string();
        }
        file_data = field
            .bytes()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
            .to_vec();
        break;
    }

    if file_data.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "No file".into()));
    }
    if mime_type == "application/octet-stream" || mime_type.trim().is_empty() {
        if let Some(corrected) = guess_mime_from_extension(&file_name) {
            mime_type = corrected.to_string();
        }
    }
    if !is_allowed_public_upload_mime(&mime_type) {
        return Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Unsupported file type".into(),
        ));
    }
    if file_data.len() > MAX_VISITOR_UPLOAD_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "File exceeds {}MB limit",
                MAX_VISITOR_UPLOAD_SIZE / 1024 / 1024
            ),
        ));
    }

    let folder_svc = FolderService::new((*state.db).clone());
    folder_svc
        .ensure_system_folder(&team_id, "用户上传文档", "/")
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    folder_svc
        .ensure_system_folder(&team_id, &portal.slug, portal_user_upload_root())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let doc_svc = DocumentService::new((*state.db).clone());
    let folder_path = portal_user_upload_folder(&portal.slug);
    let doc = doc_svc
        .upload(
            &team_id,
            &identity.user_id,
            &file_name,
            file_data,
            &mime_type,
            Some(folder_path),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let doc_id = doc.id.map(|id| id.to_hex()).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Document id missing".into(),
    ))?;

    let version_svc = DocumentVersionService::new((*state.db).clone());
    if let Ok((data, _, _)) = doc_svc.download(&team_id, &doc_id).await {
        let _ = version_svc
            .create_version(
                &doc_id,
                &team_id,
                &identity.user_id,
                &identity.user_id,
                data,
                "Initial public upload",
            )
            .await;
    }

    if let Some(agent_id) = PortalService::resolve_service_agent_id(&portal) {
        let agent_svc = AgentService::new(state.db.clone());
        if let Ok(Some(session)) = agent_svc
            .find_active_portal_session(&identity.user_id, &agent_id, &portal_id)
            .await
        {
            let _ = agent_svc
                .attach_documents_to_session(&session.session_id, &[doc_id.clone()])
                .await;
        }
    }

    Ok(Json(serde_json::json!({
        "document": {
            "id": doc_id,
            "name": doc.name,
            "mime_type": doc.mime_type,
            "file_size": doc.file_size,
            "folder_path": doc.folder_path,
            "updated_at": doc.updated_at.to_rfc3339(),
            "created_at": doc.created_at.to_rfc3339(),
        }
    })))
}

/// GET /p/{slug}/api/user-docs/{doc_id} — get uploaded or related AI document content
async fn get_visitor_user_document(
    State(state): State<PortalPublicState>,
    headers: HeaderMap,
    Path((slug, doc_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (portal, _) = load_public_portal(&state, &slug).await?;
    let identity =
        resolve_public_identity(&state, &portal.team_id.to_hex(), &headers, None).await?;
    let team_id = portal.team_id.to_hex();
    let doc_svc = DocumentService::new((*state.db).clone());
    let meta = resolve_visitor_accessible_document(
        &doc_svc,
        &team_id,
        &portal.slug,
        &identity.user_id,
        &doc_id,
    )
    .await?
    .ok_or((StatusCode::NOT_FOUND, "Document not found".into()))?;
    let (text, mime_type, total_size) = doc_svc
        .get_text_content_chunked(&team_id, &meta.id, None, None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({
        "id": meta.id,
        "name": meta.name,
        "mime_type": mime_type,
        "text": text,
        "total_size": total_size,
        "status": meta.status,
        "origin": meta.origin,
        "updated_at": meta.updated_at.to_rfc3339(),
    })))
}

/// GET /p/{slug}/api/user-docs/{doc_id}/meta — get uploaded or related AI document metadata
async fn get_visitor_user_document_meta(
    State(state): State<PortalPublicState>,
    headers: HeaderMap,
    Path((slug, doc_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (portal, _) = load_public_portal(&state, &slug).await?;
    let identity =
        resolve_public_identity(&state, &portal.team_id.to_hex(), &headers, None).await?;
    let team_id = portal.team_id.to_hex();
    let doc_svc = DocumentService::new((*state.db).clone());
    let meta = resolve_visitor_accessible_document(
        &doc_svc,
        &team_id,
        &portal.slug,
        &identity.user_id,
        &doc_id,
    )
    .await?
    .ok_or((StatusCode::NOT_FOUND, "Document not found".into()))?;
    Ok(Json(serde_json::json!({
        "id": meta.id,
        "name": meta.name,
        "display_name": meta.display_name,
        "mime_type": meta.mime_type,
        "file_size": meta.file_size,
        "folder_path": meta.folder_path,
        "origin": meta.origin,
        "status": meta.status,
        "category": meta.category,
        "source_document_ids": meta.source_document_ids,
        "source_session_id": meta.source_session_id,
        "source_mission_id": meta.source_mission_id,
        "lineage_description": meta.lineage_description,
        "updated_at": meta.updated_at.to_rfc3339(),
        "created_at": meta.created_at.to_rfc3339(),
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
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(req): Json<CancelRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let (portal, _) = load_public_portal(&state, &slug).await?;
    let portal_id = portal.id.map(|id| id.to_hex()).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Portal id missing".into(),
    ))?;
    let identity = resolve_public_identity(
        &state,
        &portal.team_id.to_hex(),
        &headers,
        Some(&req.visitor_id),
    )
    .await?;

    let agent_svc = AgentService::new(state.db.clone());
    let session = agent_svc
        .get_session(&req.session_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into()))?
        .ok_or((StatusCode::NOT_FOUND, "Session not found".into()))?;

    // H-1: Full validation matching send_visitor_message
    validate_session_ownership(
        &session,
        &identity.user_id,
        &portal_id,
        &slug,
        identity.visitor_id.as_deref(),
    )?;

    state.chat_manager.cancel(&req.session_id).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ListSessionsQuery {
    #[serde(default)]
    visitor_id: Option<String>,
}

/// GET /p/{slug}/api/chat/sessions — list visitor's chat sessions
async fn list_visitor_sessions(
    State(state): State<PortalPublicState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(q): Query<ListSessionsQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (portal, _) = load_public_portal(&state, &slug).await?;
    let identity = resolve_public_identity(
        &state,
        &portal.team_id.to_hex(),
        &headers,
        q.visitor_id.as_deref(),
    )
    .await?;

    let portal_id = portal.id.map(|id| id.to_hex()).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Portal id missing".into(),
    ))?;
    let agent_svc = AgentService::new(state.db.clone());
    // M-4: Filter sessions by portal_id to prevent cross-portal leakage
    let sessions = agent_svc
        .list_portal_sessions(&portal_id, &identity.user_id, 20)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<_> = sessions
        .into_iter()
        .map(|s| {
            serde_json::json!({
                "session_id": s.session_id,
                "title": s.title,
                "created_at": s.created_at,
                "last_message_at": s.last_message_at,
                "message_count": s.message_count,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "sessions": items })))
}
