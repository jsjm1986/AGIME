use crate::conversation::Conversation;

use super::session_memory::estimate_message_tokens;
use super::types::{
    rebuild_store_from_entry_log, CollapseCommit, CollapseSnapshot, CompactBoundaryMetadata,
    CompactDirection, ContextRuntimeState, ContextRuntimeStore, ContextRuntimeStoreEntry,
    MicrocompactState, PreservedSegmentMetadata, SessionMemoryState, SnipRemovalState,
    CURRENT_CONTEXT_RUNTIME_SCHEMA_VERSION,
};

impl ContextRuntimeStore {
    pub(crate) fn restored_from_entries(&self) -> Self {
        if self.entry_log.is_empty() {
            self.clone()
        } else {
            rebuild_store_from_entry_log(self.clone())
        }
    }

    pub(crate) fn with_backfilled_entry_log(&self) -> Self {
        if !self.entry_log.is_empty() || self.is_empty() {
            return self.clone();
        }

        let mut next = self.clone();
        next.entry_log = synthesize_entry_log_from_live_store(self);
        rebuild_store_from_entry_log(next)
    }

    pub(crate) fn append_collapse_commit(&mut self, commit: CollapseCommit) {
        self.entry_log
            .push(ContextRuntimeStoreEntry::CollapseCommitAdded {
                commit: commit.clone(),
            });
        self.collapse_commits.push(commit);
    }

    pub(crate) fn preferred_compact_direction(&self) -> Option<CompactDirection> {
        if self.session_memory.is_some() {
            return Some(CompactDirection::UpTo);
        }
        if !self.collapse_commits.is_empty() {
            let latest = self
                .collapse_commits
                .iter()
                .max_by_key(|entry| entry.created_at)
                .map(|entry| entry.direction)
                .unwrap_or_default();
            return Some(latest);
        }
        if let Some(snapshot) = &self.staged_snapshot {
            return Some(snapshot.direction);
        }
        self.boundary.as_ref().map(|boundary| boundary.direction)
    }

    pub(crate) fn normalize_active_collapse_direction(&mut self) {
        let Some(direction) = self.preferred_compact_direction() else {
            return;
        };

        let has_mixed_commits = self
            .collapse_commits
            .iter()
            .any(|entry| entry.direction != direction);
        let staged_conflicts = self
            .staged_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.direction != direction)
            && (self.session_memory.is_some()
                || self.boundary.is_some()
                || !self.collapse_commits.is_empty());

        if !has_mixed_commits && !staged_conflicts {
            return;
        }

        if has_mixed_commits {
            self.collapse_commits
                .retain(|entry| entry.direction == direction);
        }
        if staged_conflicts {
            self.staged_snapshot = None;
        }

        self.entry_log = synthesize_entry_log_from_live_store(self);
    }

    pub(crate) fn clear_committed_collapses(&mut self) {
        if self.collapse_commits.is_empty() {
            return;
        }
        self.entry_log
            .push(ContextRuntimeStoreEntry::CollapseCommitsCleared);
        self.collapse_commits.clear();
    }

    pub(crate) fn set_staged_snapshot(&mut self, snapshot: Option<CollapseSnapshot>) {
        if self.staged_snapshot == snapshot {
            return;
        }
        match snapshot.clone() {
            Some(value) => self
                .entry_log
                .push(ContextRuntimeStoreEntry::StagedSnapshotSet { snapshot: value }),
            None => self
                .entry_log
                .push(ContextRuntimeStoreEntry::StagedSnapshotCleared),
        }
        self.staged_snapshot = snapshot;
    }

    pub(crate) fn set_session_memory(&mut self, session_memory: Option<SessionMemoryState>) {
        if self.session_memory == session_memory {
            return;
        }
        match session_memory.clone() {
            Some(value) => self
                .entry_log
                .push(ContextRuntimeStoreEntry::SessionMemorySet {
                    session_memory: value,
                }),
            None => self
                .entry_log
                .push(ContextRuntimeStoreEntry::SessionMemoryCleared),
        }
        self.session_memory = session_memory;
    }

    pub(crate) fn set_snip_entries(&mut self, entries: Vec<SnipRemovalState>) {
        if self.snip_entries == entries {
            return;
        }
        if entries.is_empty() {
            self.entry_log
                .push(ContextRuntimeStoreEntry::SnipEntriesCleared);
        } else {
            self.entry_log
                .push(ContextRuntimeStoreEntry::SnipEntriesSet {
                    entries: entries.clone(),
                });
        }
        self.snip_entries = entries;
    }

    pub(crate) fn set_microcompact_entries(&mut self, entries: Vec<MicrocompactState>) {
        if self.microcompact_entries == entries {
            return;
        }
        if entries.is_empty() {
            self.entry_log
                .push(ContextRuntimeStoreEntry::MicrocompactEntriesCleared);
        } else {
            self.entry_log
                .push(ContextRuntimeStoreEntry::MicrocompactEntriesSet {
                    entries: entries.clone(),
                });
        }
        self.microcompact_entries = entries;
    }

    pub(crate) fn set_preserved_segment(
        &mut self,
        preserved_segment: Option<PreservedSegmentMetadata>,
    ) {
        if self.preserved_segment == preserved_segment {
            return;
        }
        self.preserved_segment = preserved_segment;
    }

    pub(crate) fn set_boundary(&mut self, boundary: Option<CompactBoundaryMetadata>) {
        if self.boundary == boundary {
            return;
        }
        self.boundary = boundary;
    }

    pub(crate) fn clear_projection_entries(&mut self) {
        if self.snip_entries.is_empty() && self.microcompact_entries.is_empty() {
            return;
        }
        self.entry_log
            .push(ContextRuntimeStoreEntry::SnipEntriesCleared);
        self.entry_log
            .push(ContextRuntimeStoreEntry::MicrocompactEntriesCleared);
        self.snip_entries.clear();
        self.microcompact_entries.clear();
    }

    pub(crate) fn clear_structural_runtime_state(&mut self) {
        self.clear_committed_collapses();
        self.set_staged_snapshot(None);
        self.set_session_memory(None);
        self.set_boundary(None);
        self.set_preserved_segment(None);
        self.clear_projection_entries();
    }

    pub(crate) fn projection_window(&self, total: usize) -> (usize, usize, CompactDirection) {
        if total == 0 {
            return (0, 0, CompactDirection::UpTo);
        }

        if let Some(memory) = &self.session_memory {
            return (
                memory.preserved_start_index.min(total.saturating_sub(1)),
                total.saturating_sub(1),
                CompactDirection::UpTo,
            );
        }

        if let Some(segment) = &self.preserved_segment {
            if self
                .boundary
                .as_ref()
                .is_some_and(|boundary| boundary.direction == CompactDirection::From)
            {
                return (
                    0,
                    segment.end_index.min(total.saturating_sub(1)),
                    CompactDirection::From,
                );
            }
            return (
                segment.start_index.min(total.saturating_sub(1)),
                total.saturating_sub(1),
                CompactDirection::UpTo,
            );
        }

        if let Some(boundary) = &self.boundary {
            return match boundary.direction {
                CompactDirection::UpTo => (
                    boundary.tail_index.min(total.saturating_sub(1)),
                    total.saturating_sub(1),
                    CompactDirection::UpTo,
                ),
                CompactDirection::From => (
                    0,
                    boundary.head_index.min(total.saturating_sub(1)),
                    CompactDirection::From,
                ),
            };
        }

        (0, total.saturating_sub(1), CompactDirection::UpTo)
    }

    pub(crate) fn reconcile_for_conversation(&mut self, conversation: &Conversation) {
        let messages = conversation.messages();
        let total = messages.len();
        let id_to_index = messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| message.id.as_ref().map(|id| (id.clone(), index)))
            .collect::<std::collections::HashMap<_, _>>();

        self.collapse_commits.retain_mut(|entry| {
            let start_index = resolve_index_by_id(&id_to_index, entry.start_message_id.as_deref());
            let end_index = resolve_end_index_by_id(&id_to_index, entry.end_message_id.as_deref());
            if entry.start_message_id.is_some() && start_index.is_none() {
                return false;
            }
            if entry.end_message_id.is_some() && end_index.is_none() {
                return false;
            }
            if let Some(index) = start_index {
                entry.start_index = index;
            }
            if let Some(index) = end_index {
                entry.end_index = index;
            }
            if entry.start_message_id.is_none() {
                entry.start_message_id = message_id_at(messages, entry.start_index);
            }
            if entry.end_message_id.is_none() {
                entry.end_message_id =
                    end_message_id_at(messages, entry.end_index.saturating_sub(1));
            }
            entry.start_index < entry.end_index && entry.end_index <= total
        });

        if let Some(snapshot) = self.staged_snapshot.as_mut() {
            let start_index =
                resolve_index_by_id(&id_to_index, snapshot.start_message_id.as_deref());
            let end_index =
                resolve_end_index_by_id(&id_to_index, snapshot.end_message_id.as_deref());
            if let Some(index) = start_index {
                snapshot.start_index = index;
            }
            if let Some(index) = end_index {
                snapshot.end_index = index;
            }
            if snapshot.start_message_id.is_none() {
                snapshot.start_message_id = message_id_at(messages, snapshot.start_index);
            }
            if snapshot.end_message_id.is_none() {
                snapshot.end_message_id =
                    end_message_id_at(messages, snapshot.end_index.saturating_sub(1));
            }
        }
        if self.staged_snapshot.as_ref().is_some_and(|snapshot| {
            !(snapshot.start_index < snapshot.end_index && snapshot.end_index <= total)
        }) {
            self.set_staged_snapshot(None);
        }

        self.snip_entries.retain_mut(|entry| {
            if !entry.removed_message_ids.is_empty() {
                let mut remapped = entry
                    .removed_message_ids
                    .iter()
                    .filter_map(|id| id_to_index.get(id).copied())
                    .collect::<Vec<_>>();
                remapped.sort_unstable();
                remapped.dedup();
                if !remapped.is_empty() {
                    entry.removed_indexes = remapped;
                } else {
                    return false;
                }
            } else {
                entry.removed_indexes.retain(|index| *index < total);
                entry.removed_message_ids = entry
                    .removed_indexes
                    .iter()
                    .filter_map(|index| message_id_at(messages, *index))
                    .collect();
            }
            entry.removed_count = entry.removed_indexes.len();
            !entry.removed_indexes.is_empty()
        });

        self.microcompact_entries.retain_mut(|entry| {
            let message_index = resolve_index_by_id(&id_to_index, entry.message_id.as_deref());
            if let Some(index) = message_index {
                entry.message_index = index;
            }
            if entry.message_id.is_none() {
                entry.message_id = message_id_at(messages, entry.message_index);
            }
            entry.message_index < total && !entry.tool_response_ids.is_empty()
        });

        let mut invalidate_current_session_memory = false;
        if let Some(memory) = self.session_memory.as_mut() {
            let preserved_start =
                resolve_index_by_id(&id_to_index, memory.preserved_start_message_id.as_deref());
            let preserved_end =
                resolve_index_by_id(&id_to_index, memory.preserved_end_message_id.as_deref());
            let tail_anchor =
                resolve_index_by_id(&id_to_index, memory.tail_anchor_message_id.as_deref());
            if (memory.preserved_start_message_id.is_some() && preserved_start.is_none())
                || (memory.preserved_end_message_id.is_some() && preserved_end.is_none())
                || (memory.tail_anchor_message_id.is_some() && tail_anchor.is_none())
            {
                invalidate_current_session_memory = true;
            } else {
                if let Some(index) = preserved_start {
                    memory.preserved_start_index = index;
                }
                if let Some(index) = preserved_end {
                    memory.preserved_end_index = index;
                }
                if let Some(index) = tail_anchor {
                    memory.tail_anchor_index = index;
                }
                if memory.summarized_through_message_id.is_none() {
                    memory.summarized_through_message_id = memory
                        .preserved_start_index
                        .checked_sub(1)
                        .and_then(|index| message_id_at(messages, index));
                }
                if memory.preserved_start_message_id.is_none() {
                    memory.preserved_start_message_id =
                        message_id_at(messages, memory.preserved_start_index);
                }
                if memory.preserved_end_message_id.is_none() {
                    memory.preserved_end_message_id =
                        message_id_at(messages, memory.preserved_end_index);
                }
                if memory.tail_anchor_message_id.is_none() {
                    memory.tail_anchor_message_id =
                        message_id_at(messages, memory.tail_anchor_index);
                }
            }
        }
        if invalidate_current_session_memory {
            self.set_session_memory(None);
        }
        if self
            .session_memory
            .as_mut()
            .is_some_and(|memory| !normalize_session_memory_tail(memory, messages))
        {
            self.set_session_memory(None);
        }

        if let Some(boundary) = self.boundary.as_mut() {
            let anchor_index =
                resolve_index_by_id(&id_to_index, boundary.anchor_message_id.as_deref());
            let head_index = resolve_index_by_id(&id_to_index, boundary.head_message_id.as_deref());
            let tail_index = resolve_index_by_id(&id_to_index, boundary.tail_message_id.as_deref());
            if let Some(index) = anchor_index {
                boundary.anchor_index = index;
            }
            if let Some(index) = head_index {
                boundary.head_index = index;
            }
            if let Some(index) = tail_index {
                boundary.tail_index = index;
            }
            if boundary.anchor_message_id.is_none() {
                boundary.anchor_message_id = message_id_at(messages, boundary.anchor_index);
            }
            if boundary.head_message_id.is_none() {
                boundary.head_message_id = message_id_at(messages, boundary.head_index);
            }
            if boundary.tail_message_id.is_none() {
                boundary.tail_message_id = message_id_at(messages, boundary.tail_index);
            }
        }
        if self.boundary.as_ref().is_some_and(|boundary| {
            boundary.anchor_index >= total
                || boundary.head_index >= total
                || boundary.tail_index >= total
        }) {
            self.set_boundary(None);
        }

        if let Some(segment) = self.preserved_segment.as_mut() {
            let start_index =
                resolve_index_by_id(&id_to_index, segment.start_message_id.as_deref());
            let end_index = resolve_index_by_id(&id_to_index, segment.end_message_id.as_deref());
            let tail_anchor =
                resolve_index_by_id(&id_to_index, segment.tail_anchor_message_id.as_deref());
            if let Some(index) = start_index {
                segment.start_index = index;
            }
            if let Some(index) = end_index {
                segment.end_index = index;
            }
            if let Some(index) = tail_anchor {
                segment.tail_anchor_index = index;
            }
            if segment.start_message_id.is_none() {
                segment.start_message_id = message_id_at(messages, segment.start_index);
            }
            if segment.end_message_id.is_none() {
                segment.end_message_id = message_id_at(messages, segment.end_index);
            }
            if segment.tail_anchor_message_id.is_none() {
                segment.tail_anchor_message_id = message_id_at(messages, segment.tail_anchor_index);
            }
        }
        if self
            .preserved_segment
            .as_ref()
            .is_some_and(|segment| segment.start_index >= total || segment.end_index >= total)
        {
            self.set_preserved_segment(None);
        }
    }

    pub(crate) fn refresh_boundary(&mut self, total: usize) {
        if total == 0 {
            self.set_boundary(None);
            return;
        }

        let created_at = self
            .boundary
            .as_ref()
            .map(|boundary| boundary.created_at)
            .unwrap_or_else(|| chrono::Utc::now().timestamp());

        if let Some(memory) = &self.session_memory {
            self.set_boundary(Some(CompactBoundaryMetadata {
                anchor_index: 0,
                head_index: 0,
                tail_index: memory.preserved_start_index.min(total.saturating_sub(1)),
                direction: CompactDirection::UpTo,
                anchor_message_id: None,
                head_message_id: None,
                tail_message_id: memory.preserved_start_message_id.clone(),
                created_at,
            }));
            return;
        }

        if !self.collapse_commits.is_empty() {
            let latest_commit = self
                .collapse_commits
                .iter()
                .max_by_key(|entry| entry.created_at)
                .cloned()
                .unwrap_or_default();
            match latest_commit.direction {
                CompactDirection::UpTo => {
                    let first_start = self
                        .collapse_commits
                        .iter()
                        .filter(|entry| entry.direction == CompactDirection::UpTo)
                        .map(|entry| entry.start_index)
                        .min()
                        .unwrap_or_default();
                    let preserved_start = self
                        .collapse_commits
                        .iter()
                        .filter(|entry| entry.direction == CompactDirection::UpTo)
                        .map(|entry| entry.end_index)
                        .max()
                        .unwrap_or_default()
                        .min(total.saturating_sub(1));
                    self.set_boundary(Some(CompactBoundaryMetadata {
                        anchor_index: first_start.min(total.saturating_sub(1)),
                        head_index: first_start.min(total.saturating_sub(1)),
                        tail_index: preserved_start,
                        direction: CompactDirection::UpTo,
                        anchor_message_id: self
                            .collapse_commits
                            .iter()
                            .find(|entry| entry.start_index == first_start)
                            .and_then(|entry| entry.start_message_id.clone()),
                        head_message_id: self
                            .collapse_commits
                            .iter()
                            .find(|entry| entry.start_index == first_start)
                            .and_then(|entry| entry.start_message_id.clone()),
                        tail_message_id: None,
                        created_at,
                    }));
                }
                CompactDirection::From => {
                    let preserved_end = self
                        .collapse_commits
                        .iter()
                        .filter(|entry| entry.direction == CompactDirection::From)
                        .map(|entry| entry.start_index.saturating_sub(1))
                        .min()
                        .unwrap_or_default()
                        .min(total.saturating_sub(1));
                    let collapsed_tail = self
                        .collapse_commits
                        .iter()
                        .filter(|entry| entry.direction == CompactDirection::From)
                        .map(|entry| entry.end_index.saturating_sub(1))
                        .max()
                        .unwrap_or_default()
                        .min(total.saturating_sub(1));
                    self.set_boundary(Some(CompactBoundaryMetadata {
                        anchor_index: latest_commit.start_index.min(total.saturating_sub(1)),
                        head_index: preserved_end,
                        tail_index: collapsed_tail,
                        direction: CompactDirection::From,
                        anchor_message_id: latest_commit.start_message_id.clone(),
                        head_message_id: None,
                        tail_message_id: self
                            .collapse_commits
                            .iter()
                            .filter(|entry| entry.direction == CompactDirection::From)
                            .find(|entry| entry.end_index.saturating_sub(1) == collapsed_tail)
                            .and_then(|entry| entry.end_message_id.clone()),
                        created_at,
                    }));
                }
            }
            return;
        }

        self.set_boundary(None);
    }

    pub(crate) fn refresh_preserved_segment(&mut self, total: usize) {
        if total == 0 {
            self.set_preserved_segment(None);
            return;
        }

        if let Some(memory) = &self.session_memory {
            let reason = self
                .preserved_segment
                .as_ref()
                .map(|segment| segment.reason.as_str())
                .filter(|reason| reason.contains("session_memory"))
                .unwrap_or("session_memory")
                .to_string();
            self.set_preserved_segment(Some(PreservedSegmentMetadata {
                start_index: memory.preserved_start_index.min(total.saturating_sub(1)),
                end_index: total.saturating_sub(1),
                tail_anchor_index: memory.tail_anchor_index.min(total.saturating_sub(1)),
                start_message_id: memory.preserved_start_message_id.clone(),
                end_message_id: memory.preserved_end_message_id.clone(),
                tail_anchor_message_id: memory.tail_anchor_message_id.clone(),
                reason,
            }));
            return;
        }

        let boundary_is_from = self
            .boundary
            .as_ref()
            .is_some_and(|boundary| boundary.direction == CompactDirection::From);
        if let Some(segment) = self.preserved_segment.as_mut() {
            if boundary_is_from {
                segment.start_index = 0;
                segment.end_index = segment.end_index.min(total.saturating_sub(1));
                segment.tail_anchor_index = segment.end_index.min(total.saturating_sub(1));
            } else {
                segment.start_index = segment.start_index.min(total.saturating_sub(1));
                segment.end_index = total.saturating_sub(1).max(segment.start_index);
            }
            segment.tail_anchor_index = segment.tail_anchor_index.min(total.saturating_sub(1));
            return;
        }

        if let Some(boundary) = &self.boundary {
            match boundary.direction {
                CompactDirection::UpTo => {
                    let tail_anchor = boundary.tail_index.min(total.saturating_sub(1));
                    self.set_preserved_segment(Some(PreservedSegmentMetadata {
                        start_index: tail_anchor,
                        end_index: total.saturating_sub(1),
                        tail_anchor_index: tail_anchor,
                        start_message_id: boundary.tail_message_id.clone(),
                        end_message_id: boundary.tail_message_id.clone(),
                        tail_anchor_message_id: boundary.tail_message_id.clone(),
                        reason: "boundary_tail".to_string(),
                    }));
                }
                CompactDirection::From => {
                    let head_anchor = boundary.head_index.min(total.saturating_sub(1));
                    self.set_preserved_segment(Some(PreservedSegmentMetadata {
                        start_index: 0,
                        end_index: head_anchor,
                        tail_anchor_index: head_anchor,
                        start_message_id: None,
                        end_message_id: boundary.head_message_id.clone(),
                        tail_anchor_message_id: boundary.head_message_id.clone(),
                        reason: "boundary_head".to_string(),
                    }));
                }
            }
        }
    }

    pub(crate) fn finalize_projection_refresh(&mut self, total: usize) {
        self.normalize_active_collapse_direction();
        self.refresh_boundary(total);
        self.refresh_preserved_segment(total);
    }
}

pub(crate) fn restore_from_entries(state: &ContextRuntimeState) -> ContextRuntimeState {
    let mut next = state.clone();
    next.store = next
        .store
        .with_backfilled_entry_log()
        .restored_from_entries();
    next
}

pub(crate) fn audit_state_for_loaded_conversation(
    conversation: Option<&Conversation>,
    state: &mut ContextRuntimeState,
) {
    state.store = state
        .store
        .with_backfilled_entry_log()
        .restored_from_entries();
    state.store.normalize_active_collapse_direction();

    match conversation {
        Some(conversation) if conversation.messages().is_empty() => {
            state.store.clear_structural_runtime_state();
            state.last_projection_stats = None;
            state.last_compact_reason = None;
            state.last_recovery_result = None;
        }
        Some(conversation) => {
            state.store.reconcile_for_conversation(conversation);
            crate::context_runtime::projection::refresh_projection_state(conversation, state);
        }
        None => {}
    }
}

pub(crate) fn current_visible_working_window(
    total: usize,
    state: &ContextRuntimeState,
) -> (usize, usize) {
    if total == 0 {
        return (0, 0);
    }
    let (slice_start, slice_end, direction) = state.store.projection_window(total);
    match direction {
        CompactDirection::UpTo => (slice_start.min(total), total),
        CompactDirection::From => (0, slice_end.saturating_add(1).min(total)),
    }
}

pub(crate) fn preferred_compact_direction(state: &ContextRuntimeState) -> Option<CompactDirection> {
    state.store.preferred_compact_direction()
}

pub(crate) fn has_persistent_runtime_state(state: &ContextRuntimeState) -> bool {
    let restored = restore_from_entries(state);
    restored.session_memory().is_some()
        || !restored.committed_collapses().is_empty()
        || restored.staged_snapshot().is_some()
}

pub(crate) fn relink_state_for_replaced_conversation(
    old_conversation: &Conversation,
    new_conversation: &Conversation,
    state: &mut ContextRuntimeState,
) {
    if new_conversation.messages().is_empty() {
        state.store.clear_structural_runtime_state();
        return;
    }
    if old_conversation.messages().is_empty() {
        return;
    }

    state.store = state
        .store
        .with_backfilled_entry_log()
        .restored_from_entries();
    state.store.reconcile_for_conversation(old_conversation);

    let old_to_new = build_old_to_new_index_map(old_conversation, new_conversation);
    let new_messages = new_conversation.messages();

    state.store.staged_snapshot = None;

    state.store.collapse_commits.retain_mut(|commit| {
        let old_commit_len = commit.end_index.saturating_sub(commit.start_index);
        let remapped = (commit.start_index..commit.end_index)
            .filter_map(|index| old_to_new.get(&index).copied())
            .collect::<Vec<_>>();
        let Some(new_start) = old_to_new.get(&commit.start_index).copied() else {
            return false;
        };
        let Some(old_end_last) = commit.end_index.checked_sub(1) else {
            return false;
        };
        let Some(new_end_last) = old_to_new.get(&old_end_last).copied() else {
            return false;
        };
        let remapped_is_contiguous = remapped
            .windows(2)
            .all(|window| window[1] == window[0].saturating_add(1));
        let range_is_stable = remapped.len() == old_commit_len
            && remapped_is_contiguous
            && commit_range_is_compatible(
                &old_to_new,
                commit.direction,
                commit.start_index,
                new_start,
                new_end_last,
                new_messages.len(),
            );
        if !range_is_stable {
            return false;
        }
        commit.start_index = new_start;
        commit.end_index = new_end_last.saturating_add(1);
        commit.start_message_id = new_messages
            .get(new_start)
            .and_then(|message| message.id.clone());
        commit.end_message_id = new_messages
            .get(new_end_last)
            .and_then(|message| message.id.clone());
        commit.start_index < commit.end_index && commit.end_index <= new_messages.len()
    });

    if let Some(memory) = state.store.session_memory.as_mut() {
        let old_preserved_len = memory
            .preserved_end_index
            .saturating_sub(memory.preserved_start_index)
            .saturating_add(1);
        let remapped_preserved = (memory.preserved_start_index..=memory.preserved_end_index)
            .filter_map(|index| old_to_new.get(&index).copied())
            .collect::<Vec<_>>();
        let preserved_start = remapped_preserved.first().copied();
        let preserved_end = remapped_preserved.last().copied();
        let tail_anchor = old_to_new
            .get(&memory.tail_anchor_index)
            .copied()
            .or(preserved_end);
        let summarized_through = memory
            .preserved_start_index
            .checked_sub(1)
            .and_then(|index| old_to_new.get(&index).copied());
        let mut invalidate_session_memory = false;
        match (preserved_start, preserved_end, tail_anchor) {
            (Some(new_start), Some(new_end), Some(new_tail)) => {
                let preserved_is_contiguous = remapped_preserved
                    .windows(2)
                    .all(|window| window[1] == window[0].saturating_add(1));
                let summary_prefix_is_compatible = session_memory_summary_prefix_is_compatible(
                    &old_to_new,
                    memory.preserved_start_index,
                    new_start,
                );
                if remapped_preserved.len() != old_preserved_len
                    || !preserved_is_contiguous
                    || !summary_prefix_is_compatible
                {
                    invalidate_session_memory = true;
                } else {
                    memory.preserved_start_index = new_start;
                    memory.preserved_end_index = new_end;
                    memory.tail_anchor_index = new_tail;
                    memory.preserved_start_message_id = new_messages
                        .get(new_start)
                        .and_then(|message| message.id.clone());
                    memory.preserved_end_message_id = new_messages
                        .get(new_end)
                        .and_then(|message| message.id.clone());
                    memory.tail_anchor_message_id = new_messages
                        .get(new_tail)
                        .and_then(|message| message.id.clone());
                    memory.summarized_through_message_id = summarized_through
                        .and_then(|index| new_messages.get(index))
                        .and_then(|message| message.id.clone());
                    if !normalize_session_memory_tail(memory, new_messages) {
                        invalidate_session_memory = true;
                    }
                }
            }
            _ => {
                invalidate_session_memory = true;
            }
        }
        if invalidate_session_memory {
            state.store.session_memory = None;
        }
    }

    state.store.snip_entries.retain_mut(|entry| {
        let mut remapped = entry
            .removed_indexes
            .iter()
            .filter_map(|index| old_to_new.get(index).copied())
            .collect::<Vec<_>>();
        remapped.sort_unstable();
        remapped.dedup();
        if remapped.is_empty() {
            return false;
        }
        entry.removed_indexes = remapped;
        entry.removed_count = entry.removed_indexes.len();
        entry.removed_message_ids = entry
            .removed_indexes
            .iter()
            .filter_map(|index| {
                new_messages
                    .get(*index)
                    .and_then(|message| message.id.clone())
            })
            .collect();
        true
    });

    state.store.microcompact_entries.retain_mut(|entry| {
        let Some(new_index) = old_to_new.get(&entry.message_index).copied() else {
            return false;
        };
        entry.message_index = new_index;
        entry.message_id = new_messages
            .get(new_index)
            .and_then(|message| message.id.clone());
        true
    });

    state.store.boundary = None;
    state.store.preserved_segment = None;
    state.store.entry_log.clear();
}

pub(crate) fn prune_after_rewind(conversation: &Conversation, state: &mut ContextRuntimeState) {
    if conversation.messages().is_empty() {
        state.store.clear_structural_runtime_state();
        state.last_recovery_result = None;
        state.last_compact_reason = None;
        return;
    }
    state.set_staged_snapshot(None);
    state.last_recovery_result = None;
    reconcile_state_for_conversation(conversation, state);
    let total = conversation.messages().len();
    finalize_projection_refresh(total, state);

    if state.session_memory().is_none() && state.committed_collapses().is_empty() {
        state.set_compact_boundary(None);
        state.set_preserved_segment(None);
        state.last_compact_reason = None;
    }
}

pub(crate) fn reconcile_state_for_conversation(
    conversation: &Conversation,
    state: &mut ContextRuntimeState,
) {
    state.store = state
        .store
        .with_backfilled_entry_log()
        .restored_from_entries();
    state.store.normalize_active_collapse_direction();
    if conversation.messages().is_empty() {
        state.store.clear_structural_runtime_state();
        state.last_projection_stats = None;
        state.last_compact_reason = None;
        state.last_recovery_result = None;
        return;
    }
    state.store.reconcile_for_conversation(conversation);
}

fn build_old_to_new_index_map(
    old_conversation: &Conversation,
    new_conversation: &Conversation,
) -> std::collections::HashMap<usize, usize> {
    let old_messages = old_conversation.messages();
    let new_messages = new_conversation.messages();
    let new_id_to_index = new_messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| message.id.as_ref().map(|id| (id.clone(), index)))
        .collect::<std::collections::HashMap<_, _>>();
    let old_keys = message_occurrence_keys(old_messages);
    let new_keys = message_occurrence_keys(new_messages);
    let new_signature_to_index = new_keys
        .into_iter()
        .enumerate()
        .map(|(index, key)| (key, index))
        .collect::<std::collections::HashMap<_, _>>();

    old_messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            if let Some(id) = message.id.as_ref() {
                if let Some(new_index) = new_id_to_index.get(id).copied() {
                    return Some((index, new_index));
                }
            }
            new_signature_to_index
                .get(&old_keys[index])
                .copied()
                .map(|new_index| (index, new_index))
        })
        .collect()
}

fn message_occurrence_keys(
    messages: &[crate::conversation::message::Message],
) -> Vec<(String, usize)> {
    let mut seen = std::collections::HashMap::<String, usize>::new();
    messages
        .iter()
        .map(|message| {
            let signature = format!(
                "{:?}|{}",
                message.role,
                serde_json::to_string(&message.content).unwrap_or_default()
            );
            let ordinal = seen.entry(signature.clone()).or_insert(0usize);
            let key = (signature, *ordinal);
            *ordinal += 1;
            key
        })
        .collect()
}

fn synthesize_entry_log_from_live_store(
    store: &ContextRuntimeStore,
) -> Vec<ContextRuntimeStoreEntry> {
    let mut entry_log = Vec::new();

    if let Some(snapshot) = store.staged_snapshot.clone() {
        entry_log.push(ContextRuntimeStoreEntry::StagedSnapshotSet { snapshot });
    }
    for commit in store.collapse_commits.iter().cloned() {
        entry_log.push(ContextRuntimeStoreEntry::CollapseCommitAdded { commit });
    }
    if let Some(session_memory) = store.session_memory.clone() {
        entry_log.push(ContextRuntimeStoreEntry::SessionMemorySet { session_memory });
    }
    if !store.snip_entries.is_empty() {
        entry_log.push(ContextRuntimeStoreEntry::SnipEntriesSet {
            entries: store.snip_entries.clone(),
        });
    }
    if !store.microcompact_entries.is_empty() {
        entry_log.push(ContextRuntimeStoreEntry::MicrocompactEntriesSet {
            entries: store.microcompact_entries.clone(),
        });
    }

    entry_log
}

fn resolve_index_by_id(
    id_to_index: &std::collections::HashMap<String, usize>,
    message_id: Option<&str>,
) -> Option<usize> {
    message_id.and_then(|id| id_to_index.get(id).copied())
}

fn resolve_end_index_by_id(
    id_to_index: &std::collections::HashMap<String, usize>,
    message_id: Option<&str>,
) -> Option<usize> {
    resolve_index_by_id(id_to_index, message_id).map(|index| index.saturating_add(1))
}

fn message_id_at(
    messages: &[crate::conversation::message::Message],
    index: usize,
) -> Option<String> {
    messages.get(index).and_then(|message| message.id.clone())
}

fn normalize_session_memory_tail(
    memory: &mut SessionMemoryState,
    messages: &[crate::conversation::message::Message],
) -> bool {
    if messages.is_empty() {
        return false;
    }

    let total = messages.len();
    if memory.preserved_start_index >= total {
        return false;
    }

    let preserved_start = memory.preserved_start_index.min(total.saturating_sub(1));
    let preserved_end = total.saturating_sub(1);
    let preserved_slice = &messages[preserved_start..];

    memory.preserved_start_index = preserved_start;
    memory.preserved_end_index = preserved_end;
    memory.tail_anchor_index = preserved_end;
    memory.summarized_through_message_id = preserved_start
        .checked_sub(1)
        .and_then(|index| message_id_at(messages, index));
    memory.preserved_start_message_id = message_id_at(messages, preserved_start);
    memory.preserved_end_message_id = message_id_at(messages, preserved_end);
    memory.tail_anchor_message_id = message_id_at(messages, preserved_end);
    memory.preserved_message_count = preserved_slice.len();
    memory.preserved_token_estimate = preserved_slice.iter().map(estimate_message_tokens).sum();

    true
}

fn session_memory_summary_prefix_is_compatible(
    old_to_new: &std::collections::HashMap<usize, usize>,
    old_preserved_start: usize,
    new_preserved_start: usize,
) -> bool {
    if old_preserved_start == 0 {
        return new_preserved_start == 0;
    }
    if new_preserved_start != old_preserved_start {
        return false;
    }
    (0..old_preserved_start).all(|old_index| old_to_new.get(&old_index).copied() == Some(old_index))
}

fn commit_range_is_compatible(
    old_to_new: &std::collections::HashMap<usize, usize>,
    direction: CompactDirection,
    old_start: usize,
    new_start: usize,
    new_end_last: usize,
    new_total: usize,
) -> bool {
    if new_start != old_start {
        return false;
    }
    match direction {
        CompactDirection::UpTo => true,
        CompactDirection::From => {
            (0..old_start).all(|old_index| old_to_new.get(&old_index).copied() == Some(old_index))
                && new_end_last.saturating_add(1) == new_total
        }
    }
}

fn end_message_id_at(
    messages: &[crate::conversation::message::Message],
    index: usize,
) -> Option<String> {
    messages.get(index).and_then(|message| message.id.clone())
}

pub(crate) fn finalize_projection_refresh(total: usize, state: &mut ContextRuntimeState) {
    state.schema_version = CURRENT_CONTEXT_RUNTIME_SCHEMA_VERSION;
    state.store.finalize_projection_refresh(total);
}
