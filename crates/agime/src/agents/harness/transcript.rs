use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use rmcp::model::Role;
use serde::{Deserialize, Serialize};

use crate::conversation::{message::Message, Conversation};
use crate::session::extension_data::{ExtensionData, ExtensionState};
use crate::session::SessionManager;

use super::checkpoints::HarnessCheckpoint;
use super::state::HarnessMode;
use super::task_runtime::{TaskSnapshot, TaskStatus, WorkerAttemptIdentity};

const TASK_LEDGER_RUNTIME_STATE_KEY: &str = "runtime_state";
const TASK_LEDGER_RUNTIME_DETAIL_KEY: &str = "runtime_detail";
const TASK_LEDGER_PERMISSION_TOOL_KEY: &str = "permission_tool_name";
const TASK_LEDGER_PERMISSION_DECISION_KEY: &str = "permission_decision";
const TASK_LEDGER_PERMISSION_TIMEOUT_KEY: &str = "permission_timeout_ms";
const TASK_LEDGER_CHILD_SESSION_ID_KEY: &str = "child_session_id";
const TASK_LEDGER_CHILD_SESSION_TYPE_KEY: &str = "child_session_type";
const TASK_LEDGER_CHILD_SESSION_PREVIEW_KEY: &str = "child_session_preview";
const TASK_LEDGER_CHILD_SESSION_EXCERPT_KEY: &str = "child_session_excerpt";
const TASK_LEDGER_CHILD_EVIDENCE_SIGNATURE_KEY: &str = "child_evidence_signature";
const TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_KEY: &str = "child_transcript_resume";
const TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SIGNATURE_KEY: &str = "child_transcript_resume_signature";
const TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_MESSAGE_COUNT_KEY: &str =
    "child_transcript_resume_message_count";
const TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_UPDATED_AT_KEY: &str =
    "child_transcript_resume_updated_at";
const TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SESSION_ID_KEY: &str =
    "child_transcript_resume_session_id";
const TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_STATUS_KEY: &str = "child_transcript_resume_status";
const TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SUMMARY_KEY: &str = "child_transcript_resume_summary";
const TASK_LEDGER_CHILD_SESSION_PREVIEW_MAX_CHARS: usize = 200;
const TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_MAX_LINES: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HarnessSessionState {
    pub mode: HarnessMode,
    pub updated_at: i64,
    pub turns_taken: u32,
    pub runtime_compaction_count: u32,
    pub delegation_depth: u32,
}

impl HarnessSessionState {
    pub fn new(mode: HarnessMode) -> Self {
        Self {
            mode,
            updated_at: Utc::now().timestamp(),
            turns_taken: 0,
            runtime_compaction_count: 0,
            delegation_depth: 0,
        }
    }

    pub fn snapshot(
        mode: HarnessMode,
        turns_taken: u32,
        runtime_compaction_count: u32,
        delegation_depth: u32,
    ) -> Self {
        Self::new(mode).with_snapshot(turns_taken, runtime_compaction_count, delegation_depth)
    }

    pub fn with_snapshot(
        mut self,
        turns_taken: u32,
        runtime_compaction_count: u32,
        delegation_depth: u32,
    ) -> Self {
        self.turns_taken = turns_taken;
        self.runtime_compaction_count = runtime_compaction_count;
        self.delegation_depth = delegation_depth;
        self.updated_at = Utc::now().timestamp();
        self
    }
}

impl ExtensionState for HarnessSessionState {
    const EXTENSION_NAME: &'static str = "harness";
    const VERSION: &'static str = "v0";
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HarnessCheckpointState {
    pub checkpoints: Vec<HarnessCheckpoint>,
}

impl ExtensionState for HarnessCheckpointState {
    const EXTENSION_NAME: &'static str = "harness_checkpoints";
    const VERSION: &'static str = "v0";
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HarnessTaskLedgerState {
    pub tasks: Vec<TaskSnapshot>,
}

impl ExtensionState for HarnessTaskLedgerState {
    const EXTENSION_NAME: &'static str = "harness_tasks";
    const VERSION: &'static str = "v0";
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PersistedTaskLedgerSummary {
    pub total_tasks: usize,
    pub active_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub latest_summary: Option<String>,
    pub latest_runtime_detail: Option<String>,
    pub active_runtime_states: Vec<String>,
    pub produced_artifacts: Vec<String>,
    pub accepted_artifacts: Vec<String>,
    pub child_session_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PersistedChildEvidenceItem {
    pub task_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_worker_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default)]
    pub recovered_terminal_summary: bool,
    pub child_session_id: Option<String>,
    pub session_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transcript_excerpt: Vec<String>,
    pub runtime_state: Option<String>,
    pub runtime_detail: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PersistedChildTranscriptView {
    pub task_id: String,
    pub status: String,
    pub child_session_id: Option<String>,
    pub session_preview: Option<String>,
    #[serde(default)]
    pub transcript_lines: Vec<String>,
    pub transcript_source: String,
    pub message_count: usize,
    pub runtime_state: Option<String>,
    pub runtime_detail: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildTranscriptResumeSelection {
    Active,
    RecentTerminal,
}

#[async_trait]
pub trait HarnessTranscriptStore: Send + Sync + Clone + 'static {
    async fn append_message(&self, session_id: &str, message: &Message) -> Result<()>;
    async fn append_messages(&self, session_id: &str, messages: &[Message]) -> Result<()>;
    async fn replace_conversation(
        &self,
        session_id: &str,
        conversation: &Conversation,
    ) -> Result<()>;
    async fn load_state(&self, session_id: &str) -> Result<HarnessSessionState>;
    async fn save_state(&self, session_id: &str, state: HarnessSessionState) -> Result<()>;
    async fn load_mode(&self, session_id: &str) -> Result<HarnessMode>;
    async fn save_mode(&self, session_id: &str, mode: HarnessMode) -> Result<()>;
}

#[async_trait]
pub trait HarnessCheckpointStore: Send + Sync + Clone + 'static {
    async fn record_checkpoint(
        &self,
        session_id: &str,
        checkpoint: HarnessCheckpoint,
    ) -> Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct SessionHarnessStore;

impl SessionHarnessStore {
    async fn load_extension_data(&self, session_id: &str) -> Result<ExtensionData> {
        let session = SessionManager::get_session(session_id, false).await?;
        Ok(session.extension_data)
    }

    pub async fn persist_runtime_snapshot(
        &self,
        session_id: &str,
        mode: HarnessMode,
        turns_taken: u32,
        runtime_compaction_count: u32,
        delegation_depth: u32,
    ) -> Result<()> {
        self.save_state(
            session_id,
            HarnessSessionState::snapshot(
                mode,
                turns_taken,
                runtime_compaction_count,
                delegation_depth,
            ),
        )
        .await
    }
}

pub async fn load_task_ledger_state(session_id: &str) -> Result<HarnessTaskLedgerState> {
    let session = SessionManager::get_session(session_id, false).await?;
    Ok(HarnessTaskLedgerState::from_extension_data(&session.extension_data).unwrap_or_default())
}

pub async fn upsert_task_ledger_snapshot(session_id: &str, snapshot: &TaskSnapshot) -> Result<()> {
    let mut state = load_task_ledger_state(session_id).await?;
    if let Some(existing) = state
        .tasks
        .iter_mut()
        .find(|task| task.task_id == snapshot.task_id)
    {
        *existing = snapshot.clone();
    } else {
        state.tasks.push(snapshot.clone());
    }
    SessionManager::set_extension_state(session_id, &state).await
}

pub async fn annotate_task_ledger_runtime_state(
    session_id: &str,
    task_id: &str,
    runtime_state: &str,
    runtime_detail: Option<String>,
    permission_tool_name: Option<String>,
    permission_decision: Option<String>,
    permission_timeout_ms: Option<u64>,
) -> Result<()> {
    let mut state = load_task_ledger_state(session_id).await?;
    let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
        return Ok(());
    };

    task.metadata.insert(
        TASK_LEDGER_RUNTIME_STATE_KEY.to_string(),
        runtime_state.to_string(),
    );
    match runtime_detail {
        Some(detail) => {
            task.metadata
                .insert(TASK_LEDGER_RUNTIME_DETAIL_KEY.to_string(), detail);
        }
        None => {
            task.metadata.remove(TASK_LEDGER_RUNTIME_DETAIL_KEY);
        }
    }
    match permission_tool_name {
        Some(tool) => {
            task.metadata
                .insert(TASK_LEDGER_PERMISSION_TOOL_KEY.to_string(), tool);
        }
        None => {
            task.metadata.remove(TASK_LEDGER_PERMISSION_TOOL_KEY);
        }
    }
    match permission_decision {
        Some(decision) => {
            task.metadata
                .insert(TASK_LEDGER_PERMISSION_DECISION_KEY.to_string(), decision);
        }
        None => {
            task.metadata.remove(TASK_LEDGER_PERMISSION_DECISION_KEY);
        }
    }
    match permission_timeout_ms {
        Some(timeout_ms) => {
            task.metadata.insert(
                TASK_LEDGER_PERMISSION_TIMEOUT_KEY.to_string(),
                timeout_ms.to_string(),
            );
        }
        None => {
            task.metadata.remove(TASK_LEDGER_PERMISSION_TIMEOUT_KEY);
        }
    }

    SessionManager::set_extension_state(session_id, &state).await
}

pub async fn annotate_task_ledger_child_session(
    session_id: &str,
    task_id: &str,
    child_session_id: &str,
    child_session_type: &str,
) -> Result<()> {
    let mut state = load_task_ledger_state(session_id).await?;
    let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
        return Ok(());
    };
    task.metadata.insert(
        TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
        child_session_id.to_string(),
    );
    task.metadata.insert(
        TASK_LEDGER_CHILD_SESSION_TYPE_KEY.to_string(),
        child_session_type.to_string(),
    );
    SessionManager::set_extension_state(session_id, &state).await
}

pub async fn annotate_task_ledger_child_session_preview(
    session_id: &str,
    task_id: &str,
    child_session_preview: &str,
) -> Result<()> {
    let mut state = load_task_ledger_state(session_id).await?;
    let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
        return Ok(());
    };
    let existing_excerpt = task
        .metadata
        .get(TASK_LEDGER_CHILD_SESSION_EXCERPT_KEY)
        .and_then(|value| parse_persisted_transcript_excerpt(value))
        .unwrap_or_default();
    let normalized_preview = normalize_child_session_preview(child_session_preview);
    match normalized_preview.as_ref() {
        Some(preview) => {
            task.metadata.insert(
                TASK_LEDGER_CHILD_SESSION_PREVIEW_KEY.to_string(),
                preview.clone(),
            );
        }
        None => {
            task.metadata.remove(TASK_LEDGER_CHILD_SESSION_PREVIEW_KEY);
        }
    }
    update_child_evidence_signature(task, normalized_preview.as_deref(), &existing_excerpt);
    SessionManager::set_extension_state(session_id, &state).await
}

pub async fn annotate_task_ledger_child_session_excerpt(
    session_id: &str,
    task_id: &str,
    transcript_excerpt: &[String],
) -> Result<()> {
    let mut state = load_task_ledger_state(session_id).await?;
    let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
        return Ok(());
    };
    let existing_preview = task
        .metadata
        .get(TASK_LEDGER_CHILD_SESSION_PREVIEW_KEY)
        .and_then(|value| normalize_child_session_preview(value));
    let normalized = normalize_transcript_excerpt_lines(transcript_excerpt);
    if normalized.is_empty() {
        task.metadata.remove(TASK_LEDGER_CHILD_SESSION_EXCERPT_KEY);
    } else if let Ok(serialized) = serde_json::to_string(&normalized) {
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_EXCERPT_KEY.to_string(),
            serialized,
        );
    }
    update_child_evidence_signature(task, existing_preview.as_deref(), &normalized);
    SessionManager::set_extension_state(session_id, &state).await
}

pub async fn annotate_task_ledger_child_evidence_view(
    session_id: &str,
    task_id: &str,
    session_preview: Option<&str>,
    transcript_excerpt: &[String],
) -> Result<()> {
    let mut state = load_task_ledger_state(session_id).await?;
    let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
        return Ok(());
    };
    apply_child_evidence_view_metadata(task, session_preview, transcript_excerpt);
    SessionManager::set_extension_state(session_id, &state).await
}

pub async fn has_active_persisted_tasks(session_id: &str) -> Result<bool> {
    let state = load_task_ledger_state(session_id).await?;
    Ok(replayable_task_snapshots(&state.tasks)
        .iter()
        .any(|task| matches!(task.status, TaskStatus::Running | TaskStatus::Pending)))
}

fn replayable_task_snapshots<'a>(tasks: &'a [TaskSnapshot]) -> Vec<&'a TaskSnapshot> {
    let mut items_by_replay_key: std::collections::HashMap<
        String,
        (usize, usize, u32, i64, &TaskSnapshot),
    > = std::collections::HashMap::new();
    for task in tasks {
        let attempt_identity = WorkerAttemptIdentity::from_metadata(&task.metadata);
        let replay_key = attempt_identity
            .as_ref()
            .map(|identity| format!("logical_worker:{}", identity.logical_worker_id))
            .unwrap_or_else(|| format!("task:{}", task.task_id));
        let attempt_index = attempt_identity
            .as_ref()
            .map(|identity| identity.attempt_index)
            .unwrap_or(0);
        let active_rank = usize::from(matches!(
            task.status,
            TaskStatus::Running | TaskStatus::Pending
        ));
        let terminal_rank = match task.status {
            TaskStatus::Failed => 2,
            TaskStatus::Cancelled => 1,
            TaskStatus::Completed => 0,
            TaskStatus::Running | TaskStatus::Pending => 0,
        };
        let should_replace = items_by_replay_key
            .get(&replay_key)
            .map(
                |(current_active, current_terminal, current_attempt_index, current_updated, _)| {
                    attempt_index > *current_attempt_index
                        || (attempt_index == *current_attempt_index
                            && task.updated_at > *current_updated)
                        || (attempt_index == *current_attempt_index
                            && task.updated_at == *current_updated
                            && active_rank > *current_active)
                        || (attempt_index == *current_attempt_index
                            && task.updated_at == *current_updated
                            && active_rank == *current_active
                            && terminal_rank > *current_terminal)
                },
            )
            .unwrap_or(true);
        if should_replace {
            items_by_replay_key.insert(
                replay_key,
                (
                    active_rank,
                    terminal_rank,
                    attempt_index,
                    task.updated_at,
                    task,
                ),
            );
        }
    }
    let mut items = items_by_replay_key.into_values().collect::<Vec<_>>();
    items.sort_by(
        |(left_active, left_terminal, left_attempt_index, left_updated, left),
         (right_active, right_terminal, right_attempt_index, right_updated, right)| {
            right_active
                .cmp(left_active)
                .then_with(|| right_terminal.cmp(left_terminal))
                .then_with(|| right_attempt_index.cmp(left_attempt_index))
                .then_with(|| right_updated.cmp(left_updated))
                .then_with(|| left.task_id.cmp(&right.task_id))
        },
    );
    items.into_iter().map(|(_, _, _, _, task)| task).collect()
}

pub fn build_persisted_child_evidence_view(
    state: &HarnessTaskLedgerState,
) -> Vec<PersistedChildEvidenceItem> {
    replayable_task_snapshots(&state.tasks)
        .into_iter()
        .map(|task| {
            let attempt_identity = WorkerAttemptIdentity::from_metadata(&task.metadata);
            PersistedChildEvidenceItem {
                task_id: task.task_id.clone(),
                status: format!("{:?}", task.status).to_ascii_lowercase(),
                logical_worker_id: attempt_identity
                    .as_ref()
                    .map(|identity| identity.logical_worker_id.clone()),
                attempt_id: attempt_identity
                    .as_ref()
                    .map(|identity| identity.attempt_id.clone()),
                recovered_terminal_summary: task
                    .metadata
                    .get("recovered_terminal_summary")
                    .map(|value| value == "true")
                    .unwrap_or(false),
                child_session_id: task.metadata.get(TASK_LEDGER_CHILD_SESSION_ID_KEY).cloned(),
                session_preview: task
                    .metadata
                    .get(TASK_LEDGER_CHILD_SESSION_PREVIEW_KEY)
                    .and_then(|value| normalize_child_session_preview(value)),
                transcript_excerpt: task
                    .metadata
                    .get(TASK_LEDGER_CHILD_SESSION_EXCERPT_KEY)
                    .and_then(|value| parse_persisted_transcript_excerpt(value))
                    .unwrap_or_default(),
                runtime_state: task.metadata.get(TASK_LEDGER_RUNTIME_STATE_KEY).cloned(),
                runtime_detail: task.metadata.get(TASK_LEDGER_RUNTIME_DETAIL_KEY).cloned(),
                summary: task.summary.clone(),
            }
        })
        .collect()
}

pub fn preferred_child_evidence_detail(item: &PersistedChildEvidenceItem) -> Option<String> {
    item.runtime_detail
        .clone()
        .or_else(|| item.session_preview.clone())
        .or_else(|| item.transcript_excerpt.last().cloned())
        .or_else(|| item.summary.clone())
}

pub fn render_child_evidence_line(
    item: &PersistedChildEvidenceItem,
    recent_terminal: bool,
) -> String {
    let mut line = if recent_terminal {
        String::from("Recent child evidence")
    } else {
        String::from("Child evidence")
    };
    if let Some(child_session_id) = item.child_session_id.as_deref() {
        line.push_str(&format!(" [{}]", child_session_id));
    }
    line.push_str(&format!(" task {}", item.task_id));
    if recent_terminal {
        line.push_str(&format!(" ({})", item.status));
    }
    if let Some(runtime_state) = item.runtime_state.as_deref() {
        line.push_str(&format!(": {}", runtime_state));
    }
    if let Some(detail) = preferred_child_evidence_detail(item) {
        line.push_str(&format!(" - {}", detail));
    }
    line
}

pub fn select_replayable_child_evidence<'a>(
    child_evidence: &'a [PersistedChildEvidenceItem],
) -> Vec<(&'a PersistedChildEvidenceItem, bool)> {
    let mut selected = child_evidence
        .iter()
        .filter(|item| item.status == "running" || item.status == "pending")
        .take(2)
        .map(|item| (item, false))
        .collect::<Vec<_>>();
    if selected.len() < 2 {
        let remaining = 2 - selected.len();
        selected.extend(
            child_evidence
                .iter()
                .filter(|item| item.status != "running" && item.status != "pending")
                .take(remaining)
                .map(|item| (item, true)),
        );
    }
    selected
}

pub fn select_replayable_child_transcript_resume<'a>(
    transcript_views: &'a [PersistedChildTranscriptView],
) -> Vec<(
    &'a PersistedChildTranscriptView,
    ChildTranscriptResumeSelection,
)> {
    let mut selected = transcript_views
        .iter()
        .filter(|item| item.status == "running" || item.status == "pending")
        .take(2)
        .map(|item| (item, ChildTranscriptResumeSelection::Active))
        .collect::<Vec<_>>();
    if selected.len() < 2 {
        let remaining = 2 - selected.len();
        selected.extend(
            transcript_views
                .iter()
                .filter(|item| item.status != "running" && item.status != "pending")
                .take(remaining)
                .map(|item| (item, ChildTranscriptResumeSelection::RecentTerminal)),
        );
    }
    selected
}

fn normalize_child_session_preview(input: &str) -> Option<String> {
    let compact = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return None;
    }
    if compact.chars().count() <= TASK_LEDGER_CHILD_SESSION_PREVIEW_MAX_CHARS {
        return Some(compact);
    }
    let truncated: String = compact
        .chars()
        .take(TASK_LEDGER_CHILD_SESSION_PREVIEW_MAX_CHARS.saturating_sub(3))
        .collect();
    Some(format!("{}...", truncated))
}

fn normalize_transcript_excerpt_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| normalize_child_session_preview(line))
        .take(3)
        .collect()
}

fn normalize_transcript_resume_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| normalize_child_session_preview(line))
        .take(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_MAX_LINES)
        .collect()
}

fn parse_persisted_transcript_excerpt(raw: &str) -> Option<Vec<String>> {
    let parsed = serde_json::from_str::<Vec<String>>(raw).ok()?;
    let normalized = normalize_transcript_excerpt_lines(&parsed);
    (!normalized.is_empty()).then_some(normalized)
}

fn parse_persisted_transcript_resume(raw: &str) -> Option<Vec<String>> {
    let parsed = serde_json::from_str::<Vec<String>>(raw).ok()?;
    let normalized = normalize_transcript_resume_lines(&parsed);
    (!normalized.is_empty()).then_some(normalized)
}

fn child_evidence_signature(preview: Option<&str>, excerpt: &[String]) -> Option<String> {
    let normalized_preview = preview.and_then(normalize_child_session_preview);
    let normalized_excerpt = normalize_transcript_excerpt_lines(
        &excerpt
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>(),
    );
    if normalized_preview.is_none() && normalized_excerpt.is_empty() {
        return None;
    }
    serde_json::to_string(&(normalized_preview, normalized_excerpt)).ok()
}

fn child_transcript_resume_signature(lines: &[String]) -> Option<String> {
    let normalized = normalize_transcript_resume_lines(lines);
    (!normalized.is_empty()).then(|| normalized.join("\n"))
}

fn persisted_resume_is_self_consistent(
    task: &TaskSnapshot,
    item: &PersistedChildEvidenceItem,
) -> bool {
    let metadata = &task.metadata;
    let persisted_resume = metadata
        .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_KEY)
        .and_then(|value| parse_persisted_transcript_resume(value));
    let persisted_signature = metadata
        .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SIGNATURE_KEY)
        .cloned();
    let persisted_message_count = metadata
        .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_MESSAGE_COUNT_KEY)
        .and_then(|value| value.parse::<usize>().ok());
    let persisted_updated_at_is_valid = metadata
        .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_UPDATED_AT_KEY)
        .is_none_or(|value| value.parse::<i64>().is_ok());
    let persisted_session_id = metadata
        .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SESSION_ID_KEY)
        .cloned();
    let persisted_status = metadata
        .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_STATUS_KEY)
        .cloned();
    let persisted_summary = metadata
        .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SUMMARY_KEY)
        .cloned()
        .filter(|value| !value.trim().is_empty());

    let resume_lines = persisted_resume.unwrap_or_default();
    let signature_is_consistent = match (
        persisted_signature.as_deref(),
        child_transcript_resume_signature(&resume_lines).as_deref(),
    ) {
        (Some(persisted), Some(expected)) => persisted == expected,
        (None, _) => true,
        _ => false,
    };
    let message_count_is_consistent = match persisted_message_count {
        Some(count) => count >= resume_lines.len() && count > 0,
        None => true,
    };

    !resume_lines.is_empty()
        && signature_is_consistent
        && message_count_is_consistent
        && persisted_updated_at_is_valid
        && persisted_session_id == item.child_session_id
        && persisted_status.as_deref() == Some(item.status.as_str())
        && persisted_summary == item.summary
}

fn extract_transcript_preview(conversation: &Conversation) -> Option<String> {
    for message in conversation.messages().iter().rev() {
        if message.role != Role::Assistant {
            continue;
        }
        for content in &message.content {
            if let Some(text) = content.as_text() {
                if let Some(preview) = normalize_child_session_preview(text) {
                    return Some(preview);
                }
            }
            if let Some(text) = content.as_tool_response_text() {
                if let Some(preview) = normalize_child_session_preview(&text) {
                    return Some(preview);
                }
            }
        }
    }
    None
}

fn extract_message_excerpt_line(message: &Message) -> Option<String> {
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    for content in &message.content {
        if let Some(text) = content.as_text() {
            if let Some(preview) = normalize_child_session_preview(text) {
                return Some(format!("{}: {}", role, preview));
            }
        }
        if let Some(text) = content.as_tool_response_text() {
            if let Some(preview) = normalize_child_session_preview(&text) {
                return Some(format!("{}: {}", role, preview));
            }
        }
    }
    None
}

pub fn build_child_transcript_excerpt(conversation: &Conversation) -> Vec<String> {
    let mut lines = conversation
        .messages()
        .iter()
        .rev()
        .filter_map(extract_message_excerpt_line)
        .take(3)
        .collect::<Vec<_>>();
    lines.reverse();
    normalize_transcript_excerpt_lines(&lines)
}

pub fn build_child_transcript_resume_lines(conversation: &Conversation) -> Vec<String> {
    let mut lines = conversation
        .messages()
        .iter()
        .rev()
        .filter_map(extract_message_excerpt_line)
        .take(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_MAX_LINES)
        .collect::<Vec<_>>();
    lines.reverse();
    normalize_transcript_resume_lines(&lines)
}

fn derive_child_evidence_view(
    conversation: Option<&Conversation>,
    preview_fallback: Option<&str>,
) -> (Option<String>, Vec<String>, Option<String>) {
    let preview = conversation
        .and_then(extract_transcript_preview)
        .or_else(|| preview_fallback.and_then(normalize_child_session_preview));
    let excerpt = conversation
        .map(build_child_transcript_excerpt)
        .unwrap_or_default();
    let signature = child_evidence_signature(preview.as_deref(), &excerpt);
    (preview, excerpt, signature)
}

fn update_child_evidence_signature(
    task: &mut TaskSnapshot,
    preview: Option<&str>,
    excerpt: &[String],
) {
    match child_evidence_signature(preview, excerpt) {
        Some(signature) => {
            task.metadata.insert(
                TASK_LEDGER_CHILD_EVIDENCE_SIGNATURE_KEY.to_string(),
                signature,
            );
        }
        None => {
            task.metadata
                .remove(TASK_LEDGER_CHILD_EVIDENCE_SIGNATURE_KEY);
        }
    }
}

fn apply_child_evidence_view_metadata(
    task: &mut TaskSnapshot,
    session_preview: Option<&str>,
    transcript_excerpt: &[String],
) {
    let normalized_preview = session_preview.and_then(normalize_child_session_preview);
    let normalized_excerpt = normalize_transcript_excerpt_lines(transcript_excerpt);

    match normalized_preview.as_ref() {
        Some(preview) => {
            task.metadata.insert(
                TASK_LEDGER_CHILD_SESSION_PREVIEW_KEY.to_string(),
                preview.clone(),
            );
        }
        None => {
            task.metadata.remove(TASK_LEDGER_CHILD_SESSION_PREVIEW_KEY);
        }
    }

    if normalized_excerpt.is_empty() {
        task.metadata.remove(TASK_LEDGER_CHILD_SESSION_EXCERPT_KEY);
    } else if let Ok(serialized) = serde_json::to_string(&normalized_excerpt) {
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_EXCERPT_KEY.to_string(),
            serialized,
        );
    }

    update_child_evidence_signature(task, normalized_preview.as_deref(), &normalized_excerpt);
}

fn clear_child_transcript_resume_metadata(task: &mut TaskSnapshot) {
    task.metadata
        .remove(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_KEY);
    task.metadata
        .remove(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SIGNATURE_KEY);
    task.metadata
        .remove(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_MESSAGE_COUNT_KEY);
    task.metadata
        .remove(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_UPDATED_AT_KEY);
    task.metadata
        .remove(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SESSION_ID_KEY);
    task.metadata
        .remove(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_STATUS_KEY);
    task.metadata
        .remove(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SUMMARY_KEY);
}

fn apply_child_transcript_resume_metadata(
    task: &mut TaskSnapshot,
    transcript_lines: &[String],
    message_count: usize,
    child_updated_at: Option<i64>,
) {
    let normalized_lines = normalize_transcript_resume_lines(transcript_lines);
    if normalized_lines.is_empty() {
        clear_child_transcript_resume_metadata(task);
        return;
    }
    if let Ok(serialized) = serde_json::to_string(&normalized_lines) {
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_KEY.to_string(),
            serialized,
        );
    }
    if let Some(signature) = child_transcript_resume_signature(&normalized_lines) {
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SIGNATURE_KEY.to_string(),
            signature,
        );
    } else {
        task.metadata
            .remove(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SIGNATURE_KEY);
    }
    task.metadata.insert(
        TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_MESSAGE_COUNT_KEY.to_string(),
        message_count.to_string(),
    );
    match child_updated_at {
        Some(value) => {
            task.metadata.insert(
                TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_UPDATED_AT_KEY.to_string(),
                value.to_string(),
            );
        }
        None => {
            task.metadata
                .remove(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_UPDATED_AT_KEY);
        }
    }
    match task.metadata.get(TASK_LEDGER_CHILD_SESSION_ID_KEY).cloned() {
        Some(session_id) => {
            task.metadata.insert(
                TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SESSION_ID_KEY.to_string(),
                session_id,
            );
        }
        None => {
            task.metadata
                .remove(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SESSION_ID_KEY);
        }
    }
    task.metadata.insert(
        TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_STATUS_KEY.to_string(),
        format!("{:?}", task.status).to_ascii_lowercase(),
    );
    match task
        .summary
        .clone()
        .filter(|value| !value.trim().is_empty())
    {
        Some(summary) => {
            task.metadata.insert(
                TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SUMMARY_KEY.to_string(),
                summary,
            );
        }
        None => {
            task.metadata
                .remove(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SUMMARY_KEY);
        }
    }
}

pub async fn annotate_task_ledger_child_transcript_resume(
    session_id: &str,
    task_id: &str,
    transcript_lines: &[String],
    message_count: usize,
    child_updated_at: Option<i64>,
) -> Result<()> {
    let mut state = load_task_ledger_state(session_id).await?;
    let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
        return Ok(());
    };
    apply_child_transcript_resume_metadata(task, transcript_lines, message_count, child_updated_at);
    SessionManager::set_extension_state(session_id, &state).await
}

pub async fn load_task_ledger_evidence_view(
    session_id: &str,
) -> Result<Vec<PersistedChildEvidenceItem>> {
    let mut state = load_task_ledger_state(session_id).await?;
    let mut evidence = build_persisted_child_evidence_view(&state);
    let mut state_changed = false;
    for item in &mut evidence {
        let Some(child_session_id) = item.child_session_id.as_deref() else {
            continue;
        };
        let Ok(child_session) = SessionManager::get_session(child_session_id, true).await else {
            continue;
        };
        let Some(conversation) = child_session.conversation.as_ref() else {
            continue;
        };
        let (preview, excerpt, signature) =
            derive_child_evidence_view(Some(conversation), item.session_preview.as_deref());
        let persisted_signature = state
            .tasks
            .iter()
            .find(|task| task.task_id == item.task_id)
            .and_then(|task| task.metadata.get(TASK_LEDGER_CHILD_EVIDENCE_SIGNATURE_KEY))
            .cloned();
        let needs_refresh = signature != persisted_signature
            || item.session_preview != preview
            || item.transcript_excerpt != excerpt;
        if needs_refresh {
            item.session_preview = preview.clone();
            item.transcript_excerpt = excerpt.clone();
            if let Some(task) = state
                .tasks
                .iter_mut()
                .find(|task| task.task_id == item.task_id)
            {
                apply_child_evidence_view_metadata(task, preview.as_deref(), &excerpt);
                state_changed = true;
            }
        }
    }
    if state_changed {
        SessionManager::set_extension_state(session_id, &state).await?;
    }
    Ok(evidence)
}

pub async fn load_task_ledger_transcript_resume_view(
    session_id: &str,
) -> Result<Vec<PersistedChildTranscriptView>> {
    let evidence = load_task_ledger_evidence_view(session_id).await?;
    let mut state = load_task_ledger_state(session_id).await?;
    let mut views = Vec::new();
    let mut state_changed = false;

    for item in evidence {
        let mut task = state
            .tasks
            .iter_mut()
            .find(|task| task.task_id == item.task_id);
        let mut message_count = 0usize;
        let mut transcript_lines = Vec::new();
        let mut transcript_source = "none".to_string();
        let mut stale_persisted_resume = false;
        let mut live_child_session_loaded = false;

        let persisted_resume_lines = task
            .as_ref()
            .and_then(|task| task.metadata.get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_KEY))
            .and_then(|value| parse_persisted_transcript_resume(value))
            .unwrap_or_default();
        let persisted_resume_signature = task
            .as_ref()
            .and_then(|task| {
                task.metadata
                    .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SIGNATURE_KEY)
            })
            .cloned();
        let persisted_resume_message_count = task
            .as_ref()
            .and_then(|task| {
                task.metadata
                    .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_MESSAGE_COUNT_KEY)
            })
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default();
        let persisted_resume_updated_at = task
            .as_ref()
            .and_then(|task| {
                task.metadata
                    .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_UPDATED_AT_KEY)
            })
            .and_then(|value| value.parse::<i64>().ok());
        let persisted_resume_session_id = task
            .as_ref()
            .and_then(|task| {
                task.metadata
                    .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SESSION_ID_KEY)
            })
            .cloned();
        let persisted_resume_status = task
            .as_ref()
            .and_then(|task| {
                task.metadata
                    .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_STATUS_KEY)
            })
            .cloned();
        let persisted_resume_summary = task
            .as_ref()
            .and_then(|task| {
                task.metadata
                    .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SUMMARY_KEY)
            })
            .cloned()
            .filter(|value| !value.trim().is_empty());

        if let Some(child_session_id) = item.child_session_id.as_deref() {
            if let Ok(child_session) = SessionManager::get_session(child_session_id, true).await {
                live_child_session_loaded = true;
                if let Some(conversation) = child_session.conversation.as_ref() {
                    message_count = conversation.messages().len();
                    transcript_lines = build_child_transcript_resume_lines(conversation);
                    if !transcript_lines.is_empty() {
                        transcript_source = "live_child_session".to_string();
                        let live_signature = child_transcript_resume_signature(&transcript_lines);
                        let live_updated_at = Some(child_session.updated_at.timestamp());
                        let needs_refresh = live_signature != persisted_resume_signature
                            || message_count != persisted_resume_message_count
                            || live_updated_at != persisted_resume_updated_at
                            || persisted_resume_session_id.as_deref() != Some(child_session_id)
                            || persisted_resume_status.as_deref() != Some(item.status.as_str())
                            || persisted_resume_summary != item.summary;
                        if needs_refresh {
                            if let Some(task) = task.as_deref_mut() {
                                apply_child_transcript_resume_metadata(
                                    task,
                                    &transcript_lines,
                                    message_count,
                                    live_updated_at,
                                );
                                state_changed = true;
                            }
                        }
                    } else if !persisted_resume_lines.is_empty() {
                        stale_persisted_resume = true;
                        if let Some(task) = task.as_deref_mut() {
                            clear_child_transcript_resume_metadata(task);
                            state_changed = true;
                        }
                    }
                } else if !persisted_resume_lines.is_empty() {
                    stale_persisted_resume = true;
                    if let Some(task) = task.as_deref_mut() {
                        clear_child_transcript_resume_metadata(task);
                        state_changed = true;
                    }
                }
            }
        }

        if transcript_lines.is_empty()
            && !live_child_session_loaded
            && !persisted_resume_lines.is_empty()
            && task
                .as_ref()
                .map(|task| persisted_resume_is_self_consistent(task, &item))
                .unwrap_or(false)
        {
            transcript_lines = normalize_transcript_resume_lines(&persisted_resume_lines);
            message_count = message_count.max(persisted_resume_message_count);
            transcript_source = "persisted_resume".to_string();
        } else if transcript_lines.is_empty() && !persisted_resume_lines.is_empty() {
            stale_persisted_resume = true;
            if let Some(task) = task.as_deref_mut() {
                clear_child_transcript_resume_metadata(task);
                state_changed = true;
            }
        }

        if transcript_lines.is_empty() && !item.transcript_excerpt.is_empty() {
            transcript_lines = normalize_transcript_resume_lines(&item.transcript_excerpt);
            message_count = message_count.max(transcript_lines.len());
            transcript_source = if stale_persisted_resume {
                "stale_persisted_resume".to_string()
            } else {
                "persisted_excerpt".to_string()
            };
        }
        if transcript_lines.is_empty() {
            if let Some(preview) = item.session_preview.as_ref() {
                transcript_lines = vec![format!("assistant: {}", preview)];
                message_count = message_count.max(1);
                transcript_source = if stale_persisted_resume {
                    "stale_persisted_resume".to_string()
                } else {
                    "persisted_preview".to_string()
                };
            }
        }
        if transcript_lines.is_empty() {
            if let Some(summary) = item
                .summary
                .as_ref()
                .and_then(|value| normalize_child_session_preview(value))
            {
                transcript_lines = vec![format!("summary: {}", summary)];
                message_count = message_count.max(1);
                transcript_source = if stale_persisted_resume {
                    "stale_persisted_resume".to_string()
                } else {
                    "task_summary".to_string()
                };
            }
        }

        if transcript_lines.is_empty()
            && item.child_session_id.is_none()
            && item.session_preview.is_none()
            && item.summary.is_none()
        {
            continue;
        }

        views.push(PersistedChildTranscriptView {
            task_id: item.task_id,
            status: item.status,
            child_session_id: item.child_session_id,
            session_preview: item.session_preview,
            transcript_lines,
            transcript_source,
            message_count,
            runtime_state: item.runtime_state,
            runtime_detail: item.runtime_detail,
            summary: item.summary,
        });
    }

    if state_changed {
        SessionManager::set_extension_state(session_id, &state).await?;
    }

    Ok(views)
}

pub fn summarize_task_ledger_state(state: &HarnessTaskLedgerState) -> PersistedTaskLedgerSummary {
    let replayable_tasks = replayable_task_snapshots(&state.tasks);
    let mut produced_artifacts = Vec::new();
    let mut accepted_artifacts = Vec::new();
    let mut child_session_ids = Vec::new();
    let mut active_runtime_states = Vec::new();

    for task in &replayable_tasks {
        accepted_artifacts.extend(task.accepted_targets.iter().cloned());
        if task.produced_delta {
            if task.accepted_targets.is_empty() {
                produced_artifacts.extend(task.target_artifacts.iter().cloned());
            } else {
                produced_artifacts.extend(task.accepted_targets.iter().cloned());
            }
        }
        if let Some(child_session_id) = task.metadata.get(TASK_LEDGER_CHILD_SESSION_ID_KEY) {
            child_session_ids.push(child_session_id.clone());
        }
        if matches!(task.status, TaskStatus::Running | TaskStatus::Pending) {
            if let Some(runtime_state) = task.metadata.get(TASK_LEDGER_RUNTIME_STATE_KEY) {
                active_runtime_states.push(runtime_state.clone());
            }
        }
    }

    produced_artifacts.sort();
    produced_artifacts.dedup();
    accepted_artifacts.sort();
    accepted_artifacts.dedup();
    child_session_ids.sort();
    child_session_ids.dedup();
    active_runtime_states.sort();
    active_runtime_states.dedup();

    let latest_summary = replayable_tasks
        .iter()
        .filter_map(|task| {
            task.summary
                .as_ref()
                .map(|summary| (task.updated_at, summary))
        })
        .max_by_key(|(updated_at, _)| *updated_at)
        .map(|(_, summary)| summary.clone());
    let latest_runtime_detail = replayable_tasks
        .iter()
        .filter_map(|task| {
            task.metadata
                .get(TASK_LEDGER_RUNTIME_DETAIL_KEY)
                .map(|detail| (task.updated_at, detail))
        })
        .max_by_key(|(updated_at, _)| *updated_at)
        .map(|(_, detail)| detail.clone());

    PersistedTaskLedgerSummary {
        total_tasks: replayable_tasks.len(),
        active_tasks: replayable_tasks
            .iter()
            .filter(|task| matches!(task.status, TaskStatus::Running | TaskStatus::Pending))
            .count(),
        completed_tasks: replayable_tasks
            .iter()
            .filter(|task| task.status == TaskStatus::Completed)
            .count(),
        failed_tasks: replayable_tasks
            .iter()
            .filter(|task| matches!(task.status, TaskStatus::Failed | TaskStatus::Cancelled))
            .count(),
        latest_summary,
        latest_runtime_detail,
        active_runtime_states,
        produced_artifacts,
        accepted_artifacts,
        child_session_ids,
    }
}

pub async fn load_task_ledger_summary(session_id: &str) -> Result<PersistedTaskLedgerSummary> {
    let state = load_task_ledger_state(session_id).await?;
    Ok(summarize_task_ledger_state(&state))
}

#[async_trait]
impl HarnessTranscriptStore for SessionHarnessStore {
    async fn append_message(&self, session_id: &str, message: &Message) -> Result<()> {
        SessionManager::add_message(session_id, message).await
    }

    async fn append_messages(&self, session_id: &str, messages: &[Message]) -> Result<()> {
        for message in messages {
            SessionManager::add_message(session_id, message).await?;
        }
        Ok(())
    }

    async fn replace_conversation(
        &self,
        session_id: &str,
        conversation: &Conversation,
    ) -> Result<()> {
        SessionManager::replace_conversation(session_id, conversation).await
    }

    async fn load_state(&self, session_id: &str) -> Result<HarnessSessionState> {
        let extension_data = self.load_extension_data(session_id).await?;
        Ok(HarnessSessionState::from_extension_data(&extension_data).unwrap_or_default())
    }

    async fn save_state(&self, session_id: &str, state: HarnessSessionState) -> Result<()> {
        SessionManager::set_extension_state(session_id, &state).await
    }

    async fn load_mode(&self, session_id: &str) -> Result<HarnessMode> {
        Ok(self.load_state(session_id).await?.mode)
    }

    async fn save_mode(&self, session_id: &str, mode: HarnessMode) -> Result<()> {
        let mut state = self.load_state(session_id).await?;
        state.mode = mode;
        state.updated_at = Utc::now().timestamp();
        self.save_state(session_id, state).await
    }
}

#[async_trait]
impl HarnessCheckpointStore for SessionHarnessStore {
    async fn record_checkpoint(
        &self,
        session_id: &str,
        checkpoint: HarnessCheckpoint,
    ) -> Result<()> {
        const MAX_CHECKPOINTS: usize = 100;

        let session = SessionManager::get_session(session_id, false).await?;
        let extension_data = session.extension_data;
        let mut state =
            HarnessCheckpointState::from_extension_data(&extension_data).unwrap_or_default();
        state.checkpoints.push(checkpoint);
        if state.checkpoints.len() > MAX_CHECKPOINTS {
            let trim = state.checkpoints.len().saturating_sub(MAX_CHECKPOINTS);
            state.checkpoints.drain(0..trim);
        }
        SessionManager::set_extension_state(session_id, &state).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{SessionManager, SessionType};
    use uuid::Uuid;

    #[test]
    fn harness_state_round_trips_through_extension_data() {
        let mut extension_data = ExtensionData::default();
        HarnessSessionState::new(HarnessMode::Plan)
            .with_snapshot(3, 1, 1)
            .to_extension_data(&mut extension_data)
            .expect("state should serialize");
        let restored = HarnessSessionState::from_extension_data(&extension_data)
            .expect("state should deserialize");
        assert_eq!(restored.mode, HarnessMode::Plan);
        assert_eq!(restored.turns_taken, 3);
        assert_eq!(restored.runtime_compaction_count, 1);
        assert_eq!(restored.delegation_depth, 1);
    }

    #[test]
    fn harness_checkpoints_round_trip_through_extension_data() {
        let mut extension_data = ExtensionData::default();
        let state = HarnessCheckpointState {
            checkpoints: vec![HarnessCheckpoint {
                kind: crate::agents::harness::HarnessCheckpointKind::TurnStart,
                turn: 1,
                mode: HarnessMode::Conversation,
                detail: "boot".to_string(),
            }],
        };
        state
            .to_extension_data(&mut extension_data)
            .expect("checkpoints should serialize");
        let restored = HarnessCheckpointState::from_extension_data(&extension_data)
            .expect("checkpoints should deserialize");
        assert_eq!(restored.checkpoints.len(), 1);
        assert_eq!(restored.checkpoints[0].detail, "boot");
    }

    #[test]
    fn harness_task_ledger_round_trips_through_extension_data() {
        let mut extension_data = ExtensionData::default();
        let state = HarnessTaskLedgerState {
            tasks: vec![TaskSnapshot {
                task_id: "task-1".to_string(),
                parent_session_id: "parent-1".to_string(),
                depth: 1,
                kind: super::super::task_runtime::TaskKind::SwarmWorker,
                status: TaskStatus::Running,
                description: Some("worker".to_string()),
                write_scope: vec!["docs".to_string()],
                target_artifacts: vec!["docs/a.md".to_string()],
                result_contract: vec!["docs/a.md".to_string()],
                summary: None,
                produced_delta: false,
                accepted_targets: Vec::new(),
                metadata: std::collections::HashMap::new(),
                started_at: 1,
                updated_at: 1,
                finished_at: None,
            }],
        };
        state
            .to_extension_data(&mut extension_data)
            .expect("task ledger should serialize");
        let restored = HarnessTaskLedgerState::from_extension_data(&extension_data)
            .expect("task ledger should deserialize");
        assert_eq!(restored.tasks.len(), 1);
        assert_eq!(restored.tasks[0].task_id, "task-1");
        assert_eq!(restored.tasks[0].status, TaskStatus::Running);
    }

    #[tokio::test]
    async fn active_persisted_tasks_follow_latest_snapshot_status() {
        let session_id = format!("task-ledger-{}", Uuid::new_v4());
        SessionManager::create_session_with_id(
            session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "task-ledger".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create session");

        let mut snapshot = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::SwarmWorker,
            status: TaskStatus::Running,
            description: Some("worker".to_string()),
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 1,
            finished_at: None,
        };

        upsert_task_ledger_snapshot(&session_id, &snapshot)
            .await
            .expect("persist running snapshot");
        assert!(has_active_persisted_tasks(&session_id)
            .await
            .expect("load active state"));

        snapshot.status = TaskStatus::Completed;
        snapshot.summary = Some("done".to_string());
        snapshot.finished_at = Some(2);
        upsert_task_ledger_snapshot(&session_id, &snapshot)
            .await
            .expect("persist completed snapshot");
        assert!(!has_active_persisted_tasks(&session_id)
            .await
            .expect("load completed state"));
    }

    #[tokio::test]
    async fn annotate_task_ledger_runtime_state_updates_snapshot_metadata() {
        let session_id = format!("task-ledger-runtime-{}", Uuid::new_v4());
        SessionManager::create_session_with_id(
            session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "task-ledger-runtime".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create session");

        let snapshot = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::SwarmWorker,
            status: TaskStatus::Running,
            description: Some("worker".to_string()),
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 1,
            finished_at: None,
        };

        upsert_task_ledger_snapshot(&session_id, &snapshot)
            .await
            .expect("persist snapshot");
        annotate_task_ledger_runtime_state(
            &session_id,
            "task-1",
            "permission_requested",
            Some("waiting for leader approval".to_string()),
            Some("developer__shell".to_string()),
            None,
            None,
        )
        .await
        .expect("annotate runtime state");

        let ledger = load_task_ledger_state(&session_id)
            .await
            .expect("load ledger");
        let task = &ledger.tasks[0];
        assert_eq!(
            task.metadata
                .get(TASK_LEDGER_RUNTIME_STATE_KEY)
                .map(String::as_str),
            Some("permission_requested")
        );
        assert_eq!(
            task.metadata
                .get(TASK_LEDGER_RUNTIME_DETAIL_KEY)
                .map(String::as_str),
            Some("waiting for leader approval")
        );
        assert_eq!(
            task.metadata
                .get(TASK_LEDGER_PERMISSION_TOOL_KEY)
                .map(String::as_str),
            Some("developer__shell")
        );
    }

    #[tokio::test]
    async fn annotate_task_ledger_child_session_updates_snapshot_metadata() {
        let session_id = format!("task-ledger-child-{}", Uuid::new_v4());
        SessionManager::create_session_with_id(
            session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "task-ledger-child".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create session");

        let snapshot = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Running,
            description: Some("subagent".to_string()),
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 1,
            finished_at: None,
        };

        upsert_task_ledger_snapshot(&session_id, &snapshot)
            .await
            .expect("persist snapshot");
        annotate_task_ledger_child_session(
            &session_id,
            "task-1",
            "subagent-session-1",
            "sub_agent",
        )
        .await
        .expect("annotate child session");

        let ledger = load_task_ledger_state(&session_id)
            .await
            .expect("load ledger");
        let task = &ledger.tasks[0];
        assert_eq!(
            task.metadata
                .get(TASK_LEDGER_CHILD_SESSION_ID_KEY)
                .map(String::as_str),
            Some("subagent-session-1")
        );
        assert_eq!(
            task.metadata
                .get(TASK_LEDGER_CHILD_SESSION_TYPE_KEY)
                .map(String::as_str),
            Some("sub_agent")
        );
    }

    #[tokio::test]
    async fn annotate_task_ledger_child_session_preview_updates_snapshot_metadata() {
        let session_id = format!("task-ledger-child-preview-{}", Uuid::new_v4());
        SessionManager::create_session_with_id(
            session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "task-ledger-child-preview".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create session");

        let snapshot = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Running,
            description: Some("subagent".to_string()),
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 1,
            finished_at: None,
        };

        upsert_task_ledger_snapshot(&session_id, &snapshot)
            .await
            .expect("persist snapshot");
        annotate_task_ledger_child_session_preview(&session_id, "task-1", "child preview")
            .await
            .expect("annotate child preview");

        let ledger = load_task_ledger_state(&session_id)
            .await
            .expect("load ledger");
        let task = &ledger.tasks[0];
        assert_eq!(
            task.metadata
                .get(TASK_LEDGER_CHILD_SESSION_PREVIEW_KEY)
                .map(String::as_str),
            Some("child preview")
        );
        assert!(task
            .metadata
            .contains_key(TASK_LEDGER_CHILD_EVIDENCE_SIGNATURE_KEY));
    }

    #[tokio::test]
    async fn annotate_task_ledger_child_session_preview_normalizes_and_truncates() {
        let session_id = format!("task-ledger-child-preview-normalize-{}", Uuid::new_v4());
        SessionManager::create_session_with_id(
            session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "task-ledger-child-preview-normalize".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create session");

        let snapshot = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Running,
            description: Some("subagent".to_string()),
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 1,
            finished_at: None,
        };

        upsert_task_ledger_snapshot(&session_id, &snapshot)
            .await
            .expect("persist snapshot");
        let long_preview = format!("  line one\n\n{}\tline three  ", "x".repeat(220));
        annotate_task_ledger_child_session_preview(&session_id, "task-1", &long_preview)
            .await
            .expect("annotate child preview");

        let ledger = load_task_ledger_state(&session_id)
            .await
            .expect("load ledger");
        let preview = ledger.tasks[0]
            .metadata
            .get(TASK_LEDGER_CHILD_SESSION_PREVIEW_KEY)
            .cloned()
            .expect("preview");
        assert!(!preview.contains('\n'));
        assert!(!preview.contains('\t'));
        assert!(ledger.tasks[0]
            .metadata
            .contains_key(TASK_LEDGER_CHILD_EVIDENCE_SIGNATURE_KEY));
        assert!(preview.ends_with("..."));
        assert!(preview.chars().count() <= TASK_LEDGER_CHILD_SESSION_PREVIEW_MAX_CHARS);
    }

    #[tokio::test]
    async fn annotate_task_ledger_child_session_excerpt_updates_snapshot_metadata() {
        let session_id = format!("task-ledger-child-excerpt-{}", Uuid::new_v4());
        SessionManager::create_session_with_id(
            session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "task-ledger-child-excerpt".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create session");

        let snapshot = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Running,
            description: Some("subagent".to_string()),
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 1,
            finished_at: None,
        };

        upsert_task_ledger_snapshot(&session_id, &snapshot)
            .await
            .expect("persist snapshot");
        annotate_task_ledger_child_session_excerpt(
            &session_id,
            "task-1",
            &[
                "user: inspect docs/a.md".to_string(),
                "assistant: checking the file".to_string(),
            ],
        )
        .await
        .expect("annotate child excerpt");

        let ledger = load_task_ledger_state(&session_id)
            .await
            .expect("load ledger");
        let raw = ledger.tasks[0]
            .metadata
            .get(TASK_LEDGER_CHILD_SESSION_EXCERPT_KEY)
            .cloned()
            .expect("excerpt");
        let parsed = parse_persisted_transcript_excerpt(&raw).expect("parsed excerpt");
        assert_eq!(
            parsed,
            vec![
                "user: inspect docs/a.md".to_string(),
                "assistant: checking the file".to_string()
            ]
        );
    }

    #[test]
    fn summarize_task_ledger_collects_artifacts_and_child_sessions() {
        let mut first = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: "parent".to_string(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::SwarmWorker,
            status: TaskStatus::Completed,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("updated docs/a.md".to_string()),
            produced_delta: true,
            accepted_targets: vec!["docs/a.md".to_string()],
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 10,
            finished_at: Some(10),
        };
        first.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "subagent-1".to_string(),
        );

        let second = TaskSnapshot {
            task_id: "task-2".to_string(),
            parent_session_id: "parent".to_string(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Failed,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/b.md".to_string()],
            result_contract: vec!["docs/b.md".to_string()],
            summary: Some("FAIL: could not update docs/b.md".to_string()),
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: {
                let mut metadata = std::collections::HashMap::new();
                metadata.insert(
                    TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
                    "subagent-2".to_string(),
                );
                metadata
            },
            started_at: 2,
            updated_at: 20,
            finished_at: Some(20),
        };

        let summary = summarize_task_ledger_state(&HarnessTaskLedgerState {
            tasks: vec![first, second],
        });

        assert_eq!(summary.total_tasks, 2);
        assert_eq!(summary.completed_tasks, 1);
        assert_eq!(summary.failed_tasks, 1);
        assert_eq!(
            summary.latest_summary.as_deref(),
            Some("FAIL: could not update docs/b.md")
        );
        assert_eq!(summary.latest_runtime_detail, None);
        assert!(summary.active_runtime_states.is_empty());
        assert_eq!(summary.produced_artifacts, vec!["docs/a.md".to_string()]);
        assert_eq!(summary.accepted_artifacts, vec!["docs/a.md".to_string()]);
        assert_eq!(
            summary.child_session_ids,
            vec!["subagent-1".to_string(), "subagent-2".to_string()]
        );
    }

    #[test]
    fn summarize_task_ledger_collects_active_runtime_markers() {
        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: "parent".to_string(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::SwarmWorker,
            status: TaskStatus::Running,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: None,
        };
        task.metadata.insert(
            TASK_LEDGER_RUNTIME_STATE_KEY.to_string(),
            "permission_requested".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_RUNTIME_DETAIL_KEY.to_string(),
            "worker-a waiting on developer__shell".to_string(),
        );

        let summary = summarize_task_ledger_state(&HarnessTaskLedgerState { tasks: vec![task] });
        assert_eq!(summary.active_tasks, 1);
        assert_eq!(
            summary.active_runtime_states,
            vec!["permission_requested".to_string()]
        );
        assert_eq!(
            summary.latest_runtime_detail.as_deref(),
            Some("worker-a waiting on developer__shell")
        );
    }

    #[test]
    fn build_persisted_child_evidence_view_renders_child_runtime_context() {
        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: "parent".to_string(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::SwarmWorker,
            status: TaskStatus::Running,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("drafted docs/a.md".to_string()),
            produced_delta: true,
            accepted_targets: vec!["docs/a.md".to_string()],
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: None,
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "subagent-1".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_RUNTIME_STATE_KEY.to_string(),
            "permission_requested".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_RUNTIME_DETAIL_KEY.to_string(),
            "worker-a waiting on developer__shell".to_string(),
        );

        let evidence =
            build_persisted_child_evidence_view(&HarnessTaskLedgerState { tasks: vec![task] });
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].task_id, "task-1");
        assert_eq!(evidence[0].status, "running");
        assert_eq!(evidence[0].child_session_id.as_deref(), Some("subagent-1"));
        assert_eq!(evidence[0].session_preview, None);
        assert_eq!(
            evidence[0].runtime_state.as_deref(),
            Some("permission_requested")
        );
        assert_eq!(
            evidence[0].runtime_detail.as_deref(),
            Some("worker-a waiting on developer__shell")
        );
    }

    #[test]
    fn build_persisted_child_evidence_view_prioritizes_active_and_recent_tasks() {
        let mut older_active = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: "parent".to_string(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Running,
            description: None,
            write_scope: Vec::new(),
            target_artifacts: Vec::new(),
            result_contract: Vec::new(),
            summary: Some("older active".to_string()),
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 10,
            finished_at: None,
        };
        older_active.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "subagent-older".to_string(),
        );

        let mut newer_active = older_active.clone();
        newer_active.task_id = "task-2".to_string();
        newer_active.updated_at = 20;
        newer_active.summary = Some("newer active".to_string());
        newer_active.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "subagent-newer".to_string(),
        );

        let mut completed = older_active.clone();
        completed.task_id = "task-3".to_string();
        completed.status = TaskStatus::Completed;
        completed.updated_at = 30;
        completed.summary = Some("completed".to_string());
        completed.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "subagent-completed".to_string(),
        );

        let evidence = build_persisted_child_evidence_view(&HarnessTaskLedgerState {
            tasks: vec![completed, older_active, newer_active],
        });

        assert_eq!(evidence.len(), 3);
        assert_eq!(
            evidence[0].child_session_id.as_deref(),
            Some("subagent-newer")
        );
        assert_eq!(
            evidence[1].child_session_id.as_deref(),
            Some("subagent-older")
        );
        assert_eq!(
            evidence[2].child_session_id.as_deref(),
            Some("subagent-completed")
        );
    }

    #[test]
    fn build_persisted_child_evidence_view_prioritizes_failed_terminal_tasks() {
        let mut completed = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: "parent".to_string(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Completed,
            description: None,
            write_scope: Vec::new(),
            target_artifacts: Vec::new(),
            result_contract: Vec::new(),
            summary: Some("completed".to_string()),
            produced_delta: true,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 30,
            finished_at: Some(30),
        };
        completed.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "subagent-completed".to_string(),
        );

        let mut failed = completed.clone();
        failed.task_id = "task-2".to_string();
        failed.status = TaskStatus::Failed;
        failed.updated_at = 20;
        failed.summary = Some("failed".to_string());
        failed.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "subagent-failed".to_string(),
        );

        let mut cancelled = completed.clone();
        cancelled.task_id = "task-3".to_string();
        cancelled.status = TaskStatus::Cancelled;
        cancelled.updated_at = 25;
        cancelled.summary = Some("cancelled".to_string());
        cancelled.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "subagent-cancelled".to_string(),
        );

        let evidence = build_persisted_child_evidence_view(&HarnessTaskLedgerState {
            tasks: vec![completed, cancelled, failed],
        });

        assert_eq!(
            evidence[0].child_session_id.as_deref(),
            Some("subagent-failed")
        );
        assert_eq!(
            evidence[1].child_session_id.as_deref(),
            Some("subagent-cancelled")
        );
        assert_eq!(
            evidence[2].child_session_id.as_deref(),
            Some("subagent-completed")
        );
    }

    #[test]
    fn build_persisted_child_evidence_view_deduplicates_worker_attempts() {
        let mut first_attempt = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: "parent".to_string(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::SwarmWorker,
            status: TaskStatus::Failed,
            description: None,
            write_scope: Vec::new(),
            target_artifacts: Vec::new(),
            result_contract: Vec::new(),
            summary: Some("first attempt failed".to_string()),
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 10,
            finished_at: Some(10),
        };
        WorkerAttemptIdentity::fresh("worker-a", "task-1")
            .write_to_metadata(&mut first_attempt.metadata);
        first_attempt.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "subagent-first".to_string(),
        );

        let mut retry_attempt = first_attempt.clone();
        retry_attempt.task_id = "task-2".to_string();
        retry_attempt.status = TaskStatus::Running;
        retry_attempt.summary = Some("retry running".to_string());
        retry_attempt.updated_at = 20;
        retry_attempt.finished_at = None;
        retry_attempt.metadata.clear();
        WorkerAttemptIdentity::followup("worker-a", "task-2", 1, "correction", "task-1")
            .write_to_metadata(&mut retry_attempt.metadata);
        retry_attempt.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "subagent-retry".to_string(),
        );

        let mut other_worker = first_attempt.clone();
        other_worker.task_id = "task-3".to_string();
        other_worker.updated_at = 30;
        other_worker.metadata.clear();
        WorkerAttemptIdentity::fresh("worker-b", "task-3")
            .write_to_metadata(&mut other_worker.metadata);
        other_worker.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "subagent-other".to_string(),
        );

        let evidence = build_persisted_child_evidence_view(&HarnessTaskLedgerState {
            tasks: vec![first_attempt, retry_attempt, other_worker],
        });

        assert_eq!(evidence.len(), 2);
        assert_eq!(
            evidence[0].child_session_id.as_deref(),
            Some("subagent-retry")
        );
        assert_eq!(
            evidence[1].child_session_id.as_deref(),
            Some("subagent-other")
        );
    }

    #[test]
    fn summarize_task_ledger_state_deduplicates_worker_attempts() {
        let mut first_attempt = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: "parent".to_string(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::SwarmWorker,
            status: TaskStatus::Failed,
            description: None,
            write_scope: Vec::new(),
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("first attempt failed".to_string()),
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 10,
            finished_at: Some(10),
        };
        WorkerAttemptIdentity::fresh("worker-a", "task-1")
            .write_to_metadata(&mut first_attempt.metadata);

        let mut retry_attempt = first_attempt.clone();
        retry_attempt.task_id = "task-2".to_string();
        retry_attempt.status = TaskStatus::Completed;
        retry_attempt.summary = Some("retry completed".to_string());
        retry_attempt.produced_delta = true;
        retry_attempt.accepted_targets = vec!["docs/a.md".to_string()];
        retry_attempt.updated_at = 20;
        retry_attempt.finished_at = Some(20);
        retry_attempt.metadata.clear();
        WorkerAttemptIdentity::followup("worker-a", "task-2", 1, "correction", "task-1")
            .write_to_metadata(&mut retry_attempt.metadata);

        let summary = summarize_task_ledger_state(&HarnessTaskLedgerState {
            tasks: vec![first_attempt, retry_attempt],
        });

        assert_eq!(summary.total_tasks, 1);
        assert_eq!(summary.completed_tasks, 1);
        assert_eq!(summary.failed_tasks, 0);
        assert_eq!(summary.produced_artifacts, vec!["docs/a.md".to_string()]);
        assert_eq!(summary.latest_summary.as_deref(), Some("retry completed"));
    }

    #[test]
    fn select_replayable_child_evidence_includes_recent_terminal_when_active_slots_available() {
        let active = PersistedChildEvidenceItem {
            task_id: "task-active".to_string(),
            status: "running".to_string(),
            logical_worker_id: None,
            attempt_id: None,
            recovered_terminal_summary: false,
            child_session_id: Some("subagent-active".to_string()),
            session_preview: Some("still working".to_string()),
            transcript_excerpt: Vec::new(),
            runtime_state: Some("running".to_string()),
            runtime_detail: Some("checking docs".to_string()),
            summary: None,
        };
        let failed = PersistedChildEvidenceItem {
            task_id: "task-failed".to_string(),
            status: "failed".to_string(),
            logical_worker_id: None,
            attempt_id: None,
            recovered_terminal_summary: false,
            child_session_id: Some("subagent-failed".to_string()),
            session_preview: Some("failed to update docs".to_string()),
            transcript_excerpt: Vec::new(),
            runtime_state: None,
            runtime_detail: None,
            summary: Some("FAIL: failed to update docs".to_string()),
        };

        let evidence = [active, failed];
        let selected = select_replayable_child_evidence(&evidence);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].0.task_id, "task-active");
        assert!(!selected[0].1);
        assert_eq!(selected[1].0.task_id, "task-failed");
        assert!(selected[1].1);
    }

    #[test]
    fn select_replayable_child_transcript_resume_includes_recent_terminal_when_active_slots_available(
    ) {
        let active = PersistedChildTranscriptView {
            task_id: "task-active".to_string(),
            status: "running".to_string(),
            child_session_id: Some("subagent-active".to_string()),
            session_preview: Some("still working".to_string()),
            transcript_lines: vec!["assistant: still working".to_string()],
            transcript_source: "live_child_session".to_string(),
            message_count: 1,
            runtime_state: Some("running".to_string()),
            runtime_detail: Some("checking docs".to_string()),
            summary: None,
        };
        let failed = PersistedChildTranscriptView {
            task_id: "task-failed".to_string(),
            status: "failed".to_string(),
            child_session_id: Some("subagent-failed".to_string()),
            session_preview: Some("failed to update docs".to_string()),
            transcript_lines: vec!["assistant: failed to update docs".to_string()],
            transcript_source: "persisted_excerpt".to_string(),
            message_count: 1,
            runtime_state: None,
            runtime_detail: None,
            summary: Some("FAIL: failed to update docs".to_string()),
        };

        let resume = [active, failed];
        let selected = select_replayable_child_transcript_resume(&resume);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].0.task_id, "task-active");
        assert_eq!(selected[0].1, ChildTranscriptResumeSelection::Active);
        assert_eq!(selected[1].0.task_id, "task-failed");
        assert_eq!(
            selected[1].1,
            ChildTranscriptResumeSelection::RecentTerminal
        );
    }

    #[tokio::test]
    async fn has_active_persisted_tasks_ignores_stale_attempts() {
        let parent_session_id = format!("task-ledger-active-{}", Uuid::new_v4());
        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");

        let mut first_attempt = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::SwarmWorker,
            status: TaskStatus::Running,
            description: None,
            write_scope: Vec::new(),
            target_artifacts: Vec::new(),
            result_contract: Vec::new(),
            summary: Some("first attempt still marked running".to_string()),
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 10,
            finished_at: None,
        };
        WorkerAttemptIdentity::fresh("worker-a", "task-1")
            .write_to_metadata(&mut first_attempt.metadata);

        let mut retry_attempt = first_attempt.clone();
        retry_attempt.task_id = "task-2".to_string();
        retry_attempt.status = TaskStatus::Completed;
        retry_attempt.summary = Some("retry completed".to_string());
        retry_attempt.updated_at = 20;
        retry_attempt.finished_at = Some(20);
        retry_attempt.metadata.clear();
        WorkerAttemptIdentity::followup("worker-a", "task-2", 1, "correction", "task-1")
            .write_to_metadata(&mut retry_attempt.metadata);

        upsert_task_ledger_snapshot(&parent_session_id, &first_attempt)
            .await
            .expect("persist first attempt");
        upsert_task_ledger_snapshot(&parent_session_id, &retry_attempt)
            .await
            .expect("persist retry attempt");

        let has_active = has_active_persisted_tasks(&parent_session_id)
            .await
            .expect("load active persisted tasks");
        assert!(!has_active);
    }

    #[tokio::test]
    async fn load_task_ledger_evidence_view_includes_child_session_preview() {
        let parent_session_id = format!("task-evidence-parent-{}", Uuid::new_v4());

        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");

        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Running,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: None,
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "subagent-1".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_PREVIEW_KEY.to_string(),
            "child session preview".to_string(),
        );
        upsert_task_ledger_snapshot(&parent_session_id, &task)
            .await
            .expect("persist task");

        let evidence = load_task_ledger_evidence_view(&parent_session_id)
            .await
            .expect("load evidence");
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].child_session_id.as_deref(), Some("subagent-1"));
        assert_eq!(
            evidence[0].session_preview.as_deref(),
            Some("child session preview")
        );
    }

    #[tokio::test]
    async fn load_task_ledger_evidence_view_hydrates_preview_from_child_session_transcript() {
        let parent_session_id = format!("task-evidence-parent-hydrate-{}", Uuid::new_v4());
        let child_session_id = format!("task-evidence-child-hydrate-{}", Uuid::new_v4());

        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");
        SessionManager::create_session_with_id(
            child_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "child".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create child");

        SessionManager::add_message(
            &child_session_id,
            &Message::assistant().with_content(crate::conversation::message::MessageContent::text(
                "child transcript preview from assistant",
            )),
        )
        .await
        .expect("append child message");

        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Running,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: None,
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            child_session_id,
        );
        upsert_task_ledger_snapshot(&parent_session_id, &task)
            .await
            .expect("persist task");

        let evidence = load_task_ledger_evidence_view(&parent_session_id)
            .await
            .expect("load evidence");
        assert_eq!(evidence.len(), 1);
        assert_eq!(
            evidence[0].session_preview.as_deref(),
            Some("child transcript preview from assistant")
        );
        assert_eq!(
            evidence[0].transcript_excerpt,
            vec!["assistant: child transcript preview from assistant".to_string()]
        );
    }

    #[tokio::test]
    async fn load_task_ledger_evidence_view_hydrates_multi_message_excerpt() {
        let parent_session_id = format!("task-evidence-parent-excerpt-{}", Uuid::new_v4());
        let child_session_id = format!("task-evidence-child-excerpt-{}", Uuid::new_v4());

        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");
        SessionManager::create_session_with_id(
            child_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "child".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create child");

        SessionManager::add_message(
            &child_session_id,
            &Message::user().with_content(crate::conversation::message::MessageContent::text(
                "please inspect docs/a.md",
            )),
        )
        .await
        .expect("append user message");
        SessionManager::add_message(
            &child_session_id,
            &Message::assistant().with_content(crate::conversation::message::MessageContent::text(
                "I am checking the file now",
            )),
        )
        .await
        .expect("append assistant message");
        SessionManager::add_message(
            &child_session_id,
            &Message::assistant().with_content(crate::conversation::message::MessageContent::text(
                "Found the root cause and prepared the fix",
            )),
        )
        .await
        .expect("append assistant summary");

        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Running,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: None,
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            child_session_id,
        );
        upsert_task_ledger_snapshot(&parent_session_id, &task)
            .await
            .expect("persist task");

        let evidence = load_task_ledger_evidence_view(&parent_session_id)
            .await
            .expect("load evidence");
        assert_eq!(evidence.len(), 1);
        assert_eq!(
            evidence[0].transcript_excerpt,
            vec![
                "user: please inspect docs/a.md".to_string(),
                "assistant: I am checking the file now".to_string(),
                "assistant: Found the root cause and prepared the fix".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn load_task_ledger_evidence_view_refreshes_preview_and_backfills_excerpt() {
        let parent_session_id = format!("task-evidence-parent-preview-{}", Uuid::new_v4());
        let child_session_id = format!("task-evidence-child-preview-{}", Uuid::new_v4());

        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");
        SessionManager::create_session_with_id(
            child_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "child".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create child");

        SessionManager::add_message(
            &child_session_id,
            &Message::assistant().with_content(crate::conversation::message::MessageContent::text(
                "assistant terminal line",
            )),
        )
        .await
        .expect("append child message");

        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Completed,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("done".to_string()),
            produced_delta: true,
            accepted_targets: vec!["docs/a.md".to_string()],
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: Some(12),
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            child_session_id,
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_PREVIEW_KEY.to_string(),
            "persisted preview".to_string(),
        );
        upsert_task_ledger_snapshot(&parent_session_id, &task)
            .await
            .expect("persist task");

        let evidence = load_task_ledger_evidence_view(&parent_session_id)
            .await
            .expect("load evidence");
        assert_eq!(evidence.len(), 1);
        assert_eq!(
            evidence[0].session_preview.as_deref(),
            Some("assistant terminal line")
        );
        assert_eq!(
            evidence[0].transcript_excerpt,
            vec!["assistant: assistant terminal line".to_string()]
        );

        let ledger = load_task_ledger_state(&parent_session_id)
            .await
            .expect("load ledger");
        let raw = ledger.tasks[0]
            .metadata
            .get(TASK_LEDGER_CHILD_SESSION_EXCERPT_KEY)
            .cloned()
            .expect("excerpt persisted");
        let parsed = parse_persisted_transcript_excerpt(&raw).expect("parsed excerpt");
        assert_eq!(
            parsed,
            vec!["assistant: assistant terminal line".to_string()]
        );
        assert!(ledger.tasks[0]
            .metadata
            .contains_key(TASK_LEDGER_CHILD_EVIDENCE_SIGNATURE_KEY));
    }

    #[tokio::test]
    async fn load_task_ledger_evidence_view_refreshes_stale_persisted_child_evidence() {
        let parent_session_id = format!("task-evidence-parent-refresh-{}", Uuid::new_v4());
        let child_session_id = format!("task-evidence-child-refresh-{}", Uuid::new_v4());

        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");
        SessionManager::create_session_with_id(
            child_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "child".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create child");

        SessionManager::add_message(
            &child_session_id,
            &Message::assistant().with_content(crate::conversation::message::MessageContent::text(
                "new terminal line",
            )),
        )
        .await
        .expect("append child message");

        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Completed,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("done".to_string()),
            produced_delta: true,
            accepted_targets: vec!["docs/a.md".to_string()],
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: Some(12),
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            child_session_id,
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_PREVIEW_KEY.to_string(),
            "stale preview".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_EXCERPT_KEY.to_string(),
            serde_json::to_string(&vec!["assistant: stale excerpt".to_string()])
                .expect("serialize"),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_EVIDENCE_SIGNATURE_KEY.to_string(),
            "stale-signature".to_string(),
        );
        upsert_task_ledger_snapshot(&parent_session_id, &task)
            .await
            .expect("persist task");

        let evidence = load_task_ledger_evidence_view(&parent_session_id)
            .await
            .expect("load evidence");
        assert_eq!(evidence.len(), 1);
        assert_eq!(
            evidence[0].session_preview.as_deref(),
            Some("new terminal line")
        );
        assert_eq!(
            evidence[0].transcript_excerpt,
            vec!["assistant: new terminal line".to_string()]
        );

        let ledger = load_task_ledger_state(&parent_session_id)
            .await
            .expect("load ledger");
        assert_ne!(
            ledger.tasks[0]
                .metadata
                .get(TASK_LEDGER_CHILD_EVIDENCE_SIGNATURE_KEY)
                .map(String::as_str),
            Some("stale-signature")
        );
    }

    #[tokio::test]
    async fn load_task_ledger_evidence_view_reads_persisted_excerpt_without_child_session() {
        let parent_session_id = format!("task-evidence-parent-persisted-{}", Uuid::new_v4());

        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");

        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Completed,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("done".to_string()),
            produced_delta: true,
            accepted_targets: vec!["docs/a.md".to_string()],
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: Some(12),
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "deleted-child".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_EXCERPT_KEY.to_string(),
            serde_json::to_string(&vec![
                "user: inspect docs/a.md".to_string(),
                "assistant: persisted excerpt".to_string(),
            ])
            .expect("serialize"),
        );
        upsert_task_ledger_snapshot(&parent_session_id, &task)
            .await
            .expect("persist task");

        let evidence = load_task_ledger_evidence_view(&parent_session_id)
            .await
            .expect("load evidence");
        assert_eq!(evidence.len(), 1);
        assert_eq!(
            evidence[0].transcript_excerpt,
            vec![
                "user: inspect docs/a.md".to_string(),
                "assistant: persisted excerpt".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn load_task_ledger_transcript_resume_view_prefers_live_child_session() {
        let parent_session_id = format!("task-resume-parent-live-{}", Uuid::new_v4());
        let child_session_id = format!("task-resume-child-live-{}", Uuid::new_v4());

        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");
        SessionManager::create_session_with_id(
            child_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "child".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create child");

        SessionManager::add_message(
            &child_session_id,
            &Message::user().with_content(crate::conversation::message::MessageContent::text(
                "open docs/a.md",
            )),
        )
        .await
        .expect("append user message");
        SessionManager::add_message(
            &child_session_id,
            &Message::assistant().with_content(crate::conversation::message::MessageContent::text(
                "inspected file and found the issue",
            )),
        )
        .await
        .expect("append assistant message");

        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Running,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("working".to_string()),
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: None,
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            child_session_id,
        );
        upsert_task_ledger_snapshot(&parent_session_id, &task)
            .await
            .expect("persist task");

        let views = load_task_ledger_transcript_resume_view(&parent_session_id)
            .await
            .expect("load resume view");
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].transcript_source, "live_child_session");
        assert_eq!(views[0].message_count, 2);
        assert_eq!(
            views[0].transcript_lines,
            vec![
                "user: open docs/a.md".to_string(),
                "assistant: inspected file and found the issue".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn load_task_ledger_transcript_resume_view_falls_back_to_persisted_excerpt() {
        let parent_session_id = format!("task-resume-parent-persisted-{}", Uuid::new_v4());

        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");

        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Completed,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("done".to_string()),
            produced_delta: true,
            accepted_targets: vec!["docs/a.md".to_string()],
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: Some(12),
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "deleted-child".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_EXCERPT_KEY.to_string(),
            serde_json::to_string(&vec![
                "user: inspect docs/a.md".to_string(),
                "assistant: persisted excerpt".to_string(),
            ])
            .expect("serialize"),
        );
        upsert_task_ledger_snapshot(&parent_session_id, &task)
            .await
            .expect("persist task");

        let views = load_task_ledger_transcript_resume_view(&parent_session_id)
            .await
            .expect("load resume view");
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].transcript_source, "persisted_excerpt");
        assert_eq!(views[0].message_count, 2);
        assert_eq!(
            views[0].transcript_lines,
            vec![
                "user: inspect docs/a.md".to_string(),
                "assistant: persisted excerpt".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn load_task_ledger_transcript_resume_view_refreshes_stale_persisted_resume_metadata() {
        let parent_session_id = format!("task-resume-parent-refresh-{}", Uuid::new_v4());
        let child_session_id = format!("task-resume-child-refresh-{}", Uuid::new_v4());

        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");
        SessionManager::create_session_with_id(
            child_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "child".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create child");

        SessionManager::add_message(
            &child_session_id,
            &Message::assistant().with_content(crate::conversation::message::MessageContent::text(
                "fresh child line",
            )),
        )
        .await
        .expect("append child message");

        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Completed,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("done".to_string()),
            produced_delta: true,
            accepted_targets: vec!["docs/a.md".to_string()],
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: Some(12),
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            child_session_id.clone(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_KEY.to_string(),
            serde_json::to_string(&vec!["assistant: stale line".to_string()]).expect("serialize"),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SIGNATURE_KEY.to_string(),
            "stale-signature".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_MESSAGE_COUNT_KEY.to_string(),
            "1".to_string(),
        );
        upsert_task_ledger_snapshot(&parent_session_id, &task)
            .await
            .expect("persist task");

        let views = load_task_ledger_transcript_resume_view(&parent_session_id)
            .await
            .expect("load resume view");
        assert_eq!(views[0].transcript_source, "live_child_session");
        assert_eq!(
            views[0].transcript_lines,
            vec!["assistant: fresh child line".to_string()]
        );

        let ledger = load_task_ledger_state(&parent_session_id)
            .await
            .expect("load ledger");
        let persisted = ledger.tasks[0]
            .metadata
            .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_KEY)
            .and_then(|value| parse_persisted_transcript_resume(value))
            .expect("persisted resume");
        assert_eq!(persisted, vec!["assistant: fresh child line".to_string()]);
        assert_ne!(
            ledger.tasks[0]
                .metadata
                .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SIGNATURE_KEY)
                .map(String::as_str),
            Some("stale-signature")
        );
    }

    #[tokio::test]
    async fn load_task_ledger_transcript_resume_view_refreshes_status_and_summary_metadata() {
        let parent_session_id = format!("task-resume-parent-status-refresh-{}", Uuid::new_v4());
        let child_session_id = format!("task-resume-child-status-refresh-{}", Uuid::new_v4());

        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");
        SessionManager::create_session_with_id(
            child_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "child".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create child");

        SessionManager::add_message(
            &child_session_id,
            &Message::assistant().with_content(crate::conversation::message::MessageContent::text(
                "fresh child line",
            )),
        )
        .await
        .expect("append child message");
        let child_session = SessionManager::get_session(&child_session_id, true)
            .await
            .expect("load child session");
        let child_updated_at = child_session.updated_at.timestamp();
        let transcript_lines = child_session
            .conversation
            .as_ref()
            .map(build_child_transcript_resume_lines)
            .expect("child conversation");

        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Completed,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("new summary".to_string()),
            produced_delta: true,
            accepted_targets: vec!["docs/a.md".to_string()],
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: Some(12),
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            child_session_id.clone(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_KEY.to_string(),
            serde_json::to_string(&transcript_lines).expect("serialize resume"),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SIGNATURE_KEY.to_string(),
            child_transcript_resume_signature(&transcript_lines).expect("resume signature"),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_MESSAGE_COUNT_KEY.to_string(),
            child_session
                .conversation
                .as_ref()
                .map(|conversation| conversation.messages().len())
                .unwrap_or_default()
                .to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_UPDATED_AT_KEY.to_string(),
            child_updated_at.to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SESSION_ID_KEY.to_string(),
            child_session_id,
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_STATUS_KEY.to_string(),
            "failed".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SUMMARY_KEY.to_string(),
            "old summary".to_string(),
        );
        upsert_task_ledger_snapshot(&parent_session_id, &task)
            .await
            .expect("persist task");

        let views = load_task_ledger_transcript_resume_view(&parent_session_id)
            .await
            .expect("load resume view");
        assert_eq!(views[0].transcript_source, "live_child_session");

        let ledger = load_task_ledger_state(&parent_session_id)
            .await
            .expect("load ledger");
        assert_eq!(
            ledger.tasks[0]
                .metadata
                .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_STATUS_KEY)
                .map(String::as_str),
            Some("completed")
        );
        assert_eq!(
            ledger.tasks[0]
                .metadata
                .get(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SUMMARY_KEY)
                .map(String::as_str),
            Some("new summary")
        );
    }

    #[tokio::test]
    async fn load_task_ledger_transcript_resume_view_downgrades_stale_persisted_resume() {
        let parent_session_id = format!("task-resume-parent-stale-{}", Uuid::new_v4());

        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");

        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Completed,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("new summary".to_string()),
            produced_delta: true,
            accepted_targets: vec!["docs/a.md".to_string()],
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: Some(12),
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "missing-child".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_EXCERPT_KEY.to_string(),
            serde_json::to_string(&vec!["assistant: fallback excerpt".to_string()])
                .expect("serialize excerpt"),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_KEY.to_string(),
            serde_json::to_string(&vec!["assistant: stale persisted resume".to_string()])
                .expect("serialize resume"),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SESSION_ID_KEY.to_string(),
            "missing-child".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_STATUS_KEY.to_string(),
            "failed".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SUMMARY_KEY.to_string(),
            "old summary".to_string(),
        );
        upsert_task_ledger_snapshot(&parent_session_id, &task)
            .await
            .expect("persist task");

        let views = load_task_ledger_transcript_resume_view(&parent_session_id)
            .await
            .expect("load resume view");
        assert_eq!(views[0].transcript_source, "stale_persisted_resume");
        assert_eq!(
            views[0].transcript_lines,
            vec!["assistant: fallback excerpt".to_string()]
        );

        let ledger = load_task_ledger_state(&parent_session_id)
            .await
            .expect("load ledger");
        assert!(!ledger.tasks[0]
            .metadata
            .contains_key(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_KEY));
    }

    #[tokio::test]
    async fn load_task_ledger_transcript_resume_view_downgrades_when_live_child_has_no_resume_lines(
    ) {
        let parent_session_id = format!("task-resume-parent-live-empty-{}", Uuid::new_v4());
        let child_session_id = format!("task-resume-child-live-empty-{}", Uuid::new_v4());

        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");
        SessionManager::create_session_with_id(
            child_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "child".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create child");

        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Running,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("still running".to_string()),
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: None,
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            child_session_id.clone(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_KEY.to_string(),
            serde_json::to_string(&vec!["assistant: stale persisted resume".to_string()])
                .expect("serialize resume"),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SIGNATURE_KEY.to_string(),
            child_transcript_resume_signature(&["assistant: stale persisted resume".to_string()])
                .expect("signature"),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_MESSAGE_COUNT_KEY.to_string(),
            "1".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SESSION_ID_KEY.to_string(),
            child_session_id,
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_STATUS_KEY.to_string(),
            "running".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SUMMARY_KEY.to_string(),
            "still running".to_string(),
        );
        upsert_task_ledger_snapshot(&parent_session_id, &task)
            .await
            .expect("persist task");

        let views = load_task_ledger_transcript_resume_view(&parent_session_id)
            .await
            .expect("load resume view");
        assert_eq!(views[0].transcript_source, "stale_persisted_resume");
        assert_eq!(
            views[0].transcript_lines,
            vec!["summary: still running".to_string()]
        );

        let ledger = load_task_ledger_state(&parent_session_id)
            .await
            .expect("load ledger");
        assert!(!ledger.tasks[0]
            .metadata
            .contains_key(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_KEY));
    }

    #[tokio::test]
    async fn load_task_ledger_transcript_resume_view_downgrades_invalid_persisted_resume_signature()
    {
        let parent_session_id = format!("task-resume-parent-invalid-signature-{}", Uuid::new_v4());

        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");

        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Completed,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("completed summary".to_string()),
            produced_delta: true,
            accepted_targets: vec!["docs/a.md".to_string()],
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: Some(12),
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_ID_KEY.to_string(),
            "missing-child".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_EXCERPT_KEY.to_string(),
            serde_json::to_string(&vec!["assistant: fallback excerpt".to_string()])
                .expect("serialize excerpt"),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_KEY.to_string(),
            serde_json::to_string(&vec!["assistant: persisted resume".to_string()])
                .expect("serialize resume"),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SIGNATURE_KEY.to_string(),
            "bad-signature".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_MESSAGE_COUNT_KEY.to_string(),
            "1".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SESSION_ID_KEY.to_string(),
            "missing-child".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_STATUS_KEY.to_string(),
            "completed".to_string(),
        );
        task.metadata.insert(
            TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_SUMMARY_KEY.to_string(),
            "completed summary".to_string(),
        );
        upsert_task_ledger_snapshot(&parent_session_id, &task)
            .await
            .expect("persist task");

        let views = load_task_ledger_transcript_resume_view(&parent_session_id)
            .await
            .expect("load resume view");
        assert_eq!(views[0].transcript_source, "stale_persisted_resume");
        assert_eq!(
            views[0].transcript_lines,
            vec!["assistant: fallback excerpt".to_string()]
        );

        let ledger = load_task_ledger_state(&parent_session_id)
            .await
            .expect("load ledger");
        assert!(!ledger.tasks[0]
            .metadata
            .contains_key(TASK_LEDGER_CHILD_TRANSCRIPT_RESUME_KEY));
    }

    #[test]
    fn build_persisted_child_evidence_view_normalizes_legacy_preview_metadata() {
        let mut task = TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: "parent".to_string(),
            depth: 1,
            kind: super::super::task_runtime::TaskKind::Subagent,
            status: TaskStatus::Running,
            description: None,
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 11,
            finished_at: None,
        };
        task.metadata.insert(
            TASK_LEDGER_CHILD_SESSION_PREVIEW_KEY.to_string(),
            " legacy\npreview\tvalue ".to_string(),
        );
        let evidence =
            build_persisted_child_evidence_view(&HarnessTaskLedgerState { tasks: vec![task] });
        assert_eq!(evidence.len(), 1);
        assert_eq!(
            evidence[0].session_preview.as_deref(),
            Some("legacy preview value")
        );
    }
}
