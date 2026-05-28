//! Desktop harness host — Phase 1 + Phase 2.
//!
//! Phase 1 was a bit-for-bit pass-through to `Agent::reply()`. Phase 2 keeps
//! that pass-through behavior intact for the existing `BoxStream` consumer
//! (`routes/reply.rs`) but adds an optional dual-channel side-fan-out: every
//! `AgentEvent` flowing through is mirrored as a `HarnessControlEnvelope`
//! sequence on a caller-provided broadcaster, so SSE clients can subscribe to
//! the harness control protocol without having to replay full agent messages.
//!
//! This mirror layer is intentionally observation-only — it never alters the
//! agent stream and never blocks on the broadcaster. If the receiver hangs
//! up, mirror events are silently dropped.
//!
//! Phases 3+ build on this base:
//! - Phase 3: capability policy / permission engine
//! - Phase 4: execution admission control
//! - Phase 5: agent task payload + tool dispatch
//! - Phase 6: document analysis path
//! - Phase 7: workspace service
//!
//! All callers are gated by the `desktop_harness_host` cargo feature.
//!
//! SOURCE: crates/agime-team-server/src/agent/server_harness_host.rs at
//! commit 961109f. Keep in sync manually — see CLAUDE.md long-term
//! maintenance strategy.

#![cfg(feature = "desktop_harness_host")]

use std::sync::Arc;

use agime::agents::{Agent, AgentEvent, HarnessControlEnvelope, SessionConfig};
use agime::conversation::message::Message;
use anyhow::Result;
use futures::stream::{BoxStream, StreamExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::host_stream::StreamBroadcaster;
use crate::state::AppState;

/// Thin shell around `Agent::reply()` that future phases will grow into the
/// full team-server-style host. Today it owns the parent `AppState` so future
/// phases (capability policy, admission, workspace) can pull session-scoped
/// state from a single source without touching every call site.
pub struct DesktopHarnessHost {
    state: Arc<AppState>,
}

impl DesktopHarnessHost {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    /// Reference to the parent app state. Lets future phases pull session
    /// overrides / workspace context / capability policy from a single source.
    #[allow(dead_code)]
    pub fn state(&self) -> &Arc<AppState> {
        &self.state
    }

    /// Phase 1 entry point: pass-through to `Agent::reply()`. Kept for callers
    /// that don't need the dual-channel mirror.
    pub async fn execute_chat_host<'a>(
        &self,
        agent: &'a Agent,
        user_message: Message,
        session_config: SessionConfig,
        cancel_token: Option<CancellationToken>,
    ) -> Result<BoxStream<'a, Result<AgentEvent>>> {
        agent
            .reply(user_message, session_config, cancel_token)
            .await
    }

    /// Phase 2 entry point: pass-through to `Agent::reply()` *plus* a side
    /// channel of `HarnessControlEnvelope`s mirroring each `AgentEvent`.
    ///
    /// Design constraints:
    /// - The agent stream returned to the caller must be byte-for-byte
    ///   identical to what `Agent::reply()` produces (no filtering, no
    ///   reordering).
    /// - Mirror events are best-effort: if the receiver is slow or dropped,
    ///   we never apply backpressure to the inner agent stream.
    /// - The mirror task ends when the agent stream ends or the cancel token
    ///   is triggered.
    pub async fn execute_chat_host_with_mirror<'a>(
        &self,
        agent: &'a Agent,
        user_message: Message,
        session_config: SessionConfig,
        cancel_token: Option<CancellationToken>,
        control_tx: mpsc::Sender<HarnessControlEnvelope>,
    ) -> Result<BoxStream<'a, Result<AgentEvent>>> {
        let logical_session_id = session_config.id.clone();
        let runtime_session_id = session_config.id.clone();

        let inner = agent
            .reply(user_message, session_config, cancel_token)
            .await?;

        let broadcaster = StreamBroadcaster::Chat(control_tx);

        let mirrored = inner.then(move |event_result| {
            let logical = logical_session_id.clone();
            let runtime = runtime_session_id.clone();
            // Move broadcaster into closure by clone-by-arc isn't needed —
            // mpsc::Sender is cheap to clone, and StreamBroadcaster::Chat
            // wraps a sender directly. We rebuild a fresh StreamBroadcaster
            // each tick to keep the closure FnMut-friendly.
            let tx = match &broadcaster {
                StreamBroadcaster::Chat(tx) => tx.clone(),
                StreamBroadcaster::Silent => {
                    // unreachable in this code path; preserved for symmetry.
                    return async move { event_result }.boxed();
                }
            };
            async move {
                if let Ok(event) = &event_result {
                    let local = StreamBroadcaster::Chat(tx);
                    local.emit_agent_event(&logical, &runtime, event).await;
                }
                event_result
            }
            .boxed()
        });

        Ok(mirrored.boxed())
    }
}
