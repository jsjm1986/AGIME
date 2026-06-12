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
//! (session_source, recipe contract, …) are pinned to the desktop's "chat"
//! defaults below — these can grow as desktop UI features land. Delegation
//! budgets (`parallelism_budget` / `swarm_budget`) default to a modest fan-out
//! and are overridable via `AGIME_DESKTOP_PARALLELISM_BUDGET` /
//! `AGIME_DESKTOP_SWARM_BUDGET` (set to `0` to disable delegation entirely).
//!
//! All callers are gated by the `desktop_harness_host` cargo feature.
//!
//! SOURCE: crates/agime-team-server/src/agent/server_harness_host.rs at
//! commit 961109f. Keep in sync manually — see CLAUDE.md long-term
//! maintenance strategy.

use std::sync::Arc;

use agime::agents::harness::host::{run_harness_host, HarnessHostDependencies, HarnessHostRequest};
use agime::agents::harness::{native_swarm_tool_enabled, CompletionSurfacePolicy, HarnessMode};
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

/// Default per-turn cap on concurrently running coordinator workers. The
/// coordinator/planner still decides *whether* to delegate; this only caps
/// the fan-out when it does. `0` disables delegation (matches the previous
/// `None` behavior). Override with `AGIME_DESKTOP_PARALLELISM_BUDGET`.
const DEFAULT_PARALLELISM_BUDGET: u32 = 2;

/// Default per-turn cap on swarm workers. Same semantics as
/// [`DEFAULT_PARALLELISM_BUDGET`]; override with `AGIME_DESKTOP_SWARM_BUDGET`.
const DEFAULT_SWARM_BUDGET: u32 = 2;

/// Read a `u32` budget from `env_key`, falling back to `default`. A value of
/// `0` (from either source) maps to `None`, which disables that form of
/// delegation in the core harness. Any unparseable value falls back to the
/// default so a typo can never silently disable the capability.
fn budget_from_env(env_key: &str, default: u32) -> Option<u32> {
    resolve_budget(std::env::var(env_key).ok().as_deref(), default)
}

/// Pure budget resolution, split out from [`budget_from_env`] so it can be
/// unit-tested without mutating process-wide environment state.
fn resolve_budget(raw: Option<&str>, default: u32) -> Option<u32> {
    let value = raw
        .and_then(|raw| raw.trim().parse::<u32>().ok())
        .unwrap_or(default);
    (value > 0).then_some(value)
}

/// Env flag (set-if-unset to `1` for desktop in `commands/agent.rs`) that
/// controls whether the delegation guidance overlay is appended. `0` reverts
/// to the pre-overlay behavior without a rebuild.
const DELEGATION_GUIDANCE_ENV: &str = "AGIME_DESKTOP_DELEGATION_GUIDANCE";

/// Desktop-only delegation guidance, appended to the system prompt via
/// `system_prompt_extras`. The core `system.md` does not teach delegation and
/// explicitly tells the model not to claim a delegation power unless a runtime
/// overlay makes it available — so without this overlay a desktop model in
/// Conversation mode almost never spawns a subagent/swarm even though the
/// tools are registered and (in non-Chat permission modes) executable.
///
/// Wording is deliberately conditional with an explicit negative to avoid
/// over-delegation on trivial turns, and uses "when available" so it stays
/// truthful when the subagent tool is stripped (non-Auto mode / gemini models).
const DELEGATION_GUIDANCE_SUBAGENT: &str = "When a `subagent` tool is available and the task is genuinely multi-step or decomposable into independent pieces, prefer delegating those pieces to subagents rather than doing everything inline. 当任务确属多步或可拆分为独立子任务时，优先用 subagent 委派，而不是全部内联处理。";

const DELEGATION_GUIDANCE_SWARM: &str = "When a `swarm` tool is available and the work splits into two or more independent targets that can run in parallel, use it to fan out one worker per target. 当工作可拆成两个及以上可并行的独立目标时，用 swarm 为每个目标分派一个 worker。";

const DELEGATION_GUIDANCE_RESTRAINT: &str = "For simple, single-step, or quick-answer requests, do NOT delegate — handle them directly. 对简单、单步或可直接回答的请求，不要委派，直接处理。";

/// Build the delegation guidance `system_prompt_extras` for the desktop turn.
/// Pure (env flag passed in) so it can be unit-tested without touching process
/// environment. Returns at most one combined instruction string.
///
/// - `guidance_on == false` (env `=0`) → no overlay.
/// - both budgets `None` (delegation fully disabled) → no overlay; never
///   advertise a capability that won't run.
/// - otherwise: include the subagent sentence when `parallelism_budget` is set,
///   the swarm sentence when `swarm_budget` is set AND native swarm is enabled,
///   always followed by the restraint sentence.
fn delegation_guidance_extras(
    guidance_on: bool,
    parallelism_budget: Option<u32>,
    swarm_budget: Option<u32>,
    swarm_tool_enabled: bool,
) -> Vec<String> {
    if !guidance_on {
        return Vec::new();
    }
    let subagent_on = parallelism_budget.is_some();
    let swarm_on = swarm_budget.is_some() && swarm_tool_enabled;
    if !subagent_on && !swarm_on {
        return Vec::new();
    }

    let mut parts: Vec<&str> = Vec::new();
    if subagent_on {
        parts.push(DELEGATION_GUIDANCE_SUBAGENT);
    }
    if swarm_on {
        parts.push(DELEGATION_GUIDANCE_SWARM);
    }
    parts.push(DELEGATION_GUIDANCE_RESTRAINT);
    vec![parts.join("\n\n")]
}

/// Read the delegation-guidance flag from the environment. Defaults to enabled
/// (mirrors the desktop set-if-unset in `commands/agent.rs`); only an explicit
/// falsey value disables it.
fn delegation_guidance_enabled() -> bool {
    match std::env::var(DELEGATION_GUIDANCE_ENV) {
        Ok(value) => !matches!(value.trim(), "0" | "false" | "off" | "no"),
        Err(_) => true,
    }
}

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

        let parallelism_budget =
            budget_from_env("AGIME_DESKTOP_PARALLELISM_BUDGET", DEFAULT_PARALLELISM_BUDGET);
        let swarm_budget = budget_from_env("AGIME_DESKTOP_SWARM_BUDGET", DEFAULT_SWARM_BUDGET);
        let system_prompt_extras = delegation_guidance_extras(
            delegation_guidance_enabled(),
            parallelism_budget,
            swarm_budget,
            native_swarm_tool_enabled(),
        );

        let request = HarnessHostRequest {
            logical_session_id: logical_session_id.clone(),
            working_dir,
            session_name,
            session_type: SessionType::User,
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
            parallelism_budget,
            swarm_budget,
            validation_mode: false,
            worker_extensions: Vec::new(),
            initial_context_runtime_state: None,
            task_runtime: Some(task_runtime),
            system_prompt_override: None,
            system_prompt_extras,
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

#[cfg(test)]
mod tests {
    use super::{
        delegation_guidance_extras, resolve_budget, DELEGATION_GUIDANCE_RESTRAINT,
        DELEGATION_GUIDANCE_SUBAGENT, DELEGATION_GUIDANCE_SWARM, DEFAULT_PARALLELISM_BUDGET,
        DEFAULT_SWARM_BUDGET,
    };

    #[test]
    fn resolve_budget_uses_default_when_unset() {
        assert_eq!(resolve_budget(None, DEFAULT_PARALLELISM_BUDGET), Some(2));
        assert_eq!(resolve_budget(None, DEFAULT_SWARM_BUDGET), Some(2));
    }

    #[test]
    fn resolve_budget_zero_disables_delegation() {
        assert_eq!(resolve_budget(Some("0"), DEFAULT_PARALLELISM_BUDGET), None);
    }

    #[test]
    fn resolve_budget_parses_override() {
        assert_eq!(resolve_budget(Some("4"), DEFAULT_SWARM_BUDGET), Some(4));
        assert_eq!(resolve_budget(Some("  3  "), 2), Some(3));
    }

    #[test]
    fn resolve_budget_falls_back_on_garbage() {
        // A typo must never silently disable the capability.
        assert_eq!(
            resolve_budget(Some("abc"), DEFAULT_PARALLELISM_BUDGET),
            Some(2)
        );
        assert_eq!(resolve_budget(Some(""), DEFAULT_SWARM_BUDGET), Some(2));
    }

    #[test]
    fn guidance_present_with_budgets_and_flag_on() {
        let extras = delegation_guidance_extras(true, Some(2), Some(2), true);
        assert_eq!(extras.len(), 1);
        let text = &extras[0];
        assert!(text.contains(DELEGATION_GUIDANCE_SUBAGENT));
        assert!(text.contains(DELEGATION_GUIDANCE_SWARM));
        assert!(text.contains(DELEGATION_GUIDANCE_RESTRAINT));
    }

    #[test]
    fn guidance_empty_when_flag_off() {
        assert!(delegation_guidance_extras(false, Some(2), Some(2), true).is_empty());
    }

    #[test]
    fn guidance_subagent_only_when_swarm_disabled() {
        // Swarm tool disabled (env off) → no swarm sentence even with a budget.
        let extras = delegation_guidance_extras(true, Some(2), Some(2), false);
        assert_eq!(extras.len(), 1);
        let text = &extras[0];
        assert!(text.contains(DELEGATION_GUIDANCE_SUBAGENT));
        assert!(!text.contains(DELEGATION_GUIDANCE_SWARM));
        assert!(text.contains(DELEGATION_GUIDANCE_RESTRAINT));
    }

    #[test]
    fn guidance_subagent_only_when_swarm_budget_none() {
        let extras = delegation_guidance_extras(true, Some(2), None, true);
        assert_eq!(extras.len(), 1);
        assert!(!extras[0].contains(DELEGATION_GUIDANCE_SWARM));
    }

    #[test]
    fn guidance_empty_when_all_budgets_disabled() {
        // Delegation fully off → advertise nothing.
        assert!(delegation_guidance_extras(true, None, None, true).is_empty());
    }
}
