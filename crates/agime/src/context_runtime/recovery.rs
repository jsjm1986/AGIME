use anyhow::Result;
use chrono::Utc;

use crate::conversation::Conversation;
use crate::providers::base::Provider;

use super::collapse::{build_staged_collapse, commit_staged_collapses};
use super::projection::{
    prepared_projection_state, projected_runtime_budget_for_prepared_state,
    refresh_projection_from_entry_log, refresh_projection_only,
};
use super::session_memory::build_session_memory_state;
use super::store::{has_persistent_runtime_state, prune_after_rewind};
use super::types::{
    ContextRuntimeAdvance, ContextRuntimeAdvanceKind, ContextRuntimeRecovery,
    ContextRuntimeRecoveryKind, ContextRuntimeState, PostCompactValidation, RuntimeBudget,
    SessionMemoryPolicyKind, SessionMemoryState,
};

fn apply_session_memory_compaction(
    conversation: &Conversation,
    state: &mut ContextRuntimeState,
    memory: SessionMemoryState,
    compact_reason: &str,
    recovery_result: Option<&str>,
) -> bool {
    state.set_session_memory(Some(memory));
    state.clear_committed_collapses();
    state.set_staged_snapshot(None);
    state.runtime_compactions = state.runtime_compactions.saturating_add(1);
    state.last_compact_reason = Some(compact_reason.to_string());
    if let Some(result) = recovery_result {
        state.last_recovery_result = Some(result.to_string());
    }
    post_compact_cleanup(conversation, state)
}

fn apply_staged_collapse_commit(
    conversation: &Conversation,
    state: &mut ContextRuntimeState,
    preserved_reason: &str,
    recovery_result: Option<&str>,
) -> bool {
    commit_staged_collapses(conversation, state, preserved_reason, recovery_result);
    post_compact_cleanup(conversation, state)
}

fn post_compact_cleanup(conversation: &Conversation, state: &mut ContextRuntimeState) -> bool {
    let had_projection_entries =
        !state.snip_entries().is_empty() || !state.microcompact_entries().is_empty();
    state.clear_projection_entries();
    let projection_changed = refresh_projection_from_entry_log(conversation, state);
    had_projection_entries || projection_changed
}

fn retained_range(state: &ContextRuntimeState) -> (Option<usize>, Option<usize>) {
    if let Some(memory) = state.session_memory() {
        return (
            Some(memory.preserved_start_index),
            Some(memory.preserved_end_index),
        );
    }
    if let Some(segment) = state.preserved_segment() {
        return (Some(segment.start_index), Some(segment.end_index));
    }
    (None, None)
}

fn record_post_compact_validation(
    state: &mut ContextRuntimeState,
    compact_kind: &str,
    reason: &str,
    before_projected_tokens: usize,
    after_budget: RuntimeBudget,
    threshold_override: Option<f64>,
    cleanup_triggered: bool,
) {
    let (retained_start_index, retained_end_index) = retained_range(state);
    state.last_post_compact_validation = Some(PostCompactValidation {
        compact_kind: compact_kind.to_string(),
        reason: reason.to_string(),
        before_projected_tokens,
        after_projected_tokens: after_budget.projected_tokens,
        threshold_tokens: after_budget.auto_compact_threshold_tokens(threshold_override),
        retained_start_index,
        retained_end_index,
        cleanup_triggered,
        still_over_threshold: after_budget.should_auto_session_memory(threshold_override),
        validated_at: Utc::now().timestamp(),
    });
}

async fn build_overflow_session_memory_if_possible(
    provider: &dyn Provider,
    conversation: &Conversation,
    state: &ContextRuntimeState,
) -> Result<Option<SessionMemoryState>> {
    match build_session_memory_state(
        provider,
        conversation,
        state,
        SessionMemoryPolicyKind::Overflow,
        state.session_memory(),
    )
    .await
    {
        Ok(memory) => Ok(Some(memory)),
        Err(err) if err.to_string().contains("not enough prefix messages") => Ok(None),
        Err(err) => Err(err),
    }
}

async fn build_session_memory_if_possible(
    provider: &dyn Provider,
    conversation: &Conversation,
    state: &ContextRuntimeState,
    policy_kind: SessionMemoryPolicyKind,
) -> Result<Option<SessionMemoryState>> {
    match build_session_memory_state(
        provider,
        conversation,
        state,
        policy_kind,
        state.session_memory(),
    )
    .await
    {
        Ok(memory) => Ok(Some(memory)),
        Err(err) if err.to_string().contains("not enough prefix messages") => Ok(None),
        Err(err) => Err(err),
    }
}

async fn build_overflow_session_memory_with_full_window_retry(
    provider: &dyn Provider,
    conversation: &Conversation,
    state: &mut ContextRuntimeState,
) -> Result<SessionMemoryState> {
    match build_session_memory_state(
        provider,
        conversation,
        state,
        SessionMemoryPolicyKind::Overflow,
        state.session_memory(),
    )
    .await
    {
        Ok(memory) => Ok(memory),
        Err(err)
            if err.to_string().contains("not enough prefix messages")
                && (!state.committed_collapses().is_empty()
                    || state.preserved_segment().is_some()) =>
        {
            state.clear_committed_collapses();
            state.set_compact_boundary(None);
            state.set_preserved_segment(None);
            state.clear_projection_entries();
            refresh_projection_only(conversation, state);
            build_session_memory_state(
                provider,
                conversation,
                state,
                SessionMemoryPolicyKind::Overflow,
                state.session_memory(),
            )
            .await
        }
        Err(err) => Err(err),
    }
}

async fn validate_post_compact_budget(
    provider: &dyn Provider,
    conversation: &Conversation,
    state: &mut ContextRuntimeState,
    compact_kind: &str,
    reason: &str,
    before_projected_tokens: usize,
    threshold_override: Option<f64>,
    cleanup_triggered: bool,
) -> Result<Option<ContextRuntimeAdvanceKind>> {
    let mut after_budget =
        projected_runtime_budget_for_prepared_state(provider, conversation, state);
    record_post_compact_validation(
        state,
        compact_kind,
        reason,
        before_projected_tokens,
        after_budget,
        threshold_override,
        cleanup_triggered,
    );

    if !after_budget.should_auto_session_memory(threshold_override) {
        return Ok(None);
    }

    if state.staged_snapshot().is_some() && state.session_memory().is_none() {
        let before_tokens = after_budget.projected_tokens;
        let cleanup = apply_staged_collapse_commit(
            conversation,
            state,
            "post_compact_staged_collapse",
            Some("post_compact_staged_collapse"),
        );
        after_budget = projected_runtime_budget_for_prepared_state(provider, conversation, state);
        record_post_compact_validation(
            state,
            "collapse_commit",
            "post_compact_staged_collapse",
            before_tokens,
            after_budget,
            threshold_override,
            cleanup,
        );
    }

    if !after_budget.should_auto_session_memory(threshold_override) {
        return Ok(None);
    }

    if state.session_memory().is_none() {
        let before_tokens = after_budget.projected_tokens;
        let Some(memory) =
            build_overflow_session_memory_if_possible(provider, conversation, state).await?
        else {
            return Ok(None);
        };
        let cleanup = apply_session_memory_compaction(
            conversation,
            state,
            memory,
            "post_compact_session_memory",
            Some("post_compact_session_memory"),
        );
        after_budget = projected_runtime_budget_for_prepared_state(provider, conversation, state);
        record_post_compact_validation(
            state,
            "session_memory",
            "post_compact_session_memory",
            before_tokens,
            after_budget,
            threshold_override,
            cleanup,
        );
        return Ok(Some(ContextRuntimeAdvanceKind::SessionMemoryCompaction));
    }

    Ok(None)
}

pub async fn maybe_advance_runtime_state(
    provider: &dyn Provider,
    conversation: &Conversation,
    state: &mut ContextRuntimeState,
    manual_compact: bool,
    threshold_override: Option<f64>,
) -> Result<ContextRuntimeAdvance> {
    let projection_changed = refresh_projection_only(conversation, state);
    let budget = projected_runtime_budget_for_prepared_state(provider, conversation, state);
    let usage_ratio = budget.usage_ratio();

    if manual_compact {
        let memory = build_session_memory_state(
            provider,
            conversation,
            state,
            SessionMemoryPolicyKind::Manual,
            state.session_memory(),
        )
        .await?;
        let cleanup = apply_session_memory_compaction(
            conversation,
            state,
            memory,
            "manual_session_memory",
            None,
        );
        validate_post_compact_budget(
            provider,
            conversation,
            state,
            "session_memory",
            "manual_session_memory",
            budget.projected_tokens,
            threshold_override,
            cleanup,
        )
        .await?;

        return Ok(ContextRuntimeAdvance {
            kind: ContextRuntimeAdvanceKind::SessionMemoryCompaction,
            usage_ratio,
        });
    }

    if state.staged_snapshot().is_some()
        && state.session_memory().is_none()
        && budget.should_commit_staged_collapse()
    {
        let cleanup = apply_staged_collapse_commit(conversation, state, "committed_collapse", None);
        let escalated = validate_post_compact_budget(
            provider,
            conversation,
            state,
            "collapse_commit",
            "committed_collapse",
            budget.projected_tokens,
            threshold_override,
            cleanup,
        )
        .await?;
        return Ok(ContextRuntimeAdvance {
            kind: escalated.unwrap_or(ContextRuntimeAdvanceKind::CommittedCollapse),
            usage_ratio,
        });
    }

    if budget.should_auto_session_memory(threshold_override) {
        let Some(memory) = build_session_memory_if_possible(
            provider,
            conversation,
            state,
            SessionMemoryPolicyKind::Auto,
        )
        .await?
        else {
            return Ok(ContextRuntimeAdvance {
                kind: if projection_changed {
                    ContextRuntimeAdvanceKind::ProjectionRefresh
                } else {
                    ContextRuntimeAdvanceKind::Noop
                },
                usage_ratio,
            });
        };
        let cleanup = apply_session_memory_compaction(
            conversation,
            state,
            memory,
            "auto_session_memory",
            None,
        );
        validate_post_compact_budget(
            provider,
            conversation,
            state,
            "session_memory",
            "auto_session_memory",
            budget.projected_tokens,
            threshold_override,
            cleanup,
        )
        .await?;

        return Ok(ContextRuntimeAdvance {
            kind: ContextRuntimeAdvanceKind::SessionMemoryCompaction,
            usage_ratio,
        });
    }

    if budget.should_stage()
        && state.session_memory().is_none()
        && state.staged_snapshot().is_none()
    {
        if let Some(snapshot) = build_staged_collapse(
            provider,
            conversation,
            state,
            usage_ratio,
            budget.projected_tokens,
        )
        .await?
        {
            state.set_staged_snapshot(Some(snapshot));
            state.last_compact_reason = Some("staged_collapse".to_string());
            return Ok(ContextRuntimeAdvance {
                kind: ContextRuntimeAdvanceKind::StagedCollapse,
                usage_ratio,
            });
        }
    }

    if projection_changed {
        return Ok(ContextRuntimeAdvance {
            kind: ContextRuntimeAdvanceKind::ProjectionRefresh,
            usage_ratio,
        });
    }

    Ok(ContextRuntimeAdvance {
        kind: ContextRuntimeAdvanceKind::Noop,
        usage_ratio,
    })
}

pub async fn should_auto_compact(
    provider: &dyn Provider,
    conversation: &Conversation,
    state: &ContextRuntimeState,
    threshold_override: Option<f64>,
) -> Result<bool> {
    let preview_state = prepared_projection_state(conversation, state);
    let budget =
        projected_runtime_budget_for_prepared_state(provider, conversation, &preview_state);
    Ok(
        (preview_state.staged_snapshot().is_some() && budget.should_commit_staged_collapse())
            || budget.should_auto_session_memory(threshold_override),
    )
}

pub async fn recover_on_overflow(
    provider: &dyn Provider,
    conversation: &Conversation,
    state: &mut ContextRuntimeState,
) -> Result<ContextRuntimeRecovery> {
    refresh_projection_only(conversation, state);
    let before_budget = projected_runtime_budget_for_prepared_state(provider, conversation, state);

    if state.staged_snapshot().is_some() {
        let cleanup = apply_staged_collapse_commit(
            conversation,
            state,
            "collapse_commit",
            Some("staged_collapse_committed"),
        );
        let escalated = validate_post_compact_budget(
            provider,
            conversation,
            state,
            "collapse_commit",
            "staged_collapse_committed",
            before_budget.projected_tokens,
            None,
            cleanup,
        )
        .await?;
        return Ok(ContextRuntimeRecovery {
            kind: if escalated == Some(ContextRuntimeAdvanceKind::SessionMemoryCompaction) {
                ContextRuntimeRecoveryKind::SessionMemoryCompaction
            } else {
                ContextRuntimeRecoveryKind::StagedCollapseCommitted
            },
        });
    }

    let memory =
        build_overflow_session_memory_with_full_window_retry(provider, conversation, state).await?;
    let cleanup = apply_session_memory_compaction(
        conversation,
        state,
        memory,
        "session_memory_compaction",
        Some("session_memory_compaction"),
    );
    validate_post_compact_budget(
        provider,
        conversation,
        state,
        "session_memory",
        "session_memory_compaction",
        before_budget.projected_tokens,
        None,
        cleanup,
    )
    .await?;

    Ok(ContextRuntimeRecovery {
        kind: ContextRuntimeRecoveryKind::SessionMemoryCompaction,
    })
}

pub fn rewind_context_runtime_state(
    conversation: &Conversation,
    state: &ContextRuntimeState,
) -> Option<ContextRuntimeState> {
    let mut next = state.clone();
    prune_after_rewind(conversation, &mut next);
    let _ = refresh_projection_only(conversation, &mut next);

    if has_persistent_runtime_state(&next) {
        Some(next)
    } else {
        None
    }
}
