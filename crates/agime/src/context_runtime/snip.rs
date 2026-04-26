use crate::conversation::message::{Message, MessageContent};

use super::types::{SnipRemovalState, SNIP_KEEP_RECENT_MESSAGES, SNIP_MAX_REMOVALS};

pub(crate) fn collect_removed_indexes(
    removals: &[SnipRemovalState],
) -> std::collections::HashSet<usize> {
    let mut removed = std::collections::HashSet::new();
    for removal in removals {
        for index in &removal.removed_indexes {
            removed.insert(*index);
        }
    }
    removed
}

pub(crate) fn build_snip_state(
    messages: &[Message],
    preserved_start: usize,
) -> Vec<SnipRemovalState> {
    if preserved_start == 0 {
        return Vec::new();
    }

    let mut removed_indexes = Vec::new();
    let mut removed_message_ids = Vec::new();
    let mut token_estimate_freed = 0usize;
    for (idx, message) in messages.iter().enumerate().take(preserved_start) {
        if removed_indexes.len() >= SNIP_MAX_REMOVALS {
            break;
        }
        if !is_snip_candidate(message) {
            continue;
        }
        removed_indexes.push(idx);
        removed_message_ids.push(message.id.clone().unwrap_or_default());
        token_estimate_freed =
            token_estimate_freed.saturating_add(estimate_message_tokens(message));
    }

    if removed_indexes.is_empty() {
        return Vec::new();
    }

    let removed_count = removed_indexes.len();
    vec![SnipRemovalState {
        removed_indexes,
        removed_message_ids,
        removed_count,
        token_estimate_freed,
        reason: "agent_only_runtime_noise".to_string(),
        created_at: chrono::Utc::now().timestamp(),
    }]
}

pub(crate) fn default_preserved_start(total: usize) -> usize {
    total.saturating_sub(SNIP_KEEP_RECENT_MESSAGES)
}

fn is_snip_candidate(message: &Message) -> bool {
    if !message.is_agent_visible() || message.is_user_visible() {
        return false;
    }
    if !message.content.iter().all(|content| {
        matches!(
            content,
            MessageContent::SystemNotification(_)
                | MessageContent::Thinking(_)
                | MessageContent::RedactedThinking(_)
        )
    }) {
        return false;
    }
    true
}

fn estimate_message_tokens(message: &Message) -> usize {
    let content_chars = message
        .content
        .iter()
        .map(|content| format!("{content:?}").chars().count())
        .sum::<usize>();
    let role_chars = match message.role {
        rmcp::model::Role::User | rmcp::model::Role::Assistant => 32,
    };
    role_chars + (content_chars / 4).max(1) + 12
}
