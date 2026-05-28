//! Desktop harness host — full `run_harness_host` integration.
//!
//! Earlier phases of this file were a thin pass-through to `Agent::reply()`
//! plus an observation-only mirror that converted `AgentEvent` → mock
//! `HarnessControlEnvelope`. That model only ever surfaced a small subset of
//! the 26-type harness control protocol because most envelopes
//! (`Session.Started`, `Session.Finished`, `Worker.*`, `Tool.*`,
//! `Permission.*`, `Completion.StructuredPublished`, `Runtime.*`) are emitted
//! by the core `agime::agents::harness::run_harness_host` host loop, not by
//! the inner agent stream.
//!
//! This module now drives `run_harness_host` directly. The core entry point
//! consumes the `agent.reply()` stream itself (so it can interleave control
//! envelopes with model events) and returns only a final
//! `HarnessHostResult`. To keep the existing desktop SSE projection in
//! `routes/reply.rs` unchanged, we adapt the host into a stream-shaped
//! return value:
//!
//! - A `mpsc::channel<AgentEvent>` is bridged via
//!   [`crate::host_event_sink::DesktopHarnessEventSink`] so every
//!   `AgentEvent` the host observes becomes an item on the returned stream.
//! - `run_harness_host` runs in a detached `tokio::spawn`. It owns the agent
//!   reply for this turn and emits `HarnessControlEnvelope`s into the
//!   caller-provided `control_tx`.
//! - When the host finishes (success or error), the event-sender is dropped
//!   and the returned stream completes naturally; final-conversation /
//!   workspace artifact reconciliation runs in
//!   [`crate::host_persistence::DesktopHarnessPersistence`].
//!
//! Inputs that the team-server harness derives from richer session metadata
//! (session_source, recipe contract, swarm budget, …) are pinned to the
//! desktop's "chat" defaults below — these can grow as desktop UI features
//! land.
//!
//! All callers are gated by the `desktop_harness_host` cargo feature.
//!
//! SOURCE: crates/agime-team-server/src/agent/server_harness_host.rs at
//! commit 961109f. Keep in sync manually — see CLAUDE.md long-term
//! maintenance strategy.

#![cfg(feature = "desktop_harness_host")]

use std::sync::Arc;

use agime::agents::harness::host::{run_harness_host, HarnessHostDependencies, HarnessHostRequest};
use agime::agents::harness::{CompletionSurfacePolicy, HarnessMode};
use agime::agents::{Agent, AgentEvent, HarnessControlEnvelope, SessionConfig};
use agime::conversation::message::Message;
use agime::session::{SessionManager, SessionType};
use anyhow::{Context, Result};
use futures::stream::{BoxStream, StreamExt};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;

use crate::host_control_sink::DesktopHarnessControlSink;
use crate::host_event_sink::DesktopHarnessEventSink;
use crate::host_helpers::{
    delegation_mode_for_execution_mode, harness_mode_for_session_source,
    infer_coordinator_execution_mode, provider_turn_mode_for_session_source,
};
use crate::host_persistence::DesktopHarnessPersistence;
use crate::state::AppState;

/// Desktop session-source label. Mirrors the team-server contract: the
/// desktop chat surface is `chat`, which maps to `HarnessMode::Conversation`
/// + streaming provider turns by default.
const DESKTOP_SESSION_SOURCE: &str = "chat";

/// `run_harness_host` writes the harness's view of the conversation into a
/// per-turn *runtime* session that is distinct from the desktop's
/// *logical* (long-lived) session file. The persistence adapter mirrors
/// the final conversation back into the logical session in
/// `on_finished`. We use a small bounded channel so a slow SSE consumer
/// applies backpressure to the harness instead of allocating without
/// bound.
const EVENT_CHANNEL_CAPACITY: usize = 64;

pub struct DesktopHarnessHost {
    state: Arc<AppState>,
}

impl DesktopHarnessHost {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    #[allow(dead_code)]
    pub fn state(&self) -> &Arc<AppState> {
        &self.state
    }

    /// Run the chat turn through the core `run_harness_host` loop. Returns
    /// a stream that yields every `AgentEvent` the harness observed so the
    /// existing SSE projection in `routes/reply.rs` can keep its event
    /// loop. Harness control envelopes (the 26-type protocol) flow over
    /// `control_tx` independently.
    ///
    /// The provided `session_config.id` is treated as the **logical**
    /// session id. The harness allocates its own per-turn runtime session
    /// id internally; the persistence adapter mirrors the final
    /// conversation back to the logical session so the desktop UI keeps
    /// loading the full transcript on reload.
    pub async fn execute_chat_host_with_mirror(
        &self,
        agent: Arc<Agent>,
        user_message: Message,
        session_config: SessionConfig,
        cancel_token: Option<CancellationToken>,
        control_tx: mpsc::Sender<HarnessControlEnvelope>,
    ) -> Result<BoxStream<'static, Result<AgentEvent>>> {
        let logical_session_id = session_config.id.clone();

        let logical_session = SessionManager::get_session(&logical_session_id, true)
            .await
            .with_context(|| {
                format!(
                    "DesktopHarnessHost: failed to load logical session {}",
                    logical_session_id
                )
            })?;

        let working_dir = logical_session.working_dir.clone();
        let session_name = logical_session_id.clone();
        let initial_conversation = logical_session.conversation.clone().unwrap_or_default();
        let provider = agent.provider().await?;

        let user_message_text = collect_text(&user_message);
        let target_artifacts: Vec<String> = Vec::new();
        let result_contract: Vec<String> = Vec::new();
        let server_local_tool_names: Vec<String> = Vec::new();
        let required_tool_prefixes: Vec<String> = Vec::new();

        let coordinator_execution_mode = infer_coordinator_execution_mode(
            &user_message_text,
            &target_artifacts,
            &result_contract,
        );
        let delegation_mode = delegation_mode_for_execution_mode(coordinator_execution_mode);
        let harness_mode: HarnessMode = harness_mode_for_session_source(DESKTOP_SESSION_SOURCE);
        let provider_turn_mode =
            provider_turn_mode_for_session_source(DESKTOP_SESSION_SOURCE, harness_mode);

        let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(EVENT_CHANNEL_CAPACITY);
        let event_sink: Arc<dyn agime::agents::harness::HarnessEventSink> =
            Arc::new(DesktopHarnessEventSink::new(event_tx));
        let control_sink: Arc<dyn agime::agents::HarnessControlSink> =
            Arc::new(DesktopHarnessControlSink::new(control_tx));
        let persistence: Arc<dyn agime::agents::harness::HarnessPersistenceAdapter> =
            Arc::new(DesktopHarnessPersistence::new(self.state.clone()));
        let task_runtime = self.state.task_runtime().await.inner();

        let dependencies = HarnessHostDependencies {
            agent: agent.clone(),
            event_sink,
            control_sink,
            persistence,
        };

        let request = HarnessHostRequest {
            logical_session_id: logical_session_id.clone(),
            working_dir,
            session_name,
            session_type: SessionType::Default,
            initial_conversation,
            user_message,
            session_config,
            provider,
            mode: harness_mode,
            delegation_mode,
            coordinator_execution_mode,
            provider_turn_mode,
            completion_surface_policy: CompletionSurfacePolicy::Conversation,
            write_scope: Vec::new(),
            target_artifacts,
            result_contract,
            server_local_tool_names,
            required_tool_prefixes,
            parallelism_budget: None,
            swarm_budget: None,
            validation_mode: false,
            worker_extensions: Vec::new(),
            initial_context_runtime_state: None,
            task_runtime: Some(task_runtime),
            system_prompt_override: None,
            system_prompt_extras: Vec::new(),
            extensions: Vec::new(),
            final_output: None,
            cancel_token,
        };

        // Drive `run_harness_host` to completion in the background. When it
        // finishes (success or error), the event_tx held inside the
        // event_sink Arc is dropped by the host's cleanup, which closes
        // the channel and lets the stream we hand back to the caller end
        // cleanly.
        drop(tokio::spawn(async move {
            match run_harness_host(request, dependencies).await {
                Ok(result) => {
                    tracing::info!(
                        logical_session_id = %result.runtime_session_id,
                        events_emitted = result.events_emitted,
                        "DesktopHarnessHost: run_harness_host completed"
                    );
                }
                Err(err) => {
                    tracing::warn!("DesktopHarnessHost: run_harness_host failed: {}", err);
                }
            }
        }));

        let stream = ReceiverStream::new(event_rx).map(Ok::<AgentEvent, anyhow::Error>);
        Ok(stream.boxed())
    }
}

fn collect_text(message: &Message) -> String {
    use agime::conversation::message::MessageContent;
    let mut buf = String::new();
    for content in &message.content {
        if let MessageContent::Text(text) = content {
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(&text.text);
        }
    }
    buf
}
