//! Execution-slot admission control (team-server side).
//!
//! Delegates to [`agime_runtime::admission`]; this file only adapts the team
//! server's `AgentService` (Mongo-backed) to the runtime's
//! `ExecutionSlotProvider` trait so the call sites keep their existing
//! `&AgentService` signature.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use agime_runtime::admission::{
    self, ExecutionSlotAcquireOutcome as RtExecutionSlotAcquireOutcome, ExecutionSlotProvider,
};

use super::service_mongo::{AgentService, ExecutionSlotAcquireOutcome};

struct AgentServiceSlotAdapter<'a> {
    agent_service: &'a AgentService,
}

#[async_trait]
impl<'a> ExecutionSlotProvider for AgentServiceSlotAdapter<'a> {
    async fn try_acquire_execution_slot(
        &self,
        agent_id: &str,
    ) -> Result<RtExecutionSlotAcquireOutcome> {
        match self
            .agent_service
            .try_acquire_execution_slot(agent_id)
            .await
            .map_err(|error| anyhow!("{}", error))?
        {
            ExecutionSlotAcquireOutcome::Acquired => Ok(RtExecutionSlotAcquireOutcome::Acquired),
            ExecutionSlotAcquireOutcome::Saturated => Ok(RtExecutionSlotAcquireOutcome::Saturated),
        }
    }

    async fn release_execution_slot(&self, agent_id: &str) -> Result<()> {
        self.agent_service
            .release_execution_slot(agent_id)
            .await
            .map_err(|error| anyhow!("{}", error))
    }
}

pub async fn wait_for_execution_slot(
    agent_service: &AgentService,
    agent_id: &str,
    cancel_token: &CancellationToken,
) -> Result<()> {
    let adapter = AgentServiceSlotAdapter { agent_service };
    admission::wait_for_execution_slot(&adapter, agent_id, cancel_token).await
}
