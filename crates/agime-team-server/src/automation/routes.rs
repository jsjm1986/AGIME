use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{delete, get, patch, post},
    Extension, Router,
};
use reqwest::Url;
use serde_json::json;

use agime_team::MongoDb;

use crate::{agent::service_mongo::AgentService, auth::middleware::UserContext};

use super::{
    contract::{assess_draft_publish_readiness, derive_builder_sync_payload},
    models::{
        CreateIntegrationRequest, CreateProjectRequest, CreateScheduleRequest,
        CreateTaskDraftRequest, SaveModuleRequest, StartRunRequest, TestIntegrationRequest,
        UpdateScheduleRequest, UpdateTaskDraftRequest,
    },
    runner::AutomationRunner,
    scheduler::compute_next_run_at,
    service::{
        artifact_to_compact_value, integration_to_value, module_to_compact_value, module_to_value,
        run_to_value, schedule_to_value, task_draft_to_compact_value, task_draft_to_value,
        AutomationService,
    },
};

type AutomationState = (
    Arc<AutomationService>,
    Arc<AutomationRunner>,
    Arc<AgentService>,
);

#[derive(Debug, Clone, serde::Deserialize)]
struct TeamScopedQuery {
    team_id: String,
}

pub fn router(
    db: Arc<MongoDb>,
    chat_manager: Arc<crate::agent::ChatManager>,
    workspace_root: String,
) -> Router {
    let service = Arc::new(AutomationService::new(db.clone()));
    let runner = Arc::new(AutomationRunner::new(
        db.clone(),
        chat_manager,
        workspace_root,
    ));
    let agent_service = Arc::new(AgentService::new(db));
    Router::new()
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{project_id}", delete(delete_project))
        .route(
            "/projects/{project_id}/integrations",
            get(list_integrations).post(create_integration),
        )
        .route(
            "/integrations/{integration_id}/test",
            post(test_integration),
        )
        .route(
            "/projects/{project_id}/tasks",
            get(list_task_drafts).post(create_task_draft),
        )
        .route(
            "/projects/{project_id}/app-drafts",
            get(list_task_drafts).post(create_task_draft),
        )
        .route(
            "/tasks/{draft_id}",
            get(get_task_draft).patch(update_task_draft),
        )
        .route(
            "/app-drafts/{draft_id}",
            get(get_task_draft).patch(update_task_draft),
        )
        .route(
            "/tasks/{draft_id}/builder-session",
            post(ensure_builder_session),
        )
        .route(
            "/app-drafts/{draft_id}/builder-session",
            post(ensure_builder_session),
        )
        .route("/tasks/{draft_id}/sync-builder", post(sync_builder_draft))
        .route(
            "/app-drafts/{draft_id}/sync-builder",
            post(sync_builder_draft),
        )
        .route("/tasks/{draft_id}/probe", post(probe_task_draft))
        .route("/app-drafts/{draft_id}/probe", post(probe_task_draft))
        .route("/tasks/{draft_id}/module", post(save_module))
        .route("/app-drafts/{draft_id}/publish", post(save_module))
        .route("/projects/{project_id}/modules", get(list_modules))
        .route("/projects/{project_id}/apps", get(list_modules))
        .route("/modules/{module_id}", delete(delete_module))
        .route("/apps/{module_id}", delete(delete_module))
        .route("/apps/{module_id}", get(get_app_runtime))
        .route("/apps/{module_id}/runtime", get(get_app_runtime))
        .route("/modules/{module_id}/runs", post(start_module_run))
        .route("/apps/{module_id}/runs", post(start_module_run))
        .route("/projects/{project_id}/runs", get(list_runs))
        .route("/projects/{project_id}/artifacts", get(list_artifacts))
        .route("/projects/{project_id}/schedules", get(list_schedules))
        .route("/modules/{module_id}/schedules", post(create_schedule))
        .route("/apps/{module_id}/schedules", post(create_schedule))
        .route(
            "/schedules/{schedule_id}",
            patch(update_schedule).delete(delete_schedule),
        )
        .with_state((service, runner, agent_service))
}

async fn ensure_team_member(
    service: &AgentService,
    user: &UserContext,
    team_id: &str,
) -> Result<(), StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if is_member {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

async fn list_projects(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let items = service
        .list_projects(&team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "projects": items })))
}

async fn create_project(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    ensure_team_member(&agent_service, &user, &req.team_id).await?;
    let project = service
        .create_project(req, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let item = service
        .list_projects(&project.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_iter()
        .find(|p| p.get("project_id").and_then(|v| v.as_str()) == Some(project.project_id.as_str()))
        .unwrap_or_else(|| json!({}));
    Ok(Json(json!({ "project": item })))
}

async fn delete_project(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(project_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let draft_session_ids: Vec<String> = service
        .list_task_drafts(&team_id, &project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_iter()
        .filter_map(|draft| {
            draft
                .get("builder_session_id")
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        })
        .collect();
    let modules = service
        .list_modules(&team_id, &project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut session_ids: Vec<String> = draft_session_ids;
    let mut module_ids: Vec<String> = Vec::new();
    for module in modules {
        if let Some(module_id) = module
            .get("module_id")
            .and_then(|value| value.as_str())
            .map(ToString::to_string)
        {
            module_ids.push(module_id);
        }
        if let Some(runtime_session_id) = module
            .get("runtime_session_id")
            .and_then(|value| value.as_str())
        {
            session_ids.push(runtime_session_id.to_string());
        }
        if let Some(builder_session_id) = module
            .get("latest_builder_session_id")
            .and_then(|value| value.as_str())
        {
            session_ids.push(builder_session_id.to_string());
        }
    }
    for module_id in module_ids {
        for run in service
            .list_runs_for_module(&team_id, &module_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            if let Some(session_id) = run.session_id {
                session_ids.push(session_id);
            }
        }
    }
    session_ids.sort();
    session_ids.dedup();
    let deleted = service
        .delete_project(&team_id, &project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if deleted {
        for session_id in session_ids {
            let _ = agent_service.delete_session(&session_id).await;
        }
    }
    Ok(Json(json!({ "deleted": deleted })))
}

async fn list_integrations(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(project_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let items = service
        .list_integrations(&team_id, &project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "integrations": items })))
}

async fn create_integration(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(project_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
    Json(mut req): Json<CreateIntegrationRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    req.team_id = team_id.clone();
    req.project_id = project_id;
    let integration = service
        .create_integration(req, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        json!({ "integration": integration_to_value(&integration) }),
    ))
}

async fn test_integration(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(integration_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
    Json(req): Json<TestIntegrationRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let integration = service
        .get_integration(&team_id, &integration_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let Some(mut base) = integration.base_url.clone() else {
        let updated = service
            .update_integration_test_result(
                &team_id,
                &integration_id,
                super::models::ConnectionStatus::Failed,
                Some("缺少 base_url".to_string()),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        return Ok(Json(
            json!({ "integration": updated.map(|value| integration_to_value(&value)) }),
        ));
    };
    if let Some(path) = req.probe_path.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }) {
        base = format!("{}{}", base.trim_end_matches('/'), path);
    }
    let parsed = Url::parse(&base).map_err(|_| StatusCode::BAD_REQUEST)?;
    let client = reqwest::Client::new();
    let result = client.get(parsed).send().await;
    let (status, message) = match result {
        Ok(response) => (
            super::models::ConnectionStatus::Success,
            Some(format!("HTTP {}", response.status().as_u16())),
        ),
        Err(error) => (
            super::models::ConnectionStatus::Failed,
            Some(error.to_string()),
        ),
    };
    let updated = service
        .update_integration_test_result(&team_id, &integration_id, status, message)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        json!({ "integration": updated.map(|value| integration_to_value(&value)) }),
    ))
}

async fn list_task_drafts(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(project_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let items = service
        .list_task_drafts(&team_id, &project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "tasks": items.clone(), "app_drafts": items })))
}

async fn create_task_draft(
    State((service, runner, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(project_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
    Json(mut req): Json<CreateTaskDraftRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let create_builder_session = req.create_builder_session.unwrap_or(false);
    req.team_id = team_id;
    req.project_id = project_id;
    let draft = service
        .create_task_draft(req, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let builder_session_id = if create_builder_session {
        Some(
            runner
                .ensure_builder_session_for_draft(&draft.team_id, &draft, &user.user_id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        )
    } else {
        None
    };
    let latest = service
        .get_task_draft(&draft.team_id, &draft.draft_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .unwrap_or(draft);
    let draft_value = task_draft_to_compact_value(&latest);
    Ok(Json(json!({
        "task": draft_value.clone(),
        "app_draft": draft_value,
        "builder_session_id": builder_session_id,
    })))
}

async fn get_task_draft(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(draft_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let draft = service
        .get_task_draft(&team_id, &draft_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let draft_value = task_draft_to_value(&draft);
    Ok(Json(
        json!({ "task": draft_value.clone(), "app_draft": draft_value }),
    ))
}

async fn update_task_draft(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(draft_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
    Json(req): Json<UpdateTaskDraftRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let draft = service
        .update_task_draft(&team_id, &draft_id, req)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let draft_value = task_draft_to_compact_value(&draft);
    Ok(Json(
        json!({ "task": draft_value.clone(), "app_draft": draft_value }),
    ))
}

async fn probe_task_draft(
    State((service, runner, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(draft_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let draft = service
        .get_task_draft(&team_id, &draft_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let session_id = runner
        .start_probe_for_draft(&team_id, &draft, &user.user_id)
        .await
        .map_err(|e| {
            tracing::error!("automation probe failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let updated = service
        .get_task_draft(&team_id, &draft_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let draft_value = task_draft_to_compact_value(&updated);
    Ok(Json(json!({
        "task": draft_value.clone(),
        "app_draft": draft_value,
        "builder_session_id": session_id,
    })))
}

async fn ensure_builder_session(
    State((service, runner, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(draft_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let draft = service
        .get_task_draft(&team_id, &draft_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let session_id = runner
        .ensure_builder_session_for_draft(&team_id, &draft, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let updated = service
        .get_task_draft(&team_id, &draft_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(json!({
        "task": task_draft_to_compact_value(&updated),
        "builder_session_id": session_id,
    })))
}

async fn sync_builder_draft(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(draft_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let draft = service
        .get_task_draft(&team_id, &draft_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let session_id = draft
        .builder_session_id
        .clone()
        .ok_or(StatusCode::CONFLICT)?;
    let session = agent_service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let integrations = service
        .get_integrations_by_ids(&team_id, &draft.integration_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if session.is_processing {
        let draft_value = task_draft_to_compact_value(&draft);
        return Ok(Json(json!({
            "task": draft_value.clone(),
            "app_draft": draft_value,
            "sync_state": "processing",
        })));
    }

    let final_status = session
        .last_execution_status
        .clone()
        .unwrap_or_else(|| "completed".to_string());
    let sync_payload = derive_builder_sync_payload(
        &session_id,
        &session.messages_json,
        session.last_message_preview.as_deref(),
        &final_status,
        draft.status.clone(),
        &integrations,
    );

    service
        .complete_task_draft_probe(
            &team_id,
            &draft_id,
            sync_payload.status,
            sync_payload.probe_report,
            sync_payload.candidate_plan,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let updated = service
        .get_task_draft(&team_id, &draft_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let draft_value = task_draft_to_compact_value(&updated);
    Ok(Json(json!({
        "task": draft_value.clone(),
        "app_draft": draft_value,
        "sync_state": "updated",
    })))
}

async fn save_module(
    State((service, runner, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(draft_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
    Json(req): Json<SaveModuleRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id)
        .await
        .map_err(|status| (status, Json(json!({ "error": "无权发布当前项目应用。" }))))?;
    let draft = service
        .get_task_draft(&team_id, &draft_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "读取 Agentify 草稿失败。" })),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "找不到要发布的 Agentify 草稿。" })),
        ))?;
    let readiness = assess_draft_publish_readiness(&draft);
    if !readiness.ready {
        let message = if readiness.issues.is_empty() {
            "发布前校验未通过。".to_string()
        } else {
            format!("发布前校验未通过：{}", readiness.issues.join("；"))
        };
        return Err((
            StatusCode::CONFLICT,
            Json(json!({
                "error": message,
                "publish_readiness": readiness,
            })),
        ));
    }
    let module = service
        .create_module_from_draft(&team_id, &draft, req, &user.user_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "创建已发布 Agent 失败。" })),
            )
        })?;
    let runtime_session_id = runner
        .ensure_runtime_session_for_module(&team_id, &module, &user.user_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "初始化已发布 Agent 的运行时会话失败。" })),
            )
        })?;
    let updated_module = service
        .update_module_runtime_session(&team_id, &module.module_id, &runtime_session_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "写入已发布 Agent 运行时信息失败。" })),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "已发布 Agent 创建后未能重新载入。" })),
        ))?;
    let app_value = module_to_compact_value(&updated_module);
    Ok(Json(
        json!({ "module": app_value.clone(), "app": app_value }),
    ))
}

async fn list_modules(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(project_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let items = service
        .list_modules(&team_id, &project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "modules": items.clone(), "apps": items })))
}

async fn get_app_runtime(
    State((service, runner, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(module_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let module = service
        .get_module(&team_id, &module_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let runtime_session_id = runner
        .ensure_runtime_session_for_module(&team_id, &module, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let app = service
        .get_module(&team_id, &module_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let runs = service
        .list_runs_for_module(&team_id, &module_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_iter()
        .map(|run| run_to_value(&run))
        .collect::<Vec<_>>();
    let artifacts = service
        .list_artifacts_for_module(&team_id, &module_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_iter()
        .map(|artifact| artifact_to_compact_value(&artifact))
        .collect::<Vec<_>>();
    let schedules = service
        .list_schedules_for_module(&team_id, &module_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_iter()
        .map(|schedule| schedule_to_value(&schedule))
        .collect::<Vec<_>>();
    let app_value = module_to_value(&app);
    Ok(Json(json!({
        "app": app_value.clone(),
        "runtime_session_id": runtime_session_id,
        "recent_runs": runs,
        "recent_artifacts": artifacts,
        "active_schedules": schedules,
    })))
}

async fn start_module_run(
    State((service, runner, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(module_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
    Json(req): Json<StartRunRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let module = service
        .get_module(&team_id, &module_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let run = runner
        .start_module_run(&team_id, &module, &user.user_id, req.mode, None, None)
        .await
        .map_err(|e| {
            tracing::error!("automation run failed to start: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(
        json!({ "run": run_to_value(&run), "app_id": module.module_id }),
    ))
}

async fn delete_module(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(module_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let module = service
        .get_module(&team_id, &module_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let mut session_ids = Vec::new();
    if let Some(runtime_session_id) = module.runtime_session_id.clone() {
        session_ids.push(runtime_session_id);
    }
    if let Some(builder_session_id) = module.latest_builder_session_id.clone() {
        session_ids.push(builder_session_id);
    }
    for run in service
        .list_runs_for_module(&team_id, &module_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        if let Some(session_id) = run.session_id {
            session_ids.push(session_id);
        }
    }
    session_ids.sort();
    session_ids.dedup();

    let deleted = service
        .delete_module(&team_id, &module_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if deleted {
        for session_id in session_ids {
            let _ = agent_service.delete_session(&session_id).await;
        }
    }

    Ok(Json(json!({ "deleted": deleted })))
}

async fn list_runs(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(project_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let items = service
        .list_runs(&team_id, &project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "runs": items })))
}

async fn list_artifacts(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(project_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let items = service
        .list_artifacts(&team_id, &project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "artifacts": items })))
}

async fn list_schedules(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(project_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let items = service
        .list_schedules(&team_id, &project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "schedules": items })))
}

async fn create_schedule(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(module_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
    Json(req): Json<CreateScheduleRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let module = service
        .get_module(&team_id, &module_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let next_run_at = compute_next_run_at(
        &super::models::AutomationScheduleDoc {
            id: None,
            schedule_id: String::new(),
            team_id: team_id.clone(),
            project_id: module.project_id.clone(),
            module_id: module.module_id.clone(),
            module_version: module.version,
            mode: req.mode.clone(),
            status: super::models::ScheduleStatus::Active,
            cron_expression: req.cron_expression.clone(),
            poll_interval_seconds: req.poll_interval_seconds,
            monitor_instruction: req.monitor_instruction.clone(),
            next_run_at: None,
            last_run_at: None,
            last_run_id: None,
            created_by: user.user_id.clone(),
            created_at: bson::DateTime::now(),
            updated_at: bson::DateTime::now(),
        },
        chrono::Utc::now(),
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    let schedule = service
        .create_schedule(
            &team_id,
            &module.project_id,
            &module.module_id,
            module.version,
            req,
            &user.user_id,
            next_run_at,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        json!({ "schedule": schedule_to_value(&schedule), "app_id": module.module_id }),
    ))
}

async fn update_schedule(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(schedule_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
    Json(req): Json<UpdateScheduleRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let existing = service
        .get_schedule(&team_id, &schedule_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let mut probe_doc = existing.clone();
    if let Some(status) = req.status.clone() {
        probe_doc.status = status;
    }
    if let Some(cron_expression) = req.cron_expression.clone() {
        probe_doc.cron_expression = Some(cron_expression);
    }
    if let Some(interval) = req.poll_interval_seconds {
        probe_doc.poll_interval_seconds = Some(interval);
    }
    if let Some(instruction) = req.monitor_instruction.clone() {
        probe_doc.monitor_instruction = Some(instruction);
    }
    let next_run_at = if matches!(probe_doc.status, super::models::ScheduleStatus::Paused) {
        existing.next_run_at
    } else {
        compute_next_run_at(&probe_doc, chrono::Utc::now()).map_err(|_| StatusCode::BAD_REQUEST)?
    };
    let updated = service
        .update_schedule(&team_id, &schedule_id, req, next_run_at)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(json!({ "schedule": schedule_to_value(&updated) })))
}

async fn delete_schedule(
    State((service, _, agent_service)): State<AutomationState>,
    Extension(user): Extension<UserContext>,
    Path(schedule_id): Path<String>,
    Query(query): Query<TeamScopedQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let team_id = query.team_id;
    ensure_team_member(&agent_service, &user, &team_id).await?;
    let deleted = service
        .delete_schedule(&team_id, &schedule_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "deleted": deleted })))
}
