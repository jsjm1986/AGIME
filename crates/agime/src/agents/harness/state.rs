use crate::conversation::Conversation;
use serde::{Deserialize, Serialize};
use std::fmt;

use super::delegation::{DelegationMode, DelegationRuntimeState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HarnessMode {
    #[default]
    Conversation,
    Plan,
    Execute,
    Repair,
    Blocked,
    Complete,
}

impl fmt::Display for HarnessMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conversation => write!(f, "conversation"),
            Self::Plan => write!(f, "plan"),
            Self::Execute => write!(f, "execute"),
            Self::Repair => write!(f, "repair"),
            Self::Blocked => write!(f, "blocked"),
            Self::Complete => write!(f, "complete"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoordinatorExecutionMode {
    #[default]
    SingleWorker,
    ExplicitSwarm,
    AutoSwarm,
}

impl fmt::Display for CoordinatorExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SingleWorker => write!(f, "single_worker"),
            Self::ExplicitSwarm => write!(f, "explicit_swarm"),
            Self::AutoSwarm => write!(f, "auto_swarm"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTurnMode {
    #[default]
    Streaming,
    Aggregated,
}

impl fmt::Display for ProviderTurnMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Streaming => write!(f, "streaming"),
            Self::Aggregated => write!(f, "aggregated"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HarnessState {
    pub conversation: Conversation,
    pub mode: HarnessMode,
    pub coordinator_execution_mode: CoordinatorExecutionMode,
    pub turns_taken: u32,
    pub runtime_compaction_count: u32,
    pub recovery_compaction_attempts: u32,
    pub tools_updated: bool,
    pub no_tools_called: bool,
    pub did_recovery_compact_this_iteration: bool,
    pub delegation: DelegationRuntimeState,
}

impl HarnessState {
    pub fn new(
        conversation: Conversation,
        mode: HarnessMode,
        coordinator_execution_mode: CoordinatorExecutionMode,
        delegation: DelegationRuntimeState,
    ) -> Self {
        Self {
            conversation,
            mode,
            coordinator_execution_mode,
            turns_taken: 0,
            runtime_compaction_count: 0,
            recovery_compaction_attempts: 0,
            tools_updated: false,
            no_tools_called: true,
            did_recovery_compact_this_iteration: false,
            delegation,
        }
    }

    pub fn delegation_mode(&self) -> DelegationMode {
        self.delegation.mode
    }
}
