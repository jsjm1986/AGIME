//! Shared runtime utilities for executor bridge pattern
//!
//! Both ChatExecutor and MissionExecutor use the same "bridge pattern"
//! to reuse TaskExecutor: create temp task → approve → register in
//! internal TaskManager → bridge events → execute → cleanup.
//!
//! This module extracts the shared logic to eliminate duplication.

use agime::prompt_template;
use agime_team::models::{BuiltinExtension, TaskStatus, TeamAgent};
use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use super::executor_mongo::TaskExecutor;
use super::mission_manager::MissionManager;
use super::mission_mongo::{ArtifactType, GoalStatus, MissionArtifactDoc, MissionDoc, StepStatus};
use super::service_mongo::AgentService;
use super::task_manager::{StreamEvent, TaskManager};
use uuid::Uuid;

/// Trait for broadcasting stream events to subscribers.
///
/// Both ChatManager and MissionManager implement this trait,
/// allowing shared runtime functions to work with either.
pub trait EventBroadcaster: Send + Sync + 'static {
    fn broadcast(
        &self,
        context_id: &str,
        event: StreamEvent,
    ) -> impl std::future::Future<Output = ()> + Send;
}

/// Create a temporary task record for TaskExecutor to consume.
pub async fn create_temp_task(
    agent_service: &AgentService,
    team_id: &str,
    agent_id: &str,
    user_id: &str,
    content: serde_json::Value,
) -> Result<agime_team::models::AgentTask> {
    use agime_team::models::{SubmitTaskRequest, TaskType};

    let req = SubmitTaskRequest {
        team_id: team_id.to_string(),
        agent_id: agent_id.to_string(),
        task_type: TaskType::Chat,
        content,
        priority: 0,
    };

    agent_service
        .submit_task(user_id, req)
        .await
        .map_err(|e| anyhow!("Failed to create temp task: {}", e))
}

/// Clean up a temporary task record from MongoDB.
pub async fn cleanup_temp_task(db: &MongoDb, task_id: &str) -> Result<()> {
    use mongodb::bson::doc;
    let collection: mongodb::Collection<mongodb::bson::Document> = db.collection("agent_tasks");
    collection
        .delete_one(doc! { "task_id": task_id }, None)
        .await
        .map_err(|e| anyhow!("Failed to cleanup temp task: {}", e))?;
    Ok(())
}

/// Ensure a mission has a usable execution session.
///
/// Resume paths must not fail just because the original session reference was
/// lost. If the stored mission session is missing or no longer exists, create a
/// fresh dedicated mission session and persist it before execution continues.
pub async fn ensure_mission_session(
    agent_service: &AgentService,
    mission_id: &str,
    mission: &MissionDoc,
    session_max_turns: Option<i32>,
    tool_timeout_seconds: Option<u64>,
    workspace_path: Option<&str>,
) -> Result<String> {
    if let Some(existing_session_id) = mission.session_id.as_deref() {
        match agent_service.get_session(existing_session_id).await {
            Ok(Some(_)) => {
                if let Some(path) = workspace_path {
                    agent_service
                        .set_session_workspace(existing_session_id, path)
                        .await
                        .map_err(|e| {
                            anyhow!(
                                "Failed to refresh workspace binding for mission {} session {}: {}",
                                mission_id,
                                existing_session_id,
                                e
                            )
                        })?;
                }
                return Ok(existing_session_id.to_string());
            }
            Ok(None) => {
                tracing::warn!(
                    "Mission {} references missing session {}; rebuilding dedicated mission session",
                    mission_id,
                    existing_session_id
                );
            }
            Err(e) => {
                return Err(anyhow!(
                    "Failed to load mission {} session {}: {}",
                    mission_id,
                    existing_session_id,
                    e
                ));
            }
        }
    } else {
        tracing::warn!(
            "Mission {} has no bound session; creating dedicated mission session for recovery",
            mission_id
        );
    }

    let session = agent_service
        .create_chat_session(
            &mission.team_id,
            &mission.agent_id,
            &mission.creator_id,
            mission.attached_document_ids.clone(),
            None,
            None,
            None,
            None,
            session_max_turns,
            tool_timeout_seconds,
            None,
            false,
            false,
            None,
            Some("mission".to_string()),
            Some(mission_id.to_string()),
            Some(true),
        )
        .await
        .map_err(|e| anyhow!("Failed to create recovery session for mission {}: {}", mission_id, e))?;

    let session_id = session.session_id.clone();
    agent_service
        .set_mission_session(mission_id, &session_id)
        .await
        .map_err(|e| anyhow!("Failed to bind recovery session for mission {}: {}", mission_id, e))?;

    if let Some(path) = workspace_path {
        agent_service
            .set_session_workspace(&session_id, path)
            .await
            .map_err(|e| {
                anyhow!(
                    "Failed to bind workspace {} to recovery session {} for mission {}: {}",
                    path,
                    session_id,
                    mission_id,
                    e
                )
            })?;
    }

    Ok(session_id)
}

/// Bridge events from internal TaskManager to an EventBroadcaster.
///
/// Forwards all events except Done (which the outer executor sends itself).
pub async fn bridge_events<B: EventBroadcaster>(
    task_mgr: &TaskManager,
    task_id: &str,
    broadcaster: &B,
    context_id: &str,
) {
    let Some(mut rx) = task_mgr.subscribe(task_id).await else {
        return;
    };

    loop {
        match rx.recv().await {
            Ok(event) => {
                let is_done = matches!(&event, StreamEvent::Done { .. });
                if is_done {
                    break;
                }
                broadcaster.broadcast(context_id, event).await;
            }
            Err(broadcast::error::RecvError::Closed) => break,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("Bridge lagged {} events", n);
            }
        }
    }
}

/// Execute a user message via the bridge pattern.
///
/// This is the core shared flow:
/// 1. Load session to get team_id/user_id
/// 2. Create temp task
/// 3. Approve temp task
/// 4. Register in internal TaskManager
/// 5. Bridge events to broadcaster
/// 6. Link cancellation tokens
/// 7. Execute via TaskExecutor
/// 8. Wait for bridge, cleanup
#[allow(clippy::too_many_arguments)]
pub async fn execute_via_bridge<B: EventBroadcaster>(
    db: &Arc<MongoDb>,
    agent_service: &AgentService,
    internal_task_manager: &Arc<TaskManager>,
    broadcaster: &Arc<B>,
    context_id: &str,
    agent_id: &str,
    session_id: &str,
    user_message: &str,
    cancel_token: CancellationToken,
    workspace_path: Option<&str>,
    mission_id: Option<&str>,
    llm_overrides: Option<serde_json::Value>,
    mission_context: Option<serde_json::Value>,
    mission_manager: Option<Arc<MissionManager>>,
) -> Result<()> {
    execute_via_bridge_with_turn_instruction(
        db,
        agent_service,
        internal_task_manager,
        broadcaster,
        context_id,
        agent_id,
        session_id,
        user_message,
        cancel_token,
        workspace_path,
        mission_id,
        llm_overrides,
        mission_context,
        None,
        mission_manager,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_via_bridge_with_turn_instruction<B: EventBroadcaster>(
    db: &Arc<MongoDb>,
    agent_service: &AgentService,
    internal_task_manager: &Arc<TaskManager>,
    broadcaster: &Arc<B>,
    context_id: &str,
    agent_id: &str,
    session_id: &str,
    user_message: &str,
    cancel_token: CancellationToken,
    workspace_path: Option<&str>,
    mission_id: Option<&str>,
    llm_overrides: Option<serde_json::Value>,
    mission_context: Option<serde_json::Value>,
    turn_system_instruction: Option<&str>,
    mission_manager: Option<Arc<MissionManager>>,
) -> Result<()> {
    // Load session to get team_id and user_id
    let session = agent_service
        .get_session(session_id)
        .await
        .map_err(|e| anyhow!("DB error: {}", e))?
        .ok_or_else(|| anyhow!("Session not found"))?;

    // Build task content
    let mut content = serde_json::json!({
        "messages": [{"role": "user", "content": user_message}],
        "session_id": session_id,
    });
    if let Some(wp) = workspace_path {
        content["workspace_path"] = serde_json::Value::String(wp.to_string());
    }
    if let Some(mid) = mission_id {
        content["mission_id"] = serde_json::Value::String(mid.to_string());
    }
    if let Some(overrides) = llm_overrides {
        content["llm_overrides"] = overrides;
    }
    if let Some(mc) = mission_context {
        content["mission_context"] = mc;
    }
    if let Some(turn_instruction) = turn_system_instruction
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        content["turn_system_instruction"] =
            serde_json::Value::String(turn_instruction.to_string());
    }

    // Create temp task
    let task = create_temp_task(
        agent_service,
        &session.team_id,
        agent_id,
        &session.user_id,
        content,
    )
    .await?;
    let task_id = task.id.clone();

    // Approve temp task
    if let Err(e) = agent_service.approve_task(&task_id, &session.user_id).await {
        let _ = cleanup_temp_task(db, &task_id).await;
        return Err(anyhow!("Failed to approve: {}", e));
    }

    // Register in internal task manager
    let (internal_cancel, _) = internal_task_manager.register(&task_id).await;

    // Bridge events
    let bridge_context_id = context_id.to_string();
    let bridge_broadcaster = broadcaster.clone();
    let bridge_task_mgr = internal_task_manager.clone();
    let bridge_task_id = task_id.clone();

    let bridge_handle = tokio::spawn(async move {
        bridge_events(
            &bridge_task_mgr,
            &bridge_task_id,
            bridge_broadcaster.as_ref(),
            &bridge_context_id,
        )
        .await;
    });

    // Link cancellation tokens
    let linked = internal_cancel.clone();
    let external = cancel_token.clone();
    tokio::spawn(async move {
        external.cancelled().await;
        linked.cancel();
    });

    // Execute via TaskExecutor
    let executor = TaskExecutor::new_with_mission_manager(
        db.clone(),
        internal_task_manager.clone(),
        mission_manager,
    );
    let mut exec_result = executor.execute_task(&task_id, internal_cancel).await;

    // Align bridge return value with persisted task status.
    // TaskExecutor can consume internal errors and still return Ok(()),
    // while marking task status as failed/cancelled in DB.
    if exec_result.is_ok() {
        match agent_service.get_task(&task_id).await {
            Ok(Some(task)) => match task.status {
                TaskStatus::Completed => {}
                TaskStatus::Failed => {
                    let msg = task
                        .error_message
                        .unwrap_or_else(|| "Task execution failed".to_string());
                    exec_result = Err(anyhow!("Task {} failed: {}", task_id, msg));
                }
                TaskStatus::Cancelled => {
                    exec_result = Err(anyhow!("Task {} was cancelled", task_id));
                }
                other => {
                    exec_result = Err(anyhow!(
                        "Task {} ended in unexpected status: {}",
                        task_id,
                        other
                    ));
                }
            },
            Ok(None) => {
                tracing::debug!(
                    "Temp task {} not found after execution (possibly cleaned concurrently)",
                    task_id
                );
            }
            Err(e) => {
                tracing::warn!("Failed to verify temp task {} status: {}", task_id, e);
            }
        }
    }

    // Wait for bridge to finish
    let _ = bridge_handle.await;

    // Cleanup temp task
    let _ = cleanup_temp_task(db, &task_id).await;

    exec_result
}

// ─── Shared Helpers ─────────────────────────────────────

/// Extract JSON from LLM output with multiple fallback strategies:
/// 1. ```json ... ``` code block
/// 2. ``` ... ``` code block
/// 3. First `[` to last `]` (array)
/// 4. First `{` to last `}` (object)
/// 5. Raw text as-is
pub fn extract_json_block(text: &str) -> String {
    // Strategy 1: ```json block
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    // Strategy 2: ``` block
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(end) = after.find("```") {
            let inner = after[..end].trim();
            if inner.starts_with('[') || inner.starts_with('{') {
                return inner.to_string();
            }
        }
    }
    // Strategy 3: first [ to last ]
    if let (Some(start), Some(end)) = (text.find('['), text.rfind(']')) {
        if start < end {
            return text[start..=end].to_string();
        }
    }
    // Strategy 4: first { to last }
    if let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) {
        if start < end {
            return text[start..=end].to_string();
        }
    }
    text.trim().to_string()
}

fn strip_trailing_json_commas(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        if ch == ',' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                i += 1;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

/// Normalize common non-strict JSON artifacts from LLM output.
///
/// This keeps transformation conservative:
/// - smart quotes / full-width punctuation to ASCII JSON symbols
/// - strip leading `json` language hint line
/// - remove trailing commas before `}` / `]`
pub fn normalize_loose_json(input: &str) -> String {
    let trimmed = input.trim();
    let s = trimmed.strip_prefix("json\n").unwrap_or(trimmed);
    let s = s
        .replace(['“', '”'], "\"")
        .replace(['\u{2018}', '\u{2019}'], "'")
        .replace('：', ":")
        .replace('，', ",");
    strip_trailing_json_commas(&s)
}

/// Parse the first valid JSON value from LLM output, tolerating trailing text.
///
/// This is intentionally more permissive than `serde_json::from_str` because
/// model outputs often include a valid JSON object followed by explanation or
/// markdown. The first parseable JSON value is treated as the payload.
pub fn parse_first_json_value(text: &str) -> Result<serde_json::Value> {
    let json_str = extract_json_block(text);
    let normalized = normalize_loose_json(&json_str);
    let candidates: [&str; 2] = [&json_str, &normalized];
    let mut last_err = None;

    for candidate in candidates {
        let mut stream =
            serde_json::Deserializer::from_str(candidate).into_iter::<serde_json::Value>();
        match stream.next() {
            Some(Ok(parsed)) => return Ok(parsed),
            Some(Err(err)) => last_err = Some(err.to_string()),
            None => last_err = Some("empty json payload".to_string()),
        }
    }

    Err(anyhow!(
        "Failed to parse JSON payload: {}",
        last_err.unwrap_or_else(|| "unknown error".to_string())
    ))
}

/// Extract text content from the last assistant message in a messages JSON array.
pub fn extract_last_assistant_text(messages_json: &str) -> Option<String> {
    let msgs: Vec<serde_json::Value> = serde_json::from_str(messages_json).ok()?;
    for msg in msgs.iter().rev() {
        if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }
        let content = msg.get("content")?;
        if let Some(s) = content.as_str() {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
        if let Some(arr) = content.as_array() {
            for item in arr {
                if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                    if !t.is_empty() {
                        return Some(t.to_string());
                    }
                }
            }
        }
    }
    None
}

fn extract_tool_name_from_tool_call(call: &serde_json::Value) -> Option<String> {
    if let Some(name) = call.get("name").and_then(|v| v.as_str()) {
        return Some(name.to_string());
    }
    call.get("value")
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn extract_tool_response_success(tool_result: &serde_json::Value) -> Option<bool> {
    if let Some(status) = tool_result.get("status").and_then(|v| v.as_str()) {
        return Some(status.eq_ignore_ascii_case("success"));
    }
    if tool_result.get("error").is_some() {
        return Some(false);
    }
    if tool_result.get("value").is_some() {
        return Some(true);
    }
    None
}

#[derive(Debug, Clone)]
struct ParsedToolCall {
    name: String,
    success: bool,
    resolved: bool,
}

#[derive(Debug, Clone, Default)]
pub struct MissionPreflightContract {
    pub required_artifacts: Vec<String>,
    pub completion_checks: Vec<String>,
    pub no_artifact_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractVerifyGateMode {
    Off,
    Soft,
    Hard,
}

pub fn contract_verify_gate_mode() -> ContractVerifyGateMode {
    let raw = std::env::var("TEAM_MISSION_CONTRACT_VERIFY_GATE")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "soft".to_string());
    match raw.as_str() {
        "off" | "disabled" | "none" => ContractVerifyGateMode::Off,
        "hard" | "strict" | "required" => ContractVerifyGateMode::Hard,
        _ => ContractVerifyGateMode::Soft,
    }
}

pub fn contract_verify_gate_mode_label(mode: ContractVerifyGateMode) -> &'static str {
    match mode {
        ContractVerifyGateMode::Off => "off",
        ContractVerifyGateMode::Soft => "soft",
        ContractVerifyGateMode::Hard => "hard",
    }
}

fn parse_string_list_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Vec<String> {
    obj.get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn parse_no_artifact_reason(obj: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    obj.get("no_artifact_reason")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn parse_preflight_contract_from_request_args(
    args: &serde_json::Value,
) -> MissionPreflightContract {
    let Some(obj) = args.as_object() else {
        return MissionPreflightContract::default();
    };
    MissionPreflightContract {
        required_artifacts: parse_string_list_field(obj, "required_artifacts"),
        completion_checks: parse_string_list_field(obj, "completion_checks"),
        no_artifact_reason: parse_no_artifact_reason(obj),
    }
}

fn parse_preflight_contract_from_response_text(text: &str) -> Option<MissionPreflightContract> {
    let val: serde_json::Value = serde_json::from_str(text).ok()?;
    let contract = val.get("contract")?.as_object()?;
    Some(MissionPreflightContract {
        required_artifacts: parse_string_list_field(contract, "required_artifacts"),
        completion_checks: parse_string_list_field(contract, "completion_checks"),
        no_artifact_reason: parse_no_artifact_reason(contract),
    })
}

fn parse_preflight_contract_from_tool_response(
    tool_result: &serde_json::Value,
) -> Option<MissionPreflightContract> {
    let content = tool_result
        .get("value")
        .and_then(|v| v.get("content"))
        .and_then(|v| v.as_array())?;

    for block in content {
        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
            if let Some(contract) = parse_preflight_contract_from_response_text(text) {
                return Some(contract);
            }
        }
    }
    None
}

fn parse_verify_contract_status_from_response_text(text: &str) -> Option<bool> {
    let val: serde_json::Value = serde_json::from_str(text).ok()?;
    if let Some(status) = val.get("status").and_then(|v| v.as_str()) {
        if status.eq_ignore_ascii_case("pass") || status.eq_ignore_ascii_case("ok") {
            return Some(true);
        }
        if status.eq_ignore_ascii_case("fail") || status.eq_ignore_ascii_case("error") {
            return Some(false);
        }
    }
    if let Some(pass) = val.get("pass").and_then(|v| v.as_bool()) {
        return Some(pass);
    }
    None
}

fn parse_verify_contract_status_from_tool_response(
    tool_result: &serde_json::Value,
) -> Option<bool> {
    let content = tool_result
        .get("value")
        .and_then(|v| v.get("content"))
        .and_then(|v| v.as_array())?;

    for block in content {
        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
            if let Some(status) = parse_verify_contract_status_from_response_text(text) {
                return Some(status);
            }
        }
    }
    None
}

/// Extract the latest successful `mission_preflight__preflight` contract since `start_message_index`.
///
/// Contract source priority:
/// 1) ToolRequest arguments (paired with successful response by request id)
/// 2) ToolResponse textual payload (fallback when pairing fails)
pub fn extract_latest_preflight_contract_since(
    messages_json: &str,
    start_message_index: usize,
    preflight_tool_name: &str,
) -> Option<MissionPreflightContract> {
    let msgs: Vec<serde_json::Value> = serde_json::from_str(messages_json).ok()?;
    let mut pending: HashMap<String, MissionPreflightContract> = HashMap::new();
    let mut latest: Option<MissionPreflightContract> = None;

    for msg in msgs.iter().skip(start_message_index) {
        let content = match msg.get("content").and_then(|c| c.as_array()) {
            Some(arr) => arr,
            None => continue,
        };

        for item in content {
            let item_type = item
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or_default();
            match item_type {
                "toolRequest" | "frontendToolRequest" => {
                    let id = match item.get("id").and_then(|v| v.as_str()) {
                        Some(v) if !v.is_empty() => v.to_string(),
                        _ => continue,
                    };
                    let Some(call) = item.get("toolCall") else {
                        continue;
                    };
                    let name = extract_tool_name_from_tool_call(call).unwrap_or_default();
                    if !name.eq_ignore_ascii_case(preflight_tool_name) {
                        continue;
                    }
                    let args = call
                        .get("value")
                        .and_then(|v| v.get("arguments"))
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}));
                    pending.insert(id, parse_preflight_contract_from_request_args(&args));
                }
                "tool_use" => {
                    let id = match item.get("id").and_then(|v| v.as_str()) {
                        Some(v) if !v.is_empty() => v.to_string(),
                        _ => continue,
                    };
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    if !name.eq_ignore_ascii_case(preflight_tool_name) {
                        continue;
                    }
                    let args = item
                        .get("input")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}));
                    pending.insert(id, parse_preflight_contract_from_request_args(&args));
                }
                "toolResponse" => {
                    let success = item
                        .get("toolResult")
                        .and_then(extract_tool_response_success)
                        .unwrap_or(false);
                    if !success {
                        continue;
                    }
                    let id = item.get("id").and_then(|v| v.as_str()).unwrap_or_default();
                    if let Some(contract) = pending.get(id).cloned() {
                        latest = Some(contract);
                        continue;
                    }
                    if let Some(tool_result) = item.get("toolResult") {
                        if let Some(contract) =
                            parse_preflight_contract_from_tool_response(tool_result)
                        {
                            latest = Some(contract);
                        }
                    }
                }
                "tool_result" => {
                    let success = !item
                        .get("is_error")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if !success {
                        continue;
                    }
                    let id = item
                        .get("tool_use_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    if let Some(contract) = pending.get(id).cloned() {
                        latest = Some(contract);
                        continue;
                    }
                    if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
                        for block in content {
                            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                if let Some(contract) =
                                    parse_preflight_contract_from_response_text(text)
                                {
                                    latest = Some(contract);
                                    break;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    latest
}

/// Extract the latest successful `mission_preflight__verify_contract` status since `start_message_index`.
///
/// Returns:
/// - `Some(true)` when tool returned status pass
/// - `Some(false)` when tool returned status fail
/// - `None` when no parseable verification status is found
pub fn extract_latest_verify_contract_status_since(
    messages_json: &str,
    start_message_index: usize,
    verify_tool_name: &str,
) -> Option<bool> {
    let msgs: Vec<serde_json::Value> = serde_json::from_str(messages_json).ok()?;
    let mut pending: HashSet<String> = HashSet::new();
    let mut latest: Option<bool> = None;

    for msg in msgs.iter().skip(start_message_index) {
        let content = match msg.get("content").and_then(|c| c.as_array()) {
            Some(arr) => arr,
            None => continue,
        };

        for item in content {
            let item_type = item
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or_default();
            match item_type {
                "toolRequest" | "frontendToolRequest" => {
                    let id = match item.get("id").and_then(|v| v.as_str()) {
                        Some(v) if !v.is_empty() => v.to_string(),
                        _ => continue,
                    };
                    let Some(call) = item.get("toolCall") else {
                        continue;
                    };
                    let name = extract_tool_name_from_tool_call(call).unwrap_or_default();
                    if name.eq_ignore_ascii_case(verify_tool_name) {
                        pending.insert(id);
                    }
                }
                "tool_use" => {
                    let id = match item.get("id").and_then(|v| v.as_str()) {
                        Some(v) if !v.is_empty() => v.to_string(),
                        _ => continue,
                    };
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    if name.eq_ignore_ascii_case(verify_tool_name) {
                        pending.insert(id);
                    }
                }
                "toolResponse" => {
                    let id = item.get("id").and_then(|v| v.as_str()).unwrap_or_default();
                    if !pending.contains(id) {
                        continue;
                    }
                    let success = item
                        .get("toolResult")
                        .and_then(extract_tool_response_success)
                        .unwrap_or(false);
                    if !success {
                        continue;
                    }
                    if let Some(tool_result) = item.get("toolResult") {
                        if let Some(status) =
                            parse_verify_contract_status_from_tool_response(tool_result)
                        {
                            latest = Some(status);
                        }
                    }
                }
                "tool_result" => {
                    let id = item
                        .get("tool_use_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    if !pending.contains(id) {
                        continue;
                    }
                    let success = !item
                        .get("is_error")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if !success {
                        continue;
                    }
                    if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
                        for block in content {
                            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                if let Some(status) =
                                    parse_verify_contract_status_from_response_text(text)
                                {
                                    latest = Some(status);
                                    break;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    latest
}

/// Extract tool call records from session messages JSON.
///
/// Supports both legacy (`tool_use`/`tool_result`) and current
/// (`toolRequest`/`toolResponse`) message content schemas.
pub fn extract_tool_calls_since(
    messages_json: &str,
    start_message_index: usize,
) -> Vec<(String, bool)> {
    let msgs: Vec<serde_json::Value> = match serde_json::from_str(messages_json) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let mut calls: Vec<ParsedToolCall> = Vec::new();
    let mut id_to_index: HashMap<String, usize> = HashMap::new();

    for msg in msgs.iter().skip(start_message_index) {
        let content = match msg.get("content").and_then(|c| c.as_array()) {
            Some(arr) => arr,
            None => continue,
        };

        for item in content {
            let item_type = item
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or_default();
            match item_type {
                // Legacy shape
                "tool_use" => {
                    let id = item
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let name = item
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let idx = calls.len();
                    calls.push(ParsedToolCall {
                        name,
                        success: false,
                        resolved: false,
                    });
                    if let Some(id) = id {
                        id_to_index.insert(id, idx);
                    }
                }
                // Current serialized schema from MessageContent::ToolRequest
                "toolRequest" | "frontendToolRequest" => {
                    let id = item
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let name = item
                        .get("toolCall")
                        .and_then(extract_tool_name_from_tool_call)
                        .unwrap_or_else(|| "unknown".to_string());
                    let idx = calls.len();
                    calls.push(ParsedToolCall {
                        name,
                        success: false,
                        resolved: false,
                    });
                    if let Some(id) = id {
                        id_to_index.insert(id, idx);
                    }
                }
                "tool_result" => {
                    let id = item
                        .get("tool_use_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let success = !item
                        .get("is_error")
                        .and_then(|e| e.as_bool())
                        .unwrap_or(false);
                    if let Some(id) = id {
                        if let Some(idx) = id_to_index.get(&id).copied() {
                            if let Some(call) = calls.get_mut(idx) {
                                call.success = success;
                                call.resolved = true;
                            }
                            continue;
                        }
                    }
                    if let Some(call) = calls.iter_mut().rev().find(|c| !c.resolved) {
                        call.success = success;
                        call.resolved = true;
                    }
                }
                "toolResponse" => {
                    let id = item
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let success = item
                        .get("toolResult")
                        .and_then(extract_tool_response_success)
                        .unwrap_or(false);
                    if let Some(id) = id {
                        if let Some(idx) = id_to_index.get(&id).copied() {
                            if let Some(call) = calls.get_mut(idx) {
                                call.success = success;
                                call.resolved = true;
                            }
                            continue;
                        }
                    }
                    if let Some(call) = calls.iter_mut().rev().find(|c| !c.resolved) {
                        call.success = success;
                        call.resolved = true;
                    }
                }
                _ => {}
            }
        }
    }

    calls.into_iter().map(|c| (c.name, c.success)).collect()
}

/// Extract tool call records from the full session message list.
pub fn extract_tool_calls(messages_json: &str) -> Vec<(String, bool)> {
    extract_tool_calls_since(messages_json, 0)
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionGuardSignals {
    pub max_turn_limit_warning: bool,
    pub external_output_paths: Vec<String>,
}

fn normalize_windows_absolute_path_token(token: &str) -> Option<String> {
    let trimmed = token
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | ',' | ';' | ')' | ']' | '}' | '>'))
        .trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    if !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' {
        return None;
    }
    if bytes[2] != b'/' && bytes[2] != b'\\' {
        return None;
    }
    Some(trimmed.replace('\\', "/"))
}

#[cfg(not(windows))]
fn normalize_unix_absolute_path_token(token: &str) -> Option<String> {
    let trimmed = token
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | ',' | ';' | ')' | ']' | '}' | '>'))
        .trim();
    if !trimmed.starts_with('/') || trimmed.starts_with("//") {
        return None;
    }

    let mut prefix = String::new();
    let mut cut_index = trimmed.len();
    for (idx, ch) in trimmed.char_indices() {
        let is_path_char = ch == '/'
            || ch == '.'
            || ch == '_'
            || ch == '-'
            || ch == '+'
            || ch == '@'
            || ch == '~'
            || ch.is_alphanumeric();
        if is_path_char {
            prefix.push(ch);
            continue;
        }
        cut_index = idx;
        break;
    }

    let candidate = prefix
        .trim_end_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '`' | ',' | ';' | ':' | ')' | ']' | '}' | '>' | '.'
            )
        })
        .trim();
    if candidate.is_empty() || !candidate.starts_with('/') || candidate == "/" {
        return None;
    }
    if candidate.split('/').any(|segment| segment == "...") {
        return None;
    }

    let remainder = trimmed.get(cut_index..).unwrap_or("").trim();
    if !remainder.is_empty() && remainder.chars().any(char::is_alphanumeric) {
        return None;
    }

    Some(candidate.to_string())
}

#[cfg(not(windows))]
fn looks_like_unix_filesystem_path(path: &str) -> bool {
    let first_segment = path
        .trim_start_matches('/')
        .split('/')
        .find(|segment| !segment.is_empty());
    matches!(
        first_segment,
        Some(
            "bin"
                | "boot"
                | "dev"
                | "etc"
                | "home"
                | "lib"
                | "lib64"
                | "media"
                | "mnt"
                | "opt"
                | "proc"
                | "root"
                | "run"
                | "sbin"
                | "srv"
                | "sys"
                | "tmp"
                | "usr"
                | "var"
                | "workspace"
                | "workspaces"
        )
    )
}

fn normalized_path_prefix(path: &str) -> String {
    #[cfg(windows)]
    {
        path.replace('\\', "/")
            .trim_end_matches('/')
            .to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.replace('\\', "/").trim_end_matches('/').to_string()
    }
}

fn is_path_under_workspace(path: &str, workspace_path: &str) -> bool {
    let candidate = normalized_path_prefix(path);
    let workspace = normalized_path_prefix(workspace_path);
    candidate == workspace || candidate.starts_with(&(workspace + "/"))
}

fn extract_text_fragments_from_message_item(item: &serde_json::Value, out: &mut Vec<String>) {
    if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
        if !text.is_empty() {
            out.push(text.to_string());
        }
    }
    if let Some(value) = item.get("value") {
        if let Some(text) = value.get("text").and_then(|v| v.as_str()) {
            if !text.is_empty() {
                out.push(text.to_string());
            }
        }
        if let Some(content) = value.get("content").and_then(|v| v.as_array()) {
            for block in content {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        out.push(text.to_string());
                    }
                }
            }
        }
    }
    if let Some(tool_result) = item.get("toolResult") {
        if let Some(content) = tool_result
            .get("value")
            .and_then(|v| v.get("content"))
            .and_then(|v| v.as_array())
        {
            for block in content {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        out.push(text.to_string());
                    }
                }
            }
        }
    }
    if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
        for block in content {
            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                if !text.is_empty() {
                    out.push(text.to_string());
                }
            }
        }
    }
}

fn collect_text_fragments_since(messages_json: &str, start_message_index: usize) -> Vec<String> {
    let msgs: Vec<serde_json::Value> = match serde_json::from_str(messages_json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut texts = Vec::new();
    for msg in msgs.iter().skip(start_message_index) {
        if let Some(content_str) = msg.get("content").and_then(|c| c.as_str()) {
            if !content_str.is_empty() {
                texts.push(content_str.to_string());
            }
            continue;
        }
        let Some(content) = msg.get("content").and_then(|c| c.as_array()) else {
            continue;
        };
        for item in content {
            extract_text_fragments_from_message_item(item, &mut texts);
        }
    }
    texts
}

fn collect_tool_text_fragments_since(
    messages_json: &str,
    start_message_index: usize,
) -> Vec<String> {
    let msgs: Vec<serde_json::Value> = match serde_json::from_str(messages_json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut texts = Vec::new();
    for msg in msgs.iter().skip(start_message_index) {
        if msg.get("role").and_then(|r| r.as_str()) == Some("tool") {
            if let Some(content_str) = msg.get("content").and_then(|c| c.as_str()) {
                if !content_str.is_empty() {
                    texts.push(content_str.to_string());
                }
            }
            if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                for item in content {
                    extract_text_fragments_from_message_item(item, &mut texts);
                }
            }
            continue;
        }

        let Some(content) = msg.get("content").and_then(|c| c.as_array()) else {
            continue;
        };

        for item in content {
            if item.get("toolResult").is_some()
                || item.get("type").and_then(|t| t.as_str()) == Some("tool_result")
                || item.get("type").and_then(|t| t.as_str()) == Some("toolResponse")
            {
                extract_text_fragments_from_message_item(item, &mut texts);
            }
        }
    }
    texts
}

fn text_has_max_turn_limit_warning(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("maximum turn limit")
        || lower.contains("max turn limit")
        || lower.contains("轮次上限")
        || lower.contains("达到最大轮次")
}

fn extract_absolute_paths_from_text(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split_whitespace() {
        if let Some(path) = normalize_windows_absolute_path_token(raw) {
            out.push(path);
            continue;
        }
        #[cfg(not(windows))]
        if let Some(path) = normalize_unix_absolute_path_token(raw) {
            out.push(path);
        }
    }
    out
}

/// Inspect execution transcript slices and return guard signals that should
/// block step completion (for retry/failure handling by executors).
pub fn collect_execution_guard_signals_since(
    messages_json: &str,
    start_message_index: usize,
    workspace_path: Option<&str>,
) -> ExecutionGuardSignals {
    let all_texts = collect_text_fragments_since(messages_json, start_message_index);
    if all_texts.is_empty() {
        return ExecutionGuardSignals::default();
    }

    let max_turn_limit_warning = all_texts.iter().any(|t| text_has_max_turn_limit_warning(t));

    let mut external_output_paths = Vec::new();
    if let Some(workspace) = workspace_path {
        for text in &collect_tool_text_fragments_since(messages_json, start_message_index) {
            for path in extract_absolute_paths_from_text(text) {
                #[cfg(not(windows))]
                if !looks_like_unix_filesystem_path(&path) {
                    continue;
                }
                if !is_path_under_workspace(&path, workspace)
                    && !external_output_paths.iter().any(|p| p == &path)
                {
                    external_output_paths.push(path);
                }
            }
        }
    }

    ExecutionGuardSignals {
        max_turn_limit_warning,
        external_output_paths,
    }
}

/// Count message objects in serialized session `messages_json`.
pub fn count_session_messages(messages_json: &str) -> usize {
    serde_json::from_str::<Vec<serde_json::Value>>(messages_json)
        .map(|v| v.len())
        .unwrap_or(0)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RetryPlaybookToolCall {
    pub name: String,
    pub success: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RetryPlaybookContext {
    pub mode_label: String,
    pub unit_title: String,
    pub attempt_number: u32,
    pub max_attempts: u32,
    pub failure_message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_output: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_tool_calls: Vec<RetryPlaybookToolCall>,
}

/// Keep only a bounded tail of tool calls for retry prompt context.
pub fn recent_tool_calls_for_retry(
    messages_json: &str,
    limit: usize,
) -> Vec<RetryPlaybookToolCall> {
    let all = extract_tool_calls(messages_json);
    let start = all.len().saturating_sub(limit);
    all.into_iter()
        .skip(start)
        .map(|(name, success)| RetryPlaybookToolCall { name, success })
        .collect()
}

/// Extract the latest assistant output and truncate it for retry prompt context.
pub fn latest_assistant_output_for_retry(messages_json: &str, max_chars: usize) -> Option<String> {
    let text = extract_last_assistant_text(messages_json)?;
    if text.chars().count() > max_chars {
        let truncated: String = text.chars().take(max_chars.saturating_sub(3)).collect();
        Some(format!("{}...", truncated))
    } else {
        Some(text)
    }
}

/// Render prompt-driven recovery playbook for retry attempts.
pub fn render_retry_playbook(ctx: &RetryPlaybookContext) -> String {
    match prompt_template::render_global_file("mission_retry.md", ctx) {
        Ok(rendered) => rendered,
        Err(e) => {
            tracing::warn!("Failed to render mission_retry.md template: {}", e);
            format!(
                "## Retry Recovery\n\
Current {}: {}\n\
Attempt: {}/{}\n\
Previous failure: {}\n\
- Diagnose root cause first\n\
- Do not repeat the same failing action unchanged\n\
- Verify paths/files before processing when file-related\n\
- Execute a different fix strategy and verify result",
                ctx.mode_label,
                ctx.unit_title,
                ctx.attempt_number,
                ctx.max_attempts,
                ctx.failure_message
            )
        }
    }
}

// ─── Workspace Directory Helpers ────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFileFingerprint {
    pub size: u64,
    pub modified_ms: u128,
}

pub type WorkspaceSnapshot = HashMap<String, WorkspaceFileFingerprint>;

#[derive(Debug, Clone)]
pub struct ScannedWorkspaceArtifact {
    pub name: String,
    pub relative_path: String,
    pub artifact_type: ArtifactType,
    pub content: Option<String>,
    pub mime_type: Option<String>,
    pub size: i64,
}

// Allow scanning nested output directories like output/reports/final.xxx
// while keeping traversal bounded for performance.
const DEFAULT_SCAN_MAX_DEPTH: usize = 6;
const DEFAULT_INLINE_TEXT_LIMIT: u64 = 50 * 1024;

/// Validate that a path segment is safe (no traversal, separators, or special chars).
fn validate_path_segment(segment: &str, label: &str) -> Result<()> {
    if segment.is_empty() {
        return Err(anyhow!("{} must not be empty", label));
    }
    if segment == "."
        || segment.contains("..")
        || segment.contains('/')
        || segment.contains('\\')
        || segment.contains('\0')
    {
        return Err(anyhow!(
            "{} contains invalid characters: {:?}",
            label,
            segment
        ));
    }
    Ok(())
}

/// Create a workspace directory with path validation.
///
/// `segments` are joined under `root` to form the final path.
/// Each segment is validated to prevent path traversal attacks.
pub fn create_workspace_dir(root: &str, segments: &[(&str, &str)]) -> Result<String> {
    let mut path = PathBuf::from(root);
    for (segment, label) in segments {
        validate_path_segment(segment, label)?;
        path.push(segment);
    }
    std::fs::create_dir_all(&path)
        .map_err(|e| anyhow!("Failed to create workspace dir {:?}: {}", path, e))?;
    Ok(path.to_string_lossy().to_string())
}

/// Remove a workspace directory.
///
/// Returns `Ok(true)` if removed, `Ok(false)` if path was empty/missing,
/// `Err` if `remove_dir_all` fails (callers should treat as non-fatal).
pub fn cleanup_workspace_dir(workspace_path: Option<&str>) -> Result<bool> {
    let path = match workspace_path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => return Ok(false),
    };
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&path)
        .map_err(|e| anyhow!("Failed to remove workspace dir {:?}: {}", path, e))?;
    Ok(true)
}

fn is_hidden_or_temp_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.starts_with('.')
        || name.starts_with("~$")
        || name.ends_with('~')
        || lower.ends_with(".tmp")
        || lower.ends_with(".temp")
        || lower.ends_with(".swp")
        || lower.ends_with(".swo")
        || lower == ".ds_store"
}

fn is_workspace_noise_dir(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "node_modules"
            | ".pnpm-store"
            | ".yarn"
            | ".npm"
            | ".next"
            | ".nuxt"
            | ".svelte-kit"
            | ".turbo"
            | ".cache"
            | "__pycache__"
            | ".pytest_cache"
            | ".mypy_cache"
            | ".ruff_cache"
            | ".venv"
            | "venv"
            | "env"
            | "dist"
            | "build"
            | "target"
            | "coverage"
            | ".idea"
            | ".vscode"
    )
}

fn collect_workspace_files(
    base: &Path,
    dir: &Path,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<PathBuf>,
) -> Result<()> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| anyhow!("Failed to read workspace directory {:?}: {}", dir, e))?;

    for entry in entries {
        let entry = entry.map_err(|e| anyhow!("Failed to read directory entry: {}", e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if is_hidden_or_temp_file(&name) {
            continue;
        }

        let file_type = entry
            .file_type()
            .map_err(|e| anyhow!("Failed to read file type for {:?}: {}", entry.path(), e))?;

        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if is_workspace_noise_dir(&name) {
                continue;
            }
            if depth < max_depth {
                collect_workspace_files(base, &entry.path(), depth + 1, max_depth, out)?;
            }
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        let path = entry.path();
        if !path.starts_with(base) {
            continue;
        }
        out.push(path);
    }

    Ok(())
}

fn file_fingerprint(path: &Path) -> Result<WorkspaceFileFingerprint> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| anyhow!("Failed to read file metadata {:?}: {}", path, e))?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or_default();
    Ok(WorkspaceFileFingerprint {
        size: metadata.len(),
        modified_ms,
    })
}

/// Snapshot workspace files (top-level + one subdirectory level by default).
/// Used to detect newly created or modified files between execution phases.
pub fn snapshot_workspace_files(workspace_path: &str) -> Result<WorkspaceSnapshot> {
    let base = Path::new(workspace_path);
    if !base.exists() || !base.is_dir() {
        return Ok(HashMap::new());
    }

    let mut files = Vec::new();
    collect_workspace_files(base, base, 0, DEFAULT_SCAN_MAX_DEPTH, &mut files)?;

    let mut snap = HashMap::with_capacity(files.len());
    for path in files {
        let rel = match path.strip_prefix(base) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let fp = file_fingerprint(&path)?;
        snap.insert(rel_str, fp);
    }
    Ok(snap)
}

fn infer_artifact_type(path: &Path) -> ArtifactType {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "go" | "java" | "kt" | "swift" | "c"
        | "cpp" | "cc" | "h" | "hpp" | "cs" | "php" | "rb" | "sh" | "bash" | "ps1" | "sql"
        | "vue" | "svelte" | "css" | "scss" | "less" | "html" => ArtifactType::Code,
        "md" | "txt" | "doc" | "docx" | "pdf" | "rtf" => ArtifactType::Document,
        "json" | "yaml" | "yml" | "toml" | "ini" | "conf" | "cfg" | "env" | "xml" => {
            ArtifactType::Config
        }
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" => ArtifactType::Image,
        "csv" | "tsv" | "parquet" | "xlsx" | "xls" => ArtifactType::Data,
        _ => ArtifactType::Other,
    }
}

fn should_inline_text(path: &Path, size: u64) -> bool {
    if size > DEFAULT_INLINE_TEXT_LIMIT {
        return false;
    }
    matches!(
        infer_artifact_type(path),
        ArtifactType::Code | ArtifactType::Document | ArtifactType::Config | ArtifactType::Data
    )
}

/// Scan workspace and return only newly created or modified files compared to `before`.
pub fn scan_workspace_artifacts(
    workspace_path: &str,
    before: Option<&WorkspaceSnapshot>,
) -> Result<Vec<ScannedWorkspaceArtifact>> {
    let base = Path::new(workspace_path);
    if !base.exists() || !base.is_dir() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    collect_workspace_files(base, base, 0, DEFAULT_SCAN_MAX_DEPTH, &mut files)?;
    files.sort();

    let mut artifacts = Vec::new();
    for path in files {
        let rel = match path.strip_prefix(base) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let fp = file_fingerprint(&path)?;

        let changed = before
            .and_then(|b| b.get(&rel_str))
            .map(|old| old != &fp)
            .unwrap_or(true);
        if !changed {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unnamed")
            .to_string();
        let mime_type = mime_guess::from_path(&path)
            .first_raw()
            .map(|s| s.to_string());
        let content = if should_inline_text(&path, fp.size) {
            match std::fs::read_to_string(&path) {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::debug!("Cannot read content of {:?}: {}", path, e);
                    None
                }
            }
        } else {
            None
        };

        artifacts.push(ScannedWorkspaceArtifact {
            name,
            relative_path: rel_str,
            artifact_type: infer_artifact_type(&path),
            content,
            mime_type,
            size: fp.size as i64,
        });
    }

    Ok(artifacts)
}

/// Save scanned workspace artifacts to the database.
/// Shared by both MissionExecutor (step-based) and AdaptiveExecutor (goal-based).
pub async fn save_scanned_artifacts(
    agent_service: &AgentService,
    mission_id: &str,
    step_index: u32,
    workspace_path: &str,
    before: Option<&WorkspaceSnapshot>,
    required_artifacts: Option<&[String]>,
) -> Result<()> {
    let required_paths: HashSet<String> = required_artifacts
        .unwrap_or(&[])
        .iter()
        .filter_map(|p| normalize_relative_workspace_path(p))
        .map(|p| p.to_ascii_lowercase())
        .collect();

    let mut artifacts = scan_workspace_artifacts(workspace_path, before)?;
    if before.is_some() {
        let mut seen_paths: HashSet<String> = artifacts
            .iter()
            .map(|item| item.relative_path.to_ascii_lowercase())
            .collect();
        for item in scan_workspace_artifacts(workspace_path, None)? {
            let rel_lower = item.relative_path.to_ascii_lowercase();
            if seen_paths.insert(rel_lower) {
                artifacts.push(item);
            }
        }
    }
    for item in artifacts {
        let rel_lower = item.relative_path.to_ascii_lowercase();
        let is_required = required_paths.contains(&rel_lower);
        if !is_required && is_low_signal_artifact_path(&item.relative_path) {
            continue;
        }
        let doc = MissionArtifactDoc {
            id: None,
            artifact_id: Uuid::new_v4().to_string(),
            mission_id: mission_id.to_string(),
            step_index,
            name: item.name,
            artifact_type: item.artifact_type,
            content: item.content,
            file_path: Some(item.relative_path),
            mime_type: item.mime_type,
            size: item.size,
            archived_document_id: None,
            archived_document_status: None,
            archived_at: None,
            created_at: mongodb::bson::DateTime::now(),
        };
        if let Err(e) = agent_service.save_artifact(&doc).await {
            tracing::warn!("Failed to save artifact '{}': {}", doc.name, e);
        }
    }
    Ok(())
}

pub async fn reconcile_workspace_artifacts(
    agent_service: &AgentService,
    mission_id: &str,
    step_index: u32,
    workspace_path: &str,
) -> Result<()> {
    save_scanned_artifacts(agent_service, mission_id, step_index, workspace_path, None, None).await
}

pub async fn reconcile_workspace_artifacts_with_hints(
    agent_service: &AgentService,
    mission_id: &str,
    step_index: u32,
    workspace_path: &str,
    hinted_paths: &[String],
) -> Result<()> {
    let current_snapshot = snapshot_workspace_files(workspace_path)?;
    save_scanned_artifacts(agent_service, mission_id, step_index, workspace_path, None, None).await?;
    save_required_artifacts(
        agent_service,
        mission_id,
        step_index,
        workspace_path,
        hinted_paths,
    )
    .await?;

    let keep_paths = current_snapshot.keys().cloned().collect::<Vec<_>>();
    match agent_service
        .prune_mission_artifacts_to_paths(mission_id, &keep_paths)
        .await
    {
        Ok(deleted) if deleted > 0 => {
            tracing::info!(
                mission_id = mission_id,
                deleted_artifacts = deleted,
                "Pruned stale mission artifacts after workspace reconciliation"
            );
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(
                mission_id = mission_id,
                %error,
                "Failed to prune stale mission artifacts after workspace reconciliation"
            );
        }
    }

    Ok(())
}

fn parse_worker_step_index(label: &str) -> Option<u32> {
    let digits = label
        .trim()
        .strip_prefix("Step ")?
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    let step_number = digits.parse::<u32>().ok()?;
    step_number.checked_sub(1)
}

pub fn infer_current_step_index(mission: &MissionDoc) -> Option<u32> {
    mission
        .current_step
        .or_else(|| {
            mission
                .latest_worker_state
                .as_ref()
                .and_then(|state| state.current_goal.as_deref())
                .and_then(parse_worker_step_index)
        })
        .or_else(|| {
            mission
                .steps
                .iter()
                .find(|step| {
                    matches!(
                        step.status,
                        StepStatus::Running | StepStatus::AwaitingApproval | StepStatus::Pending
                    )
                })
                .map(|step| step.index as u32)
        })
}

pub fn collect_mission_artifact_hints(mission: &MissionDoc) -> Vec<String> {
    let mut hints = Vec::new();
    if let Some(step_index) = infer_current_step_index(mission) {
        if let Some(step) = mission.steps.get(step_index as usize) {
            hints.extend(step.required_artifacts.iter().cloned());
            if let Some(contract) = step.runtime_contract.as_ref() {
                hints.extend(contract.required_artifacts.iter().cloned());
            }
        }
    }
    if let Some(goal_id) = mission.current_goal_id.as_ref() {
        if let Some(goal) = mission
            .goal_tree
            .as_ref()
            .and_then(|goals| goals.iter().find(|goal| &goal.goal_id == goal_id))
        {
            if let Some(contract) = goal.runtime_contract.as_ref() {
                hints.extend(contract.required_artifacts.iter().cloned());
            }
        }
    } else if let Some(goals) = mission.goal_tree.as_ref() {
        if let Some(goal) = goals.iter().find(|goal| {
            matches!(
                goal.status,
                GoalStatus::Running | GoalStatus::Pending | GoalStatus::Failed
            )
        }) {
            if let Some(contract) = goal.runtime_contract.as_ref() {
                hints.extend(contract.required_artifacts.iter().cloned());
            }
        }
    }
    if let Some(worker_state) = mission.latest_worker_state.as_ref() {
        hints.extend(worker_state.core_assets_now.iter().cloned());
        hints.extend(worker_state.assets_delta.iter().cloned());
    }

    let mut seen = std::collections::BTreeSet::new();
    hints
        .into_iter()
        .filter_map(|path| normalize_relative_workspace_path(&path))
        .filter(|path| seen.insert(path.to_ascii_lowercase()))
        .collect()
}

pub async fn reconcile_mission_artifacts(
    agent_service: &AgentService,
    mission: &MissionDoc,
) -> Result<()> {
    let Some(workspace_path) = mission.workspace_path.as_deref() else {
        return Ok(());
    };
    let hinted_paths = collect_mission_artifact_hints(mission);
    let step_index = infer_current_step_index(mission).unwrap_or(0);
    reconcile_workspace_artifacts_with_hints(
        agent_service,
        &mission.mission_id,
        step_index,
        workspace_path,
        &hinted_paths,
    )
    .await
}

/// Filter out low-signal helper/temp artifacts unless explicitly required by contract.
pub fn is_low_signal_artifact_path(relative_path: &str) -> bool {
    let lower = relative_path.trim().replace('\\', "/").to_ascii_lowercase();
    if lower.is_empty() {
        return true;
    }
    let ext = Path::new(&lower)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    matches!(ext, "bat" | "cmd" | "ps1" | "sh" | "bash" | "tmp" | "temp")
}

pub fn normalize_relative_workspace_path(path: &str) -> Option<String> {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty() {
        return None;
    }
    let parsed = Path::new(&normalized);
    if parsed.is_absolute() {
        return None;
    }
    if !parsed
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return None;
    }
    Some(normalized)
}

/// Ensure declared required artifacts are persisted even when unchanged
/// between snapshot boundaries (e.g. generated in previous retries).
pub async fn save_required_artifacts(
    agent_service: &AgentService,
    mission_id: &str,
    step_index: u32,
    workspace_path: &str,
    required_artifacts: &[String],
) -> Result<()> {
    if required_artifacts.is_empty() {
        return Ok(());
    }

    let base = Path::new(workspace_path);
    if !base.exists() || !base.is_dir() {
        return Ok(());
    }

    for rel in required_artifacts {
        let rel = match normalize_relative_workspace_path(rel) {
            Some(v) => v,
            None => {
                tracing::warn!("Skip invalid required artifact path: {}", rel);
                continue;
            }
        };
        let full = base.join(&rel);
        if !full.exists() || !full.is_file() {
            continue;
        }

        let fp = file_fingerprint(&full)?;
        let name = full
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unnamed")
            .to_string();
        let mime_type = mime_guess::from_path(&full)
            .first_raw()
            .map(|s| s.to_string());
        let content = if should_inline_text(&full, fp.size) {
            std::fs::read_to_string(&full).ok()
        } else {
            None
        };

        let doc = MissionArtifactDoc {
            id: None,
            artifact_id: Uuid::new_v4().to_string(),
            mission_id: mission_id.to_string(),
            step_index,
            name,
            artifact_type: infer_artifact_type(&full),
            content,
            file_path: Some(rel),
            mime_type,
            size: fp.size as i64,
            archived_document_id: None,
            archived_document_status: None,
            archived_at: None,
            created_at: mongodb::bson::DateTime::now(),
        };
        if let Err(e) = agent_service.save_artifact(&doc).await {
            tracing::warn!("Failed to save required artifact '{}': {}", doc.name, e);
        }
    }

    Ok(())
}

/// Scan a project directory and return a context string with file tree and key file contents.
/// Used to give the agent immediate awareness of the project structure at session start.
pub fn scan_project_context(project_path: &str, max_total_bytes: usize) -> String {
    let base = std::path::Path::new(project_path);
    if !base.is_dir() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str("<project_context>\n");

    // 1. File tree (max 3 levels)
    out.push_str("## 文件结构\n```\n");
    collect_tree(base, 0, 3, &mut out);
    out.push_str("```\n\n");

    // 2. Read key files
    const KEY_FILES: &[&str] = &[
        "README.md",
        "readme.md",
        "README.txt",
        "index.html",
        "package.json",
        "Cargo.toml",
        "pyproject.toml",
        "requirements.txt",
        ".env.example",
        "portal-sdk.js",
        "portal-agent-client.js",
    ];

    let mut remaining = max_total_bytes.saturating_sub(out.len());
    for name in KEY_FILES {
        if remaining < 100 {
            break;
        }
        let file_path = base.join(name);
        if let Ok(content) = std::fs::read_to_string(&file_path) {
            if content.trim().is_empty() {
                continue;
            }
            let truncated: String = content.chars().take(remaining.min(4000)).collect();
            out.push_str(&format!("## {}\n```\n{}\n```\n\n", name, truncated));
            remaining = remaining.saturating_sub(truncated.len() + name.len() + 20);
        }
    }

    out.push_str("</project_context>");
    out
}

fn collect_tree(dir: &std::path::Path, depth: usize, max_depth: usize, out: &mut String) {
    if depth >= max_depth {
        return;
    }
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    entries.sort_by_key(|e| e.file_name());

    let skip = [
        "node_modules",
        ".git",
        "target",
        "__pycache__",
        ".next",
        "dist",
    ];
    let indent = "  ".repeat(depth);
    let mut count = 0;
    for entry in &entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if skip.iter().any(|s| *s == name) {
            continue;
        }
        let is_dir = entry.file_type().is_ok_and(|ft| ft.is_dir());
        if is_dir {
            out.push_str(&format!("{}{}/\n", indent, name));
            collect_tree(&entry.path(), depth + 1, max_depth, out);
        } else {
            out.push_str(&format!("{}{}\n", indent, name));
        }
        count += 1;
        if count >= 80 {
            out.push_str(&format!("{}... (truncated)\n", indent));
            break;
        }
    }
}

/// Classify whether an error is transient and worth retrying.
pub fn is_non_retryable_external_provider_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    let direct_blockers = [
        "authentication token has been invalidated",
        "authentication failed",
        "401 unauthorized",
        "auth_unavailable",
        "no auth available",
        "provider credentials unavailable",
        "credentials unavailable",
        "no valid coding plan subscription",
        "valid coding plan subscription",
        "subscription has expired",
        "subscription expired",
        "subscription is not active",
        "billing account not active",
    ];
    direct_blockers
        .iter()
        .any(|pattern| lower.contains(pattern))
}

pub fn is_waiting_external_provider_message(message: &str) -> bool {
    if is_non_retryable_external_provider_message(message) {
        return true;
    }

    let lower = message.to_ascii_lowercase();
    let mentions_capacity = lower.contains("rate limit exceeded")
        || lower.contains("usage limit has been reached")
        || lower.contains("too many requests")
        || lower.contains("resource exhausted");
    let mentions_provider_context = lower.contains("cooling down")
        || lower.contains("all credentials")
        || lower.contains("model")
        || lower.contains("provider")
        || lower.contains("usage limit")
        || lower.contains("credential");
    mentions_capacity && mentions_provider_context
}

pub fn blocker_fingerprint(message: &str) -> Option<String> {
    let normalized = message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    let mut compact = normalized;
    if compact.len() > 180 {
        compact.truncate(180);
    }
    Some(compact)
}

pub fn is_retryable_error(e: &anyhow::Error) -> bool {
    let msg = e.to_string().to_lowercase();
    if is_non_retryable_external_provider_message(&msg) {
        return false;
    }
    if is_waiting_external_provider_message(&msg) {
        return false;
    }
    let retryable_patterns = [
        "timeout",
        "timed out",
        "context deadline exceeded",
        "rate limit",
        "too many requests",
        "429",
        "503",
        "502",
        "504",
        "service unavailable",
        "bad gateway",
        "gateway timeout",
        "connection reset",
        "connection closed",
        "connection aborted",
        "connection refused",
        "broken pipe",
        "unexpected eof",
        "end of file",
        "error decoding response body",
        "stream decode error",
        "stream ended without producing a message",
        "temporarily unavailable",
        "overloaded",
        "upstream connect error",
        "tls handshake timeout",
        "tls handshake eof",
        "handshake eof",
        "transport error",
    ];
    retryable_patterns.iter().any(|p| msg.contains(p))
}

// ─── Extension Override Helpers ────────────────────────

/// Result of computing extension overrides between agent config and runtime state.
pub struct ExtensionOverrides {
    pub disabled: Vec<String>,
    pub enabled: Vec<String>,
}

/// Compute extension overrides by diffing agent default config against active runtime names.
///
/// Shared by `executor_mongo` (session save on shutdown) and
/// `extension_manager_client` (session sync after manage operations).
///
/// Rules:
/// - ExtensionManager, ChatRecall, Team, and DocumentTools are excluded
///   (ExtensionManager/ChatRecall/Team are never loaded as regular extensions;
///   DocumentTools is always force-loaded as fallback by PlatformExtensionRunner)
/// - MCP subprocess extensions use `mcp_name()` as their runtime name
/// - Skills→team_skills replacement is handled when team_skills is active
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
            // Skills may be replaced by team_skills at runtime
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

#[cfg(test)]
mod tests {
    use super::{
        collect_execution_guard_signals_since, count_session_messages,
        extract_latest_preflight_contract_since, extract_tool_calls, extract_tool_calls_since,
        is_non_retryable_external_provider_message, is_retryable_error,
        is_waiting_external_provider_message, parse_first_json_value,
    };
    use anyhow::anyhow;
    use serde_json::json;

    #[test]
    fn extracts_tool_calls_from_current_schema() {
        let messages = json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "toolRequest",
                        "id": "req_1",
                        "toolCall": {
                            "status": "success",
                            "value": { "name": "developer__shell", "arguments": {"command":"dir"} }
                        }
                    }
                ]
            },
            {
                "role": "user",
                "content": [
                    {
                        "type": "toolResponse",
                        "id": "req_1",
                        "toolResult": {
                            "status": "success",
                            "value": {"content": []}
                        }
                    }
                ]
            }
        ]);

        let raw = serde_json::to_string(&messages).unwrap();
        let calls = extract_tool_calls(&raw);
        assert_eq!(calls, vec![("developer__shell".to_string(), true)]);
    }

    #[test]
    fn extracts_tool_calls_with_start_index() {
        let messages = json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "toolRequest",
                        "id": "old_req",
                        "toolCall": {
                            "status": "success",
                            "value": { "name": "old_tool" }
                        }
                    }
                ]
            },
            {
                "role": "user",
                "content": [
                    {
                        "type": "toolResponse",
                        "id": "old_req",
                        "toolResult": { "status": "success", "value": {"content": []} }
                    }
                ]
            },
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "toolRequest",
                        "id": "new_req",
                        "toolCall": {
                            "status": "success",
                            "value": { "name": "new_tool" }
                        }
                    }
                ]
            },
            {
                "role": "user",
                "content": [
                    {
                        "type": "toolResponse",
                        "id": "new_req",
                        "toolResult": { "status": "error", "error": "boom" }
                    }
                ]
            }
        ]);

        let raw = serde_json::to_string(&messages).unwrap();
        let calls = extract_tool_calls_since(&raw, 2);
        assert_eq!(calls, vec![("new_tool".to_string(), false)]);
        assert_eq!(count_session_messages(&raw), 4);
    }

    #[test]
    fn extracts_tool_calls_from_legacy_schema() {
        let messages = json!([
            {
                "role": "assistant",
                "content": [
                    { "type": "tool_use", "id": "legacy_1", "name": "web_scrape" }
                ]
            },
            {
                "role": "tool",
                "content": [
                    { "type": "tool_result", "tool_use_id": "legacy_1", "is_error": false }
                ]
            }
        ]);
        let raw = serde_json::to_string(&messages).unwrap();
        let calls = extract_tool_calls(&raw);
        assert_eq!(calls, vec![("web_scrape".to_string(), true)]);
    }

    #[test]
    fn detects_max_turn_limit_and_external_path() {
        let messages = json!([
            {
                "role": "assistant",
                "content": [
                    {"type":"text","text":"[Warning: Agent reached maximum turn limit (8). Task may be incomplete.]"}
                ]
            },
            {
                "role": "user",
                "content": [
                    {
                        "type":"toolResponse",
                        "id":"req_1",
                        "toolResult":{
                            "status":"success",
                            "value":{
                                "content":[
                                    {"type":"text","text":"Saved: E:/yw/agiatme/goose/output/deck.pptx"}
                                ]
                            }
                        }
                    }
                ]
            }
        ]);
        let raw = serde_json::to_string(&messages).unwrap();
        let signals = collect_execution_guard_signals_since(
            &raw,
            0,
            Some("E:/yw/agiatme/goose/data/workspaces/team/missions/abc"),
        );
        assert!(signals.max_turn_limit_warning);
        assert_eq!(signals.external_output_paths.len(), 1);
        assert_eq!(
            signals.external_output_paths[0],
            "E:/yw/agiatme/goose/output/deck.pptx"
        );
    }

    #[test]
    fn ignores_workspace_internal_absolute_path() {
        let messages = json!([
            {
                "role": "user",
                "content": [
                    {
                        "type":"toolResponse",
                        "id":"req_1",
                        "toolResult":{
                            "status":"success",
                            "value":{
                                "content":[
                                    {"type":"text","text":"Saved: E:/yw/agiatme/goose/data/workspaces/team/missions/abc/output/deck.pptx"}
                                ]
                            }
                        }
                    }
                ]
            }
        ]);
        let raw = serde_json::to_string(&messages).unwrap();
        let signals = collect_execution_guard_signals_since(
            &raw,
            0,
            Some("E:/yw/agiatme/goose/data/workspaces/team/missions/abc"),
        );
        assert!(!signals.max_turn_limit_warning);
        assert!(signals.external_output_paths.is_empty());
    }

    #[test]
    fn preflight_contract_since_ignores_older_successful_calls() {
        let messages = json!([
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "toolRequest",
                        "id": "pre_1",
                        "toolCall": {
                            "value": {
                                "name": "mission_preflight__preflight",
                                "arguments": {
                                    "required_artifacts": ["reports/old.md"],
                                    "completion_checks": ["exists:reports/old.md"]
                                }
                            }
                        }
                    }
                ]
            },
            {
                "role": "user",
                "content": [
                    {
                        "type": "toolResponse",
                        "id": "pre_1",
                        "toolResult": {
                            "status": "success",
                            "value": {
                                "content": [
                                    {
                                        "type": "text",
                                        "text": "{\"status\":\"ready\",\"contract\":{\"required_artifacts\":[\"reports/old.md\"],\"completion_checks\":[\"exists:reports/old.md\"],\"no_artifact_reason\":null}}"
                                    }
                                ]
                            }
                        }
                    }
                ]
            },
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "text",
                        "text": "retry summary without a new preflight call"
                    }
                ]
            }
        ]);

        let raw = serde_json::to_string(&messages).unwrap();
        let contract =
            extract_latest_preflight_contract_since(&raw, 2, "mission_preflight__preflight");

        assert!(contract.is_none());
    }

    #[cfg(not(windows))]
    #[test]
    fn ignores_http_endpoint_style_paths_when_collecting_external_outputs() {
        let messages = json!([
            {
                "role": "assistant",
                "content": [
                    {"type":"text","text":"GET /v1/models returned 200; POST /v1/chat/completions returned 200"}
                ]
            }
        ]);
        let raw = serde_json::to_string(&messages).unwrap();
        let signals = collect_execution_guard_signals_since(
            &raw,
            0,
            Some("/opt/agime-data/workspaces/team/missions/abc"),
        );
        assert!(signals.external_output_paths.is_empty());
    }

    #[cfg(not(windows))]
    #[test]
    fn ignores_pid_fd_log_fragments_when_collecting_external_outputs() {
        let messages = json!([
            {
                "role": "assistant",
                "content": [
                    {"type":"text","text":"Step wrote files outside workspace: /opt/agime\",pid=99019,fd=23. Save outputs under workspace-relative paths."}
                ]
            }
        ]);
        let raw = serde_json::to_string(&messages).unwrap();
        let signals = collect_execution_guard_signals_since(
            &raw,
            0,
            Some("/opt/agime-data/workspaces/team/missions/abc"),
        );
        assert!(signals.external_output_paths.is_empty());
    }

    #[cfg(not(windows))]
    #[test]
    fn ignores_placeholder_style_absolute_paths() {
        let messages = json!([
            {
                "role": "assistant",
                "content": [
                    {"type":"text","text":"Workspace root: /opt/agime-data/.../missions/..."}
                ]
            }
        ]);
        let raw = serde_json::to_string(&messages).unwrap();
        let signals = collect_execution_guard_signals_since(
            &raw,
            0,
            Some("/opt/agime-data/workspaces/team/missions/abc"),
        );
        assert!(signals.external_output_paths.is_empty());
    }

    #[cfg(not(windows))]
    #[test]
    fn ignores_absolute_paths_that_only_appear_in_assistant_explanations() {
        let messages = json!([
            {
                "role": "assistant",
                "content": [
                    {"type":"text","text":"Root cause: the old report mentioned /tmp/doc.json, but the fixed report now uses reports/doc.json."}
                ]
            }
        ]);
        let raw = serde_json::to_string(&messages).unwrap();
        let signals = collect_execution_guard_signals_since(
            &raw,
            0,
            Some("/opt/agime-data/workspaces/team/missions/abc"),
        );
        assert!(signals.external_output_paths.is_empty());
    }

    #[test]
    fn parse_first_json_value_accepts_trailing_text_after_object() {
        let value = parse_first_json_value(
            r#"{
                "decision": "continue_with_replan",
                "reason": "still missing final synthesis"
            }

            Additional note: keep reusing the existing workspace.
            "#,
        )
        .expect("json should parse");

        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("continue_with_replan")
        );
    }

    #[test]
    fn parse_first_json_value_accepts_fenced_json_with_trailing_commentary() {
        let value = parse_first_json_value(
            r#"```json
            {
              "decision": "complete",
              "reason": "evidence is sufficient"
            }
            ```

            The rest of this answer is non-JSON commentary.
            "#,
        )
        .expect("fenced json should parse");

        assert_eq!(
            value.get("reason").and_then(|v| v.as_str()),
            Some("evidence is sufficient")
        );
    }

    #[test]
    fn waiting_external_provider_message_detects_auth_and_subscription_blocks() {
        assert!(is_waiting_external_provider_message(
            "Authentication failed. Status: 401 Unauthorized. Response: Your authentication token has been invalidated."
        ));
        assert!(is_waiting_external_provider_message(
            "Request failed: Bad request (400): Your account does not have a valid coding plan subscription, or your subscription has expired"
        ));
        assert!(is_non_retryable_external_provider_message(
            "auth_unavailable: no auth available for provider"
        ));
        assert!(!is_non_retryable_external_provider_message(
            "Rate limit exceeded: All credentials for model gpt-5.2 are cooling down"
        ));
    }

    #[test]
    fn retryable_error_excludes_non_retryable_external_provider_blocks() {
        let auth_err = anyhow!(
            "Authentication failed. Status: 401 Unauthorized. Response: Your authentication token has been invalidated."
        );
        let subscription_err = anyhow!(
            "Request failed: Bad request (400): Your account does not have a valid coding plan subscription, or your subscription has expired"
        );
        let rate_limit_err = anyhow!(
            "Rate limit exceeded: All credentials for model gpt-5.2 are cooling down"
        );

        assert!(!is_retryable_error(&auth_err));
        assert!(!is_retryable_error(&subscription_err));
        assert!(!is_retryable_error(&rate_limit_err));
    }
}
