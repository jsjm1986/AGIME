use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::state::HarnessMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    ModeTransition,
    RecoveryCompaction,
    NoToolRepair,
    PlannerAutoUpgrade,
    CoordinatorCompletion,
    ToolBudgetFallback,
    ChildTaskDowngrade,
    SwarmFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionRecord {
    pub turn: u32,
    pub mode: HarnessMode,
    pub kind: TransitionKind,
    pub reason: String,
    pub metadata: BTreeMap<String, String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionTrace {
    pub records: Vec<TransitionRecord>,
    pub max_records: usize,
}

impl Default for TransitionTrace {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            max_records: 256,
        }
    }
}

impl TransitionTrace {
    pub fn record(
        &mut self,
        turn: u32,
        mode: HarnessMode,
        kind: TransitionKind,
        reason: impl Into<String>,
        metadata: BTreeMap<String, String>,
    ) {
        self.records.push(TransitionRecord {
            turn,
            mode,
            kind,
            reason: reason.into(),
            metadata,
            created_at: Utc::now().timestamp(),
        });
        if self.records.len() > self.max_records {
            let trim = self.records.len().saturating_sub(self.max_records);
            self.records.drain(0..trim);
        }
    }

    pub fn latest(&self) -> Option<&TransitionRecord> {
        self.records.last()
    }
}

#[derive(Debug, Clone, Default)]
pub struct QueryTrackingState {
    pub chain_id: Option<String>,
    pub depth: u32,
    pub turn_count: u32,
}

pub type SharedTransitionTrace = Arc<Mutex<TransitionTrace>>;

pub fn shared_transition_trace() -> SharedTransitionTrace {
    Arc::new(Mutex::new(TransitionTrace::default()))
}

pub async fn record_transition(
    trace: &SharedTransitionTrace,
    turn: u32,
    mode: HarnessMode,
    kind: TransitionKind,
    reason: impl Into<String>,
    metadata: BTreeMap<String, String>,
) {
    trace
        .lock()
        .await
        .record(turn, mode, kind, reason, metadata);
}
