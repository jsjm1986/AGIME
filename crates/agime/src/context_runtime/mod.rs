mod collapse;
mod microcompact;
mod projection;
mod recovery;
mod session_memory;
mod snip;
mod store;
mod summary;
mod types;

use anyhow::Result;
use chrono::Utc;

use crate::session::extension_data::ExtensionState;
use crate::session::extension_data::TodoState;
use crate::session::SessionManager;

pub(crate) use projection::project_prepared_view;
pub use projection::{project_for_provider, project_view, refresh_projection_only};
pub use recovery::{
    maybe_advance_runtime_state, recover_on_overflow, rewind_context_runtime_state,
    should_auto_compact,
};
pub(crate) use store::{
    audit_state_for_loaded_conversation, has_persistent_runtime_state,
    relink_state_for_replaced_conversation,
};
pub use summary::{
    observe_runtime_transition, serialize_context_runtime_state, summarize_context_runtime_state,
    ContextRuntimeTransitionObservation,
};
pub use types::{
    CollapseCommit, CollapseSnapshot, CompactBoundaryMetadata, CompactDirection,
    ContextRuntimeAdvance, ContextRuntimeAdvanceKind, ContextRuntimeRecovery,
    ContextRuntimeRecoveryKind, ContextRuntimeState, ContextRuntimeStateSummary,
    ContextRuntimeStore, MicrocompactMode, MicrocompactState, PostCompactValidation,
    PreservedSegmentMetadata, ProjectionStats, SessionMemoryState, SnipRemovalState,
    CONTEXT_RUNTIME_OUTPUT_RESERVE_TOKENS, CURRENT_CONTEXT_RUNTIME_SCHEMA_VERSION,
    DEFAULT_AUTO_COMPACT_THRESHOLD,
};

pub async fn load_context_runtime_state(session_id: &str) -> Result<ContextRuntimeState> {
    let session = SessionManager::get_session(session_id, false).await?;
    Ok(ContextRuntimeState::from_extension_data(&session.extension_data).unwrap_or_default())
}

pub async fn save_context_runtime_state(
    session_id: &str,
    state: &ContextRuntimeState,
) -> Result<()> {
    SessionManager::set_extension_state(session_id, state).await
}

pub fn migrate_legacy_todo_to_context_runtime(legacy: Option<TodoState>) -> ContextRuntimeState {
    let mut state = ContextRuntimeState {
        schema_version: CURRENT_CONTEXT_RUNTIME_SCHEMA_VERSION,
        ..ContextRuntimeState::default()
    };
    if let Some(todo) = legacy {
        state.set_session_memory(Some(SessionMemoryState {
            summary: format!("Legacy TODO context preserved:\n{}", todo.content),
            summarized_through_message_id: None,
            preserved_start_index: 0,
            preserved_end_index: 0,
            preserved_start_message_id: None,
            preserved_end_message_id: None,
            preserved_message_count: 0,
            preserved_token_estimate: 0,
            tail_anchor_index: 0,
            tail_anchor_message_id: None,
            updated_at: Utc::now().timestamp(),
        }));
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_runtime::collapse::{adjust_collapse_start, commit_staged_collapses};
    use crate::context_runtime::microcompact::build_microcompact_state;
    use crate::context_runtime::projection::apply_projection;
    use crate::context_runtime::session_memory::adjust_suffix_start;
    use crate::context_runtime::snip::build_snip_state;
    use crate::conversation::message::{Message, MessageContent};
    use crate::conversation::Conversation;
    use crate::model::ModelConfig;
    use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
    use crate::providers::errors::ProviderError;
    use async_trait::async_trait;
    use futures::executor::block_on;
    use rmcp::model::{CallToolRequestParams, Content, Tool};
    use rmcp::object;

    struct MockRuntimeProvider {
        model_config: ModelConfig,
    }

    struct EchoRuntimeProvider;

    #[async_trait]
    impl Provider for MockRuntimeProvider {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::empty()
        }

        fn get_name(&self) -> &str {
            "context-runtime-mock"
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            Ok((
                Message::assistant().with_text("summary"),
                ProviderUsage::new("mock".to_string(), Usage::default()),
            ))
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }
    }

    #[async_trait]
    impl Provider for EchoRuntimeProvider {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::empty()
        }

        fn get_name(&self) -> &str {
            "context-runtime-echo"
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            Ok((
                Message::assistant().with_text(system.to_string()),
                ProviderUsage::new("mock".to_string(), Usage::default()),
            ))
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("claude-3-7-sonnet-latest")
        }
    }

    fn tiny_context_provider() -> MockRuntimeProvider {
        MockRuntimeProvider {
            model_config: ModelConfig::new_or_fail("claude-3-7-sonnet-latest")
                .with_context_limit(Some(4_096))
                .with_output_reserve_tokens(Some(0))
                .with_auto_compact_threshold(Some(0.5)),
        }
    }

    fn four_message_conversation() -> Conversation {
        Conversation::new_unvalidated(vec![
            Message::user().with_text("one"),
            Message::assistant().with_text("two"),
            Message::user().with_text("three"),
            Message::assistant().with_text("four"),
        ])
    }

    fn long_runtime_conversation() -> Conversation {
        let mut conversation = Conversation::empty();
        for idx in 0..8 {
            let text = "x".repeat(4_000);
            let mut message = if idx % 2 == 0 {
                Message::user().with_text(format!("user-{idx} {text}"))
            } else {
                Message::assistant().with_text(format!("assistant-{idx} {text}"))
            };
            message.id = Some(format!("long-{idx}"));
            conversation.push(message);
        }
        conversation
    }

    fn medium_runtime_conversation(count: usize) -> Conversation {
        let mut conversation = Conversation::empty();
        for idx in 0..count {
            let text = "m".repeat(2_000);
            let mut message = if idx % 2 == 0 {
                Message::user().with_text(format!("user-{idx} {text}"))
            } else {
                Message::assistant().with_text(format!("assistant-{idx} {text}"))
            };
            message.id = Some(format!("mid-{idx}"));
            conversation.push(message);
        }
        conversation
    }

    fn many_long_runtime_conversation(count: usize) -> Conversation {
        let mut conversation = Conversation::empty();
        for idx in 0..count {
            let text = "x".repeat(4_000);
            let mut message = if idx % 2 == 0 {
                Message::user().with_text(format!("user-{idx} {text}"))
            } else {
                Message::assistant().with_text(format!("assistant-{idx} {text}"))
            };
            message.id = Some(format!("many-{idx}"));
            conversation.push(message);
        }
        conversation
    }

    #[test]
    fn snip_removes_old_agent_only_runtime_noise() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::user().with_text("hello"));
        conversation.push(
            Message::assistant()
                .with_system_notification(
                    crate::conversation::message::SystemNotificationType::InlineMessage,
                    "runtime note",
                )
                .agent_only(),
        );
        conversation.push(Message::assistant().with_text("recent"));

        let removals = build_snip_state(conversation.messages(), 2);
        assert_eq!(removals.len(), 1);
        assert_eq!(removals[0].removed_indexes, vec![1]);
    }

    #[test]
    fn microcompact_slims_old_tool_response_without_removing_pair() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::user().with_text("do something"));
        conversation.push(Message::assistant().with_tool_request(
            "tool-1",
            Ok(CallToolRequestParams {
                name: "developer__shell".into(),
                arguments: Some(object!({"command": "dir"})),
                meta: None,
                task: None,
            }),
        ));
        conversation.push(Message::user().with_tool_response(
            "tool-1",
            Ok(rmcp::model::CallToolResult {
                content: vec![Content::text("a".repeat(1100))],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        ));

        let entries = build_microcompact_state(conversation.messages(), 3, &[]);
        assert_eq!(entries.len(), 1);

        let mut state = ContextRuntimeState::default();
        *state.microcompact_entries_mut() = entries;
        let projected = apply_projection(&conversation, &state);
        let response_message = &projected.messages()[2];
        let tool_response_text = response_message
            .content
            .iter()
            .find_map(|content| match content {
                MessageContent::ToolResponse(response) => match &response.tool_result {
                    Ok(result) => result
                        .content
                        .iter()
                        .find_map(|content| content.as_text().map(|text| text.text.clone())),
                    Err(_) => None,
                },
                _ => None,
            })
            .unwrap_or_default();
        assert!(tool_response_text.contains("context runtime microcompact"));
    }

    #[test]
    fn summary_prompt_preserves_continuation_contract() {
        let prompt = include_str!("../prompts/summarize_oneshot.md");

        assert!(prompt.contains("constraints_and_preferences"));
        assert!(prompt.contains("files_and_artifacts"));
        assert!(prompt.contains("current_state"));
        assert!(prompt.contains("next_action"));
        assert!(prompt.contains("Do not invent paths"));
    }

    #[test]
    fn compact_boundaries_do_not_split_tool_pairs_or_assistant_continuations() {
        let tool_messages = vec![
            Message::user().with_text("inspect"),
            Message::assistant()
                .with_id("assistant-tool")
                .with_tool_request(
                    "tool-1",
                    Ok(CallToolRequestParams {
                        name: "developer__shell".into(),
                        arguments: Some(object!({"command": "dir"})),
                        meta: None,
                        task: None,
                    }),
                ),
            Message::user().with_tool_response(
                "tool-1",
                Ok(rmcp::model::CallToolResult {
                    content: vec![Content::text("ok")],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
            Message::user().with_text("tail"),
        ];
        assert_eq!(adjust_suffix_start(&tool_messages, 2), 1);
        assert_eq!(adjust_collapse_start(&tool_messages, 2, 3), 1);

        let assistant_messages = vec![
            Message::assistant()
                .with_id("assistant-cont")
                .with_text("thinking block"),
            Message::assistant()
                .with_id("assistant-cont")
                .with_text("tool block"),
            Message::user().with_text("tail"),
        ];
        assert_eq!(adjust_suffix_start(&assistant_messages, 1), 0);
        assert_eq!(adjust_collapse_start(&assistant_messages, 1, 2), 0);
    }

    #[test]
    fn microcompact_can_promote_older_tool_outputs_to_time_based_mode() {
        let mut conversation = Conversation::empty();
        for idx in 0..8 {
            let mut response = Message::user().with_tool_response(
                format!("tool-{idx}"),
                Ok(rmcp::model::CallToolResult {
                    content: vec![Content::text("b".repeat(1400))],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            );
            response.created = 1;
            response.id = Some(format!("resp-{idx}"));
            conversation.push(response);
        }
        let mut latest = Message::assistant().with_text("latest");
        latest.created = crate::context_runtime::types::TIME_BASED_MICROCOMPACT_GAP_SECS + 10;
        conversation.push(latest);

        let entries = build_microcompact_state(conversation.messages(), 8, &[]);
        assert!(entries
            .iter()
            .any(|entry| entry.mode == MicrocompactMode::TimeBased));
    }

    #[test]
    fn summarize_context_runtime_state_reports_projection_stats() {
        let mut state = ContextRuntimeState::default();
        state.runtime_compactions = 2;
        *state.snip_entries_mut() = vec![SnipRemovalState {
            removed_indexes: vec![1, 2],
            removed_message_ids: vec!["a".to_string(), "b".to_string()],
            removed_count: 2,
            token_estimate_freed: 128,
            reason: "test".to_string(),
            created_at: 1,
        }];
        *state.microcompact_entries_mut() = vec![MicrocompactState {
            message_index: 1,
            message_id: Some("m2".to_string()),
            tool_response_ids: vec!["tool-1".to_string()],
            mode: MicrocompactMode::Cached,
            original_text_chars: 1000,
            original_text_lines: 10,
            compacted_text_chars: 100,
            compacted_at: 2,
        }];
        state.last_projection_stats = Some(ProjectionStats {
            base_agent_messages: 8,
            projected_agent_messages: 4,
            snip_removed_count: 2,
            microcompacted_count: 1,
            raw_token_estimate: 2048,
            projected_token_estimate: 1024,
            freed_token_estimate: 1024,
            updated_at: 3,
        });
        let summary = summarize_context_runtime_state(&state);
        assert_eq!(summary.runtime_compactions, 2);
        assert_eq!(summary.snip_removed_count, 2);
        assert_eq!(summary.microcompacted_count, 1);
        assert_eq!(
            summary
                .last_projection_stats
                .as_ref()
                .map(|stats| stats.freed_token_estimate),
            Some(1024)
        );
    }

    #[test]
    fn project_for_provider_replays_pre_slice_collapse_summary() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::user().with_text("alpha"));
        conversation.push(Message::assistant().with_text("beta"));
        conversation.push(Message::user().with_text("gamma"));
        conversation.push(Message::assistant().with_text("delta"));

        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-1".to_string()),
            summary: "collapsed early context".to_string(),
            start_index: 0,
            end_index: 2,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            created_at: 1,
        });
        state.set_compact_boundary(Some(CompactBoundaryMetadata {
            anchor_index: 0,
            head_index: 0,
            tail_index: 2,
            direction: CompactDirection::UpTo,
            anchor_message_id: None,
            head_message_id: None,
            tail_message_id: None,
            created_at: 1,
        }));
        state.set_preserved_segment(Some(PreservedSegmentMetadata {
            start_index: 2,
            end_index: 3,
            tail_anchor_index: 3,
            start_message_id: None,
            end_message_id: None,
            tail_anchor_message_id: None,
            reason: "collapse".to_string(),
        }));

        let projected = project_for_provider(&conversation, &state);
        assert!(projected.messages()[0]
            .as_concat_text()
            .contains("collapsed early context"));
    }

    #[test]
    fn commit_staged_snapshot_promotes_into_store() {
        let mut conversation = Conversation::empty();
        conversation.push(Message::user().with_text("one"));
        conversation.push(Message::assistant().with_text("two"));
        conversation.push(Message::user().with_text("three"));
        conversation.push(Message::assistant().with_text("four"));

        let mut state = ContextRuntimeState::default();
        state.set_staged_snapshot(Some(CollapseSnapshot {
            snapshot_id: Some("snapshot-1".to_string()),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            summary: "collapse me".to_string(),
            risk: 0.9,
            staged_at: 1,
        }));

        commit_staged_collapses(&conversation, &mut state, "committed_collapse", None);
        assert!(state.staged_snapshot().is_none());
        assert_eq!(state.committed_collapses().len(), 1);
        assert!(state.compact_boundary().is_some());
        assert!(state.preserved_segment().is_some());
    }

    #[test]
    fn staged_collapse_uses_current_visible_working_set_after_committed_prefix() {
        let provider = tiny_context_provider();
        let conversation = many_long_runtime_conversation(10);
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-existing".to_string()),
            summary: "existing collapse".to_string(),
            start_index: 0,
            end_index: 4,
            direction: CompactDirection::UpTo,
            start_message_id: Some("many-0".to_string()),
            end_message_id: Some("many-3".to_string()),
            created_at: 1,
        });
        state.set_compact_boundary(Some(CompactBoundaryMetadata {
            anchor_index: 0,
            head_index: 0,
            tail_index: 4,
            direction: CompactDirection::UpTo,
            anchor_message_id: Some("many-0".to_string()),
            head_message_id: Some("many-0".to_string()),
            tail_message_id: Some("many-4".to_string()),
            created_at: 1,
        }));
        state.set_preserved_segment(Some(PreservedSegmentMetadata {
            start_index: 4,
            end_index: 9,
            tail_anchor_index: 9,
            start_message_id: Some("many-4".to_string()),
            end_message_id: Some("many-9".to_string()),
            tail_anchor_message_id: Some("many-9".to_string()),
            reason: "committed_collapse".to_string(),
        }));

        let snapshot = block_on(crate::context_runtime::collapse::build_staged_collapse(
            &provider,
            &conversation,
            &state,
            0.9,
            12_000,
        ))
        .expect("build staged collapse")
        .expect("snapshot");

        assert!(snapshot.start_index >= 4);
        assert!(snapshot.end_index <= conversation.messages().len());
        assert!(snapshot
            .start_message_id
            .as_deref()
            .is_some_and(|id| id != "many-0"
                && id != "many-1"
                && id != "many-2"
                && id != "many-3"));
    }

    #[test]
    fn staged_collapse_prefers_existing_compact_direction_over_heuristic() {
        let provider = tiny_context_provider();
        let conversation = many_long_runtime_conversation(10);
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-from".to_string()),
            summary: "suffix collapse".to_string(),
            start_index: 7,
            end_index: 10,
            direction: CompactDirection::From,
            start_message_id: Some("many-7".to_string()),
            end_message_id: Some("many-9".to_string()),
            created_at: 1,
        });
        state.set_compact_boundary(Some(CompactBoundaryMetadata {
            anchor_index: 7,
            head_index: 6,
            tail_index: 9,
            direction: CompactDirection::From,
            anchor_message_id: Some("many-7".to_string()),
            head_message_id: Some("many-6".to_string()),
            tail_message_id: Some("many-9".to_string()),
            created_at: 1,
        }));
        state.set_preserved_segment(Some(PreservedSegmentMetadata {
            start_index: 0,
            end_index: 6,
            tail_anchor_index: 6,
            start_message_id: Some("many-0".to_string()),
            end_message_id: Some("many-6".to_string()),
            tail_anchor_message_id: Some("many-6".to_string()),
            reason: "boundary_head".to_string(),
        }));

        let snapshot = block_on(crate::context_runtime::collapse::build_staged_collapse(
            &provider,
            &conversation,
            &state,
            0.9,
            12_000,
        ))
        .expect("build staged collapse")
        .expect("snapshot");

        assert_eq!(snapshot.direction, CompactDirection::From);
    }

    #[test]
    fn session_memory_suffix_respects_last_summarized_boundary() {
        let mut conversation = Conversation::empty();
        for idx in 0..6 {
            let mut message = if idx % 2 == 0 {
                Message::user().with_text(format!("user-{idx}"))
            } else {
                Message::assistant().with_text(format!("assistant-{idx}"))
            };
            message.id = Some(format!("m-{idx}"));
            conversation.push(message);
        }

        let start = crate::context_runtime::session_memory::calculate_suffix_start(
            conversation.messages(),
            crate::context_runtime::types::SessionMemoryPolicy::for_kind(
                crate::context_runtime::types::SessionMemoryPolicyKind::Manual,
            ),
            Some(3),
        );

        assert!(start >= 4);
    }

    #[test]
    fn session_memory_summary_uses_current_visible_working_set_after_committed_prefix() {
        let provider = EchoRuntimeProvider;
        let conversation = medium_runtime_conversation(10);
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-memory".to_string()),
            summary: "existing collapse".to_string(),
            start_index: 0,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: Some("mid-0".to_string()),
            end_message_id: Some("mid-2".to_string()),
            created_at: 1,
        });
        state.set_compact_boundary(Some(CompactBoundaryMetadata {
            anchor_index: 0,
            head_index: 0,
            tail_index: 3,
            direction: CompactDirection::UpTo,
            anchor_message_id: Some("mid-0".to_string()),
            head_message_id: Some("mid-0".to_string()),
            tail_message_id: Some("mid-3".to_string()),
            created_at: 1,
        }));
        state.set_preserved_segment(Some(PreservedSegmentMetadata {
            start_index: 3,
            end_index: 9,
            tail_anchor_index: 9,
            start_message_id: Some("mid-3".to_string()),
            end_message_id: Some("mid-9".to_string()),
            tail_anchor_message_id: Some("mid-9".to_string()),
            reason: "committed_collapse".to_string(),
        }));

        let memory = block_on(
            crate::context_runtime::session_memory::build_session_memory_state(
                &provider,
                &conversation,
                &state,
                crate::context_runtime::types::SessionMemoryPolicyKind::Manual,
                None,
            ),
        )
        .expect("build session memory");

        assert!(memory.summary.contains("assistant-3") || memory.summary.contains("user-4"));
        assert!(!memory.summary.contains("user-0"));
        assert!(!memory.summary.contains("assistant-1"));
    }

    #[test]
    fn session_memory_summary_carries_forward_existing_summary_context() {
        let provider = EchoRuntimeProvider;
        let conversation = medium_runtime_conversation(10);
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-memory-existing".to_string()),
            summary: "existing collapse".to_string(),
            start_index: 0,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: Some("mid-0".to_string()),
            end_message_id: Some("mid-2".to_string()),
            created_at: 1,
        });
        state.set_compact_boundary(Some(CompactBoundaryMetadata {
            anchor_index: 0,
            head_index: 0,
            tail_index: 3,
            direction: CompactDirection::UpTo,
            anchor_message_id: Some("mid-0".to_string()),
            head_message_id: Some("mid-0".to_string()),
            tail_message_id: Some("mid-3".to_string()),
            created_at: 1,
        }));
        state.set_preserved_segment(Some(PreservedSegmentMetadata {
            start_index: 3,
            end_index: 9,
            tail_anchor_index: 9,
            start_message_id: Some("mid-3".to_string()),
            end_message_id: Some("mid-9".to_string()),
            tail_anchor_message_id: Some("mid-9".to_string()),
            reason: "committed_collapse".to_string(),
        }));

        let existing_memory = SessionMemoryState {
            summary: "existing prior summary".to_string(),
            summarized_through_message_id: Some("mid-3".to_string()),
            preserved_start_index: 4,
            preserved_end_index: 9,
            preserved_start_message_id: Some("mid-4".to_string()),
            preserved_end_message_id: Some("mid-9".to_string()),
            preserved_message_count: 6,
            preserved_token_estimate: 3000,
            tail_anchor_index: 9,
            tail_anchor_message_id: Some("mid-9".to_string()),
            updated_at: 1,
        };

        let memory = block_on(
            crate::context_runtime::session_memory::build_session_memory_state(
                &provider,
                &conversation,
                &state,
                crate::context_runtime::types::SessionMemoryPolicyKind::Manual,
                Some(&existing_memory),
            ),
        )
        .expect("build session memory");

        assert!(memory.summary.contains("existing prior summary"));
        assert!(memory.summary.contains("assistant-3") || memory.summary.contains("user-4"));
        assert!(!memory.summary.contains("user-0"));
    }

    #[test]
    fn finalize_projection_refresh_normalizes_mixed_collapse_directions() {
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-up".to_string()),
            summary: "prefix collapse".to_string(),
            start_index: 0,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: Some("many-0".to_string()),
            end_message_id: Some("many-2".to_string()),
            created_at: 1,
        });
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-from".to_string()),
            summary: "suffix collapse".to_string(),
            start_index: 7,
            end_index: 10,
            direction: CompactDirection::From,
            start_message_id: Some("many-7".to_string()),
            end_message_id: Some("many-9".to_string()),
            created_at: 2,
        });
        state.set_compact_boundary(Some(CompactBoundaryMetadata {
            anchor_index: 0,
            head_index: 0,
            tail_index: 3,
            direction: CompactDirection::UpTo,
            anchor_message_id: Some("many-0".to_string()),
            head_message_id: Some("many-0".to_string()),
            tail_message_id: Some("many-3".to_string()),
            created_at: 2,
        }));
        state.set_preserved_segment(Some(PreservedSegmentMetadata {
            start_index: 3,
            end_index: 9,
            tail_anchor_index: 9,
            start_message_id: Some("many-3".to_string()),
            end_message_id: Some("many-9".to_string()),
            tail_anchor_message_id: Some("many-9".to_string()),
            reason: "committed_collapse".to_string(),
        }));
        state.set_staged_snapshot(Some(CollapseSnapshot {
            snapshot_id: Some("staged-from".to_string()),
            start_index: 7,
            end_index: 10,
            direction: CompactDirection::From,
            start_message_id: Some("many-7".to_string()),
            end_message_id: Some("many-9".to_string()),
            summary: "staged".to_string(),
            risk: 0.8,
            staged_at: 3,
        }));

        crate::context_runtime::store::finalize_projection_refresh(10, &mut state);

        assert_eq!(state.committed_collapses().len(), 1);
        assert_eq!(
            state.committed_collapses()[0].direction,
            CompactDirection::From
        );
        assert!(state.staged_snapshot().is_some());
        assert!(state.store.entry_log.iter().all(|entry| !matches!(
            entry,
            crate::context_runtime::types::ContextRuntimeStoreEntry::CollapseCommitAdded { commit }
                if commit.direction == CompactDirection::UpTo
        )));
    }

    #[test]
    fn rewind_rebuilds_projection_stats_instead_of_preserving_stale_values() {
        let conversation = four_message_conversation();
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-rewind-refresh".to_string()),
            summary: "persist".to_string(),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            created_at: 1,
        });
        state.last_projection_stats = Some(ProjectionStats {
            base_agent_messages: 999,
            projected_agent_messages: 999,
            snip_removed_count: 999,
            microcompacted_count: 999,
            raw_token_estimate: 9999,
            projected_token_estimate: 9999,
            freed_token_estimate: 0,
            updated_at: 1,
        });

        let rewound = crate::context_runtime::rewind_context_runtime_state(&conversation, &state)
            .expect("rewound state");
        let stats = rewound
            .last_projection_stats
            .as_ref()
            .expect("projection stats");

        assert_ne!(stats.raw_token_estimate, 9999);
        assert_ne!(stats.projected_token_estimate, 9999);
        assert_eq!(rewound.committed_collapses().len(), 1);
    }

    #[test]
    fn replay_entries_reanchor_by_message_id_after_prefix_changes() {
        let mut conversation = Conversation::empty();
        let mut first = Message::user().with_text("one");
        first.id = Some("m1".to_string());
        let mut second = Message::assistant().with_text("two");
        second.id = Some("m2".to_string());
        let mut third = Message::user().with_text("three");
        third.id = Some("m3".to_string());
        let mut fourth = Message::assistant().with_text("four");
        fourth.id = Some("m4".to_string());
        conversation.push(first);
        conversation.push(second);
        conversation.push(third);
        conversation.push(fourth);

        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-1".to_string()),
            summary: "collapse".to_string(),
            start_index: 0,
            end_index: 2,
            direction: CompactDirection::UpTo,
            start_message_id: Some("m2".to_string()),
            end_message_id: Some("m3".to_string()),
            created_at: 1,
        });
        state.set_session_memory(Some(SessionMemoryState {
            summary: "memory".to_string(),
            summarized_through_message_id: Some("m2".to_string()),
            preserved_start_index: 2,
            preserved_end_index: 3,
            preserved_start_message_id: Some("m3".to_string()),
            preserved_end_message_id: Some("m4".to_string()),
            preserved_message_count: 2,
            preserved_token_estimate: 10,
            tail_anchor_index: 3,
            tail_anchor_message_id: Some("m4".to_string()),
            updated_at: 1,
        }));
        state.set_snip_entries(vec![SnipRemovalState {
            removed_indexes: vec![0],
            removed_message_ids: vec!["m2".to_string()],
            removed_count: 1,
            token_estimate_freed: 12,
            reason: "test".to_string(),
            created_at: 1,
        }]);
        state.set_microcompact_entries(vec![MicrocompactState {
            message_index: 0,
            message_id: Some("m3".to_string()),
            tool_response_ids: vec!["tool-1".to_string()],
            mode: MicrocompactMode::Cached,
            original_text_chars: 100,
            original_text_lines: 2,
            compacted_text_chars: 20,
            compacted_at: 1,
        }]);

        let truncated = Conversation::new_unvalidated(vec![
            conversation.messages()[1].clone(),
            conversation.messages()[2].clone(),
            conversation.messages()[3].clone(),
        ]);
        let mut reanchored = state.clone();
        super::store::reconcile_state_for_conversation(&truncated, &mut reanchored);

        assert_eq!(reanchored.committed_collapses()[0].start_index, 0);
        assert_eq!(reanchored.committed_collapses()[0].end_index, 2);
        assert_eq!(
            reanchored
                .session_memory()
                .expect("session memory")
                .preserved_start_index,
            1
        );
        assert_eq!(reanchored.snip_entries()[0].removed_indexes, vec![0]);
        assert_eq!(reanchored.microcompact_entries()[0].message_index, 1);
    }

    #[test]
    fn relink_state_for_replaced_conversation_reanchors_id_based_snip_and_microcompact_by_signature(
    ) {
        let old_conversation = Conversation::new_unvalidated(vec![
            Message::user().with_id("old-1").with_text("prefix"),
            Message::assistant()
                .with_id("old-2")
                .with_text("tool request"),
            Message::user()
                .with_id("old-3")
                .with_text("tool response payload"),
            Message::assistant().with_id("old-4").with_text("kept"),
        ]);
        let new_conversation = Conversation::new_unvalidated(vec![
            Message::user().with_id("new-0").with_text("new-prefix"),
            Message::user().with_id("new-1").with_text("prefix"),
            Message::assistant()
                .with_id("new-2")
                .with_text("tool request"),
            Message::user()
                .with_id("new-3")
                .with_text("tool response payload"),
            Message::assistant().with_id("new-4").with_text("kept"),
        ]);

        let mut state = ContextRuntimeState::default();
        state.set_snip_entries(vec![SnipRemovalState {
            removed_indexes: vec![1],
            removed_message_ids: vec!["old-2".to_string()],
            removed_count: 1,
            token_estimate_freed: 10,
            reason: "test".to_string(),
            created_at: 1,
        }]);
        state.set_microcompact_entries(vec![MicrocompactState {
            message_index: 2,
            message_id: Some("old-3".to_string()),
            tool_response_ids: vec!["tool-1".to_string()],
            mode: MicrocompactMode::Cached,
            original_text_chars: 100,
            original_text_lines: 2,
            compacted_text_chars: 20,
            compacted_at: 1,
        }]);

        super::store::relink_state_for_replaced_conversation(
            &old_conversation,
            &new_conversation,
            &mut state,
        );

        assert_eq!(state.snip_entries()[0].removed_indexes, vec![2]);
        assert_eq!(state.snip_entries()[0].removed_message_ids, vec!["new-2"]);
        assert_eq!(state.microcompact_entries()[0].message_index, 3);
        assert_eq!(
            state.microcompact_entries()[0].message_id.as_deref(),
            Some("new-3")
        );
    }

    #[test]
    fn relink_state_for_replaced_conversation_reconciles_old_indices_before_relink() {
        let old_conversation = Conversation::new_unvalidated(vec![
            Message::user().with_id("old-1").with_text("prefix"),
            Message::assistant()
                .with_id("old-2")
                .with_text("collapsed-1"),
            Message::user().with_id("old-3").with_text("collapsed-2"),
            Message::assistant().with_id("old-4").with_text("kept"),
        ]);
        let new_conversation = Conversation::new_unvalidated(vec![
            Message::user().with_id("new-0").with_text("new-prefix"),
            Message::assistant()
                .with_id("new-1")
                .with_text("collapsed-1"),
            Message::user().with_id("new-2").with_text("collapsed-2"),
            Message::assistant().with_id("new-3").with_text("kept"),
        ]);

        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-stale".to_string()),
            summary: "collapsed summary".to_string(),
            start_index: 0,
            end_index: 2,
            direction: CompactDirection::UpTo,
            start_message_id: Some("old-2".to_string()),
            end_message_id: Some("old-3".to_string()),
            created_at: 1,
        });
        state.set_snip_entries(vec![SnipRemovalState {
            removed_indexes: vec![0],
            removed_message_ids: vec!["old-2".to_string()],
            removed_count: 1,
            token_estimate_freed: 10,
            reason: "test".to_string(),
            created_at: 1,
        }]);
        state.set_microcompact_entries(vec![MicrocompactState {
            message_index: 0,
            message_id: Some("old-3".to_string()),
            tool_response_ids: vec!["tool-1".to_string()],
            mode: MicrocompactMode::Cached,
            original_text_chars: 100,
            original_text_lines: 2,
            compacted_text_chars: 20,
            compacted_at: 1,
        }]);

        super::store::relink_state_for_replaced_conversation(
            &old_conversation,
            &new_conversation,
            &mut state,
        );

        assert_eq!(state.committed_collapses()[0].start_index, 1);
        assert_eq!(state.committed_collapses()[0].end_index, 3);
        assert_eq!(state.snip_entries()[0].removed_indexes, vec![1]);
        assert_eq!(state.microcompact_entries()[0].message_index, 2);
        assert_eq!(
            state.committed_collapses()[0].start_message_id.as_deref(),
            Some("new-1")
        );
        assert_eq!(
            state.committed_collapses()[0].end_message_id.as_deref(),
            Some("new-2")
        );
    }

    #[test]
    fn reconcile_state_backfills_missing_message_ids_from_indices() {
        let mut conversation = Conversation::empty();
        let mut first = Message::user().with_text("one");
        first.id = Some("m1".to_string());
        let mut second = Message::assistant().with_text("two");
        second.id = Some("m2".to_string());
        let mut third = Message::user().with_text("three");
        third.id = Some("m3".to_string());
        let mut fourth = Message::assistant().with_text("four");
        fourth.id = Some("m4".to_string());
        conversation.push(first);
        conversation.push(second);
        conversation.push(third);
        conversation.push(fourth);

        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-1".to_string()),
            summary: "collapse".to_string(),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            created_at: 1,
        });
        state.set_session_memory(Some(SessionMemoryState {
            summary: "memory".to_string(),
            summarized_through_message_id: None,
            preserved_start_index: 2,
            preserved_end_index: 3,
            preserved_start_message_id: None,
            preserved_end_message_id: None,
            preserved_message_count: 2,
            preserved_token_estimate: 10,
            tail_anchor_index: 3,
            tail_anchor_message_id: None,
            updated_at: 1,
        }));
        state.set_compact_boundary(Some(CompactBoundaryMetadata {
            anchor_index: 1,
            head_index: 1,
            tail_index: 2,
            direction: CompactDirection::UpTo,
            anchor_message_id: None,
            head_message_id: None,
            tail_message_id: None,
            created_at: 1,
        }));
        state.set_preserved_segment(Some(PreservedSegmentMetadata {
            start_index: 2,
            end_index: 3,
            tail_anchor_index: 3,
            start_message_id: None,
            end_message_id: None,
            tail_anchor_message_id: None,
            reason: "memory".to_string(),
        }));
        state.set_snip_entries(vec![SnipRemovalState {
            removed_indexes: vec![1],
            removed_message_ids: Vec::new(),
            removed_count: 1,
            token_estimate_freed: 12,
            reason: "test".to_string(),
            created_at: 1,
        }]);
        state.set_microcompact_entries(vec![MicrocompactState {
            message_index: 2,
            message_id: None,
            tool_response_ids: vec!["tool-1".to_string()],
            mode: MicrocompactMode::Cached,
            original_text_chars: 100,
            original_text_lines: 2,
            compacted_text_chars: 20,
            compacted_at: 1,
        }]);

        super::store::reconcile_state_for_conversation(&conversation, &mut state);

        assert_eq!(
            state.committed_collapses()[0].start_message_id.as_deref(),
            Some("m2")
        );
        assert_eq!(
            state.committed_collapses()[0].end_message_id.as_deref(),
            Some("m3")
        );
        assert_eq!(
            state
                .session_memory()
                .expect("session memory")
                .preserved_start_message_id
                .as_deref(),
            Some("m3")
        );
        assert_eq!(
            state
                .compact_boundary()
                .expect("boundary")
                .tail_message_id
                .as_deref(),
            Some("m3")
        );
        assert_eq!(
            state
                .preserved_segment()
                .expect("segment")
                .tail_anchor_message_id
                .as_deref(),
            Some("m4")
        );
    }

    #[test]
    fn restored_from_entry_log_rebuilds_state_when_live_fields_are_stale() {
        let mut state = ContextRuntimeState::default();
        let commit = CollapseCommit {
            commit_id: Some("commit-1".to_string()),
            summary: "from-log".to_string(),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: Some("m2".to_string()),
            end_message_id: Some("m3".to_string()),
            created_at: 1,
        };
        state.append_collapse_commit(commit.clone());
        state.committed_collapses_mut().clear();

        let restored = state.restored_from_entry_log();
        assert_eq!(restored.committed_collapses(), &[commit]);
    }

    #[test]
    fn restored_from_entry_log_rebuilds_snip_and_microcompact_when_live_fields_are_stale() {
        let mut state = ContextRuntimeState::default();
        let snip = SnipRemovalState {
            removed_indexes: vec![1],
            removed_message_ids: vec!["m2".to_string()],
            removed_count: 1,
            token_estimate_freed: 50,
            reason: "test".to_string(),
            created_at: 1,
        };
        let micro = MicrocompactState {
            message_index: 2,
            message_id: Some("m3".to_string()),
            tool_response_ids: vec!["tool-1".to_string()],
            mode: MicrocompactMode::Cached,
            original_text_chars: 100,
            original_text_lines: 4,
            compacted_text_chars: 20,
            compacted_at: 2,
        };
        state.set_snip_entries(vec![snip.clone()]);
        state.set_microcompact_entries(vec![micro.clone()]);
        state.snip_entries_mut().clear();
        state.microcompact_entries_mut().clear();

        let restored = state.restored_from_entry_log();
        assert_eq!(restored.snip_entries(), &[snip]);
        assert_eq!(restored.microcompact_entries(), &[micro]);
    }

    #[test]
    fn restored_from_entry_log_ignores_conflicting_live_projection_fields() {
        let mut state = ContextRuntimeState::default();
        let snip = SnipRemovalState {
            removed_indexes: vec![1],
            removed_message_ids: vec!["m2".to_string()],
            removed_count: 1,
            token_estimate_freed: 50,
            reason: "from-log".to_string(),
            created_at: 1,
        };
        let micro = MicrocompactState {
            message_index: 2,
            message_id: Some("m3".to_string()),
            tool_response_ids: vec!["tool-1".to_string()],
            mode: MicrocompactMode::Cached,
            original_text_chars: 100,
            original_text_lines: 4,
            compacted_text_chars: 20,
            compacted_at: 2,
        };
        state.set_snip_entries(vec![snip.clone()]);
        state.set_microcompact_entries(vec![micro.clone()]);
        *state.snip_entries_mut() = vec![SnipRemovalState {
            removed_indexes: vec![99],
            removed_message_ids: vec!["bad".to_string()],
            removed_count: 1,
            token_estimate_freed: 1,
            reason: "stale-live".to_string(),
            created_at: 9,
        }];
        *state.microcompact_entries_mut() = vec![MicrocompactState {
            message_index: 99,
            message_id: Some("bad".to_string()),
            tool_response_ids: vec!["bad-tool".to_string()],
            mode: MicrocompactMode::MarkerRewrite,
            original_text_chars: 1,
            original_text_lines: 1,
            compacted_text_chars: 1,
            compacted_at: 9,
        }];

        let restored = state.restored_from_entry_log();
        assert_eq!(restored.snip_entries(), &[snip]);
        assert_eq!(restored.microcompact_entries(), &[micro]);
    }

    #[test]
    fn project_view_restores_entry_log_before_projection() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("prefix"),
            Message::assistant().with_text("collapsed-1"),
            Message::user().with_text("collapsed-2"),
            Message::assistant().with_text("kept"),
        ]);

        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-log".to_string()),
            summary: "replayed collapse".to_string(),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            created_at: 1,
        });
        state.set_compact_boundary(Some(CompactBoundaryMetadata {
            anchor_index: 1,
            head_index: 1,
            tail_index: 3,
            direction: CompactDirection::UpTo,
            anchor_message_id: None,
            head_message_id: None,
            tail_message_id: None,
            created_at: 1,
        }));
        state.set_preserved_segment(Some(PreservedSegmentMetadata {
            start_index: 3,
            end_index: 3,
            tail_anchor_index: 3,
            start_message_id: None,
            end_message_id: None,
            tail_anchor_message_id: None,
            reason: "committed_collapse".to_string(),
        }));
        state.committed_collapses_mut().clear();

        let projected = crate::context_runtime::projection::project_view(&conversation, &state);
        let text = projected
            .messages()
            .iter()
            .map(|message| message.as_concat_text())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("replayed collapse"));
        assert!(text.contains("kept"));
    }

    #[test]
    fn project_view_supports_from_direction_and_replays_suffix_summary() {
        let mut conversation = Conversation::empty();
        let mut first = Message::user().with_text("alpha");
        first.id = Some("m1".to_string());
        let mut second = Message::assistant().with_text("beta");
        second.id = Some("m2".to_string());
        let mut third = Message::user().with_text("gamma");
        third.id = Some("m3".to_string());
        let mut fourth = Message::assistant().with_text("delta");
        fourth.id = Some("m4".to_string());
        conversation.push(first);
        conversation.push(second);
        conversation.push(third);
        conversation.push(fourth);

        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-from".to_string()),
            summary: "collapsed suffix".to_string(),
            start_index: 2,
            end_index: 4,
            direction: CompactDirection::From,
            start_message_id: Some("m3".to_string()),
            end_message_id: Some("m4".to_string()),
            created_at: 1,
        });
        state.set_compact_boundary(Some(CompactBoundaryMetadata {
            anchor_index: 1,
            head_index: 1,
            tail_index: 3,
            direction: CompactDirection::From,
            anchor_message_id: Some("m2".to_string()),
            head_message_id: Some("m2".to_string()),
            tail_message_id: Some("m4".to_string()),
            created_at: 1,
        }));
        state.set_preserved_segment(Some(PreservedSegmentMetadata {
            start_index: 0,
            end_index: 1,
            tail_anchor_index: 1,
            start_message_id: Some("m1".to_string()),
            end_message_id: Some("m2".to_string()),
            tail_anchor_message_id: Some("m2".to_string()),
            reason: "boundary_head".to_string(),
        }));

        let projected = crate::context_runtime::projection::project_view(&conversation, &state);
        let text = projected
            .messages()
            .iter()
            .map(|message| message.as_concat_text())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("alpha"));
        assert!(text.contains("beta"));
        assert!(text.contains("[CONTEXT_RUNTIME_COLLAPSE]"));
        assert!(text.contains("collapsed suffix"));
        assert!(!text.contains("gamma"));
        assert!(!text.contains("delta"));
    }

    #[test]
    fn project_view_refreshes_stale_cached_working_set_against_current_conversation() {
        let conversation = four_message_conversation();
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-project-view-refresh".to_string()),
            summary: "collapsed summary".to_string(),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            created_at: 1,
        });
        state.set_compact_boundary(Some(CompactBoundaryMetadata {
            anchor_index: 1,
            head_index: 1,
            tail_index: 3,
            direction: CompactDirection::UpTo,
            anchor_message_id: None,
            head_message_id: None,
            tail_message_id: None,
            created_at: 1,
        }));
        state.set_preserved_segment(Some(PreservedSegmentMetadata {
            start_index: 3,
            end_index: 3,
            tail_anchor_index: 3,
            start_message_id: None,
            end_message_id: None,
            tail_anchor_message_id: None,
            reason: "committed_collapse".to_string(),
        }));
        state.set_compact_boundary(Some(CompactBoundaryMetadata {
            anchor_index: 1,
            head_index: 1,
            tail_index: 99,
            direction: CompactDirection::UpTo,
            anchor_message_id: None,
            head_message_id: None,
            tail_message_id: None,
            created_at: 2,
        }));
        state.set_preserved_segment(Some(PreservedSegmentMetadata {
            start_index: 99,
            end_index: 99,
            tail_anchor_index: 99,
            start_message_id: None,
            end_message_id: None,
            tail_anchor_message_id: None,
            reason: "stale".to_string(),
        }));

        let projected = crate::context_runtime::projection::project_view(&conversation, &state);
        let text = projected
            .messages()
            .iter()
            .map(|message| message.as_concat_text())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("collapsed summary"));
        assert!(text.contains("four"));
    }

    #[test]
    fn finalize_projection_refresh_preserves_from_direction_boundary() {
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-from-store".to_string()),
            summary: "tail collapse".to_string(),
            start_index: 3,
            end_index: 5,
            direction: CompactDirection::From,
            start_message_id: Some("m4".to_string()),
            end_message_id: Some("m5".to_string()),
            created_at: 1,
        });

        crate::context_runtime::store::finalize_projection_refresh(5, &mut state);

        let boundary = state.compact_boundary().expect("boundary");
        assert_eq!(boundary.direction, CompactDirection::From);
        assert_eq!(boundary.head_index, 2);
        let preserved = state.preserved_segment().expect("preserved");
        assert_eq!(preserved.start_index, 0);
        assert_eq!(preserved.end_index, 2);
    }

    #[test]
    fn refresh_projection_restores_entry_log_before_reconcile() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("alpha"),
            Message::assistant().with_text("beta"),
            Message::user().with_text("gamma"),
        ]);

        let mut state = ContextRuntimeState::default();
        let snip = SnipRemovalState {
            removed_indexes: vec![1],
            removed_message_ids: vec![],
            removed_count: 1,
            token_estimate_freed: 10,
            reason: "test".to_string(),
            created_at: 1,
        };
        state.set_snip_entries(vec![snip.clone()]);
        state.snip_entries_mut().clear();

        crate::context_runtime::projection::refresh_projection_only(&conversation, &mut state);

        assert!(state.snip_entries().is_empty());
        assert!(state.last_projection_stats.is_some());
    }

    #[test]
    fn persistent_runtime_state_restores_entry_log_before_checking_presence() {
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-persist".to_string()),
            summary: "persist".to_string(),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            created_at: 1,
        });
        state.committed_collapses_mut().clear();

        assert!(crate::context_runtime::store::has_persistent_runtime_state(
            &state
        ));
    }

    #[test]
    fn rewind_restores_entry_log_before_persistent_state_decision() {
        let conversation = four_message_conversation();

        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-rewind".to_string()),
            summary: "persist".to_string(),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            created_at: 1,
        });
        state.committed_collapses_mut().clear();

        let rewound = crate::context_runtime::rewind_context_runtime_state(&conversation, &state)
            .expect("rewound state");
        assert_eq!(rewound.committed_collapses().len(), 1);
    }

    #[test]
    fn should_auto_compact_restores_entry_log_before_budget_decision() {
        let provider = tiny_context_provider();
        let conversation = four_message_conversation();
        let mut state = ContextRuntimeState::default();
        state.set_staged_snapshot(Some(CollapseSnapshot {
            snapshot_id: Some("stage-auto".to_string()),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            summary: "stage".to_string(),
            risk: 0.9,
            staged_at: 1,
        }));
        let _ = state.take_staged_snapshot();

        let decision = block_on(crate::context_runtime::should_auto_compact(
            &provider,
            &conversation,
            &state,
            None,
        ))
        .expect("auto compact decision");

        assert!(decision);
    }

    #[test]
    fn maybe_advance_restores_entry_log_before_commit_decision() {
        let provider = tiny_context_provider();
        let conversation = four_message_conversation();
        let mut state = ContextRuntimeState::default();
        state.set_staged_snapshot(Some(CollapseSnapshot {
            snapshot_id: Some("stage-advance".to_string()),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            summary: "stage".to_string(),
            risk: 0.9,
            staged_at: 1,
        }));
        let _ = state.take_staged_snapshot();

        let advance = block_on(crate::context_runtime::maybe_advance_runtime_state(
            &provider,
            &conversation,
            &mut state,
            false,
            None,
        ))
        .expect("advance");

        assert_eq!(advance.kind, ContextRuntimeAdvanceKind::CommittedCollapse);
        assert_eq!(state.committed_collapses().len(), 1);
    }

    #[test]
    fn recover_on_overflow_restores_entry_log_before_commit_decision() {
        let provider = tiny_context_provider();
        let conversation = four_message_conversation();
        let mut state = ContextRuntimeState::default();
        state.set_staged_snapshot(Some(CollapseSnapshot {
            snapshot_id: Some("stage-recover".to_string()),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            summary: "stage".to_string(),
            risk: 0.9,
            staged_at: 1,
        }));
        let _ = state.take_staged_snapshot();

        let recovery = block_on(crate::context_runtime::recover_on_overflow(
            &provider,
            &conversation,
            &mut state,
        ))
        .expect("recovery");

        assert_eq!(
            recovery.kind,
            ContextRuntimeRecoveryKind::StagedCollapseCommitted
        );
        assert_eq!(state.committed_collapses().len(), 1);
    }

    #[test]
    fn project_prepared_view_matches_project_view_for_prepared_state() {
        let conversation = four_message_conversation();
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-prepared-view".to_string()),
            summary: "prepared collapse".to_string(),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            created_at: 1,
        });

        let prepared =
            crate::context_runtime::projection::prepared_projection_state(&conversation, &state);
        let via_project_view =
            crate::context_runtime::projection::project_view(&conversation, &prepared);
        let via_prepared_view =
            crate::context_runtime::projection::project_prepared_view(&conversation, &prepared);

        assert_eq!(via_prepared_view.messages(), via_project_view.messages());
    }

    #[test]
    fn prepared_runtime_budget_matches_projected_runtime_budget() {
        let provider = tiny_context_provider();
        let conversation = four_message_conversation();
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-budget".to_string()),
            summary: "budget collapse".to_string(),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            created_at: 1,
        });

        let prepared =
            crate::context_runtime::projection::prepared_projection_state(&conversation, &state);
        let via_default = crate::context_runtime::projection::projected_runtime_budget(
            &provider,
            &conversation,
            &prepared,
        );
        let via_prepared =
            crate::context_runtime::projection::projected_runtime_budget_for_prepared_state(
                &provider,
                &conversation,
                &prepared,
            );

        assert_eq!(via_prepared.projected_tokens, via_default.projected_tokens);
        assert_eq!(via_prepared.context_limit, via_default.context_limit);
        assert_eq!(
            via_prepared.output_reserve_tokens,
            via_default.output_reserve_tokens
        );
    }

    #[test]
    fn manual_session_memory_compaction_clears_previous_working_set_state() {
        let provider = tiny_context_provider();
        let conversation = long_runtime_conversation();
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-manual".to_string()),
            summary: "old collapse".to_string(),
            start_index: 1,
            end_index: 4,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            created_at: 1,
        });
        state.set_staged_snapshot(Some(CollapseSnapshot {
            snapshot_id: Some("stage-manual".to_string()),
            start_index: 1,
            end_index: 4,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            summary: "staged".to_string(),
            risk: 0.8,
            staged_at: 1,
        }));
        state.set_snip_entries(vec![SnipRemovalState {
            removed_indexes: vec![1],
            removed_message_ids: vec!["long-1".to_string()],
            removed_count: 1,
            token_estimate_freed: 50,
            reason: "test".to_string(),
            created_at: 1,
        }]);

        let advance = block_on(crate::context_runtime::maybe_advance_runtime_state(
            &provider,
            &conversation,
            &mut state,
            true,
            None,
        ))
        .expect("manual advance");

        assert_eq!(
            advance.kind,
            ContextRuntimeAdvanceKind::SessionMemoryCompaction
        );
        assert!(state.session_memory().is_some());
        assert!(state.committed_collapses().is_empty());
        assert!(state.staged_snapshot().is_none());
        assert!(state.snip_entries().is_empty());
        assert_eq!(
            state.last_compact_reason.as_deref(),
            Some("manual_session_memory")
        );
        let validation = state
            .last_post_compact_validation
            .as_ref()
            .expect("post compact validation");
        assert_eq!(validation.compact_kind, "session_memory");
        assert_eq!(validation.reason, "manual_session_memory");
        assert!(validation.retained_start_index.is_some());
    }

    #[test]
    fn overflow_session_memory_compaction_clears_previous_working_set_state() {
        let provider = tiny_context_provider();
        let conversation = long_runtime_conversation();
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("commit-overflow".to_string()),
            summary: "old collapse".to_string(),
            start_index: 1,
            end_index: 4,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            created_at: 1,
        });
        state.set_snip_entries(vec![SnipRemovalState {
            removed_indexes: vec![1],
            removed_message_ids: vec!["long-1".to_string()],
            removed_count: 1,
            token_estimate_freed: 50,
            reason: "test".to_string(),
            created_at: 1,
        }]);

        let recovery = block_on(crate::context_runtime::recover_on_overflow(
            &provider,
            &conversation,
            &mut state,
        ))
        .expect("overflow recovery");

        assert_eq!(
            recovery.kind,
            ContextRuntimeRecoveryKind::SessionMemoryCompaction
        );
        assert!(state.session_memory().is_some());
        assert!(state.committed_collapses().is_empty());
        assert!(state.staged_snapshot().is_none());
        assert!(state.snip_entries().is_empty());
        assert_eq!(
            state.last_recovery_result.as_deref(),
            Some("session_memory_compaction")
        );
    }

    #[test]
    fn refresh_projection_does_not_append_duplicate_projection_entries() {
        let conversation = four_message_conversation();
        let mut state = ContextRuntimeState::default();

        crate::context_runtime::projection::refresh_projection_only(&conversation, &mut state);
        let first_entry_log_len = state.store.entry_log.len();
        let first_snips = state.snip_entries().to_vec();
        let first_micro = state.microcompact_entries().to_vec();
        let first_boundary = state.compact_boundary().cloned();
        let first_preserved = state.preserved_segment().cloned();

        crate::context_runtime::projection::refresh_projection_only(&conversation, &mut state);

        assert_eq!(state.store.entry_log.len(), first_entry_log_len);
        assert_eq!(state.snip_entries(), first_snips.as_slice());
        assert_eq!(state.microcompact_entries(), first_micro.as_slice());
        assert_eq!(state.compact_boundary(), first_boundary.as_ref());
        assert_eq!(state.preserved_segment(), first_preserved.as_ref());
    }

    #[test]
    fn refresh_projection_with_session_memory_does_not_append_duplicate_working_set_entries() {
        let conversation = four_message_conversation();
        let mut state = ContextRuntimeState::default();
        state.set_session_memory(Some(SessionMemoryState {
            summary: "memory".to_string(),
            summarized_through_message_id: Some("m1".to_string()),
            preserved_start_index: 2,
            preserved_end_index: 3,
            preserved_start_message_id: None,
            preserved_end_message_id: None,
            preserved_message_count: 2,
            preserved_token_estimate: 8,
            tail_anchor_index: 3,
            tail_anchor_message_id: None,
            updated_at: 1,
        }));

        crate::context_runtime::projection::refresh_projection_only(&conversation, &mut state);
        let first_entry_log_len = state.store.entry_log.len();
        let first_boundary = state.compact_boundary().cloned();
        let first_preserved = state.preserved_segment().cloned();

        crate::context_runtime::projection::refresh_projection_only(&conversation, &mut state);

        assert_eq!(state.store.entry_log.len(), first_entry_log_len);
        assert_eq!(state.compact_boundary(), first_boundary.as_ref());
        assert_eq!(state.preserved_segment(), first_preserved.as_ref());
    }

    #[test]
    fn refresh_projection_with_session_memory_extends_tail_to_latest_message() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_id("m1").with_text("one"),
            Message::assistant().with_id("m2").with_text("two"),
            Message::user().with_id("m3").with_text("three"),
            Message::assistant().with_id("m4").with_text("four"),
            Message::user().with_id("m5").with_text("five"),
            Message::assistant().with_id("m6").with_text("six"),
        ]);
        let mut state = ContextRuntimeState::default();
        state.set_session_memory(Some(SessionMemoryState {
            summary: "memory".to_string(),
            summarized_through_message_id: Some("m2".to_string()),
            preserved_start_index: 2,
            preserved_end_index: 3,
            preserved_start_message_id: Some("m3".to_string()),
            preserved_end_message_id: Some("m4".to_string()),
            preserved_message_count: 2,
            preserved_token_estimate: 8,
            tail_anchor_index: 3,
            tail_anchor_message_id: Some("m4".to_string()),
            updated_at: 1,
        }));

        crate::context_runtime::projection::refresh_projection_only(&conversation, &mut state);

        let memory = state.session_memory().expect("session memory");
        assert_eq!(memory.preserved_start_index, 2);
        assert_eq!(memory.preserved_end_index, 5);
        assert_eq!(memory.tail_anchor_index, 5);
        assert_eq!(memory.summarized_through_message_id.as_deref(), Some("m2"));
        assert_eq!(memory.preserved_start_message_id.as_deref(), Some("m3"));
        assert_eq!(memory.preserved_end_message_id.as_deref(), Some("m6"));
        assert_eq!(memory.tail_anchor_message_id.as_deref(), Some("m6"));
        assert_eq!(memory.preserved_message_count, 4);
        assert_eq!(memory.preserved_token_estimate, 4);
    }

    #[test]
    fn structural_noop_setters_do_not_append_duplicate_entry_log_events() {
        let mut state = ContextRuntimeState::default();

        state.clear_committed_collapses();
        state.set_staged_snapshot(None);
        state.set_session_memory(None);
        assert!(state.store.entry_log.is_empty());

        let snapshot = CollapseSnapshot {
            snapshot_id: Some("snapshot-noop".to_string()),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: None,
            end_message_id: None,
            summary: "same snapshot".to_string(),
            risk: 0.7,
            staged_at: 1,
        };
        state.set_staged_snapshot(Some(snapshot.clone()));
        let after_first_snapshot = state.store.entry_log.len();
        state.set_staged_snapshot(Some(snapshot));
        assert_eq!(state.store.entry_log.len(), after_first_snapshot);

        let session_memory = SessionMemoryState {
            summary: "memory".to_string(),
            summarized_through_message_id: None,
            preserved_start_index: 2,
            preserved_end_index: 3,
            preserved_start_message_id: None,
            preserved_end_message_id: None,
            preserved_message_count: 2,
            preserved_token_estimate: 10,
            tail_anchor_index: 3,
            tail_anchor_message_id: None,
            updated_at: 1,
        };
        state.set_session_memory(Some(session_memory.clone()));
        let after_first_memory = state.store.entry_log.len();
        state.set_session_memory(Some(session_memory));
        assert_eq!(state.store.entry_log.len(), after_first_memory);

        state.clear_committed_collapses();
        assert_eq!(state.store.entry_log.len(), after_first_memory);
    }

    #[test]
    fn relink_state_for_replaced_conversation_extends_session_memory_tail() {
        let old_conversation = Conversation::new_unvalidated(vec![
            Message::user().with_id("old-1").with_text("one"),
            Message::assistant().with_id("old-2").with_text("two"),
            Message::user().with_id("old-3").with_text("three"),
            Message::assistant().with_id("old-4").with_text("four"),
        ]);
        let new_conversation = Conversation::new_unvalidated(vec![
            Message::user().with_id("new-1").with_text("one"),
            Message::assistant().with_id("new-2").with_text("two"),
            Message::user().with_id("new-3").with_text("three"),
            Message::assistant().with_id("new-4").with_text("four"),
            Message::user().with_id("new-5").with_text("five"),
        ]);
        let mut state = ContextRuntimeState::default();
        state.set_session_memory(Some(SessionMemoryState {
            summary: "memory".to_string(),
            summarized_through_message_id: Some("old-2".to_string()),
            preserved_start_index: 2,
            preserved_end_index: 3,
            preserved_start_message_id: Some("old-3".to_string()),
            preserved_end_message_id: Some("old-4".to_string()),
            preserved_message_count: 2,
            preserved_token_estimate: 8,
            tail_anchor_index: 3,
            tail_anchor_message_id: Some("old-4".to_string()),
            updated_at: 1,
        }));

        crate::context_runtime::store::relink_state_for_replaced_conversation(
            &old_conversation,
            &new_conversation,
            &mut state,
        );
        crate::context_runtime::projection::refresh_projection_only(&new_conversation, &mut state);

        let memory = state.session_memory().expect("session memory");
        assert_eq!(memory.preserved_start_index, 2);
        assert_eq!(memory.preserved_end_index, 4);
        assert_eq!(memory.tail_anchor_index, 4);
        assert_eq!(
            memory.summarized_through_message_id.as_deref(),
            Some("new-2")
        );
        assert_eq!(memory.preserved_start_message_id.as_deref(), Some("new-3"));
        assert_eq!(memory.preserved_end_message_id.as_deref(), Some("new-5"));
        assert_eq!(memory.tail_anchor_message_id.as_deref(), Some("new-5"));
        assert_eq!(memory.preserved_message_count, 3);
    }

    #[test]
    fn refresh_projection_only_invalidates_collapse_commit_with_missing_message_ids() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_id("msg-1").with_text("one"),
            Message::assistant().with_id("msg-2").with_text("two"),
            Message::user().with_id("msg-3").with_text("three"),
        ]);
        let mut state = ContextRuntimeState::default();
        state.append_collapse_commit(CollapseCommit {
            commit_id: Some("missing-id-commit".to_string()),
            summary: "collapsed".to_string(),
            start_index: 1,
            end_index: 3,
            direction: CompactDirection::UpTo,
            start_message_id: Some("missing-start".to_string()),
            end_message_id: Some("missing-end".to_string()),
            created_at: 1,
        });

        crate::context_runtime::projection::refresh_projection_only(&conversation, &mut state);

        assert!(
            state.committed_collapses().is_empty(),
            "collapse commit with missing message anchors should be invalidated on read-side refresh"
        );
    }

    #[test]
    fn refresh_projection_only_invalidates_session_memory_with_missing_message_ids() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_id("msg-1").with_text("one"),
            Message::assistant().with_id("msg-2").with_text("two"),
            Message::user().with_id("msg-3").with_text("three"),
        ]);
        let mut state = ContextRuntimeState::default();
        state.set_session_memory(Some(SessionMemoryState {
            summary: "memory".to_string(),
            summarized_through_message_id: Some("msg-1".to_string()),
            preserved_start_index: 1,
            preserved_end_index: 2,
            preserved_start_message_id: Some("missing-start".to_string()),
            preserved_end_message_id: Some("missing-end".to_string()),
            preserved_message_count: 2,
            preserved_token_estimate: 8,
            tail_anchor_index: 2,
            tail_anchor_message_id: Some("missing-tail".to_string()),
            updated_at: 1,
        }));

        crate::context_runtime::projection::refresh_projection_only(&conversation, &mut state);

        assert!(
            state.session_memory().is_none(),
            "session memory with missing message anchors should be invalidated on read-side refresh"
        );
    }
}
