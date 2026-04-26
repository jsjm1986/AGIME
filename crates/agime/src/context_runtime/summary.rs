use anyhow::Result;

use super::types::{ContextRuntimeState, ContextRuntimeStateSummary};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextRuntimeTransitionObservation {
    pub phase: String,
    pub reason: Option<String>,
    pub before_tokens: Option<usize>,
    pub after_tokens: Option<usize>,
}

pub fn summarize_context_runtime_state(state: &ContextRuntimeState) -> ContextRuntimeStateSummary {
    let restored = state.restored_from_entry_log();
    ContextRuntimeStateSummary {
        schema_version: restored.schema_version,
        runtime_compactions: restored.runtime_compactions,
        committed_collapse_count: restored.committed_collapses().len(),
        staged_collapse_count: restored.staged_collapse_count(),
        snip_removed_count: restored
            .snip_entries()
            .iter()
            .map(|entry| entry.removed_count)
            .sum(),
        microcompacted_count: restored.microcompact_entries().len(),
        session_memory_active: restored.session_memory().is_some(),
        preserved_segment: restored.preserved_segment().cloned(),
        compact_boundary: restored.compact_boundary().cloned(),
        last_projection_stats: restored.last_projection_stats.clone(),
        last_compact_reason: restored.last_compact_reason.clone(),
        last_recovery_result: restored.last_recovery_result.clone(),
        last_post_compact_validation: restored.last_post_compact_validation.clone(),
    }
}

pub fn serialize_context_runtime_state(state: &ContextRuntimeState) -> Result<serde_json::Value> {
    let restored = state.restored_from_entry_log();
    serde_json::to_value(restored)
        .map_err(|err| anyhow::anyhow!("serialize context runtime state: {}", err))
}

pub fn observe_runtime_transition(
    initial: Option<&ContextRuntimeState>,
    final_state: &ContextRuntimeState,
) -> Option<ContextRuntimeTransitionObservation> {
    let restored_initial = initial.map(ContextRuntimeState::restored_from_entry_log);
    let restored_final = final_state.restored_from_entry_log();

    let initial_runtime_compactions = restored_initial
        .as_ref()
        .map(|state| state.runtime_compactions)
        .unwrap_or(0);
    let initial_staged = restored_initial
        .as_ref()
        .map(|state| state.staged_collapse_count())
        .unwrap_or(0);
    let initial_committed = restored_initial
        .as_ref()
        .map(|state| state.committed_collapses().len())
        .unwrap_or(0);
    let initial_session_memory = restored_initial
        .as_ref()
        .and_then(|state| state.session_memory())
        .is_some();

    let phase = if restored_final.last_recovery_result.is_some() {
        restored_final.last_recovery_result.clone()
    } else if restored_final.runtime_compactions > initial_runtime_compactions
        && restored_final.session_memory().is_some()
        && !initial_session_memory
    {
        Some("session_memory_compaction".to_string())
    } else if restored_final.committed_collapses().len() > initial_committed {
        Some("committed_collapse".to_string())
    } else if restored_final.staged_collapse_count() > initial_staged {
        Some("staged_collapse".to_string())
    } else {
        None
    }?;

    let stats = restored_final.last_projection_stats.as_ref();
    Some(ContextRuntimeTransitionObservation {
        phase,
        reason: restored_final
            .last_compact_reason
            .clone()
            .or_else(|| restored_final.last_recovery_result.clone()),
        before_tokens: stats.map(|value| value.raw_token_estimate),
        after_tokens: stats.map(|value| value.projected_token_estimate),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        observe_runtime_transition, serialize_context_runtime_state,
        summarize_context_runtime_state,
    };
    use crate::context_runtime::{
        CollapseCommit, CompactDirection, ContextRuntimeState, MicrocompactMode, MicrocompactState,
        ProjectionStats, SnipRemovalState,
    };

    #[test]
    fn summarize_restores_entry_log_before_reporting_counts() {
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-1".to_string()),
            summary: "replayed".to_string(),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            created_at: 1,
        });
        state.set_snip_entries(vec![SnipRemovalState {
            removed_indexes: vec![1],
            removed_message_ids: vec!["m2".to_string()],
            removed_count: 1,
            token_estimate_freed: 32,
            reason: "test".to_string(),
            created_at: 1,
        }]);
        state.set_microcompact_entries(vec![MicrocompactState {
            message_index: 2,
            message_id: Some("m3".to_string()),
            tool_response_ids: vec!["tool-1".to_string()],
            mode: MicrocompactMode::Cached,
            original_text_chars: 100,
            original_text_lines: 4,
            compacted_text_chars: 20,
            compacted_at: 2,
        }]);
        state.committed_collapses_mut().clear();
        state.snip_entries_mut().clear();
        state.microcompact_entries_mut().clear();

        let summary = summarize_context_runtime_state(&state);
        assert_eq!(summary.committed_collapse_count, 1);
        assert_eq!(summary.snip_removed_count, 1);
        assert_eq!(summary.microcompacted_count, 1);
    }

    #[test]
    fn observe_transition_uses_restored_entry_log_for_final_state() {
        let initial = ContextRuntimeState::default();
        let mut final_state = ContextRuntimeState {
            last_projection_stats: Some(ProjectionStats {
                base_agent_messages: 8,
                projected_agent_messages: 4,
                snip_removed_count: 0,
                microcompacted_count: 0,
                raw_token_estimate: 2000,
                projected_token_estimate: 1200,
                freed_token_estimate: 800,
                updated_at: 1,
            }),
            ..ContextRuntimeState::default()
        };
        final_state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-2".to_string()),
            summary: "replayed".to_string(),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            created_at: 2,
        });
        final_state.committed_collapses_mut().clear();

        let observation =
            observe_runtime_transition(Some(&initial), &final_state).expect("transition");
        assert_eq!(observation.phase, "committed_collapse");
        assert_eq!(observation.before_tokens, Some(2000));
        assert_eq!(observation.after_tokens, Some(1200));
    }

    #[test]
    fn serialize_restores_entry_log_before_emitting_json() {
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-3".to_string()),
            summary: "serialized".to_string(),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            created_at: 3,
        });
        state.committed_collapses_mut().clear();

        let value = serialize_context_runtime_state(&state).expect("serialize runtime state");
        assert_eq!(
            value["store"]["collapseCommits"]
                .as_array()
                .map(|items| items.len()),
            Some(1)
        );
        assert_eq!(
            value["store"]["collapseCommits"][0]["summary"].as_str(),
            Some("serialized")
        );
    }
}
