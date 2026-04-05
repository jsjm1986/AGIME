use serde::{Deserialize, Serialize};

use super::state::HarnessMode;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarnessCheckpointKind {
    TurnStart,
    TurnEnd,
    ToolBatch,
    Compaction,
    Delegation,
    ModeTransition,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessCheckpoint {
    pub kind: HarnessCheckpointKind,
    pub turn: u32,
    pub mode: HarnessMode,
    pub detail: String,
}

impl HarnessCheckpoint {
    pub fn turn_start(turn: u32, mode: HarnessMode) -> Self {
        Self {
            kind: HarnessCheckpointKind::TurnStart,
            turn,
            mode,
            detail: format!("turn_{}_start", turn),
        }
    }

    pub fn turn_end(turn: u32, mode: HarnessMode, no_tools_called: bool) -> Self {
        Self {
            kind: HarnessCheckpointKind::TurnEnd,
            turn,
            mode,
            detail: if no_tools_called {
                "turn_end:no_tools_called".to_string()
            } else {
                "turn_end:tool_activity".to_string()
            },
        }
    }

    pub fn tool_batch(
        turn: u32,
        mode: HarnessMode,
        batch_count: usize,
        concurrent_batch_count: usize,
        serial_batch_count: usize,
    ) -> Self {
        Self {
            kind: HarnessCheckpointKind::ToolBatch,
            turn,
            mode,
            detail: format!(
                "batches={}, concurrent_batches={}, serial_batches={}",
                batch_count, concurrent_batch_count, serial_batch_count
            ),
        }
    }

    pub fn delegation_downgraded(turn: u32, mode: HarnessMode) -> Self {
        Self {
            kind: HarnessCheckpointKind::Delegation,
            turn,
            mode,
            detail: "subagent_downgraded".to_string(),
        }
    }

    pub fn mode_transition(turn: u32, mode: HarnessMode, reason: impl Into<String>) -> Self {
        Self {
            kind: HarnessCheckpointKind::ModeTransition,
            turn,
            mode,
            detail: reason.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_end_checkpoint_encodes_no_tool_state() {
        let checkpoint = HarnessCheckpoint::turn_end(2, HarnessMode::Repair, true);
        assert_eq!(checkpoint.kind, HarnessCheckpointKind::TurnEnd);
        assert_eq!(checkpoint.detail, "turn_end:no_tools_called");
    }

    #[test]
    fn tool_batch_checkpoint_encodes_counts() {
        let checkpoint = HarnessCheckpoint::tool_batch(3, HarnessMode::Execute, 2, 1, 1);
        assert_eq!(checkpoint.kind, HarnessCheckpointKind::ToolBatch);
        assert!(checkpoint.detail.contains("batches=2"));
    }
}
