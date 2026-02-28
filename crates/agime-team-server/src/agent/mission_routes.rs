//! Mission API routes (Phase 2 - Mission Track)
//!
//! Mounted at `/api/team/agent/mission`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, Sse},
        Json,
    },
    routing::{delete, get, post},
    Extension, Router,
};
use futures::stream::Stream;
use futures::StreamExt;
use std::convert::Infallible;
use std::path::Component;
use std::sync::Arc;
use std::time::Duration;

use crate::auth::middleware::UserContext;
use agime_team::models::mongo::{
    DocumentCategory, DocumentOrigin, DocumentStatus, DocumentSummary,
};
use agime_team::services::mongo::DocumentService;
use agime_team::MongoDb;

use super::mission_executor::MissionExecutor;
use super::mission_manager::{MissionManager, MissionRegistration};
use super::mission_mongo::MissionDoc;
use super::mission_mongo::{
    resolve_execution_profile, ArtifactType, CreateFromChatRequest, CreateMissionRequest,
    GoalActionRequest, GoalStatus, ListMissionsQuery, MissionRouteMode, MissionStatus,
    StepActionRequest, StepStatus,
};
use super::service_mongo::AgentService;
use super::task_manager::StreamEvent;

type MissionState = (Arc<AgentService>, Arc<MongoDb>, Arc<MissionManager>, String);

#[derive(serde::Deserialize, Default)]
struct StreamQuery {
    last_event_id: Option<u64>,
}

#[derive(serde::Deserialize, Default)]
struct EventListQuery {
    after_event_id: Option<u64>,
    limit: Option<u32>,
    run_id: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct ArchiveArtifactRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    folder_path: Option<String>,
    #[serde(default)]
    category: Option<DocumentCategory>,
}

/// Check that the user is either the mission creator or a team admin.
async fn require_creator_or_admin(
    service: &AgentService,
    user_id: &str,
    mission: &MissionDoc,
) -> Result<(), StatusCode> {
    if mission.creator_id == user_id {
        return Ok(());
    }
    let is_admin = service
        .is_team_admin(user_id, &mission.team_id)
        .await
        .unwrap_or(false);
    if is_admin {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

fn env_flag(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| {
            let norm = v.trim().to_ascii_lowercase();
            matches!(norm.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

fn default_doc_category_for_artifact(kind: &ArtifactType) -> DocumentCategory {
    match kind {
        ArtifactType::Code | ArtifactType::Config => DocumentCategory::Code,
        ArtifactType::Document => DocumentCategory::Report,
        _ => DocumentCategory::General,
    }
}

fn is_safe_relative_path(path: &str) -> bool {
    let p = std::path::Path::new(path);
    !p.is_absolute() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

async fn read_artifact_bytes(
    artifact: &super::mission_mongo::MissionArtifactDoc,
    mission: &MissionDoc,
    workspace_root: &str,
) -> Result<Vec<u8>, StatusCode> {
    if let Some(ref content) = artifact.content {
        return Ok(content.as_bytes().to_vec());
    }

    let rel_path = artifact.file_path.as_deref().ok_or(StatusCode::NOT_FOUND)?;
    if !is_safe_relative_path(rel_path) {
        return Err(StatusCode::FORBIDDEN);
    }

    let ws_path = mission
        .workspace_path
        .as_deref()
        .ok_or(StatusCode::NOT_FOUND)?;
    let ws_canonical = tokio::fs::canonicalize(ws_path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if !ws_canonical.is_dir() {
        return Err(StatusCode::NOT_FOUND);
    }

    let workspace_root_canonical = tokio::fs::canonicalize(workspace_root)
        .await
        .unwrap_or_else(|_| std::path::PathBuf::from(workspace_root));
    if !ws_canonical.starts_with(&workspace_root_canonical) {
        tracing::warn!(
            "Reject artifact read outside workspace root: mission={}, workspace={:?}, root={:?}",
            mission.mission_id,
            ws_canonical,
            workspace_root_canonical
        );
        return Err(StatusCode::FORBIDDEN);
    }

    let rel = std::path::Path::new(rel_path);
    let full_path = ws_canonical.join(rel);
    let full_canonical = tokio::fs::canonicalize(&full_path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if !full_canonical.starts_with(&ws_canonical) || !full_canonical.is_file() {
        return Err(StatusCode::FORBIDDEN);
    }

    tokio::fs::read(&full_canonical)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)
}

/// Validate that a goal exists in the mission's goal tree and is in AwaitingApproval status.
fn validate_goal_awaiting_approval(mission: &MissionDoc, goal_id: &str) -> Result<(), StatusCode> {
    match mission.goal_tree {
        Some(ref goals) => match goals.iter().find(|g| g.goal_id == goal_id) {
            Some(g) if g.status != GoalStatus::AwaitingApproval => Err(StatusCode::CONFLICT),
            Some(_) => Ok(()),
            None => Err(StatusCode::NOT_FOUND),
        },
        None => Err(StatusCode::NOT_FOUND),
    }
}

fn should_route_to_direct(req: &CreateMissionRequest) -> bool {
    match req.route_mode.as_ref() {
        Some(MissionRouteMode::Direct) => return true,
        Some(MissionRouteMode::Mission) => return false,
        Some(MissionRouteMode::Auto) | None => {}
    }

    if !env_flag("TEAM_MISSION_AUTO_DIRECT_ENABLED", false) {
        return false;
    }

    if req
        .execution_mode
        .as_ref()
        .is_some_and(|mode| matches!(mode, super::mission_mongo::ExecutionMode::Adaptive))
    {
        return false;
    }

    if req
        .approval_policy
        .as_ref()
        .is_some_and(|policy| !matches!(policy, super::mission_mongo::ApprovalPolicy::Auto))
    {
        return false;
    }

    if req.token_budget.unwrap_or(0) > 0
        || req.step_timeout_seconds.is_some()
        || req.step_max_retries.is_some()
    {
        return false;
    }

    let goal_len = req.goal.chars().count();
    let ctx_len = req
        .context
        .as_deref()
        .map(|s| s.chars().count())
        .unwrap_or(0);
    let doc_count = req.attached_document_ids.len();
    let goal_max = env_usize("TEAM_MISSION_AUTO_DIRECT_GOAL_MAX_CHARS", 120);
    let context_max = env_usize("TEAM_MISSION_AUTO_DIRECT_CONTEXT_MAX_CHARS", 220);
    let docs_max = env_usize("TEAM_MISSION_AUTO_DIRECT_MAX_DOCS", 0);

    goal_len <= goal_max && ctx_len <= context_max && doc_count <= docs_max
}

/// Recursively convert bson DateTime JSON (`{"$date":{"$numberLong":"ms"}}`) to RFC3339 strings.
fn fix_bson_dates(val: &mut serde_json::Value) {
    match val {
        serde_json::Value::Object(map) => {
            if map.len() == 1 {
                if let Some(inner) = map.get("$date").and_then(|d| d.as_object()) {
                    if let Some(ms_str) = inner.get("$numberLong").and_then(|v| v.as_str()) {
                        if let Ok(ms) = ms_str.parse::<i64>() {
                            let dt = bson::DateTime::from_millis(ms);
                            *val = serde_json::Value::String(dt.to_chrono().to_rfc3339());
                            return;
                        }
                    }
                }
            }
            for v in map.values_mut() {
                fix_bson_dates(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                fix_bson_dates(v);
            }
        }
        _ => {}
    }
}

/// Serialize a MissionDoc to JSON with all bson::DateTime fields as RFC3339 strings.
fn mission_to_json(mission: &MissionDoc) -> serde_json::Value {
    let mut val = serde_json::to_value(mission).unwrap_or_default();
    fix_bson_dates(&mut val);
    let resolved_profile = resolve_execution_profile(mission);
    // Remove internal MongoDB _id field
    if let Some(obj) = val.as_object_mut() {
        obj.remove("_id");
        obj.insert(
            "resolved_execution_profile".to_string(),
            serde_json::to_value(resolved_profile).unwrap_or(serde_json::json!("full")),
        );
    }
    val
}

/// Register mission execution with a short grace wait.
///
/// This smooths pause->resume race where the previous executor is still
/// unwinding and has not called `complete()` yet.
async fn register_with_grace(
    mission_manager: &Arc<MissionManager>,
    mission_id: &str,
) -> Option<MissionRegistration> {
    if let Some(pair) = mission_manager.register(mission_id).await {
        return Some(pair);
    }

    let grace_ms = std::env::var("TEAM_MISSION_REGISTER_GRACE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1500);
    let step_ms = 100u64;
    let mut waited = 0u64;
    while waited < grace_ms {
        tokio::time::sleep(Duration::from_millis(step_ms)).await;
        waited = waited.saturating_add(step_ms);
        if !mission_manager.is_active(mission_id).await {
            if let Some(pair) = mission_manager.register(mission_id).await {
                return Some(pair);
            }
        }
    }
    None
}

/// Create mission router
pub fn mission_router(
    db: Arc<MongoDb>,
    mission_manager: Arc<MissionManager>,
    workspace_root: String,
) -> Router {
    let service = Arc::new(AgentService::new(db.clone()));

    Router::new()
        .route("/missions", post(create_mission))
        .route("/missions", get(list_missions))
        .route("/missions/{id}", get(get_mission))
        .route("/missions/{id}", delete(delete_mission))
        .route("/missions/{id}/start", post(start_mission))
        .route("/missions/{id}/resume", post(resume_mission_handler))
        .route("/missions/{id}/pause", post(pause_mission))
        .route("/missions/{id}/cancel", post(cancel_mission))
        .route("/missions/{id}/steps/{idx}/approve", post(approve_step))
        .route("/missions/{id}/steps/{idx}/reject", post(reject_step))
        .route("/missions/{id}/steps/{idx}/skip", post(skip_step))
        .route("/missions/{id}/stream", get(stream_mission))
        .route("/missions/{id}/events", get(list_mission_events))
        // AGE goal operations
        .route("/missions/{id}/goals/{goal_id}/approve", post(approve_goal))
        .route("/missions/{id}/goals/{goal_id}/reject", post(reject_goal))
        .route("/missions/{id}/goals/{goal_id}/pivot", post(pivot_goal))
        .route(
            "/missions/{id}/goals/{goal_id}/abandon",
            post(abandon_goal_handler),
        )
        .route("/missions/{id}/artifacts", get(list_artifacts))
        .route("/artifacts/{id}", get(get_artifact))
        .route("/artifacts/{id}/download", get(download_artifact))
        .route(
            "/artifacts/{id}/archive",
            post(archive_artifact_to_document),
        )
        .route("/from-chat", post(create_from_chat))
        // Phase 2: Document attachment
        .route(
            "/missions/{id}/documents",
            get(list_mission_documents)
                .post(attach_mission_documents)
                .delete(detach_mission_documents),
        )
        .with_state((service, db, mission_manager, workspace_root))
}

// ─── CRUD Handlers ───────────────────────────────────────

async fn create_mission(
    State((service, db, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<CreateMissionRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
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

    if should_route_to_direct(&req) {
        let direct_max_turns = std::env::var("TEAM_DIRECT_SESSION_MAX_TURNS")
            .ok()
            .and_then(|v| v.parse::<i32>().ok())
            .filter(|v| *v > 0);
        let direct_tool_timeout_secs = std::env::var("TEAM_DIRECT_SESSION_TOOL_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0);
        let session = service
            .create_chat_session(
                &team_id,
                &req.agent_id,
                &user.user_id,
                req.attached_document_ids.clone(),
                req.context.clone(),
                None,
                None,
                None,
                direct_max_turns,
                direct_tool_timeout_secs,
                None,
                false,
                false,
                None,
                None,
                None,
                None,
            )
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to create direct chat session from mission request: {:?}",
                    e
                );
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        return Ok(Json(serde_json::json!({
            "route": "direct",
            "status": "direct_ready",
            "session_id": session.session_id,
            "agent_id": session.agent_id,
            "message": "Routed to direct chat session for lightweight request",
        })));
    }

    let mission = service
        .create_mission(&req, &team_id, &user.user_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create mission: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(serde_json::json!({
        "route": "mission",
        "mission_id": mission.mission_id,
        "status": mission.status,
    })))
}

async fn list_missions(
    State((service, _, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<ListMissionsQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    let items = service.list_missions(query).await.map_err(|e| {
        tracing::error!("Failed to list missions: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(serde_json::json!(items)))
}

async fn get_mission(
    State((service, _, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &mission.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(Json(mission_to_json(&mission)))
}

async fn delete_mission(
    State((service, _, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    require_creator_or_admin(&service, &user.user_id, &mission).await?;

    let deleted = service
        .delete_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if deleted {
        // P2: Best-effort workspace cleanup (after DB delete to avoid orphaned records)
        if let Err(e) = super::runtime::cleanup_workspace_dir(mission.workspace_path.as_deref()) {
            tracing::warn!(
                "Failed to cleanup workspace for mission {}: {}",
                mission_id,
                e
            );
        }
        Ok(StatusCode::NO_CONTENT)
    } else {
        // Mission was verified above but disappeared before delete — concurrent deletion
        Err(StatusCode::CONFLICT)
    }
}

// ─── Lifecycle Handlers ──────────────────────────────────

async fn start_mission(
    State((service, db, mission_manager, ref workspace_root)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    require_creator_or_admin(&service, &user.user_id, &mission).await?;

    if matches!(
        mission.status,
        MissionStatus::Planning | MissionStatus::Running
    ) {
        return Ok(Json(
            serde_json::json!({ "mission_id": mission_id, "status": "already_running" }),
        ));
    }

    if mission.status != MissionStatus::Draft && mission.status != MissionStatus::Planned {
        return Err(StatusCode::CONFLICT);
    }

    // Start should be single-shot: do not wait-and-retry registration.
    // Graceful re-register is only appropriate for resume/step actions.
    let registration = match mission_manager.register(&mission_id).await {
        Some(registration) => registration,
        None => {
            return Ok(Json(
                serde_json::json!({ "mission_id": mission_id, "status": "already_running" }),
            ))
        }
    };
    let run_id = registration.run_id.clone();
    let cancel_token = registration.cancel_token;
    if let Err(e) = service.set_mission_current_run(&mission_id, &run_id).await {
        mission_manager.complete(&mission_id).await;
        tracing::error!(
            "Failed to set current run for mission {}: {}",
            mission_id,
            e
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    let executor =
        MissionExecutor::new(db.clone(), mission_manager.clone(), workspace_root.clone());
    let mid = mission_id.clone();
    tokio::spawn(async move {
        if let Err(e) = executor.execute_mission(&mid, cancel_token).await {
            tracing::error!("Mission execution failed: {}: {}", mid, e);
        }
    });

    Ok(Json(
        serde_json::json!({ "mission_id": mission_id, "status": "starting" }),
    ))
}

async fn pause_mission(
    State((service, _, mission_manager, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    require_creator_or_admin(&service, &user.user_id, &mission).await?;

    if !matches!(
        mission.status,
        MissionStatus::Running | MissionStatus::Planning
    ) {
        return Err(StatusCode::CONFLICT);
    }

    service
        .update_mission_status(&mission_id, &MissionStatus::Paused)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let from = match mission.status {
        MissionStatus::Planning => "planning",
        MissionStatus::Running => "running",
        _ => "unknown",
    };
    mission_manager
        .broadcast(
            &mission_id,
            StreamEvent::Status {
                status: serde_json::json!({
                    "type": "mission_pausing",
                    "from_status": from,
                })
                .to_string(),
            },
        )
        .await;
    mission_manager.signal_cancel(&mission_id).await;
    Ok(StatusCode::OK)
}

async fn resume_mission_handler(
    State((service, db, mission_manager, ref workspace_root)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
    body: Option<Json<StepActionRequest>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    require_creator_or_admin(&service, &user.user_id, &mission).await?;

    if matches!(
        mission.status,
        MissionStatus::Planning | MissionStatus::Running
    ) {
        return Ok(Json(
            serde_json::json!({ "mission_id": mission_id, "status": "already_running" }),
        ));
    }

    if !matches!(
        mission.status,
        MissionStatus::Paused | MissionStatus::Failed
    ) {
        return Err(StatusCode::CONFLICT);
    }

    let feedback = body
        .and_then(|Json(b)| b.feedback)
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let registration = match register_with_grace(&mission_manager, &mission_id).await {
        Some(registration) => registration,
        None => {
            let status = if mission.status == MissionStatus::Paused {
                "pause_in_progress"
            } else {
                "already_running"
            };
            return Ok(Json(
                serde_json::json!({ "mission_id": mission_id, "status": status }),
            ));
        }
    };
    let run_id = registration.run_id.clone();
    let cancel_token = registration.cancel_token;
    if let Err(e) = service.set_mission_current_run(&mission_id, &run_id).await {
        mission_manager.complete(&mission_id).await;
        tracing::error!(
            "Failed to set current run for mission {}: {}",
            mission_id,
            e
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    let executor =
        MissionExecutor::new(db.clone(), mission_manager.clone(), workspace_root.clone());
    let mid = mission_id.clone();
    tokio::spawn(async move {
        if let Err(e) = executor.resume_mission(&mid, cancel_token, feedback).await {
            tracing::error!("Mission resume failed: {}: {}", mid, e);
        }
    });

    Ok(Json(
        serde_json::json!({ "mission_id": mission_id, "status": "resuming" }),
    ))
}

async fn cancel_mission(
    State((service, _, mission_manager, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if mission.creator_id != user.user_id {
        let is_admin = service
            .is_team_admin(&user.user_id, &mission.team_id)
            .await
            .unwrap_or(false);
        if !is_admin {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    if mission.status == MissionStatus::Cancelled {
        return Ok(StatusCode::OK);
    }
    let cancellable = matches!(
        mission.status,
        MissionStatus::Draft
            | MissionStatus::Planned
            | MissionStatus::Planning
            | MissionStatus::Running
            | MissionStatus::Paused
    );
    if !cancellable {
        return Err(StatusCode::CONFLICT);
    }

    mission_manager.signal_cancel(&mission_id).await;
    service
        .update_mission_status(&mission_id, &MissionStatus::Cancelled)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::OK)
}

// ─── Step Handlers ───────────────────────────────────────

async fn approve_step(
    State((service, db, mission_manager, ref workspace_root)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path((mission_id, step_idx)): Path<(String, u32)>,
    Json(body): Json<StepActionRequest>,
) -> Result<StatusCode, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = service
        .is_team_admin(&user.user_id, &mission.team_id)
        .await
        .unwrap_or(false);
    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    if mission.status != MissionStatus::Paused {
        return Err(StatusCode::CONFLICT);
    }
    let step = mission
        .steps
        .iter()
        .find(|s| s.index == step_idx)
        .ok_or(StatusCode::NOT_FOUND)?;
    if step.status != StepStatus::AwaitingApproval {
        return Err(StatusCode::CONFLICT);
    }

    // Resume execution
    let registration = match register_with_grace(&mission_manager, &mission_id).await {
        Some(registration) => registration,
        None => return Err(StatusCode::CONFLICT),
    };
    let run_id = registration.run_id.clone();
    let cancel_token = registration.cancel_token;
    if let Err(e) = service.set_mission_current_run(&mission_id, &run_id).await {
        mission_manager.complete(&mission_id).await;
        tracing::error!(
            "Failed to set current run for mission {}: {}",
            mission_id,
            e
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    if let Err(e) = service
        .approve_step(&mission_id, step_idx, &user.user_id)
        .await
    {
        mission_manager.complete(&mission_id).await;
        tracing::error!(
            "Failed to approve step {} for {}: {}",
            step_idx,
            mission_id,
            e
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    let executor =
        MissionExecutor::new(db.clone(), mission_manager.clone(), workspace_root.clone());
    let mid = mission_id.clone();
    let feedback = body
        .feedback
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    tokio::spawn(async move {
        if let Err(e) = executor.resume_mission(&mid, cancel_token, feedback).await {
            tracing::error!("Mission resume failed: {}: {}", mid, e);
        }
    });

    Ok(StatusCode::OK)
}

async fn reject_step(
    State((service, _, mission_manager, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path((mission_id, step_idx)): Path<(String, u32)>,
    Json(_body): Json<StepActionRequest>,
) -> Result<StatusCode, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = service
        .is_team_admin(&user.user_id, &mission.team_id)
        .await
        .unwrap_or(false);
    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    if mission.status != MissionStatus::Paused {
        return Err(StatusCode::CONFLICT);
    }
    let step = mission
        .steps
        .iter()
        .find(|s| s.index == step_idx)
        .ok_or(StatusCode::NOT_FOUND)?;
    if step.status != StepStatus::AwaitingApproval {
        return Err(StatusCode::CONFLICT);
    }

    mission_manager.signal_cancel(&mission_id).await;

    service
        .fail_step(&mission_id, step_idx, "Rejected by admin")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    service
        .update_mission_status(&mission_id, &MissionStatus::Failed)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::OK)
}

async fn skip_step(
    State((service, db, mission_manager, ref workspace_root)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path((mission_id, step_idx)): Path<(String, u32)>,
) -> Result<StatusCode, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = service
        .is_team_admin(&user.user_id, &mission.team_id)
        .await
        .unwrap_or(false);
    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    if mission.status != MissionStatus::Paused {
        return Err(StatusCode::CONFLICT);
    }
    let step = mission
        .steps
        .iter()
        .find(|s| s.index == step_idx)
        .ok_or(StatusCode::NOT_FOUND)?;
    if step.status != StepStatus::AwaitingApproval {
        return Err(StatusCode::CONFLICT);
    }

    // Resume from next step
    let registration = match register_with_grace(&mission_manager, &mission_id).await {
        Some(registration) => registration,
        None => return Err(StatusCode::CONFLICT),
    };
    let run_id = registration.run_id.clone();
    let cancel_token = registration.cancel_token;
    if let Err(e) = service.set_mission_current_run(&mission_id, &run_id).await {
        mission_manager.complete(&mission_id).await;
        tracing::error!(
            "Failed to set current run for mission {}: {}",
            mission_id,
            e
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    if let Err(e) = service
        .update_step_status(&mission_id, step_idx, &StepStatus::Skipped)
        .await
    {
        mission_manager.complete(&mission_id).await;
        tracing::error!("Failed to skip step {} for {}: {}", step_idx, mission_id, e);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    let executor =
        MissionExecutor::new(db.clone(), mission_manager.clone(), workspace_root.clone());
    let mid = mission_id.clone();
    tokio::spawn(async move {
        if let Err(e) = executor.resume_mission(&mid, cancel_token, None).await {
            tracing::error!("Mission resume after skip failed: {}: {}", mid, e);
        }
    });

    Ok(StatusCode::OK)
}

// ─── Goal Handlers (AGE) ────────────────────────────────

async fn approve_goal(
    State((service, db, mission_manager, ref workspace_root)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path((mission_id, goal_id)): Path<(String, String)>,
    Json(body): Json<GoalActionRequest>,
) -> Result<StatusCode, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Precondition: mission must be Paused
    if mission.status != MissionStatus::Paused {
        return Err(StatusCode::CONFLICT);
    }

    let is_admin = service
        .is_team_admin(&user.user_id, &mission.team_id)
        .await
        .unwrap_or(false);
    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    validate_goal_awaiting_approval(&mission, &goal_id)?;

    let registration = match register_with_grace(&mission_manager, &mission_id).await {
        Some(registration) => registration,
        None => return Err(StatusCode::CONFLICT),
    };
    let run_id = registration.run_id.clone();
    let cancel_token = registration.cancel_token;
    if let Err(e) = service.set_mission_current_run(&mission_id, &run_id).await {
        mission_manager.complete(&mission_id).await;
        tracing::error!(
            "Failed to set current run for mission {}: {}",
            mission_id,
            e
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    if let Err(e) = service
        .update_goal_status(&mission_id, &goal_id, &GoalStatus::Pending)
        .await
    {
        mission_manager.complete(&mission_id).await;
        tracing::error!(
            "Failed to approve goal {} for {}: {}",
            goal_id,
            mission_id,
            e
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    // Mark approved checkpoint so executor doesn't pause again immediately.
    if let Err(e) = service.advance_mission_goal(&mission_id, &goal_id).await {
        service
            .update_goal_status(&mission_id, &goal_id, &GoalStatus::AwaitingApproval)
            .await
            .ok();
        mission_manager.complete(&mission_id).await;
        tracing::error!(
            "Failed to mark approved goal {} as current for {}: {}",
            goal_id,
            mission_id,
            e
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    let executor =
        MissionExecutor::new(db.clone(), mission_manager.clone(), workspace_root.clone());
    let mid = mission_id.clone();
    let feedback = body
        .feedback
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    tokio::spawn(async move {
        if let Err(e) = executor.resume_mission(&mid, cancel_token, feedback).await {
            tracing::error!("Mission resume failed: {}: {}", mid, e);
        }
    });

    Ok(StatusCode::OK)
}

async fn reject_goal(
    State((service, _, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path((mission_id, goal_id)): Path<(String, String)>,
    Json(_body): Json<GoalActionRequest>,
) -> Result<StatusCode, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Precondition: mission must be Paused
    if mission.status != MissionStatus::Paused {
        return Err(StatusCode::CONFLICT);
    }

    let is_admin = service
        .is_team_admin(&user.user_id, &mission.team_id)
        .await
        .unwrap_or(false);
    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    validate_goal_awaiting_approval(&mission, &goal_id)?;

    service
        .update_goal_status(&mission_id, &goal_id, &GoalStatus::Failed)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    service
        .update_mission_status(&mission_id, &MissionStatus::Failed)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

async fn pivot_goal(
    State((service, db, mission_manager, ref workspace_root)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path((mission_id, goal_id)): Path<(String, String)>,
    Json(body): Json<GoalActionRequest>,
) -> Result<StatusCode, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Precondition: mission must be Paused
    if mission.status != MissionStatus::Paused {
        return Err(StatusCode::CONFLICT);
    }

    let is_admin = service
        .is_team_admin(&user.user_id, &mission.team_id)
        .await
        .unwrap_or(false);
    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    validate_goal_awaiting_approval(&mission, &goal_id)?;

    let approach = body
        .alternative_approach
        .as_deref()
        .unwrap_or("manual pivot");
    service
        .set_goal_pivot(&mission_id, &goal_id, approach)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    service
        .update_goal_status(&mission_id, &goal_id, &GoalStatus::Pending)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let registration = match register_with_grace(&mission_manager, &mission_id).await {
        Some(registration) => registration,
        None => return Err(StatusCode::CONFLICT),
    };
    let run_id = registration.run_id.clone();
    let cancel_token = registration.cancel_token;
    if let Err(e) = service.set_mission_current_run(&mission_id, &run_id).await {
        mission_manager.complete(&mission_id).await;
        tracing::error!(
            "Failed to set current run for mission {}: {}",
            mission_id,
            e
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    let executor =
        MissionExecutor::new(db.clone(), mission_manager.clone(), workspace_root.clone());
    let mid = mission_id.clone();
    tokio::spawn(async move {
        if let Err(e) = executor.resume_mission(&mid, cancel_token, None).await {
            tracing::error!("Mission resume after pivot failed: {}: {}", mid, e);
        }
    });

    Ok(StatusCode::OK)
}

async fn abandon_goal_handler(
    State((service, db, mission_manager, ref workspace_root)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path((mission_id, goal_id)): Path<(String, String)>,
    Json(body): Json<GoalActionRequest>,
) -> Result<StatusCode, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Precondition: mission must be Paused
    if mission.status != MissionStatus::Paused {
        return Err(StatusCode::CONFLICT);
    }

    let is_admin = service
        .is_team_admin(&user.user_id, &mission.team_id)
        .await
        .unwrap_or(false);
    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    validate_goal_awaiting_approval(&mission, &goal_id)?;

    let reason = body.feedback.as_deref().unwrap_or("Abandoned by admin");
    service
        .abandon_goal(&mission_id, &goal_id, reason)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let registration = match register_with_grace(&mission_manager, &mission_id).await {
        Some(registration) => registration,
        None => return Err(StatusCode::CONFLICT),
    };
    let run_id = registration.run_id.clone();
    let cancel_token = registration.cancel_token;
    if let Err(e) = service.set_mission_current_run(&mission_id, &run_id).await {
        mission_manager.complete(&mission_id).await;
        tracing::error!(
            "Failed to set current run for mission {}: {}",
            mission_id,
            e
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    let executor =
        MissionExecutor::new(db.clone(), mission_manager.clone(), workspace_root.clone());
    let mid = mission_id.clone();
    tokio::spawn(async move {
        if let Err(e) = executor.resume_mission(&mid, cancel_token, None).await {
            tracing::error!("Mission resume after abandon failed: {}: {}", mid, e);
        }
    });

    Ok(StatusCode::OK)
}

// ─── Stream & Artifact Handlers ──────────────────────────

async fn stream_mission(
    State((service, _, mission_manager, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    headers: HeaderMap,
    Path(mission_id): Path<String>,
    Query(q): Query<StreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &mission.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    let last_event_id = q.last_event_id.or_else(|| {
        headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
    });

    let mission_status_str = serde_json::to_value(&mission.status)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "unknown".to_string());

    let (mut rx, history) = if let Some(pair) = mission_manager
        .subscribe_with_history(&mission_id, last_event_id)
        .await
    {
        pair
    } else if matches!(
        mission.status,
        MissionStatus::Draft
            | MissionStatus::Planned
            | MissionStatus::Paused
            | MissionStatus::Completed
            | MissionStatus::Failed
            | MissionStatus::Cancelled
    ) {
        // Mission is non-live/terminal: return one-shot done event
        // so clients can converge UI state without 404.
        let evt = StreamEvent::Done {
            status: mission_status_str.clone(),
            error: mission.error_message.clone(),
        };
        let stream = async_stream::stream! {
            let json = serde_json::to_string(&evt).unwrap_or_default();
            yield Ok(Event::default().event(evt.event_type()).data(json));
        }
        .boxed();
        return Ok(Sse::new(stream).keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        ));
    } else {
        // Mission claims to be live but no in-memory stream is registered.
        let evt = StreamEvent::Status {
            status: serde_json::json!({
                "type": "mission_stream_unavailable",
                "mission_status": mission_status_str,
            })
            .to_string(),
        };
        let stream = async_stream::stream! {
            let json = serde_json::to_string(&evt).unwrap_or_default();
            yield Ok(Event::default().event(evt.event_type()).data(json));
        }
        .boxed();
        return Ok(Sse::new(stream).keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        ));
    };

    let history_max = history.iter().map(|e| e.id).max().unwrap_or(0);
    let mut replay_watermark = last_event_id.unwrap_or(0).max(history_max);

    let stream = async_stream::stream! {
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
                    // Avoid replay overlap:
                    // events emitted between `subscribe()` and history snapshot can appear
                    // both in history and in live receiver queue.
                    if event.id > 0 && event.id <= replay_watermark {
                        continue;
                    }
                    if event.id > replay_watermark {
                        replay_watermark = event.id;
                    }
                    let is_done = event.event.is_done();
                    let json = serde_json::to_string(&event.event).unwrap_or_default();
                    let mut sse = Event::default().event(event.event.event_type()).data(json);
                    if event.id > 0 {
                        sse = sse.id(event.id.to_string());
                    }
                    yield Ok(sse);
                    if is_done { break; }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Err(_) => {
                    tracing::info!("Mission SSE stream deadline reached, closing for client reconnect");
                    break;
                }
            }
        }
    }
    .boxed();

    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    ))
}

async fn list_mission_events(
    State((service, _, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
    Query(q): Query<EventListQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &mission.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    let limit = q.limit.unwrap_or(500).clamp(1, 2000);
    let explicit_run_id = q.run_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let run_id = match explicit_run_id {
        Some(rid)
            if rid.eq_ignore_ascii_case("__all__")
                || rid.eq_ignore_ascii_case("all")
                || rid == "*" =>
        {
            None
        }
        Some(rid) => Some(rid),
        None => mission.current_run_id.as_deref(),
    };
    let events = service
        .list_mission_events(&mission_id, run_id, q.after_event_id, limit)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list mission events: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let mut value = serde_json::to_value(events).unwrap_or_default();
    fix_bson_dates(&mut value);
    Ok(Json(value))
}

async fn list_artifacts(
    State((service, db, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &mission.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    let mut items = service
        .list_mission_artifacts(&mission_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list artifacts: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let doc_service = DocumentService::new((*db).clone());
    for item in &mut items {
        let Some(doc_id) = item.archived_document_id.clone() else {
            continue;
        };
        if let Ok(doc_meta) = doc_service.get_metadata(&mission.team_id, &doc_id).await {
            item.archived_document_status = serde_json::to_value(doc_meta.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string));
        }
    }

    Ok(Json(serde_json::json!(items)))
}

async fn get_artifact(
    State((service, _, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(artifact_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let artifact = service
        .get_artifact(&artifact_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Check membership via the parent mission
    let mission = service
        .get_mission(&artifact.mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &mission.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(Json(serde_json::json!(artifact)))
}

async fn download_artifact(
    State((service, _, _, workspace_root)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(artifact_id): Path<String>,
) -> Result<axum::response::Response, StatusCode> {
    use axum::response::IntoResponse;

    let artifact = service
        .get_artifact(&artifact_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let mission = service
        .get_mission(&artifact.mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &mission.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    // If content is stored inline, return it directly
    if let Some(ref content) = artifact.content {
        let mime = artifact.mime_type.as_deref().unwrap_or("text/plain");
        return Ok((
            [
                (axum::http::header::CONTENT_TYPE, mime.to_string()),
                (
                    axum::http::header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", artifact.name),
                ),
            ],
            content.clone(),
        )
            .into_response());
    }

    // Otherwise read from workspace file_path.
    // Harden path checks to prevent traversal and workspace escape.
    let rel_path = artifact.file_path.as_deref().ok_or(StatusCode::NOT_FOUND)?;
    let rel = std::path::Path::new(rel_path);
    let is_safe_rel = !rel.is_absolute()
        && rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
    if !is_safe_rel {
        return Err(StatusCode::FORBIDDEN);
    }

    let ws_path = mission
        .workspace_path
        .as_deref()
        .ok_or(StatusCode::NOT_FOUND)?;
    let ws_canonical = tokio::fs::canonicalize(ws_path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if !ws_canonical.is_dir() {
        return Err(StatusCode::NOT_FOUND);
    }

    let workspace_root_canonical = tokio::fs::canonicalize(&workspace_root)
        .await
        .unwrap_or_else(|_| std::path::PathBuf::from(&workspace_root));
    if !ws_canonical.starts_with(&workspace_root_canonical) {
        tracing::warn!(
            "Reject artifact download outside workspace root: mission={}, workspace={:?}, root={:?}",
            mission.mission_id,
            ws_canonical,
            workspace_root_canonical
        );
        return Err(StatusCode::FORBIDDEN);
    }

    let full_path = ws_canonical.join(rel);
    let full_canonical = tokio::fs::canonicalize(&full_path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if !full_canonical.starts_with(&ws_canonical) || !full_canonical.is_file() {
        return Err(StatusCode::FORBIDDEN);
    }

    let bytes = tokio::fs::read(&full_canonical)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let mime = artifact
        .mime_type
        .as_deref()
        .unwrap_or("application/octet-stream");

    Ok((
        [
            (axum::http::header::CONTENT_TYPE, mime.to_string()),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", artifact.name),
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn archive_artifact_to_document(
    State((service, db, _, workspace_root)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(artifact_id): Path<String>,
    Json(body): Json<ArchiveArtifactRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let artifact = service
        .get_artifact(&artifact_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let mission = service
        .get_mission(&artifact.mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &mission.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    let doc_service = DocumentService::new((*db).clone());

    if let Some(ref existing_doc_id) = artifact.archived_document_id {
        if let Ok(existing_doc) = doc_service
            .get_metadata(&mission.team_id, existing_doc_id)
            .await
        {
            let status = serde_json::to_value(existing_doc.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "draft".to_string());
            let _ = service
                .set_artifact_document_link(&artifact.artifact_id, existing_doc_id, &status)
                .await;
            return Ok(Json(serde_json::json!({
                "artifact": artifact,
                "document": existing_doc,
                "created": false
            })));
        }
    }

    let file_bytes = read_artifact_bytes(&artifact, &mission, &workspace_root).await?;
    let document_name = body
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| artifact.name.clone());
    let folder_path = body
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let mime_type = artifact
        .mime_type
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            mime_guess::from_path(&document_name)
                .first_raw()
                .map(|m| m.to_string())
        })
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let category = body
        .category
        .unwrap_or_else(|| default_doc_category_for_artifact(&artifact.artifact_type));

    let created = doc_service
        .create_with_metadata(
            &mission.team_id,
            &user.user_id,
            &document_name,
            file_bytes,
            &mime_type,
            folder_path,
            DocumentOrigin::Agent,
            DocumentStatus::Draft,
            category,
            Vec::new(),
            Vec::new(),
            mission.session_id.clone(),
            Some(mission.mission_id.clone()),
            Some(mission.agent_id.clone()),
            None,
            Some("Archived from mission artifact".to_string()),
        )
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to archive artifact {} for mission {}: {}",
                artifact_id,
                mission.mission_id,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let created_summary: DocumentSummary = created.clone().into();
    let created_doc_id = created_summary.id.clone();
    service
        .set_artifact_document_link(&artifact.artifact_id, &created_doc_id, "draft")
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to link artifact {} with document {}: {}",
                artifact.artifact_id,
                created_doc_id,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let linked_artifact = service
        .get_artifact(&artifact.artifact_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(serde_json::json!({
        "artifact": linked_artifact,
        "document": created_summary,
        "created": true
    })))
}

async fn create_from_chat(
    State((service, db, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<CreateFromChatRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
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

    let create_req = CreateMissionRequest {
        agent_id: req.agent_id,
        goal: req.goal,
        context: None,
        route_mode: Some(MissionRouteMode::Mission),
        approval_policy: req.approval_policy,
        token_budget: req.token_budget,
        priority: None,
        step_timeout_seconds: None,
        step_max_retries: None,
        source_chat_session_id: Some(req.chat_session_id),
        execution_mode: None,
        execution_profile: None,
        attached_document_ids: vec![],
    };

    let mission = service
        .create_mission(&create_req, &team_id, &user.user_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create mission from chat: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(serde_json::json!({
        "mission_id": mission.mission_id,
        "status": mission.status,
    })))
}

// ── Phase 2: Mission document attachment routes ──

#[derive(serde::Deserialize)]
struct MissionDocumentIdsBody {
    document_ids: Vec<String>,
}

async fn attach_mission_documents(
    State((service, _, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
    Json(body): Json<MissionDocumentIdsBody>,
) -> Result<StatusCode, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &mission.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .attach_documents_to_mission(&mission_id, &body.document_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

async fn detach_mission_documents(
    State((service, _, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
    Json(body): Json<MissionDocumentIdsBody>,
) -> Result<StatusCode, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &mission.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .detach_documents_from_mission(&mission_id, &body.document_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn list_mission_documents(
    State((service, _, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &mission.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(Json(mission.attached_document_ids))
}
