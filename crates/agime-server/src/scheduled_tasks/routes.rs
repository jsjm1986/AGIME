//! Desktop scheduled task routes — simplified from team-server.
//!
//! No team_id/channel_id/auth logic. Uses local ScheduledTaskService (JSON files).

use std::sync::Arc;

use anyhow::Result;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use croner::Cron;
use serde::Deserialize;
use uuid::Uuid;

use crate::scheduled_tasks::intent::parse_scheduled_task_text;
use crate::scheduled_tasks::models::{
    human_schedule_for_task, infer_delivery_plan, infer_payload_kind, infer_session_binding,
    infer_task_profile_from_prompt, normalize_execution_contract, reconcile_task_contract,
    schedule_config_for_task, CreateScheduledTaskFromParseRequest, CreateScheduledTaskRequest,
    ParseScheduledTaskRequest, ScheduledTaskDeliveryTier, ScheduledTaskDetailResponse,
    ScheduledTaskDoc, ScheduledTaskExecutionContract, ScheduledTaskListView,
    ScheduledTaskOutputMode, ScheduledTaskProfile, ScheduledTaskPublishBehavior,
    ScheduledTaskRunDoc, ScheduledTaskRunResponse, ScheduledTaskScheduleConfig,
    ScheduledTaskScheduleMode, ScheduledTaskSessionBinding, ScheduledTaskSourceScope,
    ScheduledTaskStatus, ScheduledTaskSummaryResponse, UpdateScheduledTaskRequest,
};
use crate::scheduled_tasks::scheduler::{
    compute_next_fire_at, spawn_scheduler_loop, trigger_run_now,
};
use crate::scheduled_tasks::service::ScheduledTaskService;

#[cfg(feature = "desktop_harness_host")]
use crate::state::AppState;

type ScheduledTaskState = Arc<ScheduledTaskService>;

fn bad_request(message: impl ToString) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": message.to_string() })),
    )
}

fn internal_error(_: anyhow::Error) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": "internal error" })),
    )
}

fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "not found" })),
    )
}

fn trim_to_none(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn clamp_positive(value: Option<u32>, fallback: u32) -> u32 {
    value.filter(|item| *item > 0).unwrap_or(fallback)
}

fn normalize_daily_time(value: &str) -> Result<(u32, u32), (StatusCode, Json<serde_json::Value>)> {
    let trimmed = value.trim();
    let parts = trimmed.split(':').collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err(bad_request("daily_time must use HH:MM format"));
    }
    let hour = parts[0]
        .parse::<u32>()
        .map_err(|_| bad_request("daily_time hour is invalid"))?;
    let minute = parts[1]
        .parse::<u32>()
        .map_err(|_| bad_request("daily_time minute is invalid"))?;
    if hour > 23 || minute > 59 {
        return Err(bad_request("daily_time must be a valid clock time"));
    }
    Ok((hour, minute))
}

fn normalize_weekly_days(days: &[String]) -> Vec<String> {
    let allowed = ["1", "2", "3", "4", "5", "6", "0"];
    let mut result: Vec<String> = Vec::new();
    for key in allowed {
        if days.iter().any(|item| item == key) {
            result.push(key.to_string());
        }
    }
    if result.is_empty() {
        vec!["1".to_string()]
    } else {
        result
    }
}

fn cron_expression_from_schedule_config(
    config: &ScheduledTaskScheduleConfig,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    match config.mode {
        ScheduledTaskScheduleMode::EveryMinutes => Ok(format!(
            "*/{} * * * *",
            clamp_positive(config.every_minutes, 15)
        )),
        ScheduledTaskScheduleMode::EveryHours => {
            let hours = clamp_positive(config.every_hours, 1);
            if hours <= 1 {
                Ok("0 * * * *".to_string())
            } else {
                Ok(format!("0 */{} * * *", hours))
            }
        }
        ScheduledTaskScheduleMode::DailyAt => {
            let (hour, minute) = normalize_daily_time(
                config
                    .daily_time
                    .as_deref()
                    .ok_or_else(|| bad_request("daily_time is required for daily_at"))?,
            )?;
            Ok(format!("{minute} {hour} * * *"))
        }
        ScheduledTaskScheduleMode::WeekdaysAt => {
            let (hour, minute) = normalize_daily_time(
                config
                    .daily_time
                    .as_deref()
                    .ok_or_else(|| bad_request("daily_time is required for weekdays_at"))?,
            )?;
            Ok(format!("{minute} {hour} * * 1-5"))
        }
        ScheduledTaskScheduleMode::WeeklyOn => {
            let (hour, minute) = normalize_daily_time(
                config
                    .daily_time
                    .as_deref()
                    .ok_or_else(|| bad_request("daily_time is required for weekly_on"))?,
            )?;
            let weekly_days = normalize_weekly_days(
                config
                    .weekly_days
                    .as_deref()
                    .ok_or_else(|| bad_request("weekly_days is required for weekly_on"))?,
            );
            Ok(format!("{minute} {hour} * * {}", weekly_days.join(",")))
        }
        ScheduledTaskScheduleMode::Custom => trim_to_none(config.cron_expression.clone())
            .ok_or_else(|| bad_request("cron_expression is required for custom schedule mode")),
    }
}

fn resolve_cron_expression(
    task_kind: crate::scheduled_tasks::models::ScheduledTaskKind,
    schedule_config: Option<&ScheduledTaskScheduleConfig>,
    requested_cron_expression: Option<String>,
    existing: Option<&ScheduledTaskDoc>,
) -> Result<Option<String>, (StatusCode, Json<serde_json::Value>)> {
    if !matches!(
        task_kind,
        crate::scheduled_tasks::models::ScheduledTaskKind::Cron
    ) {
        return Ok(None);
    }
    if let Some(config) = schedule_config {
        return cron_expression_from_schedule_config(config).map(Some);
    }
    Ok(trim_to_none(requested_cron_expression)
        .or_else(|| existing.and_then(|item| item.cron_expression.clone())))
}

fn validate_schedule_candidate(
    task: &ScheduledTaskDoc,
) -> Result<Option<chrono::DateTime<Utc>>, (StatusCode, Json<serde_json::Value>)> {
    let now = Utc::now();
    match task.task_kind {
        crate::scheduled_tasks::models::ScheduledTaskKind::OneShot => {
            let fire_at = task
                .one_shot_at
                .ok_or_else(|| bad_request("one_shot_at is required for one_shot tasks"))?;
            if fire_at <= now {
                return Err(bad_request("one_shot_at must be in the future"));
            }
            Ok(Some(fire_at))
        }
        crate::scheduled_tasks::models::ScheduledTaskKind::Cron => {
            let cron_expr = trim_to_none(task.cron_expression.clone())
                .ok_or_else(|| bad_request("cron_expression is required for cron tasks"))?;
            // Validate cron expression
            if Cron::new(&cron_expr).parse().is_err() {
                return Err(bad_request("invalid cron expression"));
            }
            let next_fire_at = compute_next_fire_at(task, &task.timezone)
                .ok_or_else(|| bad_request("could not compute next fire time"))?;
            Ok(Some(next_fire_at))
        }
    }
}

fn build_task_doc(
    request: &CreateScheduledTaskRequest,
    timezone: String,
    task_profile: ScheduledTaskProfile,
    execution_contract: ScheduledTaskExecutionContract,
    next_fire_at: Option<chrono::DateTime<Utc>>,
    cron_expression: Option<String>,
    schedule_config: Option<ScheduledTaskScheduleConfig>,
) -> ScheduledTaskDoc {
    let now = Utc::now();
    let task_id = Uuid::new_v4().to_string();
    let delivery_tier = request
        .delivery_tier
        .unwrap_or(ScheduledTaskDeliveryTier::Durable);
    let payload_kind = request
        .payload_kind
        .unwrap_or_else(|| infer_payload_kind(task_profile, &execution_contract));
    let session_binding = request
        .session_binding
        .unwrap_or_else(|| infer_session_binding(request.prompt.trim(), delivery_tier));
    let delivery_plan = request
        .delivery_plan
        .unwrap_or_else(|| infer_delivery_plan(&execution_contract));

    ScheduledTaskDoc {
        task_id,
        owner_session_id: request.owner_session_id.clone(),
        agent_id: request.agent_id.clone(),
        title: request.title.trim().to_string(),
        prompt: request.prompt.trim().to_string(),
        task_kind: request.task_kind,
        task_profile,
        payload_kind,
        session_binding,
        delivery_plan,
        execution_contract,
        delivery_tier,
        one_shot_at: request.one_shot_at,
        cron_expression,
        schedule_config,
        timezone,
        status: ScheduledTaskStatus::Draft,
        next_fire_at,
        last_fire_at: None,
        last_run_id: None,
        last_expected_fire_at: None,
        last_missed_at: None,
        missed_fire_count: 0,
        lease_owner: None,
        lease_expires_at: None,
        created_at: now,
        updated_at: now,
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_tasks(
    State(service): State<ScheduledTaskState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let tasks = service.list_tasks().map_err(internal_error)?;
    let summaries: Vec<ScheduledTaskSummaryResponse> = tasks
        .iter()
        .map(ScheduledTaskSummaryResponse::from_doc)
        .collect();
    Ok(Json(serde_json::json!({ "tasks": summaries })))
}

async fn parse_task(
    State(_service): State<ScheduledTaskState>,
    Json(request): Json<ParseScheduledTaskRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let timezone =
        trim_to_none(request.timezone.clone()).unwrap_or_else(|| "Asia/Shanghai".to_string());
    let preview = parse_scheduled_task_text(
        request.text.trim(),
        Some(timezone.as_str()),
        trim_to_none(request.agent_id.clone()).as_deref(),
    );
    Ok(Json(serde_json::json!({ "preview": preview })))
}

async fn create_task(
    State(service): State<ScheduledTaskState>,
    Json(request): Json<CreateScheduledTaskRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if request.title.trim().is_empty() || request.prompt.trim().is_empty() {
        return Err(bad_request("title and prompt are required"));
    }
    let timezone =
        trim_to_none(request.timezone.clone()).unwrap_or_else(|| "Asia/Shanghai".to_string());
    let task_profile = request
        .task_profile
        .unwrap_or_else(|| infer_task_profile_from_prompt(request.prompt.trim()));
    let task_id_provisional = Uuid::new_v4().to_string();
    let execution_contract = normalize_execution_contract(
        &task_id_provisional,
        request.prompt.trim(),
        task_profile,
        request.execution_contract.clone(),
    );
    let cron_expression = resolve_cron_expression(
        request.task_kind,
        request.schedule_config.as_ref(),
        request.cron_expression.clone(),
        None,
    )?;
    let mut candidate = build_task_doc(
        &request,
        timezone,
        task_profile,
        execution_contract,
        None,
        cron_expression,
        request.schedule_config.clone(),
    );
    let next_fire_at = validate_schedule_candidate(&candidate)?;
    candidate.next_fire_at = next_fire_at;
    let task = service.create_task(candidate).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "failed to persist task" })),
        )
    })?;
    let response = ScheduledTaskDetailResponse {
        summary: ScheduledTaskSummaryResponse::from_doc(&task),
        prompt: task.prompt.clone(),
        runs: Vec::new(),
    };
    Ok(Json(serde_json::json!({ "task": response })))
}

async fn create_task_from_parse(
    State(service): State<ScheduledTaskState>,
    Json(request): Json<CreateScheduledTaskFromParseRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let preview = request.preview;
    if !preview.ready_to_create {
        return Err(bad_request("parsed task preview is not ready to create"));
    }
    let agent_id = trim_to_none(request.overrides.agent_id.clone())
        .or(preview.agent_id.clone())
        .ok_or_else(|| bad_request("agent_id is required"))?;
    let timezone = trim_to_none(request.overrides.timezone.clone())
        .unwrap_or_else(|| preview.schedule_spec.timezone.clone());
    let provisional_task_id = Uuid::new_v4().to_string();
    let prompt =
        trim_to_none(request.overrides.prompt.clone()).unwrap_or_else(|| preview.prompt.clone());
    let mut preview_contract = preview.execution_contract.clone();
    if request.overrides.artifact_path.is_none() {
        preview_contract.artifact_path = None;
    }
    let mut execution_contract = normalize_execution_contract(
        &provisional_task_id,
        &prompt,
        preview.task_profile,
        Some(preview_contract),
    );
    if let Some(path) = trim_to_none(request.overrides.artifact_path) {
        execution_contract.artifact_path = Some(path);
    }
    if let Some(behavior) = request.overrides.publish_behavior {
        execution_contract.publish_behavior = behavior;
    }
    let one_shot_at = request.overrides.one_shot_at.or_else(|| {
        preview
            .schedule_spec
            .one_shot_at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))
    });
    let create_request = CreateScheduledTaskRequest {
        agent_id,
        title: trim_to_none(request.overrides.title.clone())
            .unwrap_or_else(|| preview.title.clone()),
        prompt,
        task_kind: preview.task_kind,
        one_shot_at,
        cron_expression: request
            .overrides
            .schedule_config
            .as_ref()
            .and_then(|item| item.cron_expression.clone())
            .or(preview.schedule_spec.cron_expression),
        timezone: Some(timezone),
        delivery_tier: request
            .overrides
            .delivery_tier
            .or(Some(match preview.session_binding {
                ScheduledTaskSessionBinding::IsolatedTask => ScheduledTaskDeliveryTier::Durable,
                ScheduledTaskSessionBinding::BoundSession => {
                    ScheduledTaskDeliveryTier::SessionScoped
                }
            })),
        owner_session_id: None,
        schedule_config: request
            .overrides
            .schedule_config
            .or(preview.schedule_spec.schedule_config),
        task_profile: Some(preview.task_profile),
        payload_kind: Some(preview.payload_kind),
        session_binding: Some(preview.session_binding),
        delivery_plan: Some(preview.delivery_plan),
        execution_contract: Some(execution_contract),
    };
    create_task(State(service), Json(create_request)).await
}

async fn get_task(
    State(service): State<ScheduledTaskState>,
    Path(task_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let task = service
        .get_task(&task_id)
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    let runs = service.list_runs(&task_id).map_err(internal_error)?;
    let run_responses: Vec<ScheduledTaskRunResponse> = runs
        .iter()
        .map(ScheduledTaskRunResponse::from_doc)
        .collect();
    let response = ScheduledTaskDetailResponse {
        summary: ScheduledTaskSummaryResponse::from_doc(&task),
        prompt: task.prompt.clone(),
        runs: run_responses,
    };
    Ok(Json(serde_json::json!({ "task": response })))
}

async fn update_task(
    State(service): State<ScheduledTaskState>,
    Path(task_id): Path<String>,
    Json(request): Json<UpdateScheduledTaskRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut existing = service
        .get_task(&task_id)
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    if let Some(title) = trim_to_none(request.title.clone()) {
        existing.title = title;
    }
    if let Some(prompt) = trim_to_none(request.prompt.clone()) {
        existing.prompt = prompt;
    }
    if let Some(agent_id) = request.agent_id {
        existing.agent_id = agent_id;
    }
    if let Some(task_kind) = request.task_kind {
        existing.task_kind = task_kind;
    }
    if let Some(one_shot_at) = request.one_shot_at {
        existing.one_shot_at = Some(one_shot_at);
    }
    if let Some(ref cron_expression) = request.cron_expression {
        existing.cron_expression = trim_to_none(cron_expression.clone());
    }
    if let Some(timezone) = trim_to_none(request.timezone) {
        existing.timezone = timezone;
    }
    if let Some(schedule_config) = request.schedule_config.as_ref() {
        let resolved_cron_expression =
            cron_expression_from_schedule_config(schedule_config).map(Some)?;
        existing.cron_expression = resolved_cron_expression;
        existing.schedule_config = Some(schedule_config.clone());
    } else if request.cron_expression.is_none() {
        existing.cron_expression =
            resolve_cron_expression(existing.task_kind, None, None, Some(&existing))?;
    }
    if let Some(delivery_tier) = request.delivery_tier {
        existing.delivery_tier = delivery_tier;
    }
    if let Some(owner_session_id) = request.owner_session_id {
        existing.owner_session_id = trim_to_none(owner_session_id);
    }
    if request.task_profile.is_none()
        && request.execution_contract.is_none()
        && request.payload_kind.is_none()
        && request.session_binding.is_none()
        && request.delivery_plan.is_none()
    {
        reconcile_task_contract(&mut existing);
    } else {
        let task_profile = request
            .task_profile
            .unwrap_or_else(|| infer_task_profile_from_prompt(existing.prompt.trim()));
        existing.task_profile = task_profile;
        let execution_contract = if let Some(ref ec) = request.execution_contract {
            ec.clone()
        } else {
            existing.execution_contract.clone()
        };
        let execution_contract = normalize_execution_contract(
            &existing.task_id,
            existing.prompt.trim(),
            task_profile,
            Some(execution_contract),
        );
        existing.execution_contract = execution_contract;
        let payload_kind = request
            .payload_kind
            .unwrap_or_else(|| infer_payload_kind(task_profile, &existing.execution_contract));
        let session_binding = request.session_binding.unwrap_or_else(|| {
            infer_session_binding(existing.prompt.trim(), existing.delivery_tier)
        });
        let delivery_plan = request
            .delivery_plan
            .unwrap_or_else(|| infer_delivery_plan(&existing.execution_contract));
        existing.payload_kind = payload_kind;
        existing.session_binding = session_binding;
        existing.delivery_plan = delivery_plan;
    }
    let next_fire_at = validate_schedule_candidate(&existing)?;
    existing.next_fire_at = next_fire_at;
    existing.updated_at = Utc::now();
    service
        .update_task(&task_id, &existing)
        .map_err(internal_error)?;
    let runs = service.list_runs(&task_id).map_err(internal_error)?;
    let response = ScheduledTaskDetailResponse {
        summary: ScheduledTaskSummaryResponse::from_doc(&existing),
        prompt: existing.prompt.clone(),
        runs: runs
            .iter()
            .map(ScheduledTaskRunResponse::from_doc)
            .collect(),
    };
    Ok(Json(serde_json::json!({ "task": response })))
}

async fn publish_task(
    State(service): State<ScheduledTaskState>,
    Path(task_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut task = service
        .get_task(&task_id)
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    let next_fire_at = validate_schedule_candidate(&task)?;
    task.status = ScheduledTaskStatus::Active;
    task.next_fire_at = next_fire_at;
    task.updated_at = Utc::now();
    service
        .update_task(&task_id, &task)
        .map_err(internal_error)?;
    let response = ScheduledTaskDetailResponse {
        summary: ScheduledTaskSummaryResponse::from_doc(&task),
        prompt: task.prompt.clone(),
        runs: Vec::new(),
    };
    Ok(Json(serde_json::json!({ "task": response })))
}

async fn pause_task(
    State(service): State<ScheduledTaskState>,
    Path(task_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut task = service
        .get_task(&task_id)
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    task.status = ScheduledTaskStatus::Paused;
    task.lease_owner = None;
    task.lease_expires_at = None;
    task.updated_at = Utc::now();
    service
        .update_task(&task_id, &task)
        .map_err(internal_error)?;
    let response = ScheduledTaskDetailResponse {
        summary: ScheduledTaskSummaryResponse::from_doc(&task),
        prompt: task.prompt.clone(),
        runs: Vec::new(),
    };
    Ok(Json(serde_json::json!({ "task": response })))
}

async fn resume_task(
    State(service): State<ScheduledTaskState>,
    Path(task_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let task = service
        .get_task(&task_id)
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    let next_fire_at = match task.task_kind {
        crate::scheduled_tasks::models::ScheduledTaskKind::OneShot => {
            let fire_at = task.one_shot_at.ok_or_else(|| {
                (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": "one-shot task can no longer be resumed",
                        "hint": "The original one-shot time has already passed. Update the scheduled time before enabling it again, or use run-now."
                    })),
                )
            })?;
            if fire_at <= Utc::now() {
                return Err((
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": "one-shot task can no longer be resumed",
                        "hint": "The original one-shot time has already passed. Update the scheduled time before enabling it again, or use run-now."
                    })),
                ));
            }
            Some(fire_at)
        }
        crate::scheduled_tasks::models::ScheduledTaskKind::Cron => {
            validate_schedule_candidate(&task)?
        }
    };
    let mut task = task;
    task.status = ScheduledTaskStatus::Active;
    task.next_fire_at = next_fire_at;
    task.updated_at = Utc::now();
    service
        .update_task(&task_id, &task)
        .map_err(internal_error)?;
    let response = ScheduledTaskDetailResponse {
        summary: ScheduledTaskSummaryResponse::from_doc(&task),
        prompt: task.prompt.clone(),
        runs: Vec::new(),
    };
    Ok(Json(serde_json::json!({ "task": response })))
}

async fn run_task_now(
    State(service): State<ScheduledTaskState>,
    Path(task_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let task = service
        .get_task(&task_id)
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    let run = trigger_run_now(service.clone(), task, "Asia/Shanghai".to_string())
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
        })?;
    Ok(Json(serde_json::json!({
        "run": ScheduledTaskRunResponse::from_doc(&run),
    })))
}

async fn delete_task(
    State(service): State<ScheduledTaskState>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    let _ = service
        .get_task(&task_id)
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    service.delete_task(&task_id).map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_task_runs(
    State(service): State<ScheduledTaskState>,
    Path(task_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let _ = service
        .get_task(&task_id)
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    let runs = service.list_runs(&task_id).map_err(internal_error)?;
    let responses: Vec<ScheduledTaskRunResponse> = runs
        .iter()
        .map(ScheduledTaskRunResponse::from_doc)
        .collect();
    Ok(Json(serde_json::json!({ "runs": responses })))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router(service: ScheduledTaskState) -> Router {
    Router::new()
        .route("/", get(list_tasks).post(create_task))
        .route("/parse", post(parse_task))
        .route("/create-from-parse", post(create_task_from_parse))
        .route(
            "/{task_id}",
            get(get_task).patch(update_task).delete(delete_task),
        )
        .route("/{task_id}/publish", post(publish_task))
        .route("/{task_id}/pause", post(pause_task))
        .route("/{task_id}/resume", post(resume_task))
        .route("/{task_id}/run-now", post(run_task_now))
        .route("/{task_id}/runs", get(list_task_runs))
        .with_state(service)
}

/// Start the background scheduler with the given service and timezone.
#[cfg(feature = "desktop_harness_host")]
pub fn start_scheduler(service: Arc<ScheduledTaskService>, timezone: String) {
    spawn_scheduler_loop(service, timezone);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduled_tasks::models::{
        ScheduledTaskDeliveryPlanKind, ScheduledTaskPayloadKind, ScheduledTaskSourceScope,
    };

    #[test]
    fn parse_task_requires_text() {
        let service = Arc::new(ScheduledTaskService::new(
            std::env::temp_dir().join("test-scheduled-tasks"),
            std::env::temp_dir().join("test-scheduled-task-runs"),
        ));
        // The parse endpoint should handle empty text gracefully
        let result = tokio::runtime::Runtime::new().unwrap().block_on(parse_task(
            State(service),
            Json(ParseScheduledTaskRequest {
                text: "每天早上9点生成报告".to_string(),
                timezone: Some("Asia/Shanghai".to_string()),
                agent_id: None,
            }),
        ));
        assert!(result.is_ok());
    }

    #[test]
    fn build_task_doc_creates_valid_doc() {
        let request = CreateScheduledTaskRequest {
            agent_id: "agent-1".to_string(),
            title: "Test Task".to_string(),
            prompt: "每天早上9点生成报告".to_string(),
            task_kind: crate::scheduled_tasks::models::ScheduledTaskKind::Cron,
            one_shot_at: None,
            cron_expression: Some("0 9 * * *".to_string()),
            timezone: Some("Asia/Shanghai".to_string()),
            delivery_tier: None,
            owner_session_id: None,
            schedule_config: None,
            task_profile: None,
            payload_kind: None,
            session_binding: None,
            delivery_plan: None,
            execution_contract: None,
        };
        let task = build_task_doc(
            &request,
            "Asia/Shanghai".to_string(),
            ScheduledTaskProfile::WorkspaceTask,
            ScheduledTaskExecutionContract {
                output_mode: ScheduledTaskOutputMode::SummaryOnly,
                must_return_final_text: true,
                allow_partial_result: true,
                artifact_path: None,
                publish_behavior: ScheduledTaskPublishBehavior::None,
                source_scope: ScheduledTaskSourceScope::WorkspaceOnly,
                source_policy: None,
                minimum_source_attempts: None,
                minimum_successful_sources: None,
                prefer_structured_sources: None,
                allow_query_retry: None,
                fallback_to_secondary_sources: None,
                required_sections: Vec::new(),
            },
            None,
            Some("0 9 * * *".to_string()),
            None,
        );
        assert_eq!(task.title, "Test Task");
        assert_eq!(
            task.task_kind,
            crate::scheduled_tasks::models::ScheduledTaskKind::Cron
        );
        assert_eq!(task.status, ScheduledTaskStatus::Draft);
    }
}
