use anyhow::Result;
use chrono::Utc;

use crate::conversation::Conversation;
use crate::providers::base::Provider;

use super::session_memory::{
    adjust_suffix_start, estimate_message_tokens, has_textual_content, summarize_messages,
};
use super::store::{current_visible_working_window, preferred_compact_direction};
use super::types::{
    CollapseCommit, CollapseSnapshot, CompactBoundaryMetadata, CompactDirection,
    ContextRuntimeState, PreservedSegmentMetadata, CURRENT_CONTEXT_RUNTIME_SCHEMA_VERSION,
    HEAVY_STAGE_KEEP_LAST_MESSAGES, MIN_HEAVY_MESSAGES_TO_STAGE, MIN_MESSAGES_TO_STAGE,
    STAGE_ABSOLUTE_TRIGGER_TOKENS, STAGE_KEEP_FIRST_MESSAGES, STAGE_KEEP_LAST_MESSAGES,
};

pub(crate) async fn build_staged_collapse(
    provider: &dyn Provider,
    conversation: &Conversation,
    state: &ContextRuntimeState,
    usage_ratio: f64,
    projected_tokens: usize,
) -> Result<Option<CollapseSnapshot>> {
    let messages = conversation.messages();
    let (window_start, window_end_exclusive) =
        current_visible_working_window(messages.len(), state);
    if window_end_exclusive <= window_start || window_end_exclusive > messages.len() {
        return Ok(None);
    }

    let working_messages = &messages[window_start..window_end_exclusive];
    let has_heavy_low_turn_shape = working_messages.len() >= MIN_HEAVY_MESSAGES_TO_STAGE
        && projected_tokens >= STAGE_ABSOLUTE_TRIGGER_TOKENS;
    if working_messages.len() < MIN_MESSAGES_TO_STAGE && !has_heavy_low_turn_shape {
        return Ok(None);
    }

    let mut start = STAGE_KEEP_FIRST_MESSAGES.min(working_messages.len());
    let keep_last = if has_heavy_low_turn_shape {
        HEAVY_STAGE_KEEP_LAST_MESSAGES
    } else {
        STAGE_KEEP_LAST_MESSAGES
    };
    let mut end = working_messages.len().saturating_sub(keep_last);
    end = adjust_suffix_start(working_messages, end);
    start = adjust_collapse_start(working_messages, start, end);
    if end <= start {
        return Ok(None);
    }

    let direction = preferred_compact_direction(state)
        .unwrap_or_else(|| choose_compact_direction(working_messages, start, end));
    let (collapse_start, collapse_end) = match direction {
        CompactDirection::UpTo => (window_start + start, window_start + end),
        CompactDirection::From => {
            let from_start = adjust_from_start(working_messages, start, end);
            if from_start >= working_messages.len() {
                return Ok(None);
            }
            (
                window_start + from_start,
                window_start + working_messages.len(),
            )
        }
    };

    if collapse_end <= collapse_start {
        return Ok(None);
    }

    let summary = summarize_messages(provider, &messages[collapse_start..collapse_end]).await?;
    Ok(Some(CollapseSnapshot {
        snapshot_id: Some(uuid::Uuid::new_v4().to_string()),
        start_index: collapse_start,
        end_index: collapse_end,
        direction,
        start_message_id: messages
            .get(collapse_start)
            .and_then(|message| message.id.clone()),
        end_message_id: messages
            .get(collapse_end.saturating_sub(1))
            .and_then(|message| message.id.clone()),
        summary,
        risk: usage_ratio,
        staged_at: Utc::now().timestamp(),
    }))
}

pub(crate) fn commit_staged_collapses(
    conversation: &Conversation,
    state: &mut ContextRuntimeState,
    preserved_reason: &str,
    recovery_result: Option<&str>,
) {
    let Some(staged) = state.staged_snapshot().cloned() else {
        return;
    };
    state.set_staged_snapshot(None);

    let collapse_start = staged.start_index;
    let collapse_end = staged.end_index;
    let direction = staged.direction;

    state.append_collapse_commit(CollapseCommit {
        commit_id: staged.snapshot_id.clone(),
        summary: staged.summary,
        start_index: staged.start_index,
        end_index: staged.end_index,
        direction,
        start_message_id: staged.start_message_id.clone(),
        end_message_id: staged.end_message_id.clone(),
        created_at: Utc::now().timestamp(),
    });
    state.runtime_compactions = state.runtime_compactions.saturating_add(1);
    let messages = conversation.messages();
    let last_index = messages.len().saturating_sub(1);
    match direction {
        CompactDirection::UpTo => {
            let preserved_start = collapse_end.min(last_index);
            state.set_compact_boundary(Some(CompactBoundaryMetadata {
                anchor_index: collapse_start,
                head_index: collapse_start,
                tail_index: preserved_start,
                direction,
                anchor_message_id: messages
                    .get(collapse_start)
                    .and_then(|message| message.id.clone()),
                head_message_id: messages
                    .get(collapse_start)
                    .and_then(|message| message.id.clone()),
                tail_message_id: messages
                    .get(preserved_start)
                    .and_then(|message| message.id.clone()),
                created_at: Utc::now().timestamp(),
            }));
            state.set_preserved_segment(Some(PreservedSegmentMetadata {
                start_index: preserved_start,
                end_index: last_index,
                tail_anchor_index: last_index,
                start_message_id: messages
                    .get(preserved_start)
                    .and_then(|message| message.id.clone()),
                end_message_id: messages.last().and_then(|message| message.id.clone()),
                tail_anchor_message_id: messages.last().and_then(|message| message.id.clone()),
                reason: preserved_reason.to_string(),
            }));
        }
        CompactDirection::From => {
            let preserved_end = collapse_start.saturating_sub(1).min(last_index);
            state.set_compact_boundary(Some(CompactBoundaryMetadata {
                anchor_index: collapse_start.min(last_index),
                head_index: preserved_end,
                tail_index: collapse_end.saturating_sub(1).min(last_index),
                direction,
                anchor_message_id: messages
                    .get(collapse_start.min(last_index))
                    .and_then(|message| message.id.clone()),
                head_message_id: messages
                    .get(preserved_end)
                    .and_then(|message| message.id.clone()),
                tail_message_id: messages
                    .get(collapse_end.saturating_sub(1).min(last_index))
                    .and_then(|message| message.id.clone()),
                created_at: Utc::now().timestamp(),
            }));
            state.set_preserved_segment(Some(PreservedSegmentMetadata {
                start_index: 0,
                end_index: preserved_end,
                tail_anchor_index: preserved_end,
                start_message_id: messages.first().and_then(|message| message.id.clone()),
                end_message_id: messages
                    .get(preserved_end)
                    .and_then(|message| message.id.clone()),
                tail_anchor_message_id: messages
                    .get(preserved_end)
                    .and_then(|message| message.id.clone()),
                reason: preserved_reason.to_string(),
            }));
        }
    }
    state.schema_version = CURRENT_CONTEXT_RUNTIME_SCHEMA_VERSION;
    state.last_compact_reason = Some(preserved_reason.to_string());
    if let Some(result) = recovery_result {
        state.last_recovery_result = Some(result.to_string());
    }
}

pub(crate) fn adjust_collapse_start(
    messages: &[crate::conversation::message::Message],
    mut start: usize,
    end: usize,
) -> usize {
    if start == 0 || start >= end || end > messages.len() {
        return start.min(end);
    }

    let mut collapsed_tool_response_ids = std::collections::HashSet::new();
    let mut collapsed_assistant_ids = std::collections::HashSet::new();
    for msg in &messages[start..end] {
        for response_id in msg.get_tool_response_ids() {
            collapsed_tool_response_ids.insert(response_id.to_string());
        }
        if msg.role == rmcp::model::Role::Assistant {
            if let Some(message_id) = msg.id.as_deref() {
                collapsed_assistant_ids.insert(message_id.to_string());
            }
        }
    }

    for idx in (0..start).rev() {
        let msg = &messages[idx];
        let splits_tool_pair = msg
            .get_tool_request_ids()
            .iter()
            .any(|request_id| collapsed_tool_response_ids.contains(*request_id));
        let splits_assistant_continuation = msg.role == rmcp::model::Role::Assistant
            && msg
                .id
                .as_deref()
                .is_some_and(|message_id| collapsed_assistant_ids.contains(message_id));
        if splits_tool_pair || splits_assistant_continuation {
            start = idx;
        }
    }

    start
}

fn choose_compact_direction(
    messages: &[crate::conversation::message::Message],
    start: usize,
    end: usize,
) -> CompactDirection {
    let prefix = &messages[..start.min(messages.len())];
    let suffix = &messages[end.min(messages.len())..];
    let prefix_textual = prefix
        .iter()
        .filter(|message| has_textual_content(message))
        .count();
    let suffix_textual = suffix
        .iter()
        .filter(|message| has_textual_content(message))
        .count();
    let suffix_tool_responses = suffix
        .iter()
        .flat_map(|message| message.get_tool_response_ids())
        .count();
    let suffix_tokens = suffix.iter().map(estimate_message_tokens).sum::<usize>();
    let prefix_tokens = prefix.iter().map(estimate_message_tokens).sum::<usize>();

    if !suffix.is_empty()
        && prefix_textual >= 2
        && suffix_textual <= 1
        && suffix_tool_responses > 0
        && suffix_tokens >= prefix_tokens.max(1)
    {
        CompactDirection::From
    } else {
        CompactDirection::UpTo
    }
}

fn adjust_from_start(
    messages: &[crate::conversation::message::Message],
    start: usize,
    end: usize,
) -> usize {
    if end >= messages.len() {
        return end;
    }

    let mut from_start = end.min(messages.len().saturating_sub(1));
    let mut preserved_tool_request_ids = std::collections::HashSet::new();
    let mut preserved_assistant_ids = std::collections::HashSet::new();
    for msg in &messages[..from_start] {
        for request_id in msg.get_tool_request_ids() {
            preserved_tool_request_ids.insert(request_id.to_string());
        }
        if msg.role == rmcp::model::Role::Assistant {
            if let Some(message_id) = msg.id.as_deref() {
                preserved_assistant_ids.insert(message_id.to_string());
            }
        }
    }

    for idx in from_start..messages.len() {
        let msg = &messages[idx];
        let references_preserved_tool = msg
            .get_tool_response_ids()
            .iter()
            .any(|response_id| preserved_tool_request_ids.contains(*response_id));
        let shares_assistant_id = msg.role == rmcp::model::Role::Assistant
            && msg
                .id
                .as_deref()
                .is_some_and(|message_id| preserved_assistant_ids.contains(message_id));
        if references_preserved_tool || shares_assistant_id {
            from_start = idx.saturating_add(1);
            continue;
        }
        break;
    }

    from_start.max(start).min(messages.len())
}
