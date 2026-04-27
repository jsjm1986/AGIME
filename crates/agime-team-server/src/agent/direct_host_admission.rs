use anyhow::{anyhow, Result};
use tokio_util::sync::CancellationToken;

use super::service_mongo::{AgentService, ExecutionSlotAcquireOutcome};

const DIRECT_HOST_QUEUE_POLL_MS: u64 = 500;

pub async fn wait_for_execution_slot(
    agent_service: &AgentService,
    agent_id: &str,
    cancel_token: &CancellationToken,
) -> Result<()> {
    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                return Err(anyhow!("Direct harness execution cancelled while queued"));
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(DIRECT_HOST_QUEUE_POLL_MS)) => {}
        }

        match agent_service
            .try_acquire_execution_slot(agent_id)
            .await
            .map_err(|error| anyhow!("Failed to acquire agent execution slot: {}", error))?
        {
            ExecutionSlotAcquireOutcome::Acquired => return Ok(()),
            ExecutionSlotAcquireOutcome::Saturated => continue,
        }
    }
}
