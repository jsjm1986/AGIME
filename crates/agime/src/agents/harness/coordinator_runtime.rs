use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use serde::Serialize;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::agents::harness::child_tasks::{
    build_swarm_worker_instructions, build_validation_worker_instructions,
    classify_child_task_result, parse_validation_outcome, validation_response_schema,
    ChildExecutionExpectation, ValidationReport,
};
use crate::agents::harness::coordinator::{
    leader_permission_bridge_enabled, swarm_scratchpad_enabled, worker_name_for_target,
    CoordinatorRole, SwarmExecutionRequest, SwarmExecutionResult,
};
use crate::agents::harness::delegation::{
    DelegationMode, DelegationRuntimeState, SwarmBudget, SwarmPlan,
};
use crate::agents::harness::mailbox::{
    mailbox_dir, write_to_mailbox, MailboxLineage, MailboxMessage, MailboxMessageKind,
    SwarmProtocolKind, SwarmProtocolMessage,
};
use crate::agents::harness::permission_bridge::{
    list_pending_requests, resolve_permission_request, resolve_permission_request_via_runtime,
};
use crate::agents::harness::scratchpad::SwarmScratchpad;
use crate::agents::harness::swarm_runtime::{
    build_bounded_swarm_plan, decide_bounded_swarm_outcome, worker_task_spec, SwarmWorkerSpec,
};
use crate::agents::harness::task_runtime::{
    TaskKind, TaskResultEnvelope, TaskRuntime, TaskRuntimeHost, WorkerAttemptIdentity,
};
use crate::agents::subagent_handler::run_complete_subagent_task;
use crate::agents::subagent_task_config::TaskConfig;
use crate::prompt_template::render_global_file;
use crate::recipe::{Recipe, Response};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkerFollowupKind {
    Continue,
    Correction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkerExecutionExpectation {
    ReadOnlyInspection,
    MaterializingChange,
}

#[derive(Clone, Debug)]
struct WorkerExecutionResult {
    summary: String,
    accepted_targets: Vec<String>,
    produced_targets: Vec<String>,
    produced_delta: bool,
}

#[derive(Serialize)]
struct WorkerPromptContext {
    worker_name: String,
    target_artifact: String,
    result_contract: String,
    write_scope: String,
    scratchpad_root: String,
    mailbox_root: String,
    runtime_tool_surface: String,
    hidden_coordinator_tools: String,
    permission_policy: String,
    peer_messaging_policy: String,
    available_peer_workers: String,
}

#[derive(Serialize)]
struct WorkerFollowupPromptContext {
    worker_name: String,
    target_artifact: String,
    result_contract: String,
    write_scope: String,
    previous_summary: String,
    correction_reason: String,
}

fn build_attempt_identity(
    logical_worker_id: &str,
    attempt_task_id: &str,
    followup: Option<(WorkerFollowupKind, &str)>,
) -> WorkerAttemptIdentity {
    let attempt_index = attempt_task_id
        .rsplit_once("-retry")
        .and_then(|(_, suffix)| suffix.parse::<u32>().ok())
        .unwrap_or(0);
    match followup {
        Some((kind, previous_task_id)) => WorkerAttemptIdentity::followup(
            logical_worker_id.to_string(),
            attempt_task_id.to_string(),
            attempt_index,
            match kind {
                WorkerFollowupKind::Continue => "continue",
                WorkerFollowupKind::Correction => "correction",
            },
            previous_task_id.to_string(),
        ),
        None => {
            WorkerAttemptIdentity::fresh(logical_worker_id.to_string(), attempt_task_id.to_string())
        }
    }
}

fn mailbox_lineage_from_attempt_identity(
    attempt_identity: &WorkerAttemptIdentity,
) -> MailboxLineage {
    MailboxLineage {
        logical_worker_id: Some(attempt_identity.logical_worker_id.clone()),
        attempt_id: Some(attempt_identity.attempt_id.clone()),
        attempt_index: Some(attempt_identity.attempt_index),
        previous_task_id: attempt_identity.previous_task_id.clone(),
    }
}

fn peer_worker_addresses_from_book(
    worker_address_book: &[(String, String)],
    current_idx: usize,
) -> Vec<String> {
    worker_address_book
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx != current_idx)
        .map(|(_, (address, _))| address.clone())
        .collect()
}

fn peer_worker_catalog_from_book(
    worker_address_book: &[(String, String)],
    current_idx: usize,
) -> Vec<String> {
    worker_address_book
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx != current_idx)
        .map(|(_, (address, target_artifact))| format!("{} ({})", address, target_artifact))
        .collect()
}

fn build_worker_prompt_context(
    base_task_config: &TaskConfig,
    worker_name: &str,
    target_artifact: &str,
    result_contract: &[String],
    write_scope: &[String],
    scratchpad_root: Option<&PathBuf>,
    mailbox_root: &PathBuf,
    allow_worker_messaging: bool,
    peer_worker_addresses: Vec<String>,
    peer_worker_catalog: Vec<String>,
    validation_mode: bool,
) -> WorkerPromptContext {
    let capability_context = base_task_config
        .clone()
        .with_worker_runtime(
            None,
            Some(worker_name.to_string()),
            Some("leader".to_string()),
            CoordinatorRole::Worker,
            scratchpad_root.cloned(),
            Some(mailbox_root.clone()),
            None,
            leader_permission_bridge_enabled(),
            allow_worker_messaging,
            peer_worker_addresses,
            peer_worker_catalog,
            validation_mode,
        )
        .worker_capability_context()
        .unwrap_or_default();

    WorkerPromptContext {
        worker_name: worker_name.to_string(),
        target_artifact: target_artifact.to_string(),
        result_contract: if result_contract.is_empty() {
            "none".to_string()
        } else {
            result_contract.join(", ")
        },
        write_scope: if write_scope.is_empty() {
            "none".to_string()
        } else {
            write_scope.join(", ")
        },
        scratchpad_root: scratchpad_root
            .map(|value| value.display().to_string())
            .unwrap_or_else(|| "disabled".to_string()),
        mailbox_root: mailbox_root.display().to_string(),
        runtime_tool_surface: capability_context.runtime_tool_surface_text(),
        hidden_coordinator_tools: capability_context.hidden_tools_text(),
        permission_policy: capability_context.permission_policy,
        peer_messaging_policy: if capability_context.allow_worker_messaging {
            "Peer worker messaging is enabled for this swarm run. Use `send_message` only for bounded coordination with the listed recipients or the leader.".to_string()
        } else {
            "Peer worker messaging is disabled for this runtime. Coordinate only through your final summary or leader follow-up.".to_string()
        },
        available_peer_workers: if capability_context.peer_worker_catalog.is_empty() {
            "none".to_string()
        } else {
            capability_context.peer_worker_catalog.join(", ")
        },
    }
}

async fn drive_permission_bridge(
    working_dir: PathBuf,
    run_id: String,
    cancel_token: CancellationToken,
) {
    while !cancel_token.is_cancelled() {
        if let Ok(requests) = list_pending_requests(&working_dir, &run_id) {
            for request in requests {
                let resolution = resolve_permission_request_via_runtime(&request);
                let _ = resolve_permission_request(&working_dir, &run_id, &resolution);
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn is_logical_artifact_target(target: &str) -> bool {
    target.contains(':') && !Path::new(target).is_absolute()
}

fn resolve_workspace_validation_target(working_dir: &Path, target: &str) -> Option<PathBuf> {
    let trimmed = target.trim();
    if trimmed.is_empty() || is_logical_artifact_target(trimmed) {
        return None;
    }
    let normalized = trimmed.replace('\\', "/");
    let candidate = PathBuf::from(&normalized);
    if candidate.is_absolute() {
        return candidate.starts_with(working_dir).then_some(candidate);
    }
    if Path::new(&normalized).components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    }) {
        return None;
    }
    Some(working_dir.join(candidate))
}

fn precheck_materialized_validation_target(
    working_dir: &Path,
    target_artifact: &str,
) -> Option<ValidationReport> {
    let target_path = resolve_workspace_validation_target(working_dir, target_artifact)?;
    (!target_path.exists()).then(|| ValidationReport::artifact_not_materialized(target_artifact))
}

fn summarize_validation_report(report: &ValidationReport) -> String {
    if report.passed() {
        report
            .evidence_summary
            .clone()
            .unwrap_or_else(|| "PASS: validation passed".to_string())
    } else if let Some(reason) = report.failure_reason() {
        format!("FAIL: {}", reason)
    } else {
        report
            .evidence_summary
            .clone()
            .unwrap_or_else(|| "FAIL: validation failed".to_string())
    }
}

fn accepts_recovered_worker_summary(worker_summary: &str, produced_delta: bool) -> bool {
    produced_delta
        && worker_summary
            .to_ascii_lowercase()
            .contains("worker finished without an explicit terminal summary")
}

fn summarize_worker_for_leader(worker_summary: &str, recovered_terminal_summary: bool) -> String {
    if !recovered_terminal_summary {
        return worker_summary.to_string();
    }
    "completed via recovered worker trace".to_string()
}

fn worker_execution_expectation(worker: &SwarmWorkerSpec) -> WorkerExecutionExpectation {
    if worker.write_scope.is_empty() {
        WorkerExecutionExpectation::ReadOnlyInspection
    } else {
        WorkerExecutionExpectation::MaterializingChange
    }
}

fn worker_recipe(instructions: String) -> Result<Recipe> {
    worker_recipe_with_response(instructions, None)
}

fn worker_recipe_with_response(instructions: String, response: Option<Response>) -> Result<Recipe> {
    let mut builder = Recipe::builder()
        .version("1.0.0")
        .title("Swarm Worker")
        .description("Bounded swarm worker")
        .instructions(instructions);
    if let Some(response) = response {
        builder = builder.response(response);
    }
    builder
        .build()
        .map_err(|error| anyhow!("failed to build swarm worker recipe: {}", error))
}

fn render_worker_prompt(
    base_task_config: &TaskConfig,
    worker_name: &str,
    target_artifact: &str,
    result_contract: &[String],
    write_scope: &[String],
    scratchpad_root: Option<&PathBuf>,
    mailbox_root: &PathBuf,
    allow_worker_messaging: bool,
    peer_worker_addresses: Vec<String>,
    peer_worker_catalog: Vec<String>,
    validation_mode: bool,
) -> String {
    let context = build_worker_prompt_context(
        base_task_config,
        worker_name,
        target_artifact,
        result_contract,
        write_scope,
        scratchpad_root,
        mailbox_root,
        allow_worker_messaging,
        peer_worker_addresses,
        peer_worker_catalog,
        validation_mode,
    );
    let template = if validation_mode {
        "validation_worker_system.md"
    } else {
        "worker_system.md"
    };
    let mut rendered = render_global_file(template, &context).unwrap_or_else(|_| {
        format!(
            "Worker {} is bounded to target {} with result contract [{}] and write scope [{}]. Mailbox root: {}.",
            worker_name,
            target_artifact,
            context.result_contract,
            context.write_scope,
            context.mailbox_root
        )
    });
    if validation_mode {
        if let Ok(extra) = render_global_file("verification_worker.md", &context) {
            rendered.push_str("\n\n");
            rendered.push_str(&extra);
        }
    }
    rendered
}

fn render_followup_prompt(
    kind: WorkerFollowupKind,
    worker_name: &str,
    target_artifact: &str,
    result_contract: &[String],
    write_scope: &[String],
    previous_summary: &str,
    correction_reason: &str,
) -> Option<String> {
    let template = match kind {
        WorkerFollowupKind::Continue => "worker_continue.md",
        WorkerFollowupKind::Correction => "worker_correction.md",
    };
    render_global_file(
        template,
        &WorkerFollowupPromptContext {
            worker_name: worker_name.to_string(),
            target_artifact: target_artifact.to_string(),
            result_contract: if result_contract.is_empty() {
                "none".to_string()
            } else {
                result_contract.join(", ")
            },
            write_scope: if write_scope.is_empty() {
                "none".to_string()
            } else {
                write_scope.join(", ")
            },
            previous_summary: previous_summary.trim().to_string(),
            correction_reason: correction_reason.trim().to_string(),
        },
    )
    .ok()
}

fn permission_bridge_hint() -> Option<String> {
    if !leader_permission_bridge_enabled() {
        return None;
    }
    render_global_file("permission_bridge_explainer.md", &serde_json::json!({})).ok()
}

fn worker_instruction_with_prompts(
    base_task_config: &TaskConfig,
    base_instructions: &str,
    worker_name: &str,
    worker_spec: &SwarmWorkerSpec,
    scratchpad: Option<&SwarmScratchpad>,
    working_dir: &Path,
    run_id: &str,
    allow_worker_messaging: bool,
    peer_worker_addresses: Vec<String>,
    peer_worker_catalog: Vec<String>,
    validation_mode: bool,
    followup: Option<(WorkerFollowupKind, String, String)>,
) -> Result<String> {
    let mailbox_root = mailbox_dir(working_dir, run_id);
    let mut instruction = render_worker_prompt(
        base_task_config,
        worker_name,
        &worker_spec.target_artifact,
        &worker_spec.result_contract,
        if validation_mode {
            &[]
        } else {
            &worker_spec.write_scope
        },
        scratchpad.map(|value| &value.root),
        &mailbox_root,
        allow_worker_messaging,
        peer_worker_addresses,
        peer_worker_catalog,
        validation_mode,
    );
    if let Some((kind, previous_summary, reason)) = followup {
        if let Some(addendum) = render_followup_prompt(
            kind,
            worker_name,
            &worker_spec.target_artifact,
            &worker_spec.result_contract,
            &worker_spec.write_scope,
            &previous_summary,
            &reason,
        ) {
            instruction.push_str("\n\n");
            instruction.push_str(&addendum);
        }
    }
    instruction.push_str("\n\n");
    instruction.push_str(base_instructions);
    if let Some(scratchpad) = scratchpad {
        let worker_dir = scratchpad.ensure_worker_dir(worker_name)?;
        let _ = scratchpad.write_worker_note(worker_name, "instructions.md", &instruction);
        instruction.push_str(&format!(
            "\n\nShared scratchpad root: {}\nWorker scratchpad dir: {}",
            scratchpad.root.display(),
            worker_dir.display()
        ));
    }
    instruction.push_str(&format!(
        "\n\nMailbox root: {}. {}",
        mailbox_root.display(),
        if allow_worker_messaging {
            "Use the mailbox for bounded summaries and `send_message` for direct peer coordination."
        } else {
            "Use summary-only communication."
        }
    ));
    if let Some(permission_hint) = permission_bridge_hint() {
        instruction.push_str("\n\n");
        instruction.push_str(&permission_hint);
    }
    Ok(instruction)
}

async fn execute_worker_attempt(
    base_task_config: &TaskConfig,
    task_runtime: std::sync::Arc<TaskRuntime>,
    parent_session_id: &str,
    leader_name: &str,
    request_parallelism_budget: Option<u32>,
    working_dir: &Path,
    run_id: &str,
    worker_spec: SwarmWorkerSpec,
    worker_name: String,
    leader_cancel: &CancellationToken,
    scratchpad: Option<&SwarmScratchpad>,
    base_instructions: &str,
    allow_worker_messaging: bool,
    peer_worker_addresses: Vec<String>,
    peer_worker_catalog: Vec<String>,
    followup: Option<(WorkerFollowupKind, String, String)>,
    attempt_suffix: Option<&str>,
) -> Result<WorkerExecutionResult> {
    let attempt_task_id = attempt_suffix
        .map(|suffix| format!("{}-{}", worker_spec.task_id, suffix))
        .unwrap_or_else(|| worker_spec.task_id.clone());
    let worker_session_id = format!("worker-session-{}", worker_spec.task_id);
    let attempt_identity = build_attempt_identity(
        &worker_name,
        &attempt_task_id,
        followup
            .as_ref()
            .map(|value| (value.0, worker_spec.task_id.as_str())),
    );
    let attempt_spec = SwarmWorkerSpec {
        task_id: attempt_task_id.clone(),
        ..worker_spec.clone()
    };
    let instruction = worker_instruction_with_prompts(
        base_task_config,
        base_instructions,
        &worker_name,
        &attempt_spec,
        scratchpad,
        working_dir,
        run_id,
        allow_worker_messaging,
        peer_worker_addresses.clone(),
        peer_worker_catalog.clone(),
        false,
        followup.clone(),
    )?;
    write_to_mailbox(
        working_dir,
        run_id,
        &worker_name,
        MailboxMessage {
            id: attempt_task_id.clone(),
            from: leader_name.to_string(),
            to: worker_name.clone(),
            kind: MailboxMessageKind::Directive,
            text: instruction.clone(),
            summary: Some(match followup.as_ref().map(|value| value.0) {
                Some(WorkerFollowupKind::Continue) => {
                    format!("continue {}", attempt_spec.target_artifact)
                }
                Some(WorkerFollowupKind::Correction) => {
                    format!("correct {}", attempt_spec.target_artifact)
                }
                None => format!("directive for {}", attempt_spec.target_artifact),
            }),
            timestamp: String::new(),
            read: false,
            protocol: Some(SwarmProtocolMessage {
                kind: SwarmProtocolKind::Directive,
                thread_id: Some(attempt_task_id.clone()),
                directive_kind: Some(
                    followup
                        .as_ref()
                        .map(|value| match value.0 {
                            WorkerFollowupKind::Continue => "continue",
                            WorkerFollowupKind::Correction => "correction",
                        })
                        .unwrap_or("initial")
                        .to_string(),
                ),
                reason_code: None,
                target_artifacts: vec![attempt_spec.target_artifact.clone()],
                referenced_task_ids: vec![attempt_task_id.clone()],
                lineage: Some(mailbox_lineage_from_attempt_identity(&attempt_identity)),
            }),
            metadata: {
                let mut metadata = HashMap::new();
                attempt_identity.write_to_metadata(&mut metadata);
                metadata
            },
        },
    )?;

    let mut worker_task_spec = worker_task_spec(
        parent_session_id,
        base_task_config.current_depth.saturating_add(1),
        &attempt_spec,
    );
    attempt_identity.write_to_metadata(&mut worker_task_spec.metadata);
    let handle = task_runtime.spawn_task(worker_task_spec).await?;
    let worker_cancel = leader_cancel.child_token();
    let runtime_cancel = worker_cancel.clone();
    let task_cancel = handle.cancel_token.clone();
    tokio::spawn(async move {
        task_cancel.cancelled().await;
        runtime_cancel.cancel();
    });

    let worker_task_config = base_task_config
        .clone()
        .with_delegation_depth(
            base_task_config.current_depth.saturating_add(1),
            base_task_config.max_depth,
        )
        .with_summary_only(true)
        .with_write_scope(attempt_spec.write_scope.clone())
        .with_runtime_contract(
            vec![attempt_spec.target_artifact.clone()],
            attempt_spec.result_contract.clone(),
        )
        .with_delegation_mode(DelegationMode::Disabled)
        .with_parallelism_budget(request_parallelism_budget, request_parallelism_budget)
        .with_worker_runtime(
            Some(run_id.to_string()),
            Some(worker_name.clone()),
            Some(leader_name.to_string()),
            CoordinatorRole::Worker,
            scratchpad.map(|value| value.root.clone()),
            Some(mailbox_dir(working_dir, run_id)),
            Some(
                working_dir
                    .join(".agime")
                    .join("swarm")
                    .join(run_id)
                    .join("permissions"),
            ),
            leader_permission_bridge_enabled(),
            allow_worker_messaging,
            peer_worker_addresses,
            peer_worker_catalog,
            false,
        )
        .with_task_runtime(Some(task_runtime.clone()), Some(handle.task_id.clone()));

    let task_result = run_complete_subagent_task(
        worker_recipe(instruction)?,
        worker_task_config,
        true,
        worker_session_id,
        Some(worker_cancel),
    )
    .await
    .unwrap_or_else(
        |error| crate::agents::subagent_handler::CompletedSubagentTaskResult {
            summary: format!("FAIL: {}", error),
            recovered_terminal_summary: false,
        },
    );
    let summary = task_result.summary;

    let classification = classify_child_task_result(
        match worker_execution_expectation(&attempt_spec) {
            WorkerExecutionExpectation::ReadOnlyInspection => {
                ChildExecutionExpectation::ReadOnlyInspection
            }
            WorkerExecutionExpectation::MaterializingChange => {
                ChildExecutionExpectation::MaterializingChange
            }
        },
        &[attempt_spec.target_artifact.clone()],
        &attempt_spec.write_scope,
        &summary,
    );
    write_to_mailbox(
        working_dir,
        run_id,
        leader_name,
        MailboxMessage {
            id: format!("summary-{}", attempt_task_id),
            from: worker_name.clone(),
            to: leader_name.to_string(),
            kind: MailboxMessageKind::Summary,
            text: summary.clone(),
            summary: Some(attempt_spec.target_artifact.clone()),
            timestamp: String::new(),
            read: false,
            protocol: Some(SwarmProtocolMessage {
                kind: SwarmProtocolKind::Summary,
                thread_id: Some(attempt_task_id.clone()),
                directive_kind: None,
                reason_code: None,
                target_artifacts: vec![attempt_spec.target_artifact.clone()],
                referenced_task_ids: vec![attempt_spec.task_id.clone()],
                lineage: Some(mailbox_lineage_from_attempt_identity(&attempt_identity)),
            }),
            metadata: {
                let mut metadata = HashMap::new();
                attempt_identity.write_to_metadata(&mut metadata);
                metadata
            },
        },
    )?;

    let envelope = TaskResultEnvelope {
        task_id: attempt_task_id,
        kind: TaskKind::SwarmWorker,
        status: if summary.starts_with("FAIL:") {
            crate::agents::harness::TaskStatus::Failed
        } else {
            crate::agents::harness::TaskStatus::Completed
        },
        summary: summary.clone(),
        accepted_targets: classification.accepted_targets.clone(),
        produced_delta: classification.produced_delta,
        metadata: {
            let mut metadata = HashMap::new();
            attempt_identity.write_to_metadata(&mut metadata);
            if task_result.recovered_terminal_summary {
                metadata.insert("recovered_terminal_summary".to_string(), "true".to_string());
            }
            metadata
        },
    };
    if summary.starts_with("FAIL:") {
        let _ = task_runtime.fail(envelope).await;
    } else {
        let _ = task_runtime.complete(envelope).await;
    }

    Ok(WorkerExecutionResult {
        summary: summarize_worker_for_leader(&summary, task_result.recovered_terminal_summary),
        accepted_targets: classification.accepted_targets,
        produced_targets: classification.produced_targets,
        produced_delta: classification.produced_delta,
    })
}

async fn execute_validation_worker(
    base_task_config: &TaskConfig,
    task_runtime: std::sync::Arc<TaskRuntime>,
    leader_name: &str,
    working_dir: &Path,
    run_id: &str,
    worker: &SwarmWorkerSpec,
    idx: usize,
    leader_cancel: &CancellationToken,
    scratchpad: Option<&SwarmScratchpad>,
    worker_summary: &str,
    attempt_suffix: Option<&str>,
) -> Result<ValidationReport> {
    if let Some(report) =
        precheck_materialized_validation_target(working_dir, &worker.target_artifact)
    {
        return Ok(report.with_validator_context(None, vec![worker.target_artifact.clone()]));
    }

    let worker_name = format!(
        "validator_{}",
        worker_name_for_target(&worker.target_artifact, idx)
    );
    let validation_spec = SwarmWorkerSpec {
        task_id: attempt_suffix
            .map(|suffix| format!("validation-{}-{}", worker.task_id, suffix))
            .unwrap_or_else(|| format!("validation-{}", worker.task_id)),
        target_artifact: worker.target_artifact.clone(),
        write_scope: Vec::new(),
        result_contract: worker.result_contract.clone(),
        validation_mode: true,
    };
    let validation_session_id = format!("validation-session-{}", worker.task_id);
    let attempt_identity = build_attempt_identity(
        &worker_name,
        &validation_spec.task_id,
        attempt_suffix.map(|_| (WorkerFollowupKind::Correction, worker.task_id.as_str())),
    );
    let instruction = worker_instruction_with_prompts(
        base_task_config,
        &build_validation_worker_instructions(
            &worker.target_artifact,
            &worker.result_contract,
            worker_summary,
        ),
        &worker_name,
        &validation_spec,
        scratchpad,
        working_dir,
        run_id,
        false,
        Vec::new(),
        Vec::new(),
        true,
        None,
    )?;

    let mut validation_task_spec = worker_task_spec(
        &base_task_config.parent_session_id,
        base_task_config.current_depth.saturating_add(1),
        &validation_spec,
    );
    attempt_identity.write_to_metadata(&mut validation_task_spec.metadata);
    let handle = task_runtime.spawn_task(validation_task_spec).await?;
    let validation_cancel = leader_cancel.child_token();
    let runtime_cancel = validation_cancel.clone();
    let task_cancel = handle.cancel_token.clone();
    tokio::spawn(async move {
        task_cancel.cancelled().await;
        runtime_cancel.cancel();
    });

    let validation_task_config = base_task_config
        .clone()
        .with_delegation_depth(
            base_task_config.current_depth.saturating_add(1),
            base_task_config.max_depth,
        )
        .with_summary_only(true)
        .with_write_scope(Vec::new())
        .with_runtime_contract(
            vec![worker.target_artifact.clone()],
            worker.result_contract.clone(),
        )
        .with_delegation_mode(DelegationMode::Disabled)
        .with_worker_runtime(
            Some(run_id.to_string()),
            Some(worker_name),
            Some(leader_name.to_string()),
            CoordinatorRole::Worker,
            scratchpad.map(|value| value.root.clone()),
            Some(mailbox_dir(working_dir, run_id)),
            Some(
                working_dir
                    .join(".agime")
                    .join("swarm")
                    .join(run_id)
                    .join("permissions"),
            ),
            leader_permission_bridge_enabled(),
            false,
            Vec::new(),
            Vec::new(),
            true,
        )
        .with_task_runtime(Some(task_runtime.clone()), Some(handle.task_id.clone()));

    let task_result = run_complete_subagent_task(
        worker_recipe_with_response(
            instruction,
            Some(Response {
                json_schema: Some(validation_response_schema()),
            }),
        )?,
        validation_task_config,
        true,
        validation_session_id,
        Some(validation_cancel),
    )
    .await
    .unwrap_or_else(
        |error| crate::agents::subagent_handler::CompletedSubagentTaskResult {
            summary: format!("FAIL: {}", error),
            recovered_terminal_summary: false,
        },
    );
    let summary = task_result.summary;
    let validation_classification = classify_child_task_result(
        ChildExecutionExpectation::Validation,
        &[worker.target_artifact.clone()],
        &[],
        &summary,
    );
    let validation_task_id = handle.task_id.clone();
    let validation_envelope = TaskResultEnvelope {
        task_id: validation_task_id.clone(),
        kind: TaskKind::ValidationWorker,
        status: if summary.trim_start().starts_with("FAIL:") {
            crate::agents::harness::TaskStatus::Failed
        } else {
            crate::agents::harness::TaskStatus::Completed
        },
        summary: summary.clone(),
        accepted_targets: validation_classification.accepted_targets,
        produced_delta: false,
        metadata: {
            let mut metadata = HashMap::new();
            attempt_identity.write_to_metadata(&mut metadata);
            metadata
        },
    };
    if summary.trim_start().starts_with("FAIL:") {
        let _ = task_runtime.fail(validation_envelope).await;
    } else {
        let _ = task_runtime.complete(validation_envelope).await;
    }
    Ok(parse_validation_outcome(&summary).with_validator_context(
        Some(validation_task_id),
        vec![worker.target_artifact.clone()],
    ))
}

pub(crate) async fn validate_single_worker_execute_summary(
    base_task_config: &TaskConfig,
    working_dir: &Path,
    target_artifact: String,
    result_contract: Vec<String>,
    worker_summary: &str,
    cancel_token: Option<CancellationToken>,
) -> Result<crate::agents::harness::ValidationReport> {
    if let Some(report) = precheck_materialized_validation_target(working_dir, &target_artifact) {
        return Ok(report.with_validator_context(None, vec![target_artifact]));
    }

    let run_id = format!("single-worker-{}", Uuid::new_v4().simple());
    let effective_result_contract = if result_contract.is_empty() {
        vec![target_artifact.clone()]
    } else {
        result_contract
    };
    let validation_session_id = format!("single-worker-validation-{}", Uuid::new_v4().simple());
    let instruction = worker_instruction_with_prompts(
        base_task_config,
        &build_validation_worker_instructions(
            &target_artifact,
            &effective_result_contract,
            worker_summary,
        ),
        "validator_single_worker",
        &SwarmWorkerSpec {
            task_id: "single-worker-validation".to_string(),
            target_artifact: target_artifact.clone(),
            write_scope: Vec::new(),
            result_contract: effective_result_contract.clone(),
            validation_mode: true,
        },
        None,
        working_dir,
        &run_id,
        false,
        Vec::new(),
        Vec::new(),
        true,
        None,
    )?;
    let validation_task_config = base_task_config
        .clone()
        .with_summary_only(true)
        .with_write_scope(Vec::new())
        .with_runtime_contract(vec![target_artifact.clone()], effective_result_contract)
        .with_delegation_mode(DelegationMode::Disabled)
        .with_worker_runtime(
            Some(run_id),
            Some("validator_single_worker".to_string()),
            Some("leader".to_string()),
            CoordinatorRole::Worker,
            None,
            Some(mailbox_dir(working_dir, "single-worker-validation")),
            Some(
                working_dir
                    .join(".agime")
                    .join("single_worker")
                    .join("permissions"),
            ),
            leader_permission_bridge_enabled(),
            false,
            Vec::new(),
            Vec::new(),
            true,
        )
        .with_task_runtime(None, None);
    let validation_result = run_complete_subagent_task(
        worker_recipe_with_response(
            instruction,
            Some(Response {
                json_schema: Some(validation_response_schema()),
            }),
        )?,
        validation_task_config,
        true,
        validation_session_id,
        cancel_token,
    )
    .await
    .unwrap_or_else(
        |error| crate::agents::subagent_handler::CompletedSubagentTaskResult {
            summary: format!("FAIL: {}", error),
            recovered_terminal_summary: false,
        },
    );
    let validation_summary = validation_result.summary;

    Ok(parse_validation_outcome(&validation_summary)
        .with_validator_context(None, vec![target_artifact]))
}

pub async fn execute_swarm_request(
    request: SwarmExecutionRequest,
    task_config: TaskConfig,
    working_dir: PathBuf,
    cancellation_token: Option<CancellationToken>,
) -> Result<SwarmExecutionResult> {
    if request.targets.len() < 2 {
        return Err(anyhow!("swarm requires at least two targets"));
    }

    let run_id = format!("swarm-{}", Uuid::new_v4().simple());
    let leader_name = "leader".to_string();
    let leader_cancel = cancellation_token.unwrap_or_default();
    let permission_cancel = leader_cancel.child_token();
    let scratchpad = if swarm_scratchpad_enabled() {
        Some(SwarmScratchpad::create(&working_dir, &run_id)?)
    } else {
        None
    };

    let delegation = DelegationRuntimeState::new(
        DelegationMode::Swarm,
        task_config.current_depth,
        task_config.max_depth,
        if request.write_scope.is_empty() {
            task_config.write_scope.clone()
        } else {
            request.write_scope.clone()
        },
        request.targets.clone(),
        if request.result_contract.is_empty() {
            request.targets.clone()
        } else {
            request.result_contract.clone()
        },
    );
    let swarm_plan = SwarmPlan {
        budget: SwarmBudget {
            parallelism_budget: request.parallelism_budget,
        },
        targets: request.targets.clone(),
        write_scope: if request.write_scope.is_empty() {
            task_config.write_scope.clone()
        } else {
            request.write_scope.clone()
        },
        result_contract: if request.result_contract.is_empty() {
            task_config.result_contract.clone()
        } else {
            request.result_contract.clone()
        },
        validation_mode: request.validation_mode,
    };

    let runtime_plan = build_bounded_swarm_plan(
        &task_config.parent_session_id,
        &delegation,
        &swarm_plan,
        false,
    );
    if runtime_plan.workers.is_empty() {
        return Err(anyhow!("swarm planner produced no bounded workers"));
    }
    let worker_address_book = runtime_plan
        .workers
        .iter()
        .enumerate()
        .map(|(idx, worker)| {
            (
                worker_name_for_target(&worker.target_artifact, idx),
                worker.target_artifact.clone(),
            )
        })
        .collect::<Vec<_>>();
    let worker_peer_messaging_enabled =
        task_config.allow_worker_messaging && runtime_plan.workers.len() > 1;

    let permission_bridge_handle: Option<JoinHandle<()>> = if leader_permission_bridge_enabled() {
        Some(tokio::spawn(drive_permission_bridge(
            working_dir.clone(),
            run_id.clone(),
            permission_cancel.clone(),
        )))
    } else {
        None
    };

    let task_runtime = task_config
        .task_runtime
        .clone()
        .unwrap_or_else(TaskRuntime::shared);

    let spawn_limit = runtime_plan.parallelism_budget.max(1) as usize;
    let instructions = request.instructions.clone();
    let parallelism_budget = request.parallelism_budget;
    let mut jobs = FuturesUnordered::new();
    let mut next_worker_idx = 0usize;
    let launch_worker = |idx: usize,
                         worker: SwarmWorkerSpec,
                         task_config: TaskConfig,
                         task_runtime: std::sync::Arc<TaskRuntime>,
                         working_dir: PathBuf,
                         run_id: String,
                         leader_cancel: CancellationToken,
                         leader_name: String,
                         scratchpad: Option<SwarmScratchpad>,
                         instructions: String,
                         worker_address_book: Vec<(String, String)>,
                         worker_peer_messaging_enabled: bool| {
        let base_instructions = build_swarm_worker_instructions(
            &instructions,
            &worker.target_artifact,
            &worker.result_contract,
        );
        let worker_name = worker_name_for_target(&worker.target_artifact, idx);
        let peer_worker_addresses = peer_worker_addresses_from_book(&worker_address_book, idx);
        let peer_worker_catalog = peer_worker_catalog_from_book(&worker_address_book, idx);
        async move {
            let result = execute_worker_attempt(
                &task_config,
                task_runtime,
                &task_config.parent_session_id,
                &leader_name,
                parallelism_budget,
                &working_dir,
                &run_id,
                worker,
                worker_name,
                &leader_cancel,
                scratchpad.as_ref(),
                &base_instructions,
                worker_peer_messaging_enabled,
                peer_worker_addresses,
                peer_worker_catalog,
                None,
                None,
            )
            .await?;
            Ok::<(usize, WorkerExecutionResult), anyhow::Error>((idx, result))
        }
    };
    while next_worker_idx < runtime_plan.workers.len() && jobs.len() < spawn_limit {
        let idx = next_worker_idx;
        let worker = runtime_plan.workers[idx].clone();
        jobs.push(launch_worker(
            idx,
            worker,
            task_config.clone(),
            task_runtime.clone(),
            working_dir.clone(),
            run_id.clone(),
            leader_cancel.clone(),
            leader_name.clone(),
            scratchpad.clone(),
            instructions.clone(),
            worker_address_book.clone(),
            worker_peer_messaging_enabled,
        ));
        next_worker_idx += 1;
    }

    let mut worker_results = vec![None; runtime_plan.workers.len()];
    while let Some(result) = jobs.next().await {
        let (idx, worker_result) = result?;
        worker_results[idx] = Some(worker_result);
        if next_worker_idx < runtime_plan.workers.len() {
            let launch_idx = next_worker_idx;
            let worker = runtime_plan.workers[launch_idx].clone();
            jobs.push(launch_worker(
                launch_idx,
                worker,
                task_config.clone(),
                task_runtime.clone(),
                working_dir.clone(),
                run_id.clone(),
                leader_cancel.clone(),
                leader_name.clone(),
                scratchpad.clone(),
                instructions.clone(),
                worker_address_book.clone(),
                worker_peer_messaging_enabled,
            ));
            next_worker_idx += 1;
        }
    }

    let any_materializing_workers = runtime_plan.workers.iter().any(|worker| {
        worker_execution_expectation(worker) == WorkerExecutionExpectation::MaterializingChange
    });
    let validation_enabled = request.validation_mode
        || (any_materializing_workers
            && !swarm_plan.result_contract.is_empty()
            && runtime_plan.workers.len() > 1);
    let mut worker_summaries = Vec::new();
    let mut accepted_targets = Vec::new();
    let mut produced_targets = Vec::new();
    let mut validation_summaries = Vec::new();

    for (idx, worker) in runtime_plan.workers.iter().enumerate() {
        let mut worker_result = worker_results[idx]
            .take()
            .ok_or_else(|| anyhow!("missing swarm worker result for {}", worker.target_artifact))?;
        let execution_expectation = worker_execution_expectation(worker);

        if validation_enabled {
            if accepts_recovered_worker_summary(
                &worker_result.summary,
                worker_result.produced_delta,
            ) {
                validation_summaries.push(
                    "PASS: accepted recovered worker summary without additional validation retry"
                        .to_string(),
                );
                produced_targets.extend(worker_result.produced_targets.clone());
                worker_summaries.push(worker_result.summary.clone());
                continue;
            }
            let mut validation_outcome = execute_validation_worker(
                &task_config,
                task_runtime.clone(),
                &leader_name,
                &working_dir,
                &run_id,
                worker,
                idx,
                &leader_cancel,
                scratchpad.as_ref(),
                &worker_result.summary,
                None,
            )
            .await?;
            let mut validation_summary = summarize_validation_report(&validation_outcome);
            let needs_followup = (!worker_result.produced_delta
                && execution_expectation == WorkerExecutionExpectation::MaterializingChange)
                || !validation_outcome.passed();
            if needs_followup {
                let followup_kind = if worker_result.produced_delta && !validation_outcome.passed()
                {
                    WorkerFollowupKind::Correction
                } else if worker_result.summary.trim_start().starts_with("FAIL:") {
                    WorkerFollowupKind::Correction
                } else {
                    WorkerFollowupKind::Continue
                };
                let followup_reason = if !validation_outcome.passed() {
                    validation_outcome
                        .failure_reason()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| validation_summary.clone())
                } else {
                    worker_result.summary.clone()
                };
                let retry_base_instructions = build_swarm_worker_instructions(
                    &request.instructions,
                    &worker.target_artifact,
                    &worker.result_contract,
                );
                let _ = task_runtime
                    .record_followup_requested(
                        &worker.task_id,
                        match followup_kind {
                            WorkerFollowupKind::Continue => "continue",
                            WorkerFollowupKind::Correction => "correction",
                        },
                        followup_reason.clone(),
                    )
                    .await;
                worker_result = execute_worker_attempt(
                    &task_config,
                    task_runtime.clone(),
                    &task_config.parent_session_id,
                    &leader_name,
                    request.parallelism_budget,
                    &working_dir,
                    &run_id,
                    worker.clone(),
                    worker_name_for_target(&worker.target_artifact, idx),
                    &leader_cancel,
                    scratchpad.as_ref(),
                    &retry_base_instructions,
                    worker_peer_messaging_enabled,
                    peer_worker_addresses_from_book(&worker_address_book, idx),
                    peer_worker_catalog_from_book(&worker_address_book, idx),
                    Some((
                        followup_kind,
                        worker_result.summary.clone(),
                        followup_reason,
                    )),
                    Some("retry1"),
                )
                .await?;
                validation_outcome = execute_validation_worker(
                    &task_config,
                    task_runtime.clone(),
                    &leader_name,
                    &working_dir,
                    &run_id,
                    worker,
                    idx,
                    &leader_cancel,
                    scratchpad.as_ref(),
                    &worker_result.summary,
                    Some("retry1"),
                )
                .await?;
                validation_summary = summarize_validation_report(&validation_outcome);
            }
            validation_summaries.push(validation_summary.clone());
            if !validation_outcome.passed() {
                worker_result.produced_targets.clear();
                worker_result.produced_delta = false;
            }
        } else if execution_expectation == WorkerExecutionExpectation::MaterializingChange
            && !worker_result.produced_delta
            && !worker_result.summary.trim_start().starts_with("FAIL:")
        {
            let retry_base_instructions = build_swarm_worker_instructions(
                &request.instructions,
                &worker.target_artifact,
                &worker.result_contract,
            );
            let _ = task_runtime
                .record_followup_requested(
                    &worker.task_id,
                    "continue",
                    worker_result.summary.clone(),
                )
                .await;
            worker_result = execute_worker_attempt(
                &task_config,
                task_runtime.clone(),
                &task_config.parent_session_id,
                &leader_name,
                request.parallelism_budget,
                &working_dir,
                &run_id,
                worker.clone(),
                worker_name_for_target(&worker.target_artifact, idx),
                &leader_cancel,
                scratchpad.as_ref(),
                &retry_base_instructions,
                worker_peer_messaging_enabled,
                peer_worker_addresses_from_book(&worker_address_book, idx),
                peer_worker_catalog_from_book(&worker_address_book, idx),
                Some((
                    WorkerFollowupKind::Continue,
                    worker_result.summary.clone(),
                    worker_result.summary.clone(),
                )),
                Some("retry1"),
            )
            .await?;
        }

        worker_summaries.push(worker_result.summary.clone());
        accepted_targets.extend(worker_result.accepted_targets.clone());
        produced_targets.extend(worker_result.produced_targets.clone());
    }

    permission_cancel.cancel();
    if let Some(handle) = permission_bridge_handle {
        let _ = handle.await;
    }

    let outcome = decide_bounded_swarm_outcome(
        &swarm_plan.targets,
        &accepted_targets,
        Some(worker_summaries.join("\n\n")),
    );
    Ok(SwarmExecutionResult {
        run_id: run_id.clone(),
        worker_summaries,
        validation_summaries,
        accepted_targets: outcome.accepted_targets.clone(),
        produced_targets,
        downgraded: !outcome.produced_delta,
        downgrade_message: outcome.downgrade_message,
        scratchpad_path: scratchpad
            .as_ref()
            .map(|value| value.root.display().to_string()),
        mailbox_path: Some(mailbox_dir(&working_dir, &run_id).display().to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn precheck_materialized_validation_target_ignores_logical_targets() {
        assert!(
            precheck_materialized_validation_target(Path::new("."), "document:doc-1").is_none()
        );
    }

    #[test]
    fn precheck_materialized_validation_target_fails_when_workspace_file_missing() {
        let root = std::env::temp_dir().join(format!("validation-missing-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("workspace root");
        let report = precheck_materialized_validation_target(&root, "artifacts/out.md")
            .expect("missing artifact should fail");
        assert_eq!(
            report.reason_code.as_deref(),
            Some("artifact_not_materialized")
        );
        assert_eq!(
            report.target_artifacts,
            vec!["artifacts/out.md".to_string()]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn precheck_materialized_validation_target_accepts_existing_workspace_file() {
        let root = std::env::temp_dir().join(format!("validation-existing-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("artifacts")).expect("artifacts dir");
        fs::write(root.join("artifacts/out.md"), "# ok").expect("artifact file");
        assert!(precheck_materialized_validation_target(&root, "artifacts/out.md").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_recovered_worker_summary_only_when_delta_exists() {
        assert!(accepts_recovered_worker_summary(
            "Scope: recovered bounded worker trace\nResult: something\nBlocker: worker finished without an explicit terminal summary",
            true,
        ));
        assert!(!accepts_recovered_worker_summary(
            "Scope: recovered bounded worker trace\nResult: something\nBlocker: worker finished without an explicit terminal summary",
            false,
        ));
        assert!(!accepts_recovered_worker_summary(
            "PASS: validation passed",
            true,
        ));
    }
}
