//! Desktop implementation of [`agime::agents::harness::HarnessControlSink`].
//!
//! Forwards every [`HarnessControlEnvelope`] produced inside
//! `run_harness_host` to a `tokio::sync::mpsc::Sender` so that
//! `routes/reply.rs` can fan it out to the SSE stream as
//! `MessageEvent::HarnessControl`.
//!
//! Channel-closed errors are downgraded to `Ok(())` because the SSE side
//! may have torn down before the harness has emitted its final
//! `Session.Finished` envelope, and we never want that to abort the
//! harness loop.

use agime::agents::harness::HarnessControlSink;
use agime::agents::HarnessControlEnvelope;
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct DesktopHarnessControlSink {
    tx: mpsc::Sender<HarnessControlEnvelope>,
}

impl DesktopHarnessControlSink {
    pub fn new(tx: mpsc::Sender<HarnessControlEnvelope>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl HarnessControlSink for DesktopHarnessControlSink {
    async fn handle(&self, envelope: &HarnessControlEnvelope) -> Result<()> {
        match self.tx.send(envelope.clone()).await {
            Ok(()) => Ok(()),
            Err(_closed) => {
                tracing::debug!(
                    "DesktopHarnessControlSink: receiver dropped before envelope delivered"
                );
                Ok(())
            }
        }
    }
}
