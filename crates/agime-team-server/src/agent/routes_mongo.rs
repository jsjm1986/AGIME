//! Agent HTTP routes (MongoDB version)

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{sse::Sse, Json},
    routing::{delete, get, patch, post, put},
    Extension, Router,
};
use std::sync::Arc;

use crate::auth::middleware::UserContext;
use agime_team::models::{
    AgentExtensionConfig, AgentSkillConfig, AgentTask, CreateAgentRequest, CustomExtensionConfig,
    ListAgentsQuery, ListTasksQuery, PaginatedResponse, SubmitTaskRequest, TaskResult, TeamAgent,
    UpdateAgentRequest,
};
use agime_team::MongoDb;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use super::ai_describe::{
    AiDescribeError, AiDescribeService, BuiltinDescribeRequest, DescribeRequest, InsightsQuery,
    KNOWN_BUILTINS, KNOWN_BUILTIN_SKILLS,
};
use super::executor_mongo::TaskExecutor;
use super::rate_limit::RateLimiter;
use super::service_mongo::{
    AgentService, AvatarGovernanceEventPayload, AvatarGovernanceQueueItemPayload,
    AvatarInstanceSummary, AvatarWorkbenchSnapshotPayload, ServiceError,
};
use super::session_mongo::SessionListQuery;
use super::streamer::stream_task_results;
use super::task_manager::{StreamEvent, TaskManager};
use crate::config::Config;

/// Create agent router (MongoDB version)
pub fn router(db: Arc<MongoDb>) -> Router {
    let service = Arc::new(AgentService::new(db.clone()));
    let rate_limiter = Arc::new(RateLimiter::new(10, 60));
    let task_manager = Arc::new(TaskManager::new());

    // Background cleanup for stale tasks (stuck > 2 hours)
    {
        let tm = task_manager.clone();
        tokio::spawn(async move {
            let interval = std::time::Duration::from_secs(300);
            let max_age = std::time::Duration::from_secs(2 * 60 * 60);
            loop {
                tokio::time::sleep(interval).await;
                tm.cleanup_stale(max_age).await;
            }
        });
    }

    Router::new()
        .route("/avatar-instances", get(list_avatar_instances))
        .route(
            "/avatar-governance/{portal_id}",
            get(get_avatar_governance_state).put(update_avatar_governance_state),
        )
        .route(
            "/avatar-governance/{portal_id}/events",
            get(list_avatar_governance_events),
        )
        .route(
            "/avatar-governance/events",
            get(list_team_avatar_governance_events),
        )
        .route(
            "/avatar-governance/{portal_id}/queue",
            get(list_avatar_governance_queue),
        )
        .route(
            "/avatar-governance/{portal_id}/workbench",
            get(get_avatar_workbench_snapshot),
        )
        .route("/agents", post(create_agent))
        .route("/agents", get(list_agents))
        .route("/agents/{id}", get(get_agent))
        .route("/agents/{id}", put(update_agent))
        .route("/agents/{id}", delete(delete_agent))
        .route(
            "/agents/{id}/provision-from-template",
            post(provision_agent_from_template),
        )
        .route("/agents/{id}/clone", post(clone_agent_legacy))
        .route("/agents/{id}/access", put(update_agent_access))
        .route("/agents/{id}/extensions", put(update_agent_extensions))
        .route("/agents/{id}/extensions/custom", post(add_custom_extension))
        .route(
            "/agents/{id}/extensions/custom/{name}",
            patch(set_custom_extension_enabled).delete(remove_custom_extension),
        )
        .route(
            "/agents/{id}/extensions/reload",
            post(reload_agent_extensions),
        )
        .route("/agents/{id}/skills", put(update_agent_skills))
        .route("/agents/{id}/extensions/add-team", post(add_team_extension))
        .route("/agents/{id}/skills/add-team", post(add_team_skill))
        .route("/agents/{id}/skills/available", get(list_available_skills))
        .route("/agents/{id}/skills/{skill_id}", delete(remove_agent_skill))
        .route("/agents/{id}/sessions", get(list_sessions))
        .route("/sessions/{id}", get(get_session))
        .route("/sessions/{id}/archive", post(archive_session))
        .route("/tasks", post(submit_task))
        .route("/tasks", get(list_tasks))
        .route("/tasks/{id}", get(get_task))
        .route("/tasks/{id}/approve", post(approve_task))
        .route("/tasks/{id}/reject", post(reject_task))
        .route("/tasks/{id}/cancel", post(cancel_task))
        .route("/tasks/{id}/results", get(get_task_results))
        .route("/tasks/{id}/stream", get(stream_results))
        .with_state((service, db, rate_limiter, task_manager))
}

type AppState = (
    Arc<AgentService>,
    Arc<MongoDb>,
    Arc<RateLimiter>,
    Arc<TaskManager>,
);

#[derive(Debug, Deserialize)]
struct ProvisionFromTemplateRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    agent_domain: Option<String>,
    #[serde(default)]
    agent_role: Option<String>,
    #[serde(default)]
    owner_manager_agent_id: Option<String>,
    #[serde(default)]
    template_source_agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TeamScopedQuery {
    team_id: String,
}

#[derive(Debug, Deserialize)]
struct UpdateAvatarGovernanceRequest {
    #[serde(default)]
    state: Option<JsonValue>,
    #[serde(default)]
    config: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
struct AvatarGovernanceEventsQuery {
    team_id: String,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct TeamAvatarGovernanceEventsQuery {
    team_id: String,
    #[serde(default)]
    portal_id: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

fn build_dedicated_name(source_name: &str, requested_name: Option<&str>) -> String {
    let fallback = format!("{} (Dedicated)", source_name);
    let picked = requested_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(&fallback);
    let truncated: String = picked.chars().take(100).collect();
    let normalized = truncated.trim();
    if normalized.is_empty() {
        fallback.chars().take(100).collect()
    } else {
        normalized.to_string()
    }
}

// Agent handlers
async fn create_agent(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<CreateAgentRequest>,
) -> Result<Json<TeamAgent>, StatusCode> {
    let is_admin = service
        .is_team_admin(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    service.create_agent(req).await.map(Json).map_err(|e| {
        tracing::error!("Failed to create agent: {:?}", e);
        match e {
            ServiceError::Validation(_) => StatusCode::BAD_REQUEST,
            ServiceError::Database(_) | ServiceError::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    })
}

async fn list_agents(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<ListAgentsQuery>,
) -> Result<Json<PaginatedResponse<TeamAgent>>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    service.list_agents(query).await.map(Json).map_err(|e| {
        tracing::error!("Failed to list agents: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

async fn list_avatar_instances(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<Vec<AvatarInstanceSummary>>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .list_avatar_instance_projections(&query.team_id)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to list avatar instances: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn get_avatar_governance_state(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(portal_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<super::service_mongo::AvatarGovernanceStatePayload>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .get_avatar_governance_state(&query.team_id, &portal_id)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to get avatar governance state: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn update_avatar_governance_state(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(portal_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
    Json(req): Json<UpdateAvatarGovernanceRequest>,
) -> Result<Json<super::service_mongo::AvatarGovernanceStatePayload>, StatusCode> {
    let is_admin = service
        .is_team_admin(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .update_avatar_governance_state(
            &query.team_id,
            &portal_id,
            req.state,
            req.config,
            Some(&user.user_id),
            Some(&user.display_name),
        )
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to update avatar governance state: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn list_avatar_governance_events(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(portal_id): Path<String>,
    Query(query): Query<AvatarGovernanceEventsQuery>,
) -> Result<Json<Vec<AvatarGovernanceEventPayload>>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .list_avatar_governance_events(&query.team_id, &portal_id, query.limit.unwrap_or(120))
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to list avatar governance events: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn list_team_avatar_governance_events(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<TeamAvatarGovernanceEventsQuery>,
) -> Result<Json<Vec<AvatarGovernanceEventPayload>>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .list_team_avatar_governance_events(
            &query.team_id,
            query.portal_id.as_deref(),
            query.limit.unwrap_or(300),
        )
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to list team avatar governance events: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn list_avatar_governance_queue(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(portal_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<Vec<AvatarGovernanceQueueItemPayload>>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .list_avatar_governance_queue(&query.team_id, &portal_id)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to list avatar governance queue: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn get_avatar_workbench_snapshot(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(portal_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<AvatarWorkbenchSnapshotPayload>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .get_avatar_workbench_snapshot(&query.team_id, &portal_id)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to get avatar workbench snapshot: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn get_agent(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<Json<TeamAgent>, StatusCode> {
    let team_id = service
        .get_agent_team_id(&id)
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

    service
        .get_agent(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_agent(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAgentRequest>,
) -> Result<Json<TeamAgent>, StatusCode> {
    let team_id = service
        .get_agent_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = service
        .is_team_admin(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .update_agent(&id, req)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn delete_agent(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let team_id = service
        .get_agent_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = service
        .is_team_admin(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let deleted = service
        .delete_agent(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn clone_agent_legacy(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
    Json(req): Json<ProvisionFromTemplateRequest>,
) -> Result<Json<TeamAgent>, StatusCode> {
    provision_from_template_inner(service, user, id, req).await
}

async fn provision_agent_from_template(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
    Json(req): Json<ProvisionFromTemplateRequest>,
) -> Result<Json<TeamAgent>, StatusCode> {
    provision_from_template_inner(service, user, id, req).await
}

async fn provision_from_template_inner(
    service: Arc<AgentService>,
    user: UserContext,
    id: String,
    req: ProvisionFromTemplateRequest,
) -> Result<Json<TeamAgent>, StatusCode> {
    let team_id = service
        .get_agent_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = service
        .is_team_admin(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let source = service
        .get_agent_with_key(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let dedicated = service
        .is_dedicated_avatar_agent(&team_id, &source)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let target_domain = req
        .agent_domain
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let target_role = req
        .agent_role
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let owner_manager_agent_id = req
        .owner_manager_agent_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let allow_avatar_manager_clone = source.agent_domain.as_deref() == Some("digital_avatar")
        && source.agent_role.as_deref() == Some("manager")
        && target_domain == Some("digital_avatar")
        && target_role == Some("service")
        && owner_manager_agent_id == Some(id.as_str());
    if dedicated && !allow_avatar_manager_clone {
        tracing::warn!(
            "Blocked template provisioning from avatar-dedicated agent: team_id={}, agent_id={}",
            team_id,
            id
        );
        return Err(StatusCode::BAD_REQUEST);
    }

    let dedicated_name = build_dedicated_name(&source.name, req.name.as_deref());

    let create_req = CreateAgentRequest {
        team_id: source.team_id.clone(),
        name: dedicated_name,
        description: source.description.clone(),
        avatar: source.avatar.clone(),
        system_prompt: source.system_prompt.clone(),
        api_url: source.api_url.clone(),
        model: source.model.clone(),
        api_key: source.api_key.clone(),
        api_format: Some(source.api_format.to_string()),
        enabled_extensions: Some(source.enabled_extensions.clone()),
        custom_extensions: Some(source.custom_extensions.clone()),
        agent_domain: req.agent_domain.clone(),
        agent_role: req.agent_role.clone(),
        owner_manager_agent_id: req.owner_manager_agent_id.clone(),
        template_source_agent_id: req
            .template_source_agent_id
            .clone()
            .or_else(|| Some(id.clone())),
        allowed_groups: Some(source.allowed_groups.clone()),
        max_concurrent_tasks: Some(source.max_concurrent_tasks),
        thinking_enabled: Some(source.thinking_enabled),
        temperature: source.temperature,
        max_tokens: source.max_tokens,
        context_limit: source.context_limit,
        assigned_skills: Some(source.assigned_skills.clone()),
    };

    service
        .create_agent(create_req)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to provision template agent {}: {:?}", id, e);
            match e {
                ServiceError::Validation(_) => StatusCode::BAD_REQUEST,
                ServiceError::Database(_) | ServiceError::Internal(_) => {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
        })
}

// Task handlers
async fn submit_task(
    State((service, db, rate_limiter, task_manager)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<SubmitTaskRequest>,
) -> Result<Json<AgentTask>, StatusCode> {
    if !rate_limiter.check(&user.user_id).await {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    let is_member = service
        .is_team_member(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    // Enforce agent access mode (all/allowlist/denylist).
    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new((*db).clone())
            .get_user_group_ids(&req.team_id, &user.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let has_agent_access = service
        .check_agent_access(&req.agent_id, &user.user_id, &user_group_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !has_agent_access {
        return Err(StatusCode::FORBIDDEN);
    }

    // Check if auto-approve is applicable for chat tasks.
    // SECURITY: `auto_approve_chat` is an agent-level setting that can only be modified by
    // team admins/owners via the update_agent API. Ordinary members cannot change this flag.
    // When enabled, chat-type tasks skip the manual approval step for a smoother UX.
    let should_auto_approve = if req.task_type == agime_team::models::TaskType::Chat {
        service
            .get_agent(&req.agent_id)
            .await
            .ok()
            .flatten()
            .map(|a| a.auto_approve_chat)
            .unwrap_or(false)
    } else {
        false
    };

    let task = service.submit_task(&user.user_id, req).await.map_err(|e| {
        tracing::error!("Failed to submit task: {:?}", e);
        match e {
            ServiceError::Validation(_) => StatusCode::BAD_REQUEST,
            ServiceError::Database(_) | ServiceError::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    })?;

    // Auto-approve chat tasks when agent has auto_approve_chat enabled
    if should_auto_approve {
        let task_id = task.id.clone();
        match service.approve_task(&task_id, &user.user_id).await {
            Ok(Some(approved_task)) => {
                tracing::info!("Auto-approved chat task {} for agent", task_id);

                // Register and spawn execution
                let (cancel_token, _stream_tx) = task_manager.register(&task_id).await;
                let executor = TaskExecutor::new(db.clone(), task_manager.clone());
                let tid = task_id.clone();
                let svc = service.clone();
                let tm = task_manager.clone();
                tokio::spawn(async move {
                    if let Err(e) = executor.execute_task(&tid, cancel_token).await {
                        tracing::error!("Task execution failed: {}", e);
                        match svc.fail_task(&tid, &e.to_string()).await {
                            Ok(None) => tracing::warn!(
                                "fail_task: no update for task {} (already terminal?)",
                                tid
                            ),
                            Err(db_err) => {
                                tracing::error!("Failed to update task status: {}", db_err)
                            }
                            _ => {}
                        }
                        tm.broadcast(
                            &tid,
                            StreamEvent::Done {
                                status: "failed".to_string(),
                                error: Some(e.to_string()),
                            },
                        )
                        .await;
                        tm.complete(&tid).await;
                    }
                });

                return Ok(Json(approved_task));
            }
            Ok(None) => {
                tracing::warn!("Auto-approve: task {} not found after submit", task_id);
            }
            Err(e) => {
                tracing::warn!("Auto-approve failed for task {}: {}", task_id, e);
            }
        }
    }

    Ok(Json(task))
}

async fn list_tasks(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<ListTasksQuery>,
) -> Result<Json<PaginatedResponse<AgentTask>>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .list_tasks(query)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn get_task(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<Json<AgentTask>, StatusCode> {
    let team_id = service
        .get_task_team_id(&id)
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

    service
        .get_task(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn approve_task(
    State((service, db, _, task_manager)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<Json<AgentTask>, StatusCode> {
    let team_id = service
        .get_task_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = service
        .is_team_admin(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let task = service
        .approve_task(&id, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Register task for streaming
    let (cancel_token, _stream_tx) = task_manager.register(&id).await;

    // Spawn background task to execute
    let executor = TaskExecutor::new(db.clone(), task_manager.clone());
    let task_id = id.clone();
    let service_clone = service.clone();
    tokio::spawn(async move {
        if let Err(e) = executor.execute_task(&task_id, cancel_token).await {
            tracing::error!("Task execution failed: {}", e);
            match service_clone.fail_task(&task_id, &e.to_string()).await {
                Ok(None) => tracing::warn!(
                    "fail_task: no update for task {} (already terminal?)",
                    task_id
                ),
                Err(db_err) => {
                    tracing::error!("Failed to update task status to failed: {}", db_err)
                }
                _ => {}
            }
            // Send Done event so SSE subscribers know the task ended
            task_manager
                .broadcast(
                    &task_id,
                    StreamEvent::Done {
                        status: "failed".to_string(),
                        error: Some(e.to_string()),
                    },
                )
                .await;
            task_manager.complete(&task_id).await;
        }
    });

    Ok(Json(task))
}

async fn reject_task(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<Json<AgentTask>, StatusCode> {
    let team_id = service
        .get_task_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = service
        .is_team_admin(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .reject_task(&id, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn cancel_task(
    State((service, _, _, task_manager)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<Json<AgentTask>, StatusCode> {
    let team_id = service
        .get_task_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = service
        .is_team_admin(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    task_manager.cancel(&id).await;

    service
        .cancel_task(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn get_task_results(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TaskResult>>, StatusCode> {
    let team_id = service
        .get_task_team_id(&id)
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

    service
        .get_task_results(&id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn stream_results(
    State((service, _, _, task_manager)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<
    Sse<
        impl futures::stream::Stream<
            Item = Result<axum::response::sse::Event, std::convert::Infallible>,
        >,
    >,
    StatusCode,
> {
    // Verify user has access to this task's team
    let team_id = service
        .get_task_team_id(&id)
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

    Ok(stream_task_results(id, task_manager))
}

// === Access Control & Real-time Management ===

#[derive(Debug, Deserialize)]
struct UpdateAccessRequest {
    #[serde(default)]
    allowed_groups: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateExtensionsRequest {
    #[serde(default)]
    enabled_extensions: Option<Vec<AgentExtensionConfig>>,
    #[serde(default)]
    custom_extensions: Option<Vec<CustomExtensionConfig>>,
}

#[derive(Debug, Deserialize)]
struct UpdateSkillsRequest {
    #[serde(default)]
    assigned_skills: Option<Vec<AgentSkillConfig>>,
}

/// Update agent access control (allowed/denied groups)
async fn update_agent_access(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAccessRequest>,
) -> Result<Json<TeamAgent>, StatusCode> {
    let team_id = service
        .get_agent_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = service
        .is_team_admin(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .update_access_control(&id, req.allowed_groups)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// Update agent extensions (MCP real-time load/unload)
async fn update_agent_extensions(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
    Json(req): Json<UpdateExtensionsRequest>,
) -> Result<Json<TeamAgent>, StatusCode> {
    let team_id = service
        .get_agent_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = service
        .is_team_admin(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let update_req = UpdateAgentRequest {
        name: None,
        description: None,
        avatar: None,
        system_prompt: None,
        api_url: None,
        model: None,
        api_key: None,
        api_format: None,
        status: None,
        enabled_extensions: req.enabled_extensions,
        custom_extensions: req.custom_extensions,
        agent_domain: None,
        agent_role: None,
        owner_manager_agent_id: None,
        template_source_agent_id: None,
        allowed_groups: None,
        max_concurrent_tasks: None,
        temperature: None,
        max_tokens: None,
        context_limit: None,
        thinking_enabled: None,
        assigned_skills: None,
        auto_approve_chat: None,
    };

    service
        .update_agent(&id, update_req)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// Reload agent extensions (trigger re-initialization)
async fn reload_agent_extensions(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = service
        .get_agent_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = service
        .is_team_admin(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    // Mark agent as needing reload by updating status
    let now = chrono::Utc::now();
    let update_req = UpdateAgentRequest {
        name: None,
        description: None,
        avatar: None,
        system_prompt: None,
        api_url: None,
        model: None,
        api_key: None,
        api_format: None,
        status: Some(agime_team::models::AgentStatus::Idle),
        enabled_extensions: None,
        custom_extensions: None,
        agent_domain: None,
        agent_role: None,
        owner_manager_agent_id: None,
        template_source_agent_id: None,
        allowed_groups: None,
        max_concurrent_tasks: None,
        temperature: None,
        max_tokens: None,
        context_limit: None,
        thinking_enabled: None,
        assigned_skills: None,
        auto_approve_chat: None,
    };

    service
        .update_agent(&id, update_req)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Extensions reload triggered",
        "agentId": id,
        "reloadedAt": now.to_rfc3339()
    })))
}

/// Update agent skills configuration
async fn update_agent_skills(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSkillsRequest>,
) -> Result<Json<TeamAgent>, StatusCode> {
    let team_id = service
        .get_agent_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = service
        .is_team_admin(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let update_req = UpdateAgentRequest {
        name: None,
        description: None,
        avatar: None,
        system_prompt: None,
        api_url: None,
        model: None,
        api_key: None,
        api_format: None,
        status: None,
        enabled_extensions: None,
        custom_extensions: None,
        agent_domain: None,
        agent_role: None,
        owner_manager_agent_id: None,
        template_source_agent_id: None,
        allowed_groups: None,
        max_concurrent_tasks: None,
        temperature: None,
        max_tokens: None,
        context_limit: None,
        thinking_enabled: None,
        assigned_skills: req.assigned_skills,
        auto_approve_chat: None,
    };

    service
        .update_agent(&id, update_req)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// === Session Management ===

/// List sessions for an agent
async fn list_sessions(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(agent_id): Path<String>,
    Query(mut query): Query<SessionListQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = service
        .get_agent_team_id(&agent_id)
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

    query.team_id = team_id;
    query.agent_id = agent_id;

    let sessions = service
        .list_sessions(query)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({ "sessions": sessions })))
}

/// Get session details
async fn get_session(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &session.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(Json(serde_json::json!(session)))
}

/// Archive a session
async fn archive_session(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &session.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .archive_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

// === Team Extension Bridge ===

#[derive(Debug, Deserialize)]
struct AddTeamExtensionRequest {
    extension_id: String,
    team_id: String,
}

#[derive(Debug, Deserialize)]
struct AddCustomExtensionRequest {
    team_id: String,
    extension: CustomExtensionConfig,
}

#[derive(Debug, Deserialize)]
struct SetCustomExtensionEnabledRequest {
    enabled: bool,
}

/// Add a team shared extension to an agent's custom_extensions
async fn add_team_extension(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
    Json(req): Json<AddTeamExtensionRequest>,
) -> Result<Json<TeamAgent>, StatusCode> {
    // Verify user is admin of the team
    let is_admin = service
        .is_team_admin(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    // Verify agent belongs to this team
    let team_id = service
        .get_agent_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if team_id != req.team_id {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .add_team_extension_to_agent(&id, &req.extension_id, &req.team_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to add team extension to agent: {:?}", e);
            match &e {
                ServiceError::Internal(msg) if msg.contains("already exists") => {
                    StatusCode::CONFLICT
                }
                ServiceError::Validation(_) => StatusCode::BAD_REQUEST,
                ServiceError::Database(_) | ServiceError::Internal(_) => {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
        })?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// Add a custom MCP extension to an agent's custom_extensions.
async fn add_custom_extension(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
    Json(req): Json<AddCustomExtensionRequest>,
) -> Result<Json<TeamAgent>, StatusCode> {
    let is_admin = service
        .is_team_admin(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let team_id = service
        .get_agent_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if team_id != req.team_id {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .add_custom_extension_to_agent(&id, req.extension)
        .await
        .map_err(|e| {
            tracing::error!("Failed to add custom extension to agent: {:?}", e);
            match &e {
                ServiceError::Internal(msg) if msg.contains("already exists") => {
                    StatusCode::CONFLICT
                }
                ServiceError::Validation(_) => StatusCode::BAD_REQUEST,
                ServiceError::Database(_) | ServiceError::Internal(_) => {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
        })?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// Enable or disable a custom MCP extension on an agent.
async fn set_custom_extension_enabled(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path((id, name)): Path<(String, String)>,
    Query(query): Query<TeamScopedQuery>,
    Json(req): Json<SetCustomExtensionEnabledRequest>,
) -> Result<Json<TeamAgent>, StatusCode> {
    let is_admin = service
        .is_team_admin(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let team_id = service
        .get_agent_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if team_id != query.team_id {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .set_custom_extension_enabled(&id, &name, req.enabled)
        .await
        .map_err(|e| {
            tracing::error!("Failed to toggle custom extension on agent: {:?}", e);
            match &e {
                ServiceError::Validation(_) => StatusCode::BAD_REQUEST,
                ServiceError::Database(_) | ServiceError::Internal(_) => {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
        })?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// Remove a custom MCP extension from an agent.
async fn remove_custom_extension(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path((id, name)): Path<(String, String)>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<TeamAgent>, StatusCode> {
    let is_admin = service
        .is_team_admin(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let team_id = service
        .get_agent_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if team_id != query.team_id {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .remove_custom_extension_from_agent(&id, &name)
        .await
        .map_err(|e| {
            tracing::error!("Failed to remove custom extension from agent: {:?}", e);
            match &e {
                ServiceError::Validation(_) => StatusCode::BAD_REQUEST,
                ServiceError::Database(_) | ServiceError::Internal(_) => {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
        })?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// === Team Skill Bridge ===

#[derive(Debug, Deserialize)]
struct AddTeamSkillRequest {
    skill_id: String,
    team_id: String,
}

/// Add a team shared skill to an agent's assigned_skills
async fn add_team_skill(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
    Json(req): Json<AddTeamSkillRequest>,
) -> Result<Json<TeamAgent>, StatusCode> {
    let is_admin = service
        .is_team_admin(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let team_id = service
        .get_agent_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if team_id != req.team_id {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .add_team_skill_to_agent(&id, &req.skill_id, &req.team_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to add team skill to agent: {:?}", e);
            match e {
                ServiceError::Validation(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            }
        })?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// Remove a skill from an agent's assigned_skills
async fn remove_agent_skill(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path((id, skill_id)): Path<(String, String)>,
) -> Result<Json<TeamAgent>, StatusCode> {
    let team_id = service
        .get_agent_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = service
        .is_team_admin(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .remove_skill_from_agent(&id, &skill_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to remove skill from agent: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Debug, Deserialize)]
struct AvailableSkillsQuery {
    team_id: String,
}

/// List available team skills not yet assigned to the agent
async fn list_available_skills(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
    Query(query): Query<AvailableSkillsQuery>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    // Verify agent belongs to the requested team
    let team_id = service
        .get_agent_team_id(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if team_id != query.team_id {
        return Err(StatusCode::FORBIDDEN);
    }

    let is_member = service
        .is_team_member(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .list_available_skills(&id, &query.team_id)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to list available skills: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

// === AI Describe Routes ===

/// Map AiDescribeError to HTTP status code
fn ai_describe_error_to_status(e: &AiDescribeError) -> StatusCode {
    match e {
        AiDescribeError::NotConfigured => StatusCode::SERVICE_UNAVAILABLE,
        AiDescribeError::NotFound(_) => StatusCode::NOT_FOUND,
        AiDescribeError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        AiDescribeError::LlmError(_) => StatusCode::BAD_GATEWAY,
        AiDescribeError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

type AiDescribeState = (Arc<AiDescribeService>, Arc<AgentService>);

/// Create AI describe router (mounted at /api/teams)
pub fn ai_describe_router(db: Arc<MongoDb>, config: Arc<Config>) -> Router {
    let agent_service = Arc::new(AgentService::new(db.clone()));
    let ai_service = Arc::new(AiDescribeService::new(db, config, agent_service.clone()));

    Router::new()
        .route(
            "/{team_id}/extensions/{ext_id}/ai-describe",
            post(ai_describe_extension),
        )
        .route(
            "/{team_id}/skills/{skill_id}/ai-describe",
            post(ai_describe_skill),
        )
        .route(
            "/{team_id}/builtin-extensions/ai-describe",
            post(ai_describe_builtin_extension),
        )
        .route(
            "/{team_id}/builtin-extensions/ai-describe-batch",
            post(ai_describe_builtin_batch),
        )
        .route(
            "/{team_id}/builtin-skills/ai-describe",
            post(ai_describe_builtin_skill),
        )
        .route(
            "/{team_id}/builtin-skills/ai-describe-batch",
            post(ai_describe_builtin_skills_batch),
        )
        .route(
            "/{team_id}/skills/ai-describe-batch",
            post(ai_describe_skills_batch),
        )
        .route(
            "/{team_id}/extensions/ai-describe-batch",
            post(ai_describe_extensions_batch),
        )
        .route("/{team_id}/ai-insights", get(ai_insights))
        .with_state((ai_service, agent_service))
}

/// POST /api/teams/{team_id}/extensions/{ext_id}/ai-describe
async fn ai_describe_extension(
    State((ai_service, agent_service)): State<AiDescribeState>,
    Extension(user): Extension<UserContext>,
    Path((team_id, ext_id)): Path<(String, String)>,
    Json(req): Json<DescribeRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = agent_service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    match ai_service
        .describe_extension(&team_id, &ext_id, &req.lang)
        .await
    {
        Ok(resp) => Ok(Json(serde_json::json!({
            "description": resp.description,
            "lang": resp.lang,
            "generated_at": resp.generated_at.to_rfc3339(),
        }))),
        Err(e) => {
            tracing::error!("AI describe extension failed: {}", e);
            Err(ai_describe_error_to_status(&e))
        }
    }
}

/// POST /api/teams/{team_id}/skills/{skill_id}/ai-describe
async fn ai_describe_skill(
    State((ai_service, agent_service)): State<AiDescribeState>,
    Extension(user): Extension<UserContext>,
    Path((team_id, skill_id)): Path<(String, String)>,
    Json(req): Json<DescribeRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = agent_service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    match ai_service
        .describe_skill(&team_id, &skill_id, &req.lang)
        .await
    {
        Ok(resp) => Ok(Json(serde_json::json!({
            "description": resp.description,
            "lang": resp.lang,
            "generated_at": resp.generated_at.to_rfc3339(),
        }))),
        Err(e) => {
            tracing::error!("AI describe skill failed: {}", e);
            Err(ai_describe_error_to_status(&e))
        }
    }
}

/// GET /api/teams/{team_id}/ai-insights?lang=zh
async fn ai_insights(
    State((ai_service, agent_service)): State<AiDescribeState>,
    Extension(user): Extension<UserContext>,
    Path(team_id): Path<String>,
    Query(query): Query<InsightsQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = agent_service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    match ai_service.get_team_insights(&team_id, &query.lang).await {
        Ok(resp) => Ok(Json(serde_json::json!({
            "insights": resp.insights,
            "total": resp.total,
        }))),
        Err(e) => {
            tracing::error!("AI insights failed: {}", e);
            Err(ai_describe_error_to_status(&e))
        }
    }
}

/// POST /api/teams/{team_id}/builtin-extensions/ai-describe
async fn ai_describe_builtin_extension(
    State((ai_service, agent_service)): State<AiDescribeState>,
    Extension(user): Extension<UserContext>,
    Path(team_id): Path<String>,
    Json(req): Json<BuiltinDescribeRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = agent_service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    match ai_service.describe_builtin_extension(&team_id, &req).await {
        Ok(resp) => Ok(Json(serde_json::json!({
            "description": resp.description,
            "lang": resp.lang,
            "generated_at": resp.generated_at.to_rfc3339(),
        }))),
        Err(e) => {
            tracing::error!("AI describe builtin extension failed: {}", e);
            Err(ai_describe_error_to_status(&e))
        }
    }
}

/// POST /api/teams/{team_id}/builtin-extensions/ai-describe-batch
async fn ai_describe_builtin_batch(
    State((ai_service, agent_service)): State<AiDescribeState>,
    Extension(user): Extension<UserContext>,
    Path(team_id): Path<String>,
    Json(req): Json<DescribeRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = agent_service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    match ai_service
        .describe_all_builtin_extensions(&team_id, &req.lang)
        .await
    {
        Ok(results) => Ok(Json(serde_json::json!({
            "generated": results.len(),
            "total": KNOWN_BUILTINS.len(),
        }))),
        Err(e) => {
            tracing::error!("AI describe builtin batch failed: {}", e);
            Err(ai_describe_error_to_status(&e))
        }
    }
}

/// POST /api/teams/{team_id}/builtin-skills/ai-describe
async fn ai_describe_builtin_skill(
    State((ai_service, agent_service)): State<AiDescribeState>,
    Extension(user): Extension<UserContext>,
    Path(team_id): Path<String>,
    Json(req): Json<BuiltinDescribeRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = agent_service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    match ai_service.describe_builtin_skill(&team_id, &req).await {
        Ok(resp) => Ok(Json(serde_json::json!({
            "description": resp.description,
            "lang": resp.lang,
            "generated_at": resp.generated_at.to_rfc3339(),
        }))),
        Err(e) => {
            tracing::error!("AI describe builtin skill failed: {}", e);
            Err(ai_describe_error_to_status(&e))
        }
    }
}

/// POST /api/teams/{team_id}/builtin-skills/ai-describe-batch
async fn ai_describe_builtin_skills_batch(
    State((ai_service, agent_service)): State<AiDescribeState>,
    Extension(user): Extension<UserContext>,
    Path(team_id): Path<String>,
    Json(req): Json<DescribeRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = agent_service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    match ai_service
        .describe_all_builtin_skills(&team_id, &req.lang)
        .await
    {
        Ok(results) => Ok(Json(serde_json::json!({
            "generated": results.len(),
            "total": KNOWN_BUILTIN_SKILLS.len(),
        }))),
        Err(e) => {
            tracing::error!("AI describe builtin skills batch failed: {}", e);
            Err(ai_describe_error_to_status(&e))
        }
    }
}

/// POST /api/teams/{team_id}/skills/ai-describe-batch
async fn ai_describe_skills_batch(
    State((ai_service, agent_service)): State<AiDescribeState>,
    Extension(user): Extension<UserContext>,
    Path(team_id): Path<String>,
    Json(req): Json<DescribeRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = agent_service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    match ai_service.describe_all_skills(&team_id, &req.lang).await {
        Ok(results) => Ok(Json(serde_json::json!({
            "generated": results.len(),
        }))),
        Err(e) => {
            tracing::error!("AI describe skills batch failed: {}", e);
            Err(ai_describe_error_to_status(&e))
        }
    }
}

/// POST /api/teams/{team_id}/extensions/ai-describe-batch
async fn ai_describe_extensions_batch(
    State((ai_service, agent_service)): State<AiDescribeState>,
    Extension(user): Extension<UserContext>,
    Path(team_id): Path<String>,
    Json(req): Json<DescribeRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = agent_service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    match ai_service
        .describe_all_extensions(&team_id, &req.lang)
        .await
    {
        Ok(results) => Ok(Json(serde_json::json!({
            "generated": results.len(),
        }))),
        Err(e) => {
            tracing::error!("AI describe extensions batch failed: {}", e);
            Err(ai_describe_error_to_status(&e))
        }
    }
}
