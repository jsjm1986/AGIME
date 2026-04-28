use serde::{Deserialize, Serialize};

use crate::conversation::Conversation;
use std::collections::HashMap;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompletionSurfacePolicy {
    #[default]
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

fn fallback_requested_without_terminal_evidence(signal_summary: &CoordinatorSignalSummary) -> bool {
    signal_summary.fallback_requested > 0 && !runtime_completion_evidence_ready(signal_summary)
}

fn hard_validation_failure_present(signal_summary: &CoordinatorSignalSummary) -> bool {
    signal_summary.has_hard_blocking_signals()
        && signal_summary.permission_timed_out == 0
        && signal_summary.fallback_requested == 0
}

fn completion_has_hard_blocking_signals(signal_summary: &CoordinatorSignalSummary) -> bool {
    hard_validation_failure_present(signal_summary)
        || signal_summary.permission_timed_out > 0
        || fallback_requested_without_terminal_evidence(signal_summary)
}

fn conversation_has_hard_blocking_signals(signal_summary: &CoordinatorSignalSummary) -> bool {
    let has_terminal_worker_result = signal_summary.worker_completed > 0
        && signal_summary
            .latest_completion_summary
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());

    signal_summary.permission_timed_out > 0
        || fallback_requested_without_terminal_evidence(signal_summary)
        || (hard_validation_failure_present(signal_summary) && !has_terminal_worker_result)
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
            assistant_text_counts_as_terminal_reply(&text).then_some(text)
        })
}

pub fn assistant_text_counts_as_terminal_reply(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && !summary_indicates_runtime_internal_only(trimmed)
}

pub fn document_analysis_text_counts_as_final(text: &str) -> bool {
    assistant_text_counts_as_terminal_reply(text)
        && !summary_indicates_document_content_unavailable(text)
        && !summary_indicates_future_intent(text)
}

fn summary_indicates_runtime_internal_only(summary: &str) -> bool {
    let normalized = summary.trim().to_ascii_lowercase();

    if normalized.starts_with("tool `")
        && (normalized.ends_with(" completed") || normalized.contains("` failed:"))
    {
        return true;
    }

    if normalized.starts_with("ran into this error:")
        && normalized.contains("please retry if you think this is a transient or recoverable error")
    {
        return true;
    }

    if normalized.starts_with("permission requested for `")
        || normalized.starts_with("permission bridge timed out while waiting for `")
        || (normalized.starts_with("permission `") && normalized.contains(" resolved for `"))
    {
        return true;
    }

    if normalized.starts_with("document access established;")
        || normalized.starts_with("document access established for ")
        || (normalized.contains("document access established")
            && normalized.contains("materialized into the workspace"))
        || (normalized.contains("document access established")
            && normalized.contains("workspace path:")
            && normalized.contains("use developer shell"))
        || (normalized.contains("document access established")
            && normalized.contains("use developer shell, mcp, or another local tool"))
        || normalized == "document content observed; continue the final analysis"
        || normalized == "document analysis is ready for validation"
        || normalized == "document access has not been established yet"
    {
        return true;
    }

    if normalized.starts_with("swarm run `") || normalized.contains("mailbox root:") {
        return true;
    }

    if normalized.starts_with("bounded swarm produced no accepted target delta.")
        || normalized.starts_with("planner auto-upgraded this turn")
    {
        return true;
    }

    false
}

pub fn latest_terminal_user_visible_assistant_text(conversation: &Conversation) -> Option<String> {
    let messages = conversation.messages();
    let last_tool_response_idx = messages.iter().rposition(|message| {
        message.metadata.user_visible
            && message.content.iter().any(|content| {
                matches!(
                    content,
                    crate::conversation::message::MessageContent::ToolResponse(_)
                )
            })
    });

    let Some(start_idx) = last_tool_response_idx.map(|idx| idx + 1) else {
        return latest_user_visible_assistant_text(conversation);
    };
    messages[start_idx..]
        .iter()
        .rev()
        .find(|message| {
            message.role == rmcp::model::Role::Assistant && message.metadata.user_visible
        })
        .and_then(|message| {
            let text = message.as_concat_text().trim().to_string();
            assistant_text_counts_as_terminal_reply(&text).then_some(text)
        })
}

fn conversation_validation_status(
    final_conversation: &Conversation,
    signal_summary: Option<&CoordinatorSignalSummary>,
    has_terminal_assistant_summary: bool,
) -> &'static str {
    let Some(summary) = signal_summary else {
        return "not_run";
    };
    if let Some(status) = summary.validation_status() {
        return status;
    }
    if (summary.document_analysis_complete()
        || (summary.document_access_established()
            && conversation_indicates_document_content_read(final_conversation)))
        && has_terminal_assistant_summary
    {
        return "passed";
    }
    if summary.document_access_established() {
        return "failed";
    }
    "not_run"
}

fn conversation_indicates_document_content_read(conversation: &Conversation) -> bool {
    let mut tool_names = HashMap::new();
    let mut document_access_established = false;

    for message in conversation.messages() {
        for content in &message.content {
            if let Some(request) = content.as_tool_request() {
                if let Ok(tool_call) = &request.tool_call {
                    tool_names.insert(request.id.clone(), tool_call.name.to_string());
                }
                continue;
            }

            let Some(response) = content.as_tool_response() else {
                continue;
            };
            let Ok(result) = &response.tool_result else {
                continue;
            };
            let tool_name = tool_names
                .get(&response.id)
                .map(String::as_str)
                .unwrap_or("");

            if tool_name.starts_with("document_tools__") {
                if result
                    .structured_content
                    .as_ref()
                    .and_then(|value| value.get("content_accessed"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    return true;
                }
                if result
                    .structured_content
                    .as_ref()
                    .and_then(|value| value.get("access_established"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    document_access_established = true;
                }
                continue;
            }

            if !document_access_established || result.is_error.unwrap_or(false) {
                continue;
            }

            let has_text = result.content.iter().any(|item| {
                item.as_text()
                    .map(|text| !text.text.trim().is_empty())
                    .unwrap_or(false)
            });
            if has_text {
                return true;
            }
        }
    }

    false
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
    if [
        "i need to read the document first",
        "i'll start by reading",
        "let me do that now",
        "before i can provide a final output",
        "need to extract the actual content",
        "需要先读取文档",
        "需要从工作区文件中提取实际内容",
        "直接从工作区文件中提取",
        "摘录内容不完整",
        "摘录内容不足",
        "不足以完成分析",
        "让我",
        "让我先",
        "让我用",
        "我将",
        "我将使用",
        "我会",
        "下一步",
    ]
    .iter()
    .any(|pattern| normalized.contains(pattern))
    {
        return true;
    }

    let intent_prefixes = [
        "let me",
        "i need to",
        "i will",
        "i'll",
        "i am going to",
        "i’m going to",
    ];
    let document_actions = [
        "read", "extract", "inspect", "open", "parse", "access", "load",
    ];
    if intent_prefixes
        .iter()
        .any(|prefix| normalized.contains(prefix))
        && document_actions
            .iter()
            .any(|action| normalized.contains(action))
    {
        return true;
    }

    false
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
        && signal_summary.is_some_and(completion_has_hard_blocking_signals)
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

    let assistant_summary = latest_terminal_user_visible_assistant_text(final_conversation);
    let has_hard_blocking_signals =
        signal_summary.is_some_and(conversation_has_hard_blocking_signals);
    let validation_status = conversation_validation_status(
        final_conversation,
        signal_summary,
        assistant_summary.is_some(),
    )
    .to_string();
    if let Some(summary) = assistant_summary.filter(|_| !has_hard_blocking_signals) {
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
            validation_status: Some(validation_status.clone()),
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
        validation_status: Some(validation_status),
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
    report: ExecutionHostCompletionReport,
    signal_summary: Option<&CoordinatorSignalSummary>,
) -> ExecutionHostCompletionReport {
    normalize_document_analysis_completion_report(report, signal_summary, false)
}

pub fn normalize_pre_materialized_document_analysis_completion_report(
    report: ExecutionHostCompletionReport,
    signal_summary: Option<&CoordinatorSignalSummary>,
) -> ExecutionHostCompletionReport {
    normalize_document_analysis_completion_report(report, signal_summary, true)
}

fn normalize_document_analysis_completion_report(
    mut report: ExecutionHostCompletionReport,
    signal_summary: Option<&CoordinatorSignalSummary>,
    document_access_preestablished: bool,
) -> ExecutionHostCompletionReport {
    let document_access_established = document_access_preestablished
        || signal_summary
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

pub fn build_system_document_analysis_completion_report_from_conversation(
    final_output_raw: Option<&str>,
    final_conversation: &Conversation,
    completion_outcome: Option<&ExecuteCompletionOutcome>,
    signal_summary: Option<&CoordinatorSignalSummary>,
    required_tool_prefixes: &[String],
) -> ExecutionHostCompletionReport {
    if parse_execution_host_completion_report(final_output_raw).is_none()
        && !completion_outcome.is_some_and(|outcome| outcome.state.is_terminal())
        && !signal_summary.is_some_and(completion_has_hard_blocking_signals)
    {
        if let Some(summary) = latest_terminal_user_visible_assistant_text(final_conversation)
            .filter(|summary| document_analysis_text_counts_as_final(summary))
        {
            let evidence = signal_summary.map(CoordinatorSignalSummary::completion_evidence);
            return normalize_system_document_analysis_completion_report(
                ExecutionHostCompletionReport {
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
                    validation_status: None,
                    blocking_reason: None,
                    reason_code: Some("document_analysis_completed".to_string()),
                    content_accessed: Some(true),
                    analysis_complete: Some(true),
                },
                signal_summary,
            );
        }
    }

    build_system_document_analysis_completion_report(
        final_output_raw,
        completion_outcome,
        signal_summary,
        required_tool_prefixes,
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
    let has_blocking_signals = completion_has_hard_blocking_signals(signal_summary);

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
    use crate::agents::harness::{
        CoordinatorSignal, StructuredCompletionSignal, TaskKind, ToolTransportKind,
        ValidationReport, ValidationStatus, WorkerOutcome,
    };
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
            recovered_terminal_summary: false,
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
    fn derive_execute_completion_outcome_allows_worker_failures_with_terminal_summary() {
        let summary = CoordinatorSignalSummary {
            worker_completed: 1,
            worker_failed: 1,
            latest_completion_summary: Some("leader summarized partial success".to_string()),
            produced_targets: vec!["docs/out.md".to_string()],
            accepted_targets: vec!["docs/out.md".to_string()],
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
        assert!(!outcome.has_blocking_signals);
    }

    #[test]
    fn derive_execute_completion_outcome_does_not_block_completed_swarm_on_fallback_signal_alone() {
        let summary = CoordinatorSignalSummary {
            worker_completed: 2,
            fallback_requested: 1,
            latest_completion_summary: Some("swarm leader produced final summary".to_string()),
            produced_targets: vec!["README.md".to_string()],
            accepted_targets: vec!["README.md".to_string()],
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
        assert!(!outcome.has_blocking_signals);
    }

    #[test]
    fn derive_execute_completion_outcome_ignores_document_validation_failures_contradicted_by_read_tool(
    ) {
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
                        "The validation worker cannot access the target document content because no document read tools are available."
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
            CoordinatorSignal::CompletionReady {
                report: StructuredCompletionSignal {
                    status: "completed".to_string(),
                    summary: Some("Document verified and summarized.".to_string()),
                    accepted_targets: vec!["document:doc-1".to_string()],
                    produced_targets: vec![],
                    validation_status: Some("failed".to_string()),
                    blocking_reason: Some(
                        "The validation worker cannot access the target document content because no document read tools are available."
                            .to_string(),
                    ),
                    reason_code: None,
                    content_accessed: Some(true),
                    analysis_complete: Some(true),
                },
            },
        ]);

        let outcome = derive_execute_completion_outcome(
            &summary,
            &FinalOutputState {
                tool_present: true,
                collected_output: None,
            },
            &["document_tools__".to_string()],
            false,
        );

        assert_eq!(outcome.state, ExecuteCompletionState::Completed);
        assert_eq!(outcome.status, "completed");
        assert!(!outcome.has_blocking_signals);
        assert_eq!(
            outcome.summary.as_deref(),
            Some("Document verified and summarized.")
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
    fn provider_error_text_does_not_count_as_terminal_completion() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::user().with_text("hello"));
        conversation.push(Message::assistant().with_text(
            "Ran into this error: upstream timeout.\n\nPlease retry if you think this is a transient or recoverable error.",
        ));

        let report = build_conversation_completion_report(&conversation, None, None);

        assert_eq!(report.status, "blocked");
        assert_eq!(
            report.blocking_reason.as_deref(),
            Some("runtime exited without a user-visible assistant response")
        );
    }

    #[test]
    fn conversation_completion_allows_terminal_assistant_summary_despite_worker_failure() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::user().with_text("run swarm"));
        conversation.push(Message::assistant().with_text("worker A completed; worker B failed"));
        let summary = CoordinatorSignalSummary {
            worker_completed: 1,
            worker_failed: 1,
            latest_completion_summary: Some("worker A completed; worker B failed".to_string()),
            ..Default::default()
        };

        let report = build_conversation_completion_report(&conversation, None, Some(&summary));

        assert_eq!(report.status, "completed");
        assert_eq!(report.summary, "worker A completed; worker B failed");
        assert_eq!(report.blocking_reason, None);
    }

    #[test]
    fn conversation_completion_allows_terminal_swarm_summary_despite_internal_validation_failures()
    {
        let mut conversation = Conversation::empty();
        conversation.push(Message::user().with_text("run swarm"));
        conversation.push(
            Message::assistant()
                .with_text("Swarm completed. Key findings:\n- Worker 1: README.md is missing."),
        );
        let summary = CoordinatorSignalSummary {
            worker_completed: 2,
            validation_failed: 2,
            fallback_requested: 1,
            latest_completion_summary: Some(
                "Swarm completed. Key findings:\n- Worker 1: README.md is missing.".to_string(),
            ),
            ..Default::default()
        };

        let report = build_conversation_completion_report(&conversation, None, Some(&summary));

        assert_eq!(report.status, "completed");
        assert_eq!(
            report.summary,
            "Swarm completed. Key findings:\n- Worker 1: README.md is missing."
        );
        assert_eq!(report.blocking_reason, None);
    }

    #[test]
    fn conversation_completion_ignores_assistant_text_before_last_tool_response() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::assistant().with_text("hi there"));
        conversation.push(Message::user().with_tool_response(
            "tool-1",
            Ok(rmcp::model::CallToolResult {
                content: vec![rmcp::model::Content::text("tool completed")],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        ));

        let report = build_conversation_completion_report(&conversation, None, None);

        assert_eq!(report.status, "blocked");
    }

    #[test]
    fn conversation_completion_does_not_fallback_to_runtime_summary_without_terminal_reply() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::assistant().with_text("hi there"));
        conversation.push(Message::user().with_tool_response(
            "tool-1",
            Ok(rmcp::model::CallToolResult {
                content: vec![rmcp::model::Content::text("tool completed")],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        ));
        let outcome = ExecuteCompletionOutcome {
            state: ExecuteCompletionState::Completed,
            status: "completed".to_string(),
            summary: Some("worker finished".to_string()),
            blocking_reason: None,
            completion_ready: true,
            required_tools_satisfied: true,
            has_blocking_signals: false,
            active_child_tasks: false,
        };

        let report = build_conversation_completion_report(&conversation, Some(&outcome), None);

        assert_eq!(report.status, "blocked");
    }

    #[test]
    fn conversation_completion_rejects_tool_completed_echo_as_terminal_reply() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::assistant().with_tool_request(
            "tool-1",
            Ok(rmcp::model::CallToolRequestParams {
                name: "document_tools__document_inventory".into(),
                arguments: None,
                meta: None,
                task: None,
            }),
        ));
        conversation.push(Message::user().with_tool_response(
            "tool-1",
            Ok(rmcp::model::CallToolResult {
                content: vec![rmcp::model::Content::text("inventory json")],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        ));
        conversation.push(
            Message::assistant().with_text("tool `document_tools__document_inventory` completed"),
        );

        let report = build_conversation_completion_report(&conversation, None, None);

        assert_eq!(report.status, "blocked");
        assert_eq!(
            report.blocking_reason.as_deref(),
            Some("runtime exited without a user-visible assistant response")
        );
    }

    #[test]
    fn conversation_completion_rejects_document_progress_echo_as_terminal_reply() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::assistant().with_tool_request(
            "tool-1",
            Ok(rmcp::model::CallToolRequestParams {
                name: "document_tools__document_inventory".into(),
                arguments: None,
                meta: None,
                task: None,
            }),
        ));
        conversation.push(Message::user().with_tool_response(
            "tool-1",
            Ok(rmcp::model::CallToolResult {
                content: vec![rmcp::model::Content::text("document access established")],
                structured_content: Some(serde_json::json!({
                    "access_established": true,
                    "content_accessed": false
                })),
                is_error: Some(false),
                meta: None,
            }),
        ));
        conversation.push(
            Message::assistant()
                .with_text("document access established; continue with local tools"),
        );

        let report = build_conversation_completion_report(&conversation, None, None);

        assert_eq!(report.status, "blocked");
    }

    #[test]
    fn conversation_completion_rejects_document_workspace_handoff_as_terminal_reply() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::assistant().with_tool_request(
            "doc-read",
            Ok(rmcp::model::CallToolRequestParams {
                name: "document_tools__read_document".into(),
                arguments: None,
                meta: None,
                task: None,
            }),
        ));
        conversation.push(Message::user().with_tool_response(
            "doc-read",
            Ok(rmcp::model::CallToolResult {
                content: vec![rmcp::model::Content::text(
                    "Document access established and the file was materialized into the workspace.",
                )],
                structured_content: Some(serde_json::json!({
                    "access_established": true,
                    "content_accessed": false,
                    "analysis_complete": false,
                    "reason_code": "document_access_established"
                })),
                is_error: Some(false),
                meta: None,
            }),
        ));
        conversation.push(Message::assistant().with_text(
            "Document access established and the file was materialized into the workspace. Use developer shell, MCP, or another local tool to read the file content from the workspace path.",
        ));

        let report = build_conversation_completion_report(&conversation, None, None);

        assert_eq!(report.status, "blocked");
        assert_eq!(
            report.blocking_reason.as_deref(),
            Some("runtime exited without a user-visible assistant response")
        );
    }

    #[test]
    fn conversation_completion_marks_document_chat_as_passed_when_analysis_completes() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::assistant().with_text("final analysis"));
        let summary = CoordinatorSignalSummary {
            document_phase: Some(
                super::super::signals::DocumentAnalysisPhase::AnalysisReadyForValidation,
            ),
            ..Default::default()
        };

        let report = build_conversation_completion_report(&conversation, None, Some(&summary));

        assert_eq!(report.status, "completed");
        assert_eq!(report.validation_status.as_deref(), Some("passed"));
    }

    #[test]
    fn conversation_completion_marks_document_chat_as_passed_after_access_and_local_read() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::assistant().with_tool_request(
            "doc-read",
            Ok(rmcp::model::CallToolRequestParams {
                name: "document_tools__read_document".into(),
                arguments: None,
                meta: None,
                task: None,
            }),
        ));
        conversation.push(Message::user().with_tool_response(
            "doc-read",
            Ok(rmcp::model::CallToolResult {
                content: vec![rmcp::model::Content::text("document access established")],
                structured_content: Some(serde_json::json!({
                    "access_established": true,
                    "content_accessed": false
                })),
                is_error: Some(false),
                meta: None,
            }),
        ));
        conversation.push(Message::assistant().with_tool_request(
            "editor-view",
            Ok(rmcp::model::CallToolRequestParams {
                name: "developer__text_editor".into(),
                arguments: None,
                meta: None,
                task: None,
            }),
        ));
        conversation.push(Message::user().with_tool_response(
            "editor-view",
            Ok(rmcp::model::CallToolResult {
                content: vec![rmcp::model::Content::text("actual document body")],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        ));
        conversation.push(Message::assistant().with_text("final analysis"));
        let summary = CoordinatorSignalSummary {
            document_phase: Some(super::super::signals::DocumentAnalysisPhase::AccessEstablished),
            ..Default::default()
        };

        let report = build_conversation_completion_report(&conversation, None, Some(&summary));

        assert_eq!(report.status, "completed");
        assert_eq!(report.validation_status.as_deref(), Some("passed"));
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
    fn pre_materialized_document_analysis_does_not_require_document_tool_signal() {
        let report = normalize_pre_materialized_document_analysis_completion_report(
            ExecutionHostCompletionReport {
                status: "completed".to_string(),
                summary: "Document Overview\nKey Content\nKey Takeaways".to_string(),
                produced_artifacts: Vec::new(),
                accepted_artifacts: vec!["document:doc-1".to_string()],
                next_steps: Vec::new(),
                validation_status: None,
                blocking_reason: None,
                reason_code: None,
                content_accessed: Some(true),
                analysis_complete: Some(true),
            },
            None,
        );

        assert_eq!(report.status, "completed");
        assert_eq!(report.validation_status.as_deref(), Some("passed"));
        assert_eq!(
            report.reason_code.as_deref(),
            Some("document_analysis_completed")
        );
    }

    #[test]
    fn system_document_analysis_can_complete_from_terminal_assistant_text() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::user().with_text("analyze the document"));
        conversation.push(Message::assistant().with_text("Document Overview\nKey Takeaways"));

        let report = build_system_document_analysis_completion_report_from_conversation(
            None,
            &conversation,
            None,
            Some(&CoordinatorSignalSummary {
                document_phase: Some(
                    super::super::signals::DocumentAnalysisPhase::AccessEstablished,
                ),
                ..Default::default()
            }),
            &[],
        );

        assert_eq!(report.status, "completed");
        assert_eq!(report.content_accessed, Some(true));
        assert_eq!(report.analysis_complete, Some(true));
    }

    #[test]
    fn document_analysis_future_intent_text_is_not_final() {
        assert!(!document_analysis_text_counts_as_final(
            "摘录内容不完整，需要从工作区文件中提取实际内容。让我用 Python 来读取这个 PPTX 文件。"
        ));
        assert!(!document_analysis_text_counts_as_final(
            "摘录内容不足以完成分析。我将直接从工作区文件中提取 PPTX 内容。"
        ));
        assert!(!document_analysis_text_counts_as_final(
            "让我直接从 PPTX 文件中提取内容。"
        ));
        assert!(!document_analysis_text_counts_as_final(
            "The text excerpt is extremely minimal. Given the file is only 772 bytes and the excerpt is essentially just a placeholder, let me extract the actual content from the PPTX file."
        ));
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
