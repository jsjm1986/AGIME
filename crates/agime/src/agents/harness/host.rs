use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::agents::extension::ExtensionConfig;
use crate::agents::types::SessionConfig;
use crate::agents::{Agent, AgentEvent};
use crate::context_runtime::{
    load_context_runtime_state, refresh_projection_only, save_context_runtime_state,
    ContextRuntimeState,
};
use crate::conversation::{message::Message, Conversation};
use crate::providers::base::Provider;
use crate::recipe::Response;
use crate::session::{SessionManager, SessionType};

use super::delegation::DelegationMode;
use super::host_state::{
    load_host_session_state, save_host_session_state, HarnessHostSessionState,
};
use super::signals::CoordinatorSignalSummary;
use super::state::{CoordinatorExecutionMode, HarnessMode, ProviderTurnMode};
use super::task_runtime::{register_task_runtime, unregister_task_runtime, TaskRuntime};
use super::transcript::{
    load_task_ledger_evidence_view, load_task_ledger_summary,
    load_task_ledger_transcript_resume_view, render_child_evidence_line,
    select_replayable_child_evidence, HarnessTranscriptStore, PersistedChildEvidenceItem,
    PersistedChildTranscriptView, PersistedTaskLedgerSummary, SessionHarnessStore,
};
use super::transition::TransitionTrace;
use super::{
    build_completion_control_messages, build_conversation_completion_report,
    build_execute_completion_report, build_session_finished_message, build_session_started_message,
    build_system_document_analysis_completion_report_from_conversation,
    control_messages_for_agent_event, register_harness_control_sequencer,
    unregister_harness_control_sequencer, CompletionSurfacePolicy, ExecuteCompletionOutcome,
    ExecutionHostCompletionReport, HarnessControlMessage, HarnessControlSequencer,
    HarnessControlSink,
};

#[async_trait]
pub trait HarnessEventSink: Send + Sync {
    async fn handle(
        &self,
        logical_session_id: &str,
        runtime_session_id: &str,
        agent: &Agent,
        event: &AgentEvent,
    ) -> Result<()>;
}

#[async_trait]
pub trait HarnessPersistenceAdapter: Send + Sync {
    async fn on_started(
        &self,
        _logical_session_id: &str,
        _runtime_session_id: &str,
        _initial_conversation: &Conversation,
        _mode: HarnessMode,
    ) -> Result<()> {
        Ok(())
    }

    async fn on_finished(
        &self,
        _logical_session_id: &str,
        _runtime_session_id: &str,
        _final_conversation: &Conversation,
        _mode: HarnessMode,
        _context_runtime_state: Option<&ContextRuntimeState>,
        _total_tokens: Option<i32>,
        _input_tokens: Option<i32>,
        _output_tokens: Option<i32>,
    ) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct NoopHarnessEventSink;

#[async_trait]
impl HarnessEventSink for NoopHarnessEventSink {
    async fn handle(
        &self,
        _logical_session_id: &str,
        _runtime_session_id: &str,
        _agent: &Agent,
        _event: &AgentEvent,
    ) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct NoopHarnessPersistenceAdapter;

#[async_trait]
impl HarnessPersistenceAdapter for NoopHarnessPersistenceAdapter {}

#[derive(Clone)]
pub struct HarnessHostDependencies {
    pub agent: Arc<Agent>,
    pub event_sink: Arc<dyn HarnessEventSink>,
    pub control_sink: Arc<dyn HarnessControlSink>,
    pub persistence: Arc<dyn HarnessPersistenceAdapter>,
}

#[derive(Clone)]
pub struct HarnessHostRequest {
    pub logical_session_id: String,
    pub working_dir: PathBuf,
    pub session_name: String,
    pub session_type: SessionType,
    pub initial_conversation: Conversation,
    pub user_message: Message,
    pub session_config: SessionConfig,
    pub provider: Arc<dyn Provider>,
    pub mode: HarnessMode,
    pub delegation_mode: DelegationMode,
    pub coordinator_execution_mode: CoordinatorExecutionMode,
    pub provider_turn_mode: ProviderTurnMode,
    pub completion_surface_policy: CompletionSurfacePolicy,
    pub write_scope: Vec<String>,
    pub target_artifacts: Vec<String>,
    pub result_contract: Vec<String>,
    pub server_local_tool_names: Vec<String>,
    pub required_tool_prefixes: Vec<String>,
    pub parallelism_budget: Option<u32>,
    pub swarm_budget: Option<u32>,
    pub validation_mode: bool,
    pub worker_extensions: Vec<ExtensionConfig>,
    pub initial_context_runtime_state: Option<ContextRuntimeState>,
    pub task_runtime: Option<Arc<TaskRuntime>>,
    pub system_prompt_override: Option<String>,
    pub system_prompt_extras: Vec<String>,
    pub extensions: Vec<ExtensionConfig>,
    pub final_output: Option<Response>,
    pub cancel_token: Option<CancellationToken>,
}

pub struct HarnessHostResult {
    pub runtime_session_id: String,
    pub final_conversation: Conversation,
    pub final_mode: HarnessMode,
    pub final_output_string: Option<String>,
    pub completion_report: ExecutionHostCompletionReport,
    pub persisted_child_evidence: Vec<PersistedChildEvidenceItem>,
    pub persisted_child_transcript_resume: Vec<PersistedChildTranscriptView>,
    pub transition_trace: Option<TransitionTrace>,
    pub context_runtime_state: Option<ContextRuntimeState>,
    pub total_tokens: Option<i32>,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub events_emitted: usize,
    pub signal_summary: Option<CoordinatorSignalSummary>,
    pub notification_summary: Option<CoordinatorSignalSummary>,
    pub completion_outcome: Option<ExecuteCompletionOutcome>,
}

fn merge_signal_summaries(
    base: Option<CoordinatorSignalSummary>,
    incremental: Option<CoordinatorSignalSummary>,
) -> Option<CoordinatorSignalSummary> {
    match (base, incremental) {
        (None, None) => None,
        (Some(summary), None) | (None, Some(summary)) => Some(summary),
        (Some(mut base), Some(incremental)) => {
            for tool_name in incremental.completed_tool_names {
                if !base
                    .completed_tool_names
                    .iter()
                    .any(|name| name == &tool_name)
                {
                    base.completed_tool_names.push(tool_name);
                }
            }
            for tool_name in incremental.failed_tool_names {
                if !base.failed_tool_names.iter().any(|name| name == &tool_name) {
                    base.failed_tool_names.push(tool_name);
                }
            }
            for outcome in incremental.worker_outcomes {
                if !base.worker_outcomes.iter().any(|existing| {
                    existing.task_id == outcome.task_id
                        && existing.status == outcome.status
                        && existing.attempt_identity == outcome.attempt_identity
                }) {
                    base.worker_outcomes.push(outcome);
                }
            }
            for outcome in incremental.validation_outcomes {
                if !base
                    .validation_outcomes
                    .iter()
                    .any(|existing| existing == &outcome)
                {
                    base.validation_outcomes.push(outcome);
                }
            }
            for target in incremental.accepted_targets {
                if !base.accepted_targets.iter().any(|value| value == &target) {
                    base.accepted_targets.push(target);
                }
            }
            for target in incremental.produced_targets {
                if !base.produced_targets.iter().any(|value| value == &target) {
                    base.produced_targets.push(target);
                }
            }

            base.tool_completed = base.completed_tool_names.len();
            base.tool_failed = base.failed_tool_names.len();
            base.worker_completed = base
                .worker_outcomes
                .iter()
                .filter(|outcome| outcome.status == "completed")
                .count();
            base.worker_failed = base
                .worker_outcomes
                .iter()
                .filter(|outcome| outcome.status == "failed")
                .count();
            base.validation_passed = base
                .validation_outcomes
                .iter()
                .filter(|outcome| {
                    matches!(outcome.status, super::child_tasks::ValidationStatus::Passed)
                })
                .count();
            base.validation_failed = base
                .validation_outcomes
                .iter()
                .filter(|outcome| {
                    matches!(outcome.status, super::child_tasks::ValidationStatus::Failed)
                })
                .count();
            base.worker_idle = base.worker_idle.max(incremental.worker_idle);
            base.worker_followups = base.worker_followups.max(incremental.worker_followups);
            base.permission_requested = base
                .permission_requested
                .max(incremental.permission_requested);
            base.permission_resolved = base
                .permission_resolved
                .max(incremental.permission_resolved);
            base.permission_timed_out = base
                .permission_timed_out
                .max(incremental.permission_timed_out);
            base.mailbox_messages = base.mailbox_messages.max(incremental.mailbox_messages);
            base.fallback_requested = base.fallback_requested.max(incremental.fallback_requested);
            base.completion_ready = base.completion_ready || incremental.completion_ready;
            if incremental.latest_completion_summary.is_some() {
                base.latest_completion_summary = incremental.latest_completion_summary;
            }
            if incremental.latest_structured_completion.is_some() {
                base.latest_structured_completion = incremental.latest_structured_completion;
            }
            if incremental.document_phase.is_some() {
                base.document_phase = incremental.document_phase;
            }
            if incremental.document_workspace_path.is_some() {
                base.document_workspace_path = incremental.document_workspace_path;
            }
            if incremental.document_next_action.is_some() {
                base.document_next_action = incremental.document_next_action;
            }
            Some(base)
        }
    }
}

fn apply_persisted_task_ledger_fallback(
    mut report: ExecutionHostCompletionReport,
    task_summary: Option<&PersistedTaskLedgerSummary>,
    child_evidence: &[PersistedChildEvidenceItem],
    signal_summary_missing: bool,
) -> ExecutionHostCompletionReport {
    if !signal_summary_missing {
        return report;
    }
    let Some(task_summary) = task_summary else {
        return report;
    };

    if report.produced_artifacts.is_empty() {
        report.produced_artifacts = task_summary.produced_artifacts.clone();
    }
    if report.accepted_artifacts.is_empty() {
        report.accepted_artifacts = task_summary.accepted_artifacts.clone();
    }
    if report.summary.starts_with("Runtime exited without") {
        if task_summary.active_tasks > 0 {
            if let Some(detail) = task_summary.latest_runtime_detail.clone() {
                report.summary = format!(
                    "Runtime evidence recovered from persisted child tasks. {} child task(s) are still active. Latest runtime detail: {}",
                    task_summary.active_tasks, detail
                );
            } else {
                report.summary = format!(
                    "Runtime evidence recovered from persisted child tasks. {} child task(s) are still active.",
                    task_summary.active_tasks
                );
            }
        } else if let Some(latest_summary) = task_summary.latest_summary.clone() {
            report.summary = format!(
                "Runtime evidence recovered from persisted child tasks. Latest child summary: {}",
                latest_summary
            );
        }
    }
    if report.blocking_reason.is_none() {
        if task_summary.active_tasks > 0 {
            report.blocking_reason = Some(if task_summary.active_runtime_states.is_empty() {
                "child tasks are still running".to_string()
            } else {
                format!(
                    "child tasks are still running ({})",
                    task_summary.active_runtime_states.join(", ")
                )
            });
        } else if task_summary.failed_tasks > 0 {
            report.blocking_reason = Some("one or more child tasks failed".to_string());
        }
    }
    if !task_summary.child_session_ids.is_empty()
        && !report
            .next_steps
            .iter()
            .any(|step| step.contains("child session"))
    {
        report.next_steps.push(format!(
            "Inspect child session(s): {}",
            task_summary.child_session_ids.join(", ")
        ));
    }
    let evidence_source = select_replayable_child_evidence(child_evidence);
    if !evidence_source.is_empty() {
        let evidence_steps = evidence_source
            .into_iter()
            .map(|(item, recent_terminal)| render_child_evidence_line(item, recent_terminal))
            .collect::<Vec<_>>();
        for step in evidence_steps {
            if !report.next_steps.iter().any(|existing| existing == &step) {
                report.next_steps.push(step);
            }
        }
    }

    report
}

pub async fn run_harness_host(
    request: HarnessHostRequest,
    dependencies: HarnessHostDependencies,
) -> Result<HarnessHostResult> {
    tracing::info!(
        logical_session_id = %request.logical_session_id,
        delegation_mode = ?request.delegation_mode,
        coordinator_execution_mode = %request.coordinator_execution_mode,
        provider_turn_mode = %request.provider_turn_mode,
        target_count = request.target_artifacts.len(),
        result_contract_count = request.result_contract.len(),
        "run_harness_host: starting"
    );
    let transcript_store = SessionHarnessStore;
    let runtime_session = SessionManager::create_session(
        request.working_dir.clone(),
        request.session_name.clone(),
        request.session_type,
    )
    .await?;
    let runtime_session_id = runtime_session.id.clone();
    let control_sequencer = Arc::new(HarnessControlSequencer::new(
        request.logical_session_id.clone(),
        runtime_session_id.clone(),
    ));
    register_harness_control_sequencer(&runtime_session_id, control_sequencer.clone());
    if let Some(task_runtime) = request.task_runtime.clone() {
        register_task_runtime(&runtime_session_id, task_runtime);
    }

    let result = async {
        if !request.initial_conversation.is_empty() {
            SessionManager::replace_conversation(
                &runtime_session_id,
                &request.initial_conversation,
            )
            .await?;
        }

        if let Some(context_runtime_state) = request.initial_context_runtime_state.clone() {
            save_context_runtime_state(&runtime_session_id, &context_runtime_state).await?;
        }

        transcript_store
            .save_mode(&runtime_session_id, request.mode)
            .await?;
        save_host_session_state(
            &runtime_session_id,
            &HarnessHostSessionState {
                task_board_session_id: Some(request.logical_session_id.clone()),
                delegation_mode: request.delegation_mode,
                coordinator_execution_mode: request.coordinator_execution_mode,
                provider_turn_mode: request.provider_turn_mode,
                completion_surface_policy: request.completion_surface_policy,
                write_scope: request.write_scope.clone(),
                target_artifacts: request.target_artifacts.clone(),
                result_contract: request.result_contract.clone(),
                server_local_tool_names: request.server_local_tool_names.clone(),
                required_tool_prefixes: request.required_tool_prefixes.clone(),
                parallelism_budget: request.parallelism_budget,
                swarm_budget: request.swarm_budget,
                validation_mode: request.validation_mode,
                worker_extensions: request.worker_extensions.clone(),
                last_notification_summary: None,
                last_signal_summary: None,
                last_completion_outcome: None,
                last_transition_trace: None,
            },
        )
        .await?;

        dependencies
            .persistence
            .on_started(
                &request.logical_session_id,
                &runtime_session_id,
                &request.initial_conversation,
                request.mode,
            )
            .await?;

        emit_control_messages(
            dependencies.control_sink.as_ref(),
            control_sequencer.as_ref(),
            [build_session_started_message(
                request.mode,
                request.completion_surface_policy,
            )],
        )
        .await;

        dependencies
            .agent
            .update_provider(request.provider.clone(), &runtime_session_id)
            .await?;

        if let Some(template) = request.system_prompt_override.clone() {
            dependencies.agent.override_system_prompt(template).await;
        }
        for instruction in &request.system_prompt_extras {
            dependencies
                .agent
                .extend_system_prompt(instruction.clone())
                .await;
        }
        for extension in request.extensions.clone() {
            dependencies.agent.add_extension(extension).await?;
        }
        if let Some(final_output) = request.final_output.clone() {
            dependencies.agent.add_final_output_tool(final_output).await;
        }

        let mut runtime_session_config = request.session_config.clone();
        runtime_session_config.id = runtime_session_id.clone();

        let mut stream = dependencies
            .agent
            .reply(
                request.user_message.clone(),
                runtime_session_config,
                request.cancel_token.clone(),
            )
            .await?;

        let mut events_emitted = 0usize;
        while let Some(event) = stream.next().await {
            let event = event?;
            emit_control_messages(
                dependencies.control_sink.as_ref(),
                control_sequencer.as_ref(),
                control_messages_for_agent_event(&event),
            )
            .await;
            dependencies
                .event_sink
                .handle(
                    &request.logical_session_id,
                    &runtime_session_id,
                    dependencies.agent.as_ref(),
                    &event,
                )
                .await?;
            events_emitted = events_emitted.saturating_add(1);
        }
        tracing::info!(
            logical_session_id = %request.logical_session_id,
            runtime_session_id = %runtime_session_id,
            events_emitted,
            "run_harness_host: event stream drained"
        );

        let final_session = SessionManager::get_session(&runtime_session_id, true).await?;
        let final_conversation = final_session.conversation.unwrap_or_default();
        let final_mode = transcript_store.load_mode(&runtime_session_id).await?;
        let final_output_string = dependencies.agent.final_output_string().await;
        let host_state = load_host_session_state(&runtime_session_id).await?;
        let context_runtime_state = load_context_runtime_state(&runtime_session_id)
            .await
            .ok()
            .filter(|state| {
                state.runtime_compactions > 0
                    || crate::context_runtime::has_persistent_runtime_state(state)
            })
            .or_else(|| {
                request
                    .initial_context_runtime_state
                    .clone()
                    .map(|mut state| {
                        refresh_projection_only(&final_conversation, &mut state);
                        state
                    })
            });
        let notification_summary = host_state
            .as_ref()
            .and_then(|state| state.last_notification_summary.clone());
        let signal_summary = host_state
            .as_ref()
            .and_then(|state| state.last_signal_summary.clone());
        let merged_signal_summary =
            merge_signal_summaries(signal_summary.clone(), notification_summary.clone());
        let task_ledger_summary = load_task_ledger_summary(&runtime_session_id).await.ok();
        let persisted_child_evidence = load_task_ledger_evidence_view(&runtime_session_id)
            .await
            .unwrap_or_default();
        let persisted_child_transcript_resume =
            load_task_ledger_transcript_resume_view(&runtime_session_id)
                .await
                .unwrap_or_default();
        let completion_outcome = host_state
            .as_ref()
            .and_then(|state| state.last_completion_outcome.clone());
        let transition_trace = host_state
            .as_ref()
            .and_then(|state| state.last_transition_trace.clone());
        let completion_report = match request.completion_surface_policy {
            CompletionSurfacePolicy::Conversation => build_conversation_completion_report(
                &final_conversation,
                completion_outcome.as_ref(),
                merged_signal_summary.as_ref(),
            ),
            CompletionSurfacePolicy::Execute => build_execute_completion_report(
                final_output_string.as_deref(),
                completion_outcome.as_ref(),
                merged_signal_summary.as_ref(),
                &request.required_tool_prefixes,
            ),
            CompletionSurfacePolicy::SystemDocumentAnalysis => {
                build_system_document_analysis_completion_report_from_conversation(
                    final_output_string.as_deref(),
                    &final_conversation,
                    completion_outcome.as_ref(),
                    merged_signal_summary.as_ref(),
                    &request.required_tool_prefixes,
                )
            }
        };
        let completion_report = apply_persisted_task_ledger_fallback(
            completion_report,
            task_ledger_summary.as_ref(),
            &persisted_child_evidence,
            merged_signal_summary.is_none(),
        );
        tracing::info!(
            logical_session_id = %request.logical_session_id,
            runtime_session_id = %runtime_session_id,
            conversation_len = final_conversation.len(),
            final_mode = ?final_mode,
            "run_harness_host: loaded final runtime session"
        );

        dependencies
            .persistence
            .on_finished(
                &request.logical_session_id,
                &runtime_session_id,
                &final_conversation,
                final_mode,
                context_runtime_state.as_ref(),
                final_session.total_tokens,
                final_session.input_tokens,
                final_session.output_tokens,
            )
            .await?;
        tracing::info!(
            logical_session_id = %request.logical_session_id,
            runtime_session_id = %runtime_session_id,
            "run_harness_host: persistence finished"
        );

        emit_control_messages(
            dependencies.control_sink.as_ref(),
            control_sequencer.as_ref(),
            build_completion_control_messages(&completion_report, completion_outcome.as_ref()),
        )
        .await;
        emit_control_messages(
            dependencies.control_sink.as_ref(),
            control_sequencer.as_ref(),
            [build_session_finished_message(
                completion_report.status.clone(),
            )],
        )
        .await;

        Ok(HarnessHostResult {
            runtime_session_id: runtime_session_id.clone(),
            final_conversation,
            final_mode,
            final_output_string,
            completion_report,
            persisted_child_evidence,
            persisted_child_transcript_resume,
            transition_trace,
            context_runtime_state,
            total_tokens: final_session.total_tokens,
            input_tokens: final_session.input_tokens,
            output_tokens: final_session.output_tokens,
            events_emitted,
            notification_summary,
            signal_summary: merged_signal_summary,
            completion_outcome,
        })
    }
    .await;

    unregister_task_runtime(&runtime_session_id);
    unregister_harness_control_sequencer(&runtime_session_id);
    tracing::info!(
        logical_session_id = %request.logical_session_id,
        runtime_session_id = %runtime_session_id,
        success = result.is_ok(),
        "run_harness_host: cleanup complete"
    );
    result
}

async fn emit_control_messages<I>(
    sink: &dyn HarnessControlSink,
    sequencer: &HarnessControlSequencer,
    messages: I,
) where
    I: IntoIterator<Item = HarnessControlMessage>,
{
    for message in messages {
        let envelope = sequencer.next(message);
        if let Err(error) = sink.handle(&envelope).await {
            tracing::warn!(
                session_id = %envelope.session_id,
                runtime_session_id = %envelope.runtime_session_id,
                sequence = envelope.sequence,
                error = %error,
                "harness control sink failed; continuing without interrupting execution"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;
    use async_trait::async_trait;
    use tokio::sync::Mutex;

    use crate::agents::types::SessionConfig;
    use crate::agents::{Agent, AgentEvent};
    use crate::conversation::message::{Message, MessageContent};
    use crate::model::ModelConfig;
    use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
    use crate::providers::errors::ProviderError;
    use crate::session::SessionType;

    use super::*;

    struct MockProvider {
        model_config: ModelConfig,
        response: String,
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn metadata() -> ProviderMetadata
        where
            Self: Sized,
        {
            ProviderMetadata::new(
                "mock",
                "Mock",
                "Mock provider",
                "mock-model",
                vec![],
                "",
                vec![],
            )
        }

        fn get_name(&self) -> &str {
            "mock"
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[rmcp::model::Tool],
        ) -> std::result::Result<(Message, ProviderUsage), ProviderError> {
            Ok((
                Message::assistant().with_text(self.response.clone()),
                ProviderUsage::new("mock-model".to_string(), Usage::new(None, None, None)),
            ))
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        texts: Mutex<Vec<String>>,
    }

    #[derive(Default)]
    struct RecordingControlSink {
        envelopes: Mutex<Vec<crate::agents::HarnessControlEnvelope>>,
    }

    #[async_trait]
    impl HarnessEventSink for RecordingSink {
        async fn handle(
            &self,
            _logical_session_id: &str,
            _runtime_session_id: &str,
            _agent: &Agent,
            event: &AgentEvent,
        ) -> Result<()> {
            if let AgentEvent::Message(message) = event {
                for content in &message.content {
                    if let MessageContent::Text(text) = content {
                        self.texts.lock().await.push(text.text.clone());
                    }
                }
            }
            Ok(())
        }
    }

    #[async_trait]
    impl HarnessControlSink for RecordingControlSink {
        async fn handle(&self, envelope: &crate::agents::HarnessControlEnvelope) -> Result<()> {
            self.envelopes.lock().await.push(envelope.clone());
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingPersistence {
        started: Mutex<u32>,
        finished: Mutex<u32>,
    }

    #[async_trait]
    impl HarnessPersistenceAdapter for RecordingPersistence {
        async fn on_started(
            &self,
            _logical_session_id: &str,
            _runtime_session_id: &str,
            _initial_conversation: &Conversation,
            _mode: HarnessMode,
        ) -> Result<()> {
            *self.started.lock().await += 1;
            Ok(())
        }

        async fn on_finished(
            &self,
            _logical_session_id: &str,
            _runtime_session_id: &str,
            _final_conversation: &Conversation,
            _mode: HarnessMode,
            _context_runtime_state: Option<&ContextRuntimeState>,
            _total_tokens: Option<i32>,
            _input_tokens: Option<i32>,
            _output_tokens: Option<i32>,
        ) -> Result<()> {
            *self.finished.lock().await += 1;
            Ok(())
        }
    }

    #[tokio::test]
    async fn direct_host_runs_agent_reply_and_returns_conversation() {
        let agent = Arc::new(Agent::new());
        let sink = Arc::new(RecordingSink::default());
        let control_sink = Arc::new(RecordingControlSink::default());
        let persistence = Arc::new(RecordingPersistence::default());
        let provider = Arc::new(MockProvider {
            model_config: ModelConfig::new_or_fail("mock-model"),
            response: "hello from host".to_string(),
        });
        let request = HarnessHostRequest {
            logical_session_id: "logical-1".to_string(),
            working_dir: std::env::temp_dir(),
            session_name: "host-test".to_string(),
            session_type: SessionType::Hidden,
            initial_conversation: Conversation::empty(),
            user_message: Message::user().with_text("hello"),
            session_config: SessionConfig {
                id: "placeholder".to_string(),
                schedule_id: None,
                max_turns: Some(4),
                retry_config: None,
            },
            provider,
            mode: HarnessMode::Conversation,
            delegation_mode: DelegationMode::Subagent,
            coordinator_execution_mode: CoordinatorExecutionMode::SingleWorker,
            provider_turn_mode: ProviderTurnMode::Streaming,
            completion_surface_policy: CompletionSurfacePolicy::Conversation,
            write_scope: Vec::new(),
            target_artifacts: Vec::new(),
            result_contract: Vec::new(),
            server_local_tool_names: Vec::new(),
            required_tool_prefixes: Vec::new(),
            parallelism_budget: None,
            swarm_budget: None,
            validation_mode: false,
            worker_extensions: Vec::new(),
            initial_context_runtime_state: None,
            task_runtime: None,
            system_prompt_override: None,
            system_prompt_extras: Vec::new(),
            extensions: Vec::new(),
            final_output: None,
            cancel_token: None,
        };
        let result = run_harness_host(
            request,
            HarnessHostDependencies {
                agent,
                event_sink: sink.clone(),
                control_sink: control_sink.clone(),
                persistence: persistence.clone(),
            },
        )
        .await
        .expect("host should succeed");

        assert!(!result.runtime_session_id.is_empty());
        assert_eq!(result.final_conversation.len(), 2);
        assert_eq!(result.final_mode, HarnessMode::Conversation);
        assert_eq!(result.completion_report.status, "completed");
        assert_eq!(result.completion_report.summary, "hello from host");
        assert!(result.persisted_child_evidence.is_empty());
        assert!(!sink.texts.lock().await.is_empty());
        let envelopes = control_sink.envelopes.lock().await.clone();
        assert!(envelopes.len() >= 4);
        assert_eq!(envelopes[0].sequence, 1);
        assert!(envelopes.iter().any(|item| {
            matches!(
                item.payload,
                crate::agents::HarnessControlMessage::Session(
                    crate::agents::SessionControlEvent::Started { .. }
                )
            )
        }));
        assert!(envelopes.iter().any(|item| {
            matches!(
                item.payload,
                crate::agents::HarnessControlMessage::Completion(
                    crate::agents::CompletionControlEvent::StructuredPublished { .. }
                )
            )
        }));
        assert_eq!(*persistence.started.lock().await, 1);
        assert_eq!(*persistence.finished.lock().await, 1);
    }

    #[test]
    fn persisted_task_ledger_fallback_enriches_blocked_report() {
        let report = apply_persisted_task_ledger_fallback(
            ExecutionHostCompletionReport {
                status: "blocked".to_string(),
                summary: "Runtime exited without terminal completion evidence.".to_string(),
                produced_artifacts: Vec::new(),
                accepted_artifacts: Vec::new(),
                next_steps: vec!["retry".to_string()],
                validation_status: None,
                blocking_reason: None,
                reason_code: None,
                content_accessed: None,
                analysis_complete: None,
            },
            Some(&PersistedTaskLedgerSummary {
                total_tasks: 2,
                active_tasks: 0,
                completed_tasks: 1,
                failed_tasks: 1,
                latest_summary: Some("FAIL: child task failed".to_string()),
                latest_runtime_detail: None,
                active_runtime_states: Vec::new(),
                produced_artifacts: vec!["docs/a.md".to_string()],
                accepted_artifacts: vec!["docs/a.md".to_string()],
                child_session_ids: vec!["subagent-1".to_string()],
            }),
            &[],
            true,
        );

        assert!(report.summary.contains("Latest child summary"));
        assert_eq!(report.produced_artifacts, vec!["docs/a.md".to_string()]);
        assert_eq!(report.accepted_artifacts, vec!["docs/a.md".to_string()]);
        assert_eq!(
            report.blocking_reason.as_deref(),
            Some("one or more child tasks failed")
        );
    }

    #[test]
    fn persisted_task_ledger_fallback_reports_active_child_sessions() {
        let report = apply_persisted_task_ledger_fallback(
            ExecutionHostCompletionReport {
                status: "blocked".to_string(),
                summary: "Runtime exited without terminal completion evidence.".to_string(),
                produced_artifacts: Vec::new(),
                accepted_artifacts: Vec::new(),
                next_steps: vec!["retry".to_string()],
                validation_status: None,
                blocking_reason: None,
                reason_code: None,
                content_accessed: None,
                analysis_complete: None,
            },
            Some(&PersistedTaskLedgerSummary {
                total_tasks: 2,
                active_tasks: 2,
                completed_tasks: 0,
                failed_tasks: 0,
                latest_summary: None,
                latest_runtime_detail: Some("worker-a waiting on developer__shell".to_string()),
                active_runtime_states: vec!["permission_requested".to_string()],
                produced_artifacts: Vec::new(),
                accepted_artifacts: Vec::new(),
                child_session_ids: vec!["subagent-1".to_string(), "subagent-2".to_string()],
            }),
            &[PersistedChildEvidenceItem {
                task_id: "task-1".to_string(),
                status: "running".to_string(),
                logical_worker_id: None,
                attempt_id: None,
                recovered_terminal_summary: false,
                child_session_id: Some("subagent-1".to_string()),
                session_preview: Some("child session preview".to_string()),
                transcript_excerpt: Vec::new(),
                runtime_state: Some("permission_requested".to_string()),
                runtime_detail: Some("worker-a waiting on developer__shell".to_string()),
                summary: None,
            }],
            true,
        );

        assert!(report.summary.contains("2 child task(s) are still active"));
        assert!(report
            .summary
            .contains("worker-a waiting on developer__shell"));
        assert_eq!(
            report.blocking_reason.as_deref(),
            Some("child tasks are still running (permission_requested)")
        );
        assert!(report
            .next_steps
            .iter()
            .any(|step| step.contains("subagent-1") && step.contains("subagent-2")));
        assert!(report
            .next_steps
            .iter()
            .any(|step| step.contains("task-1") && step.contains("permission_requested")));
    }

    #[test]
    fn persisted_task_ledger_fallback_uses_session_preview_when_runtime_detail_missing() {
        let report = apply_persisted_task_ledger_fallback(
            ExecutionHostCompletionReport {
                status: "blocked".to_string(),
                summary: "Runtime exited without terminal completion evidence.".to_string(),
                produced_artifacts: Vec::new(),
                accepted_artifacts: Vec::new(),
                next_steps: vec!["retry".to_string()],
                validation_status: None,
                blocking_reason: None,
                reason_code: None,
                content_accessed: None,
                analysis_complete: None,
            },
            Some(&PersistedTaskLedgerSummary {
                total_tasks: 1,
                active_tasks: 1,
                completed_tasks: 0,
                failed_tasks: 0,
                latest_summary: None,
                latest_runtime_detail: None,
                active_runtime_states: vec!["running".to_string()],
                produced_artifacts: Vec::new(),
                accepted_artifacts: Vec::new(),
                child_session_ids: vec!["subagent-1".to_string()],
            }),
            &[PersistedChildEvidenceItem {
                task_id: "task-1".to_string(),
                status: "running".to_string(),
                logical_worker_id: None,
                attempt_id: None,
                recovered_terminal_summary: false,
                child_session_id: Some("subagent-1".to_string()),
                session_preview: Some("child session preview".to_string()),
                transcript_excerpt: Vec::new(),
                runtime_state: Some("running".to_string()),
                runtime_detail: None,
                summary: None,
            }],
            true,
        );

        assert!(report
            .next_steps
            .iter()
            .any(|step| step.contains("child session preview")));
    }

    #[test]
    fn persisted_task_ledger_fallback_uses_transcript_excerpt_when_preview_missing() {
        let report = apply_persisted_task_ledger_fallback(
            ExecutionHostCompletionReport {
                status: "blocked".to_string(),
                summary: "Runtime exited without terminal completion evidence.".to_string(),
                produced_artifacts: Vec::new(),
                accepted_artifacts: Vec::new(),
                next_steps: vec!["retry".to_string()],
                validation_status: None,
                blocking_reason: None,
                reason_code: None,
                content_accessed: None,
                analysis_complete: None,
            },
            Some(&PersistedTaskLedgerSummary {
                active_tasks: 1,
                child_session_ids: vec!["subagent-1".to_string()],
                ..Default::default()
            }),
            &[PersistedChildEvidenceItem {
                task_id: "task-1".to_string(),
                status: "running".to_string(),
                logical_worker_id: None,
                attempt_id: None,
                recovered_terminal_summary: false,
                child_session_id: Some("subagent-1".to_string()),
                session_preview: None,
                transcript_excerpt: vec![
                    "user: inspect docs/a.md".to_string(),
                    "assistant: checking the file".to_string(),
                ],
                runtime_state: Some("running".to_string()),
                runtime_detail: None,
                summary: None,
            }],
            true,
        );
        assert!(report
            .next_steps
            .iter()
            .any(|step| step.contains("assistant: checking the file")));
    }

    #[test]
    fn persisted_task_ledger_fallback_shows_recent_terminal_child_evidence_when_no_active_tasks() {
        let report = apply_persisted_task_ledger_fallback(
            ExecutionHostCompletionReport {
                status: "blocked".to_string(),
                summary: "Runtime exited without terminal completion evidence.".to_string(),
                produced_artifacts: Vec::new(),
                accepted_artifacts: Vec::new(),
                next_steps: vec!["retry".to_string()],
                validation_status: None,
                blocking_reason: None,
                reason_code: None,
                content_accessed: None,
                analysis_complete: None,
            },
            Some(&PersistedTaskLedgerSummary {
                total_tasks: 1,
                active_tasks: 0,
                completed_tasks: 1,
                failed_tasks: 0,
                latest_summary: Some("child completed".to_string()),
                latest_runtime_detail: None,
                active_runtime_states: Vec::new(),
                produced_artifacts: Vec::new(),
                accepted_artifacts: Vec::new(),
                child_session_ids: vec!["subagent-1".to_string()],
            }),
            &[PersistedChildEvidenceItem {
                task_id: "task-1".to_string(),
                status: "completed".to_string(),
                logical_worker_id: None,
                attempt_id: None,
                recovered_terminal_summary: false,
                child_session_id: Some("subagent-1".to_string()),
                session_preview: None,
                transcript_excerpt: vec!["assistant: child completed terminal excerpt".to_string()],
                runtime_state: None,
                runtime_detail: None,
                summary: Some("child completed".to_string()),
            }],
            true,
        );
        assert!(report
            .next_steps
            .iter()
            .any(|step| step.contains("Recent child evidence")));
        assert!(report
            .next_steps
            .iter()
            .any(|step| step.contains("completed")));
        assert!(report
            .next_steps
            .iter()
            .any(|step| step.contains("terminal excerpt")));
    }

    #[test]
    fn persisted_task_ledger_fallback_keeps_active_and_recent_terminal_child_evidence() {
        let report = apply_persisted_task_ledger_fallback(
            ExecutionHostCompletionReport {
                status: "blocked".to_string(),
                summary: "Runtime exited without terminal completion evidence.".to_string(),
                produced_artifacts: Vec::new(),
                accepted_artifacts: Vec::new(),
                next_steps: vec!["retry".to_string()],
                validation_status: None,
                blocking_reason: None,
                reason_code: None,
                content_accessed: None,
                analysis_complete: None,
            },
            Some(&PersistedTaskLedgerSummary {
                total_tasks: 2,
                active_tasks: 1,
                completed_tasks: 0,
                failed_tasks: 1,
                latest_summary: Some("FAIL: child failed".to_string()),
                latest_runtime_detail: Some("worker-a still checking".to_string()),
                active_runtime_states: vec!["running".to_string()],
                produced_artifacts: Vec::new(),
                accepted_artifacts: Vec::new(),
                child_session_ids: vec![
                    "subagent-active".to_string(),
                    "subagent-failed".to_string(),
                ],
            }),
            &[
                PersistedChildEvidenceItem {
                    task_id: "task-active".to_string(),
                    status: "running".to_string(),
                    logical_worker_id: None,
                    attempt_id: None,
                    recovered_terminal_summary: false,
                    child_session_id: Some("subagent-active".to_string()),
                    session_preview: Some("still working".to_string()),
                    transcript_excerpt: Vec::new(),
                    runtime_state: Some("running".to_string()),
                    runtime_detail: Some("worker-a still checking".to_string()),
                    summary: None,
                },
                PersistedChildEvidenceItem {
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
                    summary: Some("FAIL: child failed".to_string()),
                },
            ],
            true,
        );
        assert!(report
            .next_steps
            .iter()
            .any(|step| step.contains("subagent-active")));
        assert!(report
            .next_steps
            .iter()
            .any(|step| step.contains("subagent-failed")));
        assert!(report
            .next_steps
            .iter()
            .any(|step| step.contains("Recent child evidence")));
    }
}
