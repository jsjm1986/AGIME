//! Desktop harness host — Phase 4: execution admission control.
//!
//! Mirror of `crates/agime-runtime/src/admission.rs` (~64 lines) plus a
//! desktop-flavored in-memory implementation of the `ExecutionSlotProvider`
//! trait. The team-server backs slot bookkeeping with an atomic Mongo
//! `find_one_and_update`; the desktop is a single-process binary so a simple
//! `Arc<Mutex<HashMap<agent_id, count>>>` is sufficient.
//!
//! The full `execution_admission.rs` (~437 lines) covers persistent queue
//! transitions and surface propagation that don't apply to the desktop's
//! single-process model — those would require a `TaskQueueRepository` backed
//! by SQLite or similar, which lands in Phase 7. For now the desktop ships
//! with the slot-only fast path: agents can run as many concurrent turns as
//! their `max_slots` configures, and over-cap callers wait via
//! `wait_for_execution_slot`.
//!
//! Default policy: `max_slots = 1` (today's desktop is single-turn-per-agent).
//! Callers that want concurrency raise it via `DesktopExecutionSlotProvider::new`.
//!
//! All callers are gated by the `desktop_harness_host` cargo feature.
//!
//! SOURCE: crates/agime-runtime/src/admission.rs +
//! crates/agime-runtime/src/execution_admission.rs at commit 961109f.

#![allow(dead_code)]
// Phase-4 verbatim runtime mirror: the desktop reply path will exercise this
// surface in Milestone B; until then `--all-targets` lint sees several items
// as unused.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

const DIRECT_HOST_QUEUE_POLL_MS: u64 = 500;

/// Outcome of trying to acquire one execution slot for an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionSlotAcquireOutcome {
    Acquired,
    Saturated,
}

/// Per-agent execution-slot bookkeeping. Mirrors the team-server's trait
/// shape so a future Phase 7 SQLite-backed implementation can drop in
/// without touching call sites.
#[async_trait]
pub trait ExecutionSlotProvider: Send + Sync {
    async fn try_acquire_execution_slot(
        &self,
        agent_id: &str,
    ) -> Result<ExecutionSlotAcquireOutcome>;

    async fn release_execution_slot(&self, agent_id: &str) -> Result<()>;
}

/// In-memory desktop implementation: tracks `(active, max)` per agent in a
/// shared `HashMap` behind a tokio `Mutex`. Default `max_slots` is 1 to match
/// today's single-turn behavior — bump for power users via `with_max_slots`.
pub struct DesktopExecutionSlotProvider {
    inner: Arc<Mutex<HashMap<String, AgentSlotState>>>,
    default_max: u32,
}

#[derive(Debug, Clone, Copy)]
struct AgentSlotState {
    active: u32,
    max: u32,
}

impl Default for DesktopExecutionSlotProvider {
    fn default() -> Self {
        Self::new(1)
    }
}

impl DesktopExecutionSlotProvider {
    pub fn new(default_max: u32) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            default_max: default_max.max(1),
        }
    }

    /// Override the per-agent slot ceiling. Existing entries keep their
    /// previous max; new agents pick this up.
    #[allow(dead_code)]
    pub fn with_max_slots(default_max: u32) -> Self {
        Self::new(default_max)
    }

    /// Read the current `(active, max)` for diagnostics. Returns `(0, default)`
    /// if the agent hasn't acquired a slot yet.
    pub async fn snapshot(&self, agent_id: &str) -> (u32, u32) {
        let map = self.inner.lock().await;
        match map.get(agent_id) {
            Some(state) => (state.active, state.max),
            None => (0, self.default_max),
        }
    }
}

#[async_trait]
impl ExecutionSlotProvider for DesktopExecutionSlotProvider {
    async fn try_acquire_execution_slot(
        &self,
        agent_id: &str,
    ) -> Result<ExecutionSlotAcquireOutcome> {
        let mut map = self.inner.lock().await;
        let entry = map.entry(agent_id.to_string()).or_insert(AgentSlotState {
            active: 0,
            max: self.default_max,
        });
        if entry.active >= entry.max {
            Ok(ExecutionSlotAcquireOutcome::Saturated)
        } else {
            entry.active += 1;
            Ok(ExecutionSlotAcquireOutcome::Acquired)
        }
    }

    async fn release_execution_slot(&self, agent_id: &str) -> Result<()> {
        let mut map = self.inner.lock().await;
        if let Some(entry) = map.get_mut(agent_id) {
            entry.active = entry.active.saturating_sub(1);
        }
        Ok(())
    }
}

/// Block until an execution slot becomes available, or the cancel token
/// fires. Polls every [`DIRECT_HOST_QUEUE_POLL_MS`] ms — same cadence the
/// team-server runs at.
pub async fn wait_for_execution_slot(
    provider: &dyn ExecutionSlotProvider,
    agent_id: &str,
    cancel_token: &CancellationToken,
) -> Result<()> {
    loop {
        match provider
            .try_acquire_execution_slot(agent_id)
            .await
            .map_err(|error| anyhow!("Failed to acquire agent execution slot: {}", error))?
        {
            ExecutionSlotAcquireOutcome::Acquired => return Ok(()),
            ExecutionSlotAcquireOutcome::Saturated => {}
        }

        tokio::select! {
            _ = cancel_token.cancelled() => {
                return Err(anyhow!("Direct harness execution cancelled while queued"));
            }
            _ = tokio::time::sleep(Duration::from_millis(DIRECT_HOST_QUEUE_POLL_MS)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquire_until_saturated() {
        let provider = DesktopExecutionSlotProvider::new(2);
        assert_eq!(
            provider.try_acquire_execution_slot("a").await.unwrap(),
            ExecutionSlotAcquireOutcome::Acquired
        );
        assert_eq!(
            provider.try_acquire_execution_slot("a").await.unwrap(),
            ExecutionSlotAcquireOutcome::Acquired
        );
        assert_eq!(
            provider.try_acquire_execution_slot("a").await.unwrap(),
            ExecutionSlotAcquireOutcome::Saturated
        );
    }

    #[tokio::test]
    async fn release_frees_slot() {
        let provider = DesktopExecutionSlotProvider::new(1);
        assert_eq!(
            provider.try_acquire_execution_slot("b").await.unwrap(),
            ExecutionSlotAcquireOutcome::Acquired
        );
        assert_eq!(
            provider.try_acquire_execution_slot("b").await.unwrap(),
            ExecutionSlotAcquireOutcome::Saturated
        );
        provider.release_execution_slot("b").await.unwrap();
        assert_eq!(
            provider.try_acquire_execution_slot("b").await.unwrap(),
            ExecutionSlotAcquireOutcome::Acquired
        );
    }

    #[tokio::test]
    async fn release_below_zero_is_safe() {
        let provider = DesktopExecutionSlotProvider::new(1);
        // Release without acquire must not panic or wrap.
        provider.release_execution_slot("c").await.unwrap();
        let (active, _) = provider.snapshot("c").await;
        assert_eq!(active, 0);
    }

    #[tokio::test]
    async fn wait_returns_when_cancelled() {
        let provider = DesktopExecutionSlotProvider::new(1);
        provider.try_acquire_execution_slot("d").await.unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = wait_for_execution_slot(&provider, "d", &cancel).await;
        assert!(result.is_err());
    }
}
