//! Mission API routes (Phase 2 - Mission Track)
//!
//! Mounted at `/api/team/agent/mission`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        Json,
    },
    routing::{delete, get, post},
    Extension, Router,
};
use futures::stream::Stream;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use crate::auth::middleware::UserContext;
use agime_team::MongoDb;

use super::mission_executor::MissionExecutor;
use super::mission_manager::MissionManager;
use super::mission_mongo::{
    CreateFromChatRequest, CreateMissionRequest, GoalActionRequest, GoalStatus, ListMissionsQuery,
    MissionStatus, StepActionRequest, StepStatus,
};
use super::service_mongo::AgentService;
use super::task_manager::StreamEvent;

type MissionState = (Arc<AgentService>, Arc<MongoDb>, Arc<MissionManager>, String);

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
        .route("/missions/{id}/pause", post(pause_mission))
        .route("/missions/{id}/cancel", post(cancel_mission))
        .route("/missions/{id}/steps/{idx}/approve", post(approve_step))
        .route("/missions/{id}/steps/{idx}/reject", post(reject_step))
        .route("/missions/{id}/steps/{idx}/skip", post(skip_step))
        .route("/missions/{id}/stream", get(stream_mission))
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
    State((service, _, _, _)): State<MissionState>,
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

    let mission = service
        .create_mission(&req, &team_id, &user.user_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create mission: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(serde_json::json!({
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

    Ok(Json(serde_json::json!({
        "mission_id": mission.mission_id,
        "team_id": mission.team_id,
        "agent_id": mission.agent_id,
        "creator_id": mission.creator_id,
        "goal": mission.goal,
        "context": mission.context,
        "status": mission.status,
        "approval_policy": mission.approval_policy,
        "steps": mission.steps,
        "current_step": mission.current_step,
        "session_id": mission.session_id,
        "token_budget": mission.token_budget,
        "total_tokens_used": mission.total_tokens_used,
        "priority": mission.priority,
        "error_message": mission.error_message,
        "final_summary": mission.final_summary,
        "execution_mode": mission.execution_mode,
        "goal_tree": mission.goal_tree,
        "current_goal_id": mission.current_goal_id,
        "total_pivots": mission.total_pivots,
        "total_abandoned": mission.total_abandoned,
        "created_at": mission.created_at.to_chrono().to_rfc3339(),
        "updated_at": mission.updated_at.to_chrono().to_rfc3339(),
    })))
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

    if mission.creator_id != user.user_id {
        let is_admin = service
            .is_team_admin(&user.user_id, &mission.team_id)
            .await
            .unwrap_or(false);
        if !is_admin {
            return Err(StatusCode::FORBIDDEN);
        }
    }

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

    if mission.creator_id != user.user_id {
        let is_admin = service
            .is_team_admin(&user.user_id, &mission.team_id)
            .await
            .unwrap_or(false);
        if !is_admin {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    let (cancel_token, _) = match mission_manager.register(&mission_id).await {
        Some(pair) => pair,
        None => return Err(StatusCode::CONFLICT),
    };

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

    if mission.creator_id != user.user_id {
        let is_admin = service
            .is_team_admin(&user.user_id, &mission.team_id)
            .await
            .unwrap_or(false);
        if !is_admin {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    service
        .update_mission_status(&mission_id, &MissionStatus::Paused)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    mission_manager.cancel(&mission_id).await;
    Ok(StatusCode::OK)
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

    mission_manager.cancel(&mission_id).await;
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

    service
        .approve_step(&mission_id, step_idx, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Resume execution
    let (cancel_token, _) = match mission_manager.register(&mission_id).await {
        Some(pair) => pair,
        None => return Err(StatusCode::CONFLICT),
    };

    let executor =
        MissionExecutor::new(db.clone(), mission_manager.clone(), workspace_root.clone());
    let mid = mission_id.clone();
    // TODO: pass body.feedback to resume_mission when supported
    tokio::spawn(async move {
        if let Err(e) = executor.resume_mission(&mid, cancel_token).await {
            tracing::error!("Mission resume failed: {}: {}", mid, e);
        }
    });

    Ok(StatusCode::OK)
}

async fn reject_step(
    State((service, _, _, _)): State<MissionState>,
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

    service
        .update_step_status(&mission_id, step_idx, &StepStatus::Skipped)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Resume from next step
    let (cancel_token, _) = match mission_manager.register(&mission_id).await {
        Some(pair) => pair,
        None => return Err(StatusCode::CONFLICT),
    };

    let executor =
        MissionExecutor::new(db.clone(), mission_manager.clone(), workspace_root.clone());
    let mid = mission_id.clone();
    tokio::spawn(async move {
        if let Err(e) = executor.resume_mission(&mid, cancel_token).await {
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
    Json(_body): Json<GoalActionRequest>,
) -> Result<StatusCode, StatusCode> {
    let mission = service
        .get_mission(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Precondition: mission must be Paused or Planned
    if mission.status != MissionStatus::Paused && mission.status != MissionStatus::Planned {
        return Err(StatusCode::CONFLICT);
    }

    let is_admin = service
        .is_team_admin(&user.user_id, &mission.team_id)
        .await
        .unwrap_or(false);
    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    // Validate goal exists and is in awaiting_approval status
    match mission.goal_tree {
        Some(ref goals) => {
            let goal = goals.iter().find(|g| g.goal_id == goal_id);
            match goal {
                Some(g) if g.status != GoalStatus::AwaitingApproval => {
                    return Err(StatusCode::CONFLICT)
                }
                None => return Err(StatusCode::NOT_FOUND),
                _ => {}
            }
        }
        None => return Err(StatusCode::NOT_FOUND),
    }

    service
        .update_goal_status(&mission_id, &goal_id, &GoalStatus::Pending)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let (cancel_token, _) = match mission_manager.register(&mission_id).await {
        Some(pair) => pair,
        None => return Err(StatusCode::CONFLICT),
    };

    let executor =
        MissionExecutor::new(db.clone(), mission_manager.clone(), workspace_root.clone());
    let mid = mission_id.clone();
    tokio::spawn(async move {
        if let Err(e) = executor.resume_mission(&mid, cancel_token).await {
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

    // Validate goal exists in goal_tree
    match mission.goal_tree {
        Some(ref goals) => {
            if !goals.iter().any(|g| g.goal_id == goal_id) {
                return Err(StatusCode::NOT_FOUND);
            }
        }
        None => return Err(StatusCode::NOT_FOUND),
    }

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

    // Validate goal exists in goal_tree
    match mission.goal_tree {
        Some(ref goals) => {
            if !goals.iter().any(|g| g.goal_id == goal_id) {
                return Err(StatusCode::NOT_FOUND);
            }
        }
        None => return Err(StatusCode::NOT_FOUND),
    }

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

    let (cancel_token, _) = match mission_manager.register(&mission_id).await {
        Some(pair) => pair,
        None => return Err(StatusCode::CONFLICT),
    };

    let executor =
        MissionExecutor::new(db.clone(), mission_manager.clone(), workspace_root.clone());
    let mid = mission_id.clone();
    tokio::spawn(async move {
        if let Err(e) = executor.resume_mission(&mid, cancel_token).await {
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

    // Validate goal exists in goal_tree
    match mission.goal_tree {
        Some(ref goals) => {
            if !goals.iter().any(|g| g.goal_id == goal_id) {
                return Err(StatusCode::NOT_FOUND);
            }
        }
        None => return Err(StatusCode::NOT_FOUND),
    }

    let reason = body.feedback.as_deref().unwrap_or("Abandoned by admin");
    service
        .abandon_goal(&mission_id, &goal_id, reason)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let (cancel_token, _) = match mission_manager.register(&mission_id).await {
        Some(pair) => pair,
        None => return Err(StatusCode::CONFLICT),
    };

    let executor =
        MissionExecutor::new(db.clone(), mission_manager.clone(), workspace_root.clone());
    let mid = mission_id.clone();
    tokio::spawn(async move {
        if let Err(e) = executor.resume_mission(&mid, cancel_token).await {
            tracing::error!("Mission resume after abandon failed: {}: {}", mid, e);
        }
    });

    Ok(StatusCode::OK)
}

// ─── Stream & Artifact Handlers ──────────────────────────

async fn stream_mission(
    State((service, _, mission_manager, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
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

    let mut rx = mission_manager
        .subscribe(&mission_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let stream = async_stream::stream! {
        let sse_lifetime_secs = std::env::var("TEAM_SSE_MAX_LIFETIME_SECS")
            .ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(2 * 60 * 60);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(sse_lifetime_secs);

        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(event)) => {
                    let is_done = event.is_done();
                    let json = serde_json::to_string(&event).unwrap_or_default();
                    yield Ok(Event::default().event(event.event_type()).data(json));
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
    };

    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    ))
}

async fn list_artifacts(
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

    let items = service
        .list_mission_artifacts(&mission_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list artifacts: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

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

async fn create_from_chat(
    State((service, _, _, _)): State<MissionState>,
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

    let create_req = CreateMissionRequest {
        agent_id: req.agent_id,
        goal: req.goal,
        context: None,
        approval_policy: req.approval_policy,
        token_budget: req.token_budget,
        priority: None,
        source_chat_session_id: Some(req.chat_session_id),
        execution_mode: None,
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
