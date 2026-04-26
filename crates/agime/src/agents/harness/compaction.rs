use anyhow::Result;
use tracing::info;

use super::{
    checkpoints::{HarnessCheckpoint, HarnessCheckpointKind},
    state::HarnessMode,
    transcript::{HarnessCheckpointStore, SessionHarnessStore},
};
use crate::context_runtime::{
    load_context_runtime_state, recover_on_overflow, save_context_runtime_state,
};
use crate::conversation::{
    message::{Message, SystemNotificationType},
    Conversation,
};
use crate::providers::base::{Provider, ProviderUsage};

const COMPACTION_THINKING_TEXT: &str = "AGIME is compacting the conversation...";

#[derive(Debug)]
pub enum RecoveryCompactionExecution {
    Abort {
        message: String,
        next_runtime_compaction_count: u32,
    },
    Continue {
        conversation: Conversation,
        usage: ProviderUsage,
        prelude_messages: Vec<Message>,
        next_runtime_compaction_count: u32,
    },
}

pub fn max_compaction_attempts_for_context_runtime() -> u32 {
    3
}

pub fn compaction_strategy_label() -> &'static str {
    "context_runtime"
}

pub fn auto_compaction_inline_message(threshold_percentage: u32) -> String {
    format!(
        "Exceeded auto-compact threshold of {}%. Performing auto-compaction...",
        threshold_percentage
    )
}

pub fn recovery_compaction_inline_message() -> &'static str {
    "Context limit reached. Compacting to continue conversation..."
}

pub fn recovery_compaction_abort_message(max_compaction_attempts: u32) -> String {
    format!(
        "已达到最大压缩次数限制 ({} 次)。请创建新会话继续对话。\n\nReached maximum compaction attempts. Please start a new session to continue.",
        max_compaction_attempts
    )
}

pub fn recovery_compaction_prelude_messages() -> Vec<Message> {
    vec![
        Message::assistant().with_system_notification(
            SystemNotificationType::InlineMessage,
            recovery_compaction_inline_message(),
        ),
        Message::assistant().with_system_notification(
            SystemNotificationType::ThinkingMessage,
            COMPACTION_THINKING_TEXT,
        ),
    ]
}

pub fn compaction_checkpoint(turn: u32, mode: HarnessMode, phase: &str) -> HarnessCheckpoint {
    HarnessCheckpoint {
        kind: HarnessCheckpointKind::Compaction,
        turn,
        mode,
        detail: format!("{}:{}", phase, compaction_strategy_label()),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_recovery_compaction(
    provider: &dyn Provider,
    conversation: &Conversation,
    session_id: &str,
    transcript_store: &SessionHarnessStore,
    turns_taken: u32,
    current_mode: HarnessMode,
    max_compaction_attempts: u32,
    runtime_compaction_count: u32,
) -> Result<RecoveryCompactionExecution> {
    if runtime_compaction_count >= max_compaction_attempts {
        return Ok(RecoveryCompactionExecution::Abort {
            message: recovery_compaction_abort_message(max_compaction_attempts),
            next_runtime_compaction_count: runtime_compaction_count,
        });
    }

    let next_runtime_compaction_count = runtime_compaction_count + 1;
    info!(
        "Recovery compaction attempt {} of {} using {}",
        next_runtime_compaction_count,
        max_compaction_attempts,
        compaction_strategy_label()
    );

    let mut context_runtime_state = load_context_runtime_state(session_id).await?;
    let _recovery = recover_on_overflow(provider, conversation, &mut context_runtime_state).await?;
    save_context_runtime_state(session_id, &context_runtime_state).await?;

    let _ = transcript_store
        .record_checkpoint(
            session_id,
            compaction_checkpoint(turns_taken, current_mode, "recovery_compaction"),
        )
        .await;

    Ok(RecoveryCompactionExecution::Continue {
        conversation: conversation.clone(),
        usage: ProviderUsage::new(
            "context-runtime".to_string(),
            crate::providers::base::Usage::default(),
        ),
        prelude_messages: recovery_compaction_prelude_messages(),
        next_runtime_compaction_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_compaction_abort_message_includes_limit() {
        let message = recovery_compaction_abort_message(3);
        assert!(message.contains("3"));
        assert!(message.contains("maximum compaction attempts"));
    }

    #[test]
    fn recovery_compaction_prelude_messages_include_inline_and_thinking() {
        let messages = recovery_compaction_prelude_messages();
        assert_eq!(messages.len(), 2);
    }
}
