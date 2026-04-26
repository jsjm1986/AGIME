use crate::conversation::message::{Message, MessageMetadata};
use crate::conversation::Conversation;
use crate::providers::base::Provider;
use crate::runtime_profile::resolve_from_model_config;

use super::microcompact::apply_microcompact_to_message;
use super::microcompact::{build_microcompact_state, default_microcompact_cutoff};
use super::session_memory::estimate_message_tokens;
use super::snip::{build_snip_state, collect_removed_indexes, default_preserved_start};
use super::store::{
    finalize_projection_refresh, reconcile_state_for_conversation, restore_from_entries,
};
use super::types::{
    CompactDirection, ContextRuntimeState, ProjectionStats, RuntimeBudget,
    CONTEXT_RUNTIME_CONTINUATION_TEXT, CURRENT_CONTEXT_RUNTIME_SCHEMA_VERSION,
};

pub fn project_for_provider(
    conversation: &Conversation,
    state: &ContextRuntimeState,
) -> Conversation {
    let prepared = prepared_projection_state(conversation, state);
    project_prepared_view(conversation, &prepared)
}

pub fn project_view(conversation: &Conversation, state: &ContextRuntimeState) -> Conversation {
    let prepared = prepared_projection_state(conversation, state);
    project_prepared_view(conversation, &prepared)
}

pub fn refresh_projection_only(
    conversation: &Conversation,
    state: &mut ContextRuntimeState,
) -> bool {
    reconcile_state_for_conversation(conversation, state);
    refresh_projection_state(conversation, state)
}

pub(crate) fn refresh_projection_from_entry_log(
    conversation: &Conversation,
    state: &mut ContextRuntimeState,
) -> bool {
    state.store = state
        .store
        .with_backfilled_entry_log()
        .restored_from_entries();
    refresh_projection_state(conversation, state)
}

pub(crate) fn prepared_projection_state(
    conversation: &Conversation,
    state: &ContextRuntimeState,
) -> ContextRuntimeState {
    let mut prepared = restore_from_entries(state);
    refresh_projection_only(conversation, &mut prepared);
    prepared
}

pub(crate) fn project_prepared_view(
    conversation: &Conversation,
    prepared_state: &ContextRuntimeState,
) -> Conversation {
    apply_projection(conversation, prepared_state)
}

pub(crate) fn apply_projection(
    conversation: &Conversation,
    state: &ContextRuntimeState,
) -> Conversation {
    let messages = conversation.messages();
    if messages.is_empty() {
        return conversation.clone();
    }

    let mut projected = Vec::new();
    let total = messages.len();
    let removed = collect_removed_indexes(state.snip_entries());
    let mut committed = state.committed_collapses().to_vec();
    committed.sort_by_key(|entry| entry.start_index);

    let session_memory = state.session_memory();
    let (slice_start, slice_end, direction) = projection_window(total, state);

    if let Some(memory) = session_memory {
        if direction == CompactDirection::UpTo && slice_start > 0 {
            projected.extend(runtime_summary_messages(&memory.summary, "session_memory"));
        }
    } else if direction == CompactDirection::UpTo && slice_start > 0 {
        append_committed_summaries_before(&mut projected, &committed, slice_start);
    }

    let mut i = slice_start;
    while i <= slice_end && i < total {
        if let Some(commit) = committed.iter().find(|entry| {
            entry.direction == CompactDirection::UpTo
                && entry.start_index <= i
                && i < entry.end_index
        }) {
            projected.extend(runtime_summary_messages(&commit.summary, "collapse"));
            i = commit.end_index.min(total);
            continue;
        }

        if removed.contains(&i) {
            i += 1;
            continue;
        }

        projected.push(apply_microcompact_to_message(&messages[i], i, state));
        i += 1;
    }

    if direction == CompactDirection::From {
        append_committed_summaries_after(&mut projected, &committed, slice_end);
    }

    Conversation::new_unvalidated(projected)
}

pub(crate) fn projection_slice_start(total: usize, state: &ContextRuntimeState) -> usize {
    projection_window(total, state).0
}

fn projection_window(
    total: usize,
    state: &ContextRuntimeState,
) -> (usize, usize, CompactDirection) {
    state.store.projection_window(total)
}

fn append_committed_summaries_before(
    projected: &mut Vec<Message>,
    committed: &[super::types::CollapseCommit],
    slice_start: usize,
) {
    for commit in committed.iter().filter(|entry| {
        entry.direction == CompactDirection::UpTo
            && entry.start_index < slice_start
            && entry.end_index <= slice_start
    }) {
        projected.extend(runtime_summary_messages(&commit.summary, "collapse"));
    }
}

fn append_committed_summaries_after(
    projected: &mut Vec<Message>,
    committed: &[super::types::CollapseCommit],
    slice_end: usize,
) {
    for commit in committed
        .iter()
        .filter(|entry| entry.direction == CompactDirection::From && entry.start_index > slice_end)
    {
        projected.extend(runtime_summary_messages(&commit.summary, "collapse"));
    }
}

pub(crate) fn refresh_projection_state(
    conversation: &Conversation,
    state: &mut ContextRuntimeState,
) -> bool {
    let messages = conversation.messages();
    let total = messages.len();
    if total == 0 {
        state.clear_projection_entries();
        state.last_projection_stats = Some(ProjectionStats::default());
        return false;
    }

    let has_runtime_window = state.preserved_segment().is_some()
        || state.session_memory().is_some()
        || state.compact_boundary().is_some();
    let anchored_start = projection_slice_start(total, state);
    let preserved_start = if has_runtime_window {
        anchored_start
    } else {
        default_preserved_start(total)
    };
    let microcompact_cutoff = if has_runtime_window {
        anchored_start
    } else {
        default_microcompact_cutoff(total)
    };

    let next_snips = build_snip_state(messages, preserved_start);
    let next_microcompact =
        build_microcompact_state(messages, microcompact_cutoff, state.microcompact_entries());

    let snips_changed = state.snip_entries() != next_snips.as_slice();
    let microcompact_changed = state.microcompact_entries() != next_microcompact.as_slice();
    state.set_snip_entries(next_snips);
    state.set_microcompact_entries(next_microcompact);
    state.schema_version = CURRENT_CONTEXT_RUNTIME_SCHEMA_VERSION;
    finalize_projection_refresh(total, state);

    let raw_token_estimate =
        estimate_conversation_tokens(messages.iter().filter(|m| m.is_agent_visible()));
    let projected = apply_projection(conversation, state);
    let projected_token_estimate =
        estimate_conversation_tokens(projected.messages().iter().filter(|m| m.is_agent_visible()));
    state.last_projection_stats = Some(ProjectionStats {
        base_agent_messages: messages.iter().filter(|m| m.is_agent_visible()).count(),
        projected_agent_messages: projected
            .messages()
            .iter()
            .filter(|m| m.is_agent_visible())
            .count(),
        snip_removed_count: state
            .snip_entries()
            .iter()
            .map(|entry| entry.removed_count)
            .sum(),
        microcompacted_count: state.microcompact_entries().len(),
        raw_token_estimate,
        projected_token_estimate,
        freed_token_estimate: raw_token_estimate.saturating_sub(projected_token_estimate),
        updated_at: chrono::Utc::now().timestamp(),
    });

    snips_changed || microcompact_changed
}

fn runtime_summary_messages(summary: &str, label: &str) -> Vec<Message> {
    vec![
        Message::user()
            .with_text(format!(
                "[CONTEXT_RUNTIME_{}]\n{}",
                label.to_ascii_uppercase(),
                summary
            ))
            .with_metadata(MessageMetadata::agent_only()),
        Message::assistant()
            .with_text(CONTEXT_RUNTIME_CONTINUATION_TEXT)
            .with_metadata(MessageMetadata::agent_only()),
    ]
}

fn estimate_conversation_tokens<'a, I>(messages: I) -> usize
where
    I: IntoIterator<Item = &'a Message>,
{
    messages
        .into_iter()
        .map(estimate_message_tokens)
        .sum::<usize>()
}

#[cfg(test)]
pub(crate) fn projected_runtime_budget(
    provider: &dyn Provider,
    conversation: &Conversation,
    state: &ContextRuntimeState,
) -> RuntimeBudget {
    let restored = restore_from_entries(state);
    projected_runtime_budget_for_prepared_state(provider, conversation, &restored)
}

pub(crate) fn projected_runtime_budget_for_prepared_state(
    provider: &dyn Provider,
    conversation: &Conversation,
    prepared_state: &ContextRuntimeState,
) -> RuntimeBudget {
    let projected = project_prepared_view(conversation, prepared_state);
    let tokens = projected
        .messages()
        .iter()
        .filter(|message| message.is_agent_visible())
        .map(estimate_message_tokens)
        .sum::<usize>();
    let profile = resolve_from_model_config(&provider.get_model_config());
    RuntimeBudget::new(
        profile.context_limit.max(1),
        tokens,
        profile.output_reserve_tokens,
        profile.auto_compact_threshold,
    )
}
