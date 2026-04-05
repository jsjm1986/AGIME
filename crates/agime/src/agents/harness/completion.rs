use serde::{Deserialize, Serialize};

use crate::conversation::Conversation;

use super::{CoordinatorSignalSummary, FinalOutputState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteCompletionState {
    Running,
    AwaitingNotifications,
    AwaitingCompletion,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecuteCompletionOutcome {
    pub state: ExecuteCompletionState,
    pub status: String,
    pub summary: Option<String>,
    pub blocking_reason: Option<String>,
    pub completion_ready: bool,
    pub required_tools_satisfied: bool,
    pub has_blocking_signals: bool,
    pub active_child_tasks: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ExecutionHostCompletionReport {
    pub status: String,
    pub summary: String,
    #[serde(default)]
    pub produced_artifacts: Vec<String>,
    #[serde(default)]
    pub accepted_artifacts: Vec<String>,
    #[serde(default)]
    pub next_steps: Vec<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionSurfacePolicy {
    Conversation,
    Execute,
    SystemDocumentAnalysis,
}

impl ExecuteCompletionState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Blocked)
    }
}

pub fn parse_execution_host_completion_report(
    raw: Option<&str>,
) -> Option<ExecutionHostCompletionReport> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    serde_json::from_str(raw).ok()
}

pub fn format_execution_host_completion_text(report: &ExecutionHostCompletionReport) -> String {
    let mut lines = vec![report.summary.trim().to_string()];
    if let Some(validation_status) = report.validation_status.as_deref() {
        lines.push(format!("Validation: {}", validation_status));
    }
    if let Some(blocking_reason) = report
        .blocking_reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Blocking reason: {}", blocking_reason));
    }
    if !report.accepted_artifacts.is_empty() {
        lines.push(format!(
            "Accepted artifacts: {}",
            report.accepted_artifacts.join(", ")
        ));
    }
    if !report.next_steps.is_empty() {
        lines.push("Next steps:".to_string());
        lines.extend(report.next_steps.iter().map(|step| format!("- {}", step)));
    }
    lines.join("\n")
}

fn completion_report_from_outcome(
    outcome: &ExecuteCompletionOutcome,
    signal_summary: Option<&CoordinatorSignalSummary>,
) -> ExecutionHostCompletionReport {
    let status = if outcome.status.eq_ignore_ascii_case("completed") {
        "completed"
    } else {
        "blocked"
    };
    let summary = outcome
        .summary
        .clone()
        .or_else(|| outcome.blocking_reason.clone())
        .or_else(|| signal_summary.and_then(|summary| summary.latest_completion_summary.clone()))
        .unwrap_or_else(|| {
            if status == "completed" {
                "Execution completed from runtime-owned completion outcome.".to_string()
            } else {
                "Execution blocked from runtime-owned completion outcome.".to_string()
            }
        });

    let evidence = signal_summary.map(CoordinatorSignalSummary::completion_evidence);
    ExecutionHostCompletionReport {
        status: status.to_string(),
        summary,
        produced_artifacts: evidence
            .as_ref()
            .map(|value| value.produced_targets.clone())
            .unwrap_or_default(),
        accepted_artifacts: evidence
            .as_ref()
            .map(|value| value.accepted_targets.clone())
            .unwrap_or_default(),
        next_steps: if status == "completed" {
            Vec::new()
        } else {
            vec![
                "Provide additional context or retry after resolving the blocking issue."
                    .to_string(),
            ]
        },
        validation_status: Some(
            signal_summary
                .and_then(CoordinatorSignalSummary::validation_status)
                .unwrap_or("not_run")
                .to_string(),
        ),
        blocking_reason: if status == "completed" {
            None
        } else {
            outcome
                .blocking_reason
                .clone()
                .or_else(|| Some("runtime-owned completion outcome is blocked".to_string()))
        },
        reason_code: None,
        content_accessed: None,
        analysis_complete: None,
    }
}

fn merge_structured_completion_override(
    mut base: ExecutionHostCompletionReport,
    override_report: ExecutionHostCompletionReport,
) -> ExecutionHostCompletionReport {
    if !override_report.summary.trim().is_empty() {
        base.summary = override_report.summary;
    }
    if !override_report.produced_artifacts.is_empty() {
        base.produced_artifacts = override_report.produced_artifacts;
    }
    if !override_report.accepted_artifacts.is_empty() {
        base.accepted_artifacts = override_report.accepted_artifacts;
    }
    if !override_report.next_steps.is_empty() {
        base.next_steps = override_report.next_steps;
    }
    if override_report.validation_status.is_some() {
        base.validation_status = override_report.validation_status;
    }
    if override_report.blocking_reason.is_some() {
        base.blocking_reason = override_report.blocking_reason;
    }
    if override_report.reason_code.is_some() {
        base.reason_code = override_report.reason_code;
    }
    if override_report.content_accessed.is_some() {
        base.content_accessed = override_report.content_accessed;
    }
    if override_report.analysis_complete.is_some() {
        base.analysis_complete = override_report.analysis_complete;
    }
    if base.status.eq_ignore_ascii_case("completed") {
        base.blocking_reason = None;
    }
    base
}

fn runtime_completion_evidence_ready(signal_summary: &CoordinatorSignalSummary) -> bool {
    signal_summary.latest_structured_completion.is_some()
        || signal_summary.completion_ready
        || !signal_summary.accepted_targets.is_empty()
        || !signal_summary.produced_targets.is_empty()
        || signal_summary.worker_completed > 0
        || signal_summary.validation_passed > 0
}

fn latest_user_visible_assistant_text(conversation: &Conversation) -> Option<String> {
    conversation
        .messages()
        .iter()
        .rev()
        .find(|message| {
            message.role == rmcp::model::Role::Assistant && message.metadata.user_visible
        })
        .and_then(|message| {
            let text = message.as_concat_text().trim().to_string();
            (!text.is_empty()).then_some(text)
        })
}

fn summary_indicates_document_content_unavailable(summary: &str) -> bool {
    let normalized = summary.trim().to_ascii_lowercase();
    [
        "content is unavailable",
        "content not available",
        "actual content returned was empty",
        "actual document content was empty",
        "document contains no readable text content",
        "contains no readable text content",
        "no readable text content",
        "analysis is based on the document access result",
        "actual content was not successfully read",
        "actual content is unavailable",
        "unable to complete",
        "could not access the document content",
        "内容不可用",
        "未成功读取",
        "内容为空",
        "没有可读文本内容",
        "基于文档访问结果进行分析",
        "无法完成对文档",
        "无法读取文档内容",
    ]
    .iter()
    .any(|pattern| normalized.contains(pattern))
}

fn summary_indicates_future_intent(summary: &str) -> bool {
    let normalized = summary.trim().to_ascii_lowercase();
    [
        "i need to read the document first",
        "i'll start by reading",
        "let me do that now",
        "before i can provide a final output",
        "需要先读取文档",
        "让我先",
        "下一步",
    ]
    .iter()
    .any(|pattern| normalized.contains(pattern))
}

pub fn normalize_execution_host_completion_report(
    mut report: ExecutionHostCompletionReport,
    signal_summary: Option<&CoordinatorSignalSummary>,
    required_tool_prefixes: &[String],
) -> ExecutionHostCompletionReport {
    if report.produced_artifacts.is_empty() {
        report.produced_artifacts = signal_summary
            .map(CoordinatorSignalSummary::completion_evidence)
            .map(|value| value.produced_targets)
            .unwrap_or_default();
    }

    if report.accepted_artifacts.is_empty() {
        report.accepted_artifacts = signal_summary
            .map(CoordinatorSignalSummary::completion_evidence)
            .map(|value| value.accepted_targets)
            .unwrap_or_default();
    }

    if report.summary.trim().is_empty() {
        report.summary = signal_summary
            .and_then(|summary| summary.latest_completion_summary.clone())
            .unwrap_or_else(|| {
                if report.status.eq_ignore_ascii_case("completed") {
                    "Execution completed without a structured completion report.".to_string()
                } else {
                    "Execution blocked without a structured completion report.".to_string()
                }
            });
    }

    if report.validation_status.is_none() {
        report.validation_status = Some(
            signal_summary
                .and_then(CoordinatorSignalSummary::validation_status)
                .unwrap_or("not_run")
                .to_string(),
        );
    }

    let required_tools_satisfied = signal_summary
        .map(|summary| summary.satisfies_required_tool_prefixes(required_tool_prefixes))
        .unwrap_or_else(|| required_tool_prefixes.is_empty());
    if !required_tools_satisfied {
        report.status = "blocked".to_string();
        report.blocking_reason = Some(format!(
            "required tool contract not satisfied: {}",
            required_tool_prefixes.join(", ")
        ));
    } else if report.status.eq_ignore_ascii_case("completed")
        && signal_summary.is_some_and(CoordinatorSignalSummary::has_blocking_signals)
    {
        report.status = "blocked".to_string();
        if report.blocking_reason.is_none() {
            report.blocking_reason = signal_summary
                .and_then(CoordinatorSignalSummary::default_blocking_reason)
                .or_else(|| Some("runtime signals indicate incomplete execution".to_string()));
        }
    }

    if report.status.eq_ignore_ascii_case("blocked") && report.next_steps.is_empty() {
        report.next_steps.push(
            "Provide additional context or retry after resolving the blocking issue.".to_string(),
        );
    }

    if report.status.eq_ignore_ascii_case("completed") {
        report.blocking_reason = None;
    }

    report
}

pub fn build_execute_completion_report(
    final_output_raw: Option<&str>,
    completion_outcome: Option<&ExecuteCompletionOutcome>,
    signal_summary: Option<&CoordinatorSignalSummary>,
    required_tool_prefixes: &[String],
) -> ExecutionHostCompletionReport {
    if let Some(outcome) = completion_outcome.filter(|outcome| outcome.state.is_terminal()) {
        let runtime_report = normalize_execution_host_completion_report(
            completion_report_from_outcome(outcome, signal_summary),
            signal_summary,
            required_tool_prefixes,
        );
        if let Some(report) = parse_execution_host_completion_report(final_output_raw) {
            let override_report = normalize_execution_host_completion_report(
                report,
                signal_summary,
                required_tool_prefixes,
            );
            return merge_structured_completion_override(runtime_report, override_report);
        }
        return runtime_report;
    }

    let runtime_blocked_report = normalize_execution_host_completion_report(
        ExecutionHostCompletionReport {
            status: "blocked".to_string(),
            summary: "Runtime exited without terminal completion evidence.".to_string(),
            produced_artifacts: Vec::new(),
            accepted_artifacts: Vec::new(),
            next_steps: vec![
                "Provide additional context or retry after resolving the blocking issue."
                    .to_string(),
            ],
            validation_status: signal_summary
                .and_then(CoordinatorSignalSummary::validation_status)
                .map(str::to_string),
            blocking_reason: Some(
                "runtime exited without terminal completion evidence".to_string(),
            ),
            reason_code: None,
            content_accessed: None,
            analysis_complete: None,
        },
        signal_summary,
        required_tool_prefixes,
    );

    if let Some(report) = parse_execution_host_completion_report(final_output_raw) {
        let override_report = normalize_execution_host_completion_report(
            report,
            signal_summary,
            required_tool_prefixes,
        );
        return merge_structured_completion_override(runtime_blocked_report, override_report);
    }

    runtime_blocked_report
}

pub fn build_conversation_completion_report(
    final_conversation: &Conversation,
    completion_outcome: Option<&ExecuteCompletionOutcome>,
    signal_summary: Option<&CoordinatorSignalSummary>,
) -> ExecutionHostCompletionReport {
    if let Some(outcome) = completion_outcome.filter(|outcome| outcome.state.is_terminal()) {
        if outcome.status.eq_ignore_ascii_case("blocked") {
            return normalize_execution_host_completion_report(
                completion_report_from_outcome(outcome, signal_summary),
                signal_summary,
                &[],
            );
        }
    }

    let assistant_summary = latest_user_visible_assistant_text(final_conversation);
    let has_blocking_signals =
        signal_summary.is_some_and(CoordinatorSignalSummary::has_blocking_signals);
    if let Some(summary) = assistant_summary
        .or_else(|| completion_outcome.and_then(|outcome| outcome.summary.clone()))
        .filter(|_| !has_blocking_signals)
    {
        let evidence = signal_summary.map(CoordinatorSignalSummary::completion_evidence);
        return ExecutionHostCompletionReport {
            status: "completed".to_string(),
            summary,
            produced_artifacts: evidence
                .as_ref()
                .map(|value| value.produced_targets.clone())
                .unwrap_or_default(),
            accepted_artifacts: evidence
                .as_ref()
                .map(|value| value.accepted_targets.clone())
                .unwrap_or_default(),
            next_steps: Vec::new(),
            validation_status: Some(
                signal_summary
                    .and_then(CoordinatorSignalSummary::validation_status)
                    .unwrap_or("not_run")
                    .to_string(),
            ),
            blocking_reason: None,
            reason_code: None,
            content_accessed: None,
            analysis_complete: None,
        };
    }

    ExecutionHostCompletionReport {
        status: "blocked".to_string(),
        summary: "Runtime exited without a user-visible assistant response.".to_string(),
        produced_artifacts: Vec::new(),
        accepted_artifacts: Vec::new(),
        next_steps: vec![
            "Provide additional context or retry after resolving the blocking issue.".to_string(),
        ],
        validation_status: Some(
            signal_summary
                .and_then(CoordinatorSignalSummary::validation_status)
                .unwrap_or("not_run")
                .to_string(),
        ),
        blocking_reason: completion_outcome
            .and_then(|outcome| outcome.blocking_reason.clone())
            .or_else(|| signal_summary.and_then(CoordinatorSignalSummary::default_blocking_reason))
            .or_else(|| {
                Some("runtime exited without a user-visible assistant response".to_string())
            }),
        reason_code: None,
        content_accessed: None,
        analysis_complete: None,
    }
}

pub fn normalize_system_document_analysis_completion_report(
    mut report: ExecutionHostCompletionReport,
    signal_summary: Option<&CoordinatorSignalSummary>,
) -> ExecutionHostCompletionReport {
    let document_access_established = signal_summary
        .map(CoordinatorSignalSummary::document_access_established)
        .unwrap_or(false);
    let document_content_accessed = report.content_accessed.unwrap_or_else(|| {
        signal_summary
            .map(CoordinatorSignalSummary::document_content_accessed)
            .unwrap_or(false)
    });
    let analysis_complete = report.analysis_complete.unwrap_or_else(|| {
        signal_summary
            .map(CoordinatorSignalSummary::document_analysis_complete)
            .unwrap_or(false)
    });

    report.content_accessed = Some(document_content_accessed);
    report.analysis_complete = Some(analysis_complete);

    let blocking_reason = if !document_access_established {
        Some("document-area access was not established".to_string())
    } else if !document_content_accessed {
        Some("document content was not successfully accessed".to_string())
    } else if summary_indicates_document_content_unavailable(&report.summary) {
        Some("document analysis reported that content was unavailable".to_string())
    } else if summary_indicates_future_intent(&report.summary) {
        Some(
            "document analysis summary is still future intent; final analysis content is missing"
                .to_string(),
        )
    } else if !analysis_complete {
        Some("document analysis did not reach a terminal complete state".to_string())
    } else {
        None
    };

    report.validation_status = Some(if blocking_reason.is_none() {
        "passed".to_string()
    } else if document_access_established {
        "failed".to_string()
    } else {
        "not_run".to_string()
    });

    if let Some(blocking_reason) = blocking_reason {
        report.status = "blocked".to_string();
        report.reason_code = report
            .reason_code
            .take()
            .or_else(|| Some("document_analysis_incomplete".to_string()));
        report.blocking_reason = Some(blocking_reason);
    } else {
        report.status = "completed".to_string();
        report.blocking_reason = None;
        report.reason_code = report
            .reason_code
            .take()
            .or_else(|| Some("document_analysis_completed".to_string()));
    }

    if report.status.eq_ignore_ascii_case("blocked") && report.next_steps.is_empty() {
        report.next_steps.push(
            "Re-read the document content and complete the final analysis before retrying."
                .to_string(),
        );
    }

    report
}

pub fn build_system_document_analysis_completion_report(
    final_output_raw: Option<&str>,
    completion_outcome: Option<&ExecuteCompletionOutcome>,
    signal_summary: Option<&CoordinatorSignalSummary>,
    required_tool_prefixes: &[String],
) -> ExecutionHostCompletionReport {
    normalize_system_document_analysis_completion_report(
        build_execute_completion_report(
            final_output_raw,
            completion_outcome,
            signal_summary,
            required_tool_prefixes,
        ),
        signal_summary,
    )
}

pub fn derive_execute_completion_outcome(
    signal_summary: &CoordinatorSignalSummary,
    _final_output_state: &FinalOutputState,
    required_tool_prefixes: &[String],
    active_child_tasks: bool,
) -> ExecuteCompletionOutcome {
    let runtime_evidence_ready = runtime_completion_evidence_ready(signal_summary);
    let completion_ready = runtime_evidence_ready;
    let required_tools_satisfied =
        signal_summary.satisfies_required_tool_prefixes(required_tool_prefixes);
    let has_blocking_signals = signal_summary.has_blocking_signals();

    let structured_report = signal_summary.latest_structured_completion.clone();

    let state = if active_child_tasks {
        ExecuteCompletionState::AwaitingNotifications
    } else if let Some(report) = structured_report.as_ref() {
        if report.status.eq_ignore_ascii_case("completed")
            && required_tools_satisfied
            && !has_blocking_signals
        {
            ExecuteCompletionState::Completed
        } else {
            ExecuteCompletionState::Blocked
        }
    } else if has_blocking_signals || !required_tools_satisfied {
        ExecuteCompletionState::Blocked
    } else if completion_ready {
        ExecuteCompletionState::Completed
    } else {
        ExecuteCompletionState::Running
    };

    let status = if matches!(state, ExecuteCompletionState::Completed) {
        "completed"
    } else if matches!(state, ExecuteCompletionState::Blocked) {
        "blocked"
    } else {
        "running"
    };

    let blocking_reason = if matches!(state, ExecuteCompletionState::Blocked) {
        if !required_tools_satisfied {
            Some(format!(
                "required tool contract not satisfied: {}",
                required_tool_prefixes.join(", ")
            ))
        } else if let Some(report) = structured_report.as_ref() {
            report
                .blocking_reason
                .clone()
                .or_else(|| signal_summary.default_blocking_reason())
                .or_else(|| Some("structured completion report is blocked".to_string()))
        } else {
            signal_summary
                .default_blocking_reason()
                .or_else(|| Some("runtime signals indicate incomplete execution".to_string()))
        }
    } else {
        None
    };

    let summary = structured_report
        .as_ref()
        .and_then(|report| report.summary.clone())
        .or_else(|| signal_summary.latest_completion_summary.clone());

    ExecuteCompletionOutcome {
        state,
        status: status.to_string(),
        summary,
        blocking_reason,
        completion_ready,
        required_tools_satisfied,
        has_blocking_signals,
        active_child_tasks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::harness::{TaskKind, WorkerOutcome};
    use crate::conversation::message::Message;
    use crate::conversation::Conversation;

    #[test]
    fn completion_report_uses_worker_evidence_as_artifacts() {
        let mut summary = CoordinatorSignalSummary::default();
        summary.worker_outcomes.push(WorkerOutcome {
            task_id: "worker-1".to_string(),
            kind: TaskKind::SwarmWorker,
            status: "completed".to_string(),
            summary: "updated docs/out.md".to_string(),
            accepted_targets: vec!["docs/out.md".to_string()],
            produced_delta: true,
            attempt_identity: None,
        });
        summary.accepted_targets = vec!["docs/out.md".to_string()];
        summary.produced_targets = vec!["docs/out.md".to_string()];
        summary.latest_completion_summary = Some("updated docs/out.md".to_string());

        let report = build_execute_completion_report(None, None, Some(&summary), &[]);
        assert_eq!(report.status, "blocked");
        assert_eq!(report.accepted_artifacts, vec!["docs/out.md".to_string()]);
        assert_eq!(report.produced_artifacts, vec!["docs/out.md".to_string()]);
    }

    #[test]
    fn derive_execute_completion_outcome_completes_from_runtime_evidence_without_final_output() {
        let summary = CoordinatorSignalSummary {
            worker_completed: 1,
            accepted_targets: vec!["docs/out.md".to_string()],
            produced_targets: vec!["docs/out.md".to_string()],
            latest_completion_summary: Some("updated docs/out.md".to_string()),
            ..Default::default()
        };

        let outcome = derive_execute_completion_outcome(
            &summary,
            &FinalOutputState {
                tool_present: true,
                collected_output: None,
            },
            &[],
            false,
        );

        assert_eq!(outcome.state, ExecuteCompletionState::Completed);
        assert_eq!(outcome.status, "completed");
    }

    #[test]
    fn derive_execute_completion_outcome_uses_structured_completion_signal() {
        let summary = CoordinatorSignalSummary {
            latest_structured_completion: Some(
                crate::agents::harness::StructuredCompletionSignal {
                    status: "completed".to_string(),
                    summary: Some("structured execute completion".to_string()),
                    accepted_targets: vec!["channel:demo/thread:1".to_string()],
                    produced_targets: vec!["channel:demo/thread:1".to_string()],
                    validation_status: Some("not_run".to_string()),
                    blocking_reason: None,
                    reason_code: None,
                    content_accessed: None,
                    analysis_complete: None,
                },
            ),
            ..Default::default()
        };

        let outcome = derive_execute_completion_outcome(
            &summary,
            &FinalOutputState {
                tool_present: true,
                collected_output: None,
            },
            &[],
            false,
        );

        assert_eq!(outcome.state, ExecuteCompletionState::Completed);
        assert_eq!(
            outcome.summary.as_deref(),
            Some("structured execute completion")
        );
    }

    #[test]
    fn structured_final_output_no_longer_overrides_terminal_status() {
        let summary = CoordinatorSignalSummary {
            worker_completed: 1,
            accepted_targets: vec!["docs/out.md".to_string()],
            produced_targets: vec!["docs/out.md".to_string()],
            latest_completion_summary: Some("runtime says completed".to_string()),
            ..Default::default()
        };
        let outcome = ExecuteCompletionOutcome {
            state: ExecuteCompletionState::Completed,
            status: "completed".to_string(),
            summary: Some("runtime says completed".to_string()),
            blocking_reason: None,
            completion_ready: true,
            required_tools_satisfied: true,
            has_blocking_signals: false,
            active_child_tasks: false,
        };

        let report = build_execute_completion_report(
            Some(
                r#"{"status":"blocked","summary":"serializer says blocked","produced_artifacts":["docs/out.md"],"accepted_artifacts":["docs/out.md"],"next_steps":["retry"],"blocking_reason":"serializer-only"}"#,
            ),
            Some(&outcome),
            Some(&summary),
            &[],
        );

        assert_eq!(report.status, "completed");
        assert_eq!(report.summary, "serializer says blocked");
        assert_eq!(report.blocking_reason, None);
    }

    #[test]
    fn structured_final_output_without_runtime_terminal_evidence_stays_blocked() {
        let report = build_execute_completion_report(
            Some(
                r#"{"status":"completed","summary":"serializer only","produced_artifacts":["docs/out.md"],"accepted_artifacts":["docs/out.md"],"next_steps":[]}"#,
            ),
            None,
            None,
            &[],
        );

        assert_eq!(report.status, "blocked");
        assert_eq!(report.summary, "serializer only");
        assert_eq!(
            report.blocking_reason.as_deref(),
            Some("runtime exited without terminal completion evidence")
        );
    }

    #[test]
    fn conversation_completion_uses_user_visible_assistant_text() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::user().with_text("hello"));
        conversation.push(Message::assistant().with_text("hi there"));

        let report = build_conversation_completion_report(&conversation, None, None);

        assert_eq!(report.status, "completed");
        assert_eq!(report.summary, "hi there");
        assert_eq!(report.blocking_reason, None);
    }

    #[test]
    fn system_document_analysis_blocks_when_content_not_accessed() {
        let summary = CoordinatorSignalSummary {
            validation_outcomes: vec![super::super::ValidationReport {
                status: super::super::ValidationStatus::NotRun,
                reason: None,
                reason_code: Some("document_exported_to_workspace".to_string()),
                validator_task_id: None,
                target_artifacts: vec!["document:doc-1".to_string()],
                evidence_summary: Some("Document exported to workspace successfully.".to_string()),
                content_accessed: false,
                analysis_complete: false,
            }],
            completed_tool_names: vec!["document_tools__export_document".to_string()],
            ..Default::default()
        };
        let report = build_system_document_analysis_completion_report(
            Some(
                r#"{"status":"completed","summary":"analysis pending","produced_artifacts":[],"accepted_artifacts":["document:doc-1"],"next_steps":["continue"],"content_accessed":false,"analysis_complete":false,"reason_code":"document_exported_to_workspace"}"#,
            ),
            None,
            Some(&summary),
            &["document_tools__".to_string()],
        );

        assert_eq!(report.status, "blocked");
        assert_eq!(report.validation_status.as_deref(), Some("failed"));
        assert_eq!(report.content_accessed, Some(false));
    }

    #[test]
    fn system_document_analysis_completes_when_content_accessed_and_analysis_complete() {
        let summary = CoordinatorSignalSummary {
            validation_outcomes: vec![super::super::ValidationReport {
                status: super::super::ValidationStatus::NotRun,
                reason: None,
                reason_code: Some("document_content_available".to_string()),
                validator_task_id: None,
                target_artifacts: vec!["document:doc-1".to_string()],
                evidence_summary: Some("document tool text_read completed".to_string()),
                content_accessed: true,
                analysis_complete: false,
            }],
            completed_tool_names: vec!["document_tools__read_document".to_string()],
            ..Default::default()
        };
        let report = build_system_document_analysis_completion_report(
            Some(
                r#"{"status":"completed","summary":"Document Overview\nKey Content\nKey Takeaways","produced_artifacts":[],"accepted_artifacts":["document:doc-1"],"next_steps":[],"content_accessed":true,"analysis_complete":true,"reason_code":"document_analysis_completed"}"#,
            ),
            None,
            Some(&summary),
            &["document_tools__".to_string()],
        );

        assert_eq!(report.status, "completed");
        assert_eq!(report.validation_status.as_deref(), Some("passed"));
        assert_eq!(report.content_accessed, Some(true));
        assert_eq!(report.analysis_complete, Some(true));
    }
}
