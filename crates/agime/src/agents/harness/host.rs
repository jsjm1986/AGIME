use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::agents::extension::ExtensionConfig;
use crate::agents::types::SessionConfig;
use crate::agents::{Agent, AgentEvent};
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
use super::transcript::{HarnessTranscriptStore, SessionHarnessStore};
use super::{
    build_conversation_completion_report, build_execute_completion_report,
    build_system_document_analysis_completion_report, CompletionSurfacePolicy,
    ExecuteCompletionOutcome, ExecutionHostCompletionReport,
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
    pub total_tokens: Option<i32>,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub events_emitted: usize,
    pub signal_summary: Option<CoordinatorSignalSummary>,
    pub notification_summary: Option<CoordinatorSignalSummary>,
    pub completion_outcome: Option<ExecuteCompletionOutcome>,
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
        let notification_summary = host_state
            .as_ref()
            .and_then(|state| state.last_notification_summary.clone());
        let signal_summary = host_state
            .as_ref()
            .and_then(|state| state.last_signal_summary.clone());
        let completion_outcome = host_state.and_then(|state| state.last_completion_outcome);
        let completion_report = match request.completion_surface_policy {
            CompletionSurfacePolicy::Conversation => build_conversation_completion_report(
                &final_conversation,
                completion_outcome.as_ref(),
                signal_summary.as_ref(),
            ),
            CompletionSurfacePolicy::Execute => build_execute_completion_report(
                final_output_string.as_deref(),
                completion_outcome.as_ref(),
                signal_summary.as_ref(),
                &request.required_tool_prefixes,
            ),
            CompletionSurfacePolicy::SystemDocumentAnalysis => {
                build_system_document_analysis_completion_report(
                    final_output_string.as_deref(),
                    completion_outcome.as_ref(),
                    signal_summary.as_ref(),
                    &request.required_tool_prefixes,
                )
            }
        };
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

        Ok(HarnessHostResult {
            runtime_session_id: runtime_session_id.clone(),
            final_conversation,
            final_mode,
            final_output_string,
            completion_report,
            total_tokens: final_session.total_tokens,
            input_tokens: final_session.input_tokens,
            output_tokens: final_session.output_tokens,
            events_emitted,
            notification_summary,
            signal_summary,
            completion_outcome,
        })
    }
    .await;

    let _ = SessionManager::delete_session(&runtime_session_id).await;
    unregister_task_runtime(&runtime_session_id);
    tracing::info!(
        logical_session_id = %request.logical_session_id,
        runtime_session_id = %runtime_session_id,
        success = result.is_ok(),
        "run_harness_host: cleanup complete"
    );
    result
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
        assert!(!sink.texts.lock().await.is_empty());
        assert_eq!(*persistence.started.lock().await, 1);
        assert_eq!(*persistence.finished.lock().await, 1);
    }
}
