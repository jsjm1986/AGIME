use anyhow::Result;
use tracing::info;

use crate::context_mgmt::{compact_messages_with_active_strategy, ContextCompactionStrategy};
use crate::conversation::{
    message::{Message, SystemNotificationType},
    Conversation,
};
use crate::providers::base::{Provider, ProviderUsage};
use crate::session::SessionManager;

use super::{
    checkpoints::{HarnessCheckpoint, HarnessCheckpointKind},
    state::HarnessMode,
    transcript::{HarnessCheckpointStore, HarnessTranscriptStore, SessionHarnessStore},
};

const COMPACTION_THINKING_TEXT: &str = "AGIME is compacting the conversation...";

#[derive(Debug)]
pub enum RecoveryCompactionExecution {
    Abort {
        message: String,
        next_compaction_count: u32,
    },
    Continue {
        conversation: Conversation,
        usage: ProviderUsage,
        prelude_messages: Vec<Message>,
        next_compaction_count: u32,
    },
}

pub fn max_compaction_attempts_for(strategy: ContextCompactionStrategy) -> u32 {
    match strategy {
        ContextCompactionStrategy::LegacySegmented => 3,
        ContextCompactionStrategy::CfpmMemoryV1 | ContextCompactionStrategy::CfpmMemoryV2 => 6,
    }
}

pub fn compaction_strategy_label(strategy: ContextCompactionStrategy) -> &'static str {
    strategy.as_config_value()
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

pub fn compaction_checkpoint(
    turn: u32,
    mode: HarnessMode,
    strategy: ContextCompactionStrategy,
    phase: &str,
) -> HarnessCheckpoint {
    HarnessCheckpoint {
        kind: HarnessCheckpointKind::Compaction,
        turn,
        mode,
        detail: format!("{}:{}", phase, compaction_strategy_label(strategy)),
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
    active_compaction_strategy: ContextCompactionStrategy,
    max_compaction_attempts: u32,
    compaction_count: u32,
) -> Result<RecoveryCompactionExecution> {
    if compaction_count >= max_compaction_attempts {
        return Ok(RecoveryCompactionExecution::Abort {
            message: recovery_compaction_abort_message(max_compaction_attempts),
            next_compaction_count: compaction_count,
        });
    }

    let next_compaction_count = compaction_count + 1;
    info!(
        "Recovery compaction attempt {} of {} using {}",
        next_compaction_count,
        max_compaction_attempts,
        compaction_strategy_label(active_compaction_strategy)
    );

    let (compacted_conversation, usage) =
        compact_messages_with_active_strategy(provider, conversation, false).await?;

    transcript_store
        .replace_conversation(session_id, &compacted_conversation)
        .await?;

    if active_compaction_strategy.is_cfpm() {
        if let Err(err) = SessionManager::replace_cfpm_memory_facts_from_conversation(
            session_id,
            &compacted_conversation,
            "recovery_compaction",
        )
        .await
        {
            tracing::warn!("Failed to refresh CFPM memory facts: {}", err);
        }
    }

    let _ = transcript_store
        .record_checkpoint(
            session_id,
            compaction_checkpoint(
                turns_taken,
                current_mode,
                active_compaction_strategy,
                "recovery_compaction",
            ),
        )
        .await;

    Ok(RecoveryCompactionExecution::Continue {
        conversation: compacted_conversation,
        usage,
        prelude_messages: recovery_compaction_prelude_messages(),
        next_compaction_count,
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
