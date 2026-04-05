use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::conversation::{message::Message, Conversation};
use crate::session::extension_data::{ExtensionData, ExtensionState};
use crate::session::SessionManager;

use super::checkpoints::HarnessCheckpoint;
use super::state::HarnessMode;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HarnessSessionState {
    pub mode: HarnessMode,
    pub updated_at: i64,
    pub turns_taken: u32,
    pub compaction_count: u32,
    pub delegation_depth: u32,
}

impl HarnessSessionState {
    pub fn new(mode: HarnessMode) -> Self {
        Self {
            mode,
            updated_at: Utc::now().timestamp(),
            turns_taken: 0,
            compaction_count: 0,
            delegation_depth: 0,
        }
    }

    pub fn snapshot(
        mode: HarnessMode,
        turns_taken: u32,
        compaction_count: u32,
        delegation_depth: u32,
    ) -> Self {
        Self::new(mode).with_snapshot(turns_taken, compaction_count, delegation_depth)
    }

    pub fn with_snapshot(
        mut self,
        turns_taken: u32,
        compaction_count: u32,
        delegation_depth: u32,
    ) -> Self {
        self.turns_taken = turns_taken;
        self.compaction_count = compaction_count;
        self.delegation_depth = delegation_depth;
        self.updated_at = Utc::now().timestamp();
        self
    }
}

impl ExtensionState for HarnessSessionState {
    const EXTENSION_NAME: &'static str = "harness";
    const VERSION: &'static str = "v0";
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HarnessCheckpointState {
    pub checkpoints: Vec<HarnessCheckpoint>,
}

impl ExtensionState for HarnessCheckpointState {
    const EXTENSION_NAME: &'static str = "harness_checkpoints";
    const VERSION: &'static str = "v0";
}

#[async_trait]
pub trait HarnessTranscriptStore: Send + Sync + Clone + 'static {
    async fn append_message(&self, session_id: &str, message: &Message) -> Result<()>;
    async fn append_messages(&self, session_id: &str, messages: &[Message]) -> Result<()>;
    async fn replace_conversation(
        &self,
        session_id: &str,
        conversation: &Conversation,
    ) -> Result<()>;
    async fn load_state(&self, session_id: &str) -> Result<HarnessSessionState>;
    async fn save_state(&self, session_id: &str, state: HarnessSessionState) -> Result<()>;
    async fn load_mode(&self, session_id: &str) -> Result<HarnessMode>;
    async fn save_mode(&self, session_id: &str, mode: HarnessMode) -> Result<()>;
}

#[async_trait]
pub trait HarnessCheckpointStore: Send + Sync + Clone + 'static {
    async fn record_checkpoint(
        &self,
        session_id: &str,
        checkpoint: HarnessCheckpoint,
    ) -> Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct SessionHarnessStore;

impl SessionHarnessStore {
    async fn load_extension_data(&self, session_id: &str) -> Result<ExtensionData> {
        let session = SessionManager::get_session(session_id, false).await?;
        Ok(session.extension_data)
    }

    pub async fn persist_runtime_snapshot(
        &self,
        session_id: &str,
        mode: HarnessMode,
        turns_taken: u32,
        compaction_count: u32,
        delegation_depth: u32,
    ) -> Result<()> {
        self.save_state(
            session_id,
            HarnessSessionState::snapshot(mode, turns_taken, compaction_count, delegation_depth),
        )
        .await
    }
}

#[async_trait]
impl HarnessTranscriptStore for SessionHarnessStore {
    async fn append_message(&self, session_id: &str, message: &Message) -> Result<()> {
        SessionManager::add_message(session_id, message).await
    }

    async fn append_messages(&self, session_id: &str, messages: &[Message]) -> Result<()> {
        for message in messages {
            SessionManager::add_message(session_id, message).await?;
        }
        Ok(())
    }

    async fn replace_conversation(
        &self,
        session_id: &str,
        conversation: &Conversation,
    ) -> Result<()> {
        SessionManager::replace_conversation(session_id, conversation).await
    }

    async fn load_state(&self, session_id: &str) -> Result<HarnessSessionState> {
        let extension_data = self.load_extension_data(session_id).await?;
        Ok(HarnessSessionState::from_extension_data(&extension_data).unwrap_or_default())
    }

    async fn save_state(&self, session_id: &str, state: HarnessSessionState) -> Result<()> {
        SessionManager::set_extension_state(session_id, &state).await
    }

    async fn load_mode(&self, session_id: &str) -> Result<HarnessMode> {
        Ok(self.load_state(session_id).await?.mode)
    }

    async fn save_mode(&self, session_id: &str, mode: HarnessMode) -> Result<()> {
        let mut state = self.load_state(session_id).await?;
        state.mode = mode;
        state.updated_at = Utc::now().timestamp();
        self.save_state(session_id, state).await
    }
}

#[async_trait]
impl HarnessCheckpointStore for SessionHarnessStore {
    async fn record_checkpoint(
        &self,
        session_id: &str,
        checkpoint: HarnessCheckpoint,
    ) -> Result<()> {
        const MAX_CHECKPOINTS: usize = 100;

        let session = SessionManager::get_session(session_id, false).await?;
        let extension_data = session.extension_data;
        let mut state =
            HarnessCheckpointState::from_extension_data(&extension_data).unwrap_or_default();
        state.checkpoints.push(checkpoint);
        if state.checkpoints.len() > MAX_CHECKPOINTS {
            let trim = state.checkpoints.len().saturating_sub(MAX_CHECKPOINTS);
            state.checkpoints.drain(0..trim);
        }
        SessionManager::set_extension_state(session_id, &state).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_state_round_trips_through_extension_data() {
        let mut extension_data = ExtensionData::default();
        HarnessSessionState::new(HarnessMode::Plan)
            .with_snapshot(3, 1, 1)
            .to_extension_data(&mut extension_data)
            .expect("state should serialize");
        let restored = HarnessSessionState::from_extension_data(&extension_data)
            .expect("state should deserialize");
        assert_eq!(restored.mode, HarnessMode::Plan);
        assert_eq!(restored.turns_taken, 3);
        assert_eq!(restored.compaction_count, 1);
        assert_eq!(restored.delegation_depth, 1);
    }

    #[test]
    fn harness_checkpoints_round_trip_through_extension_data() {
        let mut extension_data = ExtensionData::default();
        let state = HarnessCheckpointState {
            checkpoints: vec![HarnessCheckpoint {
                kind: crate::agents::harness::HarnessCheckpointKind::TurnStart,
                turn: 1,
                mode: HarnessMode::Conversation,
                detail: "boot".to_string(),
            }],
        };
        state
            .to_extension_data(&mut extension_data)
            .expect("checkpoints should serialize");
        let restored = HarnessCheckpointState::from_extension_data(&extension_data)
            .expect("checkpoints should deserialize");
        assert_eq!(restored.checkpoints.len(), 1);
        assert_eq!(restored.checkpoints[0].detail, "boot");
    }
}
