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
use serde::Serialize;
use std::convert::Infallible;
use std::collections::{BTreeMap, BTreeSet};
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
use super::harness_core::{
    ArtifactMemory, HarnessDelegationMode, ProjectMemory, RunCheckpoint, RunCheckpointKind,
    RunLease, RunMemory, RunState, RunStatus, TaskEdge, TaskGraph, TaskNode,
};
use super::mission_manager::MissionManager;
use super::mission_mongo::{
    resolve_execution_profile, ArtifactType, CreateFromChatRequest, CreateMissionRequest,
    GoalActionRequest, GoalStatus, ListMissionsQuery, MissionStatus, MonitorActionRequest,
    StepActionRequest, StepStatus,
};
use super::mission_mongo::{MissionArtifactDoc, MissionDoc};
use super::mission_mongo::{resolve_launch_policy, LaunchPolicy};
use super::mission_monitor::{
    build_monitor_feedback, build_monitor_snapshot, effective_completion_assessment,
    execute_monitor_action,
    normalize_monitor_action,
};
use super::mission_mongo::normalize_concrete_deliverable_paths;
use super::runtime::{self, infer_current_step_index};
use super::service_mongo::{AgentService, ServiceError, ValidationError};
use super::task_manager::StreamEvent;

type MissionState = (Arc<AgentService>, Arc<MongoDb>, Arc<MissionManager>, String);

fn create_mission_error_response(
    err: &ServiceError,
) -> (StatusCode, Json<serde_json::Value>) {
    let message = err.to_string();
    let (status, error_code) = match err {
        ServiceError::Validation(ValidationError::MissionAgentSelection) => {
            (StatusCode::BAD_REQUEST, "invalid_mission_agent")
        }
        ServiceError::Validation(_) => (StatusCode::BAD_REQUEST, "validation_error"),
        _ if message.contains("V4 strategy resolution") => {
            (StatusCode::SERVICE_UNAVAILABLE, "v4_strategy_resolution_failed")
        }
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "mission_create_failed"),
    };
    (
        status,
        Json(serde_json::json!({
            "error": error_code,
            "message": message,
        })),
    )
}

fn v4_node_target_artifacts_from_step(step: &super::mission_mongo::MissionStep) -> Vec<String> {
    let contract_required = step
        .runtime_contract
        .as_ref()
        .map(|contract| contract.required_artifacts.clone())
        .unwrap_or_default();
    let mut targets = normalize_concrete_deliverable_paths(&contract_required);
    if targets.is_empty() {
        targets = normalize_concrete_deliverable_paths(&step.required_artifacts);
    }
    targets
}

fn v4_node_target_artifacts_from_goal(goal: &super::mission_mongo::GoalNode) -> Vec<String> {
    goal.runtime_contract
        .as_ref()
        .map(|contract| normalize_concrete_deliverable_paths(&contract.required_artifacts))
        .unwrap_or_default()
}

fn v4_node_write_scope(target_artifacts: &[String], result_contract: &[String]) -> Vec<String> {
    let targets = normalize_concrete_deliverable_paths(target_artifacts);
    if !targets.is_empty() {
        return targets;
    }
    normalize_concrete_deliverable_paths(result_contract)
}

fn v4_node_delegation_mode(
    launch_policy: &LaunchPolicy,
    allow_swarm: bool,
    allow_subagent: bool,
) -> Option<HarnessDelegationMode> {
    match launch_policy {
        LaunchPolicy::SingleWorker | LaunchPolicy::GuidedCheckpoint => None,
        LaunchPolicy::SubagentFirst | LaunchPolicy::RecoveryFirst => {
            allow_subagent.then_some(HarnessDelegationMode::Subagent)
        }
        LaunchPolicy::SwarmFirst | LaunchPolicy::Auto => {
            if allow_swarm {
                Some(HarnessDelegationMode::Swarm)
            } else if allow_subagent {
                Some(HarnessDelegationMode::Subagent)
            } else {
                None
            }
        }
    }
}

fn v4_node_swarm_mode(
    launch_policy: &LaunchPolicy,
    delegation_mode: Option<&HarnessDelegationMode>,
) -> Option<super::harness_core::HarnessSwarmMode> {
    match (launch_policy, delegation_mode) {
        (
            LaunchPolicy::SwarmFirst | LaunchPolicy::Auto,
            Some(HarnessDelegationMode::Swarm),
        ) => Some(super::harness_core::HarnessSwarmMode::RecursiveOrchestrate),
        _ => None,
    }
}

pub(crate) fn build_v4_task_graph(mission: &MissionDoc, run_id: &str) -> TaskGraph {
    let task_graph_id = format!("mission:{}:{}", mission.mission_id, run_id);
    let launch_policy = resolve_launch_policy(mission);
    let terminal = matches!(
        mission.status,
        MissionStatus::Completed | MissionStatus::Failed | MissionStatus::Cancelled
    );
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    if mission.execution_mode == super::mission_mongo::ExecutionMode::Sequential
        && !mission.steps.is_empty()
    {
        let ordered_steps = mission
            .steps
            .iter()
            .map(|step| {
                let node_id = format!("step:{}", step.index);
                let target_artifacts = v4_node_target_artifacts_from_step(step);
                let result_contract = target_artifacts.clone();
                let delegation_mode = v4_node_delegation_mode(
                    &launch_policy,
                    matches!(launch_policy, LaunchPolicy::SwarmFirst),
                    step.use_subagent
                        || matches!(
                            launch_policy,
                            LaunchPolicy::SubagentFirst | LaunchPolicy::RecoveryFirst
                        ),
                );
                TaskNode {
                    task_node_id: node_id,
                    title: Some(step.title.clone()),
                    mode: super::harness_core::HarnessTurnMode::Execute,
                    target_artifacts: target_artifacts.clone(),
                    input_artifacts: Vec::new(),
                    delegation_mode: delegation_mode.clone(),
                    parallelism_budget: match launch_policy {
                        LaunchPolicy::SwarmFirst => Some(2),
                        LaunchPolicy::SubagentFirst | LaunchPolicy::RecoveryFirst => Some(1),
                        LaunchPolicy::SingleWorker | LaunchPolicy::GuidedCheckpoint => None,
                        _ => None,
                    },
                    swarm_mode: v4_node_swarm_mode(&launch_policy, delegation_mode.as_ref()),
                    swarm_budget: match launch_policy {
                        LaunchPolicy::SwarmFirst => Some(2),
                        _ => None,
                    },
                    write_scope: v4_node_write_scope(&target_artifacts, &result_contract),
                    result_contract,
                }
            })
            .collect::<Vec<_>>();
        for pair in ordered_steps.windows(2) {
            edges.push(TaskEdge {
                from_node_id: pair[0].task_node_id.clone(),
                to_node_id: pair[1].task_node_id.clone(),
                condition_label: Some("step_complete".to_string()),
            });
        }
        let current_node_id = if terminal {
            None
        } else {
            mission
                .current_step
                .map(|index| format!("step:{}", index))
                .or_else(|| ordered_steps.first().map(|node| node.task_node_id.clone()))
        };
        let root_node_id = ordered_steps
            .first()
            .map(|node| node.task_node_id.clone())
            .unwrap_or_else(|| "mission:root".to_string());
        nodes = ordered_steps;
        return TaskGraph {
            id: None,
            task_graph_id,
            mission_id: Some(mission.mission_id.clone()),
            run_id: Some(run_id.to_string()),
            root_node_id,
            current_node_id,
            nodes,
            edges,
            graph_version: 1,
            created_at: Some(bson::DateTime::now()),
            updated_at: Some(bson::DateTime::now()),
        };
    }

    if let Some(goals) = mission.goal_tree.as_ref() {
        let ordered_goals = goals
            .iter()
            .map(|goal| {
                let target_artifacts = v4_node_target_artifacts_from_goal(goal);
                let result_contract = target_artifacts.clone();
                let delegation_mode = v4_node_delegation_mode(
                    &launch_policy,
                    matches!(launch_policy, LaunchPolicy::SwarmFirst | LaunchPolicy::Auto),
                    !matches!(
                        launch_policy,
                        LaunchPolicy::SingleWorker | LaunchPolicy::GuidedCheckpoint
                    ),
                );
                TaskNode {
                    task_node_id: format!("goal:{}", goal.goal_id),
                    title: Some(goal.title.clone()),
                    mode: super::harness_core::HarnessTurnMode::Execute,
                    target_artifacts: target_artifacts.clone(),
                    input_artifacts: Vec::new(),
                    delegation_mode: delegation_mode.clone(),
                    parallelism_budget: match launch_policy {
                        LaunchPolicy::SingleWorker | LaunchPolicy::GuidedCheckpoint => None,
                        LaunchPolicy::RecoveryFirst => Some(1),
                        LaunchPolicy::SubagentFirst => Some(1),
                        LaunchPolicy::SwarmFirst | LaunchPolicy::Auto => Some(3),
                    },
                    swarm_mode: v4_node_swarm_mode(&launch_policy, delegation_mode.as_ref()),
                    swarm_budget: match launch_policy {
                        LaunchPolicy::SingleWorker | LaunchPolicy::GuidedCheckpoint => None,
                        LaunchPolicy::RecoveryFirst => Some(1),
                        LaunchPolicy::SubagentFirst => None,
                        LaunchPolicy::SwarmFirst | LaunchPolicy::Auto => Some(3),
                    },
                    write_scope: v4_node_write_scope(&target_artifacts, &result_contract),
                    result_contract,
                }
            })
            .collect::<Vec<_>>();
        for pair in ordered_goals.windows(2) {
            edges.push(TaskEdge {
                from_node_id: pair[0].task_node_id.clone(),
                to_node_id: pair[1].task_node_id.clone(),
                condition_label: Some("goal_complete".to_string()),
            });
        }
        let current_node_id = if terminal {
            None
        } else {
            mission
                .current_goal_id
                .as_ref()
                .map(|goal_id| format!("goal:{}", goal_id))
                .or_else(|| ordered_goals.first().map(|node| node.task_node_id.clone()))
        };
        let root_node_id = ordered_goals
            .first()
            .map(|node| node.task_node_id.clone())
            .unwrap_or_else(|| "mission:root".to_string());
        nodes = ordered_goals;
        return TaskGraph {
            id: None,
            task_graph_id,
            mission_id: Some(mission.mission_id.clone()),
            run_id: Some(run_id.to_string()),
            root_node_id,
            current_node_id,
            nodes,
            edges,
            graph_version: 1,
            created_at: Some(bson::DateTime::now()),
            updated_at: Some(bson::DateTime::now()),
        };
    }

    let root_node_id = "mission:root".to_string();
    let root_target_artifacts = mission
        .delivery_manifest
        .as_ref()
        .map(|manifest| normalize_concrete_deliverable_paths(&manifest.requested_deliverables))
        .unwrap_or_default();
    let root_result_contract = root_target_artifacts.clone();
    let root_delegation_mode = v4_node_delegation_mode(
        &launch_policy,
        matches!(launch_policy, LaunchPolicy::SwarmFirst | LaunchPolicy::Auto),
        !matches!(
            launch_policy,
            LaunchPolicy::SingleWorker | LaunchPolicy::GuidedCheckpoint
        ),
    );
    nodes.push(TaskNode {
        task_node_id: root_node_id.clone(),
        title: Some(mission.goal.clone()),
        mode: super::harness_core::HarnessTurnMode::Execute,
        target_artifacts: root_target_artifacts.clone(),
        input_artifacts: Vec::new(),
        delegation_mode: root_delegation_mode.clone(),
        parallelism_budget: match launch_policy {
            LaunchPolicy::SingleWorker | LaunchPolicy::GuidedCheckpoint => None,
            LaunchPolicy::RecoveryFirst => Some(1),
            LaunchPolicy::SubagentFirst => Some(1),
            LaunchPolicy::SwarmFirst | LaunchPolicy::Auto => Some(3),
        },
        swarm_mode: v4_node_swarm_mode(&launch_policy, root_delegation_mode.as_ref()),
        swarm_budget: match launch_policy {
            LaunchPolicy::SingleWorker | LaunchPolicy::GuidedCheckpoint => None,
            LaunchPolicy::RecoveryFirst => Some(1),
            LaunchPolicy::SubagentFirst => None,
            LaunchPolicy::SwarmFirst | LaunchPolicy::Auto => Some(3),
        },
        write_scope: v4_node_write_scope(&root_target_artifacts, &root_result_contract),
        result_contract: root_result_contract,
    });
    TaskGraph {
        id: None,
        task_graph_id,
        mission_id: Some(mission.mission_id.clone()),
        run_id: Some(run_id.to_string()),
        root_node_id: root_node_id.clone(),
        current_node_id: if terminal { None } else { Some(root_node_id) },
        nodes,
        edges,
        graph_version: 1,
        created_at: Some(bson::DateTime::now()),
        updated_at: Some(bson::DateTime::now()),
    }
}

fn build_v4_project_memory(mission: &MissionDoc) -> Option<ProjectMemory> {
    let mut assumptions = Vec::new();
    let mut constraints = Vec::new();
    let mut preferences = Vec::new();

    assumptions.push(format!("execution_mode={}", serde_json::to_string(&mission.execution_mode).unwrap_or_default().trim_matches('"')));
    assumptions.push(format!("execution_profile={}", serde_json::to_string(&resolve_execution_profile(mission)).unwrap_or_default().trim_matches('"')));
    assumptions.push(format!("launch_policy={}", serde_json::to_string(&resolve_launch_policy(mission)).unwrap_or_default().trim_matches('"')));

    if let Some(context) = mission.context.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        constraints.push(context.to_string());
    }
    if mission.approval_policy != super::mission_mongo::ApprovalPolicy::Auto {
        constraints.push(format!("approval_policy={}", serde_json::to_string(&mission.approval_policy).unwrap_or_default().trim_matches('"')));
    }
    if mission.step_timeout_seconds.is_some() {
        constraints.push(format!(
            "step_timeout_seconds={}",
            mission.step_timeout_seconds.unwrap_or_default()
        ));
    }
    if mission.step_max_retries.is_some() {
        constraints.push(format!(
            "step_max_retries={}",
            mission.step_max_retries.unwrap_or_default()
        ));
    }
    if !mission.attached_document_ids.is_empty() {
        preferences.push(format!(
            "attached_document_ids={}",
            mission.attached_document_ids.join(",")
        ));
    }
    if let Some(workspace_path) = mission.workspace_path.as_deref() {
        preferences.push(format!("workspace_path={workspace_path}"));
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

fn build_v4_artifact_memory(mission: &MissionDoc) -> Option<ArtifactMemory> {
    let known_artifacts = mission
        .delivery_manifest
        .as_ref()
        .map(|manifest| {
            let mut artifacts = manifest.satisfied_deliverables.clone();
            artifacts.extend(manifest.supporting_artifacts.clone());
            normalize_concrete_deliverable_paths(&artifacts)
        })
        .or_else(|| {
            mission
                .progress_memory
                .as_ref()
                .map(|memory| normalize_concrete_deliverable_paths(&memory.done))
        })
        .unwrap_or_default();

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

pub(crate) async fn initialize_v4_run_state(
    service: &AgentService,
    mission: &MissionDoc,
    run_id: &str,
) -> Result<(), StatusCode> {
    debug_assert_eq!(
        mission.harness_version,
        super::mission_mongo::MissionHarnessVersion::V4
    );
    let graph = build_v4_task_graph(mission, run_id);
    let run_state = RunState {
        id: None,
        run_id: run_id.to_string(),
        mission_id: Some(mission.mission_id.clone()),
        task_graph_id: Some(graph.task_graph_id.clone()),
        current_node_id: graph.current_node_id.clone(),
        status: RunStatus::Executing,
        lease: mission.execution_lease.as_ref().map(RunLease::from),
        memory: mission.progress_memory.as_ref().map(RunMemory::from),
        project_memory: build_v4_project_memory(mission),
        artifact_memory: build_v4_artifact_memory(mission),
        active_subagents: Vec::new(),
        last_turn_outcome: None,
        created_at: Some(bson::DateTime::now()),
        updated_at: Some(bson::DateTime::now()),
    };
    let checkpoint = RunCheckpoint {
        id: None,
        run_id: run_id.to_string(),
        mission_id: Some(mission.mission_id.clone()),
        task_graph_id: Some(graph.task_graph_id.clone()),
        current_node_id: graph.current_node_id.clone(),
        checkpoint_kind: RunCheckpointKind::NodeStart,
        status: RunStatus::Executing,
        lease: run_state.lease.clone(),
        memory: run_state.memory.clone(),
        last_turn_outcome: None,
        created_at: Some(bson::DateTime::now()),
    };
    service
        .upsert_task_graph(&graph)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    service
        .upsert_run_state(&run_state)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    service
        .save_run_checkpoint(&checkpoint)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(())
}

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

#[derive(Serialize)]
struct MissionEventAuditMoment {
    event_id: i64,
    event_type: String,
    summary: String,
    created_at: String,
}

#[derive(Serialize)]
struct MissionEventAuditSummary {
    mission_id: String,
    run_id: Option<String>,
    total_events: usize,
    counts_by_type: BTreeMap<String, usize>,
    key_moments: Vec<MissionEventAuditMoment>,
    first_event_at: Option<String>,
    last_event_at: Option<String>,
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
fn mission_to_json(mission: &MissionDoc, artifacts: &[MissionArtifactDoc]) -> serde_json::Value {
    let mut val = serde_json::to_value(mission).unwrap_or_default();
    fix_bson_dates(&mut val);
    let effective_assessment = effective_completion_assessment(mission);
    let delivery_manifest = build_delivery_manifest(mission, artifacts, effective_assessment.as_ref());
    let required_output_hints = manifest_requested_output_hints(mission);
    let satisfied_output_hints = manifest_satisfied_output_hints(mission);
    let artifact_values: Vec<serde_json::Value> = artifacts
        .iter()
        .map(|artifact| artifact_to_json(artifact, &required_output_hints, &satisfied_output_hints))
        .collect();
    // Remove internal MongoDB _id field
    if let Some(obj) = val.as_object_mut() {
        obj.remove("_id");
        obj.remove("execution_mode");
        obj.remove("execution_profile");
        obj.remove("launch_policy");
        match effective_assessment.as_ref() {
            Some(assessment) => {
                obj.insert(
                    "completion_assessment".to_string(),
                    serde_json::to_value(assessment).unwrap_or(serde_json::Value::Null),
                );
            }
            None => {
            }
        }
        obj.insert(
            "delivery_state".to_string(),
            delivery_manifest
                .get("delivery_state")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "delivery_manifest".to_string(),
            delivery_manifest.clone(),
        );
        obj.insert(
            "requested_deliverables".to_string(),
            delivery_manifest
                .get("requested_deliverables")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        );
        obj.insert(
            "missing_core_deliverables".to_string(),
            delivery_manifest
                .get("missing_core_deliverables")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        );
        obj.insert(
            "artifacts".to_string(),
            serde_json::Value::Array(artifact_values),
        );
        obj.insert(
            "retry_after".to_string(),
            mission
                .waiting_external_until
                .map(|ts| serde_json::json!(ts.to_chrono().to_rfc3339()))
                .unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "current_step".to_string(),
            infer_current_step(mission)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
        );
        let goals = mission.goal_tree.as_deref().unwrap_or(&[]);
        obj.insert("goal_count".to_string(), serde_json::json!(goals.len()));
        obj.insert(
            "completed_goals".to_string(),
            serde_json::json!(goals
                .iter()
                .filter(|goal| goal.status == GoalStatus::Completed)
                .count()),
        );
        if let Some(current_goal_id) = mission.current_goal_id.as_deref() {
            if let Some(goal) = goals.iter().find(|goal| goal.goal_id == current_goal_id) {
                obj.insert(
                    "goal_last_activity_at".to_string(),
                    goal.last_activity_at
                        .map(|ts| serde_json::json!(ts.to_chrono().to_rfc3339()))
                        .unwrap_or(serde_json::Value::Null),
                );
                obj.insert(
                    "goal_last_progress_at".to_string(),
                    goal.last_progress_at
                        .map(|ts| serde_json::json!(ts.to_chrono().to_rfc3339()))
                        .unwrap_or(serde_json::Value::Null),
                );
            }
        }
    }
    val
}

fn ordered_unique_paths<'a>(paths: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for path in paths {
        if let Some(normalized) = normalize_artifact_hint(path) {
            if seen.insert(normalized.clone()) {
                ordered.push(normalized);
            }
        }
    }
    ordered
}

fn collect_missing_core_deliverables(
    requested_deliverables: &[String],
    artifacts: &[MissionArtifactDoc],
) -> Vec<String> {
    let mut missing = BTreeSet::new();
    let artifact_hints: Vec<BTreeSet<String>> = artifacts.iter().map(artifact_hint_candidates).collect();
    for requested in requested_deliverables {
        let Some(normalized_requested) = normalize_artifact_hint(requested) else {
            missing.insert(requested.clone());
            continue;
        };
        let matched = artifact_hints
            .iter()
            .any(|candidates| candidates.contains(&normalized_requested));
        if !matched {
            missing.insert(requested.clone());
        }
    }
    missing.into_iter().collect()
}

fn route_deliverable_paths_overlap(left: &str, right: &str) -> bool {
    let left_candidates = artifact_hint_candidates_from_value(left);
    let right_candidates = artifact_hint_candidates_from_value(right);
    left_candidates
        .iter()
        .any(|candidate| right_candidates.iter().any(|other| other == candidate))
}

fn mission_delivery_state(
    mission: &MissionDoc,
    requested_deliverables: &[String],
    missing_core_deliverables: &[String],
    effective_assessment: Option<&super::mission_mongo::MissionCompletionAssessment>,
) -> &'static str {
    if mission.status == MissionStatus::Completed {
        if let Some(assessment) = effective_assessment {
            return match assessment.disposition {
                super::mission_mongo::MissionCompletionDisposition::Complete => "complete",
                super::mission_mongo::MissionCompletionDisposition::CompletedWithMinorGaps => {
                    "completed_with_minor_gaps"
                }
                super::mission_mongo::MissionCompletionDisposition::PartialHandoff => {
                    "partial_handoff"
                }
                super::mission_mongo::MissionCompletionDisposition::BlockedByEnvironment => {
                    "blocked_by_environment"
                }
                super::mission_mongo::MissionCompletionDisposition::BlockedByTooling => {
                    "blocked_by_tooling"
                }
                super::mission_mongo::MissionCompletionDisposition::WaitingExternal => {
                    "waiting_external"
                }
                super::mission_mongo::MissionCompletionDisposition::BlockedFail => "blocked_fail",
            };
        }
    }
    if let Some(state) = &mission.delivery_state {
        return match state {
            super::mission_mongo::MissionDeliveryState::Working => "working",
            super::mission_mongo::MissionDeliveryState::RepairingDeliverables => {
                "repairing_deliverables"
            }
            super::mission_mongo::MissionDeliveryState::RepairingContract => "repairing_contract",
            super::mission_mongo::MissionDeliveryState::Replanning => "replanning",
            super::mission_mongo::MissionDeliveryState::WaitingExternal => "waiting_external",
            super::mission_mongo::MissionDeliveryState::BlockedByEnvironment => {
                "blocked_by_environment"
            }
            super::mission_mongo::MissionDeliveryState::BlockedByTooling => {
                "blocked_by_tooling"
            }
            super::mission_mongo::MissionDeliveryState::PartialHandoffCandidate => {
                "partial_handoff_candidate"
            }
            super::mission_mongo::MissionDeliveryState::ReadyToComplete => "ready_to_complete",
            super::mission_mongo::MissionDeliveryState::Complete => "complete",
            super::mission_mongo::MissionDeliveryState::CompletedWithMinorGaps => {
                "completed_with_minor_gaps"
            }
            super::mission_mongo::MissionDeliveryState::PartialHandoff => "partial_handoff",
        };
    }

    if let Some(assessment) = effective_assessment {
        return match assessment.disposition {
            super::mission_mongo::MissionCompletionDisposition::Complete => "complete",
            super::mission_mongo::MissionCompletionDisposition::CompletedWithMinorGaps => {
                "completed_with_minor_gaps"
            }
            super::mission_mongo::MissionCompletionDisposition::PartialHandoff => "partial_handoff",
            super::mission_mongo::MissionCompletionDisposition::BlockedByEnvironment => {
                "blocked_by_environment"
            }
            super::mission_mongo::MissionCompletionDisposition::BlockedByTooling => {
                "blocked_by_tooling"
            }
            super::mission_mongo::MissionCompletionDisposition::WaitingExternal => {
                "waiting_external"
            }
            super::mission_mongo::MissionCompletionDisposition::BlockedFail => "blocked_fail",
        };
    }

    if mission.waiting_external_until.is_some() {
        return "waiting_external";
    }

    match mission.delivery_state {
        Some(super::mission_mongo::MissionDeliveryState::BlockedByEnvironment) => {
            "blocked_by_environment"
        }
        Some(super::mission_mongo::MissionDeliveryState::BlockedByTooling) => {
            "blocked_by_tooling"
        }
        Some(super::mission_mongo::MissionDeliveryState::PartialHandoff) => {
            "partial_handoff_candidate"
        }
        Some(super::mission_mongo::MissionDeliveryState::RepairingDeliverables) => {
            "repairing_deliverables"
        }
        Some(super::mission_mongo::MissionDeliveryState::RepairingContract) => {
            "repairing_contract"
        }
        Some(super::mission_mongo::MissionDeliveryState::Replanning) => "replanning",
        _ if !requested_deliverables.is_empty() && missing_core_deliverables.is_empty() => {
            "ready_to_complete"
        }
        _ => "working",
    }
}

fn build_delivery_manifest(
    mission: &MissionDoc,
    artifacts: &[MissionArtifactDoc],
    effective_assessment: Option<&super::mission_mongo::MissionCompletionAssessment>,
) -> serde_json::Value {
    let requested_deliverables = mission
        .delivery_manifest
        .as_ref()
        .filter(|manifest| !manifest.requested_deliverables.is_empty())
        .map(|manifest| normalize_concrete_deliverable_paths(&manifest.requested_deliverables))
        .unwrap_or_default();
    let required_output_hints = manifest_requested_output_hints(mission);
    let satisfied_output_hints = manifest_satisfied_output_hints(mission);
    let mut satisfied = Vec::new();
    let mut supporting = Vec::new();
    let mut seen_satisfied = BTreeSet::new();
    let artifact_hints: Vec<(Option<String>, BTreeSet<String>, ArtifactDeliveryClassification)> = artifacts
        .iter()
        .map(|artifact| {
            (
                artifact.file_path.clone(),
                artifact_hint_candidates(artifact),
                classify_artifact_delivery(&required_output_hints, &satisfied_output_hints, artifact),
            )
        })
        .collect();

    for (path, _, classification) in &artifact_hints {
        if matches!(classification.role, ArtifactDeliveryRole::SupportingArtifact) {
            if let Some(path) = path {
                supporting.push(path.clone());
            }
        }
    }

    for requested in &requested_deliverables {
        for (path, candidates, classification) in &artifact_hints {
            if matches!(classification.role, ArtifactDeliveryRole::SupportingArtifact) {
                continue;
            }
            if candidates.contains(requested) {
                let value = path.clone().unwrap_or_else(|| requested.clone());
                if seen_satisfied.insert(value.clone()) {
                    satisfied.push(value);
                }
                break;
            }
        }
    }
    if let Some(progress) = mission.progress_memory.as_ref() {
        for done_path in normalize_concrete_deliverable_paths(&progress.done) {
            let done_candidates = artifact_hint_candidates_from_value(&done_path);
            let exists_in_artifacts = artifact_hints.iter().any(|(_, candidates, _)| {
                candidates
                    .iter()
                    .any(|candidate| done_candidates.contains(candidate))
            });
            if exists_in_artifacts && seen_satisfied.insert(done_path.clone()) {
                satisfied.push(done_path);
            }
        }
    }

    let mut missing_core_deliverables = collect_missing_core_deliverables(&requested_deliverables, artifacts);
    if let Some(manifest) = &mission.delivery_manifest {
        if !manifest.missing_core_deliverables.is_empty() {
            missing_core_deliverables = ordered_unique_paths(
                normalize_concrete_deliverable_paths(&manifest.missing_core_deliverables)
                    .iter()
                    .chain(missing_core_deliverables.iter())
                    .map(|value| value.as_str()),
            );
        }
    }
    if !satisfied.is_empty() && !missing_core_deliverables.is_empty() {
        missing_core_deliverables.retain(|path| {
            !satisfied
                .iter()
                .any(|done_path| route_deliverable_paths_overlap(path, done_path))
        });
    }
    if let Some(progress) = mission.progress_memory.as_ref() {
        let done_set = normalize_concrete_deliverable_paths(&progress.done)
            .into_iter()
            .collect::<BTreeSet<_>>();
        if !done_set.is_empty() {
            for path in &done_set {
                if seen_satisfied.insert(path.clone()) {
                    satisfied.push(path.clone());
                }
            }
            missing_core_deliverables.retain(|path| {
                !done_set
                    .iter()
                    .any(|done_path| route_deliverable_paths_overlap(path, done_path))
            });
        }
    }
    if effective_assessment.is_some_and(|assessment| {
        assessment.disposition == super::mission_mongo::MissionCompletionDisposition::Complete
    }) {
        missing_core_deliverables.clear();
    }
    let delivery_state = mission_delivery_state(
        mission,
        &requested_deliverables,
        &missing_core_deliverables,
        effective_assessment,
    );
    let final_outcome_summary = mission
        .delivery_manifest
        .as_ref()
        .and_then(|manifest| manifest.final_outcome_summary.clone())
        .or_else(|| effective_assessment
        .and_then(|assessment| assessment.reason.clone())
        );

    serde_json::json!({
        "requested_deliverables": requested_deliverables,
        "satisfied_deliverables": satisfied,
        "missing_core_deliverables": missing_core_deliverables,
        "supporting_artifacts": supporting,
        "delivery_state": delivery_state,
        "final_outcome_summary": final_outcome_summary,
    })
}

async fn bind_mission_session_if_missing(
    service: &AgentService,
    mission: &MissionDoc,
) -> Result<Option<String>, mongodb::error::Error> {
    if let Some(session_id) = mission.session_id.clone() {
        if service.get_session(&session_id).await?.is_some() {
            if let Some(workspace_path) = mission.workspace_path.as_deref() {
                service.set_session_workspace(&session_id, workspace_path).await?;
            }
            return Ok(Some(session_id));
        }
    }

    let session = service
        .create_chat_session(
            &mission.team_id,
            &mission.agent_id,
            &mission.creator_id,
            mission.attached_document_ids.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            false,
            None,
            Some("mission".to_string()),
            Some(mission.mission_id.clone()),
            Some(true),
        )
        .await?;

    service
        .set_mission_session(&mission.mission_id, &session.session_id)
        .await?;
    if let Some(workspace_path) = mission.workspace_path.as_deref() {
        service
            .set_session_workspace(&session.session_id, workspace_path)
            .await?;
    }
    Ok(Some(session.session_id))
}

fn infer_current_step(mission: &MissionDoc) -> Option<u32> {
    if matches!(
        mission.status,
        MissionStatus::Completed | MissionStatus::Failed | MissionStatus::Cancelled
    ) {
        return None;
    }
    infer_current_step_index(mission)
}

#[derive(Clone, Copy)]
enum ArtifactDeliveryRole {
    CoreDeliverable,
    SupportingArtifact,
}

impl ArtifactDeliveryRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::CoreDeliverable => "core_deliverable",
            Self::SupportingArtifact => "supporting_artifact",
        }
    }
}

struct ArtifactDeliveryClassification {
    role: ArtifactDeliveryRole,
    is_required_output: bool,
    reason: &'static str,
}

fn normalize_artifact_hint(value: &str) -> Option<String> {
    runtime::normalize_relative_workspace_path(value).map(|path| path.to_ascii_lowercase())
}

fn artifact_hint_candidates(artifact: &MissionArtifactDoc) -> BTreeSet<String> {
    let mut hints = BTreeSet::new();
    if let Some(path) = artifact.file_path.as_deref() {
        if let Some(normalized) = normalize_artifact_hint(path) {
            hints.insert(normalized.clone());
            if let Some(name) = normalized.rsplit('/').next() {
                hints.insert(name.to_string());
            }
        }
    }
    let artifact_name = artifact.name.trim().to_ascii_lowercase();
    if !artifact_name.is_empty() {
        hints.insert(artifact_name);
    }
    hints
}

fn artifact_hint_candidates_from_value(value: &str) -> BTreeSet<String> {
    let mut hints = BTreeSet::new();
    if let Some(normalized) = normalize_artifact_hint(value) {
        hints.insert(normalized.clone());
        if let Some(name) = normalized.rsplit('/').next() {
            hints.insert(name.to_string());
        }
    }
    hints
}

fn manifest_requested_output_hints(mission: &MissionDoc) -> BTreeSet<String> {
    let mut hints = BTreeSet::new();
    let mut add_hint = |value: &str| {
        if let Some(normalized) = normalize_artifact_hint(value) {
            hints.insert(normalized.clone());
            if let Some(name) = normalized.rsplit('/').next() {
                hints.insert(name.to_string());
            }
        }
    };
    if let Some(manifest) = &mission.delivery_manifest {
        for value in &manifest.requested_deliverables {
            add_hint(value);
        }
        for value in &manifest.missing_core_deliverables {
            add_hint(value);
        }
    }
    hints
}

fn manifest_satisfied_output_hints(mission: &MissionDoc) -> BTreeSet<String> {
    let mut hints = BTreeSet::new();
    let mut add_hint = |value: &str| {
        if let Some(normalized) = normalize_artifact_hint(value) {
            hints.insert(normalized.clone());
            if let Some(name) = normalized.rsplit('/').next() {
                hints.insert(name.to_string());
            }
        }
    };
    if let Some(manifest) = &mission.delivery_manifest {
        for value in &manifest.satisfied_deliverables {
            add_hint(value);
        }
    }
    hints
}

fn is_low_signal_supporting_path(path: Option<&str>) -> bool {
    let path_lower = path
        .map(|value| value.replace('\\', "/").to_ascii_lowercase())
        .unwrap_or_default();
    !path_lower.is_empty() && runtime::is_low_signal_artifact_path(&path_lower)
}

fn classify_artifact_delivery(
    required_output_hints: &BTreeSet<String>,
    satisfied_output_hints: &BTreeSet<String>,
    artifact: &MissionArtifactDoc,
) -> ArtifactDeliveryClassification {
    if is_low_signal_supporting_path(artifact.file_path.as_deref()) {
        return ArtifactDeliveryClassification {
            role: ArtifactDeliveryRole::SupportingArtifact,
            is_required_output: false,
            reason: "low_signal_path",
        };
    }

    let candidates = artifact_hint_candidates(artifact);
    if candidates
        .iter()
        .any(|candidate| required_output_hints.contains(candidate))
    {
        return ArtifactDeliveryClassification {
            role: ArtifactDeliveryRole::CoreDeliverable,
            is_required_output: true,
            reason: "required_output",
        };
    }

    if candidates
        .iter()
        .any(|candidate| satisfied_output_hints.contains(candidate))
    {
        return ArtifactDeliveryClassification {
            role: ArtifactDeliveryRole::CoreDeliverable,
            is_required_output: false,
            reason: "manifest_satisfied",
        };
    }

    ArtifactDeliveryClassification {
        role: ArtifactDeliveryRole::SupportingArtifact,
        is_required_output: false,
        reason: "non_manifest_artifact",
    }
}

fn artifact_to_json(
    artifact: &MissionArtifactDoc,
    required_output_hints: &BTreeSet<String>,
    satisfied_output_hints: &BTreeSet<String>,
) -> serde_json::Value {
    let classification =
        classify_artifact_delivery(required_output_hints, satisfied_output_hints, artifact);
    let mut val = serde_json::to_value(artifact).unwrap_or_default();
    fix_bson_dates(&mut val);
    if let Some(obj) = val.as_object_mut() {
        obj.remove("_id");
        if let Some(file_path) = obj.get("file_path").cloned() {
            obj.insert("relative_path".to_string(), file_path);
        }
        obj.insert(
            "delivery_role".to_string(),
            serde_json::Value::String(classification.role.as_str().to_string()),
        );
        obj.insert(
            "is_required_output".to_string(),
            serde_json::Value::Bool(classification.is_required_output),
        );
        obj.insert(
            "delivery_role_reason".to_string(),
            serde_json::Value::String(classification.reason.to_string()),
        );
    }
    val
}

fn summarize_status_event(raw: &str) -> Option<String> {
    if raw.contains("mission_planning") {
        return Some("任务规划阶段".to_string());
    }
    if raw.contains("mission_planned") {
        return Some("任务规划完成".to_string());
    }
    if raw.contains("\"type\":\"mission_completed\"") {
        return Some("任务完成".to_string());
    }
    if raw.contains("\"type\":\"mission_failed\"") {
        return Some("任务失败".to_string());
    }
    if raw.contains("\"type\":\"step_started\"") {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw) {
            if let Some(obj) = parsed.as_object() {
                let idx = obj.get("step_index").and_then(|v| v.as_u64()).unwrap_or(0);
                let title = obj
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .unwrap_or("未命名步骤");
                return Some(format!("开始步骤 {}：{}", idx + 1, title));
            }
        }
        return Some("开始顺序步骤".to_string());
    }
    if raw.contains("\"type\":\"step_complete\"") || raw.contains("\"type\":\"step_completed\"") {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw) {
            if let Some(obj) = parsed.as_object() {
                let idx = obj.get("step_index").and_then(|v| v.as_u64()).unwrap_or(0);
                let best_effort = obj
                    .get("best_effort")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                return Some(if best_effort {
                    format!("步骤 {} 完成（best-effort）", idx + 1)
                } else {
                    format!("步骤 {} 完成", idx + 1)
                });
            }
        }
        return Some("顺序步骤完成".to_string());
    }
    if raw.contains("\"type\":\"step_failed\"") {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw) {
            if let Some(obj) = parsed.as_object() {
                let idx = obj.get("step_index").and_then(|v| v.as_u64()).unwrap_or(0);
                let reason = obj
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .unwrap_or("unknown");
                return Some(format!("步骤 {} 失败：{}", idx + 1, reason));
            }
        }
        return Some("顺序步骤失败".to_string());
    }
    if raw.contains("\"type\":\"step_supervision\"") && !raw.contains("\"action\":\"continue\"") {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw) {
            if let Some(obj) = parsed.as_object() {
                let idx = obj.get("step_index").and_then(|v| v.as_u64()).unwrap_or(0);
                let action = obj
                    .get("action")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .unwrap_or("unknown");
                return Some(format!("步骤 {} 进入策略切换：{}", idx + 1, action));
            }
        }
        return Some("顺序步骤进入策略切换".to_string());
    }

    let parsed = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let obj = parsed.as_object()?;
    let event_type = obj.get("type").and_then(|v| v.as_str())?;

    match event_type {
        "step_started" => {
            let idx = obj.get("step_index").and_then(|v| v.as_u64())?;
            let title = obj
                .get("title")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .unwrap_or("未命名步骤");
            Some(format!("开始步骤 {}：{}", idx + 1, title))
        }
        "step_complete" | "step_completed" => {
            let idx = obj.get("step_index").and_then(|v| v.as_u64())?;
            let best_effort = obj
                .get("best_effort")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Some(if best_effort {
                format!("步骤 {} 完成（best-effort）", idx + 1)
            } else {
                format!("步骤 {} 完成", idx + 1)
            })
        }
        "step_failed" => {
            let idx = obj.get("step_index").and_then(|v| v.as_u64())?;
            let reason = obj
                .get("reason")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .unwrap_or("unknown");
            Some(format!("步骤 {} 失败：{}", idx + 1, reason))
        }
        "step_supervision" => {
            let action = obj
                .get("action")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            if action.is_empty() || action == "continue" {
                return None;
            }
            let idx = obj.get("step_index").and_then(|v| v.as_u64())?;
            Some(format!("步骤 {} 进入策略切换：{}", idx + 1, action))
        }
        "mission_completed" => Some("任务完成".to_string()),
        "mission_failed" => Some("任务失败".to_string()),
        _ => None,
    }
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
        .route(
            "/missions/{id}/monitor-snapshot",
            get(get_mission_monitor_snapshot),
        )
        .route(
            "/missions/{id}/monitor-actions",
            post(request_mission_monitor_action),
        )
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
        .route("/missions/{id}/events/summary", get(get_mission_event_summary))
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
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let team_id = service
        .get_agent_team_id(&req.agent_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":"team_lookup_failed"})),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error":"agent_team_not_found"})),
        ))?;

    let is_member = service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":"team_membership_check_failed"})),
            )
        })?;
    if !is_member {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error":"forbidden"})),
        ));
    }

    // Enforce agent group-based access control
    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new((*db).clone())
            .get_user_group_ids(&team_id, &user.user_id)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error":"user_group_lookup_failed"})),
                )
            })?;
    let has_agent_access = service
        .check_agent_access(&req.agent_id, &user.user_id, &user_group_ids)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":"agent_access_check_failed"})),
            )
        })?;
    if !has_agent_access {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error":"forbidden"})),
        ));
    }

    let mission = service
        .create_mission(&req, &team_id, &user.user_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create mission: {}", e);
            create_mission_error_response(&e)
        })?;
    let session_id = bind_mission_session_if_missing(&service, &mission)
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to bind dedicated mission session for mission {}: {:?}",
                mission.mission_id,
                e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":"mission_session_bind_failed"})),
            )
        })?;

    Ok(Json(serde_json::json!({
        "route": "mission",
        "mission_id": mission.mission_id,
        "status": mission.status,
        "session_id": session_id,
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

    let values: Vec<serde_json::Value> = items
        .iter()
        .map(|item| serde_json::to_value(item).unwrap_or_default())
        .collect();
    Ok(Json(serde_json::Value::Array(values)))
}

async fn get_mission(
    State((service, _, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mission = service
        .get_mission_runtime_view(&mission_id)
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

    let artifacts = service
        .list_mission_artifacts(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(mission_to_json(&mission, &artifacts)))
}

async fn get_mission_monitor_snapshot(
    State((service, _, mission_manager, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mission = service
        .get_mission_runtime_view(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    require_creator_or_admin(&service, &user.user_id, &mission).await?;

    if mission.harness_version != super::mission_mongo::MissionHarnessVersion::V4 {
        return Err(StatusCode::CONFLICT);
    }

    let artifacts = service
        .list_mission_artifacts(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let snapshot = build_monitor_snapshot(
        &mission,
        &artifacts,
        mission_manager.is_active(&mission_id).await,
    );
    Ok(Json(
        serde_json::to_value(snapshot).unwrap_or_else(|_| serde_json::json!({})),
    ))
}

async fn request_mission_monitor_action(
    State((service, db, mission_manager, ref workspace_root)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
    Json(body): Json<MonitorActionRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mission = service
        .get_mission_runtime_view(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    require_creator_or_admin(&service, &user.user_id, &mission).await?;

    if mission.harness_version != super::mission_mongo::MissionHarnessVersion::V4 {
        return Err(StatusCode::CONFLICT);
    }

    let action = normalize_monitor_action(&body.action).ok_or(StatusCode::BAD_REQUEST)?;
    let feedback = build_monitor_feedback(&action, &body);
    let outcome = execute_monitor_action(
        &service,
        &db,
        &mission_manager,
        workspace_root,
        &mission,
        action,
        feedback,
        body.observed_evidence.clone(),
        body.semantic_tags.clone(),
        body.missing_core_deliverables.clone(),
        body.confidence,
        body.strategy_patch.clone(),
        body.subagent_recommended,
        body.parallelism_budget,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({
        "mission_id": mission_id,
        "status": outcome.status,
        "action": outcome.action,
        "applied": outcome.applied,
    })))
}

async fn delete_mission(
    State((service, _, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let mission = service
        .get_mission_runtime_view(&mission_id)
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
        // Mission was verified above but disappeared before delete due to concurrent deletion.
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
        .get_mission_runtime_view(&mission_id)
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
    if let Err(e) = runtime::ensure_mission_session_for_start(
        &service,
        &mission_id,
        &mission,
        None,
        None,
        None,
    )
    .await
    {
        mission_manager.complete(&mission_id).await;
        tracing::error!(
            "Failed to pre-bind mission session for mission {} before start: {}",
            mission_id,
            e
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    if let Err(e) = service.set_mission_current_run(&mission_id, &run_id).await {
        mission_manager.complete(&mission_id).await;
        tracing::error!(
            "Failed to set current run for mission {}: {}",
            mission_id,
            e
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let refreshed_mission = service
        .get_mission_runtime_view(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    initialize_v4_run_state(&service, &refreshed_mission, &run_id).await?;

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
        .get_mission_runtime_view(&mission_id)
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
        .get_mission_runtime_view(&mission_id)
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

    let registration = match mission_manager.register_with_grace(&mission_id).await {
        Some(registration) => registration,
        None => {
            let has_active_task = service
                .mission_has_active_executor_task(&mission_id, mission.current_run_id.as_deref())
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let manager_active = mission_manager.is_active(&mission_id).await;
            if mission.status == MissionStatus::Paused && manager_active && !has_active_task {
                mission_manager.complete(&mission_id).await;
                if let Some(registration) = mission_manager.register(&mission_id).await {
                    registration
                } else {
                    return Ok(Json(
                        serde_json::json!({ "mission_id": mission_id, "status": "pause_in_progress" }),
                    ));
                }
            } else {
            let status = if mission.status == MissionStatus::Paused {
                "pause_in_progress"
            } else {
                "already_running"
            };
            return Ok(Json(
                serde_json::json!({ "mission_id": mission_id, "status": status }),
            ));
            }
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
    let refreshed_mission = service
        .get_mission_runtime_view(&mission_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    initialize_v4_run_state(&service, &refreshed_mission, &run_id).await?;

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
        .get_mission_runtime_view(&mission_id)
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
        .get_mission_runtime_view(&mission_id)
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
    let registration = match mission_manager.register_with_grace(&mission_id).await {
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
        .get_mission_runtime_view(&mission_id)
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
        .get_mission_runtime_view(&mission_id)
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
    let registration = match mission_manager.register_with_grace(&mission_id).await {
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
        .get_mission_runtime_view(&mission_id)
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

    let registration = match mission_manager.register_with_grace(&mission_id).await {
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
        .get_mission_runtime_view(&mission_id)
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
        .get_mission_runtime_view(&mission_id)
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

    let registration = match mission_manager.register_with_grace(&mission_id).await {
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
        .get_mission_runtime_view(&mission_id)
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

    let registration = match mission_manager.register_with_grace(&mission_id).await {
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
        .get_mission_runtime_view(&mission_id)
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
        .get_mission_runtime_view(&mission_id)
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

fn mission_event_audit_summary(
    mission_id: &str,
    run_id: Option<&str>,
    events: &[super::mission_mongo::MissionEventDoc],
) -> MissionEventAuditSummary {
    let mut counts_by_type = BTreeMap::new();
    for event in events {
        *counts_by_type.entry(event.event_type.clone()).or_insert(0) += 1;
    }

    let key_moments = events
        .iter()
        .filter_map(|event| {
            let payload = event.payload.as_object()?;
            let summary = match event.event_type.as_str() {
                "goal_start" => {
                    let title = payload.get("title")?.as_str()?;
                    Some(format!("开始目标：{}", title))
                }
                "goal_complete" => {
                    let goal_id = payload.get("goal_id")?.as_str()?;
                    let signal = payload
                        .get("signal")
                        .and_then(|v| v.as_str())
                        .unwrap_or("completed");
                    Some(format!("目标 {} 完成，signal={}", goal_id, signal))
                }
                "goal_abandoned" => {
                    let goal_id = payload.get("goal_id")?.as_str()?;
                    let reason = payload
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    Some(format!("目标 {} 放弃：{}", goal_id, reason))
                }
                "pivot" => {
                    let goal_id = payload.get("goal_id")?.as_str()?;
                    let to = payload
                        .get("to_approach")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    Some(format!("目标 {} 调整方法：{}", goal_id, to))
                }
                "workspace_changed" => {
                    let tool_name = payload
                        .get("tool_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("tool");
                    Some(format!("工作区有新写入：{}", tool_name))
                }
                "done" => {
                    let status = payload
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("done");
                    let error = payload.get("error").and_then(|v| v.as_str());
                    Some(match error {
                        Some(err) if !err.trim().is_empty() => {
                            format!("任务结束：{} ({})", status, err)
                        }
                        _ => format!("任务结束：{}", status),
                    })
                }
                "status" => {
                    let raw = payload.get("status")?.as_str()?;
                    summarize_status_event(raw)
                }
                _ => None,
            }?;
            Some(MissionEventAuditMoment {
                event_id: event.event_id,
                event_type: event.event_type.clone(),
                summary,
                created_at: event.created_at.to_string(),
            })
        })
        .collect();

    MissionEventAuditSummary {
        mission_id: mission_id.to_string(),
        run_id: run_id.map(str::to_string),
        total_events: events.len(),
        counts_by_type,
        key_moments,
        first_event_at: events.first().map(|e| e.created_at.to_string()),
        last_event_at: events.last().map(|e| e.created_at.to_string()),
    }
}

async fn get_mission_event_summary(
    State((service, _, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
    Query(q): Query<EventListQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mission = service
        .get_mission_runtime_view(&mission_id)
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

    let limit = q.limit.unwrap_or(2000).clamp(1, 2000);
    let explicit_run_id = q.run_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let run_id = match explicit_run_id {
        Some(rid) if rid.eq_ignore_ascii_case("__all__") || rid.eq_ignore_ascii_case("all") || rid == "*" => None,
        Some(rid) => Some(rid),
        None => mission.current_run_id.as_deref(),
    };
    let events = service
        .list_mission_events(&mission_id, run_id, q.after_event_id, limit)
        .await
        .map_err(|e| {
            tracing::error!("Failed to summarize mission events: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let mut value =
        serde_json::to_value(mission_event_audit_summary(&mission_id, run_id, &events))
            .unwrap_or_default();
    fix_bson_dates(&mut value);
    Ok(Json(value))
}

async fn list_artifacts(
    State((service, db, _, _)): State<MissionState>,
    Extension(user): Extension<UserContext>,
    Path(mission_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mission = service
        .get_mission_runtime_view(&mission_id)
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

    let required_output_hints = manifest_requested_output_hints(&mission);
    let satisfied_output_hints = manifest_satisfied_output_hints(&mission);
    let values: Vec<serde_json::Value> = items
        .iter()
        .map(|artifact| artifact_to_json(artifact, &required_output_hints, &satisfied_output_hints))
        .collect();
    Ok(Json(serde_json::Value::Array(values)))
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
        .get_mission_runtime_view(&artifact.mission_id)
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

    let required_output_hints = manifest_requested_output_hints(&mission);
    let satisfied_output_hints = manifest_satisfied_output_hints(&mission);

    Ok(Json(artifact_to_json(
        &artifact,
        &required_output_hints,
        &satisfied_output_hints,
    )))
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
        .get_mission_runtime_view(&artifact.mission_id)
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
        .get_mission_runtime_view(&artifact.mission_id)
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
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let team_id = service
        .get_agent_team_id(&req.agent_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":"team_lookup_failed"})),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error":"agent_team_not_found"})),
        ))?;

    let is_member = service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":"team_membership_check_failed"})),
            )
        })?;
    if !is_member {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error":"forbidden"})),
        ));
    }

    // Enforce agent group-based access control
    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new((*db).clone())
            .get_user_group_ids(&team_id, &user.user_id)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error":"user_group_lookup_failed"})),
                )
            })?;
    let has_agent_access = service
        .check_agent_access(&req.agent_id, &user.user_id, &user_group_ids)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":"agent_access_check_failed"})),
            )
        })?;
    if !has_agent_access {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error":"forbidden"})),
        ));
    }

    let create_req = CreateMissionRequest {
        agent_id: req.agent_id,
        goal: req.goal,
        context: None,
        approval_policy: req.approval_policy,
        token_budget: req.token_budget,
        priority: None,
        step_timeout_seconds: None,
        step_max_retries: None,
        source_chat_session_id: Some(req.chat_session_id),
        attached_document_ids: vec![],
    };

    let mission = service
        .create_mission(&create_req, &team_id, &user.user_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create mission from chat: {}", e);
            create_mission_error_response(&e)
        })?;
    let session_id = bind_mission_session_if_missing(&service, &mission)
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to bind dedicated mission session for mission {} from chat: {:?}",
                mission.mission_id,
                e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":"mission_session_bind_failed"})),
            )
        })?;

    Ok(Json(serde_json::json!({
        "mission_id": mission.mission_id,
        "status": mission.status,
        "session_id": session_id,
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
        .get_mission_runtime_view(&mission_id)
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
        .get_mission_runtime_view(&mission_id)
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
        .get_mission_runtime_view(&mission_id)
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

#[cfg(test)]
mod tests {
    use super::{artifact_to_json, classify_artifact_delivery, ArtifactDeliveryRole};
    use crate::agent::mission_mongo::{
        ApprovalPolicy, ArtifactType, ExecutionMode, ExecutionProfile, GoalNode, GoalStatus,
        LaunchPolicy, MissionArtifactDoc, MissionDeliveryManifest, MissionDeliveryState, MissionDoc,
        MissionHarnessVersion, MissionStep, MissionStatus, StepStatus,
    };
    use bson::{oid::ObjectId, DateTime};

    fn sample_mission_doc() -> MissionDoc {
        MissionDoc {
            id: None,
            mission_id: "mission-1".to_string(),
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
                required_artifacts: vec!["out/report.md".to_string()],
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
                requested_deliverables: vec!["out/report.md".to_string()],
                satisfied_deliverables: Vec::new(),
                missing_core_deliverables: vec!["out/report.md".to_string()],
                supporting_artifacts: Vec::new(),
                delivery_state: MissionDeliveryState::Working,
                final_outcome_summary: None,
            }),
            progress_memory: None,
            completion_assessment: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
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
    fn artifact_json_exposes_relative_path_alias_and_hides_bson_id() {
        let artifact = MissionArtifactDoc {
            id: Some(ObjectId::new()),
            artifact_id: "artifact-1".to_string(),
            mission_id: "mission-1".to_string(),
            step_index: 2,
            name: "report.md".to_string(),
            artifact_type: ArtifactType::Document,
            content: None,
            file_path: Some("deliverables/report.md".to_string()),
            mime_type: Some("text/markdown".to_string()),
            size: 128,
            archived_document_id: None,
            archived_document_status: None,
            archived_at: None,
            created_at: DateTime::now(),
        };

        let value = artifact_to_json(&artifact, &Default::default(), &Default::default());
        let obj = value
            .as_object()
            .expect("artifact should serialize to object");

        assert_eq!(
            obj.get("relative_path").and_then(|v| v.as_str()),
            Some("deliverables/report.md")
        );
        assert_eq!(
            obj.get("file_path").and_then(|v| v.as_str()),
            Some("deliverables/report.md")
        );
        assert!(!obj.contains_key("_id"));
    }

    #[test]
    fn classify_contract_like_artifact_as_supporting() {
        let artifact = MissionArtifactDoc {
            id: Some(ObjectId::new()),
            artifact_id: "artifact-2".to_string(),
            mission_id: "mission-1".to_string(),
            step_index: 0,
            name: "CONTRACT.md".to_string(),
            artifact_type: ArtifactType::Document,
            content: None,
            file_path: Some("deliverable/CONTRACT.md".to_string()),
            mime_type: Some("text/markdown".to_string()),
            size: 128,
            archived_document_id: None,
            archived_document_status: None,
            archived_at: None,
            created_at: DateTime::now(),
        };

        let classification =
            classify_artifact_delivery(&Default::default(), &Default::default(), &artifact);
        assert!(matches!(
            classification.role,
            ArtifactDeliveryRole::SupportingArtifact
        ));
        assert!(!classification.is_required_output);
    }

    #[test]
    fn classify_required_deliverable_as_core_output() {
        let artifact = MissionArtifactDoc {
            id: Some(ObjectId::new()),
            artifact_id: "artifact-3".to_string(),
            mission_id: "mission-1".to_string(),
            step_index: 2,
            name: "report.md".to_string(),
            artifact_type: ArtifactType::Document,
            content: None,
            file_path: Some("deliverables/report.md".to_string()),
            mime_type: Some("text/markdown".to_string()),
            size: 256,
            archived_document_id: None,
            archived_document_status: None,
            archived_at: None,
            created_at: DateTime::now(),
        };
        let mut hints = std::collections::BTreeSet::new();
        hints.insert("deliverables/report.md".to_string());
        hints.insert("report.md".to_string());

        let classification = classify_artifact_delivery(&hints, &Default::default(), &artifact);
        assert!(matches!(
            classification.role,
            ArtifactDeliveryRole::CoreDeliverable
        ));
        assert!(classification.is_required_output);
    }

    #[test]
    fn classify_required_contract_like_output_as_core_when_user_requested() {
        let artifact = MissionArtifactDoc {
            id: Some(ObjectId::new()),
            artifact_id: "artifact-4".to_string(),
            mission_id: "mission-1".to_string(),
            step_index: 2,
            name: "verification.md".to_string(),
            artifact_type: ArtifactType::Document,
            content: None,
            file_path: Some("deliverables/verification.md".to_string()),
            mime_type: Some("text/markdown".to_string()),
            size: 196,
            archived_document_id: None,
            archived_document_status: None,
            archived_at: None,
            created_at: DateTime::now(),
        };
        let mut hints = std::collections::BTreeSet::new();
        hints.insert("deliverables/verification.md".to_string());
        hints.insert("verification.md".to_string());

        let classification = classify_artifact_delivery(&hints, &Default::default(), &artifact);
        assert!(matches!(
            classification.role,
            ArtifactDeliveryRole::CoreDeliverable
        ));
        assert!(classification.is_required_output);
    }

    #[test]
    fn classify_manifest_satisfied_artifact_as_core_output() {
        let artifact = MissionArtifactDoc {
            id: Some(ObjectId::new()),
            artifact_id: "artifact-5".to_string(),
            mission_id: "mission-1".to_string(),
            step_index: 2,
            name: "overview.html".to_string(),
            artifact_type: ArtifactType::Document,
            content: None,
            file_path: Some("deliverables/overview.html".to_string()),
            mime_type: Some("text/html".to_string()),
            size: 384,
            archived_document_id: None,
            archived_document_status: None,
            archived_at: None,
            created_at: DateTime::now(),
        };
        let mut satisfied = std::collections::BTreeSet::new();
        satisfied.insert("deliverables/overview.html".to_string());
        satisfied.insert("overview.html".to_string());

        let classification =
            classify_artifact_delivery(&Default::default(), &satisfied, &artifact);
        assert!(matches!(
            classification.role,
            ArtifactDeliveryRole::CoreDeliverable
        ));
        assert!(!classification.is_required_output);
        assert_eq!(classification.reason, "manifest_satisfied");
    }

    #[test]
    fn build_v4_task_graph_clears_current_node_for_terminal_mission() {
        let mut mission = sample_mission_doc();
        mission.status = MissionStatus::Completed;
        mission.current_step = None;
        mission.goal_tree = Some(vec![GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal 1".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Completed,
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
        }]);
        mission.execution_mode = ExecutionMode::Adaptive;
        mission.current_goal_id = None;

        let graph = super::build_v4_task_graph(&mission, "run-1");
        assert!(
            graph.current_node_id.is_none(),
            "terminal V4 task graphs should not repopulate an active current node"
        );
    }
}
