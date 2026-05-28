//! Desktop harness host — Phase 2: dual-channel stream broadcaster + 12 mirror builders.
//!
//! Mirror of `crates/agime-team-server/src/agent/server_harness_host.rs` mirror
//! builders (lines ~1419-1547). Whereas the team-server version builds these as
//! private free functions inside the host file, the desktop version exposes
//! them here as a thin re-export over `agime::agents::build_*_control_message`,
//! so we never duplicate the protocol shape.
//!
//! The `StreamBroadcaster` enum is the desktop-side equivalent of the
//! team-server's chat/control fan-out: in `Chat` mode it forwards a
//! `HarnessControlEnvelope` to the SSE channel; in `Silent` mode it drops the
//! event (used by document-analysis-style flows in later phases).
//!
//! All callers are gated by the `desktop_harness_host` cargo feature so the
//! default desktop build (feature off) sees zero behavior change.
//!
//! SOURCE: crates/agime-team-server/src/agent/server_harness_host.rs
//! around lines 1419-1741 at commit 961109f. Keep in sync manually — see
//! CLAUDE.md long-term maintenance strategy.

#![cfg(feature = "desktop_harness_host")]

use agime::agents::{
    build_permission_requested_control_message, build_permission_resolved_control_message,
    build_permission_timed_out_control_message, build_tool_finished_control_message,
    build_tool_started_control_message, build_worker_finished_control_message,
    build_worker_followup_requested_control_message, build_worker_idle_control_message,
    build_worker_progress_control_message, build_worker_started_control_message,
    control_messages_for_agent_event, control_sequencer_for_session, control_timestamp_ms,
    AgentEvent, HarnessControlEnvelope, HarnessControlMessage, RuntimeControlEvent, TaskKind,
    TaskSnapshot, WorkerAttemptIdentity,
};
use tokio::sync::mpsc;

/// Dual-channel stream broadcaster. Mirrors team-server's chat-vs-silent
/// branching but stays SSE-only (no Mongo persistence side-channel).
///
/// `Chat(tx)` forwards each envelope as a `MessageEvent::HarnessControl`
/// SSE frame on the chat channel. `Silent` is a no-op sink used by
/// document-analysis-style flows that want side-effect-free observation.
pub enum StreamBroadcaster {
    Chat(mpsc::Sender<HarnessControlEnvelope>),
    Silent,
}

impl StreamBroadcaster {
    /// Send an envelope on the chat channel; silently drop in `Silent` mode.
    /// If the receiver hung up, log and continue — the caller decides whether
    /// to propagate cancellation.
    pub async fn emit(&self, envelope: HarnessControlEnvelope) {
        match self {
            StreamBroadcaster::Chat(tx) => {
                if tx.send(envelope).await.is_err() {
                    tracing::debug!("harness control receiver hung up");
                }
            }
            StreamBroadcaster::Silent => {}
        }
    }

    /// Wrap a raw payload into a sequenced envelope and emit it.
    ///
    /// `logical_session_id` is the session id the desktop tracks (same as
    /// `runtime_session_id` for desktop today — team-server splits these for
    /// dual-routing, desktop does not). If a per-session sequencer has been
    /// registered (via `register_harness_control_sequencer` from the harness
    /// run loop), we use it so the desktop sequence numbering stays in lock
    /// step with the inner harness; otherwise we fall back to a sequence of 0
    /// and a fresh timestamp, matching the team-server fallback path.
    pub async fn emit_payload(
        &self,
        logical_session_id: &str,
        runtime_session_id: &str,
        payload: HarnessControlMessage,
    ) {
        let envelope = if let Some(sequencer) = control_sequencer_for_session(runtime_session_id) {
            sequencer.next(payload)
        } else {
            HarnessControlEnvelope::new(
                logical_session_id,
                runtime_session_id,
                0,
                control_timestamp_ms(),
                payload,
            )
        };
        self.emit(envelope).await;
    }

    /// Translate an `AgentEvent` to zero or more control messages and emit
    /// them all on the chat channel. This is the single integration point
    /// that `DesktopHarnessHost::execute_chat_host` uses to keep the SSE
    /// stream observable as harness control events.
    pub async fn emit_agent_event(
        &self,
        logical_session_id: &str,
        runtime_session_id: &str,
        event: &AgentEvent,
    ) {
        for msg in control_messages_for_agent_event(event) {
            self.emit_payload(logical_session_id, runtime_session_id, msg)
                .await;
        }
    }
}

// ---------------------------------------------------------------------------
// 12 mirror builders — thin shims over `agime::agents::build_*_control_message`
//
// These exist so future desktop callers can build mirror events at the same
// granularity as the team-server's `*_mirror` helpers without re-deriving the
// argument shapes. They are intentionally trivial — keep them aligned with the
// team-server signatures so a side-by-side review stays cheap.
// ---------------------------------------------------------------------------

pub fn worker_started_mirror(
    snapshot: &TaskSnapshot,
    _attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_worker_started_control_message(snapshot)
}

pub fn worker_progress_mirror(
    task_id: String,
    message: String,
    percent: Option<u8>,
) -> HarnessControlMessage {
    build_worker_progress_control_message(task_id, message, percent)
}

pub fn worker_followup_mirror(
    task_id: String,
    kind: String,
    reason: String,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_worker_followup_requested_control_message(task_id, kind, reason, attempt_identity)
}

pub fn worker_idle_mirror(
    task_id: String,
    message: String,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_worker_idle_control_message(task_id, message, attempt_identity)
}

pub fn permission_requested_mirror(
    task_id: String,
    worker_name: Option<String>,
    tool_name: String,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_permission_requested_control_message(task_id, tool_name, worker_name, attempt_identity)
}

pub fn permission_resolved_mirror(
    task_id: String,
    worker_name: Option<String>,
    tool_name: String,
    decision: String,
    source: Option<String>,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_permission_resolved_control_message(
        task_id,
        tool_name,
        decision,
        source,
        None,
        worker_name,
        attempt_identity,
    )
}

pub fn permission_timed_out_mirror(
    task_id: String,
    worker_name: Option<String>,
    tool_name: String,
    timeout_ms: u64,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_permission_timed_out_control_message(
        task_id,
        tool_name,
        timeout_ms,
        worker_name,
        attempt_identity,
    )
}

pub fn worker_finished_mirror(
    task_id: String,
    kind: TaskKind,
    status: &str,
    summary: String,
    produced_delta: bool,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    build_worker_finished_control_message(
        task_id,
        kind,
        status,
        summary,
        produced_delta,
        attempt_identity,
    )
}

pub fn tool_started_mirror(request_id: String, tool_name: String) -> HarnessControlMessage {
    build_tool_started_control_message(request_id, tool_name)
}

pub fn tool_finished_mirror(
    request_id: String,
    tool_name: String,
    success: bool,
    content: String,
    duration_ms: Option<u64>,
) -> HarnessControlMessage {
    build_tool_finished_control_message(request_id, tool_name, success, Some(content), duration_ms)
}

/// Build a compaction observed event — desktop today doesn't expose a
/// `StreamEvent` enum like team-server, so callers feed in the raw fields.
pub fn compaction_mirror(
    strategy: Option<String>,
    reason: Option<String>,
    before_tokens: Option<usize>,
    after_tokens: Option<usize>,
    phase: Option<String>,
) -> HarnessControlMessage {
    HarnessControlMessage::Runtime(RuntimeControlEvent::CompactionObserved {
        strategy,
        reason,
        before_tokens,
        after_tokens,
        phase,
    })
}
