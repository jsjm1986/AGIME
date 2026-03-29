//! Agent service layer for business logic (MongoDB version)

use super::mission_mongo::{
    resolve_execution_profile, resolve_launch_policy, ApprovalPolicy, AttemptRecord, CreateMissionRequest, GoalNode, GoalStatus,
    ListMissionsQuery, MissionArtifactDoc, MissionCompletionAssessment,
    MissionCompletionDisposition, MissionConvergencePatch, MissionDeliverableRequirement,
    MissionDeliverableRequirementMode, MissionDeliverableRequirementWhen, MissionDeliveryManifest,
    MissionDeliveryState, MissionDoc, MissionEventDoc, MissionExecutionLease,
    MissionHarnessVersion, MissionListItem, MissionMonitorIntervention, MissionProgressMemory,
    MissionStatus, MissionStep, ProgressSignal, RuntimeContract, RuntimeContractVerification,
    StepStatus, StepSupervisorState, WorkerCompactState, normalize_concrete_deliverable_paths,
};
use super::harness_core::{
    ArtifactMemory, ProjectMemory, RunCheckpoint, RunCheckpointKind, RunJournal, RunLease,
    RunMemory, RunState, RunStatus, SubagentRun, TaskGraph, TurnOutcome,
};
use super::hook_runtime;
use super::normalize_workspace_path;
use super::provider_factory;
use super::runtime;
use super::session_mongo::{
    AgentSessionDoc, ChatEventDoc, CreateSessionRequest, SessionListItem, SessionListQuery,
    UserSessionListQuery,
};
use super::task_manager::StreamEvent;
use agime::agents::types::RetryConfig;
use agime::conversation::message::{Message, MessageContent};
use agime::providers::base::Provider;
use agime_team::models::mongo::Document as TeamDocument;
use agime_team::models::{
    AgentExtensionConfig, AgentSkillConfig, AgentStatus, AgentTask, ApiFormat, BuiltinExtension,
    CreateAgentRequest, CustomExtensionConfig, ListAgentsQuery, ListTasksQuery, PaginatedResponse,
    SubmitTaskRequest, TaskResult, TaskResultType, TaskStatus, TaskType, TeamAgent,
    UpdateAgentRequest,
};
use agime_team::MongoDb;
use chrono::{DateTime, Utc};
use futures::{StreamExt, TryStreamExt};
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

/// Custom serde module for Option<DateTime<Utc>> with BSON datetime
mod bson_datetime_option {
    use chrono::{DateTime, Utc};
    use serde::{self, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(date: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match date {
            Some(dt) => {
                let bson_dt = bson::DateTime::from_chrono(*dt);
                Serialize::serialize(&bson_dt, serializer)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<bson::DateTime> = Option::deserialize(deserializer)?;
        Ok(opt.map(|dt| dt.to_chrono()))
    }
}

/// Validation error for agent operations
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Name is required and must be 1-100 characters")]
    Name,
    #[error("Invalid API URL format")]
    ApiUrl,
    #[error("Model name must be 1-100 characters")]
    Model,
    #[error("Priority must be between 0 and 100")]
    Priority,
    #[error("Invalid extension config: missing uri_or_cmd (or legacy uriOrCmd/command)")]
    ExtensionConfig,
    #[error(
        "Invalid custom extension type. Use one of: stdio | sse | streamable_http | streamablehttp"
    )]
    CustomExtensionType,
    #[error(
        "Mission tasks can only be created with a standard general-purpose agent. Select a general/default agent instead of a derived avatar, ecosystem, manager, or service agent"
    )]
    MissionAgentSelection,
}

/// Service error that includes both database and validation errors
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("Database error: {0}")]
    Database(#[from] mongodb::error::Error),
    #[error("Validation error: {0}")]
    Validation(#[from] ValidationError),
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Validate agent name
fn validate_name(name: &str) -> Result<(), ValidationError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 100 {
        return Err(ValidationError::Name);
    }
    Ok(())
}

/// Validate API URL format
fn validate_api_url(url: &Option<String>) -> Result<(), ValidationError> {
    if let Some(ref u) = url {
        let trimmed = u.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("http://")
            && !trimmed.starts_with("https://")
        {
            return Err(ValidationError::ApiUrl);
        }
    }
    Ok(())
}

/// Validate model name
fn validate_model(model: &Option<String>) -> Result<(), ValidationError> {
    if let Some(ref m) = model {
        let trimmed = m.trim();
        if trimmed.len() > 100 {
            return Err(ValidationError::Model);
        }
    }
    Ok(())
}

/// Validate priority
fn validate_priority(priority: i32) -> Result<(), ValidationError> {
    if !(0..=100).contains(&priority) {
        return Err(ValidationError::Priority);
    }
    Ok(())
}

/// Serialize custom extension configs for MongoDB persistence.
/// We build BSON explicitly so secret envs are stored in DB even though
/// API responses keep envs redacted via `skip_serializing`.
fn custom_extension_to_bson_document(ext: &CustomExtensionConfig) -> Document {
    let args = ext
        .args
        .iter()
        .map(|arg| Bson::String(arg.clone()))
        .collect::<Vec<_>>();

    let envs = ext
        .envs
        .iter()
        .map(|(k, v)| (k.clone(), Bson::String(v.clone())))
        .collect::<Document>();

    let mut doc = doc! {
        "name": ext.name.clone(),
        "type": ext.ext_type.clone(),
        "uri_or_cmd": ext.uri_or_cmd.clone(),
        "args": args,
        "envs": envs,
        "enabled": ext.enabled,
    };

    if let Some(source) = &ext.source {
        doc.insert("source", source.clone());
    }
    if let Some(source_extension_id) = &ext.source_extension_id {
        doc.insert("source_extension_id", source_extension_id.clone());
    }

    doc
}

fn custom_extensions_to_bson(exts: &[CustomExtensionConfig]) -> Bson {
    Bson::Array(
        exts.iter()
            .map(|ext| Bson::Document(custom_extension_to_bson_document(ext)))
            .collect::<Vec<_>>(),
    )
}

fn validate_custom_extension_name(name: &str) -> Result<(), ValidationError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 120 {
        return Err(ValidationError::Name);
    }
    Ok(())
}

fn normalize_custom_extension_type(ext_type: &str) -> Result<String, ValidationError> {
    let normalized = ext_type
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .replace(' ', "_");
    match normalized.as_str() {
        "stdio" => Ok("stdio".to_string()),
        "sse" => Ok("sse".to_string()),
        "streamablehttp" | "streamable_http" => Ok("streamable_http".to_string()),
        _ => Err(ValidationError::CustomExtensionType),
    }
}

fn normalize_custom_extension_config(
    extension: CustomExtensionConfig,
) -> Result<CustomExtensionConfig, ValidationError> {
    validate_custom_extension_name(&extension.name)?;

    let uri_or_cmd = extension.uri_or_cmd.trim().to_string();
    if uri_or_cmd.is_empty() {
        return Err(ValidationError::ExtensionConfig);
    }

    let args = extension
        .args
        .into_iter()
        .map(|arg| arg.trim().to_string())
        .filter(|arg| !arg.is_empty())
        .collect::<Vec<_>>();

    let envs = extension
        .envs
        .into_iter()
        .filter_map(|(key, value)| {
            let normalized_key = key.trim().to_string();
            if normalized_key.is_empty() {
                return None;
            }
            Some((normalized_key, value))
        })
        .collect::<HashMap<_, _>>();

    let source = extension
        .source
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| Some("custom".to_string()));
    let source_extension_id = extension
        .source_extension_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    Ok(CustomExtensionConfig {
        name: extension.name.trim().to_string(),
        ext_type: normalize_custom_extension_type(&extension.ext_type)?,
        uri_or_cmd,
        args,
        envs,
        enabled: extension.enabled,
        source,
        source_extension_id,
    })
}

fn custom_extension_name_eq(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn custom_extension_matches_source_extension(
    extension: &CustomExtensionConfig,
    source_extension_id: &str,
) -> bool {
    extension
        .source_extension_id
        .as_deref()
        .map(|value| value == source_extension_id)
        .unwrap_or(false)
}

fn delivery_state_from_assessment(
    assessment: &MissionCompletionAssessment,
) -> MissionDeliveryState {
    match assessment.disposition {
        MissionCompletionDisposition::Complete => MissionDeliveryState::Complete,
        MissionCompletionDisposition::CompletedWithMinorGaps => {
            MissionDeliveryState::CompletedWithMinorGaps
        }
        MissionCompletionDisposition::PartialHandoff => MissionDeliveryState::PartialHandoff,
        MissionCompletionDisposition::BlockedByEnvironment => {
            MissionDeliveryState::BlockedByEnvironment
        }
        MissionCompletionDisposition::BlockedByTooling => MissionDeliveryState::BlockedByTooling,
        MissionCompletionDisposition::WaitingExternal => MissionDeliveryState::WaitingExternal,
        MissionCompletionDisposition::BlockedFail => MissionDeliveryState::BlockedByTooling,
    }
}

fn effective_delivery_state(mission: &MissionDoc) -> Option<MissionDeliveryState> {
    mission
        .delivery_state
        .clone()
        .or_else(|| mission.completion_assessment.as_ref().map(delivery_state_from_assessment))
}

fn collect_requested_deliverables_from_steps(steps: &[MissionStep]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for step in steps {
        for path in &step.required_artifacts {
            if let Some(normalized) = runtime::normalize_relative_workspace_path(path) {
                if !is_process_only_deliverable(&normalized) && seen.insert(normalized.clone()) {
                    values.push(normalized);
                }
            }
        }
        if let Some(contract) = &step.runtime_contract {
            for path in &contract.required_artifacts {
                if let Some(normalized) = runtime::normalize_relative_workspace_path(path) {
                    if !is_process_only_deliverable(&normalized) && seen.insert(normalized.clone()) {
                        values.push(normalized);
                    }
                }
            }
        }
    }
    values
}

fn is_process_only_deliverable(path: &str) -> bool {
    let lower = path.trim().replace('\\', "/").to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    lower.starts_with("evidence/")
        || lower.contains("/evidence/")
        || lower.starts_with("quality/")
        || lower.contains("/quality/")
        || lower.starts_with("_evidence_")
        || lower.ends_with(".log")
        || lower.ends_with("/run.log")
}

fn collect_explicit_deliverables_from_goal_text(goal: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for raw in goal.split(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                ',' | '，'
                    | '、'
                    | '。'
                    | ';'
                    | '；'
                    | ':'
                    | '：'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '【'
                    | '】'
                    | '"'
                    | '\''
                    | '“'
                    | '”'
                    | '‘'
                    | '’'
            )
    }) {
        let trimmed = raw.trim_matches(|ch: char| ch == '.' || ch == '。');
        if trimmed.is_empty() {
            continue;
        }
        let Some(normalized) = runtime::normalize_relative_workspace_path(trimmed) else {
            continue;
        };
        if normalize_concrete_deliverable_paths(std::slice::from_ref(&normalized)).is_empty() {
            continue;
        }
        if !is_process_only_deliverable(&normalized) && seen.insert(normalized.clone()) {
            values.push(normalized);
        }
    }
    let has_prefixed_paths = values.iter().any(|item| item.contains('/'));
    if has_prefixed_paths {
        values.retain(|item| item.contains('/') || looks_like_standalone_goal_deliverable(item));
    }
    values
}

#[derive(Debug, Clone, Deserialize)]
struct V4AiStrategySelection {
    execution_mode: super::mission_mongo::ExecutionMode,
    execution_profile: super::mission_mongo::ExecutionProfile,
    launch_policy: super::mission_mongo::LaunchPolicy,
    #[serde(default)]
    reason: Option<String>,
}

fn extract_json_object_block(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then_some(&text[start..=end])
}

async fn call_provider_complete_with_timeout(
    provider: &Arc<dyn Provider>,
    system: &str,
    user: &str,
) -> Result<String, ServiceError> {
    let messages = vec![Message::user().with_text(user.to_string())];
    const MAX_STRATEGY_RETRIES: usize = 3;
    for attempt in 1..=MAX_STRATEGY_RETRIES {
        let stream_attempt = provider.stream(system, &messages, &[]).await;
        match stream_attempt {
            Ok(mut stream) => {
                let mut accumulated_text = String::new();
                let mut stream_error: Option<String> = None;
                loop {
                    match tokio::time::timeout(std::time::Duration::from_secs(20), stream.next())
                        .await
                    {
                        Ok(Some(Ok((msg_opt, _usage_opt)))) => {
                            if let Some(msg) = msg_opt {
                                for part in msg.content {
                                    if let MessageContent::Text(tc) = part {
                                        if !tc.text.is_empty() {
                                            accumulated_text.push_str(&tc.text);
                                        }
                                    }
                                }
                            }
                        }
                        Ok(Some(Err(err))) => {
                            stream_error = Some(err.to_string());
                            break;
                        }
                        Ok(None) => break,
                        Err(_) => {
                            stream_error = Some(
                                "provider stream timed out during V4 strategy resolution".to_string(),
                            );
                            break;
                        }
                    }
                }

                if !accumulated_text.trim().is_empty() {
                    return Ok(accumulated_text);
                }
                if let Some(err_text) = stream_error {
                    if runtime::is_non_retryable_external_provider_message(&err_text) {
                        return Err(ServiceError::Internal(format!(
                            "provider stream failed: {}",
                            err_text
                        )));
                    }
                    if attempt < MAX_STRATEGY_RETRIES
                        && (runtime::is_transient_provider_execution_message(&err_text)
                            || runtime::is_waiting_external_provider_message(&err_text)
                            || runtime::is_retryable_error(&anyhow::anyhow!(err_text.clone())))
                    {
                        let delay_ms =
                            (1000u64.saturating_mul(1u64 << (attempt - 1) as u32)).min(8000);
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                        continue;
                    }
                    if attempt >= MAX_STRATEGY_RETRIES {
                        return Err(ServiceError::Internal(format!(
                            "provider stream failed after {} attempts: {}",
                            attempt, err_text
                        )));
                    }
                }
            }
            Err(stream_err) => {
                let err_text = stream_err.to_string();
                if runtime::is_non_retryable_external_provider_message(&err_text) {
                    return Err(ServiceError::Internal(format!(
                        "provider stream failed: {}",
                        err_text
                    )));
                }
            }
        }

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            provider.complete(system, &messages, &[]),
        )
        .await;
        match result {
            Ok(Ok((response, _usage))) => {
                let text = response.as_concat_text();
                if !text.trim().is_empty() {
                    return Ok(text);
                }
                if attempt >= MAX_STRATEGY_RETRIES {
                    return Err(ServiceError::Internal(format!(
                        "provider complete returned empty output after {} attempts",
                        attempt
                    )));
                }
            }
            Ok(Err(err)) => {
                let err_text = err.to_string();
                if runtime::is_non_retryable_external_provider_message(&err_text) {
                    return Err(ServiceError::Internal(format!(
                        "provider call failed: {}",
                        err_text
                    )));
                }
                if attempt >= MAX_STRATEGY_RETRIES {
                    return Err(ServiceError::Internal(format!(
                        "provider call failed after {} attempts: {}",
                        attempt, err_text
                    )));
                }
                if !(runtime::is_transient_provider_execution_message(&err_text)
                    || runtime::is_waiting_external_provider_message(&err_text)
                    || runtime::is_retryable_error(&anyhow::anyhow!(err_text.clone())))
                {
                    return Err(ServiceError::Internal(format!(
                        "provider call failed: {}",
                        err_text
                    )));
                }
            }
            Err(_) => {
                if attempt >= MAX_STRATEGY_RETRIES {
                    return Err(ServiceError::Internal(format!(
                        "provider call timed out during V4 strategy resolution after {} attempts",
                        attempt
                    )));
                }
            }
        }
        let delay_ms = (1000u64.saturating_mul(1u64 << (attempt - 1) as u32)).min(8000);
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
    }
    Err(ServiceError::Internal(
        "provider call exhausted retries during V4 strategy resolution".to_string(),
    ))
}

async fn call_openai_strategy_resolution_with_timeout(
    agent: &TeamAgent,
    system: &str,
    user: &str,
) -> Result<String, ServiceError> {
    let api_url = agent.api_url.as_deref().ok_or_else(|| {
        ServiceError::Internal("strategy resolution agent is missing api_url".to_string())
    })?;
    let api_key = agent.api_key.as_deref().ok_or_else(|| {
        ServiceError::Internal("strategy resolution agent is missing api_key".to_string())
    })?;
    let endpoint = format!("{}/v1/chat/completions", api_url.trim_end_matches('/'));
    let payload = serde_json::json!({
        "model": agent.model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        "max_tokens": 64,
        "temperature": 0.0,
        "stream": false,
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .map_err(|err| ServiceError::Internal(format!(
            "failed to build HTTP client for V4 strategy resolution: {}",
            err
        )))?;
    const MAX_STRATEGY_RETRIES: usize = 3;
    for attempt in 1..=MAX_STRATEGY_RETRIES {
        let response = client
            .post(&endpoint)
            .bearer_auth(api_key)
            .json(&payload)
            .send()
            .await;
        match response {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                if status.is_success() {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
                        if let Some(content) = value
                            .get("choices")
                            .and_then(|choices| choices.get(0))
                            .and_then(|choice| choice.get("message"))
                            .and_then(|message| message.get("content"))
                        {
                            if let Some(text) = content.as_str() {
                                if !text.trim().is_empty() {
                                    return Ok(text.to_string());
                                }
                            } else if let Some(parts) = content.as_array() {
                                let merged = parts
                                    .iter()
                                    .filter_map(|part| {
                                        part.get("text")
                                            .and_then(serde_json::Value::as_str)
                                            .or_else(|| part.as_str())
                                    })
                                    .collect::<String>();
                                if !merged.trim().is_empty() {
                                    return Ok(merged);
                                }
                            }
                        }
                    }
                    if attempt >= MAX_STRATEGY_RETRIES {
                        return Err(ServiceError::Internal(
                            "strategy resolution HTTP call returned empty content".to_string(),
                        ));
                    }
                } else {
                    let err_text = if body.trim().is_empty() {
                        format!("HTTP {}", status)
                    } else {
                        format!("HTTP {}: {}", status, body)
                    };
                    if runtime::is_non_retryable_external_provider_message(&err_text) {
                        return Err(ServiceError::Internal(format!(
                            "provider stream failed: {}",
                            err_text
                        )));
                    }
                    if attempt >= MAX_STRATEGY_RETRIES
                        || !(runtime::is_transient_provider_execution_message(&err_text)
                            || runtime::is_waiting_external_provider_message(&err_text))
                    {
                        return Err(ServiceError::Internal(format!(
                            "provider stream failed after {} attempts: {}",
                            attempt, err_text
                        )));
                    }
                }
            }
            Err(err) => {
                let err_text = err.to_string();
                if runtime::is_non_retryable_external_provider_message(&err_text) {
                    return Err(ServiceError::Internal(format!(
                        "provider stream failed: {}",
                        err_text
                    )));
                }
                if attempt >= MAX_STRATEGY_RETRIES
                    || !(runtime::is_transient_provider_execution_message(&err_text)
                        || runtime::is_waiting_external_provider_message(&err_text)
                        || runtime::is_retryable_error(&anyhow::anyhow!(err_text.clone())))
                {
                    return Err(ServiceError::Internal(format!(
                        "provider stream failed after {} attempts: {}",
                        attempt, err_text
                    )));
                }
            }
        }
        let delay_ms = (1000u64.saturating_mul(1u64 << (attempt - 1) as u32)).min(8000);
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
    }
    Err(ServiceError::Internal(
        "provider call exhausted retries during V4 strategy resolution".to_string(),
    ))
}

fn looks_like_standalone_goal_deliverable(path: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty() || normalized.contains('/') {
        return false;
    }
    let lower = normalized.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "readme.md"
            | "requirements.txt"
            | "package.json"
            | "cargo.toml"
            | "pyproject.toml"
            | "dockerfile"
            | "makefile"
            | "license"
            | "license.md"
    ) {
        return true;
    }
    let path = std::path::Path::new(&normalized);
    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return false;
    };
    !stem.is_empty()
        && stem.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-'
        })
}

fn normalize_requested_deliverables(items: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for item in items {
        if let Some(normalized) = runtime::normalize_relative_workspace_path(item) {
            let key = normalized.to_ascii_lowercase();
            if seen.insert(key) {
                values.push(normalized);
            }
        }
    }
    let scoped_basenames = values
        .iter()
        .filter(|value| value.contains('/'))
        .filter_map(|value| value.rsplit('/').next())
        .map(str::to_ascii_lowercase)
        .collect::<HashSet<_>>();
    if !scoped_basenames.is_empty() {
        values.retain(|value| {
            value.contains('/')
                || !scoped_basenames.contains(&value.to_ascii_lowercase())
                || matches!(
                    value.to_ascii_lowercase().as_str(),
                    "readme.md"
                        | "requirements.txt"
                        | "package.json"
                        | "cargo.toml"
                        | "pyproject.toml"
                        | "dockerfile"
                        | "makefile"
                        | "license"
                        | "license.md"
                )
        });
    }
    values
}

fn manifest_requested_gap(manifest: &MissionDeliveryManifest) -> Vec<String> {
    let requested = normalize_requested_deliverables(&manifest.requested_deliverables);
    let satisfied = normalize_requested_deliverables(&manifest.satisfied_deliverables);
    let satisfied_candidates = satisfied
        .iter()
        .map(|item| artifact_match_candidates(item))
        .collect::<Vec<_>>();
    requested
        .into_iter()
        .filter(|requested_item| {
            let requested_candidates = artifact_match_candidates(requested_item);
            !satisfied_candidates.iter().any(|candidates| {
                candidates.iter().any(|candidate| requested_candidates.iter().any(|req| req == candidate))
            })
        })
        .collect()
}

fn deliverable_paths_overlap(left: &str, right: &str) -> bool {
    let left_candidates = artifact_match_candidates(left);
    let right_candidates = artifact_match_candidates(right);
    left_candidates
        .iter()
        .any(|candidate| right_candidates.iter().any(|other| other == candidate))
}

fn terminal_progress_memory(mission: &MissionDoc) -> Option<MissionProgressMemory> {
    if mission.status != MissionStatus::Completed {
        return None;
    }

    let done = mission
        .delivery_manifest
        .as_ref()
        .map(|manifest| normalize_concrete_deliverable_paths(&manifest.satisfied_deliverables))
        .filter(|items| !items.is_empty())
        .or_else(|| {
            mission
                .completion_assessment
                .as_ref()
                .map(|assessment| {
                    normalize_concrete_deliverable_paths(&assessment.observed_evidence)
                })
                .filter(|items| !items.is_empty())
        })
        .or_else(|| {
            mission
                .latest_worker_state
                .as_ref()
                .map(|state| normalize_concrete_deliverable_paths(&state.core_assets_now))
                .filter(|items| !items.is_empty())
        })
        .or_else(|| {
            mission
                .progress_memory
                .as_ref()
                .map(|memory| normalize_concrete_deliverable_paths(&memory.done))
                .filter(|items| !items.is_empty())
        })
        .unwrap_or_default();

    Some(MissionProgressMemory {
        done,
        missing: Vec::new(),
        blocked_by: None,
        last_failed_attempt: None,
        next_best_action: None,
        confidence: None,
        updated_at: mission.completed_at.or(Some(bson::DateTime::now())),
    })
}

fn build_progress_memory(mission: &MissionDoc) -> Option<MissionProgressMemory> {
    if let Some(memory) = terminal_progress_memory(mission) {
        return Some(memory);
    }

    let manifest_gap = mission
        .delivery_manifest
        .as_ref()
        .map(manifest_requested_gap)
        .unwrap_or_default();
    let manifest_gap = normalize_concrete_deliverable_paths(&manifest_gap);
    let manifest_has_requested_truth = mission
        .delivery_manifest
        .as_ref()
        .is_some_and(|manifest| !normalize_requested_deliverables(&manifest.requested_deliverables).is_empty());

    let done = mission
        .delivery_manifest
        .as_ref()
        .map(|manifest| normalize_concrete_deliverable_paths(&manifest.satisfied_deliverables))
        .filter(|items| !items.is_empty())
        .or_else(|| {
            mission
                .latest_worker_state
                .as_ref()
                .map(|state| normalize_concrete_deliverable_paths(&state.core_assets_now))
                .filter(|items| !items.is_empty())
        })
        .unwrap_or_default();

    let mut missing = if manifest_has_requested_truth {
        manifest_gap.clone()
    } else {
        mission
            .delivery_manifest
            .as_ref()
            .map(|_| manifest_gap.clone())
            .filter(|items| !items.is_empty())
            .or_else(|| {
                mission
                    .completion_assessment
                    .as_ref()
                    .map(|assessment| normalize_concrete_deliverable_paths(&assessment.missing_core_deliverables))
                    .filter(|items| !items.is_empty())
            })
            .or_else(|| (!manifest_gap.is_empty()).then_some(manifest_gap.clone()))
            .unwrap_or_default()
    };
    if !done.is_empty() && !missing.is_empty() {
        missing.retain(|path| !done.iter().any(|done_path| deliverable_paths_overlap(path, done_path)));
    }

    let blocked_by = mission
        .latest_worker_state
        .as_ref()
        .and_then(|state| state.current_blocker.clone())
        .or_else(|| mission.error_message.clone())
        .filter(|reason| {
            !reason.starts_with("Server restarted while mission was in progress")
                && !reason.starts_with("Mission supervisor detected no activity")
        })
        .map(|reason| {
            let trimmed = reason.trim();
            if let Some(rest) = trimmed.strip_prefix("Task ") {
                if let Some((_, blocker)) = rest.split_once(" failed: ") {
                    return blocker.trim().to_string();
                }
            }
            trimmed.to_string()
        });

    let last_failed_attempt = mission
        .latest_worker_state
        .as_ref()
        .and_then(|state| state.method_summary.clone())
        .or_else(|| blocked_by.as_ref().map(|blocker| format!("blocked: {}", blocker)));

    let mut next_best_action = mission
        .latest_worker_state
        .as_ref()
        .and_then(|state| state.next_step_candidate.clone());

    let current_node_target = current_node_preferred_target(mission)
        .filter(|target| missing.iter().any(|path| deliverable_paths_overlap(path, target)));
    let preferred_missing_target = current_node_target
        .clone()
        .or_else(|| missing.first().cloned());

    if let Some(target_file) = current_node_target.as_ref() {
        let inputs = done.iter().take(3).cloned().collect::<Vec<_>>();
        next_best_action = Some(if inputs.is_empty() {
            format!(
                "Create or materially update {} first in this round, then run one minimal validation on that file.",
                target_file
            )
        } else {
            format!(
                "Use the existing completed outputs ({}) to create or materially update {} next, then run one minimal validation on that file.",
                inputs.join(", "),
                target_file
            )
        });
    } else if mission.current_goal_id.is_some()
        && !missing.is_empty()
    {
        let goal_title = mission
            .goal_tree
            .as_ref()
            .and_then(|goals| {
                mission.current_goal_id.as_deref().and_then(|goal_id| {
                    goals.iter()
                        .find(|goal| goal.goal_id == goal_id)
                        .map(|goal| goal.title.clone())
                })
            })
            .unwrap_or_else(|| "current goal".to_string());
        next_best_action = Some(format!(
            "Advance {} with a concrete workspace delta that moves the mission toward the remaining deliverables ({}). Do not invent a locked file target unless the node contract explicitly requires one.",
            goal_title,
            missing.join(", ")
        ));
    } else if done.is_empty() && !missing.is_empty() {
        next_best_action = Some(format!(
            "Create or materially update {} first in this round, then run one minimal validation on that file.",
            preferred_missing_target.as_deref().unwrap_or(&missing[0])
        ));
    } else if !done.is_empty() && !missing.is_empty() {
        let should_focus_on_first_missing = next_best_action.is_none()
            || blocked_by
                .as_deref()
                .is_some_and(|text| text.to_ascii_lowercase().contains("no tool calls"));
        if should_focus_on_first_missing {
            let inputs = done.iter().take(3).cloned().collect::<Vec<_>>();
            next_best_action = Some(format!(
                "Use the existing completed outputs ({}) to create or materially update {} next, then run one minimal validation on that file.",
                inputs.join(", "),
                preferred_missing_target.as_deref().unwrap_or(&missing[0])
            ));
        }
    }

    let confidence = None;

    let updated_at = mission
        .latest_worker_state
        .as_ref()
        .and_then(|state| state.recorded_at)
        .or_else(|| {
            mission
                .completion_assessment
                .as_ref()
                .and_then(|assessment| assessment.recorded_at)
        })
        .or_else(|| Some(mission.updated_at));

    if done.is_empty()
        && missing.is_empty()
        && blocked_by.is_none()
        && last_failed_attempt.is_none()
        && next_best_action.is_none()
        && confidence.is_none()
    {
        return None;
    }

    Some(MissionProgressMemory {
        done,
        missing,
        blocked_by,
        last_failed_attempt,
        next_best_action,
        confidence,
        updated_at,
    })
}

fn progress_memory_semantically_equal(
    left: &MissionProgressMemory,
    right: &MissionProgressMemory,
) -> bool {
    left.done == right.done
        && left.missing == right.missing
        && left.blocked_by == right.blocked_by
        && left.last_failed_attempt == right.last_failed_attempt
        && left.next_best_action == right.next_best_action
        && left.confidence == right.confidence
}

fn assessment_implies_terminal_progress(assessment: &MissionCompletionAssessment) -> bool {
    matches!(
        assessment.disposition,
        MissionCompletionDisposition::Complete
            | MissionCompletionDisposition::CompletedWithMinorGaps
            | MissionCompletionDisposition::PartialHandoff
    ) && assessment.missing_core_deliverables.is_empty()
}

fn worker_state_semantically_equal(left: &WorkerCompactState, right: &WorkerCompactState) -> bool {
    left.current_goal == right.current_goal
        && normalize_requested_deliverables(&left.core_assets_now)
            == normalize_requested_deliverables(&right.core_assets_now)
        && normalize_requested_deliverables(&left.assets_delta)
            == normalize_requested_deliverables(&right.assets_delta)
        && left.current_blocker == right.current_blocker
        && left.method_summary == right.method_summary
        && left.next_step_candidate == right.next_step_candidate
        && left.capability_signals == right.capability_signals
        && left.subtask_plan == right.subtask_plan
        && left.subtask_results_summary == right.subtask_results_summary
        && left.merge_risk == right.merge_risk
        && left.parallelism_used == right.parallelism_used
}

fn worker_state_has_material_progress(
    previous: Option<&WorkerCompactState>,
    next: Option<&WorkerCompactState>,
) -> bool {
    let Some(next) = next else {
        return false;
    };
    if !next.assets_delta.is_empty() {
        return true;
    }
    let next_assets = normalize_requested_deliverables(&next.core_assets_now);
    if next_assets.is_empty() {
        return false;
    }
    match previous {
        Some(previous) => normalize_requested_deliverables(&previous.core_assets_now) != next_assets,
        None => true,
    }
}

fn execution_lease_ttl_secs() -> i64 {
    std::env::var("TEAM_MISSION_EXECUTION_LEASE_SECS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(120)
}

fn execution_lease_active(lease: Option<&MissionExecutionLease>) -> bool {
    lease
        .and_then(|lease| lease.expires_at)
        .is_some_and(|expires_at| expires_at.to_chrono() > Utc::now())
}

fn mission_fast_reclaim_after_execution_stall(mission: &MissionDoc) -> bool {
    let Some(memory) = mission.progress_memory.as_ref() else {
        return false;
    };
    let Some(blocked_by) = memory.blocked_by.as_deref() else {
        return false;
    };
    let lower = blocked_by.to_ascii_lowercase();
    if !(lower.contains("no tool calls") || lower.contains("target file delta")) {
        return false;
    }
    let observed_at = memory
        .updated_at
        .map(|ts| ts.to_chrono())
        .unwrap_or_else(|| mission.updated_at.to_chrono());
    observed_at < (Utc::now() - chrono::Duration::seconds(30))
}

fn requirement_paths(requirement: &MissionDeliverableRequirement) -> Vec<String> {
    normalize_requested_deliverables(&requirement.paths)
}

fn requirement_is_active(
    mission: &MissionDoc,
    requirement: &MissionDeliverableRequirement,
) -> bool {
    match requirement.required_when {
        MissionDeliverableRequirementWhen::Always => true,
        MissionDeliverableRequirementWhen::BlockedByEnvironment => {
            mission.delivery_state == Some(MissionDeliveryState::BlockedByEnvironment)
                || mission.completion_assessment.as_ref().is_some_and(|assessment| {
                    assessment.disposition == MissionCompletionDisposition::BlockedByEnvironment
                })
        }
        MissionDeliverableRequirementWhen::BlockedByTooling => {
            mission.delivery_state == Some(MissionDeliveryState::BlockedByTooling)
                || mission.completion_assessment.as_ref().is_some_and(|assessment| {
                    assessment.disposition == MissionCompletionDisposition::BlockedByTooling
                })
        }
        MissionDeliverableRequirementWhen::VerificationFailed => mission
            .completion_assessment
            .as_ref()
            .is_some_and(|assessment| {
                matches!(
                    assessment.disposition,
                    MissionCompletionDisposition::PartialHandoff
                        | MissionCompletionDisposition::CompletedWithMinorGaps
                )
            }),
    }
}

fn active_requested_deliverables_from_manifest(mission: &MissionDoc) -> Vec<String> {
    let Some(manifest) = mission.delivery_manifest.as_ref() else {
        return Vec::new();
    };
    if manifest.requirements.is_empty() {
        return normalize_requested_deliverables(&manifest.requested_deliverables);
    }

    let mut seen = HashSet::new();
    let mut requested = Vec::new();
    for requirement in &manifest.requirements {
        if requirement_is_active(mission, requirement) {
            for path in requirement_paths(requirement) {
                if seen.insert(path.clone()) {
                    requested.push(path);
                }
            }
        }
    }
    requested
}

fn current_node_preferred_target(mission: &MissionDoc) -> Option<String> {
    if let Some(step_index) = mission.current_step {
        let step = mission.steps.iter().find(|step| step.index == step_index)?;
        let runtime_required = step
            .runtime_contract
            .as_ref()
            .map(|contract| normalize_concrete_deliverable_paths(&contract.required_artifacts))
            .filter(|items| !items.is_empty());
        return runtime_required
            .and_then(|items| super::mission_mongo::first_concrete_deliverable(&items).or_else(|| items.first().cloned()))
            .or_else(|| {
                let required = normalize_concrete_deliverable_paths(&step.required_artifacts);
                super::mission_mongo::first_concrete_deliverable(&required).or_else(|| required.first().cloned())
            });
    }

    let goal_id = mission.current_goal_id.as_deref()?;
    let goal = mission
        .goal_tree
        .as_ref()
        .and_then(|goals| goals.iter().find(|goal| goal.goal_id == goal_id))?;
    let runtime_required = goal
        .runtime_contract
        .as_ref()
        .map(|contract| normalize_concrete_deliverable_paths(&contract.required_artifacts))
        .filter(|items| !items.is_empty());
    runtime_required
        .and_then(|items| super::mission_mongo::first_concrete_deliverable(&items).or_else(|| items.first().cloned()))
        .or_else(|| {
            let requested = active_requested_deliverables_from_manifest(mission);
            (requested.len() == 1).then(|| requested.first().cloned()).flatten()
        })
}

fn mission_uses_research_quality_gate(mission: &MissionDoc) -> bool {
    let goal = mission.goal.to_ascii_lowercase();
    let keyword_match = ["research", "compare", "comparison", "调研", "比较", "对比"]
        .iter()
        .any(|needle| goal.contains(needle));
    if !keyword_match {
        return false;
    }
    mission
        .delivery_manifest
        .as_ref()
        .map(|manifest| {
            manifest.requested_deliverables.iter().any(|item| {
                let lower = item.to_ascii_lowercase();
                lower.ends_with("report.md")
                    || lower.ends_with("comparison.md")
                    || lower.ends_with("summary.csv")
                    || lower.ends_with("overview.html")
                    || lower.ends_with("index.html")
            })
        })
        .unwrap_or(false)
}

fn content_has_placeholder_signal(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    [
        "sample placeholder",
        "replace with researched values",
        "占位",
        "占位视图",
        "报告草稿",
        "draft report",
        "will be filled in later",
        "后续步骤会补全",
        "草稿",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn artifact_placeholder_scan_applies(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md")
        || lower.ends_with(".html")
        || lower.ends_with(".htm")
        || lower.ends_with(".csv")
        || lower.ends_with(".json")
        || lower.ends_with(".txt")
}

fn content_has_source_support(path: &str, content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    if lower.contains("http://") || lower.contains("https://") {
        return true;
    }

    let path = path.to_ascii_lowercase();
    if path.ends_with(".csv") {
        let header = lower.lines().next().unwrap_or_default();
        return header.split(',').any(|column| {
            let trimmed = column.trim();
            trimmed == "source" || trimmed == "sources" || trimmed == "citation" || trimmed == "citations"
        });
    }

    lower.contains("sources")
        || lower.contains("source:")
        || lower.contains("来源")
        || lower.contains("参考链接")
}

fn research_quality_blockers(
    mission: &MissionDoc,
    artifacts: &[MissionArtifactDoc],
    satisfied: &[String],
) -> Vec<String> {
    if !mission_uses_research_quality_gate(mission) {
        return Vec::new();
    }

    let mut blockers = Vec::new();
    let satisfied_set = satisfied
        .iter()
        .map(|value| value.trim().replace('\\', "/"))
        .collect::<HashSet<_>>();
    let mut saw_source_support = false;

    for artifact in artifacts {
        let raw_path = artifact
            .file_path
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| artifact.name.clone());
        let normalized = runtime::normalize_relative_workspace_path(&raw_path)
            .unwrap_or_else(|| raw_path.trim().replace('\\', "/"));
        if normalized.is_empty() || !satisfied_set.contains(&normalized) {
            continue;
        }
        let Some(content) = artifact.content.as_deref() else {
            continue;
        };
        if artifact_placeholder_scan_applies(&normalized) && content_has_placeholder_signal(content)
        {
            blockers.push(format!("placeholder_or_draft_content:{}", normalized));
        }
        if content_has_source_support(&normalized, content) {
            saw_source_support = true;
        }
    }

    if !saw_source_support {
        blockers.push("missing_source_support".to_string());
    }

    let mut seen = HashSet::new();
    blockers
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

fn artifact_match_candidates(value: &str) -> Vec<String> {
    let normalized = runtime::normalize_relative_workspace_path(value)
        .unwrap_or_else(|| value.trim().replace('\\', "/"));
    let mut candidates = Vec::new();
    if !normalized.is_empty() {
        candidates.push(normalized.clone());
        candidates.push(normalized.to_ascii_lowercase());
        if let Some(name) = normalized.rsplit('/').next() {
            candidates.push(name.to_string());
            candidates.push(name.to_ascii_lowercase());
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn extract_task_runtime_binding(
    content: &serde_json::Value,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let mission_id = content
        .get("mission_id")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string())
        .or_else(|| {
            content
                .get("mission_context")
                .and_then(|value| value.get("mission_id"))
                .and_then(serde_json::Value::as_str)
                .map(|value| value.to_string())
        });
    let run_id = content
        .get("run_id")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string())
        .or_else(|| {
            content
                .get("mission_context")
                .and_then(|value| value.get("current_run_id"))
                .and_then(serde_json::Value::as_str)
                .map(|value| value.to_string())
        });
    let session_id = content
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string());
    let task_role = content
        .get("task_role")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string())
        .or_else(|| mission_id.as_ref().map(|_| "mission_worker".to_string()));
    let task_node_id = content
        .get("task_node_id")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string())
        .or_else(|| {
            content
                .get("mission_context")
                .and_then(|value| value.get("task_node_id"))
                .and_then(serde_json::Value::as_str)
                .map(|value| value.to_string())
        });
    (mission_id, run_id, session_id, task_role, task_node_id)
}

fn task_filter_for_mission_run(
    mission_id: &str,
    run_id: Option<&str>,
) -> mongodb::bson::Document {
    let mut filter = doc! {
        "status": { "$in": ["pending", "approved", "running"] },
        "$and": [
            {
                "$or": [
                    { "task_role": "mission_worker" },
                    { "content.task_role": "mission_worker" },
                ]
            },
            {
                "$or": [
                    { "mission_id": mission_id },
                    { "content.mission_id": mission_id },
                ]
            }
        ],
    };
    if let Some(run_id) = run_id.filter(|value| !value.trim().is_empty()) {
        filter.insert(
            "$and",
            vec![
                doc! {
                    "$or": [
                        { "task_role": "mission_worker" },
                        { "content.task_role": "mission_worker" },
                    ]
                },
                doc! {
                    "$or": [
                        { "mission_id": mission_id },
                        { "content.mission_id": mission_id },
                    ]
                },
                doc! {
                    "$or": [
                        { "run_id": run_id },
                        { "content.run_id": run_id },
                    ]
                },
            ],
        );
    }
    filter
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::mission_mongo::{
        ApprovalPolicy, ArtifactType, ExecutionMode, ExecutionProfile, GoalNode, GoalStatus,
        LaunchPolicy, MissionHarnessVersion, RuntimeContract,
    };
    use crate::agent::runtime;
    use mongodb::bson::doc;
    use serde_json::json;
    use std::collections::HashMap;

    fn sample_mission_doc() -> MissionDoc {
        MissionDoc {
            id: None,
            mission_id: "mission-sample".to_string(),
            team_id: "team-1".to_string(),
            agent_id: "agent-1".to_string(),
            creator_id: "user-1".to_string(),
            goal: "Sample goal".to_string(),
            context: None,
            approval_policy: ApprovalPolicy::Auto,
            status: MissionStatus::Draft,
            steps: vec![MissionStep {
                index: 0,
                title: "Step 1".to_string(),
                description: "desc".to_string(),
                status: StepStatus::Pending,
                is_checkpoint: false,
                approved_by: None,
                started_at: None,
                completed_at: None,
                error_message: None,
                supervisor_state: None,
                last_activity_at: None,
                last_progress_at: None,
                progress_score: None,
                current_blocker: None,
                last_supervisor_hint: None,
                stall_count: 0,
                recent_progress_events: Vec::new(),
                evidence_bundle: None,
                tokens_used: 0,
                output_summary: None,
                retry_count: 0,
                max_retries: 2,
                timeout_seconds: None,
                required_artifacts: Vec::new(),
                completion_checks: Vec::new(),
                runtime_contract: None,
                contract_verification: None,
                use_subagent: false,
                tool_calls: Vec::new(),
            }],
            current_step: Some(0),
            session_id: Some("session-1".to_string()),
            source_chat_session_id: None,
            token_budget: 0,
            total_tokens_used: 0,
            priority: 0,
            step_timeout_seconds: None,
            step_max_retries: None,
            plan_version: 1,
            execution_mode: ExecutionMode::Sequential,
            execution_profile: ExecutionProfile::Auto,
            launch_policy: LaunchPolicy::Auto,
            harness_version: MissionHarnessVersion::V4,
            goal_tree: None,
            current_goal_id: None,
            total_pivots: 0,
            total_abandoned: 0,
            error_message: None,
            final_summary: None,
            delivery_state: Some(MissionDeliveryState::Working),
            delivery_manifest: Some(MissionDeliveryManifest {
                requirements: Vec::new(),
                requested_deliverables: Vec::new(),
                satisfied_deliverables: Vec::new(),
                missing_core_deliverables: Vec::new(),
                supporting_artifacts: Vec::new(),
                delivery_state: MissionDeliveryState::Working,
                final_outcome_summary: None,
            }),
            progress_memory: None,
            completion_assessment: None,
            created_at: bson::DateTime::now(),
            updated_at: bson::DateTime::now(),
            started_at: None,
            completed_at: None,
            attached_document_ids: Vec::new(),
            workspace_path: Some("/tmp/workspace".to_string()),
            current_run_id: Some("run-1".to_string()),
            pending_monitor_intervention: None,
            last_applied_monitor_intervention: None,
            latest_worker_state: None,
            active_repair_lane_id: None,
            consecutive_no_tool_count: 0,
            last_blocker_fingerprint: None,
            waiting_external_until: None,
            execution_lease: None,
        }
    }

    #[test]
    fn custom_extension_bson_keeps_envs_and_type_field() {
        let mut envs = HashMap::new();
        envs.insert("API_TOKEN".to_string(), "secret-token".to_string());

        let ext = CustomExtensionConfig {
            name: "demo_mcp".to_string(),
            ext_type: "stdio".to_string(),
            uri_or_cmd: "demo-cmd".to_string(),
            args: vec!["--foo".to_string()],
            envs,
            enabled: true,
            source: Some("team".to_string()),
            source_extension_id: Some("ext-id".to_string()),
        };

        let doc = custom_extension_to_bson_document(&ext);
        let env_doc = doc.get_document("envs").expect("envs must exist");
        assert_eq!(doc.get_str("type").unwrap_or_default(), "stdio");
        assert_eq!(
            env_doc.get_str("API_TOKEN").unwrap_or_default(),
            "secret-token"
        );
    }

    #[test]
    fn governance_config_prefers_top_level_config_over_nested_config() {
        let doc = doc! {
            "settings": {
                "digitalAvatarGovernanceConfig": {
                    "autoProposalTriggerCount": 5,
                    "managerApprovalMode": "human_gate",
                },
                "digitalAvatarGovernance": {
                    "config": {
                        "autoProposalTriggerCount": 9,
                        "managerApprovalMode": "manager_decides",
                    }
                }
            }
        };

        let config = AgentService::governance_config_json_from_portal_doc(&doc);
        assert_eq!(config.get("autoProposalTriggerCount"), Some(&json!(5)));
        assert_eq!(
            config.get("managerApprovalMode"),
            Some(&json!("human_gate"))
        );
    }

    #[test]
    fn governance_config_falls_back_to_nested_config_when_top_level_missing() {
        let doc = doc! {
            "settings": {
                "digitalAvatarGovernance": {
                    "config": {
                        "autoProposalTriggerCount": 7,
                        "optimizationMode": "manager_only",
                    }
                }
            }
        };

        let config = AgentService::governance_config_json_from_portal_doc(&doc);
        assert_eq!(config.get("autoProposalTriggerCount"), Some(&json!(7)));
        assert_eq!(config.get("optimizationMode"), Some(&json!("manager_only")));
    }

    #[test]
    fn governance_config_uses_defaults_when_no_config_exists() {
        let doc = doc! {
            "settings": {
                "domain": "avatar"
            }
        };

        let config = AgentService::governance_config_json_from_portal_doc(&doc);
        assert_eq!(config, AgentService::default_governance_config_json());
    }

    #[test]
    fn governance_state_uses_defaults_when_state_missing() {
        let doc = doc! {
            "settings": {
                "domain": "avatar"
            }
        };

        let state = AgentService::governance_state_json_from_portal_doc(&doc);
        assert_eq!(state, AgentService::default_governance_state_json());
    }

    #[test]
    fn governance_state_strips_nested_config_from_legacy_governance_object() {
        let doc = doc! {
            "settings": {
                "digitalAvatarGovernance": {
                    "config": {
                        "autoProposalTriggerCount": 7
                    },
                    "capabilityRequests": [
                        { "id": "cap-1", "status": "pending" }
                    ]
                }
            }
        };

        let state = AgentService::governance_state_json_from_portal_doc(&doc);
        assert_eq!(
            state,
            json!({
                "capabilityRequests": [
                    { "id": "cap-1", "status": "pending" }
                ]
            })
        );
        assert_eq!(
            AgentService::raw_portal_governance_state_json(&doc),
            Some(json!({
                "capabilityRequests": [
                    { "id": "cap-1", "status": "pending" }
                ]
            }))
        );
    }

    #[test]
    fn governance_counts_only_include_pending_statuses_used_by_current_ui() {
        let state = json!({
            "capabilityRequests": [
                { "id": "cap-1", "status": "pending" },
                { "id": "cap-2", "status": "approved" }
            ],
            "gapProposals": [
                { "id": "proposal-1", "status": "pending_approval" },
                { "id": "proposal-2", "status": "approved" }
            ],
            "optimizationTickets": [
                { "id": "ticket-1", "status": "pending" },
                { "id": "ticket-2", "status": "deployed" }
            ],
            "runtimeLogs": [
                { "id": "runtime-1", "status": "pending" },
                { "id": "runtime-2", "status": "dismissed" }
            ]
        });

        let counts = AgentService::governance_counts_from_state_json(&state);
        assert_eq!(counts.pending_capability_requests, 1);
        assert_eq!(counts.pending_gap_proposals, 1);
        assert_eq!(counts.pending_optimization_tickets, 1);
        assert_eq!(counts.pending_runtime_logs, 1);
    }

    #[test]
    fn governance_event_payload_from_doc_uses_synthetic_id_when_object_id_missing() {
        let created_at = Utc::now();
        let payload = AvatarGovernanceEventPayload::from(AvatarGovernanceEventDoc {
            id: None,
            portal_id: "portal-1".to_string(),
            team_id: "team-1".to_string(),
            event_type: "created".to_string(),
            entity_type: "capability".to_string(),
            entity_id: Some("cap-1".to_string()),
            title: "新增能力请求".to_string(),
            status: Some("pending".to_string()),
            detail: None,
            actor_id: None,
            actor_name: Some("system".to_string()),
            meta: json!({ "risk": "medium" }),
            created_at,
        });

        assert!(payload
            .event_id
            .starts_with("synthetic:team-1:portal-1:created:capability:cap-1:"));
    }

    #[test]
    fn governance_event_payloads_from_snapshot_derive_created_and_config_events() {
        let created_at = Utc::now();
        let state = json!({
            "capabilityRequests": [
                {
                    "id": "cap-1",
                    "title": "补充 CRM 查询能力",
                    "status": "pending",
                    "detail": "需要访问外部 CRM",
                    "risk": "medium"
                }
            ]
        });
        let config = json!({
            "autoProposalTriggerCount": 3
        });

        let payloads = AgentService::governance_event_payloads_from_snapshot(
            "team-1", "portal-1", &state, &config, created_at,
        );

        assert_eq!(payloads.len(), 2);
        assert!(payloads
            .iter()
            .any(|item| item.event_type == "created" && item.entity_type == "capability"));
        assert!(payloads
            .iter()
            .any(|item| item.event_type == "config_updated" && item.entity_type == "config"));
        assert!(payloads.iter().all(|item| item.created_at == created_at));
        assert!(payloads
            .iter()
            .all(|item| item.event_id.starts_with("synthetic:")));
    }

    #[test]
    fn merge_avatar_workbench_reports_keeps_latest_and_preserves_persisted_actions() {
        let now = Utc::now();
        let derived = vec![
            AvatarWorkbenchReportItemPayload {
                id: "shared".to_string(),
                ts: now,
                kind: "governance".to_string(),
                title: "派生治理汇总".to_string(),
                summary: "derived".to_string(),
                status: "pending".to_string(),
                source: "system".to_string(),
                recommendation: None,
                action_kind: Some("open_governance".to_string()),
                action_target_id: Some("portal-1".to_string()),
                work_objects: Vec::new(),
                outputs: Vec::new(),
                needs_decision: true,
            },
            AvatarWorkbenchReportItemPayload {
                id: "derived-only".to_string(),
                ts: now - chrono::Duration::minutes(1),
                kind: "runtime".to_string(),
                title: "派生运行汇总".to_string(),
                summary: "runtime".to_string(),
                status: "logged".to_string(),
                source: "system".to_string(),
                recommendation: None,
                action_kind: Some("open_logs".to_string()),
                action_target_id: Some("portal-1".to_string()),
                work_objects: Vec::new(),
                outputs: Vec::new(),
                needs_decision: false,
            },
        ];
        let persisted = vec![
            AvatarWorkbenchReportItemPayload {
                id: "manual".to_string(),
                ts: now + chrono::Duration::minutes(2),
                kind: "note".to_string(),
                title: "人工备注".to_string(),
                summary: "manual".to_string(),
                status: "active".to_string(),
                source: "manager".to_string(),
                recommendation: Some("follow".to_string()),
                action_kind: Some("open_governance".to_string()),
                action_target_id: Some("portal-1".to_string()),
                work_objects: Vec::new(),
                outputs: Vec::new(),
                needs_decision: false,
            },
            AvatarWorkbenchReportItemPayload {
                id: "shared".to_string(),
                ts: now - chrono::Duration::minutes(5),
                kind: "note".to_string(),
                title: "旧派生副本".to_string(),
                summary: "old".to_string(),
                status: "archived".to_string(),
                source: "legacy".to_string(),
                recommendation: None,
                action_kind: None,
                action_target_id: None,
                work_objects: Vec::new(),
                outputs: Vec::new(),
                needs_decision: false,
            },
        ];

        let merged = AgentService::merge_avatar_workbench_reports(&derived, &persisted, 4);

        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].id, "manual");
        assert!(merged.iter().any(|item| item.id == "derived-only"));
        assert_eq!(
            merged
                .iter()
                .find(|item| item.id == "shared")
                .map(|item| item.summary.as_str()),
            Some("derived")
        );
    }

    #[test]
    fn governance_divergence_classifies_default_only_when_both_sources_absent() {
        let kind = AgentService::classify_governance_divergence(false, false, false, false);
        assert!(matches!(kind, AvatarGovernanceDivergenceKind::DefaultOnly));
    }

    #[test]
    fn governance_divergence_classifies_differing_when_config_mismatches() {
        let kind = AgentService::classify_governance_divergence(true, true, true, false);
        assert!(matches!(kind, AvatarGovernanceDivergenceKind::Differing));
    }

    #[test]
    fn normalize_optional_scope_id_treats_blank_as_none() {
        assert_eq!(AgentService::normalize_optional_scope_id(None), None);
        assert_eq!(AgentService::normalize_optional_scope_id(Some("   ")), None);
        assert_eq!(
            AgentService::normalize_optional_scope_id(Some("  team-1  ")),
            Some("team-1".to_string())
        );
    }

    #[test]
    fn index_governance_state_docs_reports_duplicates_and_keeps_latest() {
        let older = Utc::now();
        let newer = older + chrono::Duration::minutes(5);
        let docs = vec![
            AvatarGovernanceStateDoc {
                id: None,
                portal_id: "portal-1".to_string(),
                team_id: "team-1".to_string(),
                state: json!({ "version": 1 }),
                config: json!({ "mode": "older" }),
                updated_at: older,
            },
            AvatarGovernanceStateDoc {
                id: None,
                portal_id: "portal-1".to_string(),
                team_id: "team-1".to_string(),
                state: json!({ "version": 2 }),
                config: json!({ "mode": "newer" }),
                updated_at: newer,
            },
            AvatarGovernanceStateDoc {
                id: None,
                portal_id: "portal-2".to_string(),
                team_id: "team-1".to_string(),
                state: json!({ "version": 3 }),
                config: json!({ "mode": "single" }),
                updated_at: older,
            },
        ];

        let (indexed, counts, duplicate_rows, duplicate_state_docs) =
            AgentService::index_governance_state_docs(docs);

        assert_eq!(indexed.len(), 2);
        assert_eq!(duplicate_state_docs, 1);
        assert_eq!(
            counts.get(&("team-1".to_string(), "portal-1".to_string())),
            Some(&2)
        );
        assert_eq!(duplicate_rows.len(), 1);
        assert_eq!(duplicate_rows[0].team_id, "team-1");
        assert_eq!(duplicate_rows[0].portal_id, "portal-1");
        assert_eq!(duplicate_rows[0].state_doc_count, 2);
        assert_eq!(
            indexed
                .get(&("team-1".to_string(), "portal-1".to_string()))
                .and_then(|doc| doc.config.get("mode"))
                .and_then(serde_json::Value::as_str),
            Some("newer")
        );
    }

    #[test]
    fn binding_issue_message_describes_owner_manager_mismatch() {
        let service_agent = TeamAgentDoc {
            id: None,
            agent_id: "service-1".to_string(),
            team_id: "team-1".to_string(),
            name: "Service 1".to_string(),
            description: None,
            avatar: None,
            system_prompt: None,
            api_url: None,
            model: None,
            api_key: None,
            api_format: "openai".to_string(),
            status: "idle".to_string(),
            last_error: None,
            enabled_extensions: Vec::new(),
            custom_extensions: Vec::new(),
            agent_domain: Some("digital_avatar".to_string()),
            agent_role: Some("service".to_string()),
            owner_manager_agent_id: Some("manager-legacy".to_string()),
            template_source_agent_id: None,
            allowed_groups: Vec::new(),
            max_concurrent_tasks: 1,
            temperature: None,
            max_tokens: None,
            context_limit: None,
            thinking_enabled: true,
            assigned_skills: Vec::new(),
            auto_approve_chat: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let message = AgentService::binding_issue_message(
            &AvatarBindingIssueKind::OwnerManagerMismatch,
            Some("manager-1"),
            Some("service-1"),
            Some("manager-1"),
            Some("service-1"),
            None,
            Some(&service_agent),
        );
        assert!(message.contains("owner_manager_agent_id"));
        assert!(message.contains("manager-legacy"));
        assert!(message.contains("manager-1"));
    }

    #[test]
    fn binding_issue_messages_include_human_readable_role_mismatch() {
        let manager_agent = TeamAgentDoc {
            id: None,
            agent_id: "manager-1".to_string(),
            team_id: "team-1".to_string(),
            name: "Manager 1".to_string(),
            description: None,
            avatar: None,
            system_prompt: None,
            api_url: None,
            model: None,
            api_key: None,
            api_format: "openai".to_string(),
            status: "idle".to_string(),
            last_error: None,
            enabled_extensions: Vec::new(),
            custom_extensions: Vec::new(),
            agent_domain: Some("general".to_string()),
            agent_role: Some("default".to_string()),
            owner_manager_agent_id: None,
            template_source_agent_id: None,
            allowed_groups: Vec::new(),
            max_concurrent_tasks: 1,
            temperature: None,
            max_tokens: None,
            context_limit: None,
            thinking_enabled: true,
            assigned_skills: Vec::new(),
            auto_approve_chat: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let messages = AgentService::binding_issue_messages(
            &[AvatarBindingIssueKind::ManagerRoleMismatch],
            Some("manager-1"),
            None,
            Some("manager-1"),
            None,
            Some(&manager_agent),
            None,
        );
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("digital_avatar:manager"));
        assert!(messages[0].contains("general:default"));
    }

    #[test]
    fn orphaned_recovery_filter_excludes_current_instance_missions() {
        let filter = orphaned_mission_recovery_filter("instance-current");
        let clauses = filter
            .get_array("$or")
            .expect("recovery filter should include ownership clauses");

        assert_eq!(
            filter
                .get_document("status")
                .unwrap()
                .get_array("$in")
                .unwrap()
                .len(),
            2
        );
        assert!(
            clauses.iter().any(|item| {
                item.as_document()
                    .and_then(|doc| doc.get_document("server_instance_id").ok())
                    .and_then(|doc| doc.get_str("$ne").ok())
                    == Some("instance-current")
            }),
            "current instance exclusion must be present"
        );
    }

    #[test]
    fn orphaned_recovery_filter_keeps_legacy_missing_instance_docs_recoverable() {
        let filter = orphaned_mission_recovery_filter("instance-current");
        let clauses = filter
            .get_array("$or")
            .expect("recovery filter should include ownership clauses");

        assert!(
            clauses.iter().any(|item| {
                item.as_document()
                    .and_then(|doc| doc.get("server_instance_id"))
                    == Some(&Bson::Null)
            }),
            "null-instance legacy missions should still be recoverable"
        );
        assert!(
            clauses.iter().any(|item| {
                item.as_document()
                    .and_then(|doc| doc.get_document("server_instance_id").ok())
                    .and_then(|doc| doc.get_bool("$exists").ok())
                    == Some(false)
            }),
            "missing-instance legacy missions should still be recoverable"
        );
    }

    #[test]
    fn inactive_running_recovery_filter_relies_on_db_truth_not_manager_exclusions() {
        let filter = active_running_mission_scan_filter(&[
            "mission-active".to_string(),
            "mission-busy".to_string(),
        ]);

        assert!(
            filter.get("mission_id").is_none(),
            "recovery scan should no longer exclude missions just because MissionManager still tracks them"
        );
    }

    #[test]
    fn inactive_running_recovery_filter_targets_active_statuses_before_cutoff() {
        let filter = active_running_mission_scan_filter(&[]);

        assert_eq!(
            filter
                .get_document("status")
                .unwrap()
                .get_array("$in")
                .unwrap()
                .len(),
            2
        );
        assert!(
            filter.get("mission_id").is_none(),
            "empty active mission list should not inject a mission_id filter"
        );
    }

    #[test]
    fn explicit_goal_deliverables_prefer_user_named_files() {
        let items = collect_explicit_deliverables_from_goal_text(
            "创建 README.md、sample.csv、summarize.py 和 overview.html，直接交付可用文件。",
        );
        assert_eq!(
            items,
            vec![
                "README.md".to_string(),
                "sample.csv".to_string(),
                "summarize.py".to_string(),
                "overview.html".to_string(),
            ]
        );
    }

    #[test]
    fn explicit_goal_deliverables_ignore_framework_names_when_real_paths_exist() {
        let items = collect_explicit_deliverables_from_goal_text(
            "Research Astro, Next.js, and Nuxt. Produce research/summary.csv, research/summarize.py, research/report.md, and research/overview.html with cited comparisons.",
        );
        assert_eq!(
            items,
            vec![
                "research/summary.csv".to_string(),
                "research/summarize.py".to_string(),
                "research/report.md".to_string(),
                "research/overview.html".to_string(),
            ]
        );
    }

    #[test]
    fn step_deliverables_filter_ignores_process_only_paths() {
        let mut step = sample_mission_doc().steps[0].clone();
        step.required_artifacts = vec![
            "deliverables/README.md".to_string(),
            "evidence/run.log".to_string(),
            "reports/final/quality/check.md".to_string(),
        ];
        let items = collect_requested_deliverables_from_steps(&[step]);
        assert_eq!(items, vec!["deliverables/README.md".to_string()]);
    }

    #[test]
    fn progress_memory_excludes_done_files_from_missing() {
        let mut mission = sample_mission_doc();
        mission.delivery_manifest = Some(MissionDeliveryManifest {
            requirements: Vec::new(),
            requested_deliverables: vec![
                "comparison.csv".to_string(),
                "comparison.md".to_string(),
                "index.html".to_string(),
                "summarize.py".to_string(),
            ],
            satisfied_deliverables: vec![
                "comparison.csv".to_string(),
                "comparison.md".to_string(),
                "summarize.py".to_string(),
            ],
            missing_core_deliverables: vec![
                "comparison.csv".to_string(),
                "comparison.md".to_string(),
                "index.html".to_string(),
                "summarize.py".to_string(),
            ],
            supporting_artifacts: Vec::new(),
            delivery_state: MissionDeliveryState::Working,
            final_outcome_summary: None,
        });

        let memory = build_progress_memory(&mission).expect("progress memory");
        assert_eq!(
            memory.done,
            vec![
                "comparison.csv".to_string(),
                "comparison.md".to_string(),
                "summarize.py".to_string()
            ]
        );
        assert_eq!(memory.missing, vec!["index.html".to_string()]);
    }

    #[test]
    fn extract_task_runtime_binding_includes_task_node_id() {
        let content = serde_json::json!({
            "mission_id": "m-1",
            "run_id": "run-1",
            "session_id": "s-1",
            "task_role": "mission_worker",
            "task_node_id": "goal:g-2"
        });
        let (mission_id, run_id, session_id, task_role, task_node_id) =
            extract_task_runtime_binding(&content);
        assert_eq!(mission_id.as_deref(), Some("m-1"));
        assert_eq!(run_id.as_deref(), Some("run-1"));
        assert_eq!(session_id.as_deref(), Some("s-1"));
        assert_eq!(task_role.as_deref(), Some("mission_worker"));
        assert_eq!(task_node_id.as_deref(), Some("goal:g-2"));
    }

    #[test]
    fn research_quality_gate_blocks_placeholder_without_sources() {
        let mut mission = sample_mission_doc();
        mission.goal = "调研 Bun、Deno、Node.js 的适用性，对比并交付 report.md、summary.csv、overview.html".to_string();
        mission.delivery_manifest = Some(MissionDeliveryManifest {
            requirements: Vec::new(),
            requested_deliverables: vec![
                "report.md".to_string(),
                "summary.csv".to_string(),
                "overview.html".to_string(),
            ],
            satisfied_deliverables: vec![
                "report.md".to_string(),
                "summary.csv".to_string(),
                "overview.html".to_string(),
            ],
            missing_core_deliverables: Vec::new(),
            supporting_artifacts: Vec::new(),
            delivery_state: MissionDeliveryState::Working,
            final_outcome_summary: None,
        });
        let artifacts = vec![
            MissionArtifactDoc {
                id: None,
                artifact_id: "a1".to_string(),
                mission_id: mission.mission_id.clone(),
                step_index: 0,
                name: "report.md".to_string(),
                artifact_type: ArtifactType::Document,
                content: Some("# 报告草稿\n后续步骤会补全真实结论".to_string()),
                file_path: Some("report.md".to_string()),
                mime_type: None,
                size: 10,
                archived_document_id: None,
                archived_document_status: None,
                archived_at: None,
                created_at: bson::DateTime::now(),
            },
            MissionArtifactDoc {
                id: None,
                artifact_id: "a2".to_string(),
                mission_id: mission.mission_id.clone(),
                step_index: 0,
                name: "summary.csv".to_string(),
                artifact_type: ArtifactType::Data,
                content: Some("runtime,notes\nbun,Sample placeholder scores to validate schema".to_string()),
                file_path: Some("summary.csv".to_string()),
                mime_type: None,
                size: 10,
                archived_document_id: None,
                archived_document_status: None,
                archived_at: None,
                created_at: bson::DateTime::now(),
            },
        ];

        let blockers = research_quality_blockers(
            &mission,
            &artifacts,
            &["report.md".to_string(), "summary.csv".to_string(), "overview.html".to_string()],
        );
        assert!(blockers.iter().any(|item| item.contains("placeholder_or_draft_content:report.md")));
        assert!(blockers.iter().any(|item| item.contains("placeholder_or_draft_content:summary.csv")));
        assert!(blockers.iter().any(|item| item == "missing_source_support"));
    }

    #[test]
    fn mission_inactive_for_resume_uses_step_activity_before_mission_updated_at() {
        let cutoff = Utc::now() - chrono::Duration::minutes(10);
        let mut mission = sample_mission_doc();
        mission.status = MissionStatus::Running;
        mission.current_step = Some(0);
        mission.updated_at =
            bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(30));
        mission.steps[0].last_activity_at = Some(bson::DateTime::from_chrono(
            Utc::now() - chrono::Duration::minutes(1),
        ));

        assert!(
            !mission_inactive_for_resume(&mission, cutoff),
            "fresh step activity should keep the mission recoverable only later"
        );
    }

    #[test]
    fn mission_inactive_for_resume_claims_stale_running_step() {
        let cutoff = Utc::now() - chrono::Duration::minutes(10);
        let mut mission = sample_mission_doc();
        mission.status = MissionStatus::Running;
        mission.current_step = Some(0);
        mission.updated_at =
            bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(30));
        mission.steps[0].last_activity_at = Some(bson::DateTime::from_chrono(
            Utc::now() - chrono::Duration::minutes(20),
        ));
        mission.steps[0].last_progress_at = Some(bson::DateTime::from_chrono(
            Utc::now() - chrono::Duration::minutes(15),
        ));

        assert!(
            mission_inactive_for_resume(&mission, cutoff),
            "stale step activity/progress should make the mission claimable for auto-resume"
        );
    }

    #[test]
    fn mission_inactive_for_resume_uses_current_goal_activity_when_steps_are_empty() {
        let cutoff = Utc::now() - chrono::Duration::minutes(10);
        let mut mission = sample_mission_doc();
        mission.execution_mode = ExecutionMode::Adaptive;
        mission.status = MissionStatus::Running;
        mission.current_step = None;
        mission.steps.clear();
        mission.current_goal_id = Some("g-1".to_string());
        mission.updated_at =
            bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(30));
        mission.goal_tree = Some(vec![GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal 1".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Running,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: Vec::new(),
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(40),
            )),
            started_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(20),
            )),
            last_activity_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(1),
            )),
            last_progress_at: None,
            completed_at: None,
        }]);

        assert!(
            !mission_inactive_for_resume(&mission, cutoff),
            "fresh adaptive goal activity should keep the mission from being claimed"
        );
    }

    #[test]
    fn mission_inactive_for_resume_claims_stale_running_goal() {
        let cutoff = Utc::now() - chrono::Duration::minutes(10);
        let mut mission = sample_mission_doc();
        mission.execution_mode = ExecutionMode::Adaptive;
        mission.status = MissionStatus::Running;
        mission.current_step = None;
        mission.steps.clear();
        mission.current_goal_id = Some("g-1".to_string());
        mission.updated_at =
            bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(30));
        mission.goal_tree = Some(vec![GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal 1".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Running,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: Vec::new(),
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(40),
            )),
            started_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(20),
            )),
            last_activity_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(20),
            )),
            last_progress_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(15),
            )),
            completed_at: None,
        }]);

        assert!(
            mission_inactive_for_resume(&mission, cutoff),
            "stale adaptive goal activity should make the mission claimable for auto-resume"
        );
    }

    #[test]
    fn mission_inactive_for_resume_uses_latest_worker_state_activity() {
        let cutoff = Utc::now() - chrono::Duration::minutes(10);
        let mut mission = sample_mission_doc();
        mission.status = MissionStatus::Running;
        mission.updated_at =
            bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(30));
        mission.latest_worker_state = Some(WorkerCompactState {
            current_goal: Some("step-2".to_string()),
            recorded_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(1),
            )),
            ..Default::default()
        });

        assert!(
            !mission_inactive_for_resume(&mission, cutoff),
            "fresh worker state should keep mission active for resume checks"
        );
    }

    #[test]
    fn mission_inactive_for_resume_uses_progress_memory_activity() {
        let cutoff = Utc::now() - chrono::Duration::minutes(10);
        let mut mission = sample_mission_doc();
        mission.status = MissionStatus::Running;
        mission.updated_at =
            bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(30));
        mission.progress_memory = Some(MissionProgressMemory {
            missing: vec!["deliver/report.md".to_string()],
            next_best_action: Some("Write report.md from summary.csv".to_string()),
            updated_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(2),
            )),
            ..Default::default()
        });

        assert!(
            !mission_inactive_for_resume(&mission, cutoff),
            "fresh progress memory updates should keep mission active for resume checks"
        );
    }

    #[test]
    fn mission_fast_reclaim_after_execution_stall_when_no_tool_blocker_is_old() {
        let mut mission = sample_mission_doc();
        mission.status = MissionStatus::Running;
        mission.updated_at =
            bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(2));
        mission.progress_memory = Some(MissionProgressMemory {
            missing: vec!["smoke_test.sh".to_string()],
            blocked_by: Some(
                "Mission execution produced no tool calls after 2 retries while locked target file smoke_test.sh still required"
                    .to_string(),
            ),
            updated_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::seconds(45),
            )),
            ..Default::default()
        });

        assert!(
            mission_fast_reclaim_after_execution_stall(&mission),
            "old no-tool execution stalls should be reclaimable without waiting for the full stale window"
        );
    }

    #[test]
    fn v4_progress_memory_prefers_manifest_truth_over_stale_strategy_missing() {
        let mut mission = sample_mission_doc();
        mission.status = MissionStatus::Running;
        mission.harness_version = MissionHarnessVersion::V4;
        if let Some(manifest) = mission.delivery_manifest.as_mut() {
            manifest.requested_deliverables = vec!["report/final.md".to_string()];
            manifest.satisfied_deliverables = vec!["report/final.md".to_string()];
            manifest.missing_core_deliverables.clear();
        }

        let memory = build_progress_memory(&mission).expect("progress memory");
        assert!(
            memory.missing.is_empty(),
            "V4 progress memory should treat manifest gap as the source of truth when requested deliverables are fully satisfied"
        );
    }

    #[test]
    fn current_node_preferred_target_uses_goal_contract_or_goal_text_not_goal_order() {
        let mut mission = sample_mission_doc();
        mission.status = MissionStatus::Running;
        mission.execution_mode = ExecutionMode::Adaptive;
        mission.current_step = None;
        mission.current_goal_id = Some("g-2".to_string());
        if let Some(manifest) = mission.delivery_manifest.as_mut() {
            manifest.requested_deliverables = vec![
                "pkg/summary.csv".to_string(),
                "pkg/report.md".to_string(),
                "pkg/index.html".to_string(),
            ];
        }
        mission.goal_tree = Some(vec![
            GoalNode {
                goal_id: "g-1".to_string(),
                parent_id: None,
                title: "Create summary".to_string(),
                description: "Build pkg/summary.csv".to_string(),
                success_criteria: "pkg/summary.csv exists".to_string(),
                status: GoalStatus::Completed,
                depth: 0,
                order: 0,
                exploration_budget: 1,
                attempts: Vec::new(),
                output_summary: None,
                runtime_contract: Some(RuntimeContract {
                    required_artifacts: vec!["pkg/summary.csv".to_string()],
                    completion_checks: Vec::new(),
                    no_artifact_reason: None,
                    source: None,
                    captured_at: None,
                }),
                contract_verification: None,
                pivot_reason: None,
                is_checkpoint: false,
                created_at: None,
                started_at: None,
                last_activity_at: None,
                last_progress_at: None,
                completed_at: None,
            },
            GoalNode {
                goal_id: "g-2".to_string(),
                parent_id: None,
                title: "Write the report".to_string(),
                description: "Turn the results into pkg/report.md".to_string(),
                success_criteria: "pkg/report.md exists".to_string(),
                status: GoalStatus::Running,
                depth: 0,
                order: 0,
                exploration_budget: 1,
                attempts: Vec::new(),
                output_summary: None,
                runtime_contract: None,
                contract_verification: None,
                pivot_reason: None,
                is_checkpoint: false,
                created_at: None,
                started_at: None,
                last_activity_at: None,
                last_progress_at: None,
                completed_at: None,
            },
        ]);

        let target = current_node_preferred_target(&mission);
        assert_eq!(
            target.as_deref(),
            Some("pkg/report.md"),
            "current goal targeting should derive from the goal contract/text rather than relying on goal.order indexing into requested deliverables"
        );
    }

    #[test]
    fn mission_inactive_for_resume_respects_waiting_external_cooldown() {
        let cutoff = Utc::now() - chrono::Duration::minutes(10);
        let mut mission = sample_mission_doc();
        mission.status = MissionStatus::Running;
        mission.updated_at =
            bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(30));
        mission.waiting_external_until = Some(bson::DateTime::from_chrono(
            Utc::now() + chrono::Duration::minutes(5),
        ));

        assert!(
            !mission_inactive_for_resume(&mission, cutoff),
            "missions in waiting_external cooldown should not be auto-claimed for resume"
        );
    }

    #[test]
    fn mission_inactive_for_resume_claims_retryable_waiting_external_after_cooldown() {
        let cutoff = Utc::now() - chrono::Duration::minutes(10);
        let mut mission = sample_mission_doc();
        mission.status = MissionStatus::Running;
        mission.updated_at =
            bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(20));
        mission.waiting_external_until = Some(bson::DateTime::from_chrono(
            Utc::now() - chrono::Duration::seconds(5),
        ));

        assert!(
            mission_inactive_for_resume(&mission, cutoff),
            "retryable waiting_external missions should become claimable once the cooldown expires and the mission remains stale"
        );
    }

    #[test]
    fn mission_inactive_for_resume_does_not_claim_non_retryable_waiting_external() {
        assert!(runtime::is_non_retryable_external_provider_message(
            "Request failed: Bad request (400): Your account does not have a valid coding plan subscription, or your subscription has expired"
        ));
    }
}

// MongoDB document types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamAgentDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub agent_id: String,
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub avatar: Option<String>,
    pub system_prompt: Option<String>,
    pub api_url: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub api_format: String,
    pub status: String,
    pub last_error: Option<String>,
    pub enabled_extensions: Vec<AgentExtensionConfig>,
    pub custom_extensions: Vec<CustomExtensionConfig>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub owner_manager_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub template_source_agent_id: Option<String>,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_tasks: u32,
    /// LLM temperature (0.0 - 1.0)
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub temperature: Option<f32>,
    /// Maximum output tokens per LLM call
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_tokens: Option<i32>,
    /// Context window limit override
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_limit: Option<usize>,
    /// Whether think/reasoning mode should be enabled for this agent.
    #[serde(default = "default_thinking_enabled")]
    pub thinking_enabled: bool,
    /// Skills assigned from team shared skills
    #[serde(default)]
    pub assigned_skills: Vec<AgentSkillConfig>,
    /// Auto-approve chat tasks (skip manual approval for chat messages)
    #[serde(default = "default_auto_approve_chat")]
    pub auto_approve_chat: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvatarGovernanceCounts {
    #[serde(default)]
    pub pending_capability_requests: u32,
    #[serde(default)]
    pub pending_gap_proposals: u32,
    #[serde(default)]
    pub pending_optimization_tickets: u32,
    #[serde(default)]
    pub pending_runtime_logs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarInstanceDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub portal_id: String,
    pub team_id: String,
    pub slug: String,
    pub name: String,
    pub status: String,
    pub avatar_type: String,
    pub manager_agent_id: Option<String>,
    pub service_agent_id: Option<String>,
    pub document_access_mode: String,
    pub governance_counts: AvatarGovernanceCounts,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub portal_updated_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub projected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarInstanceSummary {
    pub portal_id: String,
    pub team_id: String,
    pub slug: String,
    pub name: String,
    pub status: String,
    pub avatar_type: String,
    pub manager_agent_id: Option<String>,
    pub service_agent_id: Option<String>,
    pub document_access_mode: String,
    pub governance_counts: AvatarGovernanceCounts,
    pub portal_updated_at: DateTime<Utc>,
    pub projected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceStateDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub portal_id: String,
    pub team_id: String,
    pub state: serde_json::Value,
    pub config: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceStatePayload {
    pub portal_id: String,
    pub team_id: String,
    pub state: serde_json::Value,
    pub config: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceEventDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub portal_id: String,
    pub team_id: String,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: Option<String>,
    pub title: String,
    pub status: Option<String>,
    pub detail: Option<String>,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub meta: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceEventPayload {
    pub event_id: String,
    pub portal_id: String,
    pub team_id: String,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: Option<String>,
    pub title: String,
    pub status: Option<String>,
    pub detail: Option<String>,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub meta: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceQueueItemPayload {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub status: String,
    pub ts: String,
    pub meta: Vec<String>,
    pub source_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarWorkbenchSummaryPayload {
    pub portal_id: String,
    pub team_id: String,
    pub avatar_name: String,
    pub avatar_type: String,
    pub avatar_status: String,
    pub manager_agent_id: Option<String>,
    pub service_agent_id: Option<String>,
    pub document_access_mode: String,
    pub work_object_count: u32,
    pub pending_decision_count: u32,
    pub running_mission_count: u32,
    pub needs_attention_count: u32,
    pub last_activity_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarWorkbenchReportItemPayload {
    pub id: String,
    pub ts: DateTime<Utc>,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub source: String,
    pub recommendation: Option<String>,
    pub action_kind: Option<String>,
    pub action_target_id: Option<String>,
    #[serde(default)]
    pub work_objects: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub needs_decision: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarWorkbenchDecisionItemPayload {
    pub id: String,
    pub ts: DateTime<Utc>,
    #[serde(default)]
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub status: String,
    pub risk: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub recommendation: Option<String>,
    #[serde(default)]
    pub work_objects: Vec<String>,
    pub action_kind: Option<String>,
    pub action_target_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarWorkbenchSnapshotPayload {
    pub portal_id: String,
    pub team_id: String,
    pub summary: AvatarWorkbenchSummaryPayload,
    pub reports: Vec<AvatarWorkbenchReportItemPayload>,
    pub decisions: Vec<AvatarWorkbenchDecisionItemPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarManagerReportDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub report_id: String,
    pub portal_id: String,
    pub team_id: String,
    #[serde(default = "default_avatar_manager_report_source")]
    pub report_source: String,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub recommendation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub action_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub action_target_id: Option<String>,
    #[serde(default)]
    pub work_objects: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub needs_decision: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub synced_at: DateTime<Utc>,
}

fn default_avatar_manager_report_source() -> String {
    "derived".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvatarGovernanceBackfillReport {
    pub portals_scanned: u64,
    pub states_created: u64,
    pub events_seeded: u64,
    pub projections_synced_teams: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvatarGovernanceDivergenceKind {
    DefaultOnly,
    InSync,
    StateOnly,
    SettingsOnly,
    Differing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceDivergenceRow {
    pub team_id: String,
    pub portal_id: String,
    pub portal_name: String,
    pub slug: String,
    pub classification: AvatarGovernanceDivergenceKind,
    pub state_doc_exists: bool,
    pub portal_state_exists: bool,
    pub portal_config_exists: bool,
    pub state_matches_portal_state: bool,
    pub config_matches_portal_config: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceOrphanStateRow {
    pub team_id: String,
    pub portal_id: String,
    pub state_doc_count: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceDuplicateStateRow {
    pub team_id: String,
    pub portal_id: String,
    pub state_doc_count: u64,
    pub retained_updated_at: String,
    pub duplicate_updated_at: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvatarGovernanceDivergenceAuditReport {
    pub total_avatar_portals: u64,
    pub total_state_docs: u64,
    pub default_only: u64,
    pub in_sync: u64,
    pub state_only: u64,
    pub settings_only: u64,
    pub differing: u64,
    pub duplicate_state_docs: u64,
    pub orphan_state_docs: u64,
    pub rows: Vec<AvatarGovernanceDivergenceRow>,
    pub duplicate_rows: Vec<AvatarGovernanceDuplicateStateRow>,
    pub orphan_rows: Vec<AvatarGovernanceOrphanStateRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvatarBindingIssueKind {
    MissingExplicitManagerBinding,
    MissingExplicitServiceBinding,
    MissingManagerBinding,
    MissingServiceBinding,
    ManagerAgentNotFound,
    ServiceAgentNotFound,
    ManagerRoleMismatch,
    ServiceRoleMismatch,
    OwnerManagerMismatch,
    SameAgentReused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarBindingAuditRow {
    pub team_id: String,
    pub portal_id: String,
    pub portal_name: String,
    pub slug: String,
    pub explicit_coding_agent_id: Option<String>,
    pub explicit_service_agent_id: Option<String>,
    pub effective_manager_agent_id: Option<String>,
    pub effective_service_agent_id: Option<String>,
    pub manager_agent_domain: Option<String>,
    pub manager_agent_role: Option<String>,
    pub service_agent_domain: Option<String>,
    pub service_agent_role: Option<String>,
    pub service_owner_manager_agent_id: Option<String>,
    pub issues: Vec<AvatarBindingIssueKind>,
    pub issue_messages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvatarBindingAuditReport {
    pub total_avatar_portals: u64,
    pub valid: u64,
    pub missing_explicit_manager_binding: u64,
    pub missing_explicit_service_binding: u64,
    pub missing_manager_binding: u64,
    pub missing_service_binding: u64,
    pub manager_agent_not_found: u64,
    pub service_agent_not_found: u64,
    pub manager_role_mismatch: u64,
    pub service_role_mismatch: u64,
    pub owner_manager_mismatch: u64,
    pub same_agent_reused: u64,
    pub shadow_invariant_rows: u64,
    pub rows: Vec<AvatarBindingAuditRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarReadSideEffectAuditItem {
    pub operation: String,
    pub file: String,
    pub side_effects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarDeepWaterAuditReport {
    pub generated_at: String,
    pub requested_team_id: Option<String>,
    pub governance: AvatarGovernanceDivergenceAuditReport,
    pub bindings: AvatarBindingAuditReport,
    pub read_side_effects: Vec<AvatarReadSideEffectAuditItem>,
}

#[derive(Debug, Clone)]
struct GovernanceEntitySnapshot {
    entity_type: &'static str,
    id: String,
    title: String,
    status: String,
    detail: String,
    meta: serde_json::Value,
}

fn mission_status_label(status: &MissionStatus) -> &'static str {
    match status {
        MissionStatus::Draft => "draft",
        MissionStatus::Planning => "planning",
        MissionStatus::Planned => "planned",
        MissionStatus::Running => "running",
        MissionStatus::Paused => "paused",
        MissionStatus::Completed => "completed",
        MissionStatus::Failed => "failed",
        MissionStatus::Cancelled => "cancelled",
    }
}

fn mission_needs_attention(status: &MissionStatus) -> bool {
    matches!(status, MissionStatus::Failed | MissionStatus::Paused)
}

fn queue_item_risk(meta: &[String]) -> &'static str {
    if meta.iter().any(|item| item.contains("高风险")) {
        "high"
    } else if meta.iter().any(|item| item.contains("中风险")) {
        "medium"
    } else {
        "low"
    }
}

fn queue_item_source(kind: &str, meta: &[String]) -> String {
    meta.iter()
        .find(|item| {
            !item.contains("风险")
                && !item.contains("high")
                && !item.contains("medium")
                && !item.contains("low")
        })
        .cloned()
        .unwrap_or_else(|| match kind {
            "capability" => "能力边界".to_string(),
            "proposal" => "岗位提案".to_string(),
            "ticket" => "运行优化".to_string(),
            _ => "管理台".to_string(),
        })
}

fn queue_item_recommendation(kind: &str, risk: &str) -> String {
    match kind {
        "capability" => {
            if risk == "high" {
                "先确认这次能力放开是否触及高风险边界，再决定是交人工审批还是保持收敛。".to_string()
            } else {
                "先判断是否需要放开当前能力边界，再决定批准、试运行或继续收敛。".to_string()
            }
        }
        "proposal" => "先判断这项岗位提案是否值得试运行，再决定批准、试点或拒绝。".to_string(),
        "ticket" => {
            if risk == "high" {
                "先查看这次运行问题与风险证据，再决定回滚、人工确认或进入试运行。".to_string()
            } else {
                "先确认优化证据和预期收益，再决定试运行、批准或继续观察。".to_string()
            }
        }
        _ => "先阅读当前决策背景，再决定继续执行、调整方案或转人工确认。".to_string(),
    }
}

fn parse_rfc3339_utc(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn default_max_concurrent() -> u32 {
    1
}

fn default_auto_approve_chat() -> bool {
    true
}

fn default_thinking_enabled() -> bool {
    true
}

fn is_runtime_legacy_builtin_extension(extension: BuiltinExtension) -> bool {
    matches!(extension, BuiltinExtension::Team)
}

fn sanitize_enabled_extensions(configs: Vec<AgentExtensionConfig>) -> Vec<AgentExtensionConfig> {
    configs
        .into_iter()
        .filter(|config| !is_runtime_legacy_builtin_extension(config.extension))
        .collect()
}

impl From<TeamAgentDoc> for TeamAgent {
    fn from(doc: TeamAgentDoc) -> Self {
        Self {
            id: doc.agent_id,
            team_id: doc.team_id,
            name: doc.name,
            description: doc.description,
            avatar: doc.avatar,
            system_prompt: doc.system_prompt,
            api_url: doc.api_url,
            model: doc.model,
            api_key: None, // Don't expose API key
            api_format: doc.api_format.parse().unwrap_or(ApiFormat::OpenAI),
            enabled_extensions: sanitize_enabled_extensions(doc.enabled_extensions),
            custom_extensions: doc.custom_extensions,
            agent_domain: doc.agent_domain,
            agent_role: doc.agent_role,
            owner_manager_agent_id: doc.owner_manager_agent_id,
            template_source_agent_id: doc.template_source_agent_id,
            status: doc.status.parse().unwrap_or(AgentStatus::Idle),
            last_error: doc.last_error,
            allowed_groups: doc.allowed_groups,
            max_concurrent_tasks: doc.max_concurrent_tasks,
            temperature: doc.temperature,
            max_tokens: doc.max_tokens,
            context_limit: doc.context_limit,
            thinking_enabled: doc.thinking_enabled,
            assigned_skills: doc.assigned_skills,
            auto_approve_chat: doc.auto_approve_chat,
            created_at: doc.created_at,
            updated_at: doc.updated_at,
        }
    }
}

impl From<AvatarManagerReportDoc> for AvatarWorkbenchReportItemPayload {
    fn from(doc: AvatarManagerReportDoc) -> Self {
        Self {
            id: doc.report_id,
            ts: doc.created_at,
            kind: doc.kind,
            title: doc.title,
            summary: doc.summary,
            status: doc.status,
            source: doc.source,
            recommendation: doc.recommendation,
            action_kind: doc.action_kind,
            action_target_id: doc.action_target_id,
            work_objects: doc.work_objects,
            outputs: doc.outputs,
            needs_decision: doc.needs_decision,
        }
    }
}

impl From<AvatarInstanceDoc> for AvatarInstanceSummary {
    fn from(doc: AvatarInstanceDoc) -> Self {
        Self {
            portal_id: doc.portal_id,
            team_id: doc.team_id,
            slug: doc.slug,
            name: doc.name,
            status: doc.status,
            avatar_type: doc.avatar_type,
            manager_agent_id: doc.manager_agent_id,
            service_agent_id: doc.service_agent_id,
            document_access_mode: doc.document_access_mode,
            governance_counts: doc.governance_counts,
            portal_updated_at: doc.portal_updated_at,
            projected_at: doc.projected_at,
        }
    }
}

impl From<AvatarGovernanceStateDoc> for AvatarGovernanceStatePayload {
    fn from(doc: AvatarGovernanceStateDoc) -> Self {
        Self {
            portal_id: doc.portal_id,
            team_id: doc.team_id,
            state: doc.state,
            config: doc.config,
            updated_at: doc.updated_at,
        }
    }
}

impl From<AvatarGovernanceEventDoc> for AvatarGovernanceEventPayload {
    fn from(doc: AvatarGovernanceEventDoc) -> Self {
        let event_id = doc.id.map(|value| value.to_hex()).unwrap_or_else(|| {
            format!(
                "synthetic:{}:{}:{}:{}:{}:{}",
                doc.team_id,
                doc.portal_id,
                doc.event_type,
                doc.entity_type,
                doc.entity_id.as_deref().unwrap_or("none"),
                doc.created_at.timestamp_millis()
            )
        });
        Self {
            event_id,
            portal_id: doc.portal_id,
            team_id: doc.team_id,
            event_type: doc.event_type,
            entity_type: doc.entity_type,
            entity_id: doc.entity_id,
            title: doc.title,
            status: doc.status,
            detail: doc.detail,
            actor_id: doc.actor_id,
            actor_name: doc.actor_name,
            meta: doc.meta,
            created_at: doc.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTaskDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_node_id: Option<String>,
    pub team_id: String,
    pub agent_id: String,
    pub submitter_id: String,
    pub approver_id: Option<String>,
    pub task_type: String,
    pub content: serde_json::Value,
    pub status: String,
    pub priority: i32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub submitted_at: DateTime<Utc>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub approved_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
}

impl From<AgentTaskDoc> for AgentTask {
    fn from(doc: AgentTaskDoc) -> Self {
        Self {
            id: doc.task_id,
            team_id: doc.team_id,
            agent_id: doc.agent_id,
            submitter_id: doc.submitter_id,
            approver_id: doc.approver_id,
            task_type: doc.task_type.parse().unwrap_or(TaskType::Chat),
            content: doc.content,
            status: doc.status.parse().unwrap_or(TaskStatus::Pending),
            priority: doc.priority,
            submitted_at: doc.submitted_at,
            approved_at: doc.approved_at,
            started_at: doc.started_at,
            completed_at: doc.completed_at,
            error_message: doc.error_message,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResultDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub result_id: String,
    pub task_id: String,
    pub result_type: String,
    pub content: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl From<TaskResultDoc> for TaskResult {
    fn from(doc: TaskResultDoc) -> Self {
        let result_type = match doc.result_type.as_str() {
            "message" => TaskResultType::Message,
            "tool_call" => TaskResultType::ToolCall,
            _ => TaskResultType::Error,
        };
        Self {
            id: doc.result_id,
            task_id: doc.task_id,
            result_type,
            content: doc.content,
            created_at: doc.created_at,
        }
    }
}

/// Convert a TeamAgentDoc to TeamAgent while preserving the api_key field
/// (which is normally stripped by the From impl for API safety).
fn agent_doc_with_key(doc: TeamAgentDoc) -> TeamAgent {
    let api_key = doc.api_key.clone();
    let mut agent: TeamAgent = doc.into();
    agent.api_key = api_key;
    agent
}

/// Agent service for managing team agents and tasks (MongoDB)
pub struct AgentService {
    db: Arc<MongoDb>,
}

impl AgentService {
    pub fn new(db: Arc<MongoDb>) -> Self {
        Self { db }
    }

    fn normalize_session_source(raw: Option<String>, portal_restricted: bool) -> String {
        let v = raw
            .unwrap_or_else(|| {
                if portal_restricted {
                    "portal".to_string()
                } else {
                    "chat".to_string()
                }
            })
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_");
        match v.as_str() {
            "mission" | "portal" | "portal_coding" | "portal_manager" | "system" | "chat" => v,
            _ => {
                if portal_restricted {
                    "portal".to_string()
                } else {
                    "chat".to_string()
                }
            }
        }
    }

    fn normalize_skill_id_list(items: Vec<String>) -> Vec<String> {
        let mut seen = HashSet::new();
        items
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .filter(|item| seen.insert(item.clone()))
            .collect()
    }

    async fn resolve_session_allowed_skill_ids(
        &self,
        agent_id: &str,
        requested_allowed_skill_ids: Option<Vec<String>>,
    ) -> Result<Option<Vec<String>>, mongodb::error::Error> {
        let requested_allowed_skill_ids =
            requested_allowed_skill_ids.map(Self::normalize_skill_id_list);

        let Some(agent) = self.get_agent(agent_id).await? else {
            return Ok(requested_allowed_skill_ids);
        };

        let assigned_skill_ids = Self::normalize_skill_id_list(
            agent
                .assigned_skills
                .iter()
                .filter(|skill| skill.enabled)
                .map(|skill| skill.skill_id.clone())
                .collect(),
        );
        let assigned_lookup = assigned_skill_ids.iter().cloned().collect::<HashSet<_>>();

        let effective = match requested_allowed_skill_ids {
            Some(requested) => requested
                .into_iter()
                .filter(|skill_id| assigned_lookup.contains(skill_id))
                .collect::<Vec<_>>(),
            None => assigned_skill_ids,
        };

        Ok(Some(effective))
    }

    /// M12: Ensure MongoDB indexes for agent_sessions collection (chat track)
    pub async fn ensure_chat_indexes(&self) {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        let session_indexes = vec![
            // User session list query (sorted by last message time)
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "user_id": 1, "status": 1, "last_message_at": -1 })
                .build(),
            // User session list query with visibility filter
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "user_id": 1, "status": 1, "hidden_from_chat_list": 1, "last_message_at": -1 })
                .build(),
            // Filter by agent
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "agent_id": 1, "status": 1 })
                .build(),
            // Pinned + time sort
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "user_id": 1, "pinned": -1, "last_message_at": -1 })
                .build(),
            // Session lookup by session_id (unique)
            IndexModel::builder()
                .keys(doc! { "session_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        ];

        if let Err(e) = self.sessions().create_indexes(session_indexes, None).await {
            tracing::warn!("Failed to create chat session indexes: {}", e);
        } else {
            tracing::info!("Chat session indexes ensured");
        }

        let event_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "event_id": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "run_id": 1, "event_id": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "run_id": { "$exists": true } })
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "created_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "run_id": 1, "created_at": 1 })
                .build(),
        ];

        if let Err(e) = self.chat_events().create_indexes(event_indexes, None).await {
            tracing::warn!("Failed to create chat event indexes: {}", e);
        } else {
            tracing::info!("Chat event indexes ensured");
        }
    }

    fn agents(&self) -> mongodb::Collection<TeamAgentDoc> {
        self.db.collection("team_agents")
    }

    fn tasks(&self) -> mongodb::Collection<AgentTaskDoc> {
        self.db.collection("agent_tasks")
    }

    fn run_states(&self) -> mongodb::Collection<RunState> {
        self.db.collection("agent_run_states")
    }

    fn run_journal(&self) -> mongodb::Collection<RunJournal> {
        self.db.collection("agent_run_journal")
    }

    fn run_checkpoints(&self) -> mongodb::Collection<RunCheckpoint> {
        self.db.collection("agent_run_checkpoints")
    }

    fn subagent_runs(&self) -> mongodb::Collection<SubagentRun> {
        self.db.collection("agent_subagent_runs")
    }

    fn task_graphs(&self) -> mongodb::Collection<TaskGraph> {
        self.db.collection("agent_task_graphs")
    }

    pub async fn upsert_run_state(
        &self,
        state: &RunState,
    ) -> Result<(), mongodb::error::Error> {
        let mut state = state.clone();
        let now = bson::DateTime::now();
        if state.created_at.is_none() {
            state.created_at = Some(now);
        }
        state.updated_at = Some(now);
        self.run_states()
            .replace_one(
                doc! { "run_id": &state.run_id },
                state,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(())
    }

    pub async fn patch_run_state_after_turn(
        &self,
        run_id: &str,
        current_node_id: &str,
        status: RunStatus,
        memory: Option<&RunMemory>,
        outcome: &TurnOutcome,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        let mut set_doc = doc! {
            "current_node_id": current_node_id,
            "status": bson::to_bson(&status)
                .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
            "last_turn_outcome": bson::to_bson(outcome)
                .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
            "updated_at": now,
        };
        if let Some(memory) = memory {
            set_doc.insert(
                "memory",
                bson::to_bson(memory)
                    .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
            );
        }
        self.run_states()
            .update_one(doc! { "run_id": run_id }, doc! { "$set": set_doc }, None)
            .await?;
        Ok(())
    }

    pub async fn append_run_journal(
        &self,
        entries: &[RunJournal],
    ) -> Result<(), mongodb::error::Error> {
        if entries.is_empty() {
            return Ok(());
        }
        let now = bson::DateTime::now();
        let docs = entries
            .iter()
            .cloned()
            .map(|mut entry| {
                if entry.created_at.is_none() {
                    entry.created_at = Some(now);
                }
                entry
            })
            .collect::<Vec<_>>();
        self.run_journal().insert_many(docs, None).await?;
        Ok(())
    }

    pub async fn save_run_checkpoint(
        &self,
        checkpoint: &RunCheckpoint,
    ) -> Result<(), mongodb::error::Error> {
        let mut checkpoint = checkpoint.clone();
        if checkpoint.created_at.is_none() {
            checkpoint.created_at = Some(bson::DateTime::now());
        }
        self.run_checkpoints().insert_one(checkpoint, None).await?;
        Ok(())
    }

    pub async fn upsert_task_graph(
        &self,
        graph: &TaskGraph,
    ) -> Result<(), mongodb::error::Error> {
        let mut graph = graph.clone();
        let now = bson::DateTime::now();
        if graph.created_at.is_none() {
            graph.created_at = Some(now);
        }
        graph.updated_at = Some(now);
        self.task_graphs()
            .replace_one(
                doc! { "task_graph_id": &graph.task_graph_id },
                graph,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(())
    }

    pub async fn get_run_state(
        &self,
        run_id: &str,
    ) -> Result<Option<RunState>, mongodb::error::Error> {
        self.run_states()
            .find_one(doc! { "run_id": run_id }, None)
            .await
    }

    pub async fn project_v4_mission_overlay(
        &self,
        mission: &MissionDoc,
    ) -> Result<MissionDoc, mongodb::error::Error> {
        if mission.harness_version != MissionHarnessVersion::V4 {
            return Ok(mission.clone());
        }
        let Some(run_id) = mission.current_run_id.as_deref() else {
            return Ok(mission.clone());
        };
        let mut projected = mission.clone();
        let graph_id = format!("mission:{}:{}", mission.mission_id, run_id);
        let mut run_state = self.get_run_state(run_id).await?;
        let mut task_graph = self.get_task_graph(&graph_id).await?;
        let checkpoint_count = self
            .run_checkpoints()
            .count_documents(doc! { "run_id": run_id }, None)
            .await?;
        if run_state.is_none() || task_graph.is_none() || checkpoint_count == 0 {
            let _ = sync_v4_runtime_projection_from_mission(self, mission, None).await;
            run_state = self.get_run_state(run_id).await?;
            task_graph = self.get_task_graph(&graph_id).await?;
        }
        if let Some(run_state) = run_state {
            if let Some(memory) = run_state.memory.as_ref() {
                let done_paths = normalize_runtime_projection_paths(&memory.done);
                let missing_paths = normalize_runtime_projection_paths(&memory.missing);
                projected.progress_memory = Some(v4_progress_memory_from_run_memory(
                    memory,
                    &done_paths,
                    &missing_paths,
                ));
                let delivery_manifest = projected.delivery_manifest.get_or_insert_with(Default::default);
                delivery_manifest.satisfied_deliverables = done_paths;
                delivery_manifest.missing_core_deliverables = missing_paths;
            }
            if let Some(node_id) = run_state.current_node_id.as_deref() {
                if let Some(goal_id) = node_id.strip_prefix("goal:") {
                    projected.current_goal_id = Some(goal_id.to_string());
                    projected.current_step = None;
                } else if let Some(step_idx) = node_id.strip_prefix("step:") {
                    if let Ok(step) = step_idx.parse::<u32>() {
                        projected.current_step = Some(step);
                        projected.current_goal_id = None;
                    }
                }
            }
        }
        if let Some(task_graph) = task_graph {
            if let Some(node_id) = task_graph.current_node_id.as_deref() {
                if let Some(goal_id) = node_id.strip_prefix("goal:") {
                    projected.current_goal_id = Some(goal_id.to_string());
                    projected.current_step = None;
                } else if let Some(step_idx) = node_id.strip_prefix("step:") {
                    if let Ok(step) = step_idx.parse::<u32>() {
                        projected.current_step = Some(step);
                        projected.current_goal_id = None;
                    }
                }
            }
        }
        if matches!(
            projected.status,
            MissionStatus::Completed | MissionStatus::Failed | MissionStatus::Cancelled
        ) {
            projected.execution_lease = None;
            projected.current_goal_id = None;
            projected.current_step = None;
        }
        Ok(projected)
    }

    pub async fn ensure_run_checkpoint_exists(
        &self,
        run_id: &str,
        mission_id: Option<&str>,
        task_graph_id: Option<&str>,
        current_node_id: Option<&str>,
        status: RunStatus,
        lease: Option<&RunLease>,
        memory: Option<&RunMemory>,
    ) -> Result<bool, mongodb::error::Error> {
        let existing = self
            .run_checkpoints()
            .count_documents(doc! { "run_id": run_id }, None)
            .await?;
        if existing > 0 {
            return Ok(false);
        }
        self.save_run_checkpoint(&RunCheckpoint {
            id: None,
            run_id: run_id.to_string(),
            mission_id: mission_id.map(str::to_string),
            task_graph_id: task_graph_id.map(str::to_string),
            current_node_id: current_node_id.map(str::to_string),
            checkpoint_kind: RunCheckpointKind::NodeStart,
            status,
            lease: lease.cloned(),
            memory: memory.cloned(),
            last_turn_outcome: None,
            created_at: Some(bson::DateTime::now()),
        })
        .await?;
        Ok(true)
    }

    pub async fn get_task_graph(
        &self,
        task_graph_id: &str,
    ) -> Result<Option<TaskGraph>, mongodb::error::Error> {
        self.task_graphs()
            .find_one(doc! { "task_graph_id": task_graph_id }, None)
            .await
    }

    pub async fn upsert_subagent_run(
        &self,
        run: &SubagentRun,
    ) -> Result<(), mongodb::error::Error> {
        let mut run = run.clone();
        let now = bson::DateTime::now();
        if run.created_at.is_none() {
            run.created_at = Some(now);
        }
        run.updated_at = Some(now);
        self.subagent_runs()
            .replace_one(
                doc! { "subagent_run_id": &run.subagent_run_id },
                run,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(())
    }

    pub async fn mark_run_subagent_started(
        &self,
        parent_run_id: &str,
        subagent_run_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.run_states()
            .update_one(
                doc! { "run_id": parent_run_id },
                doc! {
                    "$addToSet": { "active_subagents": subagent_run_id },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn mark_run_subagent_finished(
        &self,
        parent_run_id: &str,
        subagent_run_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.run_states()
            .update_one(
                doc! { "run_id": parent_run_id },
                doc! {
                    "$pull": { "active_subagents": subagent_run_id },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    fn results(&self) -> mongodb::Collection<TaskResultDoc> {
        self.db.collection("agent_task_results")
    }

    fn sessions(&self) -> mongodb::Collection<AgentSessionDoc> {
        self.db.collection("agent_sessions")
    }

    fn chat_events(&self) -> mongodb::Collection<ChatEventDoc> {
        self.db.collection("agent_chat_events")
    }

    fn teams(&self) -> mongodb::Collection<Document> {
        self.db.collection("teams")
    }

    fn portals(&self) -> mongodb::Collection<Document> {
        self.db.collection(agime_team::db::collections::PORTALS)
    }

    fn documents_store(&self) -> mongodb::Collection<TeamDocument> {
        self.db.collection(agime_team::db::collections::DOCUMENTS)
    }

    fn avatar_instances(&self) -> mongodb::Collection<AvatarInstanceDoc> {
        self.db
            .collection(agime_team::db::collections::AVATAR_INSTANCES)
    }

    fn avatar_governance_states(&self) -> mongodb::Collection<AvatarGovernanceStateDoc> {
        self.db
            .collection(agime_team::db::collections::AVATAR_GOVERNANCE_STATES)
    }

    fn avatar_governance_events(&self) -> mongodb::Collection<AvatarGovernanceEventDoc> {
        self.db
            .collection(agime_team::db::collections::AVATAR_GOVERNANCE_EVENTS)
    }

    fn avatar_manager_reports(&self) -> mongodb::Collection<AvatarManagerReportDoc> {
        self.db
            .collection(agime_team::db::collections::AVATAR_MANAGER_REPORTS)
    }

    fn is_dedicated_avatar_marker(description: Option<&str>) -> bool {
        let desc = description.unwrap_or("").to_ascii_lowercase();
        desc.contains("[digital-avatar-manager]") || desc.contains("[digital-avatar-service]")
    }

    fn explicit_avatar_marker_role(description: Option<&str>) -> Option<&'static str> {
        let desc = description.unwrap_or("").to_ascii_lowercase();
        if desc.contains("[digital-avatar-manager]") {
            return Some("manager");
        }
        if desc.contains("[digital-avatar-service]") {
            return Some("service");
        }
        None
    }

    fn normalize_compact_text(value: Option<&str>) -> String {
        value
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect()
    }

    fn is_legacy_avatar_manager_candidate(agent: &TeamAgent) -> bool {
        if agent.agent_domain.as_deref() == Some("digital_avatar")
            && agent.agent_role.as_deref() == Some("manager")
        {
            return true;
        }
        if Self::is_dedicated_avatar_marker(agent.description.as_deref()) {
            return true;
        }
        let compact_name = Self::normalize_compact_text(Some(agent.name.as_str()));
        compact_name.contains("管理agent") || compact_name.contains("manageragent")
    }

    fn is_legacy_avatar_service_candidate(agent: &TeamAgent) -> bool {
        if agent.agent_domain.as_deref() == Some("digital_avatar")
            && agent.agent_role.as_deref() == Some("service")
        {
            return true;
        }
        if Self::is_dedicated_avatar_marker(agent.description.as_deref()) {
            return true;
        }
        let compact_name = Self::normalize_compact_text(Some(agent.name.as_str()));
        compact_name.ends_with("分身agent") || compact_name.contains("avataragent")
    }

    fn doc_string(doc: &Document, key: &str) -> Option<String> {
        doc.get(key).and_then(Bson::as_str).map(str::to_string)
    }

    fn nested_doc_string(doc: &Document, path: &[&str]) -> Option<String> {
        let mut current = doc;
        for key in &path[..path.len().saturating_sub(1)] {
            current = current.get_document(key).ok()?;
        }
        let leaf = *path.last()?;
        current.get(leaf).and_then(Bson::as_str).map(str::to_string)
    }

    fn doc_has_string(doc: &Document, key: &str, expected: &str) -> bool {
        Self::doc_string(doc, key)
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| value == expected)
    }

    fn nested_doc_has_string(doc: &Document, path: &[&str], expected: &str) -> bool {
        Self::nested_doc_string(doc, path)
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| value == expected)
    }

    fn doc_has_manager_tag(doc: &Document, expected: &str) -> bool {
        doc.get_array("tags")
            .ok()
            .map(|tags| {
                tags.iter()
                    .filter_map(Bson::as_str)
                    .map(str::trim)
                    .any(|tag| tag == format!("manager:{expected}"))
            })
            .unwrap_or(false)
    }

    fn extract_portal_manager_id(doc: &Document) -> Option<String> {
        Self::doc_string(doc, "coding_agent_id")
            .filter(|v| !v.trim().is_empty())
            .or_else(|| {
                Self::nested_doc_string(doc, &["settings", "managerAgentId"])
                    .filter(|v| !v.trim().is_empty())
            })
            .or_else(|| {
                Self::nested_doc_string(doc, &["settings", "managerGroupId"])
                    .filter(|v| !v.trim().is_empty())
            })
            .or_else(|| {
                doc.get_array("tags").ok().and_then(|tags| {
                    tags.iter()
                        .filter_map(Bson::as_str)
                        .map(str::trim)
                        .find_map(|tag| tag.strip_prefix("manager:").map(str::to_string))
                })
            })
            .or_else(|| Self::doc_string(doc, "agent_id").filter(|v| !v.trim().is_empty()))
    }

    fn count_pending_with_status(
        items: Option<&Vec<serde_json::Value>>,
        pending_statuses: &[&str],
    ) -> u32 {
        items.map_or(0, |entries| {
            entries
                .iter()
                .filter(|entry| {
                    entry
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|status| {
                            pending_statuses.iter().any(|expected| status == *expected)
                        })
                })
                .count() as u32
        })
    }

    fn avatar_type_from_doc(doc: &Document) -> String {
        Self::nested_doc_string(doc, &["settings", "avatarType"])
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                doc.get_array("tags").ok().and_then(|tags| {
                    if tags
                        .iter()
                        .filter_map(Bson::as_str)
                        .any(|tag| tag.eq_ignore_ascii_case("avatar:internal"))
                    {
                        Some("internal_worker".to_string())
                    } else if tags
                        .iter()
                        .filter_map(Bson::as_str)
                        .any(|tag| tag.eq_ignore_ascii_case("avatar:external"))
                    {
                        Some("external_service".to_string())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| "external_service".to_string())
    }

    fn default_governance_state_json() -> serde_json::Value {
        serde_json::json!({
            "capabilityRequests": [],
            "gapProposals": [],
            "optimizationTickets": [],
            "runtimeLogs": []
        })
    }

    fn default_governance_config_json() -> serde_json::Value {
        serde_json::json!({
            "autoProposalTriggerCount": 3,
            "managerApprovalMode": "manager_decides",
            "optimizationMode": "dual_loop",
            "lowRiskAction": "auto_execute",
            "mediumRiskAction": "manager_review",
            "highRiskAction": "human_review",
            "autoCreateCapabilityRequests": true,
            "autoCreateOptimizationTickets": true,
            "requireHumanForPublish": true
        })
    }

    fn normalize_governance_state_candidate(value: serde_json::Value) -> Option<serde_json::Value> {
        match value {
            serde_json::Value::Object(mut map) => {
                map.remove("config");
                if map.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::Object(map))
                }
            }
            serde_json::Value::Null => None,
            other => Some(other),
        }
    }

    fn raw_portal_governance_json(doc: &Document) -> Option<serde_json::Value> {
        doc.get_document("settings")
            .ok()
            .and_then(|settings| settings.get("digitalAvatarGovernance"))
            .and_then(|value| bson::from_bson::<serde_json::Value>(value.clone()).ok())
    }

    fn raw_portal_governance_config_top_level_json(doc: &Document) -> Option<serde_json::Value> {
        doc.get_document("settings")
            .ok()
            .and_then(|settings| settings.get("digitalAvatarGovernanceConfig"))
            .and_then(|value| bson::from_bson::<serde_json::Value>(value.clone()).ok())
    }

    fn governance_state_json_from_portal_doc(doc: &Document) -> serde_json::Value {
        Self::raw_portal_governance_json(doc)
            .and_then(Self::normalize_governance_state_candidate)
            .unwrap_or_else(Self::default_governance_state_json)
    }

    fn raw_portal_governance_state_json(doc: &Document) -> Option<serde_json::Value> {
        Self::raw_portal_governance_json(doc).and_then(Self::normalize_governance_state_candidate)
    }

    fn governance_config_json_from_portal_doc(doc: &Document) -> serde_json::Value {
        let from_top = Self::raw_portal_governance_config_top_level_json(doc);
        if let Some(value) = from_top {
            return value;
        }
        Self::raw_portal_governance_json(doc)
            .and_then(|governance| governance.get("config").cloned())
            .unwrap_or_else(Self::default_governance_config_json)
    }

    fn raw_portal_governance_config_json(doc: &Document) -> Option<serde_json::Value> {
        let from_top = Self::raw_portal_governance_config_top_level_json(doc);
        if from_top.is_some() {
            return from_top;
        }
        Self::raw_portal_governance_json(doc)
            .and_then(|governance| governance.get("config").cloned())
    }

    fn governance_counts_from_state_json(state: &serde_json::Value) -> AvatarGovernanceCounts {
        let capability_requests = state
            .get("capabilityRequests")
            .and_then(serde_json::Value::as_array)
            .cloned();
        let gap_proposals = state
            .get("gapProposals")
            .and_then(serde_json::Value::as_array)
            .cloned();
        let optimization_tickets = state
            .get("optimizationTickets")
            .and_then(serde_json::Value::as_array)
            .cloned();
        let runtime_logs = state
            .get("runtimeLogs")
            .and_then(serde_json::Value::as_array)
            .cloned();

        AvatarGovernanceCounts {
            pending_capability_requests: Self::count_pending_with_status(
                capability_requests.as_ref(),
                &["pending"],
            ),
            pending_gap_proposals: Self::count_pending_with_status(
                gap_proposals.as_ref(),
                &["pending_approval"],
            ),
            pending_optimization_tickets: Self::count_pending_with_status(
                optimization_tickets.as_ref(),
                &["pending"],
            ),
            pending_runtime_logs: Self::count_pending_with_status(
                runtime_logs.as_ref(),
                &["pending"],
            ),
        }
    }

    fn governance_counts_from_doc(doc: &Document) -> AvatarGovernanceCounts {
        Self::governance_counts_from_state_json(&Self::governance_state_json_from_portal_doc(doc))
    }

    fn default_avatar_governance_payload(
        team_id: &str,
        portal_id: &str,
    ) -> AvatarGovernanceStatePayload {
        AvatarGovernanceStatePayload {
            portal_id: portal_id.to_string(),
            team_id: team_id.to_string(),
            state: Self::default_governance_state_json(),
            config: Self::default_governance_config_json(),
            updated_at: Utc::now(),
        }
    }

    fn governance_payload_from_portal_doc(
        team_id: &str,
        portal_id: &str,
        portal_doc: &Document,
        updated_at: DateTime<Utc>,
    ) -> AvatarGovernanceStatePayload {
        AvatarGovernanceStatePayload {
            portal_id: portal_id.to_string(),
            team_id: team_id.to_string(),
            state: Self::governance_state_json_from_portal_doc(portal_doc),
            config: Self::governance_config_json_from_portal_doc(portal_doc),
            updated_at,
        }
    }

    async fn upsert_avatar_governance_state_payload(
        &self,
        payload: &AvatarGovernanceStatePayload,
    ) -> Result<(), mongodb::error::Error> {
        self.avatar_governance_states()
            .replace_one(
                doc! { "team_id": &payload.team_id, "portal_id": &payload.portal_id },
                AvatarGovernanceStateDoc {
                    id: None,
                    portal_id: payload.portal_id.clone(),
                    team_id: payload.team_id.clone(),
                    state: payload.state.clone(),
                    config: payload.config.clone(),
                    updated_at: payload.updated_at,
                },
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(())
    }

    async fn sync_avatar_governance_payload_to_portal(
        &self,
        payload: &AvatarGovernanceStatePayload,
    ) -> Result<Option<u64>, mongodb::error::Error> {
        let Ok(team_oid) = ObjectId::parse_str(&payload.team_id) else {
            return Ok(None);
        };
        let Ok(portal_oid) = ObjectId::parse_str(&payload.portal_id) else {
            return Ok(None);
        };

        let update_result = self
            .portals()
            .update_one(
                doc! {
                    "_id": portal_oid,
                    "team_id": team_oid,
                    "is_deleted": { "$ne": true }
                },
                doc! {
                    "$set": {
                        "settings.digitalAvatarGovernance": bson::to_bson(&payload.state).unwrap_or(Bson::Null),
                        "settings.digitalAvatarGovernanceConfig": bson::to_bson(&payload.config).unwrap_or(Bson::Null),
                        "updated_at": bson::DateTime::from_chrono(payload.updated_at),
                    }
                },
                None,
            )
            .await?;

        Ok(Some(update_result.matched_count))
    }

    async fn sync_avatar_governance_payload_to_portal_if_current(
        &self,
        payload: &AvatarGovernanceStatePayload,
        expected_updated_at: DateTime<Utc>,
    ) -> Result<Option<u64>, mongodb::error::Error> {
        let Ok(team_oid) = ObjectId::parse_str(&payload.team_id) else {
            return Ok(None);
        };
        let Ok(portal_oid) = ObjectId::parse_str(&payload.portal_id) else {
            return Ok(None);
        };

        let update_result = self
            .portals()
            .update_one(
                doc! {
                    "_id": portal_oid,
                    "team_id": team_oid,
                    "is_deleted": { "$ne": true },
                    "updated_at": bson::DateTime::from_chrono(expected_updated_at),
                },
                doc! {
                    "$set": {
                        "settings.digitalAvatarGovernance": bson::to_bson(&payload.state).unwrap_or(Bson::Null),
                        "settings.digitalAvatarGovernanceConfig": bson::to_bson(&payload.config).unwrap_or(Bson::Null),
                        "updated_at": bson::DateTime::from_chrono(payload.updated_at),
                    }
                },
                None,
            )
            .await?;

        Ok(Some(update_result.matched_count))
    }

    fn extract_portal_service_agent_id(doc: &Document) -> Option<String> {
        Self::doc_string(doc, "service_agent_id")
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                Self::nested_doc_string(doc, &["settings", "serviceRuntimeAgentId"])
                    .filter(|value| !value.trim().is_empty())
            })
            .or_else(|| Self::doc_string(doc, "agent_id").filter(|value| !value.trim().is_empty()))
    }

    fn normalize_optional_scope_id(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    fn index_governance_state_docs(
        docs: Vec<AvatarGovernanceStateDoc>,
    ) -> (
        HashMap<(String, String), AvatarGovernanceStateDoc>,
        HashMap<(String, String), u64>,
        Vec<AvatarGovernanceDuplicateStateRow>,
        u64,
    ) {
        let mut grouped: HashMap<(String, String), Vec<AvatarGovernanceStateDoc>> = HashMap::new();
        for doc in docs {
            grouped
                .entry((doc.team_id.clone(), doc.portal_id.clone()))
                .or_default()
                .push(doc);
        }

        let mut indexed = HashMap::new();
        let mut counts = HashMap::new();
        let mut duplicate_rows = Vec::new();
        let mut duplicate_state_docs = 0u64;

        for ((team_id, portal_id), mut entries) in grouped {
            entries.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
            let state_doc_count = entries.len() as u64;
            counts.insert((team_id.clone(), portal_id.clone()), state_doc_count);

            if state_doc_count > 1 {
                duplicate_state_docs += state_doc_count - 1;
                duplicate_rows.push(AvatarGovernanceDuplicateStateRow {
                    team_id: team_id.clone(),
                    portal_id: portal_id.clone(),
                    state_doc_count,
                    retained_updated_at: entries[0].updated_at.to_rfc3339(),
                    duplicate_updated_at: entries
                        .iter()
                        .skip(1)
                        .map(|doc| doc.updated_at.to_rfc3339())
                        .collect(),
                });
            }

            let retained = entries.remove(0);
            indexed.insert((team_id, portal_id), retained);
        }

        duplicate_rows.sort_by(|left, right| {
            left.team_id
                .cmp(&right.team_id)
                .then_with(|| left.portal_id.cmp(&right.portal_id))
        });

        (indexed, counts, duplicate_rows, duplicate_state_docs)
    }

    fn classify_governance_divergence(
        has_state_doc: bool,
        has_portal_governance: bool,
        state_matches_portal_state: bool,
        config_matches_portal_config: bool,
    ) -> AvatarGovernanceDivergenceKind {
        match (has_state_doc, has_portal_governance) {
            (false, false) => AvatarGovernanceDivergenceKind::DefaultOnly,
            (true, false) => AvatarGovernanceDivergenceKind::StateOnly,
            (false, true) => AvatarGovernanceDivergenceKind::SettingsOnly,
            (true, true) if state_matches_portal_state && config_matches_portal_config => {
                AvatarGovernanceDivergenceKind::InSync
            }
            _ => AvatarGovernanceDivergenceKind::Differing,
        }
    }

    async fn refresh_avatar_governance_state_read_model(
        &self,
        payload: &AvatarGovernanceStatePayload,
    ) -> Result<(), mongodb::error::Error> {
        self.upsert_avatar_governance_state_payload(payload).await
    }

    async fn refresh_avatar_governance_event_read_model(
        &self,
        team_id: &str,
        portal_id: &str,
        state: &serde_json::Value,
        config: &serde_json::Value,
    ) -> Result<u64, mongodb::error::Error> {
        self.seed_avatar_governance_events_if_missing(team_id, portal_id, state, config)
            .await
    }

    async fn persist_avatar_governance_events(
        &self,
        events: Vec<AvatarGovernanceEventDoc>,
    ) -> Result<u64, mongodb::error::Error> {
        if events.is_empty() {
            return Ok(0);
        }
        let inserted = events.len() as u64;
        let _ = self
            .avatar_governance_events()
            .insert_many(events, None)
            .await?;
        Ok(inserted)
    }

    async fn refresh_avatar_governance_projection_after_write(
        &self,
        payload: &AvatarGovernanceStatePayload,
    ) -> Result<Option<u64>, mongodb::error::Error> {
        let matched_portals = self
            .sync_avatar_governance_payload_to_portal(payload)
            .await?;
        let _ = self
            .sync_avatar_instance_projections(&payload.team_id)
            .await?;
        Ok(matched_portals)
    }

    fn derive_avatar_governance_payload_from_portal_doc(
        team_id: &str,
        portal_id: &str,
        portal_doc: &Document,
    ) -> AvatarGovernanceStatePayload {
        let updated_at = portal_doc
            .get_datetime("updated_at")
            .ok()
            .map(|value| value.to_chrono())
            .unwrap_or_else(Utc::now);
        Self::governance_payload_from_portal_doc(team_id, portal_id, portal_doc, updated_at)
    }

    fn sort_and_limit_avatar_governance_events(
        mut events: Vec<AvatarGovernanceEventPayload>,
        limit: usize,
    ) -> Vec<AvatarGovernanceEventPayload> {
        events.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.event_id.cmp(&left.event_id))
        });
        events.truncate(limit);
        events
    }

    async fn derive_avatar_governance_event_payloads(
        &self,
        team_id: &str,
        portal_id: &str,
    ) -> Result<Vec<AvatarGovernanceEventPayload>, mongodb::error::Error> {
        let payload = self.get_avatar_governance_state(team_id, portal_id).await?;
        Ok(Self::governance_event_payloads_from_snapshot(
            team_id,
            portal_id,
            &payload.state,
            &payload.config,
            payload.updated_at,
        ))
    }

    async fn derive_team_avatar_governance_event_payloads(
        &self,
        team_id: &str,
        portal_id: Option<&str>,
        excluded_portal_ids: &HashSet<String>,
    ) -> Result<Vec<AvatarGovernanceEventPayload>, mongodb::error::Error> {
        let scoped_portal_id = portal_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut resolved_portal_ids = excluded_portal_ids.clone();
        let mut derived = Vec::new();

        let mut state_filter = doc! { "team_id": team_id };
        if let Some(portal_id) = scoped_portal_id.as_deref() {
            state_filter.insert("portal_id", portal_id);
        }
        let state_docs: Vec<AvatarGovernanceStateDoc> = self
            .avatar_governance_states()
            .find(state_filter, None)
            .await?
            .try_collect()
            .await?;
        for state_doc in state_docs {
            if !resolved_portal_ids.insert(state_doc.portal_id.clone()) {
                continue;
            }
            derived.extend(Self::governance_event_payloads_from_snapshot(
                &state_doc.team_id,
                &state_doc.portal_id,
                &state_doc.state,
                &state_doc.config,
                state_doc.updated_at,
            ));
        }

        let team_oid = match ObjectId::parse_str(team_id) {
            Ok(value) => value,
            Err(_) => return Ok(derived),
        };
        let mut portal_filter = Self::avatar_portal_filter(Some(team_oid));
        if let Some(portal_id) = scoped_portal_id.as_deref() {
            let portal_oid = match ObjectId::parse_str(portal_id) {
                Ok(value) => value,
                Err(_) => return Ok(derived),
            };
            portal_filter.insert("_id", portal_oid);
        }
        let portal_docs: Vec<Document> = self
            .portals()
            .find(portal_filter, None)
            .await?
            .try_collect()
            .await?;
        for portal_doc in portal_docs {
            let Some(portal_oid) = portal_doc.get_object_id("_id").ok() else {
                continue;
            };
            let portal_id = portal_oid.to_hex();
            if !resolved_portal_ids.insert(portal_id.clone()) {
                continue;
            }
            let payload = Self::derive_avatar_governance_payload_from_portal_doc(
                team_id,
                &portal_id,
                &portal_doc,
            );
            derived.extend(Self::governance_event_payloads_from_snapshot(
                team_id,
                &portal_id,
                &payload.state,
                &payload.config,
                payload.updated_at,
            ));
        }

        Ok(derived)
    }

    fn binding_issue_message(
        issue: &AvatarBindingIssueKind,
        explicit_coding_agent_id: Option<&str>,
        explicit_service_agent_id: Option<&str>,
        effective_manager_agent_id: Option<&str>,
        effective_service_agent_id: Option<&str>,
        manager_agent: Option<&TeamAgentDoc>,
        service_agent: Option<&TeamAgentDoc>,
    ) -> String {
        match issue {
            AvatarBindingIssueKind::MissingExplicitManagerBinding => {
                "portal 缺少显式 coding_agent_id 绑定".to_string()
            }
            AvatarBindingIssueKind::MissingExplicitServiceBinding => {
                "portal 缺少显式 service_agent_id 绑定".to_string()
            }
            AvatarBindingIssueKind::MissingManagerBinding => "无法解析有效 manager agent".to_string(),
            AvatarBindingIssueKind::MissingServiceBinding => "无法解析有效 service agent".to_string(),
            AvatarBindingIssueKind::ManagerAgentNotFound => format!(
                "manager agent '{}' 在团队中不存在",
                effective_manager_agent_id.unwrap_or(explicit_coding_agent_id.unwrap_or(""))
            ),
            AvatarBindingIssueKind::ServiceAgentNotFound => format!(
                "service agent '{}' 在团队中不存在",
                effective_service_agent_id.unwrap_or(explicit_service_agent_id.unwrap_or(""))
            ),
            AvatarBindingIssueKind::ManagerRoleMismatch => format!(
                "manager agent '{}' 是 {}:{}，不是 digital_avatar:manager",
                effective_manager_agent_id.unwrap_or(explicit_coding_agent_id.unwrap_or("")),
                manager_agent
                    .and_then(|agent| agent.agent_domain.as_deref())
                    .unwrap_or("general"),
                manager_agent
                    .and_then(|agent| agent.agent_role.as_deref())
                    .unwrap_or("default")
            ),
            AvatarBindingIssueKind::ServiceRoleMismatch => format!(
                "service agent '{}' 是 {}:{}，不是 digital_avatar:service",
                effective_service_agent_id.unwrap_or(explicit_service_agent_id.unwrap_or("")),
                service_agent
                    .and_then(|agent| agent.agent_domain.as_deref())
                    .unwrap_or("general"),
                service_agent
                    .and_then(|agent| agent.agent_role.as_deref())
                    .unwrap_or("default")
            ),
            AvatarBindingIssueKind::OwnerManagerMismatch => format!(
                "service agent '{}' 的 owner_manager_agent_id 是 '{}'，但当前 manager agent 是 '{}'",
                effective_service_agent_id.unwrap_or(explicit_service_agent_id.unwrap_or("")),
                service_agent
                    .and_then(|agent| agent.owner_manager_agent_id.as_deref())
                    .unwrap_or(""),
                effective_manager_agent_id.unwrap_or(explicit_coding_agent_id.unwrap_or(""))
            ),
            AvatarBindingIssueKind::SameAgentReused => format!(
                "manager agent '{}' 和 service agent '{}' 指向了同一个 agent",
                effective_manager_agent_id.unwrap_or(explicit_coding_agent_id.unwrap_or("")),
                effective_service_agent_id.unwrap_or(explicit_service_agent_id.unwrap_or(""))
            ),
        }
    }

    fn binding_issue_messages(
        issues: &[AvatarBindingIssueKind],
        explicit_coding_agent_id: Option<&str>,
        explicit_service_agent_id: Option<&str>,
        effective_manager_agent_id: Option<&str>,
        effective_service_agent_id: Option<&str>,
        manager_agent: Option<&TeamAgentDoc>,
        service_agent: Option<&TeamAgentDoc>,
    ) -> Vec<String> {
        issues
            .iter()
            .map(|issue| {
                Self::binding_issue_message(
                    issue,
                    explicit_coding_agent_id,
                    explicit_service_agent_id,
                    effective_manager_agent_id,
                    effective_service_agent_id,
                    manager_agent,
                    service_agent,
                )
            })
            .collect()
    }

    fn governance_event_payloads_from_snapshot(
        team_id: &str,
        portal_id: &str,
        state: &serde_json::Value,
        config: &serde_json::Value,
        created_at: DateTime<Utc>,
    ) -> Vec<AvatarGovernanceEventPayload> {
        Self::diff_governance_events_at(
            portal_id,
            team_id,
            &Self::default_governance_state_json(),
            &Self::default_governance_config_json(),
            state,
            config,
            None,
            Some("system"),
            created_at,
        )
        .into_iter()
        .map(Into::into)
        .collect()
    }

    fn avatar_read_side_effect_audit_items() -> Vec<AvatarReadSideEffectAuditItem> {
        vec![
            AvatarReadSideEffectAuditItem {
                operation: "get_avatar_governance_state".to_string(),
                file: "crates/agime-team-server/src/agent/service_mongo.rs".to_string(),
                side_effects: vec![
                    "no database writes; derives payload from portal doc when state doc is missing"
                        .to_string(),
                ],
            },
            AvatarReadSideEffectAuditItem {
                operation: "list_avatar_governance_queue".to_string(),
                file: "crates/agime-team-server/src/agent/service_mongo.rs".to_string(),
                side_effects: vec![
                    "no database writes; reuses derived governance payload fallback".to_string(),
                ],
            },
            AvatarReadSideEffectAuditItem {
                operation: "list_avatar_instance_projections".to_string(),
                file: "crates/agime-team-server/src/agent/service_mongo.rs".to_string(),
                side_effects: vec![
                    "no database writes; derives projection rows from portals and governance state"
                        .to_string(),
                ],
            },
            AvatarReadSideEffectAuditItem {
                operation: "get_avatar_workbench_snapshot".to_string(),
                file: "crates/agime-team-server/src/agent/service_mongo.rs".to_string(),
                side_effects: vec![
                    "no database writes for derived reports; merges persisted non-derived reports in memory"
                        .to_string(),
                    "reads governance queues through derived state fallback without writing"
                        .to_string(),
                ],
            },
        ]
    }

    fn governance_entity_snapshots_from_state(
        state: &serde_json::Value,
    ) -> HashMap<String, GovernanceEntitySnapshot> {
        fn first_text(item: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> String {
            keys.iter()
                .find_map(|key| item.get(*key).and_then(serde_json::Value::as_str))
                .unwrap_or("")
                .trim()
                .to_string()
        }

        let mut entries = HashMap::new();
        let Some(root) = state.as_object() else {
            return entries;
        };

        let specs: [(&str, &str, &[&str]); 4] = [
            (
                "capabilityRequests",
                "capability",
                &["detail", "decisionReason", "requestedScope"],
            ),
            (
                "gapProposals",
                "proposal",
                &["description", "decisionReason", "expectedGain"],
            ),
            (
                "optimizationTickets",
                "ticket",
                &["proposal", "decisionReason", "evidence", "expectedGain"],
            ),
            (
                "runtimeLogs",
                "runtime",
                &["proposal", "evidence", "expectedGain"],
            ),
        ];

        for (array_key, entity_type, detail_keys) in specs {
            let Some(items) = root.get(array_key).and_then(serde_json::Value::as_array) else {
                continue;
            };
            for item in items {
                let Some(map) = item.as_object() else {
                    continue;
                };
                let id = map
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let title = map
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if id.is_empty() || title.is_empty() {
                    continue;
                }

                let status = map
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let detail = first_text(map, detail_keys);
                let key = format!("{entity_type}:{id}");
                entries.insert(
                    key,
                    GovernanceEntitySnapshot {
                        entity_type,
                        id,
                        title,
                        status,
                        detail,
                        meta: serde_json::Value::Object(map.clone()),
                    },
                );
            }
        }

        entries
    }

    fn diff_governance_events_at(
        portal_id: &str,
        team_id: &str,
        current_state: &serde_json::Value,
        current_config: &serde_json::Value,
        next_state: &serde_json::Value,
        next_config: &serde_json::Value,
        actor_id: Option<&str>,
        actor_name: Option<&str>,
        created_at: DateTime<Utc>,
    ) -> Vec<AvatarGovernanceEventDoc> {
        let current_entries = Self::governance_entity_snapshots_from_state(current_state);
        let next_entries = Self::governance_entity_snapshots_from_state(next_state);
        let mut events = Vec::new();

        for (key, next_entry) in &next_entries {
            match current_entries.get(key) {
                None => events.push(AvatarGovernanceEventDoc {
                    id: None,
                    portal_id: portal_id.to_string(),
                    team_id: team_id.to_string(),
                    event_type: "created".to_string(),
                    entity_type: next_entry.entity_type.to_string(),
                    entity_id: Some(next_entry.id.clone()),
                    title: next_entry.title.clone(),
                    status: Some(next_entry.status.clone()),
                    detail: (!next_entry.detail.is_empty()).then(|| next_entry.detail.clone()),
                    actor_id: actor_id.map(str::to_string),
                    actor_name: actor_name.map(str::to_string),
                    meta: next_entry.meta.clone(),
                    created_at,
                }),
                Some(current_entry)
                    if current_entry.status != next_entry.status
                        || current_entry.title != next_entry.title
                        || current_entry.detail != next_entry.detail
                        || current_entry.meta != next_entry.meta =>
                {
                    events.push(AvatarGovernanceEventDoc {
                        id: None,
                        portal_id: portal_id.to_string(),
                        team_id: team_id.to_string(),
                        event_type: "updated".to_string(),
                        entity_type: next_entry.entity_type.to_string(),
                        entity_id: Some(next_entry.id.clone()),
                        title: next_entry.title.clone(),
                        status: Some(next_entry.status.clone()),
                        detail: (!next_entry.detail.is_empty()).then(|| next_entry.detail.clone()),
                        actor_id: actor_id.map(str::to_string),
                        actor_name: actor_name.map(str::to_string),
                        meta: serde_json::json!({
                            "before": current_entry.meta,
                            "after": next_entry.meta,
                        }),
                        created_at,
                    });
                }
                Some(_) => {}
            }
        }

        for (key, current_entry) in &current_entries {
            if next_entries.contains_key(key) {
                continue;
            }
            events.push(AvatarGovernanceEventDoc {
                id: None,
                portal_id: portal_id.to_string(),
                team_id: team_id.to_string(),
                event_type: "removed".to_string(),
                entity_type: current_entry.entity_type.to_string(),
                entity_id: Some(current_entry.id.clone()),
                title: current_entry.title.clone(),
                status: Some(current_entry.status.clone()),
                detail: (!current_entry.detail.is_empty()).then(|| current_entry.detail.clone()),
                actor_id: actor_id.map(str::to_string),
                actor_name: actor_name.map(str::to_string),
                meta: current_entry.meta.clone(),
                created_at,
            });
        }

        if current_config != next_config {
            events.push(AvatarGovernanceEventDoc {
                id: None,
                portal_id: portal_id.to_string(),
                team_id: team_id.to_string(),
                event_type: "config_updated".to_string(),
                entity_type: "config".to_string(),
                entity_id: None,
                title: "治理配置已更新".to_string(),
                status: None,
                detail: None,
                actor_id: actor_id.map(str::to_string),
                actor_name: actor_name.map(str::to_string),
                meta: serde_json::json!({
                    "before": current_config,
                    "after": next_config,
                }),
                created_at,
            });
        }

        events
    }

    fn diff_governance_events(
        portal_id: &str,
        team_id: &str,
        current_state: &serde_json::Value,
        current_config: &serde_json::Value,
        next_state: &serde_json::Value,
        next_config: &serde_json::Value,
        actor_id: Option<&str>,
        actor_name: Option<&str>,
    ) -> Vec<AvatarGovernanceEventDoc> {
        Self::diff_governance_events_at(
            portal_id,
            team_id,
            current_state,
            current_config,
            next_state,
            next_config,
            actor_id,
            actor_name,
            Utc::now(),
        )
    }

    fn governance_queue_items_from_state(
        state: &serde_json::Value,
    ) -> Vec<AvatarGovernanceQueueItemPayload> {
        let mut rows = Vec::new();
        let Some(root) = state.as_object() else {
            return rows;
        };

        if let Some(items) = root
            .get("capabilityRequests")
            .and_then(serde_json::Value::as_array)
        {
            for item in items {
                let Some(map) = item.as_object() else {
                    continue;
                };
                let status = map
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("pending");
                if status != "pending" && status != "needs_human" {
                    continue;
                }
                let source_id = map
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let title = map
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if source_id.is_empty() || title.is_empty() {
                    continue;
                }
                let detail = map
                    .get("detail")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| {
                        map.get("decisionReason")
                            .and_then(serde_json::Value::as_str)
                    })
                    .unwrap_or("")
                    .to_string();
                let ts = map
                    .get("updatedAt")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| map.get("createdAt").and_then(serde_json::Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let mut meta = Vec::new();
                if let Some(risk) = map.get("risk").and_then(serde_json::Value::as_str) {
                    meta.push(risk.to_string());
                }
                if let Some(source) = map.get("source").and_then(serde_json::Value::as_str) {
                    meta.push(source.to_string());
                }
                rows.push(AvatarGovernanceQueueItemPayload {
                    id: format!("queue:capability:{source_id}"),
                    kind: "capability".to_string(),
                    title,
                    detail,
                    status: status.to_string(),
                    ts,
                    meta,
                    source_id,
                });
            }
        }

        if let Some(items) = root
            .get("gapProposals")
            .and_then(serde_json::Value::as_array)
        {
            for item in items {
                let Some(map) = item.as_object() else {
                    continue;
                };
                let status = map
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("draft");
                if status != "pending_approval" && status != "approved" && status != "pilot" {
                    continue;
                }
                let source_id = map
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let title = map
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if source_id.is_empty() || title.is_empty() {
                    continue;
                }
                let detail = map
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let ts = map
                    .get("updatedAt")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| map.get("createdAt").and_then(serde_json::Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let meta = map
                    .get("expectedGain")
                    .and_then(serde_json::Value::as_str)
                    .map(|value| vec![value.to_string()])
                    .unwrap_or_default();
                let mut meta = meta;
                if let Some(reason) = map
                    .get("decisionReason")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                {
                    meta.push(format!("决策说明: {}", reason.trim()));
                }
                rows.push(AvatarGovernanceQueueItemPayload {
                    id: format!("queue:proposal:{source_id}"),
                    kind: "proposal".to_string(),
                    title,
                    detail,
                    status: status.to_string(),
                    ts,
                    meta,
                    source_id,
                });
            }
        }

        if let Some(items) = root
            .get("optimizationTickets")
            .and_then(serde_json::Value::as_array)
        {
            for item in items {
                let Some(map) = item.as_object() else {
                    continue;
                };
                let status = map
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("pending");
                if status != "pending" && status != "approved" && status != "experimenting" {
                    continue;
                }
                let source_id = map
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let title = map
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if source_id.is_empty() || title.is_empty() {
                    continue;
                }
                let detail = map
                    .get("proposal")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| map.get("evidence").and_then(serde_json::Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let ts = map
                    .get("updatedAt")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| map.get("createdAt").and_then(serde_json::Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let mut meta = Vec::new();
                if let Some(problem_type) =
                    map.get("problemType").and_then(serde_json::Value::as_str)
                {
                    meta.push(problem_type.to_string());
                }
                if let Some(risk) = map.get("risk").and_then(serde_json::Value::as_str) {
                    meta.push(risk.to_string());
                }
                if let Some(reason) = map
                    .get("decisionReason")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                {
                    meta.push(format!("决策说明: {}", reason.trim()));
                }
                rows.push(AvatarGovernanceQueueItemPayload {
                    id: format!("queue:ticket:{source_id}"),
                    kind: "ticket".to_string(),
                    title,
                    detail,
                    status: status.to_string(),
                    ts,
                    meta,
                    source_id,
                });
            }
        }

        rows.sort_by(|a, b| b.ts.cmp(&a.ts));
        rows
    }

    fn avatar_portal_filter(team_oid: Option<ObjectId>) -> Document {
        let mut filter = doc! {
            "is_deleted": { "$ne": true },
            "$or": [
                { "domain": "avatar" },
                { "tags": "digital-avatar" },
                { "tags": { "$regex": "^avatar:", "$options": "i" } }
            ]
        };
        if let Some(value) = team_oid {
            filter.insert("team_id", value);
        }
        filter
    }

    fn is_avatar_portal_doc(doc: &Document) -> bool {
        if Self::doc_has_string(doc, "domain", "avatar")
            || Self::nested_doc_has_string(doc, &["settings", "domain"], "avatar")
        {
            return true;
        }

        doc.get_array("tags").ok().is_some_and(|tags| {
            tags.iter()
                .filter_map(Bson::as_str)
                .map(str::trim)
                .any(|tag| {
                    tag.eq_ignore_ascii_case("digital-avatar")
                        || tag.to_ascii_lowercase().starts_with("avatar:")
                })
        })
    }

    async fn seed_avatar_governance_events_if_missing(
        &self,
        team_id: &str,
        portal_id: &str,
        state: &serde_json::Value,
        config: &serde_json::Value,
    ) -> Result<u64, mongodb::error::Error> {
        let existing = self
            .avatar_governance_events()
            .count_documents(doc! { "team_id": team_id, "portal_id": portal_id }, None)
            .await?;
        if existing > 0 {
            return Ok(0);
        }

        let events = Self::diff_governance_events(
            portal_id,
            team_id,
            &Self::default_governance_state_json(),
            &Self::default_governance_config_json(),
            state,
            config,
            None,
            Some("system"),
        );
        if events.is_empty() {
            return Ok(0);
        }

        self.persist_avatar_governance_events(events).await
    }

    fn avatar_instance_from_portal_doc(team_id: &str, doc: &Document) -> Option<AvatarInstanceDoc> {
        let portal_id = doc.get_object_id("_id").ok()?.to_hex();
        let slug = Self::doc_string(doc, "slug")?;
        let name = Self::doc_string(doc, "name")?;
        let status = Self::doc_string(doc, "status").unwrap_or_else(|| "draft".to_string());
        let document_access_mode = Self::doc_string(doc, "document_access_mode")
            .unwrap_or_else(|| "read_only".to_string());
        let portal_updated_at = doc
            .get_datetime("updated_at")
            .ok()
            .map(|value| value.to_chrono())
            .unwrap_or_else(Utc::now);

        Some(AvatarInstanceDoc {
            id: None,
            portal_id,
            team_id: team_id.to_string(),
            slug,
            name,
            status,
            avatar_type: Self::avatar_type_from_doc(doc),
            manager_agent_id: Self::extract_portal_manager_id(doc),
            service_agent_id: Self::extract_portal_service_agent_id(doc),
            document_access_mode,
            governance_counts: Self::governance_counts_from_doc(doc),
            portal_updated_at,
            projected_at: Utc::now(),
        })
    }

    async fn infer_legacy_avatar_metadata(
        &self,
        team_id: &str,
        agent: &TeamAgent,
    ) -> Result<Option<(String, Option<String>)>, mongodb::error::Error> {
        let manager_candidate = Self::is_legacy_avatar_manager_candidate(agent);
        let service_candidate = Self::is_legacy_avatar_service_candidate(agent);
        if !manager_candidate && !service_candidate {
            return Ok(None);
        }

        let team_oid = match ObjectId::parse_str(team_id) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        let agent_id = agent.id.trim();
        if agent_id.is_empty() {
            return Ok(None);
        }

        let binding_filter = doc! {
            "$or": [
                { "coding_agent_id": agent_id },
                { "service_agent_id": agent_id },
                { "agent_id": agent_id },
                { "tags": format!("manager:{agent_id}") },
                { "settings.managerAgentId": agent_id },
                { "settings.managerGroupId": agent_id },
                { "settings.serviceRuntimeAgentId": agent_id }
            ]
        };
        let filter = doc! {
            "$and": [
                Self::avatar_portal_filter(Some(team_oid)),
                binding_filter,
            ]
        };

        let mut cursor = self.portals().find(filter, None).await?;
        let mut manager_match = false;
        let mut service_owner_manager_id: Option<String> = None;

        while let Some(portal) = cursor.try_next().await? {
            let explicit_manager_match = Self::doc_has_string(&portal, "coding_agent_id", agent_id)
                || Self::doc_has_manager_tag(&portal, agent_id)
                || Self::nested_doc_has_string(&portal, &["settings", "managerAgentId"], agent_id)
                || Self::nested_doc_has_string(&portal, &["settings", "managerGroupId"], agent_id);
            let explicit_service_match =
                Self::doc_has_string(&portal, "service_agent_id", agent_id)
                    || Self::nested_doc_has_string(
                        &portal,
                        &["settings", "serviceRuntimeAgentId"],
                        agent_id,
                    );
            let legacy_single_agent_match = Self::doc_has_string(&portal, "agent_id", agent_id);

            if manager_candidate && (explicit_manager_match || legacy_single_agent_match) {
                manager_match = true;
                break;
            }

            if service_candidate && (explicit_service_match || legacy_single_agent_match) {
                service_owner_manager_id = Self::extract_portal_manager_id(&portal)
                    .filter(|value| value.trim() != agent_id);
            }
        }

        if manager_match {
            return Ok(Some(("manager".to_string(), None)));
        }
        if service_candidate && service_owner_manager_id.is_some() {
            return Ok(Some(("service".to_string(), service_owner_manager_id)));
        }
        if service_candidate {
            let has_orphan_service_hint =
                Self::is_dedicated_avatar_marker(agent.description.as_deref())
                    || Self::normalize_compact_text(Some(agent.name.as_str()))
                        .ends_with("分身agent");
            if has_orphan_service_hint {
                return Ok(Some(("service".to_string(), None)));
            }
        }

        Ok(None)
    }

    async fn backfill_legacy_avatar_agent_metadata(
        &self,
        agent: &mut TeamAgent,
    ) -> Result<(), mongodb::error::Error> {
        let explicit_role = Self::explicit_avatar_marker_role(agent.description.as_deref());
        if agent.agent_domain.as_deref() == Some("digital_avatar") && agent.agent_role.is_some() {
            if explicit_role.is_none() || explicit_role == agent.agent_role.as_deref() {
                return Ok(());
            }
        }

        let inferred = self
            .infer_legacy_avatar_metadata(&agent.team_id, agent)
            .await?;
        let (role, owner_manager_agent_id) = match explicit_role {
            Some("manager") => ("manager".to_string(), None),
            Some("service") => inferred.unwrap_or_else(|| ("service".to_string(), None)),
            Some(other) => (other.to_string(), None),
            None => {
                let Some(pair) = inferred else {
                    return Ok(());
                };
                pair
            }
        };

        let mut set_doc = doc! {
            "agent_domain": "digital_avatar",
            "agent_role": role.clone(),
            "updated_at": bson::DateTime::from_chrono(Utc::now()),
        };
        match owner_manager_agent_id.clone() {
            Some(value) => {
                set_doc.insert("owner_manager_agent_id", value);
            }
            None => {
                set_doc.insert("owner_manager_agent_id", Bson::Null);
            }
        }

        self.agents()
            .update_one(
                doc! { "agent_id": &agent.id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;

        agent.agent_domain = Some("digital_avatar".to_string());
        agent.agent_role = Some(role);
        agent.owner_manager_agent_id = owner_manager_agent_id;
        Ok(())
    }

    /// Whether an agent is considered a digital-avatar dedicated agent and therefore
    /// must not be reused as a generic provision template.
    pub async fn is_dedicated_avatar_agent(
        &self,
        team_id: &str,
        agent: &TeamAgent,
    ) -> Result<bool, mongodb::error::Error> {
        if agent.agent_domain.as_deref() == Some("digital_avatar") {
            return Ok(true);
        }
        if Self::is_legacy_avatar_manager_candidate(agent)
            || Self::is_legacy_avatar_service_candidate(agent)
        {
            return Ok(self
                .infer_legacy_avatar_metadata(team_id, agent)
                .await?
                .is_some());
        }
        Ok(false)
    }

    fn is_v4_mission_selectable_agent(agent: &TeamAgent) -> bool {
        let domain = agent.agent_domain.as_deref().unwrap_or("general");
        let role = agent.agent_role.as_deref().unwrap_or("default");
        domain == "general" && role == "default"
    }

    // Permission checks - query embedded members array in teams collection
    pub async fn is_team_member(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        // team_id is ObjectId string
        let oid = match mongodb::bson::oid::ObjectId::parse_str(team_id) {
            Ok(oid) => oid,
            Err(_) => return Ok(false),
        };
        let result = self
            .teams()
            .find_one(
                doc! {
                    "_id": oid,
                    "members.user_id": user_id
                },
                None,
            )
            .await?;
        Ok(result.is_some())
    }

    pub async fn is_team_admin(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        // team_id is ObjectId string
        let oid = match mongodb::bson::oid::ObjectId::parse_str(team_id) {
            Ok(oid) => oid,
            Err(_) => return Ok(false),
        };
        let result = self
            .teams()
            .find_one(
                doc! {
                    "_id": oid,
                    "members": {
                        "$elemMatch": {
                            "user_id": user_id,
                            "role": { "$in": ["admin", "owner"] }
                        }
                    }
                },
                None,
            )
            .await?;
        Ok(result.is_some())
    }

    pub async fn get_agent_team_id(
        &self,
        agent_id: &str,
    ) -> Result<Option<String>, mongodb::error::Error> {
        let result = self
            .agents()
            .find_one(doc! { "agent_id": agent_id }, None)
            .await?;
        Ok(result.map(|r| r.team_id))
    }

    pub async fn get_task_team_id(
        &self,
        task_id: &str,
    ) -> Result<Option<String>, mongodb::error::Error> {
        let result = self
            .tasks()
            .find_one(doc! { "task_id": task_id }, None)
            .await?;
        Ok(result.map(|r| r.team_id))
    }

    // Agent CRUD operations
    pub async fn create_agent(&self, req: CreateAgentRequest) -> Result<TeamAgent, ServiceError> {
        validate_name(&req.name)?;
        validate_api_url(&req.api_url)?;
        validate_model(&req.model)?;

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let api_format = req.api_format.as_deref().unwrap_or("openai");
        let enabled_extensions =
            sanitize_enabled_extensions(req.enabled_extensions.unwrap_or_else(|| {
                BuiltinExtension::defaults()
                    .into_iter()
                    .map(|ext| AgentExtensionConfig {
                        extension: ext,
                        enabled: true,
                    })
                    .collect()
            }));

        let doc = TeamAgentDoc {
            id: None,
            agent_id: id.clone(),
            team_id: req.team_id,
            name: req.name,
            description: req.description,
            avatar: req.avatar,
            system_prompt: req.system_prompt,
            api_url: req.api_url,
            model: req.model,
            api_key: req.api_key,
            api_format: api_format.to_string(),
            status: "idle".to_string(),
            last_error: None,
            enabled_extensions,
            custom_extensions: req.custom_extensions.unwrap_or_default(),
            agent_domain: req.agent_domain,
            agent_role: req.agent_role,
            owner_manager_agent_id: req.owner_manager_agent_id,
            template_source_agent_id: req.template_source_agent_id,
            allowed_groups: req.allowed_groups.unwrap_or_default(),
            max_concurrent_tasks: req.max_concurrent_tasks.unwrap_or(1),
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            context_limit: req.context_limit,
            thinking_enabled: req.thinking_enabled.unwrap_or(true),
            assigned_skills: req.assigned_skills.unwrap_or_default(),
            auto_approve_chat: true,
            created_at: now,
            updated_at: now,
        };

        // Insert via raw BSON document to preserve custom extension envs.
        let mut insert_doc = mongodb::bson::to_document(&doc)
            .map_err(|e| ServiceError::Internal(format!("Serialize agent doc failed: {}", e)))?;
        insert_doc.insert(
            "custom_extensions",
            custom_extensions_to_bson(&doc.custom_extensions),
        );
        self.db
            .collection::<Document>("team_agents")
            .insert_one(insert_doc, None)
            .await?;
        self.get_agent(&id)
            .await?
            .ok_or_else(|| ServiceError::Internal(format!("Agent not found after insert: {}", id)))
    }

    pub async fn get_agent(&self, id: &str) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let doc = self
            .agents()
            .find_one(doc! { "agent_id": id }, None)
            .await?;
        let Some(doc) = doc else {
            return Ok(None);
        };
        let mut agent: TeamAgent = doc.into();
        self.backfill_legacy_avatar_agent_metadata(&mut agent)
            .await?;
        Ok(Some(agent))
    }

    /// Get agent with API key preserved (for internal server-side use only, never expose to API).
    pub async fn get_agent_with_key(
        &self,
        id: &str,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let doc = self
            .agents()
            .find_one(doc! { "agent_id": id, "is_deleted": { "$ne": true } }, None)
            .await?;
        let Some(doc) = doc else {
            return Ok(None);
        };
        let mut agent = agent_doc_with_key(doc);
        self.backfill_legacy_avatar_agent_metadata(&mut agent)
            .await?;
        Ok(Some(agent))
    }

    /// Get the first agent for a team that has an API key configured (for internal server-side use).
    pub async fn get_first_agent_with_key(
        &self,
        team_id: &str,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let options = mongodb::options::FindOneOptions::builder()
            .sort(doc! { "created_at": -1 })
            .build();
        let doc = self
            .agents()
            .find_one(
                doc! {
                    "team_id": team_id,
                    "is_deleted": { "$ne": true },
                    "api_key": { "$exists": true, "$nin": [null, ""] }
                },
                options,
            )
            .await?;
        Ok(doc.map(agent_doc_with_key))
    }

    /// Pick the preferred default AI-describe provider agent for a team.
    ///
    /// Preference order:
    /// 1. Oldest non-digital-avatar agent with an API key
    /// 2. Oldest digital-avatar manager agent with an API key
    /// 3. Oldest remaining non-deleted agent with an API key
    pub async fn get_default_ai_describe_agent_with_key(
        &self,
        team_id: &str,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "created_at": 1 })
            .limit(64)
            .build();
        let cursor = self
            .agents()
            .find(
                doc! {
                    "team_id": team_id,
                    "is_deleted": { "$ne": true },
                    "api_key": { "$exists": true, "$nin": [null, ""] }
                },
                options,
            )
            .await?;
        let docs: Vec<TeamAgentDoc> = cursor.try_collect().await?;
        let mut agents: Vec<TeamAgent> = Vec::with_capacity(docs.len());
        for doc in docs {
            let mut agent = agent_doc_with_key(doc);
            self.backfill_legacy_avatar_agent_metadata(&mut agent)
                .await?;
            agents.push(agent);
        }

        if let Some(agent) = agents
            .iter()
            .find(|agent| agent.agent_domain.as_deref() != Some("digital_avatar"))
            .cloned()
        {
            return Ok(Some(agent));
        }

        if let Some(agent) = agents
            .iter()
            .find(|agent| {
                agent.agent_domain.as_deref() == Some("digital_avatar")
                    && agent.agent_role.as_deref() == Some("manager")
            })
            .cloned()
        {
            return Ok(Some(agent));
        }

        Ok(agents.into_iter().next())
    }

    pub async fn list_agents(
        &self,
        query: ListAgentsQuery,
    ) -> Result<PaginatedResponse<TeamAgent>, mongodb::error::Error> {
        let clamped_limit = query.limit.min(100);
        let limit = clamped_limit as i64;
        let skip = ((query.page.saturating_sub(1)) * clamped_limit) as u64;

        let filter = doc! { "team_id": &query.team_id };
        let total = self.agents().count_documents(filter.clone(), None).await?;

        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "created_at": -1 })
            .skip(skip)
            .limit(limit)
            .build();
        let cursor = self.agents().find(filter, options).await?;

        let docs: Vec<TeamAgentDoc> = cursor.try_collect().await?;
        let mut items: Vec<TeamAgent> = docs.into_iter().map(|d| d.into()).collect();
        for agent in &mut items {
            self.backfill_legacy_avatar_agent_metadata(agent).await?;
        }

        Ok(PaginatedResponse::new(
            items,
            total,
            query.page,
            query.limit,
        ))
    }

    pub async fn sync_avatar_instance_projections(
        &self,
        team_id: &str,
    ) -> Result<Vec<AvatarInstanceSummary>, mongodb::error::Error> {
        let projection_docs = self.derive_avatar_instance_projection_docs(team_id).await?;
        let mut projections = Vec::with_capacity(projection_docs.len());
        for projection in projection_docs {
            let portal_id = projection.portal_id.clone();
            self.avatar_instances()
                .replace_one(
                    doc! { "portal_id": &portal_id, "team_id": team_id },
                    projection.clone(),
                    mongodb::options::ReplaceOptions::builder()
                        .upsert(true)
                        .build(),
                )
                .await?;
            projections.push(AvatarInstanceSummary::from(projection));
        }

        Ok(projections)
    }

    async fn derive_avatar_instance_projection_docs(
        &self,
        team_id: &str,
    ) -> Result<Vec<AvatarInstanceDoc>, mongodb::error::Error> {
        let team_oid = match ObjectId::parse_str(team_id) {
            Ok(v) => v,
            Err(_) => return Ok(Vec::new()),
        };

        let filter = Self::avatar_portal_filter(Some(team_oid));
        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .build();
        let cursor = self.portals().find(filter, options).await?;
        let portal_docs: Vec<Document> = cursor.try_collect().await?;
        let governance_docs: Vec<AvatarGovernanceStateDoc> = self
            .avatar_governance_states()
            .find(doc! { "team_id": team_id }, None)
            .await?
            .try_collect()
            .await?;
        let governance_counts_by_portal: std::collections::HashMap<String, AvatarGovernanceCounts> =
            governance_docs
                .into_iter()
                .map(|doc| {
                    (
                        doc.portal_id,
                        Self::governance_counts_from_state_json(&doc.state),
                    )
                })
                .collect();
        let mut projections = Vec::with_capacity(portal_docs.len());

        for portal_doc in portal_docs {
            let Some(mut projection) = Self::avatar_instance_from_portal_doc(team_id, &portal_doc)
            else {
                continue;
            };
            if let Some(counts) = governance_counts_by_portal.get(&projection.portal_id) {
                projection.governance_counts = counts.clone();
            }
            projections.push(projection);
        }

        Ok(projections)
    }

    pub async fn get_avatar_governance_state(
        &self,
        team_id: &str,
        portal_id: &str,
    ) -> Result<AvatarGovernanceStatePayload, mongodb::error::Error> {
        if let Some(doc) = self
            .avatar_governance_states()
            .find_one(doc! { "team_id": team_id, "portal_id": portal_id }, None)
            .await?
        {
            return Ok(doc.into());
        }

        let portal_oid = match ObjectId::parse_str(portal_id) {
            Ok(value) => value,
            Err(_) => {
                tracing::warn!(
                    team_id = %team_id,
                    portal_id = %portal_id,
                    "avatar governance requested with invalid portal id; returning default payload"
                );
                return Ok(Self::default_avatar_governance_payload(team_id, portal_id));
            }
        };

        let team_oid = match ObjectId::parse_str(team_id) {
            Ok(value) => value,
            Err(_) => {
                tracing::warn!(
                    team_id = %team_id,
                    portal_id = %portal_id,
                    "avatar governance requested with invalid team id; returning default payload"
                );
                return Ok(Self::default_avatar_governance_payload(team_id, portal_id));
            }
        };

        let Some(portal_doc) = self
            .portals()
            .find_one(
                doc! {
                    "_id": portal_oid,
                    "team_id": team_oid,
                    "is_deleted": { "$ne": true }
                },
                None,
            )
            .await?
        else {
            tracing::warn!(
                team_id = %team_id,
                portal_id = %portal_id,
                "avatar governance requested for missing portal; returning default payload"
            );
            return Ok(Self::default_avatar_governance_payload(team_id, portal_id));
        };

        if !Self::is_avatar_portal_doc(&portal_doc) {
            tracing::warn!(
                team_id = %team_id,
                portal_id = %portal_id,
                "avatar governance requested for non-avatar portal; continuing with compatibility fallback"
            );
        }

        Ok(Self::derive_avatar_governance_payload_from_portal_doc(
            team_id,
            portal_id,
            &portal_doc,
        ))
    }

    pub async fn update_avatar_governance_state(
        &self,
        team_id: &str,
        portal_id: &str,
        next_state: Option<serde_json::Value>,
        next_config: Option<serde_json::Value>,
        actor_id: Option<&str>,
        actor_name: Option<&str>,
    ) -> Result<AvatarGovernanceStatePayload, mongodb::error::Error> {
        let current = self.get_avatar_governance_state(team_id, portal_id).await?;
        let previous_state = current.state.clone();
        let previous_config = current.config.clone();
        let state = next_state.unwrap_or(previous_state.clone());
        let config = next_config.unwrap_or(previous_config.clone());
        let updated_at = Utc::now();
        let payload = AvatarGovernanceStatePayload {
            portal_id: portal_id.to_string(),
            team_id: team_id.to_string(),
            state: state.clone(),
            config: config.clone(),
            updated_at,
        };
        let governance_events = Self::diff_governance_events(
            portal_id,
            team_id,
            &previous_state,
            &previous_config,
            &state,
            &config,
            actor_id,
            actor_name,
        );

        self.refresh_avatar_governance_state_read_model(&payload)
            .await?;

        let matched_portals = self
            .refresh_avatar_governance_projection_after_write(&payload)
            .await?;
        if matches!(matched_portals, Some(0)) {
            tracing::warn!(
                team_id = %team_id,
                portal_id = %portal_id,
                "avatar governance state updated without a matching portal document; governance projection may be orphaned"
            );
        }

        if !governance_events.is_empty() {
            let _ = self
                .persist_avatar_governance_events(governance_events)
                .await?;
        }

        Ok(payload)
    }

    pub async fn update_avatar_governance_state_if_current(
        &self,
        current: &AvatarGovernanceStatePayload,
        next_state: Option<serde_json::Value>,
        next_config: Option<serde_json::Value>,
        actor_id: Option<&str>,
        actor_name: Option<&str>,
    ) -> Result<Option<AvatarGovernanceStatePayload>, mongodb::error::Error> {
        let previous_state = current.state.clone();
        let previous_config = current.config.clone();
        let state = next_state.unwrap_or(previous_state.clone());
        let config = next_config.unwrap_or(previous_config.clone());
        let updated_at = Utc::now();
        let payload = AvatarGovernanceStatePayload {
            portal_id: current.portal_id.clone(),
            team_id: current.team_id.clone(),
            state: state.clone(),
            config: config.clone(),
            updated_at,
        };
        let governance_events = Self::diff_governance_events(
            &current.portal_id,
            &current.team_id,
            &previous_state,
            &previous_config,
            &state,
            &config,
            actor_id,
            actor_name,
        );

        let matched_portals = self
            .sync_avatar_governance_payload_to_portal_if_current(&payload, current.updated_at)
            .await?;
        if matches!(matched_portals, Some(0)) {
            return Ok(None);
        }

        self.refresh_avatar_governance_state_read_model(&payload)
            .await?;

        let _ = self
            .sync_avatar_instance_projections(&payload.team_id)
            .await?;

        if !governance_events.is_empty() {
            let _ = self
                .persist_avatar_governance_events(governance_events)
                .await?;
        }

        Ok(Some(payload))
    }

    pub async fn list_avatar_governance_events(
        &self,
        team_id: &str,
        portal_id: &str,
        limit: u32,
    ) -> Result<Vec<AvatarGovernanceEventPayload>, mongodb::error::Error> {
        let limit = limit.clamp(1, 200) as usize;
        let docs: Vec<AvatarGovernanceEventDoc> = self
            .avatar_governance_events()
            .find(
                doc! { "team_id": team_id, "portal_id": portal_id },
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "created_at": -1, "_id": -1 })
                    .limit(limit as i64)
                    .build(),
            )
            .await?
            .try_collect()
            .await?;

        if !docs.is_empty() {
            return Ok(Self::sort_and_limit_avatar_governance_events(
                docs.into_iter().map(Into::into).collect(),
                limit,
            ));
        }

        Ok(Self::sort_and_limit_avatar_governance_events(
            self.derive_avatar_governance_event_payloads(team_id, portal_id)
                .await?,
            limit,
        ))
    }

    pub async fn list_team_avatar_governance_events(
        &self,
        team_id: &str,
        portal_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<AvatarGovernanceEventPayload>, mongodb::error::Error> {
        let limit = limit.clamp(1, 500) as usize;
        let mut filter = doc! { "team_id": team_id };
        if let Some(value) = portal_id.filter(|value| !value.trim().is_empty()) {
            filter.insert("portal_id", value);
        }

        let docs: Vec<AvatarGovernanceEventDoc> = self
            .avatar_governance_events()
            .find(
                filter,
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "created_at": -1, "_id": -1 })
                    .limit(limit as i64)
                    .build(),
            )
            .await?
            .try_collect()
            .await?;
        let mut events: Vec<AvatarGovernanceEventPayload> =
            docs.into_iter().map(Into::into).collect();
        if events.len() < limit {
            let excluded_portal_ids = events
                .iter()
                .map(|event| event.portal_id.clone())
                .collect::<HashSet<_>>();
            events.extend(
                self.derive_team_avatar_governance_event_payloads(
                    team_id,
                    portal_id,
                    &excluded_portal_ids,
                )
                .await?,
            );
        }

        Ok(Self::sort_and_limit_avatar_governance_events(events, limit))
    }

    pub async fn list_avatar_governance_queue(
        &self,
        team_id: &str,
        portal_id: &str,
    ) -> Result<Vec<AvatarGovernanceQueueItemPayload>, mongodb::error::Error> {
        let payload = self.get_avatar_governance_state(team_id, portal_id).await?;
        Ok(Self::governance_queue_items_from_state(&payload.state))
    }

    pub async fn backfill_avatar_governance_storage(
        &self,
        team_id: Option<&str>,
    ) -> Result<AvatarGovernanceBackfillReport, ServiceError> {
        let team_oid = match team_id {
            Some(value) if !value.trim().is_empty() => {
                Some(ObjectId::parse_str(value).map_err(|e| {
                    ServiceError::Internal(format!("Invalid team id for governance backfill: {e}"))
                })?)
            }
            _ => None,
        };

        let filter = Self::avatar_portal_filter(team_oid);
        let portal_docs: Vec<Document> = self
            .portals()
            .find(
                filter,
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await?
            .try_collect()
            .await?;

        let mut report = AvatarGovernanceBackfillReport::default();
        let mut touched_team_ids = HashSet::new();

        for portal_doc in portal_docs {
            let Some(portal_oid) = portal_doc.get_object_id("_id").ok() else {
                continue;
            };
            let Some(portal_team_oid) = portal_doc.get_object_id("team_id").ok() else {
                continue;
            };

            let portal_id = portal_oid.to_hex();
            let portal_team_id = portal_team_oid.to_hex();
            let payload = Self::governance_payload_from_portal_doc(
                &portal_team_id,
                &portal_id,
                &portal_doc,
                Utc::now(),
            );

            report.portals_scanned += 1;
            touched_team_ids.insert(portal_team_id.clone());

            let existing = self
                .avatar_governance_states()
                .find_one(
                    doc! { "team_id": &portal_team_id, "portal_id": &portal_id },
                    None,
                )
                .await?;

            if existing.is_none() {
                self.refresh_avatar_governance_state_read_model(&payload)
                    .await?;
                report.states_created += 1;
            }

            report.events_seeded += self
                .refresh_avatar_governance_event_read_model(
                    &portal_team_id,
                    &portal_id,
                    &payload.state,
                    &payload.config,
                )
                .await?;
        }

        for team_id in touched_team_ids {
            let _ = self.sync_avatar_instance_projections(&team_id).await?;
            report.projections_synced_teams += 1;
        }

        Ok(report)
    }

    pub async fn audit_avatar_deep_water(
        &self,
        team_id: Option<&str>,
    ) -> Result<AvatarDeepWaterAuditReport, ServiceError> {
        let requested_team_id = Self::normalize_optional_scope_id(team_id);
        let team_oid = match requested_team_id.as_deref() {
            Some(value) => Some(ObjectId::parse_str(value).map_err(|e| {
                ServiceError::Internal(format!("Invalid team id for avatar deep-water audit: {e}"))
            })?),
            _ => None,
        };

        let filter = Self::avatar_portal_filter(team_oid);
        let portal_docs: Vec<Document> = self
            .portals()
            .find(
                filter,
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await?
            .try_collect()
            .await?;

        let mut team_ids = Vec::new();
        for portal_doc in &portal_docs {
            if let Ok(team_oid) = portal_doc.get_object_id("team_id") {
                let team_id = team_oid.to_hex();
                if !team_ids.iter().any(|value| value == &team_id) {
                    team_ids.push(team_id);
                }
            }
        }

        let governance_docs: Vec<AvatarGovernanceStateDoc> =
            if let Some(team_id) = requested_team_id.as_deref() {
                self.avatar_governance_states()
                    .find(doc! { "team_id": team_id }, None)
                    .await?
                    .try_collect()
                    .await?
            } else {
                self.avatar_governance_states()
                    .find(doc! {}, None)
                    .await?
                    .try_collect()
                    .await?
            };
        let total_state_docs = governance_docs.len() as u64;

        let agent_docs: Vec<TeamAgentDoc> = if team_ids.is_empty() {
            Vec::new()
        } else if team_ids.len() == 1 {
            self.agents()
                .find(doc! { "team_id": &team_ids[0] }, None)
                .await?
                .try_collect()
                .await?
        } else {
            self.agents()
                .find(doc! { "team_id": { "$in": &team_ids } }, None)
                .await?
                .try_collect()
                .await?
        };

        let (mut governance_map, governance_state_doc_counts, duplicate_rows, duplicate_state_docs) =
            Self::index_governance_state_docs(governance_docs);

        let agent_map: HashMap<(String, String), TeamAgentDoc> = agent_docs
            .into_iter()
            .map(|doc| ((doc.team_id.clone(), doc.agent_id.clone()), doc))
            .collect();

        let mut governance = AvatarGovernanceDivergenceAuditReport {
            total_avatar_portals: portal_docs.len() as u64,
            total_state_docs,
            duplicate_state_docs,
            duplicate_rows,
            ..AvatarGovernanceDivergenceAuditReport::default()
        };
        let mut bindings = AvatarBindingAuditReport {
            total_avatar_portals: portal_docs.len() as u64,
            ..AvatarBindingAuditReport::default()
        };

        for portal_doc in portal_docs {
            let Some(portal_oid) = portal_doc.get_object_id("_id").ok() else {
                continue;
            };
            let Some(portal_team_oid) = portal_doc.get_object_id("team_id").ok() else {
                continue;
            };

            let portal_id = portal_oid.to_hex();
            let portal_team_id = portal_team_oid.to_hex();
            let portal_name =
                Self::doc_string(&portal_doc, "name").unwrap_or_else(|| "未命名分身".to_string());
            let slug = Self::doc_string(&portal_doc, "slug").unwrap_or_default();

            let explicit_portal_state = Self::raw_portal_governance_state_json(&portal_doc);
            let explicit_portal_config = Self::raw_portal_governance_config_json(&portal_doc);
            let state_doc = governance_map.remove(&(portal_team_id.clone(), portal_id.clone()));

            let state_doc_exists = state_doc.is_some();
            let portal_state_exists = explicit_portal_state.is_some();
            let portal_config_exists = explicit_portal_config.is_some();
            let has_portal_governance = portal_state_exists || portal_config_exists;
            let state_matches_portal_state = state_doc
                .as_ref()
                .and_then(|doc| {
                    explicit_portal_state
                        .as_ref()
                        .map(|value| doc.state == *value)
                })
                .unwrap_or(false);
            let config_matches_portal_config = state_doc
                .as_ref()
                .and_then(|doc| {
                    explicit_portal_config
                        .as_ref()
                        .map(|value| doc.config == *value)
                })
                .unwrap_or(false);

            let governance_classification = Self::classify_governance_divergence(
                state_doc_exists,
                has_portal_governance,
                state_matches_portal_state,
                config_matches_portal_config,
            );

            match governance_classification {
                AvatarGovernanceDivergenceKind::DefaultOnly => governance.default_only += 1,
                AvatarGovernanceDivergenceKind::InSync => governance.in_sync += 1,
                AvatarGovernanceDivergenceKind::StateOnly => governance.state_only += 1,
                AvatarGovernanceDivergenceKind::SettingsOnly => governance.settings_only += 1,
                AvatarGovernanceDivergenceKind::Differing => governance.differing += 1,
            }

            governance.rows.push(AvatarGovernanceDivergenceRow {
                team_id: portal_team_id.clone(),
                portal_id: portal_id.clone(),
                portal_name: portal_name.clone(),
                slug: slug.clone(),
                classification: governance_classification,
                state_doc_exists,
                portal_state_exists,
                portal_config_exists,
                state_matches_portal_state,
                config_matches_portal_config,
            });

            let explicit_coding_agent_id = Self::doc_string(&portal_doc, "coding_agent_id")
                .filter(|value| !value.trim().is_empty());
            let explicit_service_agent_id = Self::doc_string(&portal_doc, "service_agent_id")
                .filter(|value| !value.trim().is_empty());
            let effective_manager_agent_id = Self::extract_portal_manager_id(&portal_doc);
            let effective_service_agent_id = Self::extract_portal_service_agent_id(&portal_doc);

            let manager_agent = effective_manager_agent_id
                .as_ref()
                .and_then(|agent_id| agent_map.get(&(portal_team_id.clone(), agent_id.clone())));
            let service_agent = effective_service_agent_id
                .as_ref()
                .and_then(|agent_id| agent_map.get(&(portal_team_id.clone(), agent_id.clone())));

            let mut issues = Vec::new();

            if explicit_coding_agent_id.is_none() {
                issues.push(AvatarBindingIssueKind::MissingExplicitManagerBinding);
                bindings.missing_explicit_manager_binding += 1;
            }
            if explicit_service_agent_id.is_none() {
                issues.push(AvatarBindingIssueKind::MissingExplicitServiceBinding);
                bindings.missing_explicit_service_binding += 1;
            }
            if effective_manager_agent_id.is_none() {
                issues.push(AvatarBindingIssueKind::MissingManagerBinding);
                bindings.missing_manager_binding += 1;
            }
            if effective_service_agent_id.is_none() {
                issues.push(AvatarBindingIssueKind::MissingServiceBinding);
                bindings.missing_service_binding += 1;
            }
            if effective_manager_agent_id.is_some() && manager_agent.is_none() {
                issues.push(AvatarBindingIssueKind::ManagerAgentNotFound);
                bindings.manager_agent_not_found += 1;
            }
            if effective_service_agent_id.is_some() && service_agent.is_none() {
                issues.push(AvatarBindingIssueKind::ServiceAgentNotFound);
                bindings.service_agent_not_found += 1;
            }
            if let Some(manager_agent) = manager_agent {
                if manager_agent.agent_domain.as_deref() != Some("digital_avatar")
                    || manager_agent.agent_role.as_deref() != Some("manager")
                {
                    issues.push(AvatarBindingIssueKind::ManagerRoleMismatch);
                    bindings.manager_role_mismatch += 1;
                }
            }
            if let Some(service_agent) = service_agent {
                if service_agent.agent_domain.as_deref() != Some("digital_avatar")
                    || service_agent.agent_role.as_deref() != Some("service")
                {
                    issues.push(AvatarBindingIssueKind::ServiceRoleMismatch);
                    bindings.service_role_mismatch += 1;
                }
                if service_agent.owner_manager_agent_id.as_deref()
                    != effective_manager_agent_id.as_deref()
                {
                    issues.push(AvatarBindingIssueKind::OwnerManagerMismatch);
                    bindings.owner_manager_mismatch += 1;
                }
            }
            if effective_manager_agent_id.is_some()
                && effective_manager_agent_id == effective_service_agent_id
            {
                issues.push(AvatarBindingIssueKind::SameAgentReused);
                bindings.same_agent_reused += 1;
            }

            let issue_messages = Self::binding_issue_messages(
                &issues,
                explicit_coding_agent_id.as_deref(),
                explicit_service_agent_id.as_deref(),
                effective_manager_agent_id.as_deref(),
                effective_service_agent_id.as_deref(),
                manager_agent,
                service_agent,
            );
            let has_shadow_invariant_issue = issues.iter().any(|issue| {
                matches!(
                    issue,
                    AvatarBindingIssueKind::ManagerRoleMismatch
                        | AvatarBindingIssueKind::ServiceRoleMismatch
                        | AvatarBindingIssueKind::OwnerManagerMismatch
                )
            });
            if has_shadow_invariant_issue {
                bindings.shadow_invariant_rows += 1;
            }

            if issues.is_empty() {
                bindings.valid += 1;
            }

            bindings.rows.push(AvatarBindingAuditRow {
                team_id: portal_team_id,
                portal_id,
                portal_name,
                slug,
                explicit_coding_agent_id,
                explicit_service_agent_id,
                effective_manager_agent_id,
                effective_service_agent_id,
                manager_agent_domain: manager_agent.and_then(|agent| agent.agent_domain.clone()),
                manager_agent_role: manager_agent.and_then(|agent| agent.agent_role.clone()),
                service_agent_domain: service_agent.and_then(|agent| agent.agent_domain.clone()),
                service_agent_role: service_agent.and_then(|agent| agent.agent_role.clone()),
                service_owner_manager_agent_id: service_agent
                    .and_then(|agent| agent.owner_manager_agent_id.clone()),
                issues,
                issue_messages,
            });
        }

        let mut orphan_rows: Vec<_> = governance_map
            .into_values()
            .map(|doc| {
                let state_key = (doc.team_id.clone(), doc.portal_id.clone());
                AvatarGovernanceOrphanStateRow {
                    team_id: doc.team_id,
                    portal_id: doc.portal_id,
                    state_doc_count: governance_state_doc_counts
                        .get(&state_key)
                        .copied()
                        .unwrap_or(1),
                    updated_at: doc.updated_at.to_rfc3339(),
                }
            })
            .collect();
        orphan_rows.sort_by(|left, right| {
            left.team_id
                .cmp(&right.team_id)
                .then_with(|| left.portal_id.cmp(&right.portal_id))
        });
        governance.orphan_state_docs = orphan_rows.iter().map(|row| row.state_doc_count).sum();
        governance.orphan_rows = orphan_rows;

        Ok(AvatarDeepWaterAuditReport {
            generated_at: Utc::now().to_rfc3339(),
            requested_team_id,
            governance,
            bindings,
            read_side_effects: Self::avatar_read_side_effect_audit_items(),
        })
    }

    pub async fn list_avatar_instance_projections(
        &self,
        team_id: &str,
    ) -> Result<Vec<AvatarInstanceSummary>, mongodb::error::Error> {
        Ok(self
            .derive_avatar_instance_projection_docs(team_id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    fn build_avatar_manager_reports(
        latest_completed_mission: Option<&MissionListItem>,
        latest_completed_detail: Option<&MissionDoc>,
        latest_attention_mission: Option<&MissionListItem>,
        governance_events: &[AvatarGovernanceEventPayload],
        service_agent_name: &str,
        portal_id: &str,
        work_object_labels: &[String],
        delivery_outputs: &[String],
    ) -> Vec<AvatarWorkbenchReportItemPayload> {
        let mut reports = Vec::new();

        if let Some(mission) = latest_completed_mission {
            reports.push(AvatarWorkbenchReportItemPayload {
                id: format!("mission-delivery:{}", mission.mission_id),
                ts: parse_rfc3339_utc(&mission.updated_at).unwrap_or_else(Utc::now),
                kind: "delivery".to_string(),
                title: mission.goal.clone(),
                summary: latest_completed_detail
                    .and_then(|detail| detail.final_summary.clone())
                    .unwrap_or_else(|| {
                        "这项长程委托已经完成，交付摘要会优先展示在这里。".to_string()
                    }),
                status: mission_status_label(&mission.status).to_string(),
                source: service_agent_name.to_string(),
                recommendation: Some(
                    "先审阅交付结果，再决定是否继续优化、采用或交付。".to_string(),
                ),
                action_kind: Some("open_mission".to_string()),
                action_target_id: Some(mission.mission_id.clone()),
                work_objects: work_object_labels.iter().take(3).cloned().collect(),
                outputs: delivery_outputs.iter().take(3).cloned().collect(),
                needs_decision: false,
            });
        }

        if let Some(mission) = latest_attention_mission {
            let needs_decision = mission_needs_attention(&mission.status);
            reports.push(AvatarWorkbenchReportItemPayload {
                id: format!("mission-progress:{}", mission.mission_id),
                ts: parse_rfc3339_utc(&mission.updated_at).unwrap_or_else(Utc::now),
                kind: "progress".to_string(),
                title: mission.goal.clone(),
                summary: if mission.status == MissionStatus::Running {
                    "当前有复杂工作仍在处理中，可以继续跟进进度或等待下一轮汇报。".to_string()
                } else {
                    "当前有复杂工作需要恢复或继续决策，建议先查看原因再继续推进。".to_string()
                },
                status: mission_status_label(&mission.status).to_string(),
                source: service_agent_name.to_string(),
                recommendation: Some(if mission_needs_attention(&mission.status) {
                    "优先查看本次长程委托，再决定继续处理还是回到管理对话调整方案。".to_string()
                } else {
                    "先查看进度；如果对象、边界或交付目标需要变化，再回到管理对话调整。".to_string()
                }),
                action_kind: Some(if mission_needs_attention(&mission.status) {
                    "resume_mission".to_string()
                } else {
                    "open_mission".to_string()
                }),
                action_target_id: Some(mission.mission_id.clone()),
                work_objects: work_object_labels.iter().take(3).cloned().collect(),
                outputs: Vec::new(),
                needs_decision,
            });
        }

        for event in governance_events.iter().take(6) {
            let needs_decision = Self::governance_event_needs_decision(event);
            reports.push(AvatarWorkbenchReportItemPayload {
                id: format!("event:{}", event.event_id),
                ts: event.created_at,
                kind: if event.entity_type == "runtime" {
                    "runtime".to_string()
                } else {
                    "governance".to_string()
                },
                title: event.title.clone(),
                summary: event
                    .detail
                    .clone()
                    .unwrap_or_else(|| "新的治理或运行记录已经写入工作台。".to_string()),
                status: event.status.clone().unwrap_or_else(|| "logged".to_string()),
                source: event
                    .actor_name
                    .clone()
                    .or_else(|| event.actor_id.clone())
                    .unwrap_or_else(|| "系统汇总".to_string()),
                recommendation: Some(if event.entity_type == "runtime" {
                    "如果涉及失败恢复或对象补充，建议先打开日志与治理台查看细节。".to_string()
                } else {
                    "如果需要进一步确认权限、提案或策略，建议先打开治理台处理。".to_string()
                }),
                action_kind: Some(if event.entity_type == "runtime" {
                    "open_logs".to_string()
                } else {
                    "open_governance".to_string()
                }),
                action_target_id: Some(portal_id.to_string()),
                work_objects: work_object_labels.iter().take(2).cloned().collect(),
                outputs: Vec::new(),
                needs_decision,
            });
        }

        reports.sort_by(|left, right| right.ts.cmp(&left.ts));
        reports.dedup_by(|left, right| left.id == right.id);
        reports
    }

    fn merge_avatar_workbench_reports(
        derived_reports: &[AvatarWorkbenchReportItemPayload],
        persisted_reports: &[AvatarWorkbenchReportItemPayload],
        limit: usize,
    ) -> Vec<AvatarWorkbenchReportItemPayload> {
        let mut reports = persisted_reports
            .iter()
            .cloned()
            .chain(derived_reports.iter().cloned())
            .collect::<Vec<_>>();
        reports.sort_by(|left, right| right.ts.cmp(&left.ts));
        let mut seen = HashSet::new();
        let mut deduped = Vec::with_capacity(reports.len());
        for report in reports {
            if seen.insert(report.id.clone()) {
                deduped.push(report);
            }
        }
        deduped.truncate(limit);
        deduped
    }

    async fn list_persisted_avatar_manager_reports(
        &self,
        team_id: &str,
        portal_id: &str,
        limit: u32,
    ) -> Result<Vec<AvatarWorkbenchReportItemPayload>, mongodb::error::Error> {
        let docs = self
            .avatar_manager_reports()
            .find(
                doc! {
                    "team_id": team_id,
                    "portal_id": portal_id,
                    "report_source": {
                        "$exists": true,
                        "$ne": default_avatar_manager_report_source(),
                    }
                },
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .limit(i64::from(limit.clamp(1, 20)))
                    .build(),
            )
            .await?
            .try_collect::<Vec<AvatarManagerReportDoc>>()
            .await?;
        Ok(docs.into_iter().map(Into::into).collect())
    }

    pub async fn upsert_avatar_manager_report(
        &self,
        team_id: &str,
        portal_id: &str,
        report: AvatarWorkbenchReportItemPayload,
        report_source: &str,
    ) -> Result<(), mongodb::error::Error> {
        let now = Utc::now();
        self.avatar_manager_reports()
            .update_one(
                doc! {
                    "team_id": team_id,
                    "portal_id": portal_id,
                    "report_id": &report.id,
                    "report_source": report_source,
                },
                doc! {
                    "$set": {
                        "team_id": team_id,
                        "portal_id": portal_id,
                        "report_id": &report.id,
                        "report_source": report_source,
                        "kind": &report.kind,
                        "title": &report.title,
                        "summary": &report.summary,
                        "status": &report.status,
                        "source": &report.source,
                        "recommendation": bson::to_bson(&report.recommendation).unwrap_or(Bson::Null),
                        "action_kind": bson::to_bson(&report.action_kind).unwrap_or(Bson::Null),
                        "action_target_id": bson::to_bson(&report.action_target_id).unwrap_or(Bson::Null),
                        "work_objects": bson::to_bson(&report.work_objects).unwrap_or_else(|_| Bson::Array(Vec::new())),
                        "outputs": bson::to_bson(&report.outputs).unwrap_or_else(|_| Bson::Array(Vec::new())),
                        "needs_decision": report.needs_decision,
                        "created_at": bson::DateTime::from_chrono(report.ts),
                        "synced_at": bson::DateTime::from_chrono(now),
                    },
                    "$setOnInsert": {
                        "_id": ObjectId::new(),
                    }
                },
                mongodb::options::UpdateOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(())
    }

    pub async fn list_avatar_manager_reports(
        &self,
        team_id: &str,
        portal_id: &str,
        limit: u32,
    ) -> Result<Vec<AvatarWorkbenchReportItemPayload>, mongodb::error::Error> {
        let docs = self
            .avatar_manager_reports()
            .find(
                doc! { "team_id": team_id, "portal_id": portal_id },
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .limit(i64::from(limit.clamp(1, 20)))
                    .build(),
            )
            .await?
            .try_collect::<Vec<AvatarManagerReportDoc>>()
            .await?;
        Ok(docs.into_iter().map(Into::into).collect())
    }

    fn governance_event_needs_decision(event: &AvatarGovernanceEventPayload) -> bool {
        event
            .status
            .as_deref()
            .map(|status| {
                matches!(
                    status,
                    "pending"
                        | "review"
                        | "needs_review"
                        | "needs_human"
                        | "requires_approval"
                        | "attention"
                        | "failed"
                )
            })
            .unwrap_or(false)
            || event
                .meta
                .get("needs_decision")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            || matches!(
                event
                    .meta
                    .get("risk")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default(),
                "high" | "critical"
            )
    }

    async fn resolve_work_object_labels(
        &self,
        team_oid: Option<ObjectId>,
        portal_doc: Option<&Document>,
    ) -> Result<Vec<String>, mongodb::error::Error> {
        let Some(bound_ids) = portal_doc.and_then(|doc| doc.get_array("bound_document_ids").ok())
        else {
            return Ok(Vec::new());
        };
        let object_ids = bound_ids
            .iter()
            .filter_map(|value| value.as_str())
            .filter_map(|raw| ObjectId::parse_str(raw).ok())
            .collect::<Vec<_>>();
        if object_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut filter = doc! { "_id": { "$in": object_ids }, "is_deleted": false };
        if let Some(team_oid) = team_oid {
            filter.insert("team_id", team_oid);
        }
        let docs = self
            .documents_store()
            .find(
                filter,
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .limit(6)
                    .build(),
            )
            .await?
            .try_collect::<Vec<TeamDocument>>()
            .await?;
        Ok(docs
            .into_iter()
            .map(|doc| doc.display_name.unwrap_or(doc.name))
            .collect())
    }

    async fn resolve_delivery_outputs(
        &self,
        mission_id: Option<&str>,
    ) -> Result<Vec<String>, mongodb::error::Error> {
        let Some(mission_id) = mission_id else {
            return Ok(Vec::new());
        };
        let artifacts = self.list_mission_artifacts(mission_id).await?;
        Ok(artifacts
            .into_iter()
            .rev()
            .filter_map(|artifact| {
                artifact
                    .file_path
                    .filter(|path| !path.trim().is_empty())
                    .or_else(|| (!artifact.name.trim().is_empty()).then_some(artifact.name))
            })
            .take(4)
            .collect())
    }

    pub async fn get_avatar_workbench_snapshot(
        &self,
        team_id: &str,
        portal_id: &str,
    ) -> Result<AvatarWorkbenchSnapshotPayload, mongodb::error::Error> {
        let projections = self.list_avatar_instance_projections(team_id).await?;
        let projection = projections
            .iter()
            .find(|item| item.portal_id == portal_id)
            .cloned();

        let team_oid = ObjectId::parse_str(team_id).ok();
        let portal_oid = ObjectId::parse_str(portal_id).ok();
        let portal_doc = match (team_oid, portal_oid) {
            (Some(team_oid), Some(portal_oid)) => {
                self.portals()
                    .find_one(doc! { "_id": portal_oid, "team_id": team_oid }, None)
                    .await?
            }
            (_, Some(portal_oid)) => {
                self.portals()
                    .find_one(doc! { "_id": portal_oid }, None)
                    .await?
            }
            _ => None,
        };

        let avatar_name = projection
            .as_ref()
            .map(|item| item.name.clone())
            .or_else(|| {
                portal_doc
                    .as_ref()
                    .and_then(|doc| doc.get_str("name").ok().map(ToOwned::to_owned))
            })
            .unwrap_or_else(|| "未命名岗位".to_string());
        let avatar_type = projection
            .as_ref()
            .map(|item| item.avatar_type.clone())
            .or_else(|| {
                portal_doc.as_ref().and_then(|doc| {
                    doc.get_document("settings")
                        .ok()
                        .and_then(|settings| settings.get_document("digitalAvatarProfile").ok())
                        .and_then(|profile| {
                            profile.get_str("avatar_type").ok().map(ToOwned::to_owned)
                        })
                })
            })
            .unwrap_or_else(|| "unknown".to_string());
        let avatar_status = projection
            .as_ref()
            .map(|item| item.status.clone())
            .or_else(|| {
                portal_doc
                    .as_ref()
                    .and_then(|doc| doc.get_str("status").ok().map(ToOwned::to_owned))
            })
            .unwrap_or_else(|| "draft".to_string());
        let manager_agent_id = projection
            .as_ref()
            .and_then(|item| item.manager_agent_id.clone());
        let service_agent_id = projection
            .as_ref()
            .and_then(|item| item.service_agent_id.clone());
        let document_access_mode = projection
            .as_ref()
            .map(|item| item.document_access_mode.clone())
            .or_else(|| {
                portal_doc.as_ref().and_then(|doc| {
                    doc.get_str("document_access_mode")
                        .ok()
                        .map(ToOwned::to_owned)
                })
            })
            .unwrap_or_else(|| "read_only".to_string());
        let work_object_count = portal_doc
            .as_ref()
            .and_then(|doc| doc.get_array("bound_document_ids").ok())
            .map_or(0, |items| items.len() as u32);

        let governance_events = self
            .list_avatar_governance_events(team_id, portal_id, 40)
            .await
            .unwrap_or_default();
        let queue_items = self
            .list_avatar_governance_queue(team_id, portal_id)
            .await
            .unwrap_or_default();

        let missions = if let Some(manager_id) = manager_agent_id.as_deref() {
            self.list_missions(ListMissionsQuery {
                team_id: team_id.to_string(),
                agent_id: Some(manager_id.to_string()),
                status: None,
                page: 1,
                limit: 20,
            })
            .await
            .unwrap_or_default()
        } else {
            Vec::new()
        };

        let latest_completed_mission = missions
            .iter()
            .find(|mission| mission.status == MissionStatus::Completed);
        let latest_attention_mission = missions
            .iter()
            .find(|mission| {
                mission.status == MissionStatus::Failed || mission.status == MissionStatus::Paused
            })
            .or_else(|| {
                missions
                    .iter()
                    .find(|mission| mission.status == MissionStatus::Running)
            });

        let latest_completed_detail = if let Some(mission) = latest_completed_mission {
            self.get_mission(&mission.mission_id).await?
        } else {
            None
        };
        let work_object_labels = self
            .resolve_work_object_labels(team_oid, portal_doc.as_ref())
            .await
            .unwrap_or_default();
        let delivery_outputs = self
            .resolve_delivery_outputs(
                latest_completed_mission.map(|mission| mission.mission_id.as_str()),
            )
            .await
            .unwrap_or_default();

        let running_mission_count = missions
            .iter()
            .filter(|mission| mission.status == MissionStatus::Running)
            .count() as u32;
        let needs_attention_count = missions
            .iter()
            .filter(|mission| {
                mission.status == MissionStatus::Failed
                    || mission.status == MissionStatus::Paused
                    || mission.status == MissionStatus::Running
            })
            .count() as u32;

        let agent_ids = [manager_agent_id.as_deref(), service_agent_id.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let agent_names = self.batch_get_agent_names(&agent_ids).await;
        let service_agent_name = service_agent_id
            .as_deref()
            .and_then(|agent_id| agent_names.get(agent_id).cloned())
            .unwrap_or_else(|| avatar_name.clone());

        let derived_reports = Self::build_avatar_manager_reports(
            latest_completed_mission,
            latest_completed_detail.as_ref(),
            latest_attention_mission,
            &governance_events,
            &service_agent_name,
            portal_id,
            &work_object_labels,
            &delivery_outputs,
        );
        let persisted_reports = self
            .list_persisted_avatar_manager_reports(team_id, portal_id, 6)
            .await
            .unwrap_or_default();
        let reports = Self::merge_avatar_workbench_reports(&derived_reports, &persisted_reports, 4);

        let mut decisions = Vec::new();
        if let Some(mission) =
            latest_attention_mission.filter(|mission| mission_needs_attention(&mission.status))
        {
            decisions.push(AvatarWorkbenchDecisionItemPayload {
                id: format!("decision-mission:{}", mission.mission_id),
                ts: parse_rfc3339_utc(&mission.updated_at).unwrap_or_else(Utc::now),
                kind: "mission".to_string(),
                title: mission.goal.clone(),
                detail: "这项长程委托当前需要管理 Agent 继续决策或恢复执行。".to_string(),
                status: mission_status_label(&mission.status).to_string(),
                risk: "medium".to_string(),
                source: service_agent_name.clone(),
                recommendation: Some(
                    "先查看这次长程委托的进度和原因，再决定继续处理、调整边界，还是回到管理对话重新规划。"
                        .to_string(),
                ),
                work_objects: work_object_labels.iter().take(3).cloned().collect(),
                action_kind: Some("resume_mission".to_string()),
                action_target_id: Some(mission.mission_id.clone()),
            });
        }

        for item in queue_items.iter().take(4) {
            let risk = queue_item_risk(&item.meta).to_string();
            decisions.push(AvatarWorkbenchDecisionItemPayload {
                id: format!("decision-queue:{}", item.id),
                ts: parse_rfc3339_utc(&item.ts).unwrap_or_else(Utc::now),
                kind: item.kind.clone(),
                title: item.title.clone(),
                detail: item.detail.clone(),
                status: item.status.clone(),
                risk: risk.clone(),
                source: queue_item_source(&item.kind, &item.meta),
                recommendation: Some(queue_item_recommendation(&item.kind, &risk)),
                work_objects: work_object_labels.iter().take(2).cloned().collect(),
                action_kind: Some("open_governance".to_string()),
                action_target_id: Some(portal_id.to_string()),
            });
        }
        decisions.sort_by(|left, right| right.ts.cmp(&left.ts));
        decisions.dedup_by(|left, right| left.id == right.id);
        decisions.truncate(4);

        let latest_activity_at = reports
            .iter()
            .map(|item| item.ts)
            .chain(decisions.iter().map(|item| item.ts))
            .chain(
                missions
                    .iter()
                    .filter_map(|mission| parse_rfc3339_utc(&mission.updated_at)),
            )
            .chain(projection.as_ref().map(|item| item.projected_at))
            .max()
            .unwrap_or_else(Utc::now);

        Ok(AvatarWorkbenchSnapshotPayload {
            portal_id: portal_id.to_string(),
            team_id: team_id.to_string(),
            summary: AvatarWorkbenchSummaryPayload {
                portal_id: portal_id.to_string(),
                team_id: team_id.to_string(),
                avatar_name,
                avatar_type,
                avatar_status,
                manager_agent_id,
                service_agent_id,
                document_access_mode,
                work_object_count,
                pending_decision_count: decisions.len() as u32,
                running_mission_count,
                needs_attention_count,
                last_activity_at: latest_activity_at,
            },
            reports,
            decisions,
        })
    }

    pub async fn update_agent(
        &self,
        id: &str,
        req: UpdateAgentRequest,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let now = Utc::now();
        let mut set_doc = doc! { "updated_at": bson::DateTime::from_chrono(now) };

        if let Some(name) = req.name {
            set_doc.insert("name", name);
        }
        if let Some(desc) = req.description {
            set_doc.insert("description", desc);
        }
        if let Some(avatar) = req.avatar {
            set_doc.insert("avatar", avatar);
        }
        if let Some(system_prompt) = req.system_prompt {
            set_doc.insert("system_prompt", system_prompt);
        }
        if let Some(api_url) = req.api_url {
            set_doc.insert("api_url", api_url);
        }
        if let Some(model) = req.model {
            set_doc.insert("model", model);
        }
        if let Some(ref api_key) = req.api_key {
            tracing::info!(
                "Updating API key for agent {}: key length = {}",
                id,
                api_key.len()
            );
            set_doc.insert("api_key", api_key.clone());
        }
        if let Some(api_format) = req.api_format {
            set_doc.insert("api_format", api_format);
        }
        if let Some(status) = req.status {
            set_doc.insert("status", status.to_string());
        }
        if let Some(ref extensions) = req.enabled_extensions {
            let sanitized = sanitize_enabled_extensions(extensions.clone());
            let ext_bson = mongodb::bson::to_bson(&sanitized).unwrap_or(bson::Bson::Array(vec![]));
            set_doc.insert("enabled_extensions", ext_bson);
        }
        if let Some(ref custom_ext) = req.custom_extensions {
            let ext_bson = custom_extensions_to_bson(custom_ext);
            set_doc.insert("custom_extensions", ext_bson);
        }
        if let Some(ref agent_domain) = req.agent_domain {
            set_doc.insert("agent_domain", agent_domain.clone());
        }
        if let Some(ref agent_role) = req.agent_role {
            set_doc.insert("agent_role", agent_role.clone());
        }
        if let Some(ref owner_manager_agent_id) = req.owner_manager_agent_id {
            set_doc.insert("owner_manager_agent_id", owner_manager_agent_id.clone());
        }
        if let Some(ref template_source_agent_id) = req.template_source_agent_id {
            set_doc.insert("template_source_agent_id", template_source_agent_id.clone());
        }
        if let Some(ref allowed_groups) = req.allowed_groups {
            let bson_val =
                mongodb::bson::to_bson(allowed_groups).unwrap_or(bson::Bson::Array(vec![]));
            set_doc.insert("allowed_groups", bson_val);
        }
        if let Some(max_concurrent) = req.max_concurrent_tasks {
            set_doc.insert("max_concurrent_tasks", max_concurrent as i32);
        }
        if let Some(temperature) = req.temperature {
            set_doc.insert("temperature", temperature as f64);
        }
        if let Some(max_tokens) = req.max_tokens {
            set_doc.insert("max_tokens", max_tokens);
        }
        if let Some(context_limit) = req.context_limit {
            set_doc.insert("context_limit", context_limit as i64);
        }
        if let Some(thinking_enabled) = req.thinking_enabled {
            set_doc.insert("thinking_enabled", thinking_enabled);
        }
        if let Some(ref assigned_skills) = req.assigned_skills {
            let skills_bson =
                mongodb::bson::to_bson(assigned_skills).unwrap_or(bson::Bson::Array(vec![]));
            set_doc.insert("assigned_skills", skills_bson);
        }
        if let Some(auto_approve) = req.auto_approve_chat {
            set_doc.insert("auto_approve_chat", auto_approve);
        }

        self.agents()
            .update_one(doc! { "agent_id": id }, doc! { "$set": set_doc }, None)
            .await?;

        self.get_agent(id).await
    }

    pub async fn delete_agent(&self, id: &str) -> Result<bool, mongodb::error::Error> {
        let result = self
            .agents()
            .delete_one(doc! { "agent_id": id }, None)
            .await?;
        Ok(result.deleted_count > 0)
    }

    // Task operations
    pub async fn submit_task(
        &self,
        submitter_id: &str,
        req: SubmitTaskRequest,
    ) -> Result<AgentTask, ServiceError> {
        validate_priority(req.priority)?;

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let (mission_id, run_id, session_id, task_role, task_node_id) =
            extract_task_runtime_binding(&req.content);

        let doc = AgentTaskDoc {
            id: None,
            task_id: id.clone(),
            mission_id,
            run_id,
            session_id,
            task_role,
            task_node_id,
            team_id: req.team_id,
            agent_id: req.agent_id,
            submitter_id: submitter_id.to_string(),
            approver_id: None,
            task_type: req.task_type.to_string(),
            content: req.content,
            status: "pending".to_string(),
            priority: req.priority,
            submitted_at: now,
            approved_at: None,
            started_at: None,
            completed_at: None,
            error_message: None,
        };

        self.tasks().insert_one(&doc, None).await?;
        self.get_task(&id)
            .await?
            .ok_or_else(|| ServiceError::Internal(format!("Task not found after insert: {}", id)))
    }

    pub async fn get_task(&self, id: &str) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let doc = self.tasks().find_one(doc! { "task_id": id }, None).await?;
        Ok(doc.map(|d| d.into()))
    }

    pub async fn list_tasks(
        &self,
        query: ListTasksQuery,
    ) -> Result<PaginatedResponse<AgentTask>, mongodb::error::Error> {
        let clamped_limit = query.limit.min(100);
        let limit = clamped_limit as i64;
        let skip = ((query.page.saturating_sub(1)) * clamped_limit) as u64;

        let mut filter = doc! { "team_id": &query.team_id };
        if let Some(agent_id) = &query.agent_id {
            filter.insert("agent_id", agent_id);
        }
        if let Some(status) = &query.status {
            filter.insert("status", status.to_string());
        }

        let total = self.tasks().count_documents(filter.clone(), None).await?;

        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "priority": -1, "submitted_at": -1 })
            .skip(skip)
            .limit(limit)
            .build();
        let cursor = self.tasks().find(filter, options).await?;

        let docs: Vec<AgentTaskDoc> = cursor.try_collect().await?;
        let items: Vec<AgentTask> = docs.into_iter().map(|d| d.into()).collect();

        Ok(PaginatedResponse::new(
            items,
            total,
            query.page,
            query.limit,
        ))
    }

    pub async fn mission_has_active_executor_task(
        &self,
        mission_id: &str,
        run_id: Option<&str>,
    ) -> Result<bool, mongodb::error::Error> {
        self.tasks()
            .count_documents(task_filter_for_mission_run(mission_id, run_id), None)
            .await
            .map(|count| count > 0)
    }

    pub async fn close_stale_mission_tasks_for_new_run(
        &self,
        mission_id: &str,
        current_run_id: &str,
    ) -> Result<u64, mongodb::error::Error> {
        let now = bson::DateTime::now();
        let result = self
            .tasks()
            .update_many(
                doc! {
                    "status": { "$in": ["pending", "approved", "running"] },
                    "$and": [
                        {
                            "$or": [
                                { "task_role": "mission_worker" },
                                { "content.task_role": "mission_worker" },
                            ]
                        },
                        {
                            "$or": [
                                { "mission_id": mission_id },
                                { "content.mission_id": mission_id },
                            ]
                        }
                    ],
                    "$nor": [
                        { "run_id": current_run_id },
                        { "content.run_id": current_run_id },
                    ],
                },
                doc! { "$set": {
                    "status": "failed",
                    "error_message": format!("Superseded by mission run {}", current_run_id),
                    "completed_at": now,
                }},
                None,
            )
            .await?;
        Ok(result.modified_count)
    }

    pub async fn approve_task(
        &self,
        task_id: &str,
        approver_id: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let now = Utc::now();
        let result = self
            .tasks()
            .update_one(
                doc! { "task_id": task_id, "status": "pending" },
                doc! { "$set": {
                    "status": "approved",
                    "approver_id": approver_id,
                    "approved_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.modified_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    pub async fn reject_task(
        &self,
        task_id: &str,
        approver_id: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let now = Utc::now();
        let result = self
            .tasks()
            .update_one(
                doc! { "task_id": task_id, "status": "pending" },
                doc! { "$set": {
                    "status": "rejected",
                    "approver_id": approver_id,
                    "approved_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.modified_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    pub async fn cancel_task(
        &self,
        task_id: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let now = Utc::now();
        let result = self
            .tasks()
            .update_one(
                doc! {
                    "task_id": task_id,
                    "status": { "$in": ["pending", "approved", "running"] }
                },
                doc! { "$set": {
                    "status": "cancelled",
                    "completed_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.modified_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    /// Mark a task as failed with an error message.
    /// Only updates if current status is running or approved (won't overwrite cancelled/completed).
    pub async fn fail_task(
        &self,
        task_id: &str,
        error: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let now = Utc::now();
        let result = self
            .tasks()
            .update_one(
                doc! {
                    "task_id": task_id,
                    "status": { "$in": ["running", "approved"] }
                },
                doc! { "$set": {
                    "status": "failed",
                    "error_message": error,
                    "completed_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.modified_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    pub async fn get_task_results(
        &self,
        task_id: &str,
    ) -> Result<Vec<TaskResult>, mongodb::error::Error> {
        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "created_at": 1 })
            .build();
        let cursor = self
            .results()
            .find(doc! { "task_id": task_id }, options)
            .await?;

        let docs: Vec<TaskResultDoc> = cursor.try_collect().await?;
        Ok(docs.into_iter().map(|d| d.into()).collect())
    }

    /// Check if a user has access to a specific agent based on group-based access control
    pub async fn check_agent_access(
        &self,
        agent_id: &str,
        _user_id: &str,
        user_group_ids: &[String],
    ) -> Result<bool, mongodb::error::Error> {
        let agent = match self.get_agent(agent_id).await? {
            Some(a) => a,
            None => return Ok(false),
        };

        // Empty allowed_groups = all team members can use
        if agent.allowed_groups.is_empty() {
            return Ok(true);
        }
        // User must be in at least one allowed group
        Ok(user_group_ids
            .iter()
            .any(|gid| agent.allowed_groups.contains(gid)))
    }

    /// Update agent access control settings
    pub async fn update_access_control(
        &self,
        agent_id: &str,
        allowed_groups: Vec<String>,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let now = Utc::now();
        let bson_val = mongodb::bson::to_bson(&allowed_groups).unwrap_or(bson::Bson::Array(vec![]));
        let set = doc! {
            "updated_at": bson::DateTime::from_chrono(now),
            "allowed_groups": bson_val,
        };

        self.agents()
            .update_one(doc! { "agent_id": agent_id }, doc! { "$set": set }, None)
            .await?;

        self.get_agent(agent_id).await
    }

    // ========== Team Extension Bridge ==========

    fn shared_extension_to_custom_extension(
        ext: &agime_team::models::mongo::Extension,
    ) -> Result<CustomExtensionConfig, ServiceError> {
        let uri_or_cmd = ext
            .config
            .get_str("uri_or_cmd")
            .or_else(|_| ext.config.get_str("uriOrCmd"))
            .or_else(|_| ext.config.get_str("command"))
            .unwrap_or_default()
            .to_string();

        if uri_or_cmd.trim().is_empty() {
            return Err(ServiceError::Validation(ValidationError::ExtensionConfig));
        }

        let empty_args = Vec::new();
        let args: Vec<String> = ext
            .config
            .get_array("args")
            .unwrap_or(&empty_args)
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let envs: std::collections::HashMap<String, String> = ext
            .config
            .get_document("envs")
            .map(|doc| {
                doc.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        let ext_id_hex = ext.id.map(|id| id.to_hex()).unwrap_or_default();

        Ok(CustomExtensionConfig {
            name: ext.name.clone(),
            ext_type: ext.extension_type.clone(),
            uri_or_cmd,
            args,
            envs,
            enabled: true,
            source: Some("team".to_string()),
            source_extension_id: Some(ext_id_hex),
        })
    }

    /// Add a team shared extension to an agent's custom_extensions.
    /// Converts the SharedExtension from the extensions collection into a CustomExtensionConfig
    /// and appends it to the agent's custom_extensions array (with deduplication by name).
    pub async fn add_team_extension_to_agent(
        &self,
        agent_id: &str,
        extension_id: &str,
        team_id: &str,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        use agime_team::services::mongo::extension_service_mongo::ExtensionService;

        let ext_service = ExtensionService::new((*self.db).clone());

        // 1. Fetch the shared extension
        let ext = ext_service
            .get(extension_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?
            .ok_or_else(|| ServiceError::Internal("Extension not found".to_string()))?;

        // Verify team ownership
        if ext.team_id.to_hex() != team_id {
            return Err(ServiceError::Internal(
                "Extension does not belong to this team".to_string(),
            ));
        }

        // 2. Get current agent
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;

        // 3. Check for duplicate name
        if agent.custom_extensions.iter().any(|e| e.name == ext.name) {
            return Err(ServiceError::Internal(format!(
                "Extension '{}' already exists in this agent",
                ext.name
            )));
        }

        // 4. Convert SharedExtension -> CustomExtensionConfig
        let new_ext = Self::shared_extension_to_custom_extension(&ext)?;

        // 5. Append to custom_extensions via $push
        let ext_bson = Bson::Document(custom_extension_to_bson_document(&new_ext));

        let now = Utc::now();
        self.agents()
            .update_one(
                doc! { "agent_id": agent_id },
                doc! {
                    "$push": { "custom_extensions": ext_bson },
                    "$set": { "updated_at": bson::DateTime::from_chrono(now) }
                },
                None,
            )
            .await?;

        Ok(self.get_agent(agent_id).await?)
    }

    /// Return team agents that currently have a team-sourced custom extension attached.
    pub async fn list_agents_attached_to_team_extension(
        &self,
        team_id: &str,
        extension_id: &str,
    ) -> Result<Vec<TeamAgent>, ServiceError> {
        let cursor = self
            .agents()
            .find(
                doc! {
                    "team_id": team_id,
                    "is_deleted": { "$ne": true },
                    "custom_extensions.source_extension_id": extension_id,
                },
                None,
            )
            .await
            .map_err(ServiceError::Database)?;
        let docs: Vec<TeamAgentDoc> = cursor.try_collect().await.map_err(ServiceError::Database)?;
        let mut items: Vec<TeamAgent> = docs.into_iter().map(Into::into).collect();
        for agent in &mut items {
            self.backfill_legacy_avatar_agent_metadata(agent)
                .await
                .map_err(ServiceError::Database)?;
        }
        Ok(items)
    }

    /// Sync the current team extension definition into every attached agent copy.
    pub async fn sync_team_extension_to_attached_agents(
        &self,
        team_id: &str,
        extension_id: &str,
    ) -> Result<Vec<TeamAgent>, ServiceError> {
        use agime_team::services::mongo::extension_service_mongo::ExtensionService;

        let ext_service = ExtensionService::new((*self.db).clone());
        let extension = ext_service
            .get(extension_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?
            .ok_or_else(|| ServiceError::Internal("Extension not found".to_string()))?;

        if extension.team_id.to_hex() != team_id {
            return Err(ServiceError::Internal(
                "Extension does not belong to this team".to_string(),
            ));
        }

        let template = Self::shared_extension_to_custom_extension(&extension)?;
        let attached_agents = self
            .list_agents_attached_to_team_extension(team_id, extension_id)
            .await?;
        let mut updated_agents = Vec::new();

        for agent in attached_agents {
            let mut changed = false;
            let mut next_custom_extensions = agent.custom_extensions.clone();
            for existing in &mut next_custom_extensions {
                if !custom_extension_matches_source_extension(existing, extension_id) {
                    continue;
                }
                let was_enabled = existing.enabled;
                *existing = template.clone();
                existing.enabled = was_enabled;
                changed = true;
            }

            if !changed {
                continue;
            }

            if let Some(updated) = self
                .update_agent(
                    &agent.id,
                    UpdateAgentRequest {
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
                        custom_extensions: Some(next_custom_extensions),
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
                    },
                )
                .await
                .map_err(ServiceError::Database)?
            {
                updated_agents.push(updated);
            }
        }

        Ok(updated_agents)
    }

    /// Remove one team extension from every attached agent copy in the same team.
    pub async fn detach_team_extension_from_attached_agents(
        &self,
        team_id: &str,
        extension_id: &str,
    ) -> Result<Vec<TeamAgent>, ServiceError> {
        let attached_agents = self
            .list_agents_attached_to_team_extension(team_id, extension_id)
            .await?;
        let mut updated_agents = Vec::new();

        for agent in attached_agents {
            let mut next_custom_extensions = agent.custom_extensions.clone();
            let original_len = next_custom_extensions.len();
            next_custom_extensions.retain(|existing| {
                !custom_extension_matches_source_extension(existing, extension_id)
            });
            if next_custom_extensions.len() == original_len {
                continue;
            }

            if let Some(updated) = self
                .update_agent(
                    &agent.id,
                    UpdateAgentRequest {
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
                        custom_extensions: Some(next_custom_extensions),
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
                    },
                )
                .await
                .map_err(ServiceError::Database)?
            {
                updated_agents.push(updated);
            }
        }

        Ok(updated_agents)
    }

    /// Add a custom MCP extension directly onto an agent.
    pub async fn add_custom_extension_to_agent(
        &self,
        agent_id: &str,
        extension: CustomExtensionConfig,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;
        let normalized = normalize_custom_extension_config(extension)?;
        if agent
            .custom_extensions
            .iter()
            .any(|existing| custom_extension_name_eq(&existing.name, &normalized.name))
        {
            return Err(ServiceError::Internal(format!(
                "Extension '{}' already exists in this agent",
                normalized.name
            )));
        }

        let mut next_custom_extensions = agent.custom_extensions.clone();
        next_custom_extensions.push(normalized);
        next_custom_extensions.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        });

        self.update_agent(
            agent_id,
            UpdateAgentRequest {
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
                custom_extensions: Some(next_custom_extensions),
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
            },
        )
        .await
        .map_err(ServiceError::Database)
    }

    /// Enable or disable a custom MCP extension on an agent.
    pub async fn set_custom_extension_enabled(
        &self,
        agent_id: &str,
        extension_name: &str,
        enabled: bool,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;
        let mut next_custom_extensions = agent.custom_extensions.clone();
        let Some(existing) = next_custom_extensions
            .iter_mut()
            .find(|existing| custom_extension_name_eq(&existing.name, extension_name))
        else {
            return Err(ServiceError::Internal(format!(
                "Extension '{}' not found in this agent",
                extension_name
            )));
        };

        existing.enabled = enabled;

        self.update_agent(
            agent_id,
            UpdateAgentRequest {
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
                custom_extensions: Some(next_custom_extensions),
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
            },
        )
        .await
        .map_err(ServiceError::Database)
    }

    /// Remove a custom MCP extension from an agent.
    pub async fn remove_custom_extension_from_agent(
        &self,
        agent_id: &str,
        extension_name: &str,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;
        let mut next_custom_extensions = agent.custom_extensions.clone();
        let original_len = next_custom_extensions.len();
        next_custom_extensions
            .retain(|existing| !custom_extension_name_eq(&existing.name, extension_name));
        if next_custom_extensions.len() == original_len {
            return Err(ServiceError::Internal(format!(
                "Extension '{}' not found in this agent",
                extension_name
            )));
        }

        self.update_agent(
            agent_id,
            UpdateAgentRequest {
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
                custom_extensions: Some(next_custom_extensions),
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
            },
        )
        .await
        .map_err(ServiceError::Database)
    }

    // ========== Team Skill Bridge ==========

    /// Add a team shared skill to an agent's assigned_skills.
    pub async fn add_team_skill_to_agent(
        &self,
        agent_id: &str,
        skill_id: &str,
        team_id: &str,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        use agime_team::services::mongo::skill_service_mongo::SkillService;

        let skill_service = SkillService::new((*self.db).clone());

        // 1. Fetch the shared skill
        let skill = skill_service
            .get(skill_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?
            .ok_or_else(|| ServiceError::Internal("Skill not found".to_string()))?;

        // Verify team ownership
        if skill.team_id.to_hex() != team_id {
            return Err(ServiceError::Internal(
                "Skill does not belong to this team".to_string(),
            ));
        }

        // 2. Get current agent
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;

        // 3. Check for duplicate skill_id
        let skill_id_hex = skill.id.map(|id| id.to_hex()).unwrap_or_default();
        if agent
            .assigned_skills
            .iter()
            .any(|s| s.skill_id == skill_id_hex)
        {
            return Err(ServiceError::Internal(format!(
                "Skill '{}' already assigned to this agent",
                skill.name
            )));
        }

        // 4. Build AgentSkillConfig
        let new_skill = AgentSkillConfig {
            skill_id: skill_id_hex,
            name: skill.name,
            description: skill.description.clone(),
            enabled: true,
            version: skill.version.clone(),
        };

        // 5. $push to assigned_skills
        let skill_bson = mongodb::bson::to_bson(&new_skill)
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        let now = Utc::now();
        self.agents()
            .update_one(
                doc! { "agent_id": agent_id },
                doc! {
                    "$push": { "assigned_skills": skill_bson },
                    "$set": { "updated_at": bson::DateTime::from_chrono(now) }
                },
                None,
            )
            .await?;

        Ok(self.get_agent(agent_id).await?)
    }

    /// Remove a skill from an agent's assigned_skills.
    pub async fn remove_skill_from_agent(
        &self,
        agent_id: &str,
        skill_id: &str,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        let now = Utc::now();
        self.agents()
            .update_one(
                doc! { "agent_id": agent_id },
                doc! {
                    "$pull": { "assigned_skills": { "skill_id": skill_id } },
                    "$set": { "updated_at": bson::DateTime::from_chrono(now) }
                },
                None,
            )
            .await?;

        Ok(self.get_agent(agent_id).await?)
    }

    /// List available team skills that are not yet assigned to the agent.
    pub async fn list_available_skills(
        &self,
        agent_id: &str,
        team_id: &str,
    ) -> Result<Vec<serde_json::Value>, ServiceError> {
        use agime_team::services::mongo::skill_service_mongo::SkillService;

        let skill_service = SkillService::new((*self.db).clone());

        // Get agent's currently assigned skill IDs
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;

        let assigned_ids: std::collections::HashSet<String> = agent
            .assigned_skills
            .iter()
            .map(|s| s.skill_id.clone())
            .collect();

        // Get all team skills via list() with large limit
        let result = skill_service
            .list(team_id, Some(1), Some(200), None, None)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        // Filter out already assigned
        let available: Vec<serde_json::Value> = result
            .items
            .into_iter()
            .filter(|s| !assigned_ids.contains(&s.id))
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "name": s.name,
                    "description": s.description,
                    "version": s.version,
                })
            })
            .collect();

        Ok(available)
    }

    // ========== Session Management ==========

    /// Create a new agent session
    pub async fn create_session(
        &self,
        req: CreateSessionRequest,
    ) -> Result<AgentSessionDoc, mongodb::error::Error> {
        let effective_allowed_skill_ids = self
            .resolve_session_allowed_skill_ids(&req.agent_id, req.allowed_skill_ids.clone())
            .await?;
        let session_id = Uuid::new_v4().to_string();
        let now = bson::DateTime::now();
        let session_source =
            Self::normalize_session_source(req.session_source.clone(), req.portal_restricted);
        let hidden_from_chat_list = req.hidden_from_chat_list.unwrap_or_else(|| {
            req.portal_restricted
                || session_source == "mission"
                || session_source == "system"
                || session_source == "portal_coding"
                || session_source == "portal_manager"
        });

        let doc = AgentSessionDoc {
            id: None,
            session_id: session_id.clone(),
            team_id: req.team_id,
            agent_id: req.agent_id,
            user_id: req.user_id,
            name: req.name,
            status: "active".to_string(),
            messages_json: "[]".to_string(),
            message_count: 0,
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            compaction_count: 0,
            disabled_extensions: Vec::new(),
            enabled_extensions: Vec::new(),
            created_at: now,
            updated_at: now,
            // Chat Track fields
            title: None,
            pinned: false,
            last_message_preview: None,
            last_message_at: None,
            is_processing: false,
            last_execution_status: None,
            last_execution_error: None,
            last_execution_finished_at: None,
            attached_document_ids: req.attached_document_ids,
            workspace_path: None,
            extra_instructions: req.extra_instructions,
            allowed_extensions: req.allowed_extensions,
            allowed_skill_ids: effective_allowed_skill_ids,
            retry_config: req.retry_config,
            max_turns: req.max_turns,
            tool_timeout_seconds: req.tool_timeout_seconds,
            max_portal_retry_rounds: req.max_portal_retry_rounds,
            require_final_report: req.require_final_report,
            portal_restricted: req.portal_restricted,
            document_access_mode: req.document_access_mode,
            portal_id: None,
            portal_slug: None,
            visitor_id: None,
            session_source,
            source_mission_id: req.source_mission_id,
            hidden_from_chat_list,
        };

        self.sessions().insert_one(&doc, None).await?;

        Ok(self
            .sessions()
            .find_one(doc! { "session_id": &session_id }, None)
            .await?
            .unwrap_or(doc))
    }

    /// Get a session by session_id
    pub async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AgentSessionDoc>, mongodb::error::Error> {
        self.sessions()
            .find_one(doc! { "session_id": session_id }, None)
            .await
    }

    /// Find an active (non-deleted) session for a given user + agent pair.
    /// Returns the most recently updated session, if any.
    pub async fn find_active_session_by_user(
        &self,
        user_id: &str,
        agent_id: &str,
    ) -> Result<Option<AgentSessionDoc>, mongodb::error::Error> {
        let opts = mongodb::options::FindOneOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .build();
        self.sessions()
            .find_one(
                doc! {
                    "user_id": user_id,
                    "agent_id": agent_id,
                    "status": "active",
                },
                opts,
            )
            .await
    }

    /// Find an active restricted portal session by user + agent + portal.
    /// Returns the most recently updated session, if any.
    pub async fn find_active_portal_session(
        &self,
        user_id: &str,
        agent_id: &str,
        portal_id: &str,
    ) -> Result<Option<AgentSessionDoc>, mongodb::error::Error> {
        let opts = mongodb::options::FindOneOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .build();
        self.sessions()
            .find_one(
                doc! {
                    "user_id": user_id,
                    "agent_id": agent_id,
                    "status": "active",
                    "portal_restricted": true,
                    "portal_id": portal_id,
                },
                opts,
            )
            .await
    }

    /// List portal-scoped sessions for a visitor (M-4: filter by portal_id).
    pub async fn list_portal_sessions(
        &self,
        portal_id: &str,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<AgentSessionDoc>, mongodb::error::Error> {
        use futures::TryStreamExt;
        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .limit(limit)
            .build();
        let cursor = self
            .sessions()
            .find(
                doc! {
                    "user_id": user_id,
                    "portal_id": portal_id,
                    "portal_restricted": true,
                },
                opts,
            )
            .await?;
        cursor.try_collect().await
    }

    /// Update session messages and token stats (called by executor after each loop)
    pub async fn update_session_messages(
        &self,
        session_id: &str,
        messages_json: &str,
        message_count: i32,
        total_tokens: Option<i32>,
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        let mut set = doc! {
            "messages_json": messages_json,
            "message_count": message_count,
            "updated_at": now,
        };
        if let Some(t) = total_tokens {
            set.insert("total_tokens", t);
        }
        if let Some(t) = input_tokens {
            set.insert("input_tokens", t);
        }
        if let Some(t) = output_tokens {
            set.insert("output_tokens", t);
        }

        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set },
                None,
            )
            .await?;
        Ok(())
    }

    /// Increment compaction count.
    pub async fn increment_compaction_count(
        &self,
        session_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$inc": { "compaction_count": 1 },
                    "$set": {
                        "updated_at": now,
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Update session extension overrides (disabled/enabled extensions)
    pub async fn update_session_extensions(
        &self,
        session_id: &str,
        disabled_extensions: &[String],
        enabled_extensions: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$set": {
                        "disabled_extensions": disabled_extensions,
                        "enabled_extensions": enabled_extensions,
                        "updated_at": now,
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// List sessions for an agent
    pub async fn list_sessions(
        &self,
        query: SessionListQuery,
    ) -> Result<Vec<AgentSessionDoc>, mongodb::error::Error> {
        let clamped_limit = query.limit.min(100) as i64;
        let skip = ((query.page.saturating_sub(1)) * query.limit.min(100)) as u64;

        let mut filter = doc! {
            "team_id": &query.team_id,
            "agent_id": &query.agent_id,
        };
        if let Some(ref uid) = query.user_id {
            filter.insert("user_id", uid);
        }

        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .skip(skip)
            .limit(clamped_limit)
            .build();

        let cursor = self.sessions().find(filter, options).await?;
        cursor.try_collect().await
    }

    /// Archive a session
    pub async fn archive_session(&self, session_id: &str) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": {
                    "status": "archived",
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Atomically archive a session only if it is not currently processing.
    pub async fn archive_session_if_idle(
        &self,
        session_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let now = bson::DateTime::now();
        let result = self
            .sessions()
            .update_one(
                doc! { "session_id": session_id, "is_processing": false },
                doc! { "$set": {
                    "status": "archived",
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(result.modified_count > 0)
    }

    // ========== Chat Track Methods (Phase 1) ==========

    /// List user sessions with lightweight items (no messages_json).
    /// Joins agent_name from team_agents collection.
    pub async fn list_user_sessions(
        &self,
        query: UserSessionListQuery,
    ) -> Result<Vec<SessionListItem>, mongodb::error::Error> {
        let clamped_limit = query.limit.min(100) as i64;
        let skip = ((query.page.saturating_sub(1)) * query.limit.min(100)) as u64;

        let mut filter = doc! { "team_id": &query.team_id };
        // C1 fix: Always filter by user_id to prevent data leakage
        if let Some(ref user_id) = query.user_id {
            filter.insert("user_id", user_id);
        }
        if let Some(ref agent_id) = query.agent_id {
            filter.insert("agent_id", agent_id);
        }
        if let Some(ref status) = query.status {
            filter.insert("status", status);
        } else {
            filter.insert("status", "active");
        }
        if !query.include_hidden {
            filter.insert(
                "$or",
                vec![
                    doc! { "hidden_from_chat_list": { "$exists": false } },
                    doc! { "hidden_from_chat_list": false },
                ],
            );
        }

        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "pinned": -1, "last_message_at": -1, "updated_at": -1 })
            .skip(skip)
            .limit(clamped_limit)
            .projection(doc! {
                "session_id": 1, "agent_id": 1, "title": 1,
                "last_message_preview": 1, "last_message_at": 1,
                "message_count": 1, "status": 1, "pinned": 1,
                "created_at": 1, "user_id": 1,
                "team_id": 1, "updated_at": 1,
            })
            .build();

        let cursor = self.sessions().find(filter, options).await?;
        let sessions: Vec<AgentSessionDoc> = cursor.try_collect().await?;

        // Batch-fetch agent names
        let agent_ids: Vec<&str> = sessions.iter().map(|s| s.agent_id.as_str()).collect();
        let agent_names = self.batch_get_agent_names(&agent_ids).await;

        let items = sessions
            .into_iter()
            .map(|s| {
                let agent_name = agent_names
                    .get(&s.agent_id)
                    .cloned()
                    .unwrap_or_else(|| s.agent_id.clone());
                SessionListItem {
                    session_id: s.session_id,
                    agent_id: s.agent_id,
                    agent_name,
                    title: s.title,
                    last_message_preview: s.last_message_preview,
                    last_message_at: s.last_message_at.map(|d| d.to_chrono().to_rfc3339()),
                    message_count: s.message_count,
                    status: s.status,
                    pinned: s.pinned,
                    created_at: s.created_at.to_chrono().to_rfc3339(),
                }
            })
            .collect();

        Ok(items)
    }

    /// Backfill session source/visibility fields for existing data.
    /// Safe to run repeatedly (idempotent best effort).
    pub async fn backfill_session_source_and_visibility(
        &self,
    ) -> Result<(), mongodb::error::Error> {
        // 1) Portal sessions -> portal source + hidden.
        let _ = self
            .sessions()
            .update_many(
                doc! { "portal_restricted": true },
                doc! { "$set": { "session_source": "portal", "hidden_from_chat_list": true } },
                None,
            )
            .await?;

        // 1.5) Portal coding sessions (legacy rows may have been stored as chat)
        // detect by presence of portal + workspace context and mark as hidden.
        let _ = self
            .sessions()
            .update_many(
                doc! {
                    "portal_restricted": { "$ne": true },
                    "portal_id": { "$exists": true, "$ne": bson::Bson::Null },
                    "workspace_path": { "$exists": true, "$ne": bson::Bson::Null },
                },
                doc! { "$set": { "session_source": "portal_coding", "hidden_from_chat_list": true } },
                None,
            )
            .await?;

        // 2) Mission-bound sessions from mission docs -> mission source + hidden + source_mission_id.
        let mission_opts = mongodb::options::FindOptions::builder()
            .projection(doc! { "mission_id": 1, "session_id": 1, "status": 1 })
            .build();
        // Use raw bson documents here to tolerate legacy mission rows that miss
        // strongly-typed fields (e.g. team_id) and keep migration best-effort.
        let missions_raw = self.db.collection::<bson::Document>("agent_missions");
        let mut mission_cursor = missions_raw.find(doc! {}, mission_opts).await?;
        while let Some(m) = mission_cursor.try_next().await? {
            let mission_id = m.get_str("mission_id").ok().map(|s| s.to_string());
            let session_id = m.get_str("session_id").ok().map(|s| s.to_string());
            if let (Some(mission_id), Some(session_id)) = (mission_id, session_id) {
                let mission_status = m.get_str("status").ok().unwrap_or_default();
                let mut set_doc = doc! {
                    "session_source": "mission",
                    "source_mission_id": mission_id,
                    "hidden_from_chat_list": true,
                };
                if matches!(mission_status, "completed" | "cancelled") {
                    set_doc.insert("status", "archived");
                }
                let _ = self
                    .sessions()
                    .update_one(
                        doc! { "session_id": &session_id },
                        doc! { "$set": set_doc },
                        None,
                    )
                    .await?;
            }
        }

        // 3) Remaining sessions with missing source -> default chat + visible.
        let _ = self
            .sessions()
            .update_many(
                doc! { "session_source": { "$exists": false } },
                doc! { "$set": { "session_source": "chat" } },
                None,
            )
            .await?;
        let _ = self
            .sessions()
            .update_many(
                doc! { "hidden_from_chat_list": { "$exists": false } },
                doc! { "$set": { "hidden_from_chat_list": false } },
                None,
            )
            .await?;
        Ok(())
    }

    /// Batch-fetch agent names by IDs
    async fn batch_get_agent_names(
        &self,
        agent_ids: &[&str],
    ) -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();
        if agent_ids.is_empty() {
            return map;
        }
        let unique: std::collections::HashSet<&str> = agent_ids.iter().copied().collect();
        let ids_bson: Vec<bson::Bson> = unique
            .iter()
            .map(|id| bson::Bson::String(id.to_string()))
            .collect();

        let filter = doc! { "agent_id": { "$in": ids_bson } };
        let opts = mongodb::options::FindOptions::builder()
            .projection(doc! { "agent_id": 1, "name": 1 })
            .build();

        if let Ok(cursor) = self.agents().find(filter, opts).await {
            if let Ok(docs) = cursor.try_collect::<Vec<_>>().await {
                for d in docs {
                    map.insert(d.agent_id.clone(), d.name.clone());
                }
            }
        }
        map
    }

    /// Create a chat session for a specific agent, optionally with attached documents
    #[allow(clippy::too_many_arguments)]
    pub async fn create_chat_session(
        &self,
        team_id: &str,
        agent_id: &str,
        user_id: &str,
        attached_document_ids: Vec<String>,
        extra_instructions: Option<String>,
        allowed_extensions: Option<Vec<String>>,
        allowed_skill_ids: Option<Vec<String>>,
        retry_config: Option<RetryConfig>,
        max_turns: Option<i32>,
        tool_timeout_seconds: Option<u64>,
        max_portal_retry_rounds: Option<u32>,
        require_final_report: bool,
        portal_restricted: bool,
        document_access_mode: Option<String>,
        session_source: Option<String>,
        source_mission_id: Option<String>,
        hidden_from_chat_list: Option<bool>,
    ) -> Result<AgentSessionDoc, mongodb::error::Error> {
        self.create_session(CreateSessionRequest {
            team_id: team_id.to_string(),
            agent_id: agent_id.to_string(),
            user_id: user_id.to_string(),
            name: None,
            attached_document_ids,
            extra_instructions,
            allowed_extensions,
            allowed_skill_ids,
            retry_config,
            max_turns,
            tool_timeout_seconds,
            max_portal_retry_rounds,
            require_final_report,
            portal_restricted,
            document_access_mode,
            session_source,
            source_mission_id,
            hidden_from_chat_list,
        })
        .await
    }

    /// Rename a session
    pub async fn rename_session(
        &self,
        session_id: &str,
        title: &str,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": { "title": title, "updated_at": now } },
                None,
            )
            .await?;
        Ok(())
    }

    /// Pin or unpin a session
    pub async fn pin_session(
        &self,
        session_id: &str,
        pinned: bool,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": { "pinned": pinned, "updated_at": now } },
                None,
            )
            .await?;
        Ok(())
    }

    /// Permanently delete a session
    pub async fn delete_session(&self, session_id: &str) -> Result<bool, mongodb::error::Error> {
        let result = self
            .sessions()
            .delete_one(doc! { "session_id": session_id }, None)
            .await?;
        Ok(result.deleted_count > 0)
    }

    /// Atomically delete a session only if it is not currently processing.
    pub async fn delete_session_if_idle(
        &self,
        session_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let result = self
            .sessions()
            .delete_one(
                doc! { "session_id": session_id, "is_processing": false },
                None,
            )
            .await?;
        Ok(result.deleted_count > 0)
    }

    /// Attach documents to a session
    pub async fn attach_documents_to_session(
        &self,
        session_id: &str,
        document_ids: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$addToSet": { "attached_document_ids": { "$each": document_ids } },
                    "$set": { "updated_at": now },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Detach documents from a session
    pub async fn detach_documents_from_session(
        &self,
        session_id: &str,
        document_ids: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$pullAll": { "attached_document_ids": document_ids },
                    "$set": { "updated_at": now },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Set session processing state (prevents concurrent sends)
    pub async fn set_session_processing(
        &self,
        session_id: &str,
        is_processing: bool,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": {
                    "is_processing": is_processing,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Atomically try to set is_processing = true, only if currently false.
    /// Returns Ok(true) if successfully claimed, Ok(false) if already processing.
    /// This prevents TOCTOU race conditions on concurrent send_message calls.
    pub async fn try_start_processing(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let now = bson::DateTime::now();
        // Try normal claim: is_processing == false
        let result = self
            .sessions()
            .find_one_and_update(
                doc! {
                    "session_id": session_id,
                    "user_id": user_id,
                    "is_processing": false,
                },
                doc! { "$set": {
                    "is_processing": true,
                    "last_execution_status": "running",
                    "last_execution_error": bson::Bson::Null,
                    "last_execution_finished_at": bson::Bson::Null,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        if result.is_some() {
            return Ok(true);
        }
        // Auto-recover: if stuck processing > 10 minutes, force claim
        let stale_cutoff =
            bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::minutes(10));
        let recovered = self
            .sessions()
            .find_one_and_update(
                doc! {
                    "session_id": session_id,
                    "user_id": user_id,
                    "is_processing": true,
                    "updated_at": { "$lt": stale_cutoff },
                },
                doc! { "$set": {
                    "is_processing": true,
                    "last_execution_status": "running",
                    "last_execution_error": bson::Bson::Null,
                    "last_execution_finished_at": bson::Bson::Null,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        if recovered.is_some() {
            tracing::warn!("Auto-recovered stuck session {}", session_id);
        }
        Ok(recovered.is_some())
    }

    /// Reset stuck `is_processing` flags for sessions that have been processing
    /// longer than the given timeout. This recovers from crashed/timed-out executions.
    pub async fn reset_stuck_processing(
        &self,
        max_age: std::time::Duration,
    ) -> Result<u64, mongodb::error::Error> {
        let cutoff = bson::DateTime::from_chrono(
            chrono::Utc::now() - chrono::Duration::from_std(max_age).unwrap_or_default(),
        );
        let now = bson::DateTime::now();
        let result = self
            .sessions()
            .update_many(
                doc! {
                    "is_processing": true,
                    "updated_at": { "$lt": cutoff },
                },
                doc! { "$set": {
                    "is_processing": false,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(result.modified_count)
    }

    /// Update session metadata after a message completes
    pub async fn update_session_after_message(
        &self,
        session_id: &str,
        messages_json: &str,
        message_count: i32,
        last_preview: &str,
        title: Option<&str>,
        tokens: Option<i32>,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        // H2 fix: use chars() for safe Unicode slicing
        let preview: String = if last_preview.chars().count() > 200 {
            let truncated: String = last_preview.chars().take(197).collect();
            format!("{}...", truncated)
        } else {
            last_preview.to_string()
        };

        let mut set = doc! {
            "messages_json": messages_json,
            "message_count": message_count,
            "last_message_preview": &preview,
            "last_message_at": now,
            "is_processing": false,
            "updated_at": now,
        };
        if let Some(t) = title {
            set.insert("title", t);
        }
        if let Some(t) = tokens {
            set.insert("total_tokens", t);
        }

        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set },
                None,
            )
            .await?;
        Ok(())
    }

    /// Persist the result of the most recent send_message execution without
    /// changing the session lifecycle status (active/archived).
    pub async fn update_session_execution_result(
        &self,
        session_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        let mut set = doc! {
            "is_processing": false,
            "last_execution_status": status,
            "last_execution_finished_at": now,
            "updated_at": now,
        };
        set.insert(
            "last_execution_error",
            error
                .map(|value| bson::Bson::String(value.to_string()))
                .unwrap_or(bson::Bson::Null),
        );
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn update_session_preview(
        &self,
        session_id: &str,
        preview: &str,
    ) -> Result<(), mongodb::error::Error> {
        let preview = preview.trim();
        if preview.is_empty() {
            return Ok(());
        }
        let limited = if preview.chars().count() > 200 {
            let truncated: String = preview.chars().take(197).collect();
            format!("{}...", truncated)
        } else {
            preview.to_string()
        };
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$set": {
                        "last_message_preview": limited,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    async fn append_session_notice_internal(
        &self,
        session_id: &str,
        text: &str,
        user_visible: bool,
        agent_visible: bool,
    ) -> Result<(), mongodb::error::Error> {
        let notice = text.trim();
        if notice.is_empty() {
            return Ok(());
        }
        let Some(session) = self.get_session(session_id).await? else {
            return Ok(());
        };
        let mut messages: Vec<serde_json::Value> =
            serde_json::from_str(&session.messages_json).unwrap_or_default();
        let duplicate_last = messages.last().and_then(|msg| {
            let role = msg.get("role").and_then(serde_json::Value::as_str)?;
            if role != "assistant" {
                return None;
            }
            let content = msg.get("content")?.as_array()?;
            let first = content.first()?;
            first.get("text").and_then(serde_json::Value::as_str)
        }) == Some(notice);
        if duplicate_last {
            return Ok(());
        }

        messages.push(serde_json::json!({
            "id": null,
            "role": "assistant",
            "created": chrono::Utc::now().timestamp(),
            "content": [
                {
                    "type": "text",
                    "text": notice,
                }
            ],
            "metadata": {
                "userVisible": user_visible,
                "agentVisible": agent_visible,
                "systemGenerated": true,
            }
        }));

        let messages_json =
            serde_json::to_string(&messages).unwrap_or_else(|_| session.messages_json.clone());
        if user_visible {
            let preview = notice.to_string();
            self.update_session_after_message(
                session_id,
                &messages_json,
                messages.len() as i32,
                &preview,
                session.title.as_deref(),
                session.total_tokens,
            )
            .await
        } else {
            self.sessions()
                .update_one(
                    doc! { "session_id": session_id },
                    doc! {
                        "$set": {
                            "messages_json": &messages_json,
                            "updated_at": bson::DateTime::now(),
                        }
                    },
                    None,
                )
                .await?;
            Ok(())
        }
    }

    /// Append a hidden assistant-visible system notice into an existing session.
    /// This is useful when runtime policy changes and the agent must treat old
    /// conversation assumptions as stale without cluttering the user's UI.
    pub async fn append_hidden_session_notice(
        &self,
        session_id: &str,
        text: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.append_session_notice_internal(session_id, text, false, true)
            .await
    }

    /// Append a visible assistant system notice into an existing session so the
    /// user can continue a runtime conversation with awareness of the latest run.
    pub async fn append_visible_session_notice(
        &self,
        session_id: &str,
        text: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.append_session_notice_internal(session_id, text, true, true)
            .await
    }

    /// Persist chat runtime stream events for replay/analysis.
    pub async fn save_chat_stream_events(
        &self,
        items: &[(String, String, u64, StreamEvent)],
    ) -> Result<(), mongodb::error::Error> {
        if items.is_empty() {
            return Ok(());
        }

        let docs: Vec<ChatEventDoc> = items
            .iter()
            .map(|(session_id, run_id, event_id, event)| ChatEventDoc {
                id: None,
                session_id: session_id.clone(),
                run_id: Some(run_id.clone()),
                event_id: (*event_id).try_into().unwrap_or(i64::MAX),
                event_type: event.event_type().to_string(),
                payload: serde_json::to_value(event).unwrap_or_else(|_| serde_json::json!({})),
                created_at: bson::DateTime::now(),
            })
            .collect();

        // Best-effort retry for transient write failures.
        // Use unordered insert so one duplicate key does not abort the whole batch.
        let opts = mongodb::options::InsertManyOptions::builder()
            .ordered(false)
            .build();
        let mut attempt: u8 = 0;
        loop {
            match self
                .chat_events()
                .insert_many(docs.clone(), opts.clone())
                .await
            {
                Ok(_) => return Ok(()),
                Err(e) => {
                    let msg = e.to_string();
                    let duplicate_only = msg.contains("E11000")
                        || msg.to_ascii_lowercase().contains("duplicate key");
                    if duplicate_only {
                        return Ok(());
                    }

                    attempt = attempt.saturating_add(1);
                    if attempt >= 3 {
                        return Err(e);
                    }

                    let backoff_ms = 50_u64 * (attempt as u64);
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                }
            }
        }
    }

    pub async fn list_chat_events(
        &self,
        session_id: &str,
        run_id: Option<&str>,
        after_event_id: Option<u64>,
        before_event_id: Option<u64>,
        limit: u32,
        descending: bool,
    ) -> Result<Vec<ChatEventDoc>, mongodb::error::Error> {
        let clamped_limit = limit.clamp(1, 2000);
        let mut filter = doc! { "session_id": session_id };
        if let Some(run) = run_id {
            filter.insert("run_id", run);
        }
        let mut event_id_filter = Document::new();
        if let Some(after) = after_event_id {
            event_id_filter.insert("$gt", after as i64);
        }
        if let Some(before) = before_event_id {
            event_id_filter.insert("$lt", before as i64);
        }
        if !event_id_filter.is_empty() {
            filter.insert("event_id", event_id_filter);
        }
        let sort_dir = if descending { -1 } else { 1 };

        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "event_id": sort_dir, "created_at": sort_dir })
            .limit(clamped_limit as i64)
            .build();
        let mut events: Vec<ChatEventDoc> = self
            .chat_events()
            .find(filter.clone(), opts.clone())
            .await?
            .try_collect()
            .await?;

        // Handle run switches gracefully:
        // if caller sends a stale `after_event_id` from a previous run,
        // restart from beginning of current run instead of returning empty forever.
        if !descending && before_event_id.is_none() && events.is_empty() {
            if let (Some(run), Some(after)) = (run_id, after_event_id) {
                let max_opts = mongodb::options::FindOneOptions::builder()
                    .sort(doc! { "event_id": -1, "created_at": -1 })
                    .build();
                let max_doc = self
                    .chat_events()
                    .find_one(doc! { "session_id": session_id, "run_id": run }, max_opts)
                    .await?;
                if let Some(latest) = max_doc {
                    if latest.event_id < after as i64 {
                        let restart_filter = doc! { "session_id": session_id, "run_id": run };
                        events = self
                            .chat_events()
                            .find(restart_filter, opts)
                            .await?
                            .try_collect()
                            .await?;
                    }
                }
            }
        }

        Ok(events)
    }

    // ═══════════════════════════════════════════════════════
    // Mission Track (Phase 2) - Collection accessors
    // ═══════════════════════════════════════════════════════

    fn missions(&self) -> mongodb::Collection<MissionDoc> {
        self.db.collection("agent_missions")
    }

    fn artifacts(&self) -> mongodb::Collection<MissionArtifactDoc> {
        self.db.collection("agent_mission_artifacts")
    }

    fn mission_events(&self) -> mongodb::Collection<MissionEventDoc> {
        self.db.collection("agent_mission_events")
    }

    // ─── Mission CRUD ────────────────────────────────────

    pub async fn create_mission(
        &self,
        req: &CreateMissionRequest,
        team_id: &str,
        creator_id: &str,
    ) -> Result<MissionDoc, ServiceError> {
        let Some(selected_agent) = self.get_agent_with_key(&req.agent_id).await? else {
            return Err(ServiceError::Validation(
                ValidationError::MissionAgentSelection,
            ));
        };
        if selected_agent.team_id != team_id || !Self::is_v4_mission_selectable_agent(&selected_agent) {
            return Err(ServiceError::Validation(
                ValidationError::MissionAgentSelection,
            ));
        }
        let now = bson::DateTime::now();
        let approval_policy = req.approval_policy.clone().unwrap_or_default();
        let (execution_mode, execution_profile, launch_policy) = self
            .resolve_v4_creation_strategy(req, &approval_policy)
            .await?;
        let step_timeout_seconds = req
            .step_timeout_seconds
            .filter(|v| *v > 0)
            .map(|v| v.min(7200));
        let step_max_retries = req.step_max_retries.filter(|v| *v > 0).map(|v| v.min(8));
        let mission = MissionDoc {
            id: None,
            mission_id: Uuid::new_v4().to_string(),
            team_id: team_id.to_string(),
            agent_id: req.agent_id.clone(),
            creator_id: creator_id.to_string(),
            goal: req.goal.clone(),
            context: req.context.clone(),
            status: MissionStatus::Draft,
            approval_policy,
            steps: vec![],
            current_step: None,
            session_id: None,
            source_chat_session_id: req.source_chat_session_id.clone(),
            token_budget: req.token_budget.unwrap_or(0),
            total_tokens_used: 0,
            priority: req.priority.unwrap_or(0),
            step_timeout_seconds,
            step_max_retries,
            plan_version: 1,
            execution_mode,
            execution_profile,
            launch_policy,
            harness_version: MissionHarnessVersion::V4,
            goal_tree: None,
            current_goal_id: None,
            total_pivots: 0,
            total_abandoned: 0,
            error_message: None,
            final_summary: None,
            delivery_state: Some(MissionDeliveryState::Working),
            delivery_manifest: Some(MissionDeliveryManifest {
                requirements: Vec::new(),
                requested_deliverables: Vec::new(),
                satisfied_deliverables: Vec::new(),
                missing_core_deliverables: Vec::new(),
                supporting_artifacts: Vec::new(),
                delivery_state: MissionDeliveryState::Working,
                final_outcome_summary: None,
            }),
            progress_memory: None,
            completion_assessment: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
            attached_document_ids: req.attached_document_ids.clone(),
            workspace_path: None,
            current_run_id: None,
            pending_monitor_intervention: None,
            last_applied_monitor_intervention: None,
            latest_worker_state: None,
            active_repair_lane_id: None,
            consecutive_no_tool_count: 0,
            last_blocker_fingerprint: None,
            waiting_external_until: None,
            execution_lease: None,
        };
        self.missions().insert_one(&mission, None).await?;
        Ok(mission)
    }

    async fn resolve_v4_creation_strategy(
        &self,
        req: &CreateMissionRequest,
        approval_policy: &ApprovalPolicy,
    ) -> Result<
        (
            super::mission_mongo::ExecutionMode,
            super::mission_mongo::ExecutionProfile,
            super::mission_mongo::LaunchPolicy,
        ),
        ServiceError,
    > {
        if let Some(selection) = self.resolve_v4_creation_strategy_via_ai(req).await? {
            let execution_mode = selection.execution_mode;
            let execution_profile = selection.execution_profile;
            if matches!(execution_profile, super::mission_mongo::ExecutionProfile::Auto) {
                return Err(ServiceError::Internal(format!(
                    "V4 strategy resolution returned unresolved execution_profile=auto for agent {}",
                    req.agent_id
                )));
            }
            if matches!(
                (&execution_mode, &execution_profile),
                (
                    super::mission_mongo::ExecutionMode::Adaptive,
                    super::mission_mongo::ExecutionProfile::Fast
                )
            ) {
                return Err(ServiceError::Internal(format!(
                    "V4 strategy resolution returned invalid adaptive+fast combination for agent {}",
                    req.agent_id
                )));
            }
            let launch_policy = if *approval_policy != ApprovalPolicy::Auto {
                super::mission_mongo::LaunchPolicy::GuidedCheckpoint
            } else {
                selection.launch_policy
            };
            if matches!(launch_policy, super::mission_mongo::LaunchPolicy::Auto) {
                return Err(ServiceError::Internal(format!(
                    "V4 strategy resolution returned unresolved launch_policy=auto for agent {}",
                    req.agent_id
                )));
            }
            return Ok((execution_mode, execution_profile, launch_policy));
        }
        Err(ServiceError::Internal(format!(
            "V4 strategy resolution failed for mission agent {}. The request was not created because V4 requires AI-owned method selection.",
            req.agent_id
        )))
    }

    async fn resolve_v4_creation_strategy_via_ai(
        &self,
        req: &CreateMissionRequest,
    ) -> Result<Option<V4AiStrategySelection>, ServiceError> {
        let Some(agent) = self.get_agent_with_key(&req.agent_id).await? else {
            return Ok(None);
        };
        if matches!(agent.api_format, ApiFormat::Local) {
            return Err(ServiceError::Internal(format!(
                "V4 strategy resolution failed for agent {} because Local API agents do not support provider-based strategy selection",
                req.agent_id
            )));
        }
        let system = r#"You are the V4 mission strategy resolver.
Choose the method that best fits the task itself.
Return JSON only:
{"execution_mode":"sequential|adaptive","execution_profile":"fast|full","launch_policy":"single_worker|subagent_first|swarm_first|guided_checkpoint|recovery_first","reason":"short"}
Rules:
- adaptive for decomposition, comparison, research bundles, multi-output integration, or any task that benefits from orchestration across multiple goal nodes.
- sequential for narrow linear work where one main execution lane can finish the package directly.
- subagent_first for bounded helper delegation where one specialist worker is enough but multi-worker swarm is unnecessary.
- swarm_first for comparison/research/evaluation bundles or any task that explicitly asks for multiple distinct deliverables that should be gathered, drafted, validated, and assembled in parallel.
- single_worker for small linear packages even if they contain two small files.
- adaptive should use full, not fast.
- human review/checkpoint requests should use guided_checkpoint.
Examples:
- one small CSV/script/README package -> sequential + single_worker
- one main deliverable plus one bounded helper artifact -> sequential or adaptive + subagent_first
- comparison bundle with csv/json/md/html outputs -> adaptive + swarm_first"#;

        let user = serde_json::json!({
            "goal": req.goal,
            "context": req.context,
            "approval_policy": req.approval_policy,
            "attached_document_count": req.attached_document_ids.len(),
        })
        .to_string();

        let raw = if matches!(agent.api_format, ApiFormat::OpenAI) {
            call_openai_strategy_resolution_with_timeout(&agent, system, &user).await
        } else {
            let mut strategy_agent = agent.clone();
            strategy_agent.max_tokens = Some(strategy_agent.max_tokens.unwrap_or(256).min(256));
            strategy_agent.context_limit =
                Some(strategy_agent.context_limit.unwrap_or(32768).min(32768));
            strategy_agent.thinking_enabled = false;
            strategy_agent.temperature = Some(strategy_agent.temperature.unwrap_or(0.0).min(0.2));
            let provider = match provider_factory::create_provider_for_agent(&strategy_agent) {
                Ok(provider) => provider,
                Err(err) => {
                    return Err(ServiceError::Internal(format!(
                        "V4 strategy resolution failed for agent {} because provider initialization failed: {}",
                        req.agent_id, err
                    )));
                }
            };
            call_provider_complete_with_timeout(&provider, system, &user).await
        }
        .map_err(|err| {
                ServiceError::Internal(format!(
                    "V4 strategy resolution failed for agent {} because {}",
                    req.agent_id, err
                ))
            })?;
        let try_parse = |text: &str| {
            extract_json_object_block(text)
                .and_then(|block| serde_json::from_str::<V4AiStrategySelection>(block).ok())
        };
        if let Some(selection) = try_parse(&raw) {
            return Ok(Some(selection));
        }

        let repair_user = format!(
            "Your previous reply was not valid JSON for V4 mission strategy resolution.\nReturn only one JSON object that matches the required schema.\nPrevious reply:\n{}",
            raw
        );
        let repaired_raw = if matches!(agent.api_format, ApiFormat::OpenAI) {
            call_openai_strategy_resolution_with_timeout(&agent, system, &repair_user).await
        } else {
            let mut strategy_agent = agent.clone();
            strategy_agent.max_tokens = Some(strategy_agent.max_tokens.unwrap_or(256).min(256));
            strategy_agent.context_limit =
                Some(strategy_agent.context_limit.unwrap_or(32768).min(32768));
            strategy_agent.thinking_enabled = false;
            strategy_agent.temperature = Some(strategy_agent.temperature.unwrap_or(0.0).min(0.2));
            let provider = match provider_factory::create_provider_for_agent(&strategy_agent) {
                Ok(provider) => provider,
                Err(err) => {
                    return Err(ServiceError::Internal(format!(
                        "V4 strategy resolution failed for agent {} because provider initialization failed: {}",
                        req.agent_id, err
                    )));
                }
            };
            call_provider_complete_with_timeout(&provider, system, &repair_user).await
        }
        .map_err(|err| {
            ServiceError::Internal(format!(
                "V4 strategy resolution repair failed for agent {} because {}",
                req.agent_id, err
            ))
        })?;
        if let Some(selection) = try_parse(&repaired_raw) {
            return Ok(Some(selection));
        }
        Err(ServiceError::Internal(format!(
            "V4 strategy resolution returned invalid JSON for agent {} after repair attempt",
            req.agent_id
        )))
    }

    pub async fn get_mission(
        &self,
        mission_id: &str,
    ) -> Result<Option<MissionDoc>, mongodb::error::Error> {
        self.missions()
            .find_one(doc! { "mission_id": mission_id }, None)
            .await
    }

    pub async fn get_mission_runtime_view(
        &self,
        mission_id: &str,
    ) -> Result<Option<MissionDoc>, mongodb::error::Error> {
        let Some(mut mission) = self.get_mission(mission_id).await? else {
            return Ok(None);
        };
        if mission.workspace_path.is_some()
            && mission.current_run_id.is_some()
            && matches!(
                mission.status,
                MissionStatus::Running
                    | MissionStatus::Planning
                    | MissionStatus::Paused
                    | MissionStatus::Failed
            )
        {
            let _ = runtime::ensure_mission_session(
                self,
                &mission.mission_id,
                &mission,
                None,
                None,
                mission.workspace_path.as_deref(),
            )
            .await;
            if let Some(refreshed) = self.get_mission(mission_id).await? {
                mission = refreshed;
            }
        }
        let mut projected = self.project_v4_mission_overlay(&mission).await?;
        if runtime_projection_ready_for_settle(&projected)
            && self.settle_mission_result_if_ready(mission_id).await?
        {
            if let Some(settled) = self.get_mission(mission_id).await? {
                return self.project_v4_mission_overlay(&settled).await.map(Some);
            }
        }
        if runtime_projection_needs_terminal_sync(self, &projected).await? {
            sync_v4_runtime_projection_from_mission(self, &projected, None).await?;
            projected = self.project_v4_mission_overlay(&projected).await?;
        }
        Ok(Some(projected))
    }

    pub async fn heal_active_v4_runtime_state(&self) -> Result<u64, mongodb::error::Error> {
        let cursor = self
            .missions()
            .find(
                doc! {
                    "harness_version": "v4",
                    "status": { "$in": ["running", "planning", "paused", "failed"] },
                    "current_run_id": { "$type": "string" },
                },
                None,
            )
            .await?;
        let missions: Vec<MissionDoc> = cursor.try_collect().await?;
        let mut healed = 0u64;
        for mission in missions {
            let mission = self.project_v4_mission_overlay(&mission).await?;
            let Some(run_id) = mission.current_run_id.as_deref() else {
                continue;
            };
            let graph_id = format!("mission:{}:{}", mission.mission_id, run_id);
            let has_run_state = self.get_run_state(run_id).await?.is_some();
            let has_task_graph = self.get_task_graph(&graph_id).await?.is_some();
            let has_checkpoint = self
                .run_checkpoints()
                .count_documents(doc! { "run_id": run_id }, None)
                .await?
                > 0;
            if !has_run_state || !has_task_graph || !has_checkpoint {
                sync_v4_runtime_projection_from_mission(self, &mission, None).await?;
                healed = healed.saturating_add(1);
            }
            let projected = self.project_v4_mission_overlay(&mission).await?;
            if runtime_projection_ready_for_settle(&projected)
                && self
                    .settle_mission_result_if_ready(&mission.mission_id)
                    .await?
            {
                healed = healed.saturating_add(1);
            }
        }
        Ok(healed)
    }

    pub async fn heal_terminal_v4_runtime_state(&self) -> Result<u64, mongodb::error::Error> {
        let cursor = self
            .missions()
            .find(
                doc! {
                    "harness_version": "v4",
                    "status": { "$in": ["completed", "failed", "cancelled"] },
                    "current_run_id": { "$type": "string" },
                },
                None,
            )
            .await?;
        let missions: Vec<MissionDoc> = cursor.try_collect().await?;
        let mut healed = 0u64;
        for mission in missions {
            let projected = self.project_v4_mission_overlay(&mission).await?;
            if runtime_projection_needs_terminal_sync(self, &projected).await? {
                sync_v4_runtime_projection_from_mission(self, &projected, None).await?;
                healed = healed.saturating_add(1);
            }
        }
        Ok(healed)
    }

    /// Attach documents to a mission
    pub async fn attach_documents_to_mission(
        &self,
        mission_id: &str,
        document_ids: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$addToSet": { "attached_document_ids": { "$each": document_ids } },
                    "$set": { "updated_at": now },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Detach documents from a mission
    pub async fn detach_documents_from_mission(
        &self,
        mission_id: &str,
        document_ids: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$pullAll": { "attached_document_ids": document_ids },
                    "$set": { "updated_at": now },
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn list_missions(
        &self,
        query: ListMissionsQuery,
    ) -> Result<Vec<MissionListItem>, mongodb::error::Error> {
        let mut filter = doc! { "team_id": &query.team_id };
        if let Some(ref aid) = query.agent_id {
            filter.insert("agent_id", aid);
        }
        if let Some(ref s) = query.status {
            filter.insert("status", s);
        }

        let skip = ((query.page.max(1) - 1) * query.limit) as u64;
        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .skip(skip)
            .limit(query.limit as i64)
            .build();

        let cursor = self.missions().find(filter, opts).await?;
        let missions: Vec<MissionDoc> = cursor.try_collect().await?;

        // Batch-fetch agent names to avoid N+1 queries
        let agent_ids: Vec<&str> = missions.iter().map(|m| m.agent_id.as_str()).collect();
        let agent_names = self.batch_get_agent_names(&agent_ids).await;

        let mut items = Vec::with_capacity(missions.len());
        for mission in missions {
            let m = self
                .get_mission_runtime_view(&mission.mission_id)
                .await?
                .unwrap_or(mission);
            let agent_name = agent_names.get(&m.agent_id).cloned().unwrap_or_default();

            let completed_steps = m
                .steps
                .iter()
                .filter(|s| s.status == StepStatus::Completed)
                .count();

            let goal_count = m
                .goal_tree
                .as_ref()
                .map(|g| {
                    g.iter()
                        .filter(|n| n.status != GoalStatus::Abandoned)
                        .count()
                })
                .unwrap_or(0);
            let completed_goals = m
                .goal_tree
                .as_ref()
                .map(|g| {
                    g.iter()
                        .filter(|n| n.status == GoalStatus::Completed)
                        .count()
                })
                .unwrap_or(0);

            let delivery_state = effective_delivery_state(&m);
            let missing_core_deliverables = m
                .delivery_manifest
                .as_ref()
                .map(|manifest| manifest.missing_core_deliverables.clone())
                .or_else(|| {
                    m.completion_assessment
                        .as_ref()
                        .map(|assessment| assessment.missing_core_deliverables.clone())
                })
                .unwrap_or_default();
            items.push(MissionListItem {
                mission_id: m.mission_id,
                agent_id: m.agent_id,
                agent_name,
                goal: m.goal,
                status: m.status,
                delivery_state,
                missing_core_deliverables,
                progress_memory: m.progress_memory.clone(),
                retry_after: m
                    .waiting_external_until
                    .map(|ts| ts.to_chrono().to_rfc3339()),
                approval_policy: m.approval_policy,
                step_count: m.steps.len(),
                completed_steps,
                current_step: m.current_step,
                total_tokens_used: m.total_tokens_used,
                created_at: m.created_at.to_chrono().to_rfc3339(),
                updated_at: m.updated_at.to_chrono().to_rfc3339(),
                goal_count,
                completed_goals,
                pivots: m.total_pivots,
                attached_doc_count: m.attached_document_ids.len(),
            });
        }
        Ok(items)
    }

    pub async fn delete_mission(&self, mission_id: &str) -> Result<bool, mongodb::error::Error> {
        // Only Draft, Cancelled, or Failed missions can be deleted
        let result = self
            .missions()
            .delete_one(
                doc! {
                    "mission_id": mission_id,
                    "status": { "$in": ["draft", "cancelled", "failed"] }
                },
                None,
            )
            .await?;
        Ok(result.deleted_count > 0)
    }

    // ─── Mission Status Management ───────────────────────

    /// Update mission status with atomic precondition to prevent race conditions.
    /// Returns an error when the transition precondition is not satisfied.
    pub async fn update_mission_status(
        &self,
        mission_id: &str,
        status: &MissionStatus,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        if matches!(status, MissionStatus::Completed) {
            self.refresh_delivery_manifest_from_artifacts(mission_id).await?;
            self.refresh_progress_memory(mission_id).await?;
        }
        let current_mission = self.get_mission(mission_id).await?;
        if matches!(status, MissionStatus::Failed | MissionStatus::Cancelled) {
            self.refresh_delivery_manifest_from_artifacts(mission_id).await?;
            self.refresh_progress_memory(mission_id).await?;
            if self.settle_mission_result_if_ready(mission_id).await? {
                return Ok(());
            }
        }
        if matches!(status, MissionStatus::Completed) {
            if let Some(mission) = current_mission.as_ref() {
                let manifest_missing = mission
                    .delivery_manifest
                    .as_ref()
                    .map(|manifest| {
                        normalize_concrete_deliverable_paths(&manifest.missing_core_deliverables)
                    })
                    .unwrap_or_default();
                let allow_terminal_with_gaps = mission
                    .completion_assessment
                    .as_ref()
                    .is_some_and(|assessment| {
                        matches!(
                            assessment.disposition,
                            MissionCompletionDisposition::CompletedWithMinorGaps
                                | MissionCompletionDisposition::PartialHandoff
                                | MissionCompletionDisposition::BlockedByEnvironment
                                | MissionCompletionDisposition::BlockedByTooling
                                | MissionCompletionDisposition::WaitingExternal
                                | MissionCompletionDisposition::BlockedFail
                        )
                    });
                if !manifest_missing.is_empty() && !allow_terminal_with_gaps {
                    return Err(mongodb::error::Error::custom(format!(
                        "mission cannot transition to completed while required deliverables are still missing: {}",
                        manifest_missing.join(", ")
                    )));
                }
            }
        }
        let should_set_started_at = if matches!(status, MissionStatus::Running) {
            current_mission
                .as_ref()
                .map(|m| m.started_at.is_none())
                .unwrap_or(true)
        } else {
            false
        };
        let status_bson = bson::to_bson(status)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        let mut set = doc! {
            "status": status_bson,
            "updated_at": now,
        };
        // Set timestamps for terminal states
        match status {
            MissionStatus::Running | MissionStatus::Planning => {
                // Clear terminal timestamp when mission re-enters active states.
                set.insert("completed_at", bson::Bson::Null);
                set.insert("completion_assessment", bson::Bson::Null);
                if should_set_started_at {
                    set.insert("started_at", now);
                }
                // Stamp server instance for orphaned mission recovery
                if let Ok(iid) = std::env::var("TEAM_SERVER_INSTANCE_ID") {
                    set.insert("server_instance_id", iid);
                }
            }
            MissionStatus::Completed | MissionStatus::Failed | MissionStatus::Cancelled => {
                set.insert("completed_at", now);
                set.insert("pending_monitor_intervention", bson::Bson::Null);
                set.insert("current_goal_id", bson::Bson::Null);
                set.insert("current_step", bson::Bson::Null);
                set.insert("active_repair_lane_id", bson::Bson::Null);
                set.insert("consecutive_no_tool_count", 0);
                set.insert("last_blocker_fingerprint", bson::Bson::Null);
                set.insert("waiting_external_until", bson::Bson::Null);
                set.insert("execution_lease", bson::Bson::Null);
                if let Some(mission) = current_mission.as_ref() {
                    if let Some(assessment) = mission.completion_assessment.as_ref() {
                        let terminal_delivery_state = delivery_state_from_assessment(assessment);
                        set.insert(
                            "delivery_state",
                            bson::to_bson(&terminal_delivery_state).map_err(|e| {
                                mongodb::error::Error::custom(format!(
                                    "BSON serialize error: {}",
                                    e
                                ))
                            })?,
                        );
                        set.insert(
                            "delivery_manifest.delivery_state",
                            bson::to_bson(&terminal_delivery_state).map_err(|e| {
                                mongodb::error::Error::custom(format!(
                                    "BSON serialize error: {}",
                                    e
                                ))
                            })?,
                        );
                    }
                }
            }
            _ => {}
        }

        // Atomic precondition: only allow valid state transitions
        let allowed_from: Vec<&str> = match status {
            MissionStatus::Planning => vec!["draft", "planned", "paused", "failed"],
            MissionStatus::Planned => vec!["planning", "paused", "failed"],
            MissionStatus::Running => vec!["draft", "planned", "planning", "paused", "failed"],
            MissionStatus::Paused => vec!["running", "planning"],
            MissionStatus::Completed => vec!["running"],
            MissionStatus::Failed => vec!["running", "planning", "paused", "planned"],
            MissionStatus::Cancelled => vec!["draft", "planned", "running", "paused", "planning"],
            _ => vec![],
        };

        let filter = if allowed_from.is_empty() {
            doc! { "mission_id": mission_id }
        } else {
            let bson_arr: Vec<bson::Bson> = allowed_from
                .iter()
                .map(|s| bson::Bson::String(s.to_string()))
                .collect();
            doc! { "mission_id": mission_id, "status": { "$in": bson_arr } }
        };

        let result = self
            .missions()
            .update_one(filter, doc! { "$set": set }, None)
            .await?;

        if result.modified_count == 0 {
            if let Some(current) = self.get_mission(mission_id).await? {
                if current.status == *status {
                    // Idempotent transition request; treat as success.
                    if matches!(
                        status,
                        MissionStatus::Completed | MissionStatus::Failed | MissionStatus::Cancelled
                    ) {
                        self.refresh_delivery_manifest_from_artifacts(mission_id).await?;
                        self.refresh_progress_memory(mission_id).await?;
                        let _ = self.settle_mission_result_if_ready(mission_id).await?;
                    }
                    return Ok(());
                }
                return Err(mongodb::error::Error::custom(format!(
                    "mission status transition rejected: mission_id={}, from={:?}, to={:?}",
                    mission_id, current.status, status
                )));
            }
            tracing::warn!(
                "update_mission_status: no update for mission {} to {:?} (precondition failed)",
                mission_id,
                status
            );
            return Err(mongodb::error::Error::custom(format!(
                "mission not found or transition rejected: mission_id={}, to={:?}",
                mission_id, status
            )));
        }
        if matches!(
            status,
            MissionStatus::Completed | MissionStatus::Failed | MissionStatus::Cancelled
        ) {
            self.refresh_delivery_manifest_from_artifacts(mission_id).await?;
            self.refresh_progress_memory(mission_id).await?;
            if matches!(status, MissionStatus::Failed | MissionStatus::Cancelled)
                && self.settle_mission_result_if_ready(mission_id).await?
            {
                return Ok(());
            }
        }
        if let Some(refreshed) = self.get_mission(mission_id).await? {
            let run_status = match status {
                MissionStatus::Planning => Some(RunStatus::Planning),
                MissionStatus::Running => Some(RunStatus::Executing),
                MissionStatus::Paused => Some(RunStatus::Paused),
                MissionStatus::Completed => Some(RunStatus::Completed),
                MissionStatus::Failed => Some(RunStatus::Failed),
                MissionStatus::Cancelled => Some(RunStatus::Cancelled),
                _ => None,
            };
            let _ = sync_v4_runtime_projection_from_mission(self, &refreshed, run_status).await;
        }
        Ok(())
    }

    pub async fn set_mission_completion_assessment(
        &self,
        mission_id: &str,
        assessment: &MissionCompletionAssessment,
    ) -> Result<(), mongodb::error::Error> {
        let assessment_bson = bson::to_bson(assessment)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        let mut set_doc = doc! {
            "completion_assessment": assessment_bson,
            "delivery_state": bson::to_bson(&delivery_state_from_assessment(assessment))
                .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
            "delivery_manifest.delivery_state": bson::to_bson(&delivery_state_from_assessment(assessment))
                .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
            "delivery_manifest.final_outcome_summary": assessment.reason.clone(),
            "delivery_manifest.missing_core_deliverables": bson::to_bson(&assessment.missing_core_deliverables)
                .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
            "updated_at": bson::DateTime::now(),
        };
        if assessment_implies_terminal_progress(assessment) {
            let terminal_memory = MissionProgressMemory {
                done: normalize_concrete_deliverable_paths(&assessment.observed_evidence),
                missing: Vec::new(),
                blocked_by: None,
                last_failed_attempt: None,
                next_best_action: None,
                confidence: None,
                updated_at: Some(bson::DateTime::now()),
            };
            set_doc.insert(
                "progress_memory",
                bson::to_bson(&terminal_memory).map_err(|e| {
                    mongodb::error::Error::custom(format!("BSON serialize error: {}", e))
                })?,
            );
        }
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        self.refresh_delivery_manifest_from_artifacts(mission_id).await?;
        self.refresh_progress_memory(mission_id).await?;
        if !assessment_implies_terminal_progress(assessment) {
            let _ = self.settle_mission_result_if_ready(mission_id).await?;
        }
        Ok(())
    }

    pub async fn clear_mission_completion_assessment(
        &self,
        mission_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "completion_assessment": bson::Bson::Null,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        self.refresh_progress_memory(mission_id).await?;
        Ok(())
    }

    pub async fn set_mission_session(
        &self,
        mission_id: &str,
        session_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "session_id": session_id,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_mission_current_run(
        &self,
        mission_id: &str,
        run_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "current_run_id": run_id,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        if let Err(err) = self
            .close_stale_mission_tasks_for_new_run(mission_id, run_id)
            .await
        {
            tracing::warn!(
                "Failed to close stale mission tasks for mission {} run {}: {}",
                mission_id,
                run_id,
                err
            );
        }
        self.refresh_execution_lease(mission_id, "monitor_resume").await?;
        if let Some(refreshed) = self.get_mission(mission_id).await? {
            let _ =
                sync_v4_runtime_projection_from_mission(self, &refreshed, Some(RunStatus::Executing))
                    .await;
        }
        Ok(())
    }

    pub async fn set_pending_monitor_intervention(
        &self,
        mission_id: &str,
        intervention: &MissionMonitorIntervention,
    ) -> Result<(), mongodb::error::Error> {
        let intervention_bson = bson::to_bson(intervention)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        let set_doc = doc! {
            "pending_monitor_intervention": intervention_bson,
            "updated_at": bson::DateTime::now(),
        };
        let mut unset_doc = Document::new();
        if intervention.action.trim().eq_ignore_ascii_case("mark_waiting_external") {
            unset_doc.insert("error_message", bson::Bson::Null);
        }
        let mut update_doc = doc! { "$set": set_doc };
        if !unset_doc.is_empty() {
            update_doc.insert("$unset", unset_doc);
        }
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                update_doc,
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_latest_worker_state(
        &self,
        mission_id: &str,
        worker_state: Option<&WorkerCompactState>,
    ) -> Result<(), mongodb::error::Error> {
        let previous_mission = self.get_mission(mission_id).await?;
        let previous_state = previous_mission
            .as_ref()
            .and_then(|mission| mission.latest_worker_state.as_ref());
        let should_refresh_lease =
            worker_state_has_material_progress(previous_state, worker_state);
        let worker_state = match (previous_state, worker_state) {
            (Some(previous), Some(next)) if worker_state_semantically_equal(previous, next) => {
                let mut preserved = next.clone();
                preserved.recorded_at = previous.recorded_at;
                Some(preserved)
            }
            (_, Some(next)) => Some(next.clone()),
            (_, None) => None,
        };
        let worker_state_bson = match worker_state {
            Some(worker_state) => bson::to_bson(&worker_state).map_err(|e| {
                mongodb::error::Error::custom(format!("BSON serialize error: {}", e))
            })?,
            None => bson::Bson::Null,
        };
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "latest_worker_state": worker_state_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        if should_refresh_lease {
            self.refresh_execution_lease(mission_id, "worker").await?;
        }
        self.refresh_progress_memory(mission_id).await?;
        Ok(())
    }

    pub async fn refresh_execution_lease(
        &self,
        mission_id: &str,
        holder_kind: &str,
    ) -> Result<(), mongodb::error::Error> {
        let Some(mission) = self.get_mission(mission_id).await? else {
            return Ok(());
        };
        let now = bson::DateTime::now();
        let expires_at = bson::DateTime::from_millis(
            now.timestamp_millis() + execution_lease_ttl_secs() * 1000,
        );
        let lease = MissionExecutionLease {
            holder_kind: Some(holder_kind.to_string()),
            run_id: mission.current_run_id.clone(),
            session_id: mission.session_id.clone(),
            last_heartbeat_at: Some(now),
            expires_at: Some(expires_at),
        };
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "execution_lease": bson::to_bson(&lease)
                        .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn refresh_progress_memory(
        &self,
        mission_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let Some(mission) = self.get_mission(mission_id).await? else {
            return Ok(());
        };
        let derived_memory = build_progress_memory(&mission);
        let persisted_memory = match (&mission.progress_memory, derived_memory) {
            (Some(existing), Some(mut derived))
                if progress_memory_semantically_equal(existing, &derived) =>
            {
                derived.updated_at = existing.updated_at;
                Some(derived)
            }
            (_, some) => some,
        };
        if matches!(
            (&mission.progress_memory, &persisted_memory),
            (None, None)
        ) {
            return Ok(());
        }
        if let (Some(existing), Some(current)) = (&mission.progress_memory, &persisted_memory) {
            if progress_memory_semantically_equal(existing, current)
                && existing.updated_at == current.updated_at
            {
                return Ok(());
            }
        }
        let memory_bson = match persisted_memory {
            Some(memory) => bson::to_bson(&memory).map_err(|e| {
                mongodb::error::Error::custom(format!("BSON serialize error: {}", e))
            })?,
            None => bson::Bson::Null,
        };
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": { "progress_memory": memory_bson }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn settle_mission_result_if_ready(
        &self,
        mission_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let Some(mission) = self.get_mission(mission_id).await? else {
            return Ok(false);
        };
        let mission = if mission.harness_version == MissionHarnessVersion::V4 {
            self.project_v4_mission_overlay(&mission).await?
        } else {
            mission
        };
        finalize_inactive_semantic_completion_if_ready(self, &mission).await
    }

    pub async fn patch_mission_convergence_state(
        &self,
        mission_id: &str,
        patch: &MissionConvergencePatch,
    ) -> Result<(), mongodb::error::Error> {
        let mut set_doc = doc! {
            "updated_at": bson::DateTime::now(),
        };

        if let Some(value) = &patch.active_repair_lane_id {
            set_doc.insert(
                "active_repair_lane_id",
                match value {
                    Some(value) => bson::Bson::String(value.clone()),
                    None => bson::Bson::Null,
                },
            );
        }
        if let Some(value) = patch.consecutive_no_tool_count {
            set_doc.insert(
                "consecutive_no_tool_count",
                bson::Bson::Int64(i64::from(value)),
            );
        }
        if let Some(value) = &patch.last_blocker_fingerprint {
            set_doc.insert(
                "last_blocker_fingerprint",
                match value {
                    Some(value) => bson::Bson::String(value.clone()),
                    None => bson::Bson::Null,
                },
            );
        }
        if let Some(value) = &patch.waiting_external_until {
            set_doc.insert(
                "waiting_external_until",
                match value {
                    Some(value) => bson::to_bson(value).map_err(|e| {
                        mongodb::error::Error::custom(format!("BSON serialize error: {}", e))
                    })?,
                    None => bson::Bson::Null,
                },
            );
        }

        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_mission_token_budget(
        &self,
        mission_id: &str,
        token_budget: i64,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "token_budget": token_budget,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn mark_pending_monitor_intervention_applied(
        &self,
        mission_id: &str,
        intervention: &MissionMonitorIntervention,
    ) -> Result<bool, mongodb::error::Error> {
        let Some(requested_at) = intervention.requested_at else {
            return Ok(false);
        };
        let mut applied = intervention.clone();
        applied.applied_at = Some(bson::DateTime::now());
        let applied_bson = bson::to_bson(&applied)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        let result = self
            .missions()
            .update_one(
                doc! {
                    "mission_id": mission_id,
                    "pending_monitor_intervention.requested_at": requested_at,
                    "pending_monitor_intervention.action": &intervention.action,
                },
                doc! { "$set": {
                    "last_applied_monitor_intervention": applied_bson,
                    "pending_monitor_intervention": bson::Bson::Null,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(result.modified_count > 0)
    }

    pub async fn record_monitor_intervention_applied(
        &self,
        mission_id: &str,
        intervention: &MissionMonitorIntervention,
    ) -> Result<(), mongodb::error::Error> {
        let mut applied = intervention.clone();
        applied.applied_at = Some(bson::DateTime::now());
        let applied_bson = bson::to_bson(&applied)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "last_applied_monitor_intervention": applied_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_mission_workspace(
        &self,
        mission_id: &str,
        path: &str,
    ) -> Result<(), mongodb::error::Error> {
        let normalized = normalize_workspace_path(path);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "workspace_path": normalized,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_session_workspace(
        &self,
        session_id: &str,
        path: &str,
    ) -> Result<(), mongodb::error::Error> {
        let normalized = normalize_workspace_path(path);
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": {
                    "workspace_path": normalized,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Bind an existing session to a portal context.
    pub async fn set_session_portal_context(
        &self,
        session_id: &str,
        portal_id: &str,
        portal_slug: &str,
        visitor_id: Option<&str>,
        document_access_mode: Option<&str>,
        portal_restricted: bool,
    ) -> Result<(), mongodb::error::Error> {
        let mut set_doc = doc! {
            "portal_restricted": portal_restricted,
            "portal_id": portal_id,
            "portal_slug": portal_slug,
            "updated_at": bson::DateTime::now(),
        };
        match document_access_mode {
            Some(mode) if !mode.trim().is_empty() => {
                set_doc.insert("document_access_mode", mode.trim());
            }
            _ => {
                set_doc.insert("document_access_mode", bson::Bson::Null);
            }
        }
        match visitor_id {
            Some(v) if !v.trim().is_empty() => {
                set_doc.insert("visitor_id", v.trim());
            }
            _ => {
                set_doc.insert("visitor_id", bson::Bson::Null);
            }
        }
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    /// Sync portal runtime policy for an existing session so portal config changes
    /// (documents, allowlists, prompt constraints) take effect immediately.
    #[allow(clippy::too_many_arguments)]
    pub async fn sync_portal_session_policy(
        &self,
        session_id: &str,
        attached_document_ids: Vec<String>,
        extra_instructions: Option<String>,
        allowed_extensions: Option<Vec<String>>,
        allowed_skill_ids: Option<Vec<String>>,
        retry_config: Option<RetryConfig>,
        require_final_report: bool,
        document_access_mode: Option<String>,
    ) -> Result<(), mongodb::error::Error> {
        let mut set_doc = doc! {
            "attached_document_ids": attached_document_ids,
            "portal_restricted": true,
            "require_final_report": require_final_report,
            "updated_at": bson::DateTime::now(),
        };
        match document_access_mode {
            Some(v) if !v.trim().is_empty() => {
                set_doc.insert("document_access_mode", v);
            }
            _ => {
                set_doc.insert("document_access_mode", bson::Bson::Null);
            }
        }

        match extra_instructions {
            Some(v) if !v.trim().is_empty() => {
                set_doc.insert("extra_instructions", v);
            }
            _ => {
                set_doc.insert("extra_instructions", bson::Bson::Null);
            }
        }
        match allowed_extensions {
            Some(v) => {
                set_doc.insert(
                    "allowed_extensions",
                    mongodb::bson::to_bson(&v).unwrap_or(bson::Bson::Array(vec![])),
                );
            }
            None => {
                set_doc.insert("allowed_extensions", bson::Bson::Null);
            }
        }
        match allowed_skill_ids {
            Some(v) => {
                set_doc.insert(
                    "allowed_skill_ids",
                    mongodb::bson::to_bson(&v).unwrap_or(bson::Bson::Array(vec![])),
                );
            }
            None => {
                set_doc.insert("allowed_skill_ids", bson::Bson::Null);
            }
        }
        match retry_config {
            Some(v) => {
                set_doc.insert(
                    "retry_config",
                    mongodb::bson::to_bson(&v).unwrap_or(bson::Bson::Null),
                );
            }
            None => {
                set_doc.insert("retry_config", bson::Bson::Null);
            }
        }

        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn save_mission_plan(
        &self,
        mission_id: &str,
        steps: Vec<MissionStep>,
    ) -> Result<(), mongodb::error::Error> {
        let steps_bson = bson::to_bson(&steps).unwrap_or_default();
        let explicit_requested = self
            .get_mission(mission_id)
            .await?
            .map(|mission| collect_explicit_deliverables_from_goal_text(&mission.goal))
            .unwrap_or_default();
        let requested_deliverables = if explicit_requested.is_empty() {
            collect_requested_deliverables_from_steps(&steps)
        } else {
            explicit_requested
        };
        let requested_bson = bson::to_bson(&requested_deliverables).unwrap_or_default();
        let requirements = requested_deliverables
            .iter()
            .enumerate()
            .map(|(index, path)| MissionDeliverableRequirement {
                id: format!("req-{}", index + 1),
                label: path.clone(),
                paths: vec![path.clone()],
                mode: MissionDeliverableRequirementMode::AllOf,
                required_when: MissionDeliverableRequirementWhen::Always,
            })
            .collect::<Vec<_>>();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "steps": steps_bson,
                    "delivery_state": bson::to_bson(&MissionDeliveryState::Working)
                        .unwrap_or(bson::Bson::Null),
                    "delivery_manifest.requirements": bson::to_bson(&requirements)
                        .unwrap_or(bson::Bson::Array(Vec::new())),
                    "delivery_manifest.requested_deliverables": requested_bson,
                    "delivery_manifest.missing_core_deliverables": bson::to_bson(&Vec::<String>::new())
                        .unwrap_or(bson::Bson::Array(Vec::new())),
                    "delivery_manifest.delivery_state": bson::to_bson(&MissionDeliveryState::Working)
                        .unwrap_or(bson::Bson::Null),
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        self.refresh_progress_memory(mission_id).await?;
        Ok(())
    }

    pub async fn set_mission_error(
        &self,
        mission_id: &str,
        error: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "error_message": error,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn clear_mission_error(&self, mission_id: &str) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "error_message": bson::Bson::Null,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_mission_final_summary(
        &self,
        mission_id: &str,
        summary: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "final_summary": summary,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn add_mission_tokens(
        &self,
        mission_id: &str,
        tokens_used: i32,
    ) -> Result<(), mongodb::error::Error> {
        if tokens_used <= 0 {
            return Ok(());
        }
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$inc": { "total_tokens_used": tokens_used as i64 },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    // ─── Step Management ─────────────────────────────────

    pub async fn update_step_status(
        &self,
        mission_id: &str,
        step_index: u32,
        status: &StepStatus,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.status", step_index);
        let status_bson = bson::to_bson(status)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        let now = bson::DateTime::now();
        let mut set_doc = doc! {
            &field: status_bson,
            "updated_at": now,
        };
        if matches!(status, StepStatus::Running) {
            let started_field = format!("steps.{}.started_at", step_index);
            set_doc.insert(started_field, now);
        }
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_step_supervision(
        &self,
        mission_id: &str,
        step_index: u32,
        state: StepSupervisorState,
        last_activity_at: Option<bson::DateTime>,
        last_progress_at: Option<bson::DateTime>,
        progress_score: Option<i32>,
        current_blocker: Option<&str>,
        last_supervisor_hint: Option<&str>,
        increment_stall_count: bool,
        stall_count_override: Option<u32>,
    ) -> Result<(), mongodb::error::Error> {
        let state_field = format!("steps.{}.supervisor_state", step_index);
        let last_activity_field = format!("steps.{}.last_activity_at", step_index);
        let last_progress_field = format!("steps.{}.last_progress_at", step_index);
        let progress_score_field = format!("steps.{}.progress_score", step_index);
        let current_blocker_field = format!("steps.{}.current_blocker", step_index);
        let last_supervisor_hint_field = format!("steps.{}.last_supervisor_hint", step_index);
        let stall_count_field = format!("steps.{}.stall_count", step_index);
        let state_bson = bson::to_bson(&state)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        let now = bson::DateTime::now();
        let mut set_doc = doc! {
            &state_field: state_bson,
            "updated_at": now,
        };
        match last_activity_at {
            Some(ts) => {
                set_doc.insert(&last_activity_field, ts);
            }
            None => {
                set_doc.insert(&last_activity_field, Bson::Null);
            }
        }
        match last_progress_at {
            Some(ts) => {
                set_doc.insert(&last_progress_field, ts);
            }
            None => {
                set_doc.insert(&last_progress_field, Bson::Null);
            }
        }
        match progress_score {
            Some(score) => {
                set_doc.insert(&progress_score_field, score);
            }
            None => {
                set_doc.insert(&progress_score_field, Bson::Null);
            }
        }
        match current_blocker {
            Some(blocker) if !blocker.trim().is_empty() => {
                set_doc.insert(&current_blocker_field, blocker);
            }
            _ => {
                set_doc.insert(&current_blocker_field, Bson::Null);
            }
        }
        match last_supervisor_hint {
            Some(hint) if !hint.trim().is_empty() => {
                set_doc.insert(&last_supervisor_hint_field, hint);
            }
            _ => {
                set_doc.insert(&last_supervisor_hint_field, Bson::Null);
            }
        }
        if let Some(stall_count) = stall_count_override {
            set_doc.insert(&stall_count_field, stall_count as i64);
        }

        let mut update_doc = doc! { "$set": set_doc };
        if increment_stall_count && stall_count_override.is_none() {
            update_doc.insert("$inc", doc! { &stall_count_field: 1 });
        }

        self.missions()
            .update_one(doc! { "mission_id": mission_id }, update_doc, None)
            .await?;
        Ok(())
    }

    pub async fn touch_step_activity(
        &self,
        mission_id: &str,
        step_index: u32,
    ) -> Result<(), mongodb::error::Error> {
        let last_activity_field = format!("steps.{}.last_activity_at", step_index);
        let now = bson::DateTime::now();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &last_activity_field: now,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn complete_step(
        &self,
        mission_id: &str,
        step_index: u32,
        tokens_used: i32,
    ) -> Result<(), mongodb::error::Error> {
        let status_field = format!("steps.{}.status", step_index);
        let completed_field = format!("steps.{}.completed_at", step_index);
        let tokens_field = format!("steps.{}.tokens_used", step_index);
        let now = bson::DateTime::now();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        &status_field: "completed",
                        &completed_field: now,
                        &tokens_field: tokens_used,
                        "updated_at": now,
                    },
                    "$inc": { "total_tokens_used": tokens_used as i64 },
                },
                None,
            )
            .await?;
        self.refresh_progress_memory(mission_id).await?;
        Ok(())
    }

    pub async fn fail_step(
        &self,
        mission_id: &str,
        step_index: u32,
        error: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_field = format!("steps.{}.status", step_index);
        let error_field = format!("steps.{}.error_message", step_index);
        let completed_field = format!("steps.{}.completed_at", step_index);
        let now = bson::DateTime::now();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &status_field: "failed",
                    &error_field: error,
                    &completed_field: now,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Save the structured output summary for a completed step.
    pub async fn set_step_output_summary(
        &self,
        mission_id: &str,
        step_index: u32,
        summary: &str,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.output_summary", step_index);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &field: summary,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Persist step runtime contract extracted from mission_preflight.
    pub async fn set_step_runtime_contract(
        &self,
        mission_id: &str,
        step_index: u32,
        contract: &RuntimeContract,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.runtime_contract", step_index);
        let contract_bson = bson::to_bson(contract)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &field: contract_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Persist step contract verification result.
    pub async fn set_step_contract_verification(
        &self,
        mission_id: &str,
        step_index: u32,
        verification: &RuntimeContractVerification,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.contract_verification", step_index);
        let verify_bson = bson::to_bson(verification)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &field: verify_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_step_tool_calls(
        &self,
        mission_id: &str,
        step_index: u32,
        tool_calls: &[super::mission_mongo::ToolCallRecord],
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.tool_calls", step_index);
        let bson_arr = bson::to_bson(tool_calls).unwrap_or(bson::Bson::Array(vec![]));
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": { &field: bson_arr } },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_step_observability(
        &self,
        mission_id: &str,
        step_index: u32,
        recent_progress_events: &[super::mission_mongo::StepProgressEvent],
        evidence_bundle: Option<&super::mission_mongo::StepEvidenceBundle>,
    ) -> Result<(), mongodb::error::Error> {
        let events_field = format!("steps.{}.recent_progress_events", step_index);
        let bundle_field = format!("steps.{}.evidence_bundle", step_index);
        let events_bson =
            bson::to_bson(recent_progress_events).unwrap_or(bson::Bson::Array(vec![]));
        let bundle_bson = match evidence_bundle {
            Some(bundle) => bson::to_bson(bundle).unwrap_or(bson::Bson::Null),
            None => bson::Bson::Null,
        };
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &events_field: events_bson,
                    &bundle_field: bundle_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Increment the retry count for a step.
    pub async fn increment_step_retry(
        &self,
        mission_id: &str,
        step_index: u32,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.retry_count", step_index);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$inc": { &field: 1 },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Reset a step to pending so mission resume can retry from this step.
    pub async fn reset_step_for_retry(
        &self,
        mission_id: &str,
        step_index: u32,
    ) -> Result<(), mongodb::error::Error> {
        let status_field = format!("steps.{}.status", step_index);
        let error_field = format!("steps.{}.error_message", step_index);
        let started_field = format!("steps.{}.started_at", step_index);
        let completed_field = format!("steps.{}.completed_at", step_index);
        let summary_field = format!("steps.{}.output_summary", step_index);
        let tool_calls_field = format!("steps.{}.tool_calls", step_index);
        let supervisor_state_field = format!("steps.{}.supervisor_state", step_index);
        let last_activity_field = format!("steps.{}.last_activity_at", step_index);
        let last_progress_field = format!("steps.{}.last_progress_at", step_index);
        let progress_score_field = format!("steps.{}.progress_score", step_index);
        let current_blocker_field = format!("steps.{}.current_blocker", step_index);
        let last_supervisor_hint_field = format!("steps.{}.last_supervisor_hint", step_index);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        &status_field: "pending",
                        "updated_at": bson::DateTime::now(),
                    },
                    "$unset": {
                        &error_field: "",
                        &started_field: "",
                        &completed_field: "",
                        &summary_field: "",
                        &tool_calls_field: "",
                        &supervisor_state_field: "",
                        &last_activity_field: "",
                        &last_progress_field: "",
                        &progress_score_field: "",
                        &current_blocker_field: "",
                        &last_supervisor_hint_field: "",
                    },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Replace remaining steps after a re-plan, incrementing plan_version.
    pub async fn replan_remaining_steps(
        &self,
        mission_id: &str,
        all_steps: Vec<MissionStep>,
    ) -> Result<(), mongodb::error::Error> {
        let steps_bson = bson::to_bson(&all_steps)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        "steps": steps_bson,
                        "updated_at": bson::DateTime::now(),
                    },
                    "$inc": { "plan_version": 1 },
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn approve_step(
        &self,
        mission_id: &str,
        step_index: u32,
        approver_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_field = format!("steps.{}.status", step_index);
        let approver_field = format!("steps.{}.approved_by", step_index);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &status_field: "pending",
                    &approver_field: approver_id,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn advance_mission_step(
        &self,
        mission_id: &str,
        next_step: u32,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "current_step": next_step as i32,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        if let Some(refreshed) = self.get_mission(mission_id).await? {
            let _ = sync_v4_runtime_projection_from_mission(self, &refreshed, None).await;
        }
        Ok(())
    }

    // ─── Artifact Management ─────────────────────────────

    pub async fn save_artifact(
        &self,
        artifact: &MissionArtifactDoc,
    ) -> Result<(), mongodb::error::Error> {
        // Upsert by mission_id + file_path to deduplicate across steps/goals
        if let Some(ref fp) = artifact.file_path {
            let filter = doc! {
                "mission_id": &artifact.mission_id,
                "file_path": fp,
            };
            let mut replacement = artifact.clone();
            replacement.id = None;
            let opts = mongodb::options::ReplaceOptions::builder()
                .upsert(true)
                .build();
            self.artifacts()
                .replace_one(filter, replacement, opts)
                .await?;
        } else {
            self.artifacts().insert_one(artifact, None).await?;
        }
        Ok(())
    }

    pub async fn list_mission_artifacts(
        &self,
        mission_id: &str,
    ) -> Result<Vec<MissionArtifactDoc>, mongodb::error::Error> {
        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "step_index": 1, "created_at": 1 })
            .build();
        let cursor = self
            .artifacts()
            .find(doc! { "mission_id": mission_id }, opts)
            .await?;
        cursor.try_collect().await
    }

    pub async fn refresh_delivery_manifest_from_artifacts(
        &self,
        mission_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let Some(mission) = self.get_mission(mission_id).await? else {
            return Ok(());
        };
        let requested = active_requested_deliverables_from_manifest(&mission);
        let requirements = mission
            .delivery_manifest
            .as_ref()
            .map(|manifest| manifest.requirements.clone())
            .unwrap_or_default();

        let artifacts = self.list_mission_artifacts(mission_id).await?;
        let mut requested_remaining = requested.iter().cloned().collect::<BTreeSet<_>>();
        let mut satisfied = Vec::new();
        let mut supporting = Vec::new();
        let mut seen_satisfied = BTreeSet::new();
        let mut seen_supporting = BTreeSet::new();

        for artifact in artifacts {
            let raw_value = artifact
                .file_path
                .clone()
                .filter(|path| !path.trim().is_empty())
                .unwrap_or_else(|| artifact.name.clone());
            if raw_value.trim().is_empty() {
                continue;
            }
            let normalized = runtime::normalize_relative_workspace_path(&raw_value)
                .unwrap_or_else(|| raw_value.trim().replace('\\', "/"));
            if normalized.is_empty() {
                continue;
            }
            let candidates = artifact_match_candidates(&normalized);
            let matched_request = requested_remaining
                .iter()
                .find(|requested_item| {
                    let request_candidates = artifact_match_candidates(requested_item);
                    candidates
                        .iter()
                        .any(|candidate| request_candidates.iter().any(|req| req == candidate))
                })
                .cloned();

            if let Some(requested_item) = matched_request {
                requested_remaining.remove(&requested_item);
                if seen_satisfied.insert(normalized.clone()) {
                    satisfied.push(normalized);
                }
                continue;
            }

            if seen_supporting.insert(normalized.clone()) {
                supporting.push(normalized);
            }
        }

        let mut missing_core_deliverables = requested_remaining.into_iter().collect::<Vec<_>>();
        if !requirements.is_empty() {
            let mut conditional_missing = BTreeSet::new();
            for requirement in &requirements {
                if !requirement_is_active(&mission, requirement) {
                    continue;
                }
                let paths = requirement_paths(requirement);
                if paths.is_empty() {
                    continue;
                }
                let matched = paths
                    .iter()
                    .filter(|path| {
                        satisfied
                            .iter()
                            .any(|done| deliverable_paths_overlap(done, path))
                    })
                    .count();
                let satisfied_requirement = match requirement.mode {
                    MissionDeliverableRequirementMode::AllOf => matched == paths.len(),
                    MissionDeliverableRequirementMode::AnyOf => matched > 0,
                };
                if !satisfied_requirement {
                    match requirement.mode {
                        MissionDeliverableRequirementMode::AllOf => {
                            for path in paths {
                                if !satisfied
                                    .iter()
                                    .any(|done| deliverable_paths_overlap(done, &path))
                                {
                                    conditional_missing.insert(path);
                                }
                            }
                        }
                        MissionDeliverableRequirementMode::AnyOf => {
                            for path in paths {
                                conditional_missing.insert(path);
                            }
                        }
                    }
                }
            }
            if !conditional_missing.is_empty() {
                missing_core_deliverables = conditional_missing.into_iter().collect();
            }
        }
        let final_outcome_summary = mission
            .delivery_manifest
            .as_ref()
            .and_then(|manifest| manifest.final_outcome_summary.clone());
        let manifest = MissionDeliveryManifest {
            requirements,
            requested_deliverables: requested,
            satisfied_deliverables: satisfied,
            missing_core_deliverables,
            supporting_artifacts: supporting,
            delivery_state: effective_delivery_state(&mission)
                .unwrap_or(MissionDeliveryState::Working),
            final_outcome_summary,
        };
        let manifest_bson = bson::to_bson(&manifest)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "delivery_manifest": manifest_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        let _ = self.settle_mission_result_if_ready(mission_id).await?;
        Ok(())
    }

    pub async fn prune_mission_artifacts_to_paths(
        &self,
        mission_id: &str,
        keep_file_paths: &[String],
    ) -> Result<u64, mongodb::error::Error> {
        let keep_paths = keep_file_paths
            .iter()
            .map(|path| bson::Bson::String(path.clone()))
            .collect::<Vec<_>>();
        let filter = doc! {
            "mission_id": mission_id,
            "file_path": {
                "$exists": true,
                "$nin": keep_paths,
            }
        };
        let result = self.artifacts().delete_many(filter, None).await?;
        Ok(result.deleted_count)
    }

    pub async fn get_artifact(
        &self,
        artifact_id: &str,
    ) -> Result<Option<MissionArtifactDoc>, mongodb::error::Error> {
        self.artifacts()
            .find_one(doc! { "artifact_id": artifact_id }, None)
            .await
    }

    pub async fn set_artifact_document_link(
        &self,
        artifact_id: &str,
        document_id: &str,
        document_status: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.artifacts()
            .update_one(
                doc! { "artifact_id": artifact_id },
                doc! {
                    "$set": {
                        "archived_document_id": document_id,
                        "archived_document_status": document_status,
                        "archived_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Persist mission runtime stream events for replay/analysis.
    pub async fn save_mission_stream_events(
        &self,
        items: &[(String, String, u64, StreamEvent)],
    ) -> Result<(), mongodb::error::Error> {
        if items.is_empty() {
            return Ok(());
        }

        let docs: Vec<MissionEventDoc> = items
            .iter()
            .map(|(mission_id, run_id, event_id, event)| MissionEventDoc {
                id: None,
                mission_id: mission_id.clone(),
                run_id: Some(run_id.clone()),
                event_id: (*event_id).try_into().unwrap_or(i64::MAX),
                event_type: event.event_type().to_string(),
                payload: serde_json::to_value(event).unwrap_or_else(|_| serde_json::json!({})),
                created_at: bson::DateTime::now(),
            })
            .collect();

        self.mission_events().insert_many(docs, None).await?;
        Ok(())
    }

    pub async fn list_mission_events(
        &self,
        mission_id: &str,
        run_id: Option<&str>,
        after_event_id: Option<u64>,
        limit: u32,
    ) -> Result<Vec<MissionEventDoc>, mongodb::error::Error> {
        let clamped_limit = limit.clamp(1, 2000);
        let mut filter = doc! { "mission_id": mission_id };
        if let Some(run) = run_id {
            filter.insert("run_id", run);
        }
        if let Some(after) = after_event_id {
            filter.insert("event_id", doc! { "$gt": after as i64 });
        }

        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "event_id": 1, "created_at": 1 })
            .limit(clamped_limit as i64)
            .build();
        let mut events: Vec<MissionEventDoc> = self
            .mission_events()
            .find(filter.clone(), opts.clone())
            .await?
            .try_collect()
            .await?;

        // Handle run switches gracefully:
        // if caller sends a stale `after_event_id` from a previous run,
        // restart from beginning of current run instead of returning empty forever.
        if events.is_empty() {
            if let (Some(run), Some(after)) = (run_id, after_event_id) {
                let max_opts = mongodb::options::FindOneOptions::builder()
                    .sort(doc! { "event_id": -1, "created_at": -1 })
                    .build();
                let max_doc = self
                    .mission_events()
                    .find_one(doc! { "mission_id": mission_id, "run_id": run }, max_opts)
                    .await?;
                if let Some(latest) = max_doc {
                    if latest.event_id < after as i64 {
                        let restart_filter = doc! { "mission_id": mission_id, "run_id": run };
                        events = self
                            .mission_events()
                            .find(restart_filter, opts)
                            .await?
                            .try_collect()
                            .await?;
                    }
                }
            }
        }

        Ok(events)
    }

    // ─── Goal Tree Management (AGE) ─────────────────────

    /// Save initial goal tree for a mission.
    pub async fn save_goal_tree(
        &self,
        mission_id: &str,
        goals: Vec<GoalNode>,
    ) -> Result<(), mongodb::error::Error> {
        let goals_bson = bson::to_bson(&goals)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree": goals_bson,
                    "delivery_state": bson::to_bson(&MissionDeliveryState::Working)
                        .unwrap_or(bson::Bson::Null),
                    "delivery_manifest.delivery_state": bson::to_bson(&MissionDeliveryState::Working)
                        .unwrap_or(bson::Bson::Null),
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        self.refresh_progress_memory(mission_id).await?;
        Ok(())
    }

    pub async fn set_delivery_manifest_requested_deliverables(
        &self,
        mission_id: &str,
        requested_deliverables: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let normalized = normalize_requested_deliverables(requested_deliverables);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "delivery_state": bson::to_bson(&MissionDeliveryState::Working)
                        .unwrap_or(bson::Bson::Null),
                    "delivery_manifest.requested_deliverables": bson::to_bson(&normalized)
                        .unwrap_or(bson::Bson::Array(Vec::new())),
                    "delivery_manifest.delivery_state": bson::to_bson(&MissionDeliveryState::Working)
                        .unwrap_or(bson::Bson::Null),
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        self.refresh_progress_memory(mission_id).await?;
        Ok(())
    }

    pub async fn set_delivery_manifest_requirements(
        &self,
        mission_id: &str,
        requirements: &[MissionDeliverableRequirement],
    ) -> Result<(), mongodb::error::Error> {
        let Some(mission) = self.get_mission(mission_id).await? else {
            return Ok(());
        };
        let requested_deliverables = if requirements.is_empty() {
            mission
                .delivery_manifest
                .as_ref()
                .map(|manifest| manifest.requested_deliverables.clone())
                .unwrap_or_default()
        } else {
            let mut seen = HashSet::new();
            let mut requested = Vec::new();
            for requirement in requirements {
                if requirement_is_active(&mission, requirement) {
                    for path in requirement_paths(requirement) {
                        if seen.insert(path.clone()) {
                            requested.push(path);
                        }
                    }
                }
            }
            requested
        };
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "delivery_manifest.requirements": bson::to_bson(requirements)
                        .unwrap_or(bson::Bson::Array(Vec::new())),
                    "delivery_manifest.requested_deliverables": bson::to_bson(&requested_deliverables)
                        .unwrap_or(bson::Bson::Array(Vec::new())),
                    "delivery_manifest.delivery_state": bson::to_bson(&MissionDeliveryState::Working)
                        .unwrap_or(bson::Bson::Null),
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        self.refresh_progress_memory(mission_id).await?;
        Ok(())
    }

    /// Update a goal's status using arrayFilters.
    /// Automatically sets completed_at when status is Completed or Abandoned.
    pub async fn update_goal_status(
        &self,
        mission_id: &str,
        goal_id: &str,
        status: &GoalStatus,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(status)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let now = bson::DateTime::now();
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();

        let mut set_doc = doc! {
            "goal_tree.$[elem].status": status_bson,
            "updated_at": now,
        };

        if matches!(status, GoalStatus::Running) {
            set_doc.insert("goal_tree.$[elem].started_at", now);
            set_doc.insert("goal_tree.$[elem].last_activity_at", now);
        }

        // Set completed_at for terminal statuses
        if matches!(
            status,
            GoalStatus::Completed | GoalStatus::Abandoned | GoalStatus::Failed
        ) {
            set_doc.insert("goal_tree.$[elem].completed_at", now);
        }

        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": set_doc },
                opts,
            )
            .await?;
        Ok(())
    }

    pub async fn set_goal_supervision(
        &self,
        mission_id: &str,
        goal_id: &str,
        last_activity_at: Option<bson::DateTime>,
        last_progress_at: Option<bson::DateTime>,
    ) -> Result<(), mongodb::error::Error> {
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        let now = bson::DateTime::now();
        let mut set_doc = doc! {
            "updated_at": now,
        };

        match last_activity_at {
            Some(ts) => {
                set_doc.insert("goal_tree.$[elem].last_activity_at", ts);
            }
            None => {
                set_doc.insert("goal_tree.$[elem].last_activity_at", Bson::Null);
            }
        }

        match last_progress_at {
            Some(ts) => {
                set_doc.insert("goal_tree.$[elem].last_progress_at", ts);
            }
            None => {
                set_doc.insert("goal_tree.$[elem].last_progress_at", Bson::Null);
            }
        }

        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": set_doc },
                opts,
            )
            .await?;
        Ok(())
    }

    pub async fn touch_goal_activity(
        &self,
        mission_id: &str,
        goal_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        let now = bson::DateTime::now();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].last_activity_at": now,
                    "updated_at": now,
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Reset a goal to pending so adaptive resume can retry it.
    pub async fn reset_goal_for_retry(
        &self,
        mission_id: &str,
        goal_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        "goal_tree.$[elem].status": "pending",
                        "updated_at": bson::DateTime::now(),
                    },
                    "$unset": {
                        "goal_tree.$[elem].started_at": "",
                        "goal_tree.$[elem].last_activity_at": "",
                        "goal_tree.$[elem].last_progress_at": "",
                        "goal_tree.$[elem].completed_at": "",
                    },
                },
                opts,
            )
            .await?;
        Ok(())
    }

    /// Append an AttemptRecord to a goal's attempts array.
    pub async fn push_goal_attempt(
        &self,
        mission_id: &str,
        goal_id: &str,
        attempt: &AttemptRecord,
    ) -> Result<(), mongodb::error::Error> {
        let attempt_bson = bson::to_bson(attempt)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$push": { "goal_tree.$[elem].attempts": attempt_bson },
                    "$set": {
                        "updated_at": bson::DateTime::now(),
                        "goal_tree.$[elem].last_activity_at": bson::DateTime::now(),
                        "goal_tree.$[elem].last_progress_at": bson::DateTime::now(),
                    },
                },
                opts,
            )
            .await?;
        Ok(())
    }

    /// Update the signal of the last attempt for a goal.
    pub async fn update_last_attempt_signal(
        &self,
        mission_id: &str,
        goal_id: &str,
        signal: &ProgressSignal,
    ) -> Result<(), mongodb::error::Error> {
        // Read current goal to find the last attempt index
        let mission = self.get_mission(mission_id).await?;
        if let Some(mission) = mission {
            if let Some(goals) = &mission.goal_tree {
                if let Some(goal) = goals.iter().find(|g| g.goal_id == goal_id) {
                    if !goal.attempts.is_empty() {
                        let last_idx = goal.attempts.len() - 1;
                        let signal_bson = bson::to_bson(signal).map_err(|e| {
                            mongodb::error::Error::custom(format!("BSON error: {}", e))
                        })?;
                        let field = format!("goal_tree.$[elem].attempts.{}.signal", last_idx);
                        let opts = mongodb::options::UpdateOptions::builder()
                            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
                            .build();
                        self.missions()
                            .update_one(
                                doc! { "mission_id": mission_id },
                                doc! { "$set": { &field: signal_bson, "updated_at": bson::DateTime::now() } },
                                opts,
                            )
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Set a goal's output_summary.
    pub async fn set_goal_output_summary(
        &self,
        mission_id: &str,
        goal_id: &str,
        summary: &str,
    ) -> Result<(), mongodb::error::Error> {
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].output_summary": summary,
                    "goal_tree.$[elem].last_activity_at": bson::DateTime::now(),
                    "goal_tree.$[elem].last_progress_at": bson::DateTime::now(),
                    "updated_at": bson::DateTime::now(),
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Persist goal runtime contract extracted from mission_preflight.
    pub async fn set_goal_runtime_contract(
        &self,
        mission_id: &str,
        goal_id: &str,
        contract: &RuntimeContract,
    ) -> Result<(), mongodb::error::Error> {
        let contract_bson = bson::to_bson(contract)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].runtime_contract": contract_bson,
                    "goal_tree.$[elem].last_activity_at": bson::DateTime::now(),
                    "goal_tree.$[elem].last_progress_at": bson::DateTime::now(),
                    "updated_at": bson::DateTime::now(),
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Persist goal contract verification result.
    pub async fn set_goal_contract_verification(
        &self,
        mission_id: &str,
        goal_id: &str,
        verification: &RuntimeContractVerification,
    ) -> Result<(), mongodb::error::Error> {
        let verify_bson = bson::to_bson(verification)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].contract_verification": verify_bson,
                    "goal_tree.$[elem].last_activity_at": bson::DateTime::now(),
                    "goal_tree.$[elem].last_progress_at": bson::DateTime::now(),
                    "updated_at": bson::DateTime::now(),
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Set a goal's pivot_reason.
    pub async fn set_goal_pivot(
        &self,
        mission_id: &str,
        goal_id: &str,
        reason: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(&GoalStatus::Pivoting)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].pivot_reason": reason,
                    "goal_tree.$[elem].status": status_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Update current_goal_id on the mission.
    pub async fn advance_mission_goal(
        &self,
        mission_id: &str,
        goal_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "current_goal_id": goal_id,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        if let Some(refreshed) = self.get_mission(mission_id).await? {
            let _ = sync_v4_runtime_projection_from_mission(self, &refreshed, None).await;
        }
        Ok(())
    }

    /// Clear current_goal_id on the mission when adaptive orchestration
    /// closes or completes the remaining plan.
    pub async fn clear_mission_current_goal(
        &self,
        mission_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "current_goal_id": bson::Bson::Null,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        if let Some(refreshed) = self.get_mission(mission_id).await? {
            let _ = sync_v4_runtime_projection_from_mission(self, &refreshed, None).await;
        }
        Ok(())
    }

    /// Atomically set goal pivot status + increment total_pivots counter.
    pub async fn pivot_goal_atomic(
        &self,
        mission_id: &str,
        goal_id: &str,
        reason: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(&GoalStatus::Pivoting)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let now = bson::DateTime::now();
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        "goal_tree.$[elem].pivot_reason": reason,
                        "goal_tree.$[elem].status": status_bson,
                        "updated_at": now,
                    },
                    "$inc": { "total_pivots": 1 },
                },
                opts,
            )
            .await?;
        Ok(())
    }

    /// Atomically abandon goal + increment total_abandoned counter.
    pub async fn abandon_goal_atomic(
        &self,
        mission_id: &str,
        goal_id: &str,
        reason: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(&GoalStatus::Abandoned)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let now = bson::DateTime::now();
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        "goal_tree.$[elem].status": status_bson,
                        "goal_tree.$[elem].pivot_reason": reason,
                        "goal_tree.$[elem].completed_at": now,
                        "updated_at": now,
                    },
                    "$inc": { "total_abandoned": 1 },
                },
                opts,
            )
            .await?;
        Ok(())
    }

    /// Insert child goals into the goal_tree array.
    pub async fn insert_child_goals(
        &self,
        mission_id: &str,
        new_goals: Vec<GoalNode>,
    ) -> Result<(), mongodb::error::Error> {
        let goals_bson = bson::to_bson(&new_goals)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$push": { "goal_tree": { "$each": goals_bson } },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Mark a goal as Abandoned with reason and timestamp.
    pub async fn abandon_goal(
        &self,
        mission_id: &str,
        goal_id: &str,
        reason: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(&GoalStatus::Abandoned)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let now = bson::DateTime::now();
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].status": status_bson,
                    "goal_tree.$[elem].pivot_reason": reason,
                    "goal_tree.$[elem].completed_at": now,
                    "updated_at": now,
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Claim orphaned Running/Planning missions on server startup so the current
    /// instance can auto-resume them instead of requiring manual recovery.
    pub async fn claim_orphaned_missions_for_resume(
        &self,
        instance_id: &str,
    ) -> Result<Vec<String>, mongodb::error::Error> {
        let now = bson::DateTime::now();
        let filter = orphaned_mission_recovery_filter(instance_id);
        let candidates = self
            .missions()
            .find(filter, None)
            .await?
            .try_collect::<Vec<MissionDoc>>()
            .await?;

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let mut claimed = Vec::new();
        for mission in candidates {
            let mission = self.project_v4_mission_overlay(&mission).await?;
            if mission_should_skip_auto_resume(&mission) {
                continue;
            }
            if self.settle_mission_result_if_ready(&mission.mission_id).await? {
                continue;
            }
            let result = self
                .missions()
                .update_one(
                    doc! {
                        "mission_id": &mission.mission_id,
                        "status": { "$in": ["running", "planning"] },
                        "updated_at": mission.updated_at,
                    },
                    paused_for_auto_resume_update(
                        now,
                        "Server restarted while mission was in progress; preparing auto-resume",
                    ),
                    None,
                )
                .await?;
            if result.modified_count > 0 {
                self.refresh_delivery_manifest_from_artifacts(&mission.mission_id)
                    .await?;
                self.refresh_progress_memory(&mission.mission_id).await?;
                let _ = self.settle_mission_result_if_ready(&mission.mission_id).await?;
                claimed.push(mission.mission_id);
            }
        }
        Ok(claimed)
    }

    /// Claim missions that still look active in Mongo but are no longer being
    /// tracked by the in-process MissionManager. This covers executor exits
    /// where the runtime loop stops without transitioning the persisted mission
    /// out of running/planning.
    pub async fn claim_inactive_running_missions_for_resume(
        &self,
        max_age: std::time::Duration,
        active_mission_ids: &[String],
    ) -> Result<Vec<String>, mongodb::error::Error> {
        let stale_before = Utc::now()
            - chrono::Duration::from_std(max_age).unwrap_or_else(|_| chrono::Duration::minutes(10));
        let filter = active_running_mission_scan_filter(active_mission_ids);
        let candidates = self
            .missions()
            .find(filter, None)
            .await?
            .try_collect::<Vec<MissionDoc>>()
            .await?;

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let now = bson::DateTime::now();
        let mut claimed = Vec::new();
        for mission in candidates {
            let mission = self.project_v4_mission_overlay(&mission).await?;
            if self.settle_mission_result_if_ready(&mission.mission_id).await? {
                continue;
            }
            let has_active_task = self
                .mission_has_active_executor_task(&mission.mission_id, mission.current_run_id.as_deref())
                .await?;
            let lease_before =
                Utc::now() - chrono::Duration::seconds(execution_lease_ttl_secs().max(1));
            let stale_reclaim = mission_inactive_for_resume(&mission, stale_before);
            let conservative_lease_reclaim = mission_inactive_for_resume(&mission, lease_before);
            let fast_execution_stall_reclaim =
                !has_active_task && mission_fast_reclaim_after_execution_stall(&mission);
            if has_active_task && !conservative_lease_reclaim {
                continue;
            }
            if !stale_reclaim && !conservative_lease_reclaim && !fast_execution_stall_reclaim {
                continue;
            }
            let result = self
                .missions()
                .update_one(
                    doc! {
                        "mission_id": &mission.mission_id,
                        "status": { "$in": ["running", "planning"] },
                        "updated_at": mission.updated_at,
                    },
                    paused_for_auto_resume_update(
                        now,
                        "Mission supervisor detected no activity and scheduled auto-resume",
                    ),
                    None,
                )
                .await?;
            if result.modified_count > 0 {
                self.refresh_delivery_manifest_from_artifacts(&mission.mission_id)
                    .await?;
                self.refresh_progress_memory(&mission.mission_id).await?;
                let _ = self.settle_mission_result_if_ready(&mission.mission_id).await?;
                claimed.push(mission.mission_id);
            }
        }

        Ok(claimed)
    }

    pub async fn collect_paused_missions_for_attention(
        &self,
    ) -> Result<Vec<String>, mongodb::error::Error> {
        let candidates = self
            .missions()
            .find(doc! { "status": { "$in": ["paused", "failed"] } }, None)
            .await?
            .try_collect::<Vec<MissionDoc>>()
            .await?;

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let mut claimed = Vec::new();
        for mission in candidates {
            let mission = self.project_v4_mission_overlay(&mission).await?;
            if mission_should_skip_auto_resume(&mission) {
                continue;
            }
            if self.settle_mission_result_if_ready(&mission.mission_id).await? {
                continue;
            }
            let has_active_task = self
                .mission_has_active_executor_task(&mission.mission_id, mission.current_run_id.as_deref())
                .await?;
            if has_active_task || execution_lease_active(mission.execution_lease.as_ref()) {
                continue;
            }

            let pause_reason = mission.error_message.clone().unwrap_or_default().to_ascii_lowercase();
            let blocker = mission
                .progress_memory
                .as_ref()
                .and_then(|memory| memory.blocked_by.clone())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let auto_attention_pause = pause_reason.contains("auto-resume")
                || pause_reason.contains("server restarted while mission was in progress")
                || pause_reason.contains("mission supervisor detected no activity")
                || pause_reason.contains("task cancelled during fallback complete call")
                || blocker.contains("no tool calls")
                || blocker.contains("target file delta")
                || blocker.contains("task cancelled during fallback complete call");
            if auto_attention_pause {
                claimed.push(mission.mission_id);
            }
        }

        Ok(claimed)
    }

    pub async fn settle_completed_mission_candidates(
        &self,
    ) -> Result<Vec<String>, mongodb::error::Error> {
        let candidates = self
            .missions()
            .find(
                doc! {
                    "status": { "$in": ["running", "planning", "paused", "failed", "cancelled"] },
                },
                None,
            )
            .await?
            .try_collect::<Vec<MissionDoc>>()
            .await?;

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let mut settled = Vec::new();
        for mission in candidates {
            let mission = self.project_v4_mission_overlay(&mission).await?;
            let should_consider = mission
                .delivery_manifest
                .as_ref()
                .map(|manifest| {
                    !normalize_concrete_deliverable_paths(&manifest.requested_deliverables)
                        .is_empty()
                        && normalize_concrete_deliverable_paths(&manifest.missing_core_deliverables)
                            .is_empty()
                })
                .unwrap_or(false);
            if !should_consider {
                continue;
            }
            if self.settle_mission_result_if_ready(&mission.mission_id).await? {
                settled.push(mission.mission_id);
            }
        }
        Ok(settled)
    }

    pub async fn pause_mission_for_auto_resume(
        &self,
        mission_id: &str,
        reason: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let mission_before = self.get_mission(mission_id).await?;
        if let Some(mission) = mission_before.as_ref() {
            if mission_should_skip_auto_resume(&mission) {
                return Ok(false);
            }
        }
        let now = bson::DateTime::now();
        let result = self
            .missions()
            .update_one(
                doc! {
                    "mission_id": mission_id,
                    "status": { "$in": ["running", "planning"] },
                    "$or": [
                        { "waiting_external_until": { "$exists": false } },
                        { "waiting_external_until": bson::Bson::Null },
                        { "waiting_external_until": { "$lte": now } },
                    ],
                },
                paused_for_auto_resume_update(now, reason),
                None,
            )
            .await?;
        if result.modified_count > 0 {
            if let Some(mission) = mission_before.as_ref() {
                let _ = sync_v4_runtime_projection_from_mission(
                    self,
                    mission,
                    Some(RunStatus::Paused),
                )
                .await;
            }
            self.refresh_delivery_manifest_from_artifacts(mission_id)
                .await?;
            self.refresh_progress_memory(mission_id).await?;
            let _ = self.settle_mission_result_if_ready(mission_id).await?;
        }
        Ok(result.modified_count > 0)
    }

    pub async fn restore_waiting_external_running_state(
        &self,
        mission_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let mission_before = self.get_mission(mission_id).await?;
        let now = bson::DateTime::now();
        let mut set = doc! {
            "status": "running",
            "updated_at": now,
        };
        if let Ok(instance_id) = std::env::var("TEAM_SERVER_INSTANCE_ID") {
            set.insert("server_instance_id", instance_id);
        }

        let result = self
            .missions()
            .update_one(
                doc! {
                    "mission_id": mission_id,
                    "waiting_external_until": { "$gt": now },
                    "status": { "$in": ["paused", "planning", "running"] },
                },
                doc! {
                    "$set": set,
                    "$unset": {
                        "error_message": "",
                    }
                },
                None,
            )
            .await?;
        if result.modified_count > 0 {
            if let Some(mission) = mission_before.as_ref() {
                let _ = sync_v4_runtime_projection_from_mission(
                    self,
                    mission,
                    Some(RunStatus::Executing),
                )
                .await;
            }
        }
        Ok(result.modified_count > 0)
    }

    // ─── Mission Indexes ─────────────────────────────────

    pub async fn ensure_mission_indexes(&self) {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        let mission_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "status": 1, "created_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "agent_id": 1, "status": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "creator_id": 1, "status": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        ];

        if let Err(e) = self.missions().create_indexes(mission_indexes, None).await {
            tracing::warn!("Failed to create mission indexes: {}", e);
        }

        let artifact_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "step_index": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "artifact_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        ];

        if let Err(e) = self
            .artifacts()
            .create_indexes(artifact_indexes, None)
            .await
        {
            tracing::warn!("Failed to create artifact indexes: {}", e);
        }

        let event_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "event_id": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "run_id": 1, "event_id": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "run_id": { "$exists": true } })
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "created_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "run_id": 1, "created_at": 1 })
                .build(),
        ];

        if let Err(e) = self
            .mission_events()
            .create_indexes(event_indexes, None)
            .await
        {
            tracing::warn!("Failed to create mission event indexes: {}", e);
        }

        let run_state_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "run_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "status": 1, "updated_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "task_graph_id": 1, "current_node_id": 1 })
                .build(),
        ];

        if let Err(e) = self.run_states().create_indexes(run_state_indexes, None).await {
            tracing::warn!("Failed to create V4 run_state indexes: {}", e);
        }

        let run_journal_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "run_id": 1, "created_at": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "created_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "run_id": 1, "task_node_id": 1, "created_at": 1 })
                .build(),
        ];

        if let Err(e) = self.run_journal().create_indexes(run_journal_indexes, None).await {
            tracing::warn!("Failed to create V4 run_journal indexes: {}", e);
        }

        let checkpoint_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "run_id": 1, "created_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "created_at": -1 })
                .build(),
        ];

        if let Err(e) = self
            .run_checkpoints()
            .create_indexes(checkpoint_indexes, None)
            .await
        {
            tracing::warn!("Failed to create V4 checkpoint indexes: {}", e);
        }

        let subagent_run_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "subagent_run_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! { "parent_run_id": 1, "status": 1, "updated_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "parent_task_node_id": 1 })
                .build(),
        ];

        if let Err(e) = self
            .subagent_runs()
            .create_indexes(subagent_run_indexes, None)
            .await
        {
            tracing::warn!("Failed to create V4 subagent_run indexes: {}", e);
        }

        let task_graph_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "task_graph_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "graph_version": -1 })
                .build(),
        ];

        if let Err(e) = self.task_graphs().create_indexes(task_graph_indexes, None).await {
            tracing::warn!("Failed to create V4 task_graph indexes: {}", e);
        }
    }
}

fn orphaned_mission_recovery_filter(instance_id: &str) -> mongodb::bson::Document {
    let running_states = vec![
        bson::Bson::String("running".to_string()),
        bson::Bson::String("planning".to_string()),
    ];

    let ownership_guard = if instance_id.trim().is_empty() {
        vec![
            doc! { "server_instance_id": { "$exists": false } },
            doc! { "server_instance_id": bson::Bson::Null },
        ]
    } else {
        vec![
            doc! { "server_instance_id": { "$exists": false } },
            doc! { "server_instance_id": bson::Bson::Null },
            doc! { "server_instance_id": { "$ne": instance_id } },
        ]
    };

    doc! {
        "status": { "$in": running_states },
        "$or": ownership_guard,
    }
}

fn active_running_mission_scan_filter(_active_mission_ids: &[String]) -> mongodb::bson::Document {
    let running_states = vec![
        bson::Bson::String("running".to_string()),
        bson::Bson::String("planning".to_string()),
    ];

    doc! {
        "status": { "$in": running_states },
    }
}

fn goal_is_open_for_semantic_close(status: &GoalStatus) -> bool {
    matches!(
        status,
        GoalStatus::Pending | GoalStatus::Running | GoalStatus::AwaitingApproval | GoalStatus::Pivoting
    )
}

fn manifest_completion_assessment_if_satisfied(
    mission: &MissionDoc,
) -> Option<MissionCompletionAssessment> {
    let manifest = mission.delivery_manifest.as_ref()?;
    let requested = normalize_concrete_deliverable_paths(&manifest.requested_deliverables);
    if requested.is_empty() {
        return None;
    }
    let satisfied = normalize_concrete_deliverable_paths(&manifest.satisfied_deliverables);
    let missing = normalize_concrete_deliverable_paths(&manifest.missing_core_deliverables);
    if !missing.is_empty() {
        return None;
    }
    if requested
        .iter()
        .any(|requested_item| !satisfied.iter().any(|item| item == requested_item))
    {
        return None;
    }

    let observed_evidence = if !manifest.satisfied_deliverables.is_empty() {
        manifest.satisfied_deliverables.clone()
    } else {
        mission
            .latest_worker_state
            .as_ref()
            .map(|state| state.core_assets_now.clone())
            .unwrap_or_default()
    };
    let result_satisfied_reason =
        "All requested deliverables are already satisfied; finalizing mission".to_string();
    let reason = match effective_delivery_state(mission) {
        Some(MissionDeliveryState::WaitingExternal)
        | Some(MissionDeliveryState::Working)
        | Some(MissionDeliveryState::RepairingDeliverables)
        | Some(MissionDeliveryState::RepairingContract)
        | Some(MissionDeliveryState::ReadyToComplete) => Some(result_satisfied_reason.clone()),
        _ => mission
            .final_summary
            .clone()
            .or_else(|| manifest.final_outcome_summary.clone())
            .or_else(|| Some(result_satisfied_reason.clone())),
    };

    let disposition = MissionCompletionDisposition::Complete;

    Some(MissionCompletionAssessment {
        disposition,
        reason,
        observed_evidence,
        missing_core_deliverables: Vec::new(),
        recorded_at: Some(bson::DateTime::now()),
    })
}

fn close_goal_tree_for_semantic_completion(
    mission: &MissionDoc,
    reason: &str,
) -> Option<Vec<GoalNode>> {
    let goals = mission.goal_tree.as_ref()?;
    let now = bson::DateTime::now();
    let current_goal_id = mission.current_goal_id.as_deref();
    let mut changed = false;
    let mut closed_goals = Vec::with_capacity(goals.len());

    for goal in goals {
        let mut updated = goal.clone();
        if goal_is_open_for_semantic_close(&updated.status) {
            changed = true;
            updated.status = if current_goal_id.is_some_and(|goal_id| goal_id == updated.goal_id) {
                GoalStatus::Completed
            } else {
                GoalStatus::Abandoned
            };
            if updated
                .output_summary
                .as_ref()
                .is_none_or(|value| value.trim().is_empty())
            {
                updated.output_summary = Some(reason.to_string());
            }
            updated.completed_at = Some(now);
            updated.last_progress_at = Some(now);
            updated.last_activity_at = Some(now);
        }
        closed_goals.push(updated);
    }

    changed.then_some(closed_goals)
}

pub(crate) async fn finalize_inactive_semantic_completion_if_ready(
    service: &AgentService,
    mission: &MissionDoc,
) -> Result<bool, mongodb::error::Error> {
    let assessment = manifest_completion_assessment_if_satisfied(mission);
    let Some(assessment) = assessment else {
        return Ok(false);
    };
    let artifacts = service.list_mission_artifacts(&mission.mission_id).await?;
    let quality_blockers =
        research_quality_blockers(mission, &artifacts, &assessment.observed_evidence);
    if !quality_blockers.is_empty() {
        tracing::info!(
            "Skipping semantic completion for mission {} due to research quality blockers: {}",
            mission.mission_id,
            quality_blockers.join(", ")
        );
        return Ok(false);
    }
    let terminal_delivery_state = match assessment.disposition {
        MissionCompletionDisposition::Complete => MissionDeliveryState::Complete,
        MissionCompletionDisposition::CompletedWithMinorGaps => {
            MissionDeliveryState::CompletedWithMinorGaps
        }
        MissionCompletionDisposition::PartialHandoff => MissionDeliveryState::PartialHandoff,
        MissionCompletionDisposition::BlockedByEnvironment => {
            MissionDeliveryState::BlockedByEnvironment
        }
        MissionCompletionDisposition::BlockedByTooling => MissionDeliveryState::BlockedByTooling,
        MissionCompletionDisposition::WaitingExternal => MissionDeliveryState::WaitingExternal,
        MissionCompletionDisposition::BlockedFail => MissionDeliveryState::PartialHandoffCandidate,
    };
    let now = bson::DateTime::now();
    let completed_done = if !assessment.observed_evidence.is_empty() {
        normalize_concrete_deliverable_paths(&assessment.observed_evidence)
    } else {
        mission
            .delivery_manifest
            .as_ref()
            .map(|manifest| normalize_concrete_deliverable_paths(&manifest.satisfied_deliverables))
            .unwrap_or_default()
    };
    let terminal_progress_memory = MissionProgressMemory {
        done: completed_done,
        missing: Vec::new(),
        blocked_by: None,
        last_failed_attempt: None,
        next_best_action: None,
        confidence: None,
        updated_at: Some(now),
    };
    let mut set_doc = doc! {
        "status": bson::to_bson(&MissionStatus::Completed)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
        "completion_assessment": bson::to_bson(&assessment)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
        "delivery_state": bson::to_bson(&terminal_delivery_state)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
        "delivery_manifest.delivery_state": bson::to_bson(&terminal_delivery_state)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
        "delivery_manifest.final_outcome_summary": assessment.reason.clone(),
        "delivery_manifest.missing_core_deliverables": bson::to_bson(&Vec::<String>::new())
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
        "progress_memory": bson::to_bson(&terminal_progress_memory)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
        "pending_monitor_intervention": bson::Bson::Null,
        "current_goal_id": bson::Bson::Null,
        "current_step": bson::Bson::Null,
        "active_repair_lane_id": bson::Bson::Null,
        "consecutive_no_tool_count": 0,
        "last_blocker_fingerprint": bson::Bson::Null,
        "waiting_external_until": bson::Bson::Null,
        "completed_at": now,
        "updated_at": now,
        "error_message": bson::Bson::Null,
    };

    let completion_reason = assessment.reason.as_deref().unwrap_or(
        "AI marked the mission complete and all requested deliverables are satisfied",
    );
    if let Some(closed_goals) =
        close_goal_tree_for_semantic_completion(mission, completion_reason)
    {
        set_doc.insert(
            "goal_tree",
            bson::to_bson(&closed_goals)
                .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
        );
    }

    let result = service
        .missions()
        .update_one(
            doc! {
                "mission_id": &mission.mission_id,
                "status": { "$in": ["running", "planning", "paused", "failed", "cancelled"] },
            },
            doc! { "$set": set_doc },
            None,
        )
        .await?;
    if result.modified_count > 0 {
        let refreshed = service.get_mission(&mission.mission_id).await?;
        let sync_source = refreshed.as_ref().unwrap_or(mission);
        let _ = sync_v4_runtime_projection_from_mission(
            service,
            sync_source,
            Some(RunStatus::Completed),
        )
        .await;
        let _ = hook_runtime::run_settle_hooks(service, sync_source).await;
        let _ = service
            .tasks()
            .update_many(
                doc! {
                    "status": { "$in": ["pending", "approved", "running"] },
                    "$and": [
                        {
                            "$or": [
                                { "task_role": "mission_worker" },
                                { "content.task_role": "mission_worker" },
                            ]
                        },
                        {
                            "$or": [
                                { "mission_id": &mission.mission_id },
                                { "content.mission_id": &mission.mission_id },
                            ]
                        }
                    ],
                },
                doc! { "$set": {
                    "status": "cancelled",
                    "error_message": "Superseded by mission semantic completion",
                    "completed_at": now,
                }},
                None,
            )
            .await;
    }
    Ok(result.modified_count > 0)
}

fn v4_runtime_current_node_id(mission: &MissionDoc) -> Option<String> {
    mission
        .current_goal_id
        .as_ref()
        .map(|goal_id| format!("goal:{}", goal_id))
        .or_else(|| mission.current_step.map(|index| format!("step:{}", index)))
        .or_else(|| mission.current_run_id.as_ref().map(|_| "mission:root".to_string()))
}

fn v4_project_memory_from_mission(mission: &MissionDoc) -> Option<ProjectMemory> {
    let mut assumptions = Vec::new();
    let mut constraints = Vec::new();
    let mut preferences = Vec::new();

    assumptions.push(format!(
        "execution_mode={}",
        serde_json::to_string(&mission.execution_mode)
            .unwrap_or_default()
            .trim_matches('"')
    ));
    assumptions.push(format!(
        "execution_profile={}",
        serde_json::to_string(&resolve_execution_profile(mission))
            .unwrap_or_default()
            .trim_matches('"')
    ));
    assumptions.push(format!(
        "launch_policy={}",
        serde_json::to_string(&resolve_launch_policy(mission))
            .unwrap_or_default()
            .trim_matches('"')
    ));

    if let Some(context) = mission.context.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        constraints.push(context.to_string());
    }
    if mission.approval_policy != super::mission_mongo::ApprovalPolicy::Auto {
        constraints.push(format!(
            "approval_policy={}",
            serde_json::to_string(&mission.approval_policy)
                .unwrap_or_default()
                .trim_matches('"')
        ));
    }
    if let Some(path) = mission.workspace_path.as_deref() {
        preferences.push(format!("workspace_path={path}"));
    }
    if !mission.attached_document_ids.is_empty() {
        preferences.push(format!(
            "attached_document_ids={}",
            mission.attached_document_ids.join(",")
        ));
    }

    if assumptions.is_empty() && constraints.is_empty() && preferences.is_empty() {
        None
    } else {
        Some(ProjectMemory {
            assumptions,
            constraints,
            preferences,
            updated_at: Some(bson::DateTime::now()),
        })
    }
}

fn v4_artifact_memory_from_mission(mission: &MissionDoc) -> Option<ArtifactMemory> {
    let mut known_artifacts = mission
        .delivery_manifest
        .as_ref()
        .map(|manifest| {
            let mut values = manifest.satisfied_deliverables.clone();
            values.extend(manifest.supporting_artifacts.clone());
            normalize_concrete_deliverable_paths(&values)
        })
        .unwrap_or_default();

    if known_artifacts.is_empty() {
        known_artifacts = mission
            .progress_memory
            .as_ref()
            .map(|memory| normalize_concrete_deliverable_paths(&memory.done))
            .unwrap_or_default();
    }

    let templates = mission
        .attached_document_ids
        .iter()
        .map(|id| format!("document:{id}"))
        .collect::<Vec<_>>();
    let scripts = known_artifacts
        .iter()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            lower.ends_with(".py") || lower.ends_with(".sh") || lower.ends_with(".js")
        })
        .cloned()
        .collect::<Vec<_>>();

    if known_artifacts.is_empty() && templates.is_empty() && scripts.is_empty() {
        None
    } else {
        Some(ArtifactMemory {
            known_artifacts,
            templates,
            scripts,
            updated_at: Some(bson::DateTime::now()),
        })
    }
}

async fn sync_v4_runtime_projection_from_mission(
    service: &AgentService,
    mission: &MissionDoc,
    status_override: Option<RunStatus>,
) -> Result<(), mongodb::error::Error> {
    let Some(run_id) = mission.current_run_id.as_deref() else {
        return Ok(());
    };
    let now = bson::DateTime::now();
    let resolved_status = status_override.clone().unwrap_or_else(|| match mission.status {
        MissionStatus::Draft | MissionStatus::Planned => RunStatus::Pending,
        MissionStatus::Planning => RunStatus::Planning,
        MissionStatus::Running => RunStatus::Executing,
        MissionStatus::Paused => RunStatus::Paused,
        MissionStatus::Completed => RunStatus::Completed,
        MissionStatus::Failed => RunStatus::Failed,
        MissionStatus::Cancelled => RunStatus::Cancelled,
    });
    let terminal_status = matches!(
        resolved_status,
        RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled
    );
    let current_node_id = if terminal_status {
        None
    } else {
        v4_runtime_current_node_id(mission)
    };
    let mut set_doc = doc! {
        "status": bson::to_bson(&resolved_status)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
        "updated_at": now,
    };
    let projected_memory = runtime_memory_for_projection(mission, &resolved_status, now);
    if let Some(memory) = projected_memory.as_ref() {
        set_doc.insert(
            "memory",
            bson::to_bson(&memory)
                .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
        );
    }
    if let Some(project_memory) = v4_project_memory_from_mission(mission) {
        set_doc.insert(
            "project_memory",
            bson::to_bson(&project_memory)
                .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
        );
    }
    if let Some(artifact_memory) = v4_artifact_memory_from_mission(mission) {
        set_doc.insert(
            "artifact_memory",
            bson::to_bson(&artifact_memory)
                .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
        );
    }
    if let Some(node_id) = current_node_id.as_deref() {
        set_doc.insert("current_node_id", node_id);
    } else if terminal_status {
        set_doc.insert("current_node_id", bson::Bson::Null);
    }
    if matches!(
        status_override,
        Some(RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled | RunStatus::Paused)
    ) || matches!(
        resolved_status,
        RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled | RunStatus::Paused
    ) {
        set_doc.insert(
            "active_subagents",
            bson::to_bson(&Vec::<String>::new())
                .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?,
        );
        set_doc.insert("last_turn_outcome", bson::Bson::Null);
    }
    let existing_state = service.get_run_state(run_id).await?;
    if existing_state.is_some() {
        service
            .run_states()
            .update_one(doc! { "run_id": run_id }, doc! { "$set": set_doc }, None)
            .await?;
    } else {
        service
            .upsert_run_state(&RunState {
                id: None,
                run_id: run_id.to_string(),
                mission_id: Some(mission.mission_id.clone()),
                task_graph_id: Some(format!("mission:{}:{}", mission.mission_id, run_id)),
                current_node_id: current_node_id.clone(),
                status: resolved_status.clone(),
                lease: mission.execution_lease.as_ref().map(RunLease::from),
                memory: projected_memory.clone(),
                project_memory: v4_project_memory_from_mission(mission),
                artifact_memory: v4_artifact_memory_from_mission(mission),
                active_subagents: Vec::new(),
                last_turn_outcome: None,
                created_at: Some(now),
                updated_at: Some(now),
            })
            .await?;
    }

    let graph_id = format!("mission:{}:{}", mission.mission_id, run_id);
    let desired_graph = {
        let mut graph = super::mission_routes::build_v4_task_graph(mission, run_id);
        if terminal_status {
            graph.current_node_id = None;
        }
        graph
    };
    let existing_graph = service.get_task_graph(&graph_id).await?;
    let graph_shape_changed = existing_graph.as_ref().is_some_and(|graph| {
        let current_ids = graph
            .nodes
            .iter()
            .map(|node| node.task_node_id.as_str())
            .collect::<Vec<_>>();
        let desired_ids = desired_graph
            .nodes
            .iter()
            .map(|node| node.task_node_id.as_str())
            .collect::<Vec<_>>();
        current_ids != desired_ids
            || graph.root_node_id != desired_graph.root_node_id
            || graph.nodes.len() != desired_graph.nodes.len()
    });
    if graph_shape_changed {
        service.upsert_task_graph(&desired_graph).await?;
    } else if existing_graph.is_some() {
        let mut graph_set_doc = doc! {
            "updated_at": now,
        };
        if let Some(node_id) = current_node_id.as_deref() {
            graph_set_doc.insert("current_node_id", node_id);
        } else if terminal_status {
            graph_set_doc.insert("current_node_id", bson::Bson::Null);
        }
        service
            .task_graphs()
            .update_one(
                doc! { "task_graph_id": &graph_id },
                doc! { "$set": graph_set_doc },
                None,
            )
            .await?;
    } else {
        service.upsert_task_graph(&desired_graph).await?;
    }
    let lease = mission.execution_lease.as_ref().map(RunLease::from);
    let memory = projected_memory;
    let _ = service
        .ensure_run_checkpoint_exists(
            run_id,
            Some(&mission.mission_id),
            Some(&graph_id),
            current_node_id.as_deref(),
            resolved_status,
            lease.as_ref(),
            memory.as_ref(),
        )
        .await?;
    Ok(())
}

fn runtime_memory_for_projection(
    mission: &MissionDoc,
    resolved_status: &RunStatus,
    now: bson::DateTime,
) -> Option<RunMemory> {
    if matches!(resolved_status, RunStatus::Completed) {
        let done = mission
            .progress_memory
            .as_ref()
            .map(|memory| normalize_concrete_deliverable_paths(&memory.done))
            .filter(|done| !done.is_empty())
            .or_else(|| {
                mission
                    .delivery_manifest
                    .as_ref()
                    .map(|manifest| normalize_concrete_deliverable_paths(&manifest.satisfied_deliverables))
                    .filter(|done| !done.is_empty())
            })
            .unwrap_or_default();
        return Some(RunMemory {
            done,
            missing: Vec::new(),
            blocked_by: None,
            last_failed_attempt: None,
            next_best_action: None,
            confidence: None,
            updated_at: Some(now),
        });
    }
    mission.progress_memory.as_ref().map(RunMemory::from)
}

fn v4_progress_memory_from_run_memory(
    memory: &RunMemory,
    done: &[String],
    missing: &[String],
) -> MissionProgressMemory {
    MissionProgressMemory {
        done: done.to_vec(),
        missing: missing.to_vec(),
        blocked_by: memory.blocked_by.clone(),
        last_failed_attempt: memory.last_failed_attempt.clone(),
        next_best_action: memory.next_best_action.clone(),
        confidence: memory.confidence,
        updated_at: memory.updated_at,
    }
}

fn normalize_runtime_projection_paths(items: &[String]) -> Vec<String> {
    let collapsed = items
        .iter()
        .map(|item| collapse_repeated_runtime_projection_path(item))
        .collect::<Vec<_>>();
    normalize_concrete_deliverable_paths(&collapsed)
}

fn collapse_repeated_runtime_projection_path(value: &str) -> String {
    let normalized = value.trim().replace('\\', "/").trim_matches('/').to_string();
    let mut components = normalized
        .split('/')
        .filter(|segment| !segment.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    while components.len() >= 2 && components.len() % 2 == 0 {
        let mid = components.len() / 2;
        if components[..mid] == components[mid..] {
            components.truncate(mid);
            continue;
        }
        break;
    }
    components.join("/")
}

fn runtime_projection_ready_for_settle(mission: &MissionDoc) -> bool {
    if !matches!(
        mission.status,
        MissionStatus::Running
            | MissionStatus::Planning
            | MissionStatus::Paused
            | MissionStatus::Failed
            | MissionStatus::Cancelled
    ) {
        return false;
    }
    let progress_ready = mission
        .progress_memory
        .as_ref()
        .is_some_and(|memory| !memory.done.is_empty() && memory.missing.is_empty());
    let manifest_ready = mission
        .delivery_manifest
        .as_ref()
        .is_some_and(|manifest| {
            !manifest.requested_deliverables.is_empty()
                && manifest.missing_core_deliverables.is_empty()
                && !manifest.satisfied_deliverables.is_empty()
        });
    progress_ready || manifest_ready
}

async fn runtime_projection_needs_terminal_sync(
    service: &AgentService,
    mission: &MissionDoc,
) -> Result<bool, mongodb::error::Error> {
    if !matches!(
        mission.status,
        MissionStatus::Completed | MissionStatus::Failed | MissionStatus::Cancelled
    ) {
        return Ok(false);
    }
    let Some(run_id) = mission.current_run_id.as_deref() else {
        return Ok(false);
    };
    let Some(run_state) = service.get_run_state(run_id).await? else {
        return Ok(true);
    };
    let desired_status = match mission.status {
        MissionStatus::Completed => RunStatus::Completed,
        MissionStatus::Failed => RunStatus::Failed,
        MissionStatus::Cancelled => RunStatus::Cancelled,
        _ => return Ok(false),
    };
    if run_state.status != desired_status {
        return Ok(true);
    }
    if run_state.current_node_id.is_some() {
        return Ok(true);
    }
    let graph_id = format!("mission:{}:{}", mission.mission_id, run_id);
    if service
        .get_task_graph(&graph_id)
        .await?
        .is_some_and(|graph| graph.current_node_id.is_some())
    {
        return Ok(true);
    }
    if desired_status == RunStatus::Completed {
        let Some(memory) = run_state.memory.as_ref() else {
            return Ok(true);
        };
        return Ok(
            !memory.missing.is_empty()
                || memory.blocked_by.is_some()
                || memory.last_failed_attempt.is_some()
                || memory.next_best_action.is_some()
                || run_state.last_turn_outcome.is_some()
                || !run_state.active_subagents.is_empty()
        );
    }
    Ok(
        run_state.last_turn_outcome.is_some()
            || !run_state.active_subagents.is_empty()
    )
}

pub(crate) fn mission_inactive_for_resume(mission: &MissionDoc, stale_before: DateTime<Utc>) -> bool {
    if !matches!(
        mission.status,
        MissionStatus::Running | MissionStatus::Planning
    ) {
        return false;
    }
    if mission_should_skip_auto_resume(mission) {
        return false;
    }
    if execution_lease_active(mission.execution_lease.as_ref()) {
        return false;
    }

    let last_observed_at = current_step_last_observed_at(mission)
        .or_else(|| current_goal_last_observed_at(mission))
        .or_else(|| {
            mission
                .progress_memory
                .as_ref()
                .and_then(|memory| memory.updated_at)
                .map(|ts| ts.to_chrono())
        })
        .or_else(|| {
            mission
                .latest_worker_state
                .as_ref()
                .and_then(|state| state.recorded_at)
                .map(|ts| ts.to_chrono())
        })
        .unwrap_or_else(|| mission.updated_at.to_chrono());
    last_observed_at < stale_before
}

fn mission_waiting_external_reason(mission: &MissionDoc) -> Option<&str> {
    mission
        .completion_assessment
        .as_ref()
        .and_then(|assessment| assessment.reason.as_deref())
        .or_else(|| {
            mission
                .latest_worker_state
                .as_ref()
                .and_then(|state| state.current_blocker.as_deref())
        })
        .or_else(|| {
            mission
                .pending_monitor_intervention
                .as_ref()
                .and_then(|intervention| intervention.feedback.as_deref())
        })
        .or_else(|| {
            mission
                .last_applied_monitor_intervention
                .as_ref()
                .and_then(|intervention| intervention.feedback.as_deref())
        })
        .or_else(|| {
            mission
                .pending_monitor_intervention
                .as_ref()
                .and_then(|intervention| intervention.observed_evidence.first())
                .map(|text| text.as_str())
        })
        .or_else(|| {
            mission
                .last_applied_monitor_intervention
                .as_ref()
                .and_then(|intervention| intervention.observed_evidence.first())
                .map(|text| text.as_str())
        })
        .or(mission.error_message.as_deref())
}

pub fn mission_should_skip_auto_resume(mission: &MissionDoc) -> bool {
    let has_waiting_external_state = mission.waiting_external_until.is_some()
        || mission
            .delivery_state
            .as_ref()
            .is_some_and(|state| *state == MissionDeliveryState::WaitingExternal)
        || mission
            .completion_assessment
            .as_ref()
            .is_some_and(|assessment| {
                matches!(
                    assessment.disposition,
                    super::mission_mongo::MissionCompletionDisposition::WaitingExternal
                )
            });

    if !has_waiting_external_state {
        return false;
    }

    if mission
        .waiting_external_until
        .as_ref()
        .is_some_and(|waiting_until| waiting_until.to_chrono() > Utc::now())
    {
        return true;
    }

    mission_waiting_external_reason(mission)
        .is_some_and(runtime::is_non_retryable_external_provider_message)
}

fn paused_for_auto_resume_update(now: bson::DateTime, reason: &str) -> Document {
    doc! {
        "$set": {
            "status": "paused",
            "updated_at": now,
            "error_message": reason,
        },
        "$unset": {
            "completed_at": "",
            "current_run_id": "",
            "execution_lease": "",
        }
    }
}

fn current_step_last_observed_at(mission: &MissionDoc) -> Option<DateTime<Utc>> {
    let step_index = mission.current_step? as usize;
    let step = mission.steps.get(step_index)?;
    [
        step.last_progress_at.as_ref().map(|dt| dt.to_chrono()),
        step.last_activity_at.as_ref().map(|dt| dt.to_chrono()),
        step.started_at.as_ref().map(|dt| dt.to_chrono()),
    ]
    .into_iter()
    .flatten()
    .max()
}

fn current_goal_last_observed_at(mission: &MissionDoc) -> Option<DateTime<Utc>> {
    let goals = mission.goal_tree.as_ref()?;
    let goal = mission
        .current_goal_id
        .as_deref()
        .and_then(|goal_id| goals.iter().find(|g| g.goal_id == goal_id))
        .or_else(|| {
            goals
                .iter()
                .find(|g| matches!(g.status, GoalStatus::Running))
        })?;

    [
        goal.last_progress_at.as_ref().map(|dt| dt.to_chrono()),
        goal.last_activity_at.as_ref().map(|dt| dt.to_chrono()),
        goal.started_at.as_ref().map(|dt| dt.to_chrono()),
        goal.created_at.as_ref().map(|dt| dt.to_chrono()),
    ]
    .into_iter()
    .flatten()
    .max()
}
