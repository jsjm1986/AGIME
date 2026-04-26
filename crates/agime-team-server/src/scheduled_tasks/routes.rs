use std::sync::Arc;

use anyhow::Result;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::Utc;
use mongodb::bson::{self, doc};
use serde::Deserialize;
use uuid::Uuid;

use agime_team::MongoDb;

use crate::agent::chat_channel_manager::ChatChannelManager;
use crate::agent::chat_channels::{
    ChatChannelService, ChatChannelType, ChatChannelVisibility, CreateChatChannelRequest,
    UpdateChatChannelRequest,
};
use crate::agent::service_mongo::AgentService;
use crate::auth::middleware::UserContext;

use super::intent::parse_scheduled_task_text;
use super::models::{
    infer_delivery_plan, infer_payload_kind, infer_session_binding, infer_task_profile_from_prompt,
    normalize_execution_contract, CreateScheduledTaskFromParseRequest, CreateScheduledTaskRequest,
    ParseScheduledTaskRequest, ScheduledTaskDeliveryTier, ScheduledTaskDoc,
    ScheduledTaskExecutionContract, ScheduledTaskListView, ScheduledTaskOutputMode,
    ScheduledTaskProfile, ScheduledTaskPublishBehavior, ScheduledTaskScheduleConfig,
    ScheduledTaskScheduleMode, ScheduledTaskSessionBinding, ScheduledTaskSourceScope,
    ScheduledTaskStatus, UpdateScheduledTaskRequest,
};
use super::scheduler::{compute_next_fire_at, create_initial_channel_message, start_task_run};
use super::service::ScheduledTaskService;

type ScheduledTaskState = (
    Arc<AgentService>,
    Arc<MongoDb>,
    Arc<ChatChannelManager>,
    String,
);

const MAX_TASKS_PER_TEAM: u64 = 100;

#[derive(Debug, Deserialize)]
struct TeamQuery {
    team_id: String,
}

#[derive(Debug, Deserialize)]
struct ListTasksQuery {
    team_id: String,
    #[serde(default)]
    view: Option<ScheduledTaskListView>,
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

fn bad_request(message: impl ToString) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": message.to_string() })),
    )
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

async fn is_team_admin(
    service: &AgentService,
    user: &UserContext,
    team_id: &str,
) -> Result<bool, StatusCode> {
    service
        .is_team_admin(&user.user_id, team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn can_access_task(task: &ScheduledTaskDoc, user: &UserContext, admin: bool) -> bool {
    if task.owner_user_id == user.user_id {
        return true;
    }
    match task.delivery_tier {
        ScheduledTaskDeliveryTier::Durable => admin,
        ScheduledTaskDeliveryTier::SessionScoped => false,
    }
}

fn resolve_effective_owner_session_id(
    delivery_tier: ScheduledTaskDeliveryTier,
    requested_owner_session_id: Option<Option<String>>,
    user: &UserContext,
    existing_owner_session_id: Option<&str>,
) -> Option<String> {
    if !matches!(delivery_tier, ScheduledTaskDeliveryTier::SessionScoped) {
        return None;
    }
    requested_owner_session_id
        .and_then(trim_to_none)
        .or_else(|| existing_owner_session_id.map(ToString::to_string))
        .or_else(|| user.current_session_id.clone())
}

fn build_schedule_candidate(
    task_id: Option<String>,
    existing: Option<&ScheduledTaskDoc>,
    request: &CreateScheduledTaskRequest,
    timezone: String,
    one_shot_at: Option<chrono::DateTime<Utc>>,
    cron_expression: Option<String>,
    schedule_config: Option<ScheduledTaskScheduleConfig>,
    task_profile: ScheduledTaskProfile,
    execution_contract: ScheduledTaskExecutionContract,
) -> ScheduledTaskDoc {
    let now = bson::DateTime::now();
    let delivery_tier = request.delivery_tier.unwrap_or_else(|| {
        existing
            .map(|item| item.delivery_tier)
            .unwrap_or(ScheduledTaskDeliveryTier::Durable)
    });
    ScheduledTaskDoc {
        id: None,
        task_id: task_id
            .or_else(|| existing.map(|item| item.task_id.clone()))
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        team_id: existing
            .map(|item| item.team_id.clone())
            .unwrap_or_default(),
        channel_id: existing
            .map(|item| item.channel_id.clone())
            .unwrap_or_default(),
        owner_user_id: existing
            .map(|item| item.owner_user_id.clone())
            .unwrap_or_default(),
        agent_id: request.agent_id.clone(),
        title: request.title.trim().to_string(),
        prompt: request.prompt.trim().to_string(),
        task_kind: request.task_kind,
        task_profile,
        payload_kind: request
            .payload_kind
            .unwrap_or_else(|| infer_payload_kind(task_profile, &execution_contract)),
        session_binding: request
            .session_binding
            .unwrap_or_else(|| infer_session_binding(request.prompt.trim(), delivery_tier)),
        delivery_plan: request
            .delivery_plan
            .unwrap_or_else(|| infer_delivery_plan(&execution_contract)),
        execution_contract,
        delivery_tier,
        owner_session_id: request
            .owner_session_id
            .clone()
            .or_else(|| existing.and_then(|item| item.owner_session_id.clone())),
        one_shot_at: one_shot_at.map(bson::DateTime::from_chrono),
        cron_expression: cron_expression
            .or_else(|| existing.and_then(|item| item.cron_expression.clone())),
        schedule_config: schedule_config
            .or_else(|| existing.and_then(|item| item.schedule_config.clone())),
        timezone,
        status: existing
            .map(|item| item.status)
            .unwrap_or(ScheduledTaskStatus::Draft),
        next_fire_at: existing.and_then(|item| item.next_fire_at),
        last_fire_at: existing.and_then(|item| item.last_fire_at),
        last_run_id: existing.and_then(|item| item.last_run_id.clone()),
        last_expected_fire_at: existing.and_then(|item| item.last_expected_fire_at),
        last_missed_at: existing.and_then(|item| item.last_missed_at),
        missed_fire_count: existing.map(|item| item.missed_fire_count).unwrap_or(0),
        lease_owner: existing.and_then(|item| item.lease_owner.clone()),
        lease_expires_at: existing.and_then(|item| item.lease_expires_at),
        created_at: existing.map(|item| item.created_at).unwrap_or(now),
        updated_at: now,
    }
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
    let mut result = Vec::new();
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

fn clamp_positive(value: Option<u32>, fallback: u32) -> u32 {
    value.filter(|item| *item > 0).unwrap_or(fallback)
}

fn validate_artifact_path(
    artifact_path: Option<&str>,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let Some(path) = artifact_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(bad_request(
            "artifact_path must be a relative workspace path",
        ));
    }
    if path.contains("..") {
        return Err(bad_request("artifact_path must stay inside workspace"));
    }
    Ok(())
}

fn validate_execution_contract(
    profile: ScheduledTaskProfile,
    contract: &ScheduledTaskExecutionContract,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if contract.must_return_final_text
        && matches!(contract.output_mode, ScheduledTaskOutputMode::SummaryOnly)
        && matches!(
            contract.publish_behavior,
            ScheduledTaskPublishBehavior::None
        )
    {
        validate_artifact_path(None)?;
    }
    if matches!(
        contract.output_mode,
        ScheduledTaskOutputMode::SummaryAndArtifact
    ) {
        validate_artifact_path(contract.artifact_path.as_deref())?;
    }
    match profile {
        ScheduledTaskProfile::DocumentTask => {
            if matches!(
                contract.source_scope,
                ScheduledTaskSourceScope::WorkspaceOnly
            ) {
                return Err(bad_request(
                    "document_task must not use workspace_only source_scope",
                ));
            }
            if !matches!(
                contract.publish_behavior,
                ScheduledTaskPublishBehavior::None
            ) && !matches!(
                contract.output_mode,
                ScheduledTaskOutputMode::SummaryAndArtifact
            ) {
                return Err(bad_request(
                    "document publish tasks must use summary_and_artifact output_mode",
                ));
            }
        }
        ScheduledTaskProfile::WorkspaceTask => {
            if !matches!(
                contract.source_scope,
                ScheduledTaskSourceScope::WorkspaceOnly
            ) {
                return Err(bad_request(
                    "workspace_task must use workspace_only source_scope",
                ));
            }
            if !matches!(
                contract.publish_behavior,
                ScheduledTaskPublishBehavior::None
            ) {
                return Err(bad_request(
                    "workspace_task does not support document publish behavior",
                ));
            }
        }
        ScheduledTaskProfile::HybridTask => {
            if !matches!(contract.source_scope, ScheduledTaskSourceScope::Mixed) {
                return Err(bad_request("hybrid_task must use mixed source_scope"));
            }
        }
        ScheduledTaskProfile::RetrievalTask => {
            if !matches!(
                contract.source_scope,
                ScheduledTaskSourceScope::ExternalRetrieval
            ) {
                return Err(bad_request(
                    "retrieval_task must use external_retrieval source_scope",
                ));
            }
            if contract.source_policy.is_none() {
                return Err(bad_request("retrieval_task must define a source_policy"));
            }
            if contract.minimum_source_attempts.unwrap_or(0) < 1 {
                return Err(bad_request(
                    "retrieval_task must request at least one source attempt",
                ));
            }
            if contract.minimum_successful_sources.unwrap_or(0) < 1 {
                return Err(bad_request(
                    "retrieval_task must request at least one successful source",
                ));
            }
            if contract.minimum_successful_sources.unwrap_or(0)
                > contract.minimum_source_attempts.unwrap_or(0)
            {
                return Err(bad_request(
                    "minimum_successful_sources must not exceed minimum_source_attempts",
                ));
            }
        }
    }
    Ok(())
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
    task_kind: super::models::ScheduledTaskKind,
    schedule_config: Option<&ScheduledTaskScheduleConfig>,
    requested_cron_expression: Option<String>,
    existing: Option<&ScheduledTaskDoc>,
) -> Result<Option<String>, (StatusCode, Json<serde_json::Value>)> {
    if !matches!(task_kind, super::models::ScheduledTaskKind::Cron) {
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
        super::models::ScheduledTaskKind::OneShot => {
            let fire_at = task
                .one_shot_at
                .map(|value| value.to_chrono())
                .ok_or_else(|| bad_request("one_shot_at is required for one_shot tasks"))?;
            if fire_at <= now {
                return Err(bad_request("one_shot_at must be in the future"));
            }
            Ok(Some(fire_at))
        }
        super::models::ScheduledTaskKind::Cron => {
            if trim_to_none(task.cron_expression.clone()).is_none() {
                return Err(bad_request("cron_expression is required for cron tasks"));
            }
            let next_fire_at =
                compute_next_fire_at(task, now).map_err(|error| bad_request(error))?;
            Ok(next_fire_at)
        }
    }
}

fn validate_resume_candidate(
    task: &ScheduledTaskDoc,
) -> Result<Option<chrono::DateTime<Utc>>, (StatusCode, Json<serde_json::Value>)> {
    match task.task_kind {
        super::models::ScheduledTaskKind::OneShot => {
            let fire_at = task
                .one_shot_at
                .map(|value| value.to_chrono())
                .ok_or_else(|| bad_request("one_shot_at is required for one_shot tasks"))?;
            if fire_at <= Utc::now() {
                return Err((
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": "one-shot task can no longer be resumed",
                        "hint": "The original one-shot time has already passed. Update the scheduled time before enabling it again, or use run-now."
                    })),
                ));
            }
            Ok(Some(fire_at))
        }
        super::models::ScheduledTaskKind::Cron => validate_schedule_candidate(task),
    }
}

async fn ensure_task_access(
    service: &AgentService,
    scheduled_service: &ScheduledTaskService,
    user: &UserContext,
    team_id: &str,
    task_id: &str,
) -> Result<ScheduledTaskDoc, StatusCode> {
    ensure_team_member(service, user, team_id).await?;
    let admin = is_team_admin(service, user, team_id).await?;
    let task = scheduled_service
        .get_task(team_id, task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if can_access_task(&task, user, admin) {
        Ok(task)
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

async fn list_tasks(
    State((service, db, _, _)): State<ScheduledTaskState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<ListTasksQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    ensure_team_member(service.as_ref(), &user, &query.team_id).await?;
    let admin = is_team_admin(service.as_ref(), &user, &query.team_id).await?;
    let scheduled_service = ScheduledTaskService::new(db);
    let view = query.view.unwrap_or(ScheduledTaskListView::Mine);
    let result = scheduled_service
        .list_tasks(&query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let filtered = result
        .tasks
        .into_iter()
        .filter(|task| {
            if task.owner_user_id == user.user_id {
                return true;
            }
            if matches!(task.delivery_tier, ScheduledTaskDeliveryTier::SessionScoped) {
                return false;
            }
            matches!(view, ScheduledTaskListView::AllVisible) && admin
        })
        .collect::<Vec<_>>();
    let repair_events = result
        .repair_events
        .into_iter()
        .filter(|event| {
            if event.owner_user_id == user.user_id {
                return true;
            }
            if matches!(
                event.delivery_tier,
                ScheduledTaskDeliveryTier::SessionScoped
            ) {
                return false;
            }
            matches!(view, ScheduledTaskListView::AllVisible) && admin
        })
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({
        "tasks": filtered,
        "repair_events": repair_events,
    })))
}

async fn parse_task_preview(
    State((service, _, _, _)): State<ScheduledTaskState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<TeamQuery>,
    Json(request): Json<ParseScheduledTaskRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    ensure_team_member(service.as_ref(), &user, &query.team_id)
        .await
        .map_err(|status| (status, Json(serde_json::json!({ "error": "forbidden" }))))?;

    if let Some(agent_id) = request.agent_id.as_deref() {
        let Some(agent) = service
            .get_agent(agent_id)
            .await
            .map_err(|_| bad_request("failed to load agent"))?
        else {
            return Err(bad_request("agent not found"));
        };
        if agent.team_id != query.team_id {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "error": "agent out of team" })),
            ));
        }
    }

    let timezone =
        trim_to_none(request.timezone.clone()).unwrap_or_else(|| "Asia/Shanghai".to_string());
    let mut preview = parse_scheduled_task_text(
        request.text.trim(),
        Some(&timezone),
        trim_to_none(request.agent_id.clone()),
    );
    if matches!(
        preview.session_binding,
        ScheduledTaskSessionBinding::BoundSession
    ) && user.current_session_id.is_none()
    {
        preview
            .warnings
            .push("当前没有可绑定的登录会话，创建时将无法使用会话级绑定。".to_string());
    }
    Ok(Json(serde_json::json!({ "preview": preview })))
}

pub(crate) async fn create_task_internal(
    service: &AgentService,
    db: Arc<MongoDb>,
    user: &UserContext,
    team_id: &str,
    request: CreateScheduledTaskRequest,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let Some(agent) = service
        .get_agent(&request.agent_id)
        .await
        .map_err(|_| bad_request("failed to load agent"))?
    else {
        return Err(bad_request("agent not found"));
    };
    if agent.team_id != team_id {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "agent out of team" })),
        ));
    }
    let scheduled_service = ScheduledTaskService::new(db.clone());
    let existing_count = scheduled_service
        .count_non_deleted_tasks(team_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to count tasks" })),
            )
        })?;
    if existing_count >= MAX_TASKS_PER_TEAM {
        return Err(bad_request(format!(
            "too many scheduled tasks (max {})",
            MAX_TASKS_PER_TEAM
        )));
    }
    let delivery_tier = request
        .delivery_tier
        .unwrap_or(ScheduledTaskDeliveryTier::Durable);
    let owner_session_id = resolve_effective_owner_session_id(
        delivery_tier,
        request.owner_session_id.clone().map(Some),
        user,
        None,
    );
    if matches!(delivery_tier, ScheduledTaskDeliveryTier::SessionScoped)
        && owner_session_id.is_none()
    {
        return Err(bad_request(
            "session_scoped tasks require an authenticated web session",
        ));
    }
    let timezone = trim_to_none(request.timezone.clone()).unwrap_or_else(|| "UTC".to_string());
    let cron_expression = resolve_cron_expression(
        request.task_kind,
        request.schedule_config.as_ref(),
        request.cron_expression.clone(),
        None,
    )?;
    let task_id = Uuid::new_v4().to_string();
    let task_profile = request
        .task_profile
        .unwrap_or_else(|| infer_task_profile_from_prompt(request.prompt.trim()));
    let execution_contract = normalize_execution_contract(
        &task_id,
        request.prompt.trim(),
        task_profile,
        request.execution_contract.clone(),
    );
    let payload_kind = request
        .payload_kind
        .unwrap_or_else(|| infer_payload_kind(task_profile, &execution_contract));
    let session_binding = request
        .session_binding
        .unwrap_or_else(|| infer_session_binding(request.prompt.trim(), delivery_tier));
    let delivery_plan = request
        .delivery_plan
        .unwrap_or_else(|| infer_delivery_plan(&execution_contract));
    validate_execution_contract(task_profile, &execution_contract)?;
    let schedule_candidate = build_schedule_candidate(
        Some(task_id.clone()),
        None,
        &request,
        timezone.clone(),
        request.one_shot_at,
        cron_expression.clone(),
        request.schedule_config.clone(),
        task_profile,
        execution_contract.clone(),
    );
    let next_fire_at = validate_schedule_candidate(&schedule_candidate)?;
    let channel_service = ChatChannelService::new(db.clone());
    let channel = channel_service
        .create_channel(
            team_id,
            &user.user_id,
            CreateChatChannelRequest {
                name: request.title.clone(),
                description: Some("定时任务频道".to_string()),
                visibility: Some(ChatChannelVisibility::TeamPrivate),
                channel_type: Some(ChatChannelType::ScheduledTask),
                default_agent_id: request.agent_id.clone(),
                member_user_ids: Vec::new(),
                workspace_display_name: None,
                repo_default_branch: None,
            },
        )
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to create channel" })),
            )
        })?;

    let now = bson::DateTime::now();
    let mut task = ScheduledTaskDoc {
        id: None,
        task_id: task_id.clone(),
        team_id: team_id.to_string(),
        channel_id: channel.channel_id.clone(),
        owner_user_id: user.user_id.clone(),
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
        owner_session_id,
        one_shot_at: request.one_shot_at.map(bson::DateTime::from_chrono),
        cron_expression,
        schedule_config: request.schedule_config.clone(),
        timezone,
        status: ScheduledTaskStatus::Draft,
        next_fire_at: next_fire_at.map(bson::DateTime::from_chrono),
        last_fire_at: None,
        last_run_id: None,
        last_expected_fire_at: None,
        last_missed_at: None,
        missed_fire_count: 0,
        lease_owner: None,
        lease_expires_at: None,
        created_at: now,
        updated_at: now,
    };
    if task.title.is_empty() || task.prompt.is_empty() {
        let _ = channel_service
            .delete_channel(
                &channel.channel_id,
                &user.user_id,
                crate::agent::chat_channels::ChatChannelDeleteMode::PreserveDocuments,
            )
            .await;
        return Err(bad_request("title and prompt are required"));
    }
    task = scheduled_service.create_task(task).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "failed to persist task" })),
        )
    })?;
    let _ = create_initial_channel_message(db.clone(), &task).await;
    let detail = scheduled_service
        .get_task_detail(team_id, &task_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load task detail" })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "created task missing" })),
            )
        })?;
    Ok(Json(serde_json::json!({ "task": detail })))
}

async fn create_task(
    State((service, db, _, _workspace_root)): State<ScheduledTaskState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<TeamQuery>,
    Json(request): Json<CreateScheduledTaskRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    ensure_team_member(service.as_ref(), &user, &query.team_id)
        .await
        .map_err(|status| (status, Json(serde_json::json!({ "error": "forbidden" }))))?;
    create_task_internal(service.as_ref(), db, &user, &query.team_id, request).await
}

async fn create_task_from_parse(
    State((service, db, _, _)): State<ScheduledTaskState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<TeamQuery>,
    Json(request): Json<CreateScheduledTaskFromParseRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    ensure_team_member(service.as_ref(), &user, &query.team_id)
        .await
        .map_err(|status| (status, Json(serde_json::json!({ "error": "forbidden" }))))?;

    let preview = request.preview;
    if !preview.ready_to_create {
        return Err(bad_request("parsed task preview is not ready to create"));
    }
    let agent_id = trim_to_none(request.overrides.agent_id)
        .or(preview.agent_id.clone())
        .ok_or_else(|| bad_request("agent_id is required"))?;
    let timezone = trim_to_none(request.overrides.timezone)
        .unwrap_or_else(|| preview.schedule_spec.timezone.clone());
    let provisional_task_id = Uuid::new_v4().to_string();
    let prompt = trim_to_none(request.overrides.prompt.clone()).unwrap_or(preview.prompt.clone());
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
    let create_request = CreateScheduledTaskRequest {
        agent_id,
        title: trim_to_none(request.overrides.title).unwrap_or(preview.title),
        prompt,
        task_kind: preview.task_kind,
        one_shot_at: request.overrides.one_shot_at.or_else(|| {
            preview
                .schedule_spec
                .one_shot_at
                .as_deref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc))
        }),
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
    create_task_internal(service.as_ref(), db, &user, &query.team_id, create_request).await
}

async fn get_task_detail(
    State((service, db, _, _)): State<ScheduledTaskState>,
    Extension(user): Extension<UserContext>,
    Path(task_id): Path<String>,
    Query(query): Query<TeamQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let _ = ensure_task_access(
        service.as_ref(),
        &ScheduledTaskService::new(db.clone()),
        &user,
        &query.team_id,
        &task_id,
    )
    .await?;
    let scheduled_service = ScheduledTaskService::new(db);
    let detail = scheduled_service
        .get_task_detail(&query.team_id, &task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(serde_json::json!({ "task": detail })))
}

pub(crate) async fn update_task_internal(
    service: &AgentService,
    db: Arc<MongoDb>,
    user: &UserContext,
    team_id: &str,
    task_id: &str,
    request: UpdateScheduledTaskRequest,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let existing = ensure_task_access(
        service,
        &ScheduledTaskService::new(db.clone()),
        user,
        team_id,
        task_id,
    )
    .await
    .map_err(|status| {
        (
            status,
            Json(serde_json::json!({ "error": "task not found" })),
        )
    })?;

    let mut candidate = existing.clone();

    if let Some(agent_id) = request.agent_id.as_deref() {
        let Some(agent) = service
            .get_agent(agent_id)
            .await
            .map_err(|_| bad_request("failed to load agent"))?
        else {
            return Err(bad_request("agent not found"));
        };
        if agent.team_id != team_id {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "error": "agent out of team" })),
            ));
        }
        candidate.agent_id = agent_id.to_string();
    }

    let mut set_doc = doc! { "updated_at": bson::DateTime::now() };
    if let Some(title) = trim_to_none(request.title.clone()) {
        candidate.title = title.clone();
        set_doc.insert("title", title.clone());
        let _ = ChatChannelService::new(db.clone())
            .update_channel(
                &existing.channel_id,
                UpdateChatChannelRequest {
                    name: Some(title),
                    description: None,
                    visibility: None,
                    channel_type: None,
                    default_agent_id: None,
                    agent_autonomy_mode: None,
                    channel_goal: None,
                    participant_notes: None,
                    expected_outputs: None,
                    collaboration_style: None,
                    workspace_display_name: None,
                    repo_default_branch: None,
                },
            )
            .await;
    }
    if let Some(prompt) = trim_to_none(request.prompt.clone()) {
        candidate.prompt = prompt.clone();
        set_doc.insert("prompt", prompt);
    }
    if let Some(agent_id) = request.agent_id {
        candidate.agent_id = agent_id.clone();
        set_doc.insert("agent_id", agent_id.clone());
        let _ = ChatChannelService::new(db.clone())
            .update_channel(
                &existing.channel_id,
                UpdateChatChannelRequest {
                    name: None,
                    description: None,
                    visibility: None,
                    channel_type: None,
                    default_agent_id: Some(agent_id),
                    agent_autonomy_mode: None,
                    channel_goal: None,
                    participant_notes: None,
                    expected_outputs: None,
                    collaboration_style: None,
                    workspace_display_name: None,
                    repo_default_branch: None,
                },
            )
            .await;
    }
    if let Some(task_kind) = request.task_kind {
        candidate.task_kind = task_kind;
        set_doc.insert(
            "task_kind",
            bson::to_bson(&task_kind).map_err(|_| bad_request("invalid task kind"))?,
        );
        match task_kind {
            super::models::ScheduledTaskKind::OneShot => {
                set_doc.insert(
                    "cron_expression",
                    bson::to_bson(&Option::<String>::None)
                        .map_err(|_| bad_request("invalid cron expression"))?,
                );
            }
            super::models::ScheduledTaskKind::Cron => {
                set_doc.insert("one_shot_at", bson::Bson::Null);
            }
        }
    }
    if let Some(one_shot_at) = request.one_shot_at {
        candidate.one_shot_at = Some(bson::DateTime::from_chrono(one_shot_at));
        set_doc.insert("one_shot_at", bson::DateTime::from_chrono(one_shot_at));
    }
    if let Some(cron_expression) = request.cron_expression.as_ref() {
        candidate.cron_expression = trim_to_none(cron_expression.clone());
        set_doc.insert(
            "cron_expression",
            bson::to_bson(&trim_to_none(cron_expression.clone()))
                .map_err(|_| bad_request("invalid cron"))?,
        );
    }
    if let Some(timezone) = trim_to_none(request.timezone) {
        candidate.timezone = timezone.clone();
        set_doc.insert("timezone", timezone);
    }
    if let Some(schedule_config) = request.schedule_config.as_ref() {
        let resolved_cron_expression =
            cron_expression_from_schedule_config(schedule_config).map(Some)?;
        candidate.cron_expression = resolved_cron_expression.clone();
        candidate.schedule_config = Some(schedule_config.clone());
        set_doc.insert(
            "cron_expression",
            bson::to_bson(&resolved_cron_expression)
                .map_err(|_| bad_request("invalid schedule config"))?,
        );
        set_doc.insert(
            "schedule_config",
            bson::to_bson(&Some(schedule_config.clone()))
                .map_err(|_| bad_request("invalid schedule config"))?,
        );
    } else if request.cron_expression.is_none() {
        candidate.cron_expression =
            resolve_cron_expression(candidate.task_kind, None, None, Some(&existing))?;
    }
    if let Some(delivery_tier) = request.delivery_tier {
        candidate.delivery_tier = delivery_tier;
        set_doc.insert(
            "delivery_tier",
            bson::to_bson(&delivery_tier).map_err(|_| bad_request("invalid delivery tier"))?,
        );
    }
    let resolved_owner_session_id = resolve_effective_owner_session_id(
        candidate.delivery_tier,
        request.owner_session_id,
        &user,
        existing.owner_session_id.as_deref(),
    );
    candidate.owner_session_id = resolved_owner_session_id.clone();
    set_doc.insert(
        "owner_session_id",
        bson::to_bson(&resolved_owner_session_id)
            .map_err(|_| bad_request("invalid owner session"))?,
    );
    if matches!(
        candidate.delivery_tier,
        ScheduledTaskDeliveryTier::SessionScoped
    ) && candidate.owner_session_id.is_none()
    {
        return Err(bad_request(
            "session_scoped tasks require an authenticated web session",
        ));
    }
    let task_profile = request
        .task_profile
        .unwrap_or_else(|| infer_task_profile_from_prompt(candidate.prompt.trim()));
    candidate.task_profile = task_profile;
    let execution_contract = normalize_execution_contract(
        &candidate.task_id,
        candidate.prompt.trim(),
        task_profile,
        request
            .execution_contract
            .clone()
            .or_else(|| Some(candidate.execution_contract.clone())),
    );
    validate_execution_contract(task_profile, &execution_contract)?;
    candidate.execution_contract = execution_contract.clone();
    let payload_kind = request
        .payload_kind
        .unwrap_or_else(|| infer_payload_kind(task_profile, &execution_contract));
    let session_binding = request
        .session_binding
        .unwrap_or_else(|| infer_session_binding(candidate.prompt.trim(), candidate.delivery_tier));
    let delivery_plan = request
        .delivery_plan
        .unwrap_or_else(|| infer_delivery_plan(&execution_contract));
    candidate.payload_kind = payload_kind;
    candidate.session_binding = session_binding;
    candidate.delivery_plan = delivery_plan;
    set_doc.insert(
        "task_profile",
        bson::to_bson(&task_profile).map_err(|_| bad_request("invalid task profile"))?,
    );
    set_doc.insert(
        "payload_kind",
        bson::to_bson(&payload_kind).map_err(|_| bad_request("invalid payload kind"))?,
    );
    set_doc.insert(
        "session_binding",
        bson::to_bson(&session_binding).map_err(|_| bad_request("invalid session binding"))?,
    );
    set_doc.insert(
        "delivery_plan",
        bson::to_bson(&delivery_plan).map_err(|_| bad_request("invalid delivery plan"))?,
    );
    set_doc.insert(
        "execution_contract",
        bson::to_bson(&execution_contract)
            .map_err(|_| bad_request("invalid execution contract"))?,
    );
    let recomputed_next_fire_at = validate_schedule_candidate(&candidate)?;
    set_doc.insert(
        "next_fire_at",
        bson::to_bson(&recomputed_next_fire_at.map(bson::DateTime::from_chrono))
            .map_err(|_| bad_request("invalid next fire"))?,
    );
    let scheduled_service = ScheduledTaskService::new(db.clone());
    scheduled_service
        .update_task_doc(team_id, task_id, set_doc)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to update task" })),
            )
        })?;
    let detail = scheduled_service
        .get_task_detail(team_id, task_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to reload task" })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "task not found" })),
            )
        })?;
    Ok(Json(serde_json::json!({ "task": detail })))
}

async fn update_task(
    State((service, db, _, _)): State<ScheduledTaskState>,
    Extension(user): Extension<UserContext>,
    Path(task_id): Path<String>,
    Query(query): Query<TeamQuery>,
    Json(request): Json<UpdateScheduledTaskRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    update_task_internal(
        service.as_ref(),
        db,
        &user,
        &query.team_id,
        &task_id,
        request,
    )
    .await
}

async fn publish_task(
    State((service, db, _, _)): State<ScheduledTaskState>,
    Extension(user): Extension<UserContext>,
    Path(task_id): Path<String>,
    Query(query): Query<TeamQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let task = ensure_task_access(
        service.as_ref(),
        &ScheduledTaskService::new(db.clone()),
        &user,
        &query.team_id,
        &task_id,
    )
    .await
    .map_err(|status| {
        (
            status,
            Json(serde_json::json!({ "error": "task not found" })),
        )
    })?;
    let next_fire_at = validate_schedule_candidate(&task)?;
    let scheduled_service = ScheduledTaskService::new(db.clone());
    scheduled_service
        .update_task_doc(
            &query.team_id,
            &task_id,
            doc! {
                "status": bson::to_bson(&ScheduledTaskStatus::Active).map_err(|_| bad_request("invalid status"))?,
                "next_fire_at": bson::to_bson(&next_fire_at.map(bson::DateTime::from_chrono)).map_err(|_| bad_request("invalid next fire"))?,
                "updated_at": bson::DateTime::now(),
            },
        )
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "failed to publish task" }))))?;
    let detail = scheduled_service
        .get_task_detail(&query.team_id, &task_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to reload task" })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "task not found" })),
            )
        })?;
    Ok(Json(serde_json::json!({ "task": detail })))
}

async fn pause_task(
    State((service, db, _, _)): State<ScheduledTaskState>,
    Extension(user): Extension<UserContext>,
    Path(task_id): Path<String>,
    Query(query): Query<TeamQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let _ = ensure_task_access(
        service.as_ref(),
        &ScheduledTaskService::new(db.clone()),
        &user,
        &query.team_id,
        &task_id,
    )
    .await?;
    let scheduled_service = ScheduledTaskService::new(db.clone());
    scheduled_service
        .update_task_doc(
            &query.team_id,
            &task_id,
            doc! {
                "status": "paused",
                "lease_owner": bson::Bson::Null,
                "lease_expires_at": bson::Bson::Null,
                "updated_at": bson::DateTime::now(),
            },
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let detail = scheduled_service
        .get_task_detail(&query.team_id, &task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(serde_json::json!({ "task": detail })))
}

async fn resume_task(
    State((service, db, _, _)): State<ScheduledTaskState>,
    Extension(user): Extension<UserContext>,
    Path(task_id): Path<String>,
    Query(query): Query<TeamQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let task = ensure_task_access(
        service.as_ref(),
        &ScheduledTaskService::new(db.clone()),
        &user,
        &query.team_id,
        &task_id,
    )
    .await
    .map_err(|status| {
        (
            status,
            Json(serde_json::json!({ "error": "task not found" })),
        )
    })?;
    let next_fire_at = validate_resume_candidate(&task)?;
    let scheduled_service = ScheduledTaskService::new(db.clone());
    scheduled_service
        .update_task_doc(
            &query.team_id,
            &task_id,
            doc! {
                "status": "active",
                "next_fire_at": bson::to_bson(&next_fire_at.map(bson::DateTime::from_chrono)).map_err(|_| bad_request("invalid next fire"))?,
                "updated_at": bson::DateTime::now(),
            },
        )
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "failed to resume task" }))))?;
    let detail = scheduled_service
        .get_task_detail(&query.team_id, &task_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to reload task" })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "task not found" })),
            )
        })?;
    Ok(Json(serde_json::json!({ "task": detail })))
}

async fn run_task_now(
    State((service, db, channel_manager, workspace_root)): State<ScheduledTaskState>,
    Extension(user): Extension<UserContext>,
    Path(task_id): Path<String>,
    Query(query): Query<TeamQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let _ = ensure_task_access(
        service.as_ref(),
        &ScheduledTaskService::new(db.clone()),
        &user,
        &query.team_id,
        &task_id,
    )
    .await
    .map_err(|status| (status, Json(serde_json::json!({ "error": "forbidden" }))))?;
    let scheduled_service = ScheduledTaskService::new(db.clone());
    let lease_owner = format!("manual-run:{}", Uuid::new_v4());
    let task = scheduled_service
        .claim_task_for_manual_run(&query.team_id, &task_id, &lease_owner, 120)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to claim task" })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "task is already running or unavailable" })),
            )
        })?;
    let run = start_task_run(
        db.clone(),
        channel_manager.clone(),
        workspace_root.clone(),
        task.clone(),
        "manual",
        lease_owner.clone(),
    )
    .await
    .map_err(|error| {
        let db = db.clone();
        let team_id = query.team_id.clone();
        let task_id = task_id.clone();
        let lease_owner = lease_owner.clone();
        tokio::spawn(async move {
            let _ = ScheduledTaskService::new(db)
                .release_lease(&team_id, &task_id, &lease_owner)
                .await;
        });
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
    })?;
    Ok(Json(serde_json::json!({
        "run": super::models::ScheduledTaskRunResponse::from_doc(&run),
    })))
}

async fn cancel_task_run(
    State((service, db, channel_manager, _)): State<ScheduledTaskState>,
    Extension(user): Extension<UserContext>,
    Path(task_id): Path<String>,
    Query(query): Query<TeamQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let task = ensure_task_access(
        service.as_ref(),
        &ScheduledTaskService::new(db),
        &user,
        &query.team_id,
        &task_id,
    )
    .await?;
    let cancelled = channel_manager.cancel(&task.channel_id, None).await;
    if !cancelled {
        return Err(StatusCode::CONFLICT);
    }
    Ok(Json(serde_json::json!({ "cancelled": true })))
}

async fn delete_task(
    State((service, db, _, _)): State<ScheduledTaskState>,
    Extension(user): Extension<UserContext>,
    Path(task_id): Path<String>,
    Query(query): Query<TeamQuery>,
) -> Result<StatusCode, StatusCode> {
    let task = ensure_task_access(
        service.as_ref(),
        &ScheduledTaskService::new(db.clone()),
        &user,
        &query.team_id,
        &task_id,
    )
    .await?;
    let scheduled_service = ScheduledTaskService::new(db.clone());
    scheduled_service
        .update_task_doc(
            &query.team_id,
            &task_id,
            doc! {
                "status": "deleted",
                "next_fire_at": bson::Bson::Null,
                "lease_owner": bson::Bson::Null,
                "lease_expires_at": bson::Bson::Null,
                "updated_at": bson::DateTime::now(),
            },
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let _ = ChatChannelService::new(db)
        .archive_channel(&task.channel_id)
        .await;
    Ok(StatusCode::NO_CONTENT)
}

pub fn router(
    db: Arc<MongoDb>,
    channel_manager: Arc<ChatChannelManager>,
    workspace_root: String,
) -> Router {
    let service = Arc::new(AgentService::new(db.clone()));
    let scheduled_service = ScheduledTaskService::new(db.clone());
    tokio::spawn(async move {
        let _ = scheduled_service.ensure_indexes().await;
    });
    Router::new()
        .route("/", get(list_tasks).post(create_task))
        .route("/parse", post(parse_task_preview))
        .route("/create-from-parse", post(create_task_from_parse))
        .route(
            "/{task_id}",
            get(get_task_detail).patch(update_task).delete(delete_task),
        )
        .route("/{task_id}/publish", post(publish_task))
        .route("/{task_id}/pause", post(pause_task))
        .route("/{task_id}/resume", post(resume_task))
        .route("/{task_id}/run-now", post(run_task_now))
        .route("/{task_id}/cancel", post(cancel_task_run))
        .with_state((service, db, channel_manager, workspace_root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::middleware::UserContext;
    use crate::auth::service_mongo::UserPreferences;
    use crate::scheduled_tasks::models::{ScheduledTaskDeliveryPlanKind, ScheduledTaskPayloadKind};

    fn sample_user(user_id: &str) -> UserContext {
        UserContext {
            user_id: user_id.to_string(),
            email: format!("{user_id}@example.com"),
            display_name: user_id.to_string(),
            role: "member".to_string(),
            preferences: UserPreferences::default(),
            current_session_id: Some(format!("session-{user_id}")),
        }
    }

    fn sample_task(
        delivery_tier: ScheduledTaskDeliveryTier,
        owner_user_id: &str,
    ) -> ScheduledTaskDoc {
        let now = bson::DateTime::now();
        ScheduledTaskDoc {
            id: None,
            task_id: "task-1".to_string(),
            team_id: "team-1".to_string(),
            channel_id: "channel-1".to_string(),
            owner_user_id: owner_user_id.to_string(),
            agent_id: "agent-1".to_string(),
            title: "Task".to_string(),
            prompt: "Prompt".to_string(),
            task_kind: super::super::models::ScheduledTaskKind::OneShot,
            task_profile: ScheduledTaskProfile::WorkspaceTask,
            payload_kind: ScheduledTaskPayloadKind::SystemSummary,
            session_binding: ScheduledTaskSessionBinding::IsolatedTask,
            delivery_plan: ScheduledTaskDeliveryPlanKind::ChannelOnly,
            execution_contract: ScheduledTaskExecutionContract {
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
            delivery_tier,
            owner_session_id: None,
            one_shot_at: Some(bson::DateTime::from_chrono(
                Utc::now() + chrono::Duration::minutes(10),
            )),
            cron_expression: None,
            schedule_config: None,
            timezone: "UTC".to_string(),
            status: ScheduledTaskStatus::Draft,
            next_fire_at: None,
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

    #[test]
    fn durable_tasks_are_visible_to_admin() {
        let user = sample_user("viewer");
        let task = sample_task(ScheduledTaskDeliveryTier::Durable, "owner");
        assert!(can_access_task(&task, &user, true));
    }

    #[test]
    fn session_scoped_tasks_remain_owner_only_even_for_admin() {
        let user = sample_user("viewer");
        let task = sample_task(ScheduledTaskDeliveryTier::SessionScoped, "owner");
        assert!(!can_access_task(&task, &user, true));
    }

    #[test]
    fn session_scoped_owner_defaults_to_current_session() {
        let user = sample_user("viewer");
        assert_eq!(
            resolve_effective_owner_session_id(
                ScheduledTaskDeliveryTier::SessionScoped,
                None,
                &user,
                None,
            ),
            Some("session-viewer".to_string())
        );
    }

    #[test]
    fn durable_tasks_clear_owner_session_binding() {
        let user = sample_user("viewer");
        assert_eq!(
            resolve_effective_owner_session_id(
                ScheduledTaskDeliveryTier::Durable,
                None,
                &user,
                Some("existing-session"),
            ),
            None
        );
    }
}
