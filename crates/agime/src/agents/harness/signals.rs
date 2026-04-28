use std::collections::VecDeque;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::conversation::message::Message;
use crate::conversation::message::SystemNotificationType;

use super::child_tasks::{parse_validation_outcome, ValidationReport, ValidationStatus};
use super::mailbox::{MailboxMessage, MailboxMessageKind};
use super::task_runtime::{
    TaskKind, TaskRuntime, TaskRuntimeEvent, TaskRuntimeHost, WorkerAttemptIdentity,
};
use super::tools::ToolTransportKind;
use super::transcript::{annotate_task_ledger_runtime_state, upsert_task_ledger_snapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentAnalysisPhase {
    AccessNotEstablished,
    AccessEstablished,
    ContentRead,
    AnalysisReadyForValidation,
}

impl DocumentAnalysisPhase {
    pub fn rank(self) -> u8 {
        match self {
            Self::AccessNotEstablished => 0,
            Self::AccessEstablished => 1,
            Self::ContentRead => 2,
            Self::AnalysisReadyForValidation => 3,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AccessNotEstablished => "access_not_established",
            Self::AccessEstablished => "access_established",
            Self::ContentRead => "content_read",
            Self::AnalysisReadyForValidation => "analysis_ready_for_validation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordinatorSignal {
    ToolCompleted {
        request_id: String,
        tool_name: String,
        transport: ToolTransportKind,
    },
    ToolFailed {
        request_id: String,
        tool_name: String,
        transport: ToolTransportKind,
        error: String,
    },
    WorkerCompleted {
        task_id: String,
        kind: TaskKind,
        summary: String,
        accepted_targets: Vec<String>,
        produced_delta: bool,
        recovered_terminal_summary: bool,
        attempt_identity: Option<WorkerAttemptIdentity>,
    },
    WorkerFailed {
        task_id: String,
        kind: TaskKind,
        summary: String,
        recovered_terminal_summary: bool,
        attempt_identity: Option<WorkerAttemptIdentity>,
    },
    ValidationReported {
        report: ValidationReport,
    },
    DocumentPhaseUpdated {
        phase: DocumentAnalysisPhase,
        workspace_path: Option<String>,
        doc_id: Option<String>,
        reason_code: Option<String>,
        next_action: Option<String>,
    },
    WorkerIdle {
        task_id: String,
        message: String,
        attempt_identity: Option<WorkerAttemptIdentity>,
    },
    WorkerFollowupRequested {
        task_id: String,
        kind: String,
        reason: String,
        attempt_identity: Option<WorkerAttemptIdentity>,
    },
    PermissionRequested {
        task_id: String,
        worker_name: Option<String>,
        tool_name: String,
        attempt_identity: Option<WorkerAttemptIdentity>,
    },
    PermissionResolved {
        task_id: String,
        worker_name: Option<String>,
        tool_name: String,
        decision: String,
        source: Option<String>,
        attempt_identity: Option<WorkerAttemptIdentity>,
    },
    PermissionTimedOut {
        task_id: String,
        worker_name: Option<String>,
        tool_name: String,
        timeout_ms: u64,
        attempt_identity: Option<WorkerAttemptIdentity>,
    },
    MailboxMessageReceived {
        from: String,
        to: String,
        kind: String,
        text: String,
        summary: Option<String>,
        attempt_identity: Option<WorkerAttemptIdentity>,
    },
    CompletionReady {
        report: StructuredCompletionSignal,
    },
    FallbackRequested {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerOutcome {
    pub task_id: String,
    pub kind: TaskKind,
    pub status: String,
    pub summary: String,
    pub accepted_targets: Vec<String>,
    pub produced_delta: bool,
    #[serde(default)]
    pub recovered_terminal_summary: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_identity: Option<WorkerAttemptIdentity>,
}

impl Default for WorkerOutcome {
    fn default() -> Self {
        Self {
            task_id: String::new(),
            kind: TaskKind::Subagent,
            status: String::new(),
            summary: String::new(),
            accepted_targets: Vec::new(),
            produced_delta: false,
            recovered_terminal_summary: false,
            attempt_identity: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StructuredCompletionSignal {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub accepted_targets: Vec<String>,
    #[serde(default)]
    pub produced_targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_accessed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis_complete: Option<bool>,
}

pub type ValidationSignalOutcome = ValidationReport;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwarmCompletionEvidence {
    pub accepted_targets: Vec<String>,
    pub produced_targets: Vec<String>,
    pub worker_outcomes: Vec<WorkerOutcome>,
    pub validation_outcomes: Vec<ValidationSignalOutcome>,
}

pub type CoordinatorNotification = CoordinatorSignal;
pub type NotificationQueue = CoordinatorSignalStore;

#[derive(Debug, Default)]
pub struct CoordinatorSignalStore {
    pending: Mutex<VecDeque<CoordinatorNotification>>,
    history: Mutex<Vec<CoordinatorNotification>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinatorSignalSummary {
    pub tool_completed: usize,
    pub tool_failed: usize,
    pub completed_tool_names: Vec<String>,
    pub failed_tool_names: Vec<String>,
    pub worker_completed: usize,
    pub worker_failed: usize,
    pub worker_idle: usize,
    pub worker_followups: usize,
    pub validation_passed: usize,
    pub validation_failed: usize,
    pub permission_requested: usize,
    pub permission_resolved: usize,
    pub permission_timed_out: usize,
    pub mailbox_messages: usize,
    pub fallback_requested: usize,
    pub completion_ready: bool,
    pub latest_completion_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_structured_completion: Option<StructuredCompletionSignal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_phase: Option<DocumentAnalysisPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_workspace_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_next_action: Option<String>,
    pub worker_outcomes: Vec<WorkerOutcome>,
    pub validation_outcomes: Vec<ValidationSignalOutcome>,
    pub accepted_targets: Vec<String>,
    pub produced_targets: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotificationDrainResult {
    pub notifications: Vec<CoordinatorNotification>,
    pub summary: CoordinatorSignalSummary,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeNotificationInput {
    pub notifications: Vec<CoordinatorNotification>,
    pub summary: CoordinatorSignalSummary,
}

pub type SharedCoordinatorSignalStore = Arc<CoordinatorSignalStore>;

const RECOVERED_TERMINAL_SUMMARY_KEY: &str = "recovered_terminal_summary";

impl CoordinatorSignalSummary {
    pub fn from_signals(signals: &[CoordinatorSignal]) -> Self {
        let mut summary = Self::default();
        for signal in signals {
            match signal {
                CoordinatorSignal::ToolCompleted { tool_name, .. } => {
                    summary.tool_completed += 1;
                    if !summary
                        .completed_tool_names
                        .iter()
                        .any(|name| name == tool_name)
                    {
                        summary.completed_tool_names.push(tool_name.clone());
                    }
                }
                CoordinatorSignal::ToolFailed { tool_name, .. } => {
                    summary.tool_failed += 1;
                    if !summary
                        .failed_tool_names
                        .iter()
                        .any(|name| name == tool_name)
                    {
                        summary.failed_tool_names.push(tool_name.clone());
                    }
                }
                CoordinatorSignal::WorkerCompleted {
                    task_id,
                    kind,
                    summary: text,
                    accepted_targets,
                    produced_delta,
                    recovered_terminal_summary,
                    attempt_identity,
                } => {
                    summary.worker_completed += 1;
                    if !summary
                        .worker_outcomes
                        .iter()
                        .any(|outcome| outcome.task_id == *task_id && outcome.status == "completed")
                    {
                        summary.worker_outcomes.push(WorkerOutcome {
                            task_id: task_id.clone(),
                            kind: *kind,
                            status: "completed".to_string(),
                            summary: text.clone(),
                            accepted_targets: accepted_targets.clone(),
                            produced_delta: *produced_delta,
                            recovered_terminal_summary: *recovered_terminal_summary,
                            attempt_identity: attempt_identity.clone(),
                        });
                    }
                    for target in accepted_targets {
                        if !summary.accepted_targets.iter().any(|value| value == target) {
                            summary.accepted_targets.push(target.clone());
                        }
                        if *produced_delta
                            && !summary.produced_targets.iter().any(|value| value == target)
                        {
                            summary.produced_targets.push(target.clone());
                        }
                    }
                }
                CoordinatorSignal::WorkerFailed {
                    task_id,
                    kind,
                    summary: text,
                    recovered_terminal_summary,
                    attempt_identity,
                } => {
                    summary.worker_failed += 1;
                    if !summary
                        .worker_outcomes
                        .iter()
                        .any(|outcome| outcome.task_id == *task_id && outcome.status == "failed")
                    {
                        summary.worker_outcomes.push(WorkerOutcome {
                            task_id: task_id.clone(),
                            kind: *kind,
                            status: "failed".to_string(),
                            summary: text.clone(),
                            accepted_targets: Vec::new(),
                            produced_delta: false,
                            recovered_terminal_summary: *recovered_terminal_summary,
                            attempt_identity: attempt_identity.clone(),
                        });
                    }
                }
                CoordinatorSignal::WorkerIdle { .. } => summary.worker_idle += 1,
                CoordinatorSignal::WorkerFollowupRequested { .. } => summary.worker_followups += 1,
                CoordinatorSignal::ValidationReported { report } => {
                    match report.status {
                        ValidationStatus::Passed => summary.validation_passed += 1,
                        ValidationStatus::Failed => summary.validation_failed += 1,
                        ValidationStatus::NotRun => {}
                    }
                    if !summary.validation_outcomes.iter().any(|outcome| {
                        outcome.validator_task_id == report.validator_task_id
                            && outcome.status == report.status
                            && outcome.reason_code == report.reason_code
                            && outcome.content_accessed == report.content_accessed
                            && outcome.analysis_complete == report.analysis_complete
                            && outcome.target_artifacts == report.target_artifacts
                    }) {
                        summary.validation_outcomes.push(report.clone());
                    }
                }
                CoordinatorSignal::DocumentPhaseUpdated {
                    phase,
                    workspace_path,
                    next_action,
                    ..
                } => {
                    let should_replace = summary
                        .document_phase
                        .map(|existing| phase.rank() >= existing.rank())
                        .unwrap_or(true);
                    if should_replace {
                        summary.document_phase = Some(*phase);
                        if let Some(path) = workspace_path
                            .as_ref()
                            .filter(|value| !value.trim().is_empty())
                        {
                            summary.document_workspace_path = Some(path.clone());
                        }
                        if let Some(action) = next_action
                            .as_ref()
                            .filter(|value| !value.trim().is_empty())
                        {
                            summary.document_next_action = Some(action.clone());
                        }
                    }
                }
                CoordinatorSignal::PermissionRequested { .. } => summary.permission_requested += 1,
                CoordinatorSignal::PermissionResolved { .. } => summary.permission_resolved += 1,
                CoordinatorSignal::PermissionTimedOut { .. } => summary.permission_timed_out += 1,
                CoordinatorSignal::MailboxMessageReceived { .. } => summary.mailbox_messages += 1,
                CoordinatorSignal::FallbackRequested { .. } => summary.fallback_requested += 1,
                CoordinatorSignal::CompletionReady { report } => {
                    summary.completion_ready = true;
                    summary.latest_structured_completion = Some(report.clone());
                    summary.latest_completion_summary = report
                        .summary
                        .clone()
                        .filter(|value| !value.trim().is_empty());
                    if let Some(status) = report.validation_status.as_deref() {
                        let validation_outcome = ValidationSignalOutcome {
                            status: if status.eq_ignore_ascii_case("passed") {
                                ValidationStatus::Passed
                            } else if status.eq_ignore_ascii_case("failed") {
                                ValidationStatus::Failed
                            } else {
                                ValidationStatus::NotRun
                            },
                            reason: if status.eq_ignore_ascii_case("passed") {
                                None
                            } else {
                                report
                                    .blocking_reason
                                    .clone()
                                    .or_else(|| report.summary.clone())
                            },
                            validator_task_id: Some("completion-ready".to_string()),
                            target_artifacts: if report.produced_targets.is_empty() {
                                report.accepted_targets.clone()
                            } else {
                                report.produced_targets.clone()
                            },
                            evidence_summary: report.summary.clone(),
                            reason_code: report.reason_code.clone(),
                            content_accessed: report.content_accessed.unwrap_or(false),
                            analysis_complete: report
                                .analysis_complete
                                .unwrap_or(status.eq_ignore_ascii_case("passed")),
                        };
                        if !summary.validation_outcomes.iter().any(|existing| {
                            existing.validator_task_id == validation_outcome.validator_task_id
                                && existing.status == validation_outcome.status
                        }) {
                            summary.validation_outcomes.push(validation_outcome);
                        }
                    }
                    for target in &report.accepted_targets {
                        if !summary.accepted_targets.iter().any(|value| value == target) {
                            summary.accepted_targets.push(target.clone());
                        }
                    }
                    for target in &report.produced_targets {
                        if !summary.produced_targets.iter().any(|value| value == target) {
                            summary.produced_targets.push(target.clone());
                        }
                    }
                }
            }
            match signal {
                CoordinatorSignal::WorkerCompleted { summary: text, .. }
                | CoordinatorSignal::WorkerFailed { summary: text, .. } => {
                    if !text.trim().is_empty() {
                        summary.latest_completion_summary = Some(text.trim().to_string());
                    }
                }
                CoordinatorSignal::ValidationReported { report } => {
                    if let Some(text) = report.evidence_summary.as_deref().map(str::trim) {
                        if !text.is_empty() {
                            summary.latest_completion_summary = Some(text.to_string());
                        }
                    } else if let Some(reason) = report.reason.as_deref().map(str::trim) {
                        if !reason.is_empty() {
                            summary.latest_completion_summary = Some(reason.to_string());
                        }
                    }
                }
                CoordinatorSignal::FallbackRequested { .. }
                | CoordinatorSignal::WorkerFollowupRequested { .. }
                | CoordinatorSignal::PermissionTimedOut { .. }
                | CoordinatorSignal::WorkerIdle { .. }
                | CoordinatorSignal::PermissionResolved { .. }
                | CoordinatorSignal::PermissionRequested { .. } => {}
                CoordinatorSignal::MailboxMessageReceived {
                    kind,
                    text,
                    summary: mailbox_summary,
                    ..
                } => {
                    if kind == "summary" {
                        summary.latest_completion_summary = mailbox_summary
                            .clone()
                            .filter(|value| !value.trim().is_empty())
                            .or_else(|| (!text.trim().is_empty()).then(|| text.trim().to_string()));
                    }
                }
                CoordinatorSignal::ToolCompleted { .. }
                | CoordinatorSignal::ToolFailed { .. }
                | CoordinatorSignal::DocumentPhaseUpdated { .. } => {}
                CoordinatorSignal::CompletionReady { report } => {
                    if let Some(text) = report.summary.as_deref().map(str::trim) {
                        if !text.is_empty() {
                            summary.latest_completion_summary = Some(text.to_string());
                        }
                    } else if let Some(reason) = report.blocking_reason.as_deref().map(str::trim) {
                        if !reason.is_empty() {
                            summary.latest_completion_summary = Some(reason.to_string());
                        }
                    }
                }
            }
        }
        summary
    }

    pub fn has_blocking_signals(&self) -> bool {
        self.worker_failed > 0
            || self.validation_failed > 0
            || self.permission_timed_out > 0
            || self.fallback_requested > 0
    }

    fn completed_document_read_tool(&self) -> bool {
        self.completed_tool_names
            .iter()
            .any(|name| name == "document_tools__read_document")
    }

    fn validation_failure_contradicted_by_document_read(&self, report: &ValidationReport) -> bool {
        if report.status != ValidationStatus::Failed || !self.completed_document_read_tool() {
            return false;
        }
        if !report
            .target_artifacts
            .iter()
            .any(|artifact| artifact.starts_with("document:"))
        {
            return false;
        }
        let reason = report
            .reason
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        reason.contains("no document read tools")
            || reason.contains("no document tools")
            || reason.contains("document read tools")
            || reason.contains("missing document access")
            || reason.contains("missing required document access tools")
            || reason.contains("document validation requires document_tools")
            || reason.contains("runtime tool surface")
    }

    pub fn has_hard_blocking_signals(&self) -> bool {
        self.permission_timed_out > 0
            || self.fallback_requested > 0
            || self.validation_outcomes.iter().any(|report| {
                report.status == ValidationStatus::Failed
                    && !self.validation_failure_contradicted_by_document_read(report)
            })
    }

    pub fn validation_status(&self) -> Option<&'static str> {
        if self.validation_outcomes.iter().any(|report| {
            report.status == ValidationStatus::Failed
                && !self.validation_failure_contradicted_by_document_read(report)
        }) {
            Some("failed")
        } else if self
            .validation_outcomes
            .iter()
            .any(|report| report.status == ValidationStatus::Passed)
            || self.completed_document_read_tool()
        {
            Some("passed")
        } else {
            None
        }
    }

    pub fn document_access_established(&self) -> bool {
        if self
            .document_phase
            .is_some_and(|phase| phase.rank() >= DocumentAnalysisPhase::AccessEstablished.rank())
        {
            return true;
        }
        self.validation_outcomes.iter().any(|report| {
            report
                .target_artifacts
                .iter()
                .any(|artifact| artifact.starts_with("document:"))
        })
    }

    pub fn document_content_accessed(&self) -> bool {
        if self
            .document_phase
            .is_some_and(|phase| phase.rank() >= DocumentAnalysisPhase::ContentRead.rank())
        {
            return true;
        }
        if self.completed_document_read_tool() {
            return true;
        }
        self.validation_outcomes
            .iter()
            .any(|report| report.content_accessed)
    }

    pub fn document_analysis_complete(&self) -> bool {
        if self.document_phase.is_some_and(|phase| {
            phase.rank() >= DocumentAnalysisPhase::AnalysisReadyForValidation.rank()
        }) {
            return true;
        }
        self.validation_outcomes
            .iter()
            .any(|report| report.analysis_complete)
    }

    pub fn default_blocking_reason(&self) -> Option<String> {
        if let Some(reason) = self
            .validation_outcomes
            .iter()
            .find(|report| {
                report.status == ValidationStatus::Failed
                    && !self.validation_failure_contradicted_by_document_read(report)
            })
            .and_then(|report| report.reason.clone())
        {
            Some(reason)
        } else if self.has_hard_blocking_signals() && self.validation_failed > 0 {
            Some("validation failed".to_string())
        } else if self.worker_failed > 0 {
            Some("worker execution failed".to_string())
        } else if self.permission_timed_out > 0 {
            Some("permission request timed out".to_string())
        } else if self.fallback_requested > 0 {
            Some("fallback requested by runtime".to_string())
        } else {
            None
        }
    }

    pub fn completion_evidence(&self) -> SwarmCompletionEvidence {
        SwarmCompletionEvidence {
            accepted_targets: self.accepted_targets.clone(),
            produced_targets: self.produced_targets.clone(),
            worker_outcomes: self.worker_outcomes.clone(),
            validation_outcomes: self.validation_outcomes.clone(),
        }
    }

    pub fn satisfies_required_tool_prefixes(&self, required_prefixes: &[String]) -> bool {
        if required_prefixes.is_empty() {
            return true;
        }
        required_prefixes.iter().any(|prefix| {
            self.completed_tool_names
                .iter()
                .any(|tool_name| tool_name.starts_with(prefix))
        })
    }
}

impl NotificationDrainResult {
    pub fn has_notifications(&self) -> bool {
        !self.notifications.is_empty()
    }

    pub fn into_runtime_input(self) -> Option<RuntimeNotificationInput> {
        if self.notifications.is_empty() {
            return None;
        }
        Some(RuntimeNotificationInput {
            notifications: self.notifications,
            summary: self.summary,
        })
    }

    pub fn into_agent_input_message(&self) -> Option<Message> {
        let Some(input) = RuntimeNotificationInput {
            notifications: self.notifications.clone(),
            summary: self.summary.clone(),
        }
        .into_agent_input_message() else {
            return None;
        };
        Some(input)
    }
}

impl RuntimeNotificationInput {
    pub fn has_notifications(&self) -> bool {
        !self.notifications.is_empty()
    }

    fn render_notification_batch_text(&self) -> String {
        let mut lines = vec!["<task-notification-batch>".to_string()];
        for notification in &self.notifications {
            match notification {
                CoordinatorSignal::ToolCompleted {
                    request_id,
                    tool_name,
                    transport,
                } => {
                    lines.push("<task-notification>".to_string());
                    lines.push("<kind>tool-completed</kind>".to_string());
                    lines.push(format!("<request-id>{}</request-id>", request_id));
                    lines.push(format!("<tool>{}</tool>", tool_name));
                    lines.push(format!("<transport>{:?}</transport>", transport));
                    lines.push("</task-notification>".to_string());
                }
                CoordinatorSignal::ToolFailed {
                    request_id,
                    tool_name,
                    transport,
                    error,
                } => {
                    lines.push("<task-notification>".to_string());
                    lines.push("<kind>tool-failed</kind>".to_string());
                    lines.push(format!("<request-id>{}</request-id>", request_id));
                    lines.push(format!("<tool>{}</tool>", tool_name));
                    lines.push(format!("<transport>{:?}</transport>", transport));
                    lines.push(format!("<summary>{}</summary>", error));
                    lines.push("</task-notification>".to_string());
                }
                CoordinatorSignal::WorkerCompleted {
                    task_id,
                    kind,
                    summary,
                    attempt_identity,
                    ..
                } => {
                    lines.push("<task-notification>".to_string());
                    lines.push("<kind>worker-completed</kind>".to_string());
                    lines.push(format!("<task-id>{}</task-id>", task_id));
                    if let Some(identity) = attempt_identity {
                        lines.push(format!(
                            "<logical-worker-id>{}</logical-worker-id>",
                            identity.logical_worker_id
                        ));
                        lines.push(format!("<attempt-id>{}</attempt-id>", identity.attempt_id));
                        lines.push(format!(
                            "<attempt-index>{}</attempt-index>",
                            identity.attempt_index
                        ));
                        lines.push(format!(
                            "<followup-kind>{}</followup-kind>",
                            identity.followup_kind
                        ));
                    }
                    lines.push(format!("<worker-kind>{:?}</worker-kind>", kind));
                    lines.push(format!("<summary>{}</summary>", summary));
                    lines.push("</task-notification>".to_string());
                }
                CoordinatorSignal::WorkerFailed {
                    task_id,
                    kind,
                    summary,
                    attempt_identity,
                    ..
                } => {
                    lines.push("<task-notification>".to_string());
                    lines.push("<kind>worker-failed</kind>".to_string());
                    lines.push(format!("<task-id>{}</task-id>", task_id));
                    if let Some(identity) = attempt_identity {
                        lines.push(format!(
                            "<logical-worker-id>{}</logical-worker-id>",
                            identity.logical_worker_id
                        ));
                        lines.push(format!("<attempt-id>{}</attempt-id>", identity.attempt_id));
                        lines.push(format!(
                            "<attempt-index>{}</attempt-index>",
                            identity.attempt_index
                        ));
                        lines.push(format!(
                            "<followup-kind>{}</followup-kind>",
                            identity.followup_kind
                        ));
                    }
                    lines.push(format!("<worker-kind>{:?}</worker-kind>", kind));
                    lines.push(format!("<summary>{}</summary>", summary));
                    lines.push("</task-notification>".to_string());
                }
                CoordinatorSignal::ValidationReported { report } => {
                    lines.push("<task-notification>".to_string());
                    lines.push("<kind>validation-reported</kind>".to_string());
                    lines.push(format!(
                        "<validation-status>{}</validation-status>",
                        report.status.as_str()
                    ));
                    if let Some(task_id) = &report.validator_task_id {
                        lines.push(format!("<task-id>{}</task-id>", task_id));
                    }
                    if let Some(summary) = &report.evidence_summary {
                        lines.push(format!("<summary>{}</summary>", summary));
                    }
                    if let Some(reason) = &report.reason {
                        lines.push(format!("<reason>{}</reason>", reason));
                    }
                    lines.push("</task-notification>".to_string());
                }
                CoordinatorSignal::WorkerIdle {
                    task_id,
                    message,
                    attempt_identity,
                } => {
                    lines.push("<task-notification>".to_string());
                    lines.push("<kind>worker-idle</kind>".to_string());
                    lines.push(format!("<task-id>{}</task-id>", task_id));
                    if let Some(identity) = attempt_identity {
                        lines.push(format!(
                            "<logical-worker-id>{}</logical-worker-id>",
                            identity.logical_worker_id
                        ));
                        lines.push(format!("<attempt-id>{}</attempt-id>", identity.attempt_id));
                    }
                    lines.push(format!("<summary>{}</summary>", message));
                    lines.push("</task-notification>".to_string());
                }
                CoordinatorSignal::WorkerFollowupRequested {
                    task_id,
                    kind,
                    reason,
                    attempt_identity,
                } => {
                    lines.push("<task-notification>".to_string());
                    lines.push("<kind>worker-followup-requested</kind>".to_string());
                    lines.push(format!("<task-id>{}</task-id>", task_id));
                    if let Some(identity) = attempt_identity {
                        lines.push(format!(
                            "<logical-worker-id>{}</logical-worker-id>",
                            identity.logical_worker_id
                        ));
                        lines.push(format!("<attempt-id>{}</attempt-id>", identity.attempt_id));
                        lines.push(format!(
                            "<attempt-index>{}</attempt-index>",
                            identity.attempt_index
                        ));
                    }
                    lines.push(format!("<followup-kind>{}</followup-kind>", kind));
                    lines.push(format!("<summary>{}</summary>", reason));
                    lines.push("</task-notification>".to_string());
                }
                CoordinatorSignal::PermissionRequested {
                    task_id,
                    tool_name,
                    attempt_identity,
                    ..
                } => {
                    lines.push("<task-notification>".to_string());
                    lines.push("<kind>permission-requested</kind>".to_string());
                    lines.push(format!("<task-id>{}</task-id>", task_id));
                    if let Some(identity) = attempt_identity {
                        lines.push(format!(
                            "<logical-worker-id>{}</logical-worker-id>",
                            identity.logical_worker_id
                        ));
                        lines.push(format!("<attempt-id>{}</attempt-id>", identity.attempt_id));
                    }
                    lines.push(format!("<tool>{}</tool>", tool_name));
                    lines.push("</task-notification>".to_string());
                }
                CoordinatorSignal::PermissionResolved {
                    task_id,
                    tool_name,
                    decision,
                    source,
                    attempt_identity,
                    ..
                } => {
                    lines.push("<task-notification>".to_string());
                    lines.push("<kind>permission-resolved</kind>".to_string());
                    lines.push(format!("<task-id>{}</task-id>", task_id));
                    if let Some(identity) = attempt_identity {
                        lines.push(format!(
                            "<logical-worker-id>{}</logical-worker-id>",
                            identity.logical_worker_id
                        ));
                        lines.push(format!("<attempt-id>{}</attempt-id>", identity.attempt_id));
                    }
                    lines.push(format!("<tool>{}</tool>", tool_name));
                    lines.push(format!("<decision>{}</decision>", decision));
                    if let Some(source) = source.as_ref() {
                        lines.push(format!("<source>{}</source>", source));
                    }
                    lines.push("</task-notification>".to_string());
                }
                CoordinatorSignal::PermissionTimedOut {
                    task_id,
                    tool_name,
                    timeout_ms,
                    attempt_identity,
                    ..
                } => {
                    lines.push("<task-notification>".to_string());
                    lines.push("<kind>permission-timed-out</kind>".to_string());
                    lines.push(format!("<task-id>{}</task-id>", task_id));
                    if let Some(identity) = attempt_identity {
                        lines.push(format!(
                            "<logical-worker-id>{}</logical-worker-id>",
                            identity.logical_worker_id
                        ));
                        lines.push(format!("<attempt-id>{}</attempt-id>", identity.attempt_id));
                    }
                    lines.push(format!("<tool>{}</tool>", tool_name));
                    lines.push(format!("<summary>{} ms</summary>", timeout_ms));
                    lines.push("</task-notification>".to_string());
                }
                CoordinatorSignal::MailboxMessageReceived {
                    from,
                    to,
                    kind,
                    text,
                    summary,
                    attempt_identity,
                } => {
                    lines.push("<task-notification>".to_string());
                    lines.push("<kind>mailbox-message</kind>".to_string());
                    lines.push(format!("<mailbox-kind>{}</mailbox-kind>", kind));
                    lines.push(format!("<from>{}</from>", from));
                    lines.push(format!("<to>{}</to>", to));
                    if let Some(identity) = attempt_identity {
                        lines.push(format!(
                            "<logical-worker-id>{}</logical-worker-id>",
                            identity.logical_worker_id
                        ));
                        lines.push(format!("<attempt-id>{}</attempt-id>", identity.attempt_id));
                    }
                    if let Some(summary) = summary {
                        lines.push(format!("<summary>{}</summary>", summary));
                    }
                    if !text.trim().is_empty() {
                        lines.push(format!("<text>{}</text>", text));
                    }
                    lines.push("</task-notification>".to_string());
                }
                CoordinatorSignal::CompletionReady { report } => {
                    lines.push("<task-notification>".to_string());
                    lines.push("<kind>completion-ready</kind>".to_string());
                    lines.push(format!("<status>{}</status>", report.status));
                    if let Some(summary) = report.summary.as_deref() {
                        lines.push(format!("<summary>{}</summary>", summary));
                    }
                    if !report.accepted_targets.is_empty() {
                        lines.push(format!(
                            "<accepted-targets>{}</accepted-targets>",
                            report.accepted_targets.join(",")
                        ));
                    }
                    if !report.produced_targets.is_empty() {
                        lines.push(format!(
                            "<produced-targets>{}</produced-targets>",
                            report.produced_targets.join(",")
                        ));
                    }
                    if let Some(validation_status) = report.validation_status.as_deref() {
                        lines.push(format!(
                            "<validation-status>{}</validation-status>",
                            validation_status
                        ));
                    }
                    if let Some(blocking_reason) = report.blocking_reason.as_deref() {
                        lines.push(format!(
                            "<blocking-reason>{}</blocking-reason>",
                            blocking_reason
                        ));
                    }
                    lines.push("</task-notification>".to_string());
                }
                CoordinatorSignal::DocumentPhaseUpdated {
                    phase,
                    workspace_path,
                    doc_id,
                    reason_code,
                    next_action,
                } => {
                    lines.push("<task-notification>".to_string());
                    lines.push("<kind>document-phase</kind>".to_string());
                    lines.push(format!("<phase>{}</phase>", phase.as_str()));
                    if let Some(doc_id) = doc_id.as_deref() {
                        lines.push(format!("<document-id>{}</document-id>", doc_id));
                    }
                    if let Some(path) = workspace_path.as_deref() {
                        lines.push(format!("<workspace-path>{}</workspace-path>", path));
                    }
                    if let Some(reason_code) = reason_code.as_deref() {
                        lines.push(format!("<reason-code>{}</reason-code>", reason_code));
                    }
                    if let Some(next_action) = next_action.as_deref() {
                        lines.push(format!("<next-action>{}</next-action>", next_action));
                    }
                    lines.push("</task-notification>".to_string());
                }
                CoordinatorSignal::FallbackRequested { reason } => {
                    lines.push("<task-notification>".to_string());
                    lines.push("<kind>fallback-requested</kind>".to_string());
                    lines.push(format!("<summary>{}</summary>", reason));
                    lines.push("</task-notification>".to_string());
                }
            }
        }
        if let Some(summary) = self.summary.latest_completion_summary.as_deref() {
            lines.push("<notification-summary>".to_string());
            lines.push(format!("<summary>{}</summary>", summary));
            lines.push("</notification-summary>".to_string());
        }
        lines.push("</task-notification-batch>".to_string());
        lines.join("\n")
    }

    pub fn render_for_system_prompt(&self) -> String {
        format!(
            "<runtime_notifications>\n{}\n</runtime_notifications>",
            self.render_notification_batch_text()
        )
    }

    pub fn into_agent_input_message(self) -> Option<Message> {
        if self.notifications.is_empty() {
            return None;
        }
        Some(
            Message::user()
                .with_system_notification(
                    SystemNotificationType::RuntimeNotificationAttachment,
                    self.render_notification_batch_text(),
                )
                .agent_only(),
        )
    }
}

impl CoordinatorSignalStore {
    pub fn shared() -> SharedCoordinatorSignalStore {
        Arc::new(Self::default())
    }

    pub async fn record(&self, signal: CoordinatorSignal) {
        self.pending.lock().await.push_back(signal.clone());
        self.history.lock().await.push(signal);
    }

    pub async fn snapshot(&self) -> Vec<CoordinatorSignal> {
        self.history.lock().await.clone()
    }

    pub async fn drain(&self) -> Vec<CoordinatorSignal> {
        let mut guard = self.pending.lock().await;
        guard.drain(..).collect()
    }

    pub async fn summarize(&self) -> CoordinatorSignalSummary {
        CoordinatorSignalSummary::from_signals(&self.snapshot().await)
    }

    pub async fn drain_notifications(&self) -> NotificationDrainResult {
        let notifications = self.drain().await;
        let summary = CoordinatorSignalSummary::from_signals(&notifications);
        NotificationDrainResult {
            notifications,
            summary,
        }
    }
}

pub fn mailbox_message_to_notification(message: &MailboxMessage) -> CoordinatorNotification {
    let kind = match message.protocol.as_ref().map(|protocol| &protocol.kind) {
        Some(super::mailbox::SwarmProtocolKind::Directive) => "directive",
        Some(super::mailbox::SwarmProtocolKind::Progress) => "progress",
        Some(super::mailbox::SwarmProtocolKind::Summary) => "summary",
        Some(super::mailbox::SwarmProtocolKind::PeerMessage) => "peer_message",
        Some(super::mailbox::SwarmProtocolKind::PeerReply) => "peer_reply",
        Some(super::mailbox::SwarmProtocolKind::PermissionRequest) => "permission_request",
        Some(super::mailbox::SwarmProtocolKind::PermissionResolution) => "permission_resolution",
        Some(super::mailbox::SwarmProtocolKind::FollowupRequest) => "followup_request",
        Some(super::mailbox::SwarmProtocolKind::IdleNotification) => "idle_notification",
        Some(super::mailbox::SwarmProtocolKind::ShutdownNotification) => "shutdown_notification",
        Some(super::mailbox::SwarmProtocolKind::PlanControl) => "plan_control",
        None => match message.kind {
            MailboxMessageKind::Directive => "directive",
            MailboxMessageKind::Progress => "progress",
            MailboxMessageKind::Summary => "summary",
            MailboxMessageKind::PeerMessage => "peer_message",
            MailboxMessageKind::PeerReply => "peer_reply",
            MailboxMessageKind::PermissionRequest => "permission_request",
            MailboxMessageKind::PermissionResolution => "permission_resolution",
            MailboxMessageKind::FollowupRequest => "followup_request",
            MailboxMessageKind::IdleNotification => "idle_notification",
            MailboxMessageKind::ShutdownNotification => "shutdown_notification",
            MailboxMessageKind::PlanControl => "plan_control",
        },
    };
    CoordinatorSignal::MailboxMessageReceived {
        from: message.from.clone(),
        to: message.to.clone(),
        kind: kind.to_string(),
        text: message.text.clone(),
        summary: message.summary.clone(),
        attempt_identity: WorkerAttemptIdentity::from_metadata(&message.metadata),
    }
}

pub fn spawn_task_runtime_signal_bridge(
    task_runtime: Arc<TaskRuntime>,
    parent_session_id: String,
    signal_store: SharedCoordinatorSignalStore,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        #[derive(Clone)]
        struct TrackedTaskState {
            attempt_identity: Option<WorkerAttemptIdentity>,
            target_artifacts: Vec<String>,
        }

        let mut tracked_tasks = std::collections::HashMap::<String, TrackedTaskState>::new();
        let mut rx = task_runtime.subscribe_all();
        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            match event {
                TaskRuntimeEvent::Started(snapshot) => {
                    if snapshot.parent_session_id == parent_session_id {
                        let _ = upsert_task_ledger_snapshot(&parent_session_id, &snapshot).await;
                        tracked_tasks.insert(
                            snapshot.task_id,
                            TrackedTaskState {
                                attempt_identity: WorkerAttemptIdentity::from_metadata(
                                    &snapshot.metadata,
                                ),
                                target_artifacts: snapshot.target_artifacts,
                            },
                        );
                    }
                }
                TaskRuntimeEvent::PermissionTimedOut {
                    task_id,
                    worker_name,
                    tool_name,
                    timeout_ms,
                } => {
                    let _ = annotate_task_ledger_runtime_state(
                        &parent_session_id,
                        &task_id,
                        "permission_timed_out",
                        worker_name
                            .clone()
                            .map(|name| format!("{} timed out on {}", name, tool_name)),
                        Some(tool_name.clone()),
                        None,
                        Some(timeout_ms),
                    )
                    .await;
                    if let Some(state) = tracked_tasks.get(&task_id) {
                        signal_store
                            .record(CoordinatorSignal::PermissionTimedOut {
                                task_id,
                                worker_name,
                                tool_name,
                                timeout_ms,
                                attempt_identity: state.attempt_identity.clone(),
                            })
                            .await;
                    }
                }
                TaskRuntimeEvent::PermissionRequested {
                    task_id,
                    worker_name,
                    tool_name,
                } => {
                    let _ = annotate_task_ledger_runtime_state(
                        &parent_session_id,
                        &task_id,
                        "permission_requested",
                        worker_name
                            .clone()
                            .map(|name| format!("{} waiting on {}", name, tool_name)),
                        Some(tool_name.clone()),
                        None,
                        None,
                    )
                    .await;
                    if let Some(state) = tracked_tasks.get(&task_id) {
                        signal_store
                            .record(CoordinatorSignal::PermissionRequested {
                                task_id,
                                worker_name,
                                tool_name,
                                attempt_identity: state.attempt_identity.clone(),
                            })
                            .await;
                    }
                }
                TaskRuntimeEvent::PermissionResolved {
                    task_id,
                    worker_name,
                    tool_name,
                    decision,
                    source,
                } => {
                    let _ = annotate_task_ledger_runtime_state(
                        &parent_session_id,
                        &task_id,
                        "permission_resolved",
                        worker_name
                            .clone()
                            .map(|name| format!("{} resolved {}", name, tool_name)),
                        Some(tool_name.clone()),
                        Some(decision.clone()),
                        None,
                    )
                    .await;
                    if let Some(state) = tracked_tasks.get(&task_id) {
                        signal_store
                            .record(CoordinatorSignal::PermissionResolved {
                                task_id,
                                worker_name,
                                tool_name,
                                decision,
                                source,
                                attempt_identity: state.attempt_identity.clone(),
                            })
                            .await;
                    }
                }
                TaskRuntimeEvent::FollowupRequested {
                    task_id,
                    kind,
                    reason,
                } => {
                    let _ = annotate_task_ledger_runtime_state(
                        &parent_session_id,
                        &task_id,
                        "followup_requested",
                        Some(format!("{}: {}", kind, reason)),
                        None,
                        None,
                        None,
                    )
                    .await;
                    if let Some(state) = tracked_tasks.get(&task_id) {
                        signal_store
                            .record(CoordinatorSignal::WorkerFollowupRequested {
                                task_id,
                                kind,
                                reason,
                                attempt_identity: state.attempt_identity.clone(),
                            })
                            .await;
                    }
                }
                TaskRuntimeEvent::Idle { task_id, message } => {
                    let _ = annotate_task_ledger_runtime_state(
                        &parent_session_id,
                        &task_id,
                        "idle",
                        Some(message.clone()),
                        None,
                        None,
                        None,
                    )
                    .await;
                    if let Some(state) = tracked_tasks.get(&task_id) {
                        signal_store
                            .record(CoordinatorSignal::WorkerIdle {
                                task_id,
                                message,
                                attempt_identity: state.attempt_identity.clone(),
                            })
                            .await;
                    }
                }
                TaskRuntimeEvent::Completed(result) => {
                    let task_id = result.task_id.clone();
                    if let Some(snapshot) = task_runtime.snapshot(&result.task_id).await {
                        if snapshot.parent_session_id != parent_session_id {
                            continue;
                        }
                        let _ = upsert_task_ledger_snapshot(&parent_session_id, &snapshot).await;
                        let attempt_identity = tracked_tasks
                            .get(&result.task_id)
                            .and_then(|state| state.attempt_identity.clone())
                            .or_else(|| WorkerAttemptIdentity::from_metadata(&snapshot.metadata));
                        let target_artifacts = tracked_tasks
                            .get(&result.task_id)
                            .map(|state| state.target_artifacts.clone())
                            .unwrap_or_else(|| snapshot.target_artifacts.clone());
                        if result.kind == TaskKind::ValidationWorker {
                            let report = parse_validation_outcome(&result.summary)
                                .with_validator_context(Some(task_id.clone()), target_artifacts);
                            let signal = CoordinatorSignal::ValidationReported { report };
                            signal_store.record(signal).await;
                        } else {
                            let recovered_terminal_summary = result
                                .metadata
                                .get(RECOVERED_TERMINAL_SUMMARY_KEY)
                                .is_some_and(|value| value == "true");
                            signal_store
                                .record(CoordinatorSignal::WorkerCompleted {
                                    task_id,
                                    kind: result.kind,
                                    summary: result.summary,
                                    accepted_targets: result.accepted_targets,
                                    produced_delta: result.produced_delta,
                                    recovered_terminal_summary,
                                    attempt_identity,
                                })
                                .await;
                        }
                        tracked_tasks.remove(&result.task_id);
                    }
                }
                TaskRuntimeEvent::Failed(result) => {
                    let task_id = result.task_id.clone();
                    if let Some(snapshot) = task_runtime.snapshot(&result.task_id).await {
                        if snapshot.parent_session_id != parent_session_id {
                            continue;
                        }
                        let _ = upsert_task_ledger_snapshot(&parent_session_id, &snapshot).await;
                        let attempt_identity = tracked_tasks
                            .get(&result.task_id)
                            .and_then(|state| state.attempt_identity.clone())
                            .or_else(|| WorkerAttemptIdentity::from_metadata(&snapshot.metadata));
                        let target_artifacts = tracked_tasks
                            .get(&result.task_id)
                            .map(|state| state.target_artifacts.clone())
                            .unwrap_or_else(|| snapshot.target_artifacts.clone());
                        if result.kind == TaskKind::ValidationWorker {
                            let report = parse_validation_outcome(&result.summary)
                                .with_validator_context(Some(task_id.clone()), target_artifacts);
                            signal_store
                                .record(CoordinatorSignal::ValidationReported { report })
                                .await;
                        } else {
                            let recovered_terminal_summary = result
                                .metadata
                                .get(RECOVERED_TERMINAL_SUMMARY_KEY)
                                .is_some_and(|value| value == "true");
                            signal_store
                                .record(CoordinatorSignal::WorkerFailed {
                                    task_id,
                                    kind: result.kind,
                                    summary: result.summary,
                                    recovered_terminal_summary,
                                    attempt_identity,
                                })
                                .await;
                        }
                        tracked_tasks.remove(&result.task_id);
                    }
                }
                TaskRuntimeEvent::Cancelled { task_id } => {
                    if let Some(snapshot) = task_runtime.snapshot(&task_id).await {
                        if snapshot.parent_session_id == parent_session_id {
                            let _ =
                                upsert_task_ledger_snapshot(&parent_session_id, &snapshot).await;
                        }
                    }
                    tracked_tasks.remove(&task_id);
                }
                TaskRuntimeEvent::Progress { .. } => {}
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::harness::load_task_ledger_state;
    use crate::agents::harness::task_runtime::{TaskResultEnvelope, TaskRuntimeHost};
    use crate::conversation::message::MessageMetadata;
    use crate::conversation::message::{MessageContent, SystemNotificationType};
    use crate::session::{SessionManager, SessionType};
    use std::collections::HashMap;
    use uuid::Uuid;

    #[tokio::test]
    async fn task_runtime_signal_bridge_records_worker_and_validation_signals() {
        let parent_session_id = format!("parent-{}", Uuid::new_v4());
        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "signal-bridge-parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent session");

        let runtime = TaskRuntime::shared();
        let signals = CoordinatorSignalStore::shared();
        let bridge = spawn_task_runtime_signal_bridge(
            runtime.clone(),
            parent_session_id.clone(),
            signals.clone(),
        );

        let handle = runtime
            .spawn_task(super::super::task_runtime::TaskSpec {
                task_id: "worker-1".to_string(),
                parent_session_id: parent_session_id.clone(),
                depth: 1,
                kind: TaskKind::SwarmWorker,
                description: None,
                write_scope: vec!["docs".to_string()],
                target_artifacts: vec!["docs/a.md".to_string()],
                result_contract: vec!["docs/a.md".to_string()],
                metadata: HashMap::new(),
            })
            .await
            .expect("spawn task");
        runtime
            .complete(TaskResultEnvelope {
                task_id: handle.task_id,
                kind: TaskKind::SwarmWorker,
                status: super::super::task_runtime::TaskStatus::Completed,
                summary: "updated docs/a.md".to_string(),
                accepted_targets: vec!["docs/a.md".to_string()],
                produced_delta: true,
                metadata: HashMap::new(),
            })
            .await
            .expect("complete");

        let validator = runtime
            .spawn_task(super::super::task_runtime::TaskSpec {
                task_id: "validator-1".to_string(),
                parent_session_id: parent_session_id.clone(),
                depth: 1,
                kind: TaskKind::ValidationWorker,
                description: None,
                write_scope: Vec::new(),
                target_artifacts: vec!["docs/a.md".to_string()],
                result_contract: vec!["docs/a.md".to_string()],
                metadata: HashMap::new(),
            })
            .await
            .expect("spawn validator");
        runtime
            .complete(TaskResultEnvelope {
                task_id: validator.task_id,
                kind: TaskKind::ValidationWorker,
                status: super::super::task_runtime::TaskStatus::Completed,
                summary: "PASS: artifact verified".to_string(),
                accepted_targets: Vec::new(),
                produced_delta: false,
                metadata: HashMap::new(),
            })
            .await
            .expect("complete validator");

        let mut snapshot = Vec::new();
        for _ in 0..20 {
            snapshot = signals.snapshot().await;
            let has_worker = snapshot.iter().any(|signal| {
                matches!(
                    signal,
                    CoordinatorSignal::WorkerCompleted { task_id, .. } if task_id == "worker-1"
                )
            });
            let has_validator = snapshot.iter().any(|signal| {
                matches!(
                    signal,
                    CoordinatorSignal::ValidationReported { report }
                        if report.validator_task_id.as_deref() == Some("validator-1")
                            && report.status == ValidationStatus::Passed
                )
            });
            if has_worker && has_validator {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        bridge.abort();

        assert!(snapshot.iter().any(|signal| matches!(
            signal,
            CoordinatorSignal::WorkerCompleted { task_id, .. } if task_id == "worker-1"
        )));
        assert!(snapshot.iter().any(|signal| matches!(
            signal,
            CoordinatorSignal::ValidationReported { report }
                if report.validator_task_id.as_deref() == Some("validator-1")
                    && report.status == ValidationStatus::Passed
        )));
    }

    #[tokio::test]
    async fn task_runtime_signal_bridge_persists_task_ledger_for_parent_session() {
        let parent_session_id = format!("parent-ledger-{}", Uuid::new_v4());
        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "task-ledger-parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent session");

        let runtime = TaskRuntime::shared();
        let signals = CoordinatorSignalStore::shared();
        let bridge =
            spawn_task_runtime_signal_bridge(runtime.clone(), parent_session_id.clone(), signals);
        tokio::task::yield_now().await;

        let handle = runtime
            .spawn_task(super::super::task_runtime::TaskSpec {
                task_id: "worker-ledger-1".to_string(),
                parent_session_id: parent_session_id.clone(),
                depth: 1,
                kind: TaskKind::SwarmWorker,
                description: Some("worker".to_string()),
                write_scope: vec!["docs".to_string()],
                target_artifacts: vec!["docs/a.md".to_string()],
                result_contract: vec!["docs/a.md".to_string()],
                metadata: HashMap::new(),
            })
            .await
            .expect("spawn task");

        runtime
            .complete(TaskResultEnvelope {
                task_id: handle.task_id,
                kind: TaskKind::SwarmWorker,
                status: super::super::task_runtime::TaskStatus::Completed,
                summary: "updated docs/a.md".to_string(),
                accepted_targets: vec!["docs/a.md".to_string()],
                produced_delta: true,
                metadata: HashMap::new(),
            })
            .await
            .expect("complete task");

        let mut ledger = load_task_ledger_state(&parent_session_id)
            .await
            .expect("load task ledger");
        for _ in 0..20 {
            if !ledger.tasks.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            ledger = load_task_ledger_state(&parent_session_id)
                .await
                .expect("reload task ledger");
        }
        bridge.abort();

        assert_eq!(ledger.tasks.len(), 1);
        assert_eq!(ledger.tasks[0].task_id, "worker-ledger-1");
        assert_eq!(
            ledger.tasks[0].status,
            super::super::task_runtime::TaskStatus::Completed
        );
        assert_eq!(
            ledger.tasks[0].summary.as_deref(),
            Some("updated docs/a.md")
        );
    }

    #[tokio::test]
    async fn task_runtime_signal_bridge_persists_runtime_markers_to_task_ledger() {
        let parent_session_id = format!("parent-ledger-runtime-{}", Uuid::new_v4());
        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "task-ledger-runtime-parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent session");

        let runtime = TaskRuntime::shared();
        let signals = CoordinatorSignalStore::shared();
        let bridge =
            spawn_task_runtime_signal_bridge(runtime.clone(), parent_session_id.clone(), signals);
        tokio::task::yield_now().await;

        let handle = runtime
            .spawn_task(super::super::task_runtime::TaskSpec {
                task_id: "worker-runtime-1".to_string(),
                parent_session_id: parent_session_id.clone(),
                depth: 1,
                kind: TaskKind::SwarmWorker,
                description: Some("worker".to_string()),
                write_scope: vec!["docs".to_string()],
                target_artifacts: vec!["docs/a.md".to_string()],
                result_contract: vec!["docs/a.md".to_string()],
                metadata: HashMap::new(),
            })
            .await
            .expect("spawn task");

        runtime
            .record_permission_requested(
                &handle.task_id,
                Some("worker-a".to_string()),
                "developer__shell".to_string(),
            )
            .await
            .expect("record permission requested");

        let mut ledger = load_task_ledger_state(&parent_session_id)
            .await
            .expect("load task ledger");
        for _ in 0..20 {
            let task = ledger
                .tasks
                .iter()
                .find(|task| task.task_id == handle.task_id);
            if task
                .and_then(|task| task.metadata.get("runtime_state"))
                .is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            ledger = load_task_ledger_state(&parent_session_id)
                .await
                .expect("reload task ledger");
        }
        bridge.abort();

        let task = ledger
            .tasks
            .iter()
            .find(|task| task.task_id == "worker-runtime-1")
            .expect("persisted task");
        assert_eq!(
            task.metadata.get("runtime_state").map(String::as_str),
            Some("permission_requested")
        );
        assert_eq!(
            task.metadata
                .get("permission_tool_name")
                .map(String::as_str),
            Some("developer__shell")
        );
    }

    #[test]
    fn coordinator_signal_summary_counts_blocking_and_completion_state() {
        let summary = CoordinatorSignalSummary::from_signals(&[
            CoordinatorSignal::ToolCompleted {
                request_id: "req-1".to_string(),
                tool_name: "developer__write_file".to_string(),
                transport: ToolTransportKind::ServerLocal,
            },
            CoordinatorSignal::WorkerFailed {
                task_id: "worker-1".to_string(),
                kind: TaskKind::SwarmWorker,
                summary: "FAIL: worker crashed".to_string(),
                recovered_terminal_summary: false,
                attempt_identity: None,
            },
            CoordinatorSignal::CompletionReady {
                report: StructuredCompletionSignal {
                    status: "completed".to_string(),
                    summary: Some("done".to_string()),
                    accepted_targets: Vec::new(),
                    produced_targets: Vec::new(),
                    validation_status: None,
                    blocking_reason: None,
                    reason_code: None,
                    content_accessed: None,
                    analysis_complete: None,
                },
            },
        ]);

        assert_eq!(summary.tool_completed, 1);
        assert_eq!(
            summary.completed_tool_names,
            vec!["developer__write_file".to_string()]
        );
        assert_eq!(summary.worker_failed, 1);
        assert!(summary.completion_ready);
        assert_eq!(summary.latest_completion_summary.as_deref(), Some("done"));
        assert!(summary.has_blocking_signals());
        assert!(!summary.has_hard_blocking_signals());
    }

    #[test]
    fn coordinator_signal_summary_checks_required_tool_prefixes() {
        let summary = CoordinatorSignalSummary::from_signals(&[CoordinatorSignal::ToolCompleted {
            request_id: "req-1".to_string(),
            tool_name: "document_tools__read_document".to_string(),
            transport: ToolTransportKind::ServerLocal,
        }]);

        assert!(summary.satisfies_required_tool_prefixes(&["document_tools__".to_string()]));
        assert!(!summary.satisfies_required_tool_prefixes(&["developer__".to_string()]));
    }

    #[test]
    fn document_read_evidence_overrides_stale_missing_tool_validation_failure() {
        let summary = CoordinatorSignalSummary::from_signals(&[
            CoordinatorSignal::ToolCompleted {
                request_id: "req-1".to_string(),
                tool_name: "document_tools__read_document".to_string(),
                transport: ToolTransportKind::ServerLocal,
            },
            CoordinatorSignal::ValidationReported {
                report: ValidationReport {
                    status: ValidationStatus::Failed,
                    reason: Some(
                        "No document read tools are available in the current tool surface."
                            .to_string(),
                    ),
                    reason_code: None,
                    validator_task_id: Some("validator-1".to_string()),
                    target_artifacts: vec!["document:doc-1".to_string()],
                    evidence_summary: None,
                    content_accessed: false,
                    analysis_complete: false,
                },
            },
        ]);

        assert!(!summary.has_hard_blocking_signals());
        assert_eq!(summary.validation_status(), Some("passed"));
        assert!(summary.document_content_accessed());
        assert_eq!(summary.default_blocking_reason(), None);
    }

    #[test]
    fn document_read_evidence_overrides_direct_host_validation_tool_surface_failure() {
        let summary = CoordinatorSignalSummary::from_signals(&[
            CoordinatorSignal::ToolCompleted {
                request_id: "req-1".to_string(),
                tool_name: "document_tools__read_document".to_string(),
                transport: ToolTransportKind::ServerLocal,
            },
            CoordinatorSignal::ValidationReported {
                report: ValidationReport {
                    status: ValidationStatus::Failed,
                    reason: Some(
                        "Missing required document access tools. The runtime tool surface only includes skills__loadSkill and recipe__final_output. Document validation requires document_tools."
                            .to_string(),
                    ),
                    reason_code: None,
                    validator_task_id: Some("validator-1".to_string()),
                    target_artifacts: vec!["document:doc-1".to_string()],
                    evidence_summary: None,
                    content_accessed: false,
                    analysis_complete: false,
                },
            },
        ]);

        assert!(!summary.has_hard_blocking_signals());
        assert_eq!(summary.validation_status(), Some("passed"));
        assert_eq!(summary.default_blocking_reason(), None);
    }

    #[test]
    fn tool_and_document_progress_do_not_become_completion_summary() {
        let summary = CoordinatorSignalSummary::from_signals(&[
            CoordinatorSignal::ToolCompleted {
                request_id: "req-1".to_string(),
                tool_name: "document_tools__document_inventory".to_string(),
                transport: ToolTransportKind::ServerLocal,
            },
            CoordinatorSignal::DocumentPhaseUpdated {
                phase: DocumentAnalysisPhase::AccessEstablished,
                workspace_path: Some("/tmp/docs".to_string()),
                doc_id: Some("doc-1".to_string()),
                reason_code: None,
                next_action: Some("read_document".to_string()),
            },
        ]);

        assert_eq!(summary.tool_completed, 1);
        assert_eq!(
            summary.document_phase,
            Some(DocumentAnalysisPhase::AccessEstablished)
        );
        assert_eq!(summary.latest_completion_summary, None);
    }

    #[tokio::test]
    async fn drain_notifications_returns_pending_items_without_losing_history() {
        let signals = CoordinatorSignalStore::shared();
        signals
            .record(CoordinatorSignal::ToolCompleted {
                request_id: "req-1".to_string(),
                tool_name: "document_tools__read_document".to_string(),
                transport: ToolTransportKind::ServerLocal,
            })
            .await;
        signals
            .record(CoordinatorSignal::WorkerCompleted {
                task_id: "worker-1".to_string(),
                kind: TaskKind::SwarmWorker,
                summary: "worker finished".to_string(),
                accepted_targets: vec!["docs/a.md".to_string()],
                produced_delta: true,
                recovered_terminal_summary: false,
                attempt_identity: None,
            })
            .await;

        let drained = signals.drain_notifications().await;
        assert_eq!(drained.notifications.len(), 2);
        assert_eq!(drained.summary.tool_completed, 1);
        assert_eq!(drained.summary.worker_completed, 1);
        assert!(signals.drain().await.is_empty());

        let history = signals.summarize().await;
        assert_eq!(history.tool_completed, 1);
        assert_eq!(history.worker_completed, 1);
    }

    #[test]
    fn notification_drain_result_builds_agent_only_task_notification_message() {
        let drained = NotificationDrainResult {
            notifications: vec![CoordinatorSignal::WorkerCompleted {
                task_id: "worker-1".to_string(),
                kind: TaskKind::SwarmWorker,
                summary: "implemented the change".to_string(),
                accepted_targets: vec!["docs/a.md".to_string()],
                produced_delta: true,
                recovered_terminal_summary: false,
                attempt_identity: None,
            }],
            summary: CoordinatorSignalSummary {
                worker_completed: 1,
                latest_completion_summary: Some("implemented the change".to_string()),
                ..Default::default()
            },
        };

        let message = drained
            .into_agent_input_message()
            .expect("notification message");
        assert_eq!(message.metadata, MessageMetadata::agent_only());
        assert!(matches!(
            message.content.first(),
            Some(MessageContent::SystemNotification(notification))
                if notification.notification_type
                    == SystemNotificationType::RuntimeNotificationAttachment
                    && notification.msg.contains("<task-notification-batch>")
                    && notification.msg.contains("worker-completed")
                    && notification.msg.contains("implemented the change")
        ));
    }
}
