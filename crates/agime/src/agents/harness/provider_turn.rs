use anyhow::Result;
use futures::TryStreamExt;
use rmcp::model::{CallToolResult, Content, ErrorCode, ErrorData, Tool};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::agents::agent::{
    Agent, AgentEvent, BackendToolExecutionResult, DeferredToolHandling, FrontendTransportHandling,
    HistoryCapturePolicy, ModelResponseHandling, ProviderErrorHandling, ProviderSuccessHandling,
    RecoveryCompactionHandling, ToolCategorizeResult, ToolTransportRequestEvent,
};
use crate::agents::extension_manager_extension::MANAGE_EXTENSIONS_TOOL_NAME_COMPLETE;
use crate::agents::final_output_tool::FINAL_OUTPUT_TOOL_NAME;
use crate::agents::subagent_task_config::TaskConfig;
use crate::agents::subagent_tool::SUBAGENT_TOOL_NAME;
use crate::agents::swarm_tool::SWARM_TOOL_NAME;
use crate::agents::tool_execution::CHAT_MODE_TOOL_SKIPPED_RESPONSE;
use crate::agents::types::SessionConfig;
use crate::config::AgimeMode;
use crate::conversation::{
    message::{Message, MessageContent, MessageMetadata, SystemNotificationType, ToolRequest},
    Conversation,
};
use crate::permission::permission_judge::PermissionCheckResult;
use crate::providers::{base::ProviderUsage, errors::ProviderError};
use crate::session::Session;
use crate::utils::{normalize_delegation_summary_text, safe_truncate};

use super::coordinator_runtime::validate_single_worker_execute_summary;
use super::delegation::DelegationBootstrapPlan;
use super::signals::StructuredCompletionSignal;
use super::{
    build_auto_swarm_tool_request, build_scheduled_tool_calls, detect_explicit_delegation_intent,
    execute_recovery_compaction, maybe_build_explicit_delegation_bootstrap_request,
    maybe_plan_swarm_upgrade, parse_execution_host_completion_report, partition_tool_calls,
    record_transition, CoordinatorExecutionMode, CoordinatorSignal, DelegationMode,
    DelegationRuntimeState, HarnessCheckpointStore, HarnessContext, HarnessMode, HarnessPolicy,
    RecoveryCompactionExecution, SessionHarnessStore, ToolResponsePlan, ToolTransportKind,
    TransitionKind, TransportDispatcher,
};

impl Agent {
    fn has_explicit_delegation_tool_request(
        frontend_requests: &[ToolRequest],
        remaining_requests: &[ToolRequest],
    ) -> bool {
        frontend_requests
            .iter()
            .chain(remaining_requests.iter())
            .filter_map(|request| request.tool_call.as_ref().ok())
            .any(|tool_call| {
                tool_call.name == SUBAGENT_TOOL_NAME || tool_call.name == SWARM_TOOL_NAME
            })
    }

    fn semantic_tool_error_text(result: &CallToolResult) -> Option<String> {
        if !result.is_error.unwrap_or(false) {
            return None;
        }
        result.content.iter().find_map(|content| {
            content.as_text().and_then(|text| {
                let trimmed = text.text.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            })
        })
    }

    fn tool_result_text_blocks(
        tool_response: Option<&crate::conversation::message::ToolResponse>,
    ) -> Vec<String> {
        tool_response
            .and_then(|response| response.tool_result.as_ref().ok())
            .map(|result| {
                result
                    .content
                    .iter()
                    .filter_map(|content| {
                        content.as_text().and_then(|text| {
                            let trimmed = text.text.trim();
                            (!trimmed.is_empty()).then(|| trimmed.to_string())
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn document_tool_evidence_summary(
        structured: &serde_json::Map<String, serde_json::Value>,
    ) -> Option<String> {
        let kind = structured
            .get("type")
            .and_then(serde_json::Value::as_str)?;
        let subject = structured
            .get("doc_name")
            .or_else(|| structured.get("source_name"))
            .or_else(|| structured.get("doc_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("document");
        let label = match kind {
            "document_access" => "Document access established",
            "document_access_redirect" => "Document access already established",
            "workspace_export" | "binary_export" => "Document exported to workspace",
            "text_read" => "Document text read",
            other => return Some(format!("Document tool {} completed for {}", other, subject)),
        };
        Some(format!("{} for {}", label, subject))
    }

    fn summarize_target_list(targets: &[String]) -> String {
        let normalized = targets
            .iter()
            .map(|target| target.trim())
            .filter(|target| !target.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        match normalized.as_slice() {
            [] => "the requested scope".to_string(),
            [single] => single.clone(),
            [left, right] => format!("{} and {}", left, right),
            many => format!(
                "{}, and {}",
                many[..many.len() - 1].join(", "),
                many[many.len() - 1]
            ),
        }
    }

    fn compact_swarm_finding_text(raw: &str) -> Option<String> {
        let normalized = normalize_delegation_summary_text(raw);
        if normalized.is_empty() {
            return None;
        }

        if let Some((target, detail)) = normalized
            .strip_prefix("bounded swarm worker for ")
            .and_then(|rest| rest.split_once(':'))
        {
            let target = target.trim();
            let detail = detail.trim();
            if detail.contains("Contents: Files:") {
                let files = detail
                    .split("Contents: Files:")
                    .nth(1)
                    .unwrap_or_default()
                    .split_whitespace()
                    .take(4)
                    .collect::<Vec<_>>();
                if !files.is_empty() {
                    return Some(format!("{}: contains {}", target, files.join(", ")));
                }
            }
            if detail.starts_with('#')
                || detail.starts_with("/repo/")
                || detail.starts_with("/opt/")
                || detail.contains("```")
                || detail.contains("### ")
                || detail.contains("| ")
                || detail.len() > 180
            {
                return Some(format!("{}: inspected successfully", target));
            }
            return Some(format!(
                "{}: {}",
                target,
                safe_truncate(detail.trim_matches('`'), 140)
            ));
        }

        if normalized.starts_with("/repo/") || normalized.starts_with("/opt/") {
            return None;
        }
        if normalized.contains("```") || normalized.contains("### ") || normalized.contains("| ") {
            return None;
        }
        Some(safe_truncate(&normalized, 140))
    }

    fn build_swarm_completion_summary(
        tool_response: Option<&crate::conversation::message::ToolResponse>,
        produced_targets: &[String],
    ) -> String {
        let summary_line = format!(
            "Swarm completed for {}. Key findings:",
            Self::summarize_target_list(produced_targets)
        );

        let mut findings = Vec::new();
        for block in Self::tool_result_text_blocks(tool_response) {
            for line in block.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty()
                    || trimmed.starts_with("Swarm run `")
                    || trimmed.starts_with("Scratchpad:")
                    || trimmed.starts_with("Mailbox root:")
                    || trimmed.starts_with("Worker summaries:")
                    || trimmed.starts_with("Validation summaries:")
                {
                    continue;
                }

                let Some(normalized) =
                    Self::compact_swarm_finding_text(trimmed.trim_start_matches("- ").trim())
                else {
                    continue;
                };
                if !findings.iter().any(|item| item == &normalized) {
                    findings.push(normalized);
                }
                if findings.len() >= 2 {
                    break;
                }
            }
            if findings.len() >= 2 {
                break;
            }
        }

        if findings.is_empty() {
            return normalize_delegation_summary_text(&summary_line);
        }
        let summary = findings
            .into_iter()
            .take(2)
            .fold(summary_line, |mut acc, finding| {
                acc.push_str("\n- ");
                acc.push_str(&finding);
                acc
            });
        normalize_delegation_summary_text(&summary)
    }

    fn apply_explicit_delegation_bootstrap_override(
        frontend_requests: &mut Vec<ToolRequest>,
        remaining_requests: &mut Vec<ToolRequest>,
        conversation: &Conversation,
        response: &Message,
        delegation_state: &mut DelegationRuntimeState,
    ) -> Option<DelegationBootstrapPlan> {
        if Self::has_explicit_delegation_tool_request(frontend_requests, remaining_requests) {
            return None;
        }
        match detect_explicit_delegation_intent(conversation, delegation_state) {
            Some(super::delegation::ExplicitDelegationIntent::Swarm)
                if delegation_state.swarm_calls_this_run > 0 =>
            {
                return None;
            }
            Some(super::delegation::ExplicitDelegationIntent::Subagent)
                if delegation_state.subagent_calls_this_turn > 0 =>
            {
                return None;
            }
            _ => {}
        }
        let bootstrap = maybe_build_explicit_delegation_bootstrap_request(
            conversation,
            response,
            delegation_state,
        )?;
        frontend_requests.clear();
        remaining_requests.clear();
        remaining_requests.push(bootstrap.request.clone());
        Some(bootstrap)
    }

    fn coordinator_signal_for_tool_response(
        request_id: &str,
        tool_name: String,
        transport: ToolTransportKind,
        tool_response: Option<&crate::conversation::message::ToolResponse>,
    ) -> CoordinatorSignal {
        if let Some(error) = tool_response
            .and_then(|result| result.tool_result.as_ref().ok())
            .and_then(Self::semantic_tool_error_text)
        {
            return CoordinatorSignal::ToolFailed {
                request_id: request_id.to_string(),
                tool_name,
                transport,
                error,
            };
        }
        match tool_response.and_then(|result| result.tool_result.as_ref().err()) {
            Some(error) => CoordinatorSignal::ToolFailed {
                request_id: request_id.to_string(),
                tool_name,
                transport,
                error: error.to_string(),
            },
            None => CoordinatorSignal::ToolCompleted {
                request_id: request_id.to_string(),
                tool_name,
                transport,
            },
        }
    }

    async fn record_structured_tool_signals(
        request: &ToolRequest,
        response_message: &Message,
        coordinator_signals: &super::SharedCoordinatorSignalStore,
    ) {
        let Ok(tool_call) = request.tool_call.as_ref() else {
            return;
        };
        let tool_response = response_message
            .content
            .iter()
            .find_map(|content| content.as_tool_response());
        let structured = tool_response
            .and_then(|tool_response| tool_response.tool_result.as_ref().ok())
            .and_then(|tool_result| tool_result.structured_content.as_ref())
            .and_then(serde_json::Value::as_object);

        let Some(structured) = structured else {
            if tool_call.name == FINAL_OUTPUT_TOOL_NAME {
                if let Some(arguments) = tool_call.arguments.as_ref() {
                    if let Ok(raw) = serde_json::to_string(arguments) {
                        if let Some(report) = parse_execution_host_completion_report(Some(&raw)) {
                            coordinator_signals
                                .record(CoordinatorSignal::CompletionReady {
                                    report: StructuredCompletionSignal {
                                        status: report.status,
                                        summary: Some(report.summary),
                                        accepted_targets: report.accepted_artifacts,
                                        produced_targets: report.produced_artifacts,
                                        validation_status: report.validation_status,
                                        blocking_reason: report.blocking_reason,
                                        reason_code: report.reason_code,
                                        content_accessed: report.content_accessed,
                                        analysis_complete: report.analysis_complete,
                                    },
                                })
                                .await;
                        }
                    }
                }
            }
            return;
        };
        if tool_call.name.starts_with("document_tools__") {
            let content_accessed = structured
                .get("content_accessed")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let analysis_complete = structured
                .get("analysis_complete")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let reason_code = structured
                .get("reason_code")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string);
            let target_artifacts = structured
                .get("doc_id")
                .and_then(serde_json::Value::as_str)
                .map(|doc_id| vec![format!("document:{}", doc_id)])
                .unwrap_or_default();
            let doc_id = structured
                .get("doc_id")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string);
            let workspace_path = structured
                .get("file_path")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string);
            let next_action = structured
                .get("next_action")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string);
            let phase = if analysis_complete {
                Some(super::signals::DocumentAnalysisPhase::AnalysisReadyForValidation)
            } else if content_accessed {
                Some(super::signals::DocumentAnalysisPhase::ContentRead)
            } else if structured
                .get("access_established")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                Some(super::signals::DocumentAnalysisPhase::AccessEstablished)
            } else {
                None
            };
            let evidence_summary = Self::document_tool_evidence_summary(structured);
            coordinator_signals
                .record(CoordinatorSignal::ValidationReported {
                    report: super::ValidationReport {
                        status: super::ValidationStatus::NotRun,
                        reason: None,
                        reason_code: reason_code.clone(),
                        validator_task_id: None,
                        target_artifacts,
                        evidence_summary,
                        content_accessed,
                        analysis_complete,
                    },
                })
                .await;
            if let Some(phase) = phase {
                coordinator_signals
                    .record(CoordinatorSignal::DocumentPhaseUpdated {
                        phase,
                        workspace_path,
                        doc_id,
                        reason_code: reason_code.clone(),
                        next_action,
                    })
                    .await;
            }
            return;
        }
        if tool_call.name != SWARM_TOOL_NAME {
            return;
        }
        let accepted_targets = structured
            .get("accepted_targets")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let produced_targets = structured
            .get("produced_targets")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let validation_status = structured
            .get("validation_summary_count")
            .and_then(serde_json::Value::as_u64)
            .map(|count| {
                if count > 0 {
                    "passed".to_string()
                } else {
                    "not_run".to_string()
                }
            });
        let completion_targets = if accepted_targets.is_empty() {
            produced_targets.clone()
        } else {
            accepted_targets.clone()
        };
        if !completion_targets.is_empty() {
            let summary = Self::build_swarm_completion_summary(tool_response, &completion_targets);
            coordinator_signals
                .record(CoordinatorSignal::CompletionReady {
                    report: StructuredCompletionSignal {
                        status: "completed".to_string(),
                        summary: Some(summary),
                        accepted_targets: accepted_targets.clone(),
                        produced_targets: produced_targets.clone(),
                        validation_status,
                        blocking_reason: None,
                        reason_code: None,
                        content_accessed: None,
                        analysis_complete: None,
                    },
                })
                .await;
        }
        let downgraded = structured
            .get("downgraded")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if !downgraded {
            return;
        }

        let reason = structured
            .get("downgrade_message")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("swarm execution requested fallback")
            .to_string();
        coordinator_signals
            .record(CoordinatorSignal::FallbackRequested { reason })
            .await;
    }

    fn ensure_resolved_tool_response_message(
        request: &ToolRequest,
        response_message: Message,
    ) -> Message {
        if !response_message.content.is_empty() {
            return response_message;
        }

        Message::user()
            .with_id(
                response_message
                    .id
                    .unwrap_or_else(|| format!("msg_{}", uuid::Uuid::new_v4())),
            )
            .with_tool_response(
                request.id.clone(),
                Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "Tool execution did not produce a transcripted response before the turn ended.",
                    None,
                )),
            )
    }

    pub(crate) fn extract_persistable_assistant_text_message(message: &Message) -> Option<Message> {
        if message.role != rmcp::model::Role::Assistant || !message.metadata.user_visible {
            return None;
        }

        let content = message
            .content
            .iter()
            .filter_map(|content| match content {
                MessageContent::Text(text) if !text.text.trim().is_empty() => {
                    Some(MessageContent::Text(text.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        if content.is_empty() {
            return None;
        }

        let mut persisted = Message::new(message.role.clone(), message.created, content);
        persisted.metadata = message.metadata.clone();
        persisted.id = message.id.clone();
        Some(persisted)
    }

    async fn record_execute_text_completion_if_ready(
        current_mode: HarnessMode,
        message: Option<&Message>,
        coordinator_signals: &super::SharedCoordinatorSignalStore,
    ) {
        if !matches!(current_mode, HarnessMode::Execute) {
            return;
        }

        let Some(message) = message else {
            return;
        };

        let summary = message.as_concat_text().trim().to_string();
        if summary.is_empty() {
            return;
        }

        coordinator_signals
            .record(CoordinatorSignal::CompletionReady {
                report: StructuredCompletionSignal {
                    status: "completed".to_string(),
                    summary: Some(summary),
                    accepted_targets: Vec::new(),
                    produced_targets: Vec::new(),
                    validation_status: None,
                    blocking_reason: None,
                    reason_code: None,
                    content_accessed: None,
                    analysis_complete: None,
                },
            })
            .await;
    }

    async fn record_single_worker_validation_completion_if_ready(
        &self,
        current_mode: HarnessMode,
        message: Option<&Message>,
        harness_context: &HarnessContext,
        session_id: &str,
        coordinator_signals: &super::SharedCoordinatorSignalStore,
    ) -> Result<bool> {
        if !matches!(current_mode, HarnessMode::Execute)
            || harness_context.coordinator_execution_mode != CoordinatorExecutionMode::SingleWorker
            || !harness_context.validation_mode
        {
            return Ok(false);
        }

        let Some(message) = message else {
            return Ok(false);
        };

        let summary = message.as_concat_text().trim().to_string();
        if summary.is_empty() {
            return Ok(false);
        }

        let extension_configs = self.get_extension_configs().await;
        let provider = self.provider().await?;
        let task_config = TaskConfig::new(
            provider,
            session_id,
            &harness_context.working_dir,
            extension_configs,
        )
        .with_summary_only(true)
        .with_write_scope(harness_context.write_scope.clone())
        .with_runtime_contract(
            harness_context.target_artifacts.clone(),
            harness_context.result_contract.clone(),
        )
        .with_delegation_mode(DelegationMode::Disabled);
        let target_artifact = harness_context
            .target_artifacts
            .first()
            .cloned()
            .or_else(|| harness_context.result_contract.first().cloned())
            .unwrap_or_else(|| "execute:single_worker".to_string());
        let report = validate_single_worker_execute_summary(
            &task_config,
            &harness_context.working_dir,
            target_artifact,
            harness_context.result_contract.clone(),
            &summary,
            harness_context.cancel_token.clone(),
        )
        .await?;

        coordinator_signals
            .record(CoordinatorSignal::ValidationReported {
                report: report.clone(),
            })
            .await;

        if report.passed() {
            coordinator_signals
                .record(CoordinatorSignal::CompletionReady {
                    report: StructuredCompletionSignal {
                        status: "completed".to_string(),
                        summary: Some(summary),
                        accepted_targets: harness_context.target_artifacts.clone(),
                        produced_targets: Vec::new(),
                        validation_status: Some(report.status.as_str().to_string()),
                        blocking_reason: None,
                        reason_code: report.reason_code.clone(),
                        content_accessed: Some(report.content_accessed),
                        analysis_complete: Some(report.analysis_complete),
                    },
                })
                .await;
        }

        Ok(true)
    }

    pub(crate) fn capture_event_message(
        messages_to_add: &mut Conversation,
        event: &AgentEvent,
        policy: HistoryCapturePolicy,
    ) {
        let AgentEvent::Message(message) = event else {
            return;
        };

        match policy {
            HistoryCapturePolicy::SystemNotificationsOnly => {
                if matches!(
                    message.content.last(),
                    Some(MessageContent::SystemNotification(_))
                ) {
                    messages_to_add.push(message.clone());
                }
            }
            HistoryCapturePolicy::AllMessages => {
                messages_to_add.push(message.clone());
            }
        }
    }

    pub(crate) async fn process_frontend_tool_requests(
        &self,
        frontend_requests: &[ToolRequest],
        tools: &[Tool],
        harness_policy: &HarnessPolicy,
        coordinator_execution_mode: CoordinatorExecutionMode,
        delegation_state: &DelegationRuntimeState,
        tool_response_plan: &ToolResponsePlan,
        server_local_tool_names: &[String],
        surface: super::ToolInvocationSurface,
    ) -> Result<FrontendTransportHandling> {
        let scheduled_frontend_calls =
            build_scheduled_tool_calls(frontend_requests, tools, |_| true);
        let allowed_frontend_calls: Vec<crate::agents::harness::tools::ScheduledToolCall> = self
            .apply_runtime_policy_to_calls(
                scheduled_frontend_calls,
                harness_policy,
                coordinator_execution_mode,
                delegation_state,
                tool_response_plan.request_to_response_map(),
            )
            .await;
        let dispatch_plan =
            TransportDispatcher::dispatch(allowed_frontend_calls, surface, server_local_tool_names);

        let mut events = Vec::new();
        let pending_request_ids = dispatch_plan.pending_request_ids();
        let request_transports = dispatch_plan
            .dispatches
            .iter()
            .map(|dispatch| (dispatch.call.request.id.clone(), dispatch.transport))
            .collect::<std::collections::HashMap<_, _>>();
        for dispatch in dispatch_plan.dispatches {
            events.push(AgentEvent::ToolTransportRequest(
                ToolTransportRequestEvent {
                    request: dispatch.call.request.clone(),
                    transport: dispatch.transport,
                    surface: dispatch.surface,
                },
            ));
        }

        Ok(FrontendTransportHandling {
            events,
            pending_request_ids,
            request_transports,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn process_backend_tool_requests(
        &self,
        remaining_requests: &[ToolRequest],
        tools: &[Tool],
        conversation: &Conversation,
        harness_policy: &HarnessPolicy,
        delegation_state: &mut DelegationRuntimeState,
        harness_context: &HarnessContext,
        tool_response_plan: &ToolResponsePlan,
        cancel_token: Option<CancellationToken>,
        session: &Session,
        session_id: &str,
        turns_taken: u32,
        current_mode: HarnessMode,
        transcript_store: &SessionHarnessStore,
    ) -> Result<BackendToolExecutionResult> {
        let inspection_results = self
            .tool_inspection_manager
            .inspect_tools(remaining_requests, conversation.messages())
            .await?;

        let permission_check_result = self
            .tool_inspection_manager
            .process_inspection_results_with_permission_inspector(
                remaining_requests,
                &inspection_results,
            )
            .unwrap_or_else(|| {
                let mut result = PermissionCheckResult {
                    approved: vec![],
                    needs_approval: vec![],
                    denied: vec![],
                };
                result
                    .needs_approval
                    .extend(remaining_requests.iter().cloned());
                result
            });

        let mut enable_extension_request_ids = vec![];
        for request in remaining_requests {
            if let Ok(tool_call) = &request.tool_call {
                if tool_call.name == MANAGE_EXTENSIONS_TOOL_NAME_COMPLETE {
                    enable_extension_request_ids.push(request.id.clone());
                }
            }
        }

        Self::handle_denied_tools(
            &permission_check_result,
            tool_response_plan.request_to_response_map(),
        )
        .await;

        let approved_requests =
            std::sync::Arc::new(Mutex::new(permission_check_result.approved.clone()));

        let mut tool_approval_stream = self.handle_approval_tool_requests(
            &permission_check_result.needs_approval,
            approved_requests.clone(),
            tool_response_plan.request_to_response_map(),
            cancel_token.clone(),
            &inspection_results,
        );

        let mut approval_messages = Vec::new();
        while let Some(msg) = tool_approval_stream.try_next().await? {
            approval_messages.push(msg);
        }

        let approved_requests = {
            let approved_requests = approved_requests.lock().await;
            approved_requests.clone()
        };

        let scheduled_calls = build_scheduled_tool_calls(&approved_requests, tools, |_| false);
        let filtered_calls = self
            .apply_runtime_policy_to_calls(
                scheduled_calls,
                harness_policy,
                harness_context.coordinator_execution_mode,
                delegation_state,
                tool_response_plan.request_to_response_map(),
            )
            .await;

        let (tool_batches, batch_result_meta) = partition_tool_calls(filtered_calls);
        let _ = transcript_store
            .record_checkpoint(
                session_id,
                crate::agents::harness::HarnessCheckpoint::tool_batch(
                    turns_taken,
                    current_mode,
                    batch_result_meta.batch_count,
                    batch_result_meta.concurrent_batch_count,
                    batch_result_meta.serial_batch_count,
                ),
            )
            .await;

        let batch_result = self
            .execute_scheduled_tool_batches(
                tool_batches,
                tool_response_plan.request_to_response_map(),
                &enable_extension_request_ids,
                cancel_token,
                session,
                session_id,
                !enable_extension_request_ids.is_empty(),
                turns_taken,
                current_mode,
                transcript_store,
                delegation_state,
                &harness_context.task_runtime,
                &harness_context.tool_result_budget,
                &harness_context.transition_trace,
            )
            .await?;

        Ok(BackendToolExecutionResult {
            approval_messages,
            batch_events: batch_result.events,
            tools_updated: batch_result.all_install_successful
                && !enable_extension_request_ids.is_empty(),
            executed_tool_calls: batch_result.executed_tool_calls,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn process_model_response(
        &self,
        response: &Message,
        tools: &[Tool],
        conversation: &Conversation,
        messages_to_add: &mut Conversation,
        harness_policy: &HarnessPolicy,
        delegation_state: &mut DelegationRuntimeState,
        harness_context: &HarnessContext,
        agime_mode: AgimeMode,
        cancel_token: Option<CancellationToken>,
        session: &Session,
        session_id: &str,
        turns_taken: u32,
        current_mode: HarnessMode,
        transcript_store: &SessionHarnessStore,
        auto_swarm_injected_this_reply: &mut bool,
    ) -> Result<ModelResponseHandling> {
        let ToolCategorizeResult {
            frontend_requests,
            remaining_requests,
            filtered_response,
        } = self.categorize_tools(response, tools).await;
        let mut frontend_requests = frontend_requests;
        let mut remaining_requests = remaining_requests;
        let mut auto_planned_swarm = None;
        let mut delegation_bootstrap_notice = None;
        let mut delegation_bootstrap_reason = None;
        let has_explicit_delegation_intent =
            detect_explicit_delegation_intent(conversation, delegation_state).is_some();
        if !*auto_swarm_injected_this_reply
            && !has_explicit_delegation_intent
            && agime_mode != AgimeMode::Chat
            && frontend_requests.is_empty()
            && remaining_requests.is_empty()
        {
            let auto_decision = maybe_plan_swarm_upgrade(
                current_mode,
                harness_context.coordinator_execution_mode,
                conversation,
                delegation_state,
            );
            if let Some(auto_request) =
                build_auto_swarm_tool_request(conversation, response, &auto_decision)
            {
                *auto_swarm_injected_this_reply = true;
                if let Some(plan) = auto_decision.plan.as_ref() {
                    tracing::info!(
                        turn = turns_taken,
                        mode = ?current_mode,
                        target_count = plan.targets.len(),
                        result_contract_count = plan.result_contract.len(),
                        parallelism_budget = plan.budget.parallelism_budget.unwrap_or_default(),
                        "Planner auto-upgraded this turn into a bounded swarm request"
                    );
                }
                auto_planned_swarm = auto_decision.plan.clone();
                remaining_requests.push(auto_request);
            }
        }
        if let Some(bootstrap) = Self::apply_explicit_delegation_bootstrap_override(
            &mut frontend_requests,
            &mut remaining_requests,
            conversation,
            response,
            delegation_state,
        ) {
            delegation_bootstrap_notice = bootstrap.inline_notice.clone();
            delegation_bootstrap_reason = Some(bootstrap.transition_reason.clone());
        }

        let requests_to_record: Vec<ToolRequest> = frontend_requests
            .iter()
            .chain(remaining_requests.iter())
            .cloned()
            .collect();
        if requests_to_record.is_empty() {
            tracing::debug!("process_provider_success_result: recording tool requests");
        } else {
            tracing::info!(
                request_count = requests_to_record.len(),
                "process_provider_success_result: recording tool requests"
            );
        }
        self.tool_route_manager
            .record_tool_requests(&requests_to_record)
            .await;
        if requests_to_record.is_empty() {
            tracing::debug!("process_provider_success_result: tool request recording complete");
        } else {
            tracing::info!("process_provider_success_result: tool request recording complete");
        }

        let mut events = vec![AgentEvent::Message(filtered_response.clone())];
        if let Some(notice) = delegation_bootstrap_notice.clone() {
            events.push(AgentEvent::Message(
                Message::assistant()
                    .with_system_notification(SystemNotificationType::InlineMessage, notice),
            ));
        }
        let persistable_assistant_message =
            Self::extract_persistable_assistant_text_message(&filtered_response);
        let num_tool_requests = frontend_requests.len() + remaining_requests.len();
        if num_tool_requests == 0 {
            tracing::debug!(
                num_tool_requests,
                has_persistable_assistant_message = persistable_assistant_message.is_some(),
                "process_provider_success_result: response categorized"
            );
        } else {
            tracing::info!(
                num_tool_requests,
                has_persistable_assistant_message = persistable_assistant_message.is_some(),
                "process_provider_success_result: response categorized"
            );
        }
        if let Some(reason) = delegation_bootstrap_reason.as_deref() {
            let mut metadata = std::collections::BTreeMap::new();
            metadata.insert(
                "mode".to_string(),
                format!("{:?}", delegation_state.mode).to_ascii_lowercase(),
            );
            metadata.insert("request_count".to_string(), num_tool_requests.to_string());
            record_transition(
                &harness_context.transition_trace,
                turns_taken,
                current_mode,
                TransitionKind::ProviderTurnRecovery,
                reason,
                metadata,
            )
            .await;
        }
        if num_tool_requests == 0 {
            if let Some(message) = persistable_assistant_message.as_ref() {
                if !self
                    .record_single_worker_validation_completion_if_ready(
                        current_mode,
                        Some(message),
                        harness_context,
                        session_id,
                        &harness_context.coordinator_signals,
                    )
                    .await?
                {
                    Self::record_execute_text_completion_if_ready(
                        current_mode,
                        Some(message),
                        &harness_context.coordinator_signals,
                    )
                    .await;
                }
                messages_to_add.push(message.clone());
            } else {
                messages_to_add.push(response.clone());
            }
            tracing::debug!(
                message_count = messages_to_add.len(),
                "process_provider_success_result: returning no-tool handling"
            );
            return Ok(ModelResponseHandling {
                events,
                no_tools_called: true,
                tools_updated: false,
                yield_after_first_event: true,
                deferred_tool_handling: None,
            });
        }

        if let Some(plan) = auto_planned_swarm.as_ref() {
            let mut metadata = std::collections::BTreeMap::new();
            metadata.insert("targets".to_string(), plan.targets.join(","));
            metadata.insert(
                "parallelism_budget".to_string(),
                plan.budget
                    .parallelism_budget
                    .unwrap_or_default()
                    .to_string(),
            );
            metadata.insert(
                "validation_mode".to_string(),
                plan.validation_mode.to_string(),
            );
            record_transition(
                &harness_context.transition_trace,
                turns_taken,
                current_mode,
                TransitionKind::ProviderTurnRecovery,
                "planner_auto_upgrade",
                metadata,
            )
            .await;
            events.push(AgentEvent::Message(
                Message::assistant().with_system_notification(
                    SystemNotificationType::InlineMessage,
                    format!(
                        "Planner auto-upgraded this turn to bounded swarm for targets: {}",
                        plan.targets.join(", ")
                    ),
                ),
            ));
        }

        if let Some(message) = persistable_assistant_message {
            messages_to_add.push(message);
        }

        let tool_response_plan = ToolResponsePlan::new(&frontend_requests, &remaining_requests);
        let frontend_transport = self
            .process_frontend_tool_requests(
                &frontend_requests,
                tools,
                harness_policy,
                harness_context.coordinator_execution_mode,
                delegation_state,
                &tool_response_plan,
                &harness_context.server_local_tool_names,
                super::tool_invocation_surface(
                    harness_policy.mode,
                    matches!(session.session_type, crate::session::SessionType::SubAgent),
                ),
            )
            .await?;
        let pending_frontend_request_ids = frontend_transport.pending_request_ids.clone();
        let frontend_request_transports = frontend_transport.request_transports.clone();
        events.extend(frontend_transport.events);

        let mut no_tools_called = true;
        let mut tools_updated = false;

        if agime_mode == AgimeMode::Chat {
            for request in &remaining_requests {
                if let Some(response_msg) = tool_response_plan
                    .request_to_response_map()
                    .get(&request.id)
                {
                    let mut response = response_msg.lock().await;
                    *response = response.clone().with_tool_response(
                        request.id.clone(),
                        Ok(CallToolResult {
                            content: vec![Content::text(CHAT_MODE_TOOL_SKIPPED_RESPONSE)],
                            structured_content: None,
                            is_error: Some(false),
                            meta: None,
                        }),
                    );
                }
            }
        } else {
            let backend_result = self
                .process_backend_tool_requests(
                    &remaining_requests,
                    tools,
                    conversation,
                    harness_policy,
                    delegation_state,
                    harness_context,
                    &tool_response_plan,
                    cancel_token,
                    session,
                    session_id,
                    turns_taken,
                    current_mode,
                    transcript_store,
                )
                .await?;

            for msg in backend_result.approval_messages {
                events.push(AgentEvent::Message(msg));
            }

            for event in backend_result.batch_events {
                Self::capture_event_message(
                    messages_to_add,
                    &event,
                    HistoryCapturePolicy::SystemNotificationsOnly,
                );
                events.push(event);
            }

            tools_updated = backend_result.tools_updated;
            no_tools_called =
                frontend_requests.is_empty() && backend_result.executed_tool_calls == 0;
        }

        if !frontend_requests.is_empty() {
            no_tools_called = false;
        }

        Ok(ModelResponseHandling {
            events,
            no_tools_called,
            tools_updated,
            yield_after_first_event: true,
            deferred_tool_handling: Some(DeferredToolHandling {
                tool_response_plan,
                pending_frontend_request_ids,
                frontend_request_ids: frontend_requests
                    .iter()
                    .map(|request| request.id.clone())
                    .collect(),
                frontend_request_transports,
            }),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn process_provider_success_result(
        &self,
        response: Option<Message>,
        usage: Option<ProviderUsage>,
        tools: &[Tool],
        conversation: &Conversation,
        messages_to_add: &mut Conversation,
        harness_policy: &HarnessPolicy,
        delegation_state: &mut DelegationRuntimeState,
        harness_context: &HarnessContext,
        agime_mode: AgimeMode,
        cancel_token: Option<CancellationToken>,
        session: &Session,
        session_config: &SessionConfig,
        turns_taken: u32,
        current_mode: HarnessMode,
        transcript_store: &SessionHarnessStore,
        auto_swarm_injected_this_reply: &mut bool,
    ) -> Result<ProviderSuccessHandling> {
        let mut pre_response_events = Vec::new();

        if let Some(ref usage) = usage {
            let provider = self.provider().await?;
            if let Some(lead_worker) = provider.as_lead_worker() {
                let active_model = usage.model.clone();
                let (lead_model, worker_model) = lead_worker.get_model_info();
                let mode = if active_model == lead_model {
                    "lead"
                } else if active_model == worker_model {
                    "worker"
                } else {
                    "unknown"
                };

                pre_response_events.push(AgentEvent::ModelChange {
                    model: active_model,
                    mode: mode.to_string(),
                });
            }

            Self::update_session_metrics(session_config, usage, false).await?;
        }

        let response_handling = match response {
            Some(response) => Some(
                self.process_model_response(
                    &response,
                    tools,
                    conversation,
                    messages_to_add,
                    harness_policy,
                    delegation_state,
                    harness_context,
                    agime_mode,
                    cancel_token,
                    session,
                    &session_config.id,
                    turns_taken,
                    current_mode,
                    transcript_store,
                    auto_swarm_injected_this_reply,
                )
                .await?,
            ),
            None => {
                if detect_explicit_delegation_intent(conversation, delegation_state).is_some() {
                    let synthetic_response =
                        Message::assistant().with_metadata(MessageMetadata::agent_only());
                    Some(
                        self.process_model_response(
                            &synthetic_response,
                            tools,
                            conversation,
                            messages_to_add,
                            harness_policy,
                            delegation_state,
                            harness_context,
                            agime_mode,
                            cancel_token,
                            session,
                            &session_config.id,
                            turns_taken,
                            current_mode,
                            transcript_store,
                            auto_swarm_injected_this_reply,
                        )
                        .await?,
                    )
                } else {
                    None
                }
            }
        };

        Ok(ProviderSuccessHandling {
            pre_response_events,
            response_handling,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn handle_recovery_compaction(
        &self,
        conversation: &Conversation,
        session_config: &SessionConfig,
        transcript_store: &SessionHarnessStore,
        turns_taken: u32,
        current_mode: HarnessMode,
        max_compaction_attempts: u32,
        runtime_compaction_count: &mut u32,
    ) -> Result<RecoveryCompactionHandling> {
        match execute_recovery_compaction(
            self.provider().await?.as_ref(),
            conversation,
            &session_config.id,
            transcript_store,
            turns_taken,
            current_mode,
            max_compaction_attempts,
            *runtime_compaction_count,
        )
        .await
        {
            Ok(RecoveryCompactionExecution::Abort {
                message,
                next_runtime_compaction_count,
            }) => {
                *runtime_compaction_count = next_runtime_compaction_count;
                tracing::error!(
                    "Exceeded maximum compaction attempts ({}) for strategy {}",
                    max_compaction_attempts,
                    crate::agents::harness::compaction_strategy_label()
                );
                Ok(RecoveryCompactionHandling::Abort {
                    events: vec![AgentEvent::Message(Message::assistant().with_text(message))],
                })
            }
            Ok(RecoveryCompactionExecution::Continue {
                conversation: compacted_conversation,
                usage,
                prelude_messages,
                next_runtime_compaction_count,
            }) => {
                *runtime_compaction_count = next_runtime_compaction_count;
                Self::update_session_metrics(session_config, &usage, true).await?;
                let mut events = prelude_messages
                    .into_iter()
                    .map(AgentEvent::Message)
                    .collect::<Vec<_>>();
                events.push(AgentEvent::HistoryReplaced(compacted_conversation.clone()));
                Ok(RecoveryCompactionHandling::Continue {
                    conversation: compacted_conversation,
                    events,
                })
            }
            Err(e) => {
                tracing::error!("Error: {}", e);
                Ok(RecoveryCompactionHandling::Abort {
                    events: vec![AgentEvent::Message(Message::assistant().with_text(format!(
                        "Ran into this error trying to compact: {e}.\n\nPlease retry if you think this is a transient or recoverable error."
                    )))],
                })
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn handle_provider_stream_error(
        &self,
        provider_err: &ProviderError,
        conversation: &Conversation,
        session_config: &SessionConfig,
        transcript_store: &SessionHarnessStore,
        turns_taken: u32,
        current_mode: HarnessMode,
        max_compaction_attempts: u32,
        runtime_compaction_count: &mut u32,
        transition_trace: &super::SharedTransitionTrace,
    ) -> Result<ProviderErrorHandling> {
        crate::posthog::emit_error(provider_err.telemetry_type());

        if matches!(provider_err, ProviderError::ContextLengthExceeded(_)) {
            match self
                .handle_recovery_compaction(
                    conversation,
                    session_config,
                    transcript_store,
                    turns_taken,
                    current_mode,
                    max_compaction_attempts,
                    runtime_compaction_count,
                )
                .await?
            {
                RecoveryCompactionHandling::Continue {
                    conversation,
                    events,
                } => {
                    record_transition(
                        transition_trace,
                        turns_taken,
                        current_mode,
                        TransitionKind::ProviderTurnRecovery,
                        "context_length_recovery_continue",
                        std::collections::BTreeMap::new(),
                    )
                    .await;
                    return Ok(ProviderErrorHandling::ContinueTurn {
                        conversation,
                        events,
                        did_recovery_compact_this_iteration: true,
                    });
                }
                RecoveryCompactionHandling::Abort { events } => {
                    record_transition(
                        transition_trace,
                        turns_taken,
                        current_mode,
                        TransitionKind::ProviderTurnRecovery,
                        "context_length_recovery_abort",
                        std::collections::BTreeMap::new(),
                    )
                    .await;
                    return Ok(ProviderErrorHandling::BreakLoop { events });
                }
            }
        }

        tracing::error!("Error: {}", provider_err);
        Ok(ProviderErrorHandling::BreakLoop {
            events: vec![AgentEvent::Message(Message::assistant().with_text(format!(
                "Ran into this error: {provider_err}.\n\nPlease retry if you think this is a transient or recoverable error."
            )))],
        })
    }

    pub(crate) async fn emit_tool_request_transcript_messages(
        &self,
        tool_response_plan: &ToolResponsePlan,
        frontend_request_ids: &[String],
        messages_to_add: &mut Conversation,
        harness_context: &HarnessContext,
    ) -> Result<Vec<Message>> {
        let mut emitted_messages = Vec::new();
        let omit_frontend_transcript_in_execute =
            matches!(harness_context.mode, HarnessMode::Execute);

        for (request, response_message) in tool_response_plan.ordered_pairs() {
            if omit_frontend_transcript_in_execute
                && frontend_request_ids
                    .iter()
                    .any(|request_id| request_id == &request.id)
            {
                continue;
            }
            if request.tool_call.is_ok() {
                let request_msg = Message::assistant()
                    .with_id(format!("msg_{}", uuid::Uuid::new_v4()))
                    .with_tool_request(request.id.clone(), request.tool_call.clone());
                messages_to_add.push(request_msg);

                let final_response = Self::ensure_resolved_tool_response_message(
                    &request,
                    response_message.lock().await.clone(),
                );
                if !frontend_request_ids
                    .iter()
                    .any(|request_id| request_id == &request.id)
                {
                    Self::record_structured_tool_signals(
                        &request,
                        &final_response,
                        &harness_context.coordinator_signals,
                    )
                    .await;
                }
                emitted_messages.push(final_response.clone());
                messages_to_add.push(final_response);
            }
        }

        Ok(emitted_messages)
    }

    pub(crate) async fn complete_deferred_tool_handling(
        &self,
        deferred: DeferredToolHandling,
        messages_to_add: &mut Conversation,
        harness_context: &HarnessContext,
    ) -> Result<Vec<AgentEvent>> {
        let mut pending_frontend = deferred
            .pending_frontend_request_ids
            .into_iter()
            .collect::<std::collections::HashSet<_>>();

        while !pending_frontend.is_empty() {
            let Some((request_id, result)) = self.tool_result_rx.lock().await.recv().await else {
                return Err(anyhow::anyhow!(
                    "frontend tool result channel closed before all pending tool requests were resolved"
                ));
            };

            if !pending_frontend.remove(&request_id) {
                continue;
            }

            if let Some(response_msg) = deferred
                .tool_response_plan
                .request_to_response_map()
                .get(&request_id)
            {
                let mut response = response_msg.lock().await;
                *response = response
                    .clone()
                    .with_tool_response(request_id.clone(), result);
            }

            if let Some(request) = deferred.tool_response_plan.request_for(&request_id) {
                let tool_name = request
                    .tool_call
                    .as_ref()
                    .ok()
                    .map(|call| call.name.to_string())
                    .unwrap_or_default();
                let transport = deferred
                    .frontend_request_transports
                    .get(&request_id)
                    .copied()
                    .unwrap_or(ToolTransportKind::ExternalFrontend);
                let signal = if let Some(response_message) = deferred
                    .tool_response_plan
                    .response_message_for(&request_id)
                {
                    let response = response_message.lock().await.clone();
                    let tool_result = response
                        .content
                        .iter()
                        .find_map(|content| content.as_tool_response());
                    Self::coordinator_signal_for_tool_response(
                        &request_id,
                        tool_name,
                        transport,
                        tool_result,
                    )
                } else {
                    CoordinatorSignal::ToolCompleted {
                        request_id: request_id.clone(),
                        tool_name,
                        transport,
                    }
                };
                harness_context.coordinator_signals.record(signal).await;
                if let Some(response_message) = deferred
                    .tool_response_plan
                    .response_message_for(&request_id)
                {
                    let response = response_message.lock().await.clone();
                    Self::record_structured_tool_signals(
                        request,
                        &response,
                        &harness_context.coordinator_signals,
                    )
                    .await;
                }
            }
        }

        let mut events = Vec::new();
        for message in self
            .emit_tool_request_transcript_messages(
                &deferred.tool_response_plan,
                &deferred.frontend_request_ids,
                messages_to_add,
                harness_context,
            )
            .await?
        {
            events.push(AgentEvent::Message(message));
        }

        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::CallToolRequestParams;
    use serde_json::json;

    #[test]
    fn extract_persistable_assistant_text_message_keeps_only_text() {
        let message = Message::assistant()
            .with_text("final summary")
            .with_tool_request(
                "tool-1",
                Ok(CallToolRequestParams {
                    name: "swarm".into(),
                    arguments: Some(
                        json!({"targets":["docs/a.md","docs/b.md"]})
                            .as_object()
                            .cloned()
                            .expect("object"),
                    ),
                    meta: None,
                    task: None,
                }),
            );

        let persisted =
            Agent::extract_persistable_assistant_text_message(&message).expect("persisted");
        assert_eq!(persisted.content.len(), 1);
        match &persisted.content[0] {
            MessageContent::Text(text) => assert_eq!(text.text, "final summary"),
            other => panic!("expected text content, got {:?}", other),
        }
    }

    #[test]
    fn unresolved_tool_response_placeholder_becomes_error_response() {
        let request = ToolRequest {
            id: "tool-1".to_string(),
            tool_call: Ok(CallToolRequestParams {
                name: "swarm".into(),
                arguments: Some(
                    json!({"targets":["docs/a.md","docs/b.md"]})
                        .as_object()
                        .cloned()
                        .expect("object"),
                ),
                meta: None,
                task: None,
            }),
            thought_signature: None,
        };

        let message = Agent::ensure_resolved_tool_response_message(
            &request,
            Message::user().with_id("msg_tool_1".to_string()),
        );

        assert_eq!(message.content.len(), 1);
        match &message.content[0] {
            MessageContent::ToolResponse(response) => {
                assert_eq!(response.id, "tool-1");
                assert!(response.tool_result.is_err());
            }
            other => panic!("expected tool response, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_text_response_records_runtime_completion_ready() {
        let signals = super::super::signals::CoordinatorSignalStore::shared();
        let message = Message::assistant().with_text("final execute answer");

        Agent::record_execute_text_completion_if_ready(
            HarnessMode::Execute,
            Some(&message),
            &signals,
        )
        .await;

        let summary = signals.summarize().await;
        assert!(summary.completion_ready);
        assert_eq!(
            summary.latest_completion_summary.as_deref(),
            Some("final execute answer")
        );
    }

    #[test]
    fn semantic_tool_error_result_becomes_tool_failed_signal() {
        let tool_response = crate::conversation::message::ToolResponse {
            id: "tool-1".to_string(),
            tool_result: Ok(CallToolResult {
                content: vec![Content::text(
                    "Tool execution violated contract.".to_string(),
                )],
                structured_content: Some(json!({
                    "reason_code": "tool_contract_violation"
                })),
                is_error: Some(true),
                meta: None,
            }),
        };

        let signal = Agent::coordinator_signal_for_tool_response(
            "tool-1",
            "document_tools__read_document".to_string(),
            ToolTransportKind::ServerLocal,
            Some(&tool_response),
        );

        match signal {
            CoordinatorSignal::ToolFailed { error, .. } => {
                assert!(error.contains("violated contract"));
            }
            other => panic!("expected ToolFailed, got {:?}", other),
        }
    }

    #[test]
    fn document_tool_evidence_summary_does_not_reuse_internal_message_field() {
        let structured = json!({
            "type": "document_access_redirect",
            "doc_name": "fig_1.png",
            "message": "read_document was already completed for document 'fig_1.png'"
        });
        let summary =
            Agent::document_tool_evidence_summary(structured.as_object().expect("object"))
                .expect("summary");

        assert_eq!(summary, "Document access already established for fig_1.png");
        assert!(!summary.contains("read_document was already completed"));
    }

    #[test]
    fn swarm_completion_summary_prefers_worker_findings_over_generic_count() {
        let tool_response = crate::conversation::message::ToolResponse {
            id: "tool-1".to_string(),
            tool_result: Ok(CallToolResult {
                content: vec![Content::text(
                    "Swarm run `swarm-1` completed.\nWorker summaries:\n- README.md: updated build instructions\n- Cargo.toml: validated package metadata".to_string(),
                )],
                structured_content: Some(json!({
                    "produced_targets": ["README.md", "Cargo.toml"]
                })),
                is_error: Some(false),
                meta: None,
            }),
        };

        let summary = Agent::build_swarm_completion_summary(
            Some(&tool_response),
            &["README.md".to_string(), "Cargo.toml".to_string()],
        );

        assert!(summary.contains("Swarm completed for README.md and Cargo.toml."));
        assert!(summary.contains("README.md: updated build instructions"));
        assert!(summary.contains("Cargo.toml: validated package metadata"));
    }

    #[test]
    fn explicit_delegation_tool_detection_requires_real_subagent_or_swarm_call() {
        let shell_request = ToolRequest {
            id: "tool-shell".to_string(),
            tool_call: Ok(CallToolRequestParams {
                name: "developer__shell".into(),
                arguments: Some(
                    json!({"command":"pwd"})
                        .as_object()
                        .cloned()
                        .expect("object"),
                ),
                meta: None,
                task: None,
            }),
            thought_signature: None,
        };
        let swarm_request = ToolRequest {
            id: "tool-swarm".to_string(),
            tool_call: Ok(CallToolRequestParams {
                name: "swarm".into(),
                arguments: Some(
                    json!({"targets":["README.md","Cargo.toml"]})
                        .as_object()
                        .cloned()
                        .expect("object"),
                ),
                meta: None,
                task: None,
            }),
            thought_signature: None,
        };

        assert!(!Agent::has_explicit_delegation_tool_request(
            &[shell_request.clone()],
            &[],
        ));
        assert!(Agent::has_explicit_delegation_tool_request(
            &[],
            &[swarm_request],
        ));
        assert!(Agent::has_explicit_delegation_tool_request(
            &[shell_request],
            &[ToolRequest {
                id: "tool-subagent".to_string(),
                tool_call: Ok(CallToolRequestParams {
                    name: "subagent".into(),
                    arguments: Some(
                        json!({"instructions":"inspect env"})
                            .as_object()
                            .cloned()
                            .expect("object"),
                    ),
                    meta: None,
                    task: None,
                }),
                thought_signature: None,
            }],
        ));
    }

    #[test]
    fn explicit_swarm_bootstrap_override_replaces_non_delegation_tool_requests() {
        let conversation = Conversation::new_unvalidated(vec![Message::user().with_text(
            "Use swarm with these exact bounded targets in parallel: README.md and Cargo.toml.",
        )]);
        let response = Message::assistant()
            .with_text("Let me inspect the repo first.")
            .with_tool_request(
                "tool-shell",
                Ok(CallToolRequestParams {
                    name: "developer__shell".into(),
                    arguments: Some(
                        json!({"command":"pwd"})
                            .as_object()
                            .cloned()
                            .expect("object"),
                    ),
                    meta: None,
                    task: None,
                }),
            );
        let mut frontend_requests = Vec::new();
        let mut remaining_requests = vec![ToolRequest {
            id: "tool-shell".to_string(),
            tool_call: Ok(CallToolRequestParams {
                name: "developer__shell".into(),
                arguments: Some(
                    json!({"command":"pwd"})
                        .as_object()
                        .cloned()
                        .expect("object"),
                ),
                meta: None,
                task: None,
            }),
            thought_signature: None,
        }];
        let mut delegation = DelegationRuntimeState::new(
            DelegationMode::Swarm,
            0,
            1,
            vec!["README.md".to_string(), "Cargo.toml".to_string()],
            vec!["README.md".to_string(), "Cargo.toml".to_string()],
            vec!["README.md".to_string(), "Cargo.toml".to_string()],
        );

        let bootstrap = Agent::apply_explicit_delegation_bootstrap_override(
            &mut frontend_requests,
            &mut remaining_requests,
            &conversation,
            &response,
            &mut delegation,
        )
        .expect("bootstrap");

        assert!(frontend_requests.is_empty());
        assert_eq!(remaining_requests.len(), 1);
        let tool_call = remaining_requests[0]
            .tool_call
            .as_ref()
            .ok()
            .expect("tool call");
        assert_eq!(tool_call.name.as_ref(), "swarm");
        assert_eq!(bootstrap.transition_reason, "explicit_swarm_bootstrap");
    }

    #[test]
    fn explicit_swarm_bootstrap_override_is_idempotent_after_first_swarm_call() {
        let conversation = Conversation::new_unvalidated(vec![Message::user().with_text(
            "Use swarm with these exact bounded targets in parallel: README.md and docs/.",
        )]);
        let response = Message::assistant().with_text("I will coordinate the workers.");
        let mut frontend_requests = Vec::new();
        let mut remaining_requests = Vec::new();
        let mut delegation = DelegationRuntimeState::new(
            DelegationMode::Swarm,
            0,
            1,
            vec!["README.md".to_string(), "docs/".to_string()],
            vec!["README.md".to_string(), "docs/".to_string()],
            vec!["README.md".to_string(), "docs/".to_string()],
        );
        delegation.note_swarm_call();

        let bootstrap = Agent::apply_explicit_delegation_bootstrap_override(
            &mut frontend_requests,
            &mut remaining_requests,
            &conversation,
            &response,
            &mut delegation,
        );

        assert!(bootstrap.is_none());
        assert!(frontend_requests.is_empty());
        assert!(remaining_requests.is_empty());
    }
}
