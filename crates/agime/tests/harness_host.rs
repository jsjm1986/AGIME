use std::sync::Arc;

use agime::agents::harness::CompletionSurfacePolicy;
use agime::agents::{
    run_harness_host, Agent, AgentEvent, CoordinatorExecutionMode, HarnessEventSink,
    HarnessHostDependencies, HarnessHostRequest, HarnessMode, HarnessPersistenceAdapter,
    ProviderTurnMode, SessionConfig,
};
use agime::conversation::message::{Message, MessageContent};
use agime::conversation::Conversation;
use agime::model::ModelConfig;
use agime::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use agime::providers::errors::ProviderError;
use agime::session::SessionType;
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;

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
        delegation_mode: agime::agents::DelegationMode::Subagent,
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
    assert_eq!(result.total_tokens, None);
    assert!(!sink.texts.lock().await.is_empty());
    assert_eq!(*persistence.started.lock().await, 1);
    assert_eq!(*persistence.finished.lock().await, 1);
}

#[tokio::test]
async fn direct_host_planner_auto_injects_swarm_request() {
    std::env::set_var("AGIME_ENABLE_NATIVE_SWARM_TOOL", "true");
    std::env::set_var("AGIME_ENABLE_SWARM_PLANNER_AUTO", "true");

    let agent = Arc::new(Agent::new());
    let sink = Arc::new(RecordingSink::default());
    let persistence = Arc::new(RecordingPersistence::default());
    let provider = Arc::new(MockProvider {
        model_config: ModelConfig::new_or_fail("mock-model"),
        response: "I will split this bounded request into parallel workstreams.".to_string(),
    });
    let request = HarnessHostRequest {
        logical_session_id: "logical-auto".to_string(),
        working_dir: std::env::temp_dir(),
        session_name: "host-auto".to_string(),
        session_type: SessionType::Hidden,
        initial_conversation: Conversation::empty(),
        user_message: Message::user()
            .with_text("Please produce docs/a.md and docs/b.md in parallel."),
        session_config: SessionConfig {
            id: "placeholder-auto".to_string(),
            schedule_id: None,
            max_turns: Some(4),
            retry_config: None,
        },
        provider,
        mode: HarnessMode::Conversation,
        delegation_mode: agime::agents::DelegationMode::Swarm,
        coordinator_execution_mode: CoordinatorExecutionMode::AutoSwarm,
        provider_turn_mode: ProviderTurnMode::Streaming,
        completion_surface_policy: CompletionSurfacePolicy::Conversation,
        write_scope: vec!["docs".to_string()],
        target_artifacts: vec!["docs/a.md".to_string(), "docs/b.md".to_string()],
        result_contract: vec!["docs/a.md".to_string(), "docs/b.md".to_string()],
        server_local_tool_names: Vec::new(),
        required_tool_prefixes: Vec::new(),
        parallelism_budget: Some(2),
        swarm_budget: Some(2),
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
            event_sink: sink,
            persistence,
        },
    )
    .await
    .expect("host should succeed");

    let saw_swarm_request = result.final_conversation.messages().iter().any(|message| {
        message.content.iter().any(|content| {
            content
                .as_tool_request()
                .and_then(|request| request.tool_call.as_ref().ok())
                .map(|tool_call| tool_call.name.as_ref() == "swarm")
                .unwrap_or(false)
        })
    });
    let saw_tool_response = result.final_conversation.messages().iter().any(|message| {
        message
            .content
            .iter()
            .any(|content| content.as_tool_response().map(|_| true).unwrap_or(false))
    });

    assert!(saw_swarm_request);
    assert!(saw_tool_response);

    std::env::remove_var("AGIME_ENABLE_NATIVE_SWARM_TOOL");
    std::env::remove_var("AGIME_ENABLE_SWARM_PLANNER_AUTO");
}
