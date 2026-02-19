//! Agent HTTP routes

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Json, sse::Sse},
    routing::{delete, get, post, put},
    Extension, Router,
};
use sqlx::SqlitePool;
use std::sync::Arc;

use crate::auth::middleware::UserContext;

use agime_team::models::{
    AgentTask, CreateAgentRequest, ListAgentsQuery, ListTasksQuery,
    PaginatedResponse, SubmitTaskRequest, TaskResult, TeamAgent,
    UpdateAgentRequest,
};

use super::full_executor::FullAgentExecutor;
use super::rate_limit::RateLimiter;
use super::service::{AgentService, ServiceError};
use super::streamer::stream_task_results;
use super::task_manager::TaskManager;

/// Create agent router
pub fn router(pool: Arc<SqlitePool>) -> Router {
    let service = Arc::new(AgentService::new(pool.clone()));
    // Rate limiter: 10 task submissions per minute per user
    let rate_limiter = Arc::new(RateLimiter::new(10, 60));
    // Task manager for tracking running tasks
    let task_manager = Arc::new(TaskManager::new());

    Router::new()
        .route("/agents", post(create_agent))
        .route("/agents", get(list_agents))
        .route("/agents/{id}", get(get_agent))
        .route("/agents/{id}", put(update_agent))
        .route("/agents/{id}", delete(delete_agent))
        .route("/tasks", post(submit_task))
        .route("/tasks", get(list_tasks))
        .route("/tasks/{id}", get(get_task))
        .route("/tasks/{id}/approve", post(approve_task))
        .route("/tasks/{id}/reject", post(reject_task))
        .route("/tasks/{id}/cancel", post(cancel_task))
        .route("/tasks/{id}/results", get(get_task_results))
        .route("/tasks/{id}/stream", get(stream_results))
        .route("/tasks/{id}/execute", post(execute_task))
        .with_state((service, pool, rate_limiter, task_manager))
}

// Type alias for state
type AppState = (Arc<AgentService>, Arc<SqlitePool>, Arc<RateLimiter>, Arc<TaskManager>);

// Agent handlers

async fn create_agent(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<CreateAgentRequest>,
) -> Result<Json<TeamAgent>, StatusCode> {
    // Check if user is admin of the team
    let is_admin = service
        .is_team_admin(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .create_agent(req)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to create agent: {:?}", e);
            match e {
                ServiceError::Validation(_) => StatusCode::BAD_REQUEST,
                ServiceError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            }
        })
}

async fn list_agents(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<ListAgentsQuery>,
) -> Result<Json<PaginatedResponse<TeamAgent>>, StatusCode> {
    // Check if user is member of the team
    let is_member = service
        .is_team_member(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .list_agents(query)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to list agents: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn get_agent(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<Json<TeamAgent>, StatusCode> {
    // Get agent's team_id and check permission
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
    // Get agent's team_id and check admin permission
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
    // Get agent's team_id and check admin permission
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

// Task handlers

async fn submit_task(
    State((service, _, rate_limiter, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<SubmitTaskRequest>,
) -> Result<Json<AgentTask>, StatusCode> {
    // Check rate limit
    if !rate_limiter.check(&user.user_id).await {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    // Check if user is member of the team
    let is_member = service
        .is_team_member(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .submit_task(&user.user_id, req)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to submit task: {:?}", e);
            match e {
                ServiceError::Validation(_) => StatusCode::BAD_REQUEST,
                ServiceError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            }
        })
}

async fn list_tasks(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<ListTasksQuery>,
) -> Result<Json<PaginatedResponse<AgentTask>>, StatusCode> {
    // Check if user is member of the team
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
    // Get task's team_id and check permission
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
    State((service, pool, _, task_manager)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<Json<AgentTask>, StatusCode> {
    // Get task's team_id and check admin permission
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

    // IMPORTANT: Register task BEFORE returning, so frontend can immediately subscribe to SSE
    // This fixes the race condition where frontend calls /stream before task is registered
    let (_cancel_token, _stream_tx) = task_manager.register(&id).await;

    // Auto-execute the task after approval using FullAgentExecutor
    let task_id = id.clone();
    let pool_clone = pool.clone();
    let task_manager_clone = task_manager.clone();
    tokio::spawn(async move {
        let executor = FullAgentExecutor::new(pool_clone, task_manager_clone.clone());
        if let Err(e) = executor.execute_task(&task_id).await {
            tracing::error!("Failed to execute task {}: {:?}", task_id, e);
            // Broadcast error event through task manager
            task_manager_clone.broadcast(&task_id, super::task_manager::StreamEvent::Done {
                status: "failed".to_string(),
                error: Some(e.to_string()),
            }).await;
        }

        // Mark task as completed and remove from tracking
        task_manager_clone.complete(&task_id).await;
    });

    Ok(Json(task))
}

async fn reject_task(
    State((service, _, _, _)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<Json<AgentTask>, StatusCode> {
    // Get task's team_id and check admin permission
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
    // Get task's team_id and check admin permission
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

    // Try to cancel running task via task manager
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
    // Get task's team_id and check permission
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

// SSE streaming handler
async fn stream_results(
    State((_, _, _, task_manager)): State<AppState>,
    Path(id): Path<String>,
) -> Sse<impl futures::stream::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    stream_task_results(id, task_manager)
}

// Task execution handler using FullAgentExecutor with MCP and Skills support
async fn execute_task(
    State((service, pool, _, task_manager)): State<AppState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    // Get task's team_id and check admin permission
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

    let executor = FullAgentExecutor::new(pool, task_manager);

    executor
        .execute_task(&id)
        .await
        .map(|_| StatusCode::ACCEPTED)
        .map_err(|e| {
            tracing::error!("Failed to execute task: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}
