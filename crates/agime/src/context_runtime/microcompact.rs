use crate::conversation::message::{Message, MessageContent};

use super::types::{
    ContextRuntimeState, MicrocompactMode, MicrocompactState, MICROCOMPACT_KEEP_RECENT_MESSAGES,
    MICROCOMPACT_MAX_ENTRIES, MICROCOMPACT_MIN_TEXT_CHARS, TIME_BASED_MICROCOMPACT_GAP_SECS,
    TIME_BASED_MICROCOMPACT_KEEP_RECENT_RESULTS,
};

pub(crate) fn default_microcompact_cutoff(total: usize) -> usize {
    total.saturating_sub(MICROCOMPACT_KEEP_RECENT_MESSAGES)
}

pub(crate) fn build_microcompact_state(
    messages: &[Message],
    cutoff: usize,
    existing: &[MicrocompactState],
) -> Vec<MicrocompactState> {
    let latest_created = messages
        .iter()
        .map(|message| message.created)
        .max()
        .unwrap_or(0);
    let candidate_indexes = collect_time_based_candidates(messages, cutoff);
    let mut entries = Vec::new();
    for (idx, message) in messages.iter().enumerate().take(cutoff) {
        if entries.len() >= MICROCOMPACT_MAX_ENTRIES {
            break;
        }
        let Some((tool_response_ids, original_text_chars, original_text_lines)) =
            tool_response_text_stats(message)
        else {
            continue;
        };
        if original_text_chars < MICROCOMPACT_MIN_TEXT_CHARS {
            continue;
        }

        let mode = if candidate_indexes.contains(&idx)
            && latest_created.saturating_sub(message.created) >= TIME_BASED_MICROCOMPACT_GAP_SECS
        {
            MicrocompactMode::TimeBased
        } else {
            existing
                .iter()
                .find(|entry| {
                    entry.message_index == idx && entry.tool_response_ids == tool_response_ids
                })
                .map(|_| MicrocompactMode::Cached)
                .unwrap_or(MicrocompactMode::MarkerRewrite)
        };

        let compacted_text = microcompact_marker_text(
            &mode,
            &tool_response_ids,
            original_text_chars,
            original_text_lines,
        );
        entries.push(MicrocompactState {
            message_index: idx,
            message_id: message.id.clone(),
            tool_response_ids,
            mode,
            original_text_chars,
            original_text_lines,
            compacted_text_chars: compacted_text.chars().count(),
            compacted_at: chrono::Utc::now().timestamp(),
        });
    }
    entries
}

pub(crate) fn apply_microcompact_to_message(
    message: &Message,
    message_index: usize,
    state: &ContextRuntimeState,
) -> Message {
    let Some(entry) = state
        .microcompact_entries()
        .iter()
        .find(|entry| entry.message_index == message_index)
    else {
        return message.clone();
    };

    let mut compacted = message.clone();
    compacted.content = compacted
        .content
        .iter()
        .map(|content| match content {
            MessageContent::ToolResponse(response)
                if entry
                    .tool_response_ids
                    .iter()
                    .any(|response_id| response_id == &response.id) =>
            {
                let marker = microcompact_marker_text(
                    &entry.mode,
                    &entry.tool_response_ids,
                    entry.original_text_chars,
                    entry.original_text_lines,
                );
                let tool_result = match &response.tool_result {
                    Ok(result) => Ok(rmcp::model::CallToolResult {
                        content: vec![rmcp::model::Content::text(marker)],
                        structured_content: result.structured_content.clone(),
                        is_error: result.is_error,
                        meta: result.meta.clone(),
                    }),
                    Err(err) => Err(err.clone()),
                };
                MessageContent::tool_response(response.id.clone(), tool_result)
            }
            _ => content.clone(),
        })
        .collect();
    compacted
}

fn tool_response_text_stats(message: &Message) -> Option<(Vec<String>, usize, usize)> {
    let mut tool_response_ids = Vec::new();
    let mut original_text_chars = 0usize;
    let mut original_text_lines = 0usize;

    for content in &message.content {
        let MessageContent::ToolResponse(response) = content else {
            continue;
        };
        tool_response_ids.push(response.id.clone());
        let text_fragments = match &response.tool_result {
            Ok(result) => result
                .content
                .iter()
                .filter_map(|item| item.as_text().map(|text| text.text.clone()))
                .collect::<Vec<_>>(),
            Err(error_message) => vec![error_message.to_string()],
        };
        let combined = text_fragments.join("\n");
        original_text_chars = original_text_chars.saturating_add(combined.chars().count());
        original_text_lines = original_text_lines.saturating_add(
            combined
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count()
                .max(1),
        );
    }

    if tool_response_ids.is_empty() {
        return None;
    }
    Some((tool_response_ids, original_text_chars, original_text_lines))
}

pub(crate) fn microcompact_marker_text(
    mode: &MicrocompactMode,
    tool_response_ids: &[String],
    original_text_chars: usize,
    original_text_lines: usize,
) -> String {
    format!(
        "[context runtime microcompact] Older tool output compacted for context efficiency. mode={} toolResponses={}, originalChars={}, originalLines={}. Full output remains in transcript history.",
        match mode {
            MicrocompactMode::MarkerRewrite => "marker_rewrite",
            MicrocompactMode::Cached => "cached",
            MicrocompactMode::TimeBased => "time_based",
        },
        tool_response_ids.join(","),
        original_text_chars,
        original_text_lines,
    )
}

fn collect_time_based_candidates(
    messages: &[Message],
    cutoff: usize,
) -> std::collections::HashSet<usize> {
    let candidate_indexes = messages
        .iter()
        .enumerate()
        .take(cutoff)
        .filter_map(|(idx, message)| tool_response_text_stats(message).map(|_| idx))
        .collect::<Vec<_>>();

    if candidate_indexes.len() <= TIME_BASED_MICROCOMPACT_KEEP_RECENT_RESULTS {
        return std::collections::HashSet::new();
    }

    let to_clear = candidate_indexes.len() - TIME_BASED_MICROCOMPACT_KEEP_RECENT_RESULTS;
    candidate_indexes.into_iter().take(to_clear).collect()
}
