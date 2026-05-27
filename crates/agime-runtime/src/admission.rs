//! Execution-slot admission control.
//!
//! Migrated from `agime-team-server::agent::direct_host_admission`. The team
//! server wraps this with an adapter that implements
//! [`ExecutionSlotProvider`] on its `AgentService`.

use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

const DIRECT_HOST_QUEUE_POLL_MS: u64 = 500;

/// Outcome of trying to acquire one execution slot for an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionSlotAcquireOutcome {
    Acquired,
    Saturated,
}

/// Surface that owns per-agent execution-slot bookkeeping.
///
/// Team server: implemented on the Mongo-backed `AgentService` (atomic
/// `find_one_and_update` increments `active_execution_slots`).
/// Desktop server: typically a single-process counter behind a `Mutex`.
#[async_trait]
pub trait ExecutionSlotProvider: Send + Sync {
    async fn try_acquire_execution_slot(
        &self,
        agent_id: &str,
    ) -> Result<ExecutionSlotAcquireOutcome>;

    async fn release_execution_slot(&self, agent_id: &str) -> Result<()>;
}

/// Block until an execution slot becomes available, or the cancel token fires.
///
/// Polls every [`DIRECT_HOST_QUEUE_POLL_MS`] ms, the same cadence the
/// team-server runs at today.
pub async fn wait_for_execution_slot(
    provider: &dyn ExecutionSlotProvider,
    agent_id: &str,
    cancel_token: &CancellationToken,
) -> Result<()> {
    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                return Err(anyhow!("Direct harness execution cancelled while queued"));
            }
            _ = tokio::time::sleep(Duration::from_millis(DIRECT_HOST_QUEUE_POLL_MS)) => {}
        }

        match provider
            .try_acquire_execution_slot(agent_id)
            .await
            .map_err(|error| anyhow!("Failed to acquire agent execution slot: {}", error))?
        {
            ExecutionSlotAcquireOutcome::Acquired => return Ok(()),
            ExecutionSlotAcquireOutcome::Saturated => continue,
        }
    }
}
