use std::collections::HashMap;

use agime::agents::harness::{
    CoordinatorSignalSummary, HarnessTaskLedgerState, PersistedChildEvidenceItem, TaskKind,
    TaskSnapshot, TaskStatus, WorkerAttemptIdentity, WorkerOutcome,
};
use agime::utils::{normalize_delegation_summary_text, safe_truncate};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationRuntimeStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl DelegationRuntimeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationRuntimeMode {
    Subagent,
    Swarm,
}

impl DelegationRuntimeMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Subagent => "subagent",
            Self::Swarm => "swarm",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegationRuntimeWorkerResponse {
    pub worker_id: String,
    pub role: String,
    pub title: String,
    pub status: DelegationRuntimeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegationRuntimeResponse {
    pub active_run: bool,
    pub mode: DelegationRuntimeMode,
    pub status: DelegationRuntimeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leader: Option<DelegationRuntimeWorkerResponse>,
    #[serde(default)]
    pub workers: Vec<DelegationRuntimeWorkerResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegationRuntimeEventPayload {
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<DelegationRuntimeMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<DelegationRuntimeStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<DelegationRuntimeWorkerResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_message_id: Option<String>,
}

fn normalize_status(status: TaskStatus) -> DelegationRuntimeStatus {
    match status {
        TaskStatus::Pending => DelegationRuntimeStatus::Pending,
        TaskStatus::Running => DelegationRuntimeStatus::Running,
        TaskStatus::Completed => DelegationRuntimeStatus::Completed,
        TaskStatus::Failed | TaskStatus::Cancelled => DelegationRuntimeStatus::Failed,
    }
}

fn normalize_persisted_status(status: &str) -> DelegationRuntimeStatus {
    let normalized = status.trim().to_ascii_lowercase();
    if normalized.contains("running") {
        DelegationRuntimeStatus::Running
    } else if normalized.contains("pending") || normalized.contains("queued") {
        DelegationRuntimeStatus::Pending
    } else if normalized.contains("complete") || normalized.contains("success") {
        DelegationRuntimeStatus::Completed
    } else if normalized.contains("fail")
        || normalized.contains("cancel")
        || normalized.contains("block")
        || normalized.contains("error")
    {
        DelegationRuntimeStatus::Failed
    } else {
        DelegationRuntimeStatus::Completed
    }
}

fn task_role(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Subagent => "subagent",
        TaskKind::SwarmWorker => "swarm_worker",
        TaskKind::ValidationWorker => "validation_worker",
    }
}

fn derive_mode(tasks: &[TaskSnapshot]) -> DelegationRuntimeMode {
    if tasks.len() <= 1
        && tasks
            .iter()
            .all(|task| matches!(task.kind, TaskKind::Subagent))
    {
        DelegationRuntimeMode::Subagent
    } else {
        DelegationRuntimeMode::Swarm
    }
}

fn derive_overall_status(tasks: &[TaskSnapshot]) -> DelegationRuntimeStatus {
    if tasks
        .iter()
        .any(|task| matches!(task.status, TaskStatus::Running))
    {
        return DelegationRuntimeStatus::Running;
    }
    if tasks
        .iter()
        .any(|task| matches!(task.status, TaskStatus::Pending))
    {
        return DelegationRuntimeStatus::Pending;
    }
    if tasks
        .iter()
        .any(|task| matches!(task.status, TaskStatus::Failed | TaskStatus::Cancelled))
    {
        return DelegationRuntimeStatus::Failed;
    }
    DelegationRuntimeStatus::Completed
}

fn format_unix_timestamp(value: i64) -> Option<String> {
    let dt = if value > 1_000_000_000_000 {
        DateTime::<Utc>::from_timestamp_millis(value)
    } else {
        DateTime::<Utc>::from_timestamp(value, 0)
    }?;
    Some(dt.to_rfc3339())
}

fn compact_summary_line(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = normalize_delegation_summary_text(raw);
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.replace('\n', " "))
        }
    })
}

fn canonicalize_display_worker_id(worker_id: &str) -> String {
    let trimmed = worker_id.trim();
    let Some(rest) = trimmed.strip_prefix("worker_") else {
        return trimmed.to_string();
    };
    let Some((index, raw_target)) = rest.split_once('_') else {
        return trimmed.to_string();
    };
    if !index.chars().all(|item| item.is_ascii_digit()) {
        return trimmed.to_string();
    }

    let mut target = raw_target;
    for separator in ["__", "_бк_", "_БК_", "_бд_", "_БД_"] {
        if let Some((head, _)) = target.split_once(separator) {
            target = head;
        }
    }

    let normalized = target.trim_matches('_').replace('\\', "/");
    if normalized.is_empty() {
        trimmed.to_string()
    } else {
        format!("worker_{}_{}", index, normalized)
    }
}

fn worker_target_label(worker: &DelegationRuntimeWorkerResponse) -> Option<String> {
    canonicalize_display_worker_id(&worker.worker_id)
        .strip_prefix("worker_")
        .and_then(|rest| rest.split_once('_').map(|(_, target)| target))
        .map(|target| target.trim().replace('_', "/"))
        .filter(|target| !target.is_empty())
}

fn extract_worker_index(value: &str) -> Option<&str> {
    value
        .strip_prefix("worker_")
        .and_then(|rest| rest.split('_').next())
        .filter(|index| index.chars().all(|item| item.is_ascii_digit()))
}

fn display_worker_title(worker_id: &str, title: &str, role: &str) -> String {
    let trimmed = title.trim();
    if let Some(index) = extract_worker_index(worker_id) {
        if trimmed.is_empty()
            || trimmed == worker_id
            || trimmed.starts_with("worker_")
            || matches!(trimmed, "Subagent" | "Swarm Worker" | "Validation Worker")
        {
            return format!("Worker {}", index);
        }
    }
    if role == "subagent" && (trimmed.is_empty() || trimmed == worker_id || trimmed == "Subagent") {
        return "Worker".to_string();
    }
    if trimmed.is_empty() {
        return worker_id.to_string();
    }
    trimmed.to_string()
}

fn extract_structured_field(text: &str, label: &str) -> Option<String> {
    const LABELS: [&str; 4] = ["Scope:", "Result:", "Artifacts changed:", "Blocker:"];
    let compact = text.replace('\n', " ");
    let start = compact.find(label)?;
    let value_start = start + label.len();
    let rest = compact.get(value_start..)?.trim_start();
    let next_boundary = LABELS
        .iter()
        .filter(|candidate| **candidate != label)
        .filter_map(|candidate| rest.find(candidate))
        .min()
        .unwrap_or(rest.len());
    let value = rest[..next_boundary].trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn internal_artifact_item(item: &str) -> bool {
    let normalized = item.trim().replace('\\', "/").to_ascii_lowercase();
    normalized.is_empty()
        || normalized.contains("/mailbox")
        || normalized.contains("mailbox/")
        || normalized.contains("/scratchpad")
        || normalized.contains("scratchpad/")
        || normalized.contains("/tmp/")
        || normalized.starts_with("/tmp/")
        || normalized.contains(".agime/")
        || normalized.contains("/.agime/")
}

fn refine_artifacts_changed(value: &str) -> Option<String> {
    let items = value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty() && !internal_artifact_item(item))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    (!items.is_empty()).then(|| items.join(", "))
}

fn internal_execution_blocker(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized.contains("read-only")
        || normalized.contains("read only")
        || normalized.contains("cannot write")
        || normalized.contains("can't write")
        || normalized.contains("cannot create file")
        || normalized.contains("can't create file")
        || normalized.contains("write scope")
        || normalized.contains("permission policy")
        || normalized.contains("mailbox")
        || normalized.contains("scratchpad")
        || normalized.contains("bounded worker")
        || normalized.contains("coordinator-only")
        || normalized.contains("coordinator only")
}

fn concise_worker_detail(worker: &DelegationRuntimeWorkerResponse) -> Option<String> {
    let raw = worker
        .result_summary
        .as_deref()
        .or(worker.summary.as_deref())
        .map(normalize_delegation_summary_text)?;
    if raw.trim().is_empty() {
        return None;
    }

    let result = extract_structured_field(&raw, "Result:").or_else(|| {
        raw.lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(ToString::to_string)
    });
    let blocker = extract_structured_field(&raw, "Blocker:")
        .filter(|value| !internal_execution_blocker(value));
    let artifacts = extract_structured_field(&raw, "Artifacts changed:")
        .and_then(|value| refine_artifacts_changed(&value));
    let target = worker_target_label(worker);
    let compact_result = result.as_deref().and_then(|value| {
        let normalized = normalize_delegation_summary_text(value);
        if normalized.is_empty() {
            return None;
        }
        if normalized.contains("Contents: Files:") {
            let files = normalized
                .split("Contents: Files:")
                .nth(1)
                .unwrap_or_default()
                .split_whitespace()
                .take(4)
                .collect::<Vec<_>>();
            if !files.is_empty() {
                return Some(format!("contains {}", files.join(", ")));
            }
        }
        if normalized.starts_with('#')
            || normalized.starts_with("/repo/")
            || normalized.starts_with("/opt/")
            || normalized.contains("```")
            || normalized.contains("### ")
            || normalized.contains("| ")
            || normalized.len() > 180
        {
            return target
                .as_ref()
                .map(|value| format!("inspected {}", value))
                .or_else(|| Some("inspected successfully".to_string()));
        }
        Some(safe_truncate(normalized.trim_matches('`'), 140))
    });

    let mut detail = compact_result.unwrap_or_else(|| {
        target
            .as_ref()
            .map(|value| format!("inspected {}", value))
            .unwrap_or(raw)
    });
    if let Some(blocker) = blocker.filter(|value| !value.is_empty()) {
        detail.push_str(" Blocker: ");
        detail.push_str(&blocker);
    }
    if let Some(artifacts) = artifacts.filter(|value| !value.is_empty()) {
        detail.push_str(" Artifacts changed: ");
        detail.push_str(&artifacts);
    }
    Some(detail)
}

fn presentable_worker_title(worker: &DelegationRuntimeWorkerResponse) -> String {
    display_worker_title(&worker.worker_id, &worker.title, &worker.role)
}

fn evidence_map(
    evidence: &[PersistedChildEvidenceItem],
) -> HashMap<&str, &PersistedChildEvidenceItem> {
    evidence
        .iter()
        .map(|item| (item.task_id.as_str(), item))
        .collect()
}

fn build_worker_title(task: &TaskSnapshot) -> String {
    if let Some(description) = compact_summary_line(task.description.as_deref()) {
        return description;
    }
    if let Some(identity) = WorkerAttemptIdentity::from_metadata(&task.metadata) {
        return identity.logical_worker_id;
    }
    match task.kind {
        TaskKind::Subagent => "Subagent".to_string(),
        TaskKind::SwarmWorker => "Swarm Worker".to_string(),
        TaskKind::ValidationWorker => "Validation Worker".to_string(),
    }
}

fn build_worker_response(
    task: &TaskSnapshot,
    evidence: Option<&PersistedChildEvidenceItem>,
) -> DelegationRuntimeWorkerResponse {
    let runtime_detail = compact_summary_line(
        task.metadata
            .get("runtime_detail")
            .map(String::as_str)
            .or_else(|| evidence.and_then(|item| item.runtime_detail.as_deref())),
    );
    let preview = compact_summary_line(
        task.metadata
            .get("child_session_preview")
            .map(String::as_str)
            .or_else(|| evidence.and_then(|item| item.session_preview.as_deref())),
    );
    let summary = compact_summary_line(task.summary.as_deref())
        .or_else(|| evidence.and_then(|item| compact_summary_line(item.summary.as_deref())));
    let status = normalize_status(task.status);
    let recovered_terminal_summary = task
        .metadata
        .get("recovered_terminal_summary")
        .map(|value| value == "true")
        .or_else(|| evidence.map(|item| item.recovered_terminal_summary))
        .unwrap_or(false);
    let error = if matches!(status, DelegationRuntimeStatus::Failed) {
        runtime_detail
            .clone()
            .or_else(|| summary.clone())
            .or_else(|| preview.clone())
            .map(|value| normalize_delegation_summary_text(&value))
    } else {
        None
    };
    let result_summary = if matches!(status, DelegationRuntimeStatus::Completed) {
        if recovered_terminal_summary {
            Some("completed via recovered worker trace".to_string())
        } else {
            summary
                .clone()
                .or_else(|| preview.clone())
                .map(|value| normalize_delegation_summary_text(&value))
        }
    } else {
        None
    };
    let worker_summary = match status {
        DelegationRuntimeStatus::Pending => preview.or(runtime_detail.clone()),
        DelegationRuntimeStatus::Running => runtime_detail.clone().or(preview),
        DelegationRuntimeStatus::Completed => summary.clone().or(runtime_detail.clone()),
        DelegationRuntimeStatus::Failed => error.clone().or(summary.clone()),
    }
    .map(|value| normalize_delegation_summary_text(&value));
    let worker_id = WorkerAttemptIdentity::from_metadata(&task.metadata)
        .map(|identity| identity.logical_worker_id)
        .unwrap_or_else(|| task.task_id.clone());
    let worker_id = canonicalize_display_worker_id(&worker_id);
    let role = task_role(task.kind).to_string();
    let title = display_worker_title(&worker_id, &build_worker_title(task), &role);

    DelegationRuntimeWorkerResponse {
        worker_id,
        role,
        title,
        status,
        summary: worker_summary,
        last_update_at: format_unix_timestamp(task.updated_at),
        result_summary,
        error,
    }
}

fn derive_mode_from_evidence(evidence: &[PersistedChildEvidenceItem]) -> DelegationRuntimeMode {
    if evidence.len() <= 1 {
        DelegationRuntimeMode::Subagent
    } else {
        DelegationRuntimeMode::Swarm
    }
}

fn derive_overall_status_from_workers(
    workers: &[DelegationRuntimeWorkerResponse],
) -> DelegationRuntimeStatus {
    if workers
        .iter()
        .any(|worker| matches!(worker.status, DelegationRuntimeStatus::Running))
    {
        return DelegationRuntimeStatus::Running;
    }
    if workers
        .iter()
        .any(|worker| matches!(worker.status, DelegationRuntimeStatus::Pending))
    {
        return DelegationRuntimeStatus::Pending;
    }
    if workers
        .iter()
        .any(|worker| matches!(worker.status, DelegationRuntimeStatus::Failed))
    {
        return DelegationRuntimeStatus::Failed;
    }
    DelegationRuntimeStatus::Completed
}

fn build_persisted_worker_response(
    item: &PersistedChildEvidenceItem,
    mode: DelegationRuntimeMode,
) -> DelegationRuntimeWorkerResponse {
    let status = normalize_persisted_status(&item.status);
    let summary = compact_summary_line(item.summary.as_deref());
    let preview = compact_summary_line(item.session_preview.as_deref());
    let runtime_detail = compact_summary_line(item.runtime_detail.as_deref());
    let worker_summary = match status {
        DelegationRuntimeStatus::Pending => preview.clone().or(runtime_detail.clone()),
        DelegationRuntimeStatus::Running => runtime_detail.clone().or(preview.clone()),
        DelegationRuntimeStatus::Completed => summary.clone().or(preview.clone()),
        DelegationRuntimeStatus::Failed => runtime_detail
            .clone()
            .or(summary.clone())
            .or(preview.clone()),
    }
    .map(|value| normalize_delegation_summary_text(&value));

    let worker_id = item
        .logical_worker_id
        .clone()
        .unwrap_or_else(|| item.task_id.clone());
    let worker_id = canonicalize_display_worker_id(&worker_id);
    let role = match mode {
        DelegationRuntimeMode::Subagent => "subagent".to_string(),
        DelegationRuntimeMode::Swarm => "swarm_worker".to_string(),
    };
    let title = display_worker_title(&worker_id, &worker_id, &role);

    DelegationRuntimeWorkerResponse {
        worker_id: worker_id.clone(),
        role,
        title,
        status: status.clone(),
        summary: worker_summary,
        last_update_at: None,
        result_summary: if matches!(status, DelegationRuntimeStatus::Completed) {
            if item.recovered_terminal_summary {
                Some("completed via recovered worker trace".to_string())
            } else {
                summary.clone().or(preview.clone())
            }
        } else {
            None
        }
        .map(|value| normalize_delegation_summary_text(&value)),
        error: if matches!(status, DelegationRuntimeStatus::Failed) {
            runtime_detail.clone().or(summary.clone())
        } else {
            None
        }
        .map(|value| normalize_delegation_summary_text(&value)),
    }
}

fn derive_mode_from_worker_outcomes(outcomes: &[WorkerOutcome]) -> DelegationRuntimeMode {
    if outcomes.len() <= 1
        && outcomes
            .iter()
            .all(|item| matches!(item.kind, TaskKind::Subagent))
    {
        DelegationRuntimeMode::Subagent
    } else {
        DelegationRuntimeMode::Swarm
    }
}

fn build_worker_response_from_outcome(outcome: &WorkerOutcome) -> DelegationRuntimeWorkerResponse {
    let status = normalize_persisted_status(&outcome.status);
    let summary = compact_summary_line(Some(outcome.summary.as_str()));
    let worker_id = outcome
        .attempt_identity
        .as_ref()
        .map(|identity| identity.logical_worker_id.clone())
        .unwrap_or_else(|| outcome.task_id.clone());
    let worker_id = canonicalize_display_worker_id(&worker_id);
    let role = task_role(outcome.kind).to_string();
    let title = display_worker_title(&worker_id, &worker_id, &role);
    let result_summary = if matches!(status, DelegationRuntimeStatus::Completed) {
        if outcome.recovered_terminal_summary {
            Some("completed via recovered worker trace".to_string())
        } else {
            summary.clone()
        }
    } else {
        None
    };
    let error = if matches!(status, DelegationRuntimeStatus::Failed) {
        summary.clone()
    } else {
        None
    };

    DelegationRuntimeWorkerResponse {
        worker_id,
        role,
        title,
        status,
        summary,
        last_update_at: None,
        result_summary,
        error,
    }
}

fn is_generic_leader_like_summary(summary: &str) -> bool {
    let trimmed = summary.trim();
    trimmed.starts_with("Swarm run `") || trimmed.starts_with("Subagent run `")
}

fn worker_response_score(worker: &DelegationRuntimeWorkerResponse) -> usize {
    let mut score = 0usize;
    if !worker
        .summary
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .is_empty()
    {
        score += 2;
    }
    if !worker
        .result_summary
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .is_empty()
    {
        score += 3;
    }
    if !worker
        .error
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .is_empty()
    {
        score += 2;
    }
    score += worker
        .summary
        .as_deref()
        .map(str::len)
        .unwrap_or_default()
        .min(200);
    score
}

fn is_control_worker(worker: &DelegationRuntimeWorkerResponse) -> bool {
    let worker_id = worker.worker_id.trim();
    let title = worker.title.trim().to_ascii_lowercase();
    let summary = worker
        .summary
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    worker_id.starts_with("auto_swarm_")
        || worker_id.starts_with("auto_subagent_")
        || title == "child task for swarm"
        || title == "child task for subagent"
        || summary.contains("child task for swarm")
        || summary.contains("child task for subagent")
}

fn normalize_workers(
    leader_summary: Option<&str>,
    workers: Vec<DelegationRuntimeWorkerResponse>,
) -> Vec<DelegationRuntimeWorkerResponse> {
    let leader_summary = leader_summary
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut best_by_id: HashMap<String, DelegationRuntimeWorkerResponse> = HashMap::new();

    for worker in workers {
        if worker.role == "validation_worker" {
            continue;
        }
        if is_control_worker(&worker) {
            continue;
        }
        let worker_summary = worker.summary.as_deref().map(str::trim).unwrap_or_default();
        let has_stable_worker_name =
            worker.worker_id.starts_with("worker_") || worker.title.starts_with("worker_");
        if worker.worker_id.starts_with("call_") && worker_summary.starts_with("Swarm run `") {
            continue;
        }
        let synthetic_leader_like = !has_stable_worker_name
            && is_generic_leader_like_summary(worker_summary)
            && leader_summary
                .map(|leader| worker_summary == leader || leader.starts_with(worker_summary))
                .unwrap_or(true);
        if synthetic_leader_like {
            continue;
        }

        match best_by_id.get(worker.worker_id.as_str()) {
            Some(existing) if worker_response_score(existing) >= worker_response_score(&worker) => {
            }
            _ => {
                best_by_id.insert(worker.worker_id.clone(), worker);
            }
        }
    }

    let mut normalized = best_by_id.into_values().collect::<Vec<_>>();
    normalized.sort_by(|left, right| left.title.cmp(&right.title));
    normalized
}

fn leader_summary_needs_enrichment(summary: &str) -> bool {
    let trimmed = summary.trim();
    trimmed.starts_with("swarm completed with ")
        || trimmed.starts_with("subagent completed")
        || trimmed.starts_with("Bounded swarm produced ")
        || trimmed.starts_with("Swarm run `")
        || trimmed.starts_with("Subagent run `")
        || trimmed.starts_with("Swarm completed for ")
        || trimmed.starts_with("Subagent completed for ")
        || trimmed.contains("\n## ")
        || trimmed.contains("\n|")
        || trimmed.contains("\n---")
        || trimmed.len() > 320
}

fn synthesize_leader_summary_from_workers(
    mode: &DelegationRuntimeMode,
    workers: &[DelegationRuntimeWorkerResponse],
) -> Option<String> {
    let mut lines = workers
        .iter()
        .filter_map(|worker| {
            let detail = concise_worker_detail(worker)?;
            Some(format!("{}: {}", presentable_worker_title(worker), detail))
        })
        .take(2)
        .collect::<Vec<_>>();

    if lines.is_empty() {
        return None;
    }

    let heading = match mode {
        DelegationRuntimeMode::Subagent => "Worker completed. Key result:",
        DelegationRuntimeMode::Swarm => "Swarm completed. Key findings:",
    };
    let mut summary = String::from(heading);
    for line in lines.drain(..) {
        summary.push_str("\n- ");
        summary.push_str(&line);
    }
    Some(summary)
}

fn finalize_leader_summary(
    leader_summary: Option<String>,
    mode: &DelegationRuntimeMode,
    workers: &[DelegationRuntimeWorkerResponse],
) -> Option<String> {
    let synthesized = synthesize_leader_summary_from_workers(mode, workers);
    let summary = match (mode, synthesized, leader_summary) {
        (DelegationRuntimeMode::Swarm, Some(summary), _) => Some(summary),
        (_, Some(summary), Some(existing)) if leader_summary_needs_enrichment(&existing) => {
            Some(summary)
        }
        (_, Some(summary), None) => Some(summary),
        (_, _, leader_summary) => leader_summary,
    };
    summary.map(|value| normalize_delegation_summary_text(&value))
}

fn build_runtime_summary(
    status: DelegationRuntimeStatus,
    mode: DelegationRuntimeMode,
    tasks: &[TaskSnapshot],
) -> String {
    let running = tasks
        .iter()
        .filter(|task| matches!(task.status, TaskStatus::Running))
        .count();
    let pending = tasks
        .iter()
        .filter(|task| matches!(task.status, TaskStatus::Pending))
        .count();
    let completed = tasks
        .iter()
        .filter(|task| matches!(task.status, TaskStatus::Completed))
        .count();
    let failed = tasks
        .iter()
        .filter(|task| matches!(task.status, TaskStatus::Failed | TaskStatus::Cancelled))
        .count();
    let mode_label = match mode {
        DelegationRuntimeMode::Subagent => "Subagent",
        DelegationRuntimeMode::Swarm => "Swarm",
    };
    match status {
        DelegationRuntimeStatus::Running => format!(
            "{} running: {} active{}{}",
            mode_label,
            running,
            if pending > 0 {
                format!(", {} pending", pending)
            } else {
                String::new()
            },
            if completed > 0 {
                format!(", {} completed", completed)
            } else {
                String::new()
            }
        ),
        DelegationRuntimeStatus::Pending => {
            format!(
                "{} pending: {} worker(s)",
                mode_label,
                pending.max(tasks.len())
            )
        }
        DelegationRuntimeStatus::Completed => {
            format!(
                "{} completed: {} worker(s)",
                mode_label,
                completed.max(tasks.len())
            )
        }
        DelegationRuntimeStatus::Failed => format!(
            "{} failed: {} worker(s) failed{}",
            mode_label,
            failed.max(1),
            if completed > 0 {
                format!(", {} completed", completed)
            } else {
                String::new()
            }
        ),
    }
}

fn build_runtime_summary_from_workers(
    status: DelegationRuntimeStatus,
    mode: DelegationRuntimeMode,
    workers: &[DelegationRuntimeWorkerResponse],
) -> String {
    let running = workers
        .iter()
        .filter(|worker| matches!(worker.status, DelegationRuntimeStatus::Running))
        .count();
    let pending = workers
        .iter()
        .filter(|worker| matches!(worker.status, DelegationRuntimeStatus::Pending))
        .count();
    let completed = workers
        .iter()
        .filter(|worker| matches!(worker.status, DelegationRuntimeStatus::Completed))
        .count();
    let failed = workers
        .iter()
        .filter(|worker| matches!(worker.status, DelegationRuntimeStatus::Failed))
        .count();
    let mode_label = match mode {
        DelegationRuntimeMode::Subagent => "Subagent",
        DelegationRuntimeMode::Swarm => "Swarm",
    };
    match status {
        DelegationRuntimeStatus::Running => format!(
            "{} running: {} active{}{}",
            mode_label,
            running,
            if pending > 0 {
                format!(", {} pending", pending)
            } else {
                String::new()
            },
            if completed > 0 {
                format!(", {} completed", completed)
            } else {
                String::new()
            }
        ),
        DelegationRuntimeStatus::Pending => {
            format!(
                "{} pending: {} worker(s)",
                mode_label,
                pending.max(workers.len())
            )
        }
        DelegationRuntimeStatus::Completed => format!(
            "{} completed: {} worker(s)",
            mode_label,
            completed.max(workers.len())
        ),
        DelegationRuntimeStatus::Failed => format!(
            "{} failed: {} worker(s) failed{}",
            mode_label,
            failed.max(1),
            if completed > 0 {
                format!(", {} completed", completed)
            } else {
                String::new()
            }
        ),
    }
}

pub fn build_delegation_runtime(
    leader_title: Option<&str>,
    leader_status: Option<&str>,
    leader_summary: Option<&str>,
    leader_error: Option<&str>,
    ledger: Option<&HarnessTaskLedgerState>,
    persisted_child_evidence: Option<&[PersistedChildEvidenceItem]>,
) -> Option<DelegationRuntimeResponse> {
    let tasks = ledger
        .map(|state| state.tasks.as_slice())
        .unwrap_or_default()
        .iter()
        .filter(|task| {
            matches!(
                task.kind,
                TaskKind::Subagent | TaskKind::SwarmWorker | TaskKind::ValidationWorker
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    let evidence = persisted_child_evidence.unwrap_or(&[]);
    let (mode, status, workers) = if tasks.is_empty() {
        if evidence.is_empty() {
            return None;
        }
        let mode = derive_mode_from_evidence(evidence);
        let workers = evidence
            .iter()
            .map(|item| build_persisted_worker_response(item, mode.clone()))
            .collect::<Vec<_>>();
        let status = derive_overall_status_from_workers(&workers);
        (mode, status, workers)
    } else {
        let evidence_by_task = evidence_map(evidence);
        let mode = derive_mode(&tasks);
        let status = derive_overall_status(&tasks);
        let workers = tasks
            .iter()
            .map(|task| {
                build_worker_response(task, evidence_by_task.get(task.task_id.as_str()).copied())
            })
            .collect::<Vec<_>>();
        (mode, status, workers)
    };
    let leader_summary = compact_summary_line(leader_summary);
    let workers = normalize_workers(leader_summary.as_deref(), workers);
    let leader_summary = finalize_leader_summary(leader_summary, &mode, &workers);
    let leader = Some(DelegationRuntimeWorkerResponse {
        worker_id: "leader".to_string(),
        role: "leader".to_string(),
        title: leader_title.unwrap_or("Leader").to_string(),
        status: match leader_status.map(|value| value.trim().to_ascii_lowercase()) {
            Some(status) if status == "failed" || status == "blocked" => {
                DelegationRuntimeStatus::Failed
            }
            Some(status) if status == "running" => DelegationRuntimeStatus::Running,
            Some(status) if status == "pending" => DelegationRuntimeStatus::Pending,
            _ => status.clone(),
        },
        summary: leader_summary.clone(),
        last_update_at: None,
        result_summary: if matches!(status, DelegationRuntimeStatus::Completed) {
            leader_summary.clone()
        } else {
            None
        },
        error: compact_summary_line(leader_error),
    });

    let runtime_summary =
        build_runtime_summary_from_workers(status.clone(), mode.clone(), &workers);

    Some(DelegationRuntimeResponse {
        active_run: matches!(
            status,
            DelegationRuntimeStatus::Pending | DelegationRuntimeStatus::Running
        ),
        summary: Some(normalize_delegation_summary_text(&runtime_summary)),
        mode,
        status,
        leader,
        workers,
    })
}

pub fn build_delegation_runtime_from_signal_summary(
    leader_title: Option<&str>,
    leader_status: Option<&str>,
    leader_summary: Option<&str>,
    leader_error: Option<&str>,
    signal_summary: Option<&CoordinatorSignalSummary>,
    persisted_child_evidence: Option<&[PersistedChildEvidenceItem]>,
) -> Option<DelegationRuntimeResponse> {
    let outcomes = signal_summary
        .map(|summary| summary.worker_outcomes.as_slice())
        .unwrap_or_default();
    if outcomes.is_empty() {
        return build_delegation_runtime(
            leader_title,
            leader_status,
            leader_summary,
            leader_error,
            None,
            persisted_child_evidence,
        );
    }

    let mode = derive_mode_from_worker_outcomes(outcomes);
    let workers = outcomes
        .iter()
        .map(build_worker_response_from_outcome)
        .collect::<Vec<_>>();
    let leader_summary = compact_summary_line(leader_summary);
    let workers = normalize_workers(leader_summary.as_deref(), workers);
    let leader_summary = finalize_leader_summary(leader_summary, &mode, &workers);
    let status = derive_overall_status_from_workers(&workers);
    let leader = Some(DelegationRuntimeWorkerResponse {
        worker_id: "leader".to_string(),
        role: "leader".to_string(),
        title: leader_title.unwrap_or("Leader").to_string(),
        status: match leader_status.map(|value| value.trim().to_ascii_lowercase()) {
            Some(status) if status == "failed" || status == "blocked" => {
                DelegationRuntimeStatus::Failed
            }
            Some(status) if status == "running" => DelegationRuntimeStatus::Running,
            Some(status) if status == "pending" => DelegationRuntimeStatus::Pending,
            _ => status.clone(),
        },
        summary: leader_summary.clone(),
        last_update_at: None,
        result_summary: if matches!(status, DelegationRuntimeStatus::Completed) {
            leader_summary.clone()
        } else {
            None
        },
        error: compact_summary_line(leader_error),
    });

    Some(DelegationRuntimeResponse {
        active_run: matches!(
            status,
            DelegationRuntimeStatus::Pending | DelegationRuntimeStatus::Running
        ),
        summary: Some(normalize_delegation_summary_text(
            &build_runtime_summary_from_workers(status.clone(), mode.clone(), &workers),
        )),
        mode,
        status,
        leader,
        workers,
    })
}

pub fn delegation_event_from_worker_stream(
    event: &crate::agent::task_manager::StreamEvent,
    thread_root_id: Option<&str>,
    root_message_id: Option<&str>,
) -> Option<DelegationRuntimeEventPayload> {
    use crate::agent::task_manager::StreamEvent;

    let make_worker = |worker_id: String,
                       role: String,
                       title: String,
                       status: DelegationRuntimeStatus,
                       summary: Option<String>,
                       result_summary: Option<String>,
                       error: Option<String>|
     -> DelegationRuntimeWorkerResponse {
        DelegationRuntimeWorkerResponse {
            worker_id,
            role,
            title,
            status,
            summary,
            last_update_at: None,
            result_summary,
            error,
        }
    };

    let with_context = |mut payload: DelegationRuntimeEventPayload| {
        payload.thread_root_id = thread_root_id.map(str::to_string);
        payload.root_message_id = root_message_id.map(str::to_string);
        payload
    };

    match event {
        StreamEvent::WorkerStarted {
            task_id,
            kind,
            target,
            logical_worker_id,
            ..
        } => Some(with_context(DelegationRuntimeEventPayload {
            event: "worker_started".to_string(),
            mode: Some(if kind == "subagent" {
                DelegationRuntimeMode::Subagent
            } else {
                DelegationRuntimeMode::Swarm
            }),
            status: Some(DelegationRuntimeStatus::Running),
            summary: None,
            worker: Some(make_worker(
                logical_worker_id.clone().unwrap_or_else(|| task_id.clone()),
                kind.clone(),
                target.clone().unwrap_or_else(|| {
                    logical_worker_id.clone().unwrap_or_else(|| task_id.clone())
                }),
                DelegationRuntimeStatus::Running,
                None,
                None,
                None,
            )),
            thread_root_id: None,
            root_message_id: None,
        })),
        StreamEvent::WorkerProgress {
            task_id, message, ..
        } => Some(with_context(DelegationRuntimeEventPayload {
            event: "worker_progress".to_string(),
            mode: None,
            status: Some(DelegationRuntimeStatus::Running),
            summary: None,
            worker: Some(make_worker(
                task_id.clone(),
                "worker".to_string(),
                task_id.clone(),
                DelegationRuntimeStatus::Running,
                compact_summary_line(Some(message.as_str())),
                None,
                None,
            )),
            thread_root_id: None,
            root_message_id: None,
        })),
        StreamEvent::WorkerFollowup {
            task_id,
            kind,
            reason,
            logical_worker_id,
            ..
        } => Some(with_context(DelegationRuntimeEventPayload {
            event: "worker_followup".to_string(),
            mode: Some(DelegationRuntimeMode::Swarm),
            status: Some(DelegationRuntimeStatus::Running),
            summary: None,
            worker: Some(make_worker(
                logical_worker_id.clone().unwrap_or_else(|| task_id.clone()),
                "worker".to_string(),
                kind.clone(),
                DelegationRuntimeStatus::Running,
                compact_summary_line(Some(reason.as_str())),
                None,
                None,
            )),
            thread_root_id: None,
            root_message_id: None,
        })),
        StreamEvent::WorkerIdle {
            task_id,
            message,
            logical_worker_id,
            ..
        } => Some(with_context(DelegationRuntimeEventPayload {
            event: "worker_idle".to_string(),
            mode: None,
            status: Some(DelegationRuntimeStatus::Pending),
            summary: None,
            worker: Some(make_worker(
                logical_worker_id.clone().unwrap_or_else(|| task_id.clone()),
                "worker".to_string(),
                logical_worker_id.clone().unwrap_or_else(|| task_id.clone()),
                DelegationRuntimeStatus::Pending,
                compact_summary_line(Some(message.as_str())),
                None,
                None,
            )),
            thread_root_id: None,
            root_message_id: None,
        })),
        StreamEvent::WorkerFinished {
            task_id,
            kind,
            status,
            summary,
            logical_worker_id,
            ..
        } => {
            let normalized_status = if status.eq_ignore_ascii_case("completed") {
                DelegationRuntimeStatus::Completed
            } else {
                DelegationRuntimeStatus::Failed
            };
            Some(with_context(DelegationRuntimeEventPayload {
                event: "worker_finished".to_string(),
                mode: Some(if kind == "subagent" {
                    DelegationRuntimeMode::Subagent
                } else {
                    DelegationRuntimeMode::Swarm
                }),
                status: Some(normalized_status.clone()),
                summary: None,
                worker: Some(make_worker(
                    logical_worker_id.clone().unwrap_or_else(|| task_id.clone()),
                    kind.clone(),
                    logical_worker_id.clone().unwrap_or_else(|| task_id.clone()),
                    normalized_status.clone(),
                    if matches!(normalized_status, DelegationRuntimeStatus::Completed) {
                        None
                    } else {
                        compact_summary_line(Some(summary.as_str()))
                    },
                    if matches!(normalized_status, DelegationRuntimeStatus::Completed) {
                        compact_summary_line(Some(summary.as_str()))
                    } else {
                        None
                    },
                    if matches!(normalized_status, DelegationRuntimeStatus::Failed) {
                        compact_summary_line(Some(summary.as_str()))
                    } else {
                        None
                    },
                )),
                thread_root_id: None,
                root_message_id: None,
            }))
        }
        StreamEvent::PermissionRequested {
            task_id,
            tool_name,
            logical_worker_id,
            ..
        } => Some(with_context(DelegationRuntimeEventPayload {
            event: "permission_requested".to_string(),
            mode: None,
            status: Some(DelegationRuntimeStatus::Running),
            summary: None,
            worker: Some(make_worker(
                logical_worker_id.clone().unwrap_or_else(|| task_id.clone()),
                "worker".to_string(),
                logical_worker_id.clone().unwrap_or_else(|| task_id.clone()),
                DelegationRuntimeStatus::Running,
                Some(format!("Waiting for permission: {}", tool_name)),
                None,
                None,
            )),
            thread_root_id: None,
            root_message_id: None,
        })),
        StreamEvent::PermissionResolved {
            task_id,
            tool_name,
            decision,
            logical_worker_id,
            ..
        } => Some(with_context(DelegationRuntimeEventPayload {
            event: "permission_resolved".to_string(),
            mode: None,
            status: Some(DelegationRuntimeStatus::Running),
            summary: None,
            worker: Some(make_worker(
                logical_worker_id.clone().unwrap_or_else(|| task_id.clone()),
                "worker".to_string(),
                logical_worker_id.clone().unwrap_or_else(|| task_id.clone()),
                DelegationRuntimeStatus::Running,
                Some(format!("Permission {} for {}", decision, tool_name)),
                None,
                None,
            )),
            thread_root_id: None,
            root_message_id: None,
        })),
        StreamEvent::PermissionTimedOut {
            task_id,
            tool_name,
            timeout_ms,
            logical_worker_id,
            ..
        } => Some(with_context(DelegationRuntimeEventPayload {
            event: "permission_timed_out".to_string(),
            mode: None,
            status: Some(DelegationRuntimeStatus::Failed),
            summary: None,
            worker: Some(make_worker(
                logical_worker_id.clone().unwrap_or_else(|| task_id.clone()),
                "worker".to_string(),
                logical_worker_id.clone().unwrap_or_else(|| task_id.clone()),
                DelegationRuntimeStatus::Failed,
                None,
                None,
                Some(format!(
                    "Permission timed out for {} ({}ms)",
                    tool_name, timeout_ms
                )),
            )),
            thread_root_id: None,
            root_message_id: None,
        })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_delegation_runtime, delegation_event_from_worker_stream, DelegationRuntimeMode,
        DelegationRuntimeStatus, DelegationRuntimeWorkerResponse,
    };
    use agime::agents::harness::{
        CoordinatorSignalSummary, HarnessTaskLedgerState, PersistedChildEvidenceItem, TaskKind,
        TaskSnapshot, TaskStatus, WorkerAttemptIdentity, WorkerOutcome,
    };
    use std::collections::HashMap;

    fn sample_task(
        task_id: &str,
        kind: TaskKind,
        status: TaskStatus,
        description: &str,
    ) -> TaskSnapshot {
        let mut metadata = HashMap::new();
        WorkerAttemptIdentity::fresh(task_id, format!("attempt-{}", task_id))
            .write_to_metadata(&mut metadata);
        TaskSnapshot {
            task_id: task_id.to_string(),
            parent_session_id: "parent".to_string(),
            depth: 1,
            kind,
            status,
            description: Some(description.to_string()),
            write_scope: Vec::new(),
            target_artifacts: Vec::new(),
            result_contract: Vec::new(),
            summary: Some(format!("summary {}", task_id)),
            produced_delta: true,
            accepted_targets: Vec::new(),
            metadata,
            started_at: 1_700_000_000,
            updated_at: 1_700_000_010,
            finished_at: None,
        }
    }

    #[test]
    fn build_runtime_detects_swarm_and_worker_details() {
        let ledger = HarnessTaskLedgerState {
            tasks: vec![
                sample_task(
                    "worker-a",
                    TaskKind::SwarmWorker,
                    TaskStatus::Running,
                    "worker a",
                ),
                sample_task(
                    "validator",
                    TaskKind::ValidationWorker,
                    TaskStatus::Pending,
                    "validator",
                ),
            ],
        };
        let runtime = build_delegation_runtime(
            Some("Leader"),
            Some("running"),
            Some("leader summary"),
            None,
            Some(&ledger),
            None,
        )
        .expect("runtime");
        assert_eq!(runtime.mode, DelegationRuntimeMode::Swarm);
        assert_eq!(runtime.status, DelegationRuntimeStatus::Running);
        assert_eq!(runtime.workers.len(), 2);
        assert_eq!(runtime.leader.expect("leader").title, "Leader");
    }

    #[test]
    fn build_runtime_enriches_from_persisted_child_evidence() {
        let ledger = HarnessTaskLedgerState {
            tasks: vec![sample_task(
                "worker-a",
                TaskKind::Subagent,
                TaskStatus::Completed,
                "worker a",
            )],
        };
        let evidence = vec![PersistedChildEvidenceItem {
            task_id: "worker-a".to_string(),
            status: "completed".to_string(),
            logical_worker_id: None,
            attempt_id: None,
            recovered_terminal_summary: false,
            child_session_id: Some("child-1".to_string()),
            session_preview: Some("preview text".to_string()),
            transcript_excerpt: Vec::new(),
            runtime_state: None,
            runtime_detail: None,
            summary: Some("done".to_string()),
        }];
        let runtime = build_delegation_runtime(
            Some("Leader"),
            Some("completed"),
            Some("leader summary"),
            None,
            Some(&ledger),
            Some(&evidence),
        )
        .expect("runtime");
        assert_eq!(runtime.mode, DelegationRuntimeMode::Subagent);
        assert_eq!(runtime.status, DelegationRuntimeStatus::Completed);
        assert_eq!(
            runtime.workers[0].result_summary.as_deref(),
            Some("summary worker-a")
        );
    }

    #[test]
    fn build_runtime_falls_back_to_persisted_child_evidence_without_ledger_tasks() {
        let evidence = vec![PersistedChildEvidenceItem {
            task_id: "worker-a".to_string(),
            status: "completed".to_string(),
            logical_worker_id: None,
            attempt_id: None,
            recovered_terminal_summary: false,
            child_session_id: Some("child-1".to_string()),
            session_preview: Some("preview text".to_string()),
            transcript_excerpt: Vec::new(),
            runtime_state: None,
            runtime_detail: None,
            summary: Some("done".to_string()),
        }];
        let runtime = build_delegation_runtime(
            Some("Leader"),
            Some("completed"),
            Some("leader summary"),
            None,
            None,
            Some(&evidence),
        )
        .expect("runtime");
        assert_eq!(runtime.mode, DelegationRuntimeMode::Subagent);
        assert_eq!(runtime.status, DelegationRuntimeStatus::Completed);
        assert_eq!(runtime.workers.len(), 1);
        assert_eq!(runtime.workers[0].worker_id, "worker-a");
        assert_eq!(runtime.workers[0].result_summary.as_deref(), Some("done"));
    }

    #[test]
    fn delegation_event_maps_worker_stream() {
        let event = crate::agent::task_manager::StreamEvent::WorkerStarted {
            task_id: "task-1".to_string(),
            kind: "swarm_worker".to_string(),
            target: Some("src/app.ts".to_string()),
            logical_worker_id: Some("worker-a".to_string()),
            attempt_id: None,
            attempt_index: None,
            previous_task_id: None,
        };
        let payload =
            delegation_event_from_worker_stream(&event, Some("thread-1"), None).expect("payload");
        assert_eq!(payload.event, "worker_started");
        assert_eq!(payload.mode, Some(DelegationRuntimeMode::Swarm));
        assert_eq!(payload.thread_root_id.as_deref(), Some("thread-1"));
    }

    #[test]
    fn build_runtime_from_signal_summary_deduplicates_worker_attempts() {
        let attempt_identity = WorkerAttemptIdentity::fresh("worker-a", "attempt-a");
        let runtime = super::build_delegation_runtime_from_signal_summary(
            Some("Leader"),
            Some("completed"),
            Some("rich leader summary"),
            None,
            Some(&CoordinatorSignalSummary {
                worker_outcomes: vec![
                    WorkerOutcome {
                        task_id: "task-1".to_string(),
                        kind: TaskKind::SwarmWorker,
                        status: "completed".to_string(),
                        summary: "short".to_string(),
                        accepted_targets: vec!["README.md".to_string()],
                        produced_delta: true,
                        recovered_terminal_summary: false,
                        attempt_identity: Some(attempt_identity.clone()),
                    },
                    WorkerOutcome {
                        task_id: "task-1-retry".to_string(),
                        kind: TaskKind::SwarmWorker,
                        status: "completed".to_string(),
                        summary: "much richer worker summary for README.md".to_string(),
                        accepted_targets: vec!["README.md".to_string()],
                        produced_delta: true,
                        recovered_terminal_summary: false,
                        attempt_identity: Some(attempt_identity),
                    },
                ],
                ..CoordinatorSignalSummary::default()
            }),
            None,
        )
        .expect("runtime");
        assert_eq!(runtime.workers.len(), 1);
        assert_eq!(runtime.workers[0].worker_id, "worker-a");
        assert_eq!(
            runtime.workers[0].result_summary.as_deref(),
            Some("much richer worker summary for README.md")
        );
    }

    #[test]
    fn build_runtime_from_signal_summary_filters_synthetic_leader_like_worker() {
        let runtime = super::build_delegation_runtime_from_signal_summary(
            Some("Leader"),
            Some("completed"),
            Some("Swarm run `swarm-1` completed. Mailbox root: /tmp/swarm"),
            None,
            Some(&CoordinatorSignalSummary {
                worker_outcomes: vec![
                    WorkerOutcome {
                        task_id: "call_123".to_string(),
                        kind: TaskKind::SwarmWorker,
                        status: "completed".to_string(),
                        summary: "Swarm run `swarm-1` completed. Mailbox root: /tmp/swarm"
                            .to_string(),
                        accepted_targets: Vec::new(),
                        produced_delta: true,
                        recovered_terminal_summary: false,
                        attempt_identity: None,
                    },
                    WorkerOutcome {
                        task_id: "task-2".to_string(),
                        kind: TaskKind::SwarmWorker,
                        status: "completed".to_string(),
                        summary: "README worker completed".to_string(),
                        accepted_targets: vec!["README.md".to_string()],
                        produced_delta: true,
                        recovered_terminal_summary: false,
                        attempt_identity: Some(WorkerAttemptIdentity::fresh(
                            "worker_readme",
                            "attempt-readme",
                        )),
                    },
                ],
                ..CoordinatorSignalSummary::default()
            }),
            None,
        )
        .expect("runtime");
        assert_eq!(runtime.workers.len(), 1);
        assert_eq!(runtime.workers[0].worker_id, "worker_readme");
    }

    #[test]
    fn build_runtime_from_signal_summary_filters_control_workers() {
        let runtime = super::build_delegation_runtime_from_signal_summary(
            Some("Leader"),
            Some("completed"),
            Some("Swarm completed. Key findings:\n- README missing"),
            None,
            Some(&CoordinatorSignalSummary {
                worker_outcomes: vec![
                    WorkerOutcome {
                        task_id: "task-1".to_string(),
                        kind: TaskKind::SwarmWorker,
                        status: "completed".to_string(),
                        summary: "README.md is missing".to_string(),
                        accepted_targets: vec!["README.md".to_string()],
                        produced_delta: false,
                        recovered_terminal_summary: false,
                        attempt_identity: Some(WorkerAttemptIdentity::fresh(
                            "worker_readme",
                            "attempt-readme",
                        )),
                    },
                    WorkerOutcome {
                        task_id: "auto_swarm_deadbeef".to_string(),
                        kind: TaskKind::SwarmWorker,
                        status: "running".to_string(),
                        summary: "child task for swarm".to_string(),
                        accepted_targets: Vec::new(),
                        produced_delta: false,
                        recovered_terminal_summary: false,
                        attempt_identity: Some(WorkerAttemptIdentity::fresh(
                            "auto_swarm_deadbeef",
                            "attempt-control",
                        )),
                    },
                ],
                ..CoordinatorSignalSummary::default()
            }),
            None,
        )
        .expect("runtime");
        assert_eq!(runtime.status, DelegationRuntimeStatus::Completed);
        assert_eq!(runtime.workers.len(), 1);
        assert_eq!(runtime.workers[0].worker_id, "worker_readme");
    }

    #[test]
    fn concise_worker_detail_softens_recovered_trace_language() {
        let worker = DelegationRuntimeWorkerResponse {
            worker_id: "worker_1".to_string(),
            role: "swarm_worker".to_string(),
            title: "Worker 1".to_string(),
            status: DelegationRuntimeStatus::Completed,
            summary: Some(
                "Scope: recovered bounded worker trace\nResult: Explicit swarm request detected. Injected a bounded swarm bootstrap for this turn.\nBlocker: worker finished without an explicit terminal summary".to_string(),
            ),
            last_update_at: None,
            result_summary: None,
            error: None,
        };
        assert_eq!(
            super::concise_worker_detail(&worker).as_deref(),
            Some("completed via recovered worker trace")
        );
    }

    #[test]
    fn build_runtime_from_signal_summary_enriches_generic_leader_summary_from_workers() {
        let runtime = super::build_delegation_runtime_from_signal_summary(
            Some("Leader"),
            Some("completed"),
            Some("swarm completed with 1 accepted target(s)"),
            None,
            Some(&CoordinatorSignalSummary {
                worker_outcomes: vec![WorkerOutcome {
                    task_id: "task-1".to_string(),
                    kind: TaskKind::SwarmWorker,
                    status: "completed".to_string(),
                    summary: "README updated with build instructions".to_string(),
                    accepted_targets: vec!["README.md".to_string()],
                    produced_delta: true,
                    recovered_terminal_summary: false,
                    attempt_identity: Some(WorkerAttemptIdentity::fresh(
                        "worker_readme",
                        "attempt-readme",
                    )),
                }],
                ..CoordinatorSignalSummary::default()
            }),
            None,
        )
        .expect("runtime");
        let leader = runtime.leader.expect("leader");
        let leader_summary = leader.summary.as_deref().unwrap_or_default();
        assert!(leader_summary.contains("README"));
        assert!(leader_summary.contains("README updated with build instructions"));
    }

    #[test]
    fn build_runtime_from_signal_summary_extracts_structured_worker_result() {
        let runtime = super::build_delegation_runtime_from_signal_summary(
            Some("Leader"),
            Some("completed"),
            Some("Bounded swarm produced no accepted target delta. Fall back to a single-worker path."),
            None,
            Some(&CoordinatorSignalSummary {
                worker_outcomes: vec![
                    WorkerOutcome {
                        task_id: "task-1".to_string(),
                        kind: TaskKind::SwarmWorker,
                        status: "completed".to_string(),
                        summary: "Scope: inspect README\nResult: README.md does not exist бк verified via filesystem check.\nBlocker: leader must decide whether to create it.".to_string(),
                        accepted_targets: Vec::new(),
                        produced_delta: false,
                        recovered_terminal_summary: false,
                        attempt_identity: Some(WorkerAttemptIdentity::fresh(
                            "worker_1_scope",
                            "attempt-1",
                        )),
                    },
                    WorkerOutcome {
                        task_id: "task-2".to_string(),
                        kind: TaskKind::SwarmWorker,
                        status: "completed".to_string(),
                        summary: "Scope: inspect docs\nResult: docs directory does not exist.\nArtifacts changed: artifacts/docs-check.txt".to_string(),
                        accepted_targets: Vec::new(),
                        produced_delta: true,
                        recovered_terminal_summary: false,
                        attempt_identity: Some(WorkerAttemptIdentity::fresh(
                            "worker_2_scope",
                            "attempt-2",
                        )),
                    },
                ],
                ..CoordinatorSignalSummary::default()
            }),
            None,
        )
        .expect("runtime");

        let leader_summary = runtime
            .leader
            .as_ref()
            .and_then(|leader| leader.summary.as_deref())
            .expect("leader summary");
        assert!(leader_summary.starts_with("Swarm completed. Key findings:"));
        assert!(leader_summary.contains("README"));
        assert!(leader_summary.contains("verified via filesystem check"));
        assert!(leader_summary.contains("docs"));
        assert!(leader_summary.contains("Artifacts changed: artifacts/docs-check.txt"));
        assert!(!leader_summary.contains("Scope:"));
        assert!(!leader_summary.contains("бк"));
    }

    #[test]
    fn build_runtime_from_signal_summary_humanizes_worker_title_and_filters_internal_noise() {
        let runtime = super::build_delegation_runtime_from_signal_summary(
            Some("Leader"),
            Some("completed"),
            Some("swarm completed with 2 accepted target(s)"),
            None,
            Some(&CoordinatorSignalSummary {
                worker_outcomes: vec![
                    WorkerOutcome {
                        task_id: "task-1".to_string(),
                        kind: TaskKind::SwarmWorker,
                        status: "completed".to_string(),
                        summary: "Scope: inspect README\nResult: README.md is missing.\nBlocker: worker is read-only and cannot create file.\nArtifacts changed: mailbox/worker-1.md, artifacts/readme-check.txt".to_string(),
                        accepted_targets: Vec::new(),
                        produced_delta: true,
                        recovered_terminal_summary: false,
                        attempt_identity: Some(WorkerAttemptIdentity::fresh(
                            "worker_1_readme",
                            "attempt-1",
                        )),
                    },
                    WorkerOutcome {
                        task_id: "task-2".to_string(),
                        kind: TaskKind::SwarmWorker,
                        status: "completed".to_string(),
                        summary: "Scope: inspect docs\nResult: docs/ directory is missing.".to_string(),
                        accepted_targets: Vec::new(),
                        produced_delta: false,
                        recovered_terminal_summary: false,
                        attempt_identity: Some(WorkerAttemptIdentity::fresh(
                            "worker_2_docs",
                            "attempt-2",
                        )),
                    },
                ],
                ..CoordinatorSignalSummary::default()
            }),
            None,
        )
        .expect("runtime");

        assert_eq!(runtime.workers[0].title, "Worker 1");
        assert_eq!(runtime.workers[1].title, "Worker 2");

        let leader_summary = runtime
            .leader
            .as_ref()
            .and_then(|leader| leader.summary.as_deref())
            .expect("leader summary");
        assert!(leader_summary.contains("README"));
        assert!(leader_summary.contains("README.md is missing."));
        assert!(!leader_summary.contains("read-only"));
        assert!(!leader_summary.contains("mailbox/worker-1.md"));
        assert!(leader_summary.contains("Artifacts changed: artifacts/readme-check.txt"));
    }

    #[test]
    fn build_runtime_from_signal_summary_prefers_programmatic_swarm_leader_summary() {
        let runtime = super::build_delegation_runtime_from_signal_summary(
            Some("Leader"),
            Some("completed"),
            Some(
                "Two workers completed.\n\n---\n\n## Project overview\n\n| file | role |\n|------|------|\n| README.md | project summary |",
            ),
            None,
            Some(&CoordinatorSignalSummary {
                worker_outcomes: vec![
                    WorkerOutcome {
                        task_id: "task-1".to_string(),
                        kind: TaskKind::SwarmWorker,
                        status: "completed".to_string(),
                        summary: "Scope: inspect README\nResult: README.md is missing.".to_string(),
                        accepted_targets: Vec::new(),
                        produced_delta: false,
                        recovered_terminal_summary: false,
                        attempt_identity: Some(WorkerAttemptIdentity::fresh(
                            "worker_1_readme",
                            "attempt-1",
                        )),
                    },
                    WorkerOutcome {
                        task_id: "task-2".to_string(),
                        kind: TaskKind::SwarmWorker,
                        status: "completed".to_string(),
                        summary: "Scope: inspect docs\nResult: docs/ directory is missing.".to_string(),
                        accepted_targets: Vec::new(),
                        produced_delta: false,
                        recovered_terminal_summary: false,
                        attempt_identity: Some(WorkerAttemptIdentity::fresh(
                            "worker_2_docs",
                            "attempt-2",
                        )),
                    },
                ],
                ..CoordinatorSignalSummary::default()
            }),
            None,
        )
        .expect("runtime");

        let leader_summary = runtime
            .leader
            .as_ref()
            .and_then(|leader| leader.summary.as_deref())
            .expect("leader summary");
        assert!(leader_summary.starts_with("Swarm completed. Key findings:"));
        assert!(leader_summary.contains("README"));
        assert!(leader_summary.contains("README.md is missing."));
        assert!(leader_summary.contains("docs"));
        assert!(leader_summary.contains("docs/ directory is missing."));
        assert!(!leader_summary.contains("## Project overview"));
        assert!(!leader_summary.contains("| file |"));
    }
}
