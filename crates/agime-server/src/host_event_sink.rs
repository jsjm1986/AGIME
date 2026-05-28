//! Desktop implementation of [`agime::agents::harness::HarnessEventSink`].
//!
//! `run_harness_host` consumes the inner `agent.reply()` stream itself and
//! does not return it to the caller. To keep the desktop SSE projection
//! model intact (which expects to fan `AgentEvent`s out as
//! `MessageEvent::Message` / `MessageEvent::Notification` etc.), the
//! desktop sink forwards every observed `AgentEvent` over a
//! `tokio::sync::mpsc::Sender<AgentEvent>` so `routes/reply.rs` can keep
//! its existing event loop.
//!
//! Channel-closed errors are downgraded to `Ok(())` because the SSE side
//! may have torn down before the harness has emitted its final event, and
//! we never want that to abort the harness loop.

use agime::agents::harness::HarnessEventSink;
use agime::agents::{Agent, AgentEvent};
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct DesktopHarnessEventSink {
    tx: mpsc::Sender<AgentEvent>,
}

impl DesktopHarnessEventSink {
    pub fn new(tx: mpsc::Sender<AgentEvent>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl HarnessEventSink for DesktopHarnessEventSink {
    async fn handle(
        &self,
        _logical_session_id: &str,
        _runtime_session_id: &str,
        _agent: &Agent,
        event: &AgentEvent,
    ) -> Result<()> {
        match self.tx.send(event.clone()).await {
            Ok(()) => Ok(()),
            Err(_closed) => {
                tracing::debug!("DesktopHarnessEventSink: receiver dropped before event delivered");
                Ok(())
            }
        }
    }
}
