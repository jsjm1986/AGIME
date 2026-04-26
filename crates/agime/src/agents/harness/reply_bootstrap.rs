use std::time::Instant;

use anyhow::{anyhow, Result};
use rmcp::model::Tool;
use serde::Serialize;

use crate::agents::agent::{
    Agent, AgentEvent, PreparedReplyConversation, PreparedTurnInput, ReplyBootstrap,
    TurnStartHandling,
};
use crate::agents::types::SessionConfig;
use crate::context_runtime::{
    load_context_runtime_state, maybe_advance_runtime_state, project_prepared_view,
    refresh_projection_only, save_context_runtime_state,
};
use crate::conversation::message::{Message, SystemNotificationType};
use crate::prompt_template::render_global_file;
use crate::runtime_profile::resolve_from_model_config;
use crate::session::SessionManager;

use super::{
    auto_compaction_inline_message, compaction_checkpoint, maybe_plan_swarm_upgrade,
    mode_system_prompt, record_transition, snapshot_final_output_state, DelegationRuntimeState,
    HarnessCheckpointStore, HarnessMode, HarnessTranscriptStore, RuntimeNotificationInput,
    SessionHarnessStore, SharedTransitionTrace, TransitionKind,
};

impl Agent {
    fn render_leader_swarm_prompt(delegation: &DelegationRuntimeState) -> Option<String> {
        #[derive(Serialize)]
        struct LeaderPromptContext {
            delegation_mode: String,
            current_depth: u32,
            max_depth: u32,
            targets: String,
            result_contract: String,
            write_scope: String,
        }

        if !crate::agents::harness::native_swarm_tool_enabled()
            && !crate::agents::harness::planner_auto_swarm_enabled()
        {
            return None;
        }

        render_global_file(
            "leader_system.md",
            &LeaderPromptContext {
                delegation_mode: format!("{:?}", delegation.mode),
                current_depth: delegation.current_depth,
                max_depth: delegation.max_depth,
                targets: if delegation.target_artifacts.is_empty() {
                    "none".to_string()
                } else {
                    delegation.target_artifacts.join(", ")
                },
                result_contract: if delegation.result_contract.is_empty() {
                    "none".to_string()
                } else {
                    delegation.result_contract.join(", ")
                },
                write_scope: if delegation.write_scope.is_empty() {
                    "none".to_string()
                } else {
                    delegation.write_scope.join(", ")
                },
            },
        )
        .ok()
    }

    pub(crate) async fn record_incoming_message_for_reply(
        &self,
        transcript_store: &SessionHarnessStore,
        session_config: &SessionConfig,
        user_message: &Message,
        message_text: &str,
    ) -> Result<()> {
        let slash_command_recipe = if message_text.trim().starts_with('/') {
            let command = message_text.split_whitespace().next();
            command.and_then(crate::slash_commands::resolve_slash_command)
        } else {
            None
        };

        if let Some(recipe) = slash_command_recipe {
            let prompt = [recipe.instructions.as_deref(), recipe.prompt.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n\n");
            let prompt_message = Message::user()
                .with_text(prompt)
                .with_visibility(false, true);
            transcript_store
                .append_message(&session_config.id, &prompt_message)
                .await?;
            transcript_store
                .append_message(
                    &session_config.id,
                    &user_message.clone().with_visibility(true, false),
                )
                .await?;
        } else {
            transcript_store
                .append_message(&session_config.id, user_message)
                .await?;
        }

        Ok(())
    }

    pub(crate) async fn bootstrap_reply_state(
        &self,
        transcript_store: &SessionHarnessStore,
        session_config: &SessionConfig,
        is_manual_compact: bool,
        reply_start: Instant,
        transition_trace: &SharedTransitionTrace,
    ) -> Result<ReplyBootstrap> {
        let session = SessionManager::get_session(&session_config.id, true).await?;
        let conversation = session
            .conversation
            .clone()
            .ok_or_else(|| anyhow!("Session {} has no conversation", session_config.id))?;

        tracing::info!(
            "[PERF] check_compaction start, elapsed: {:?}",
            reply_start.elapsed()
        );
        let context_runtime_state = load_context_runtime_state(&session_config.id).await?;
        let runtime_profile = resolve_from_model_config(&self.provider().await?.get_model_config());
        let needs_auto_compact = !is_manual_compact
            && crate::context_runtime::should_auto_compact(
                self.provider().await?.as_ref(),
                &conversation,
                &context_runtime_state,
                Some(runtime_profile.auto_compact_threshold),
            )
            .await?;
        if is_manual_compact {
            let mut metadata = std::collections::BTreeMap::new();
            metadata.insert("session_id".to_string(), session_config.id.clone());
            record_transition(
                transition_trace,
                0,
                transcript_store.load_mode(&session_config.id).await?,
                TransitionKind::ReplyBootstrap,
                "manual_compact_requested",
                metadata,
            )
            .await;
        } else if needs_auto_compact {
            let mut metadata = std::collections::BTreeMap::new();
            metadata.insert(
                "threshold".to_string(),
                runtime_profile.auto_compact_threshold.to_string(),
            );
            record_transition(
                transition_trace,
                0,
                transcript_store.load_mode(&session_config.id).await?,
                TransitionKind::ReplyBootstrap,
                "auto_compact_pre_turn",
                metadata,
            )
            .await;
        }
        tracing::info!(
            "[PERF] check_compaction done, elapsed: {:?}",
            reply_start.elapsed()
        );

        Ok(ReplyBootstrap {
            current_mode: transcript_store.load_mode(&session_config.id).await?,
            needs_auto_compact,
            is_manual_compact,
            session,
            conversation,
            context_runtime_state,
        })
    }

    pub(crate) async fn prepare_conversation_for_reply_loop(
        &self,
        transcript_store: &SessionHarnessStore,
        session_config: &SessionConfig,
        bootstrap: &ReplyBootstrap,
        transition_trace: &SharedTransitionTrace,
    ) -> Result<PreparedReplyConversation> {
        let mut events = Vec::new();
        let runtime_profile = resolve_from_model_config(&self.provider().await?.get_model_config());
        if !bootstrap.is_manual_compact {
            if bootstrap.needs_auto_compact {
                let threshold = runtime_profile.auto_compact_threshold;
                let threshold_percentage = (threshold * 100.0) as u32;

                events.push(AgentEvent::Message(
                    Message::assistant().with_system_notification(
                        SystemNotificationType::InlineMessage,
                        auto_compaction_inline_message(threshold_percentage),
                    ),
                ));
            }
        }

        let mut context_runtime_state = bootstrap.context_runtime_state.clone();
        match maybe_advance_runtime_state(
            self.provider().await?.as_ref(),
            &bootstrap.conversation,
            &mut context_runtime_state,
            bootstrap.is_manual_compact,
            Some(runtime_profile.auto_compact_threshold),
        )
        .await
        {
            Ok(outcome) => {
                if !matches!(
                    outcome.kind,
                    crate::context_runtime::ContextRuntimeAdvanceKind::Noop
                ) {
                    events.push(AgentEvent::Message(
                        Message::assistant().with_system_notification(
                            SystemNotificationType::ThinkingMessage,
                            "AGIME is updating runtime context...",
                        ),
                    ));
                }

                crate::context_runtime::save_context_runtime_state(
                    &session_config.id,
                    &context_runtime_state,
                )
                .await?;

                let detail = match outcome.kind {
                    crate::context_runtime::ContextRuntimeAdvanceKind::Noop => "noop",
                    crate::context_runtime::ContextRuntimeAdvanceKind::ProjectionRefresh => {
                        "projection_refresh"
                    }
                    crate::context_runtime::ContextRuntimeAdvanceKind::StagedCollapse => {
                        "staged_collapse"
                    }
                    crate::context_runtime::ContextRuntimeAdvanceKind::CommittedCollapse => {
                        "committed_collapse"
                    }
                    crate::context_runtime::ContextRuntimeAdvanceKind::SessionMemoryCompaction => {
                        "session_memory_compaction"
                    }
                };
                let _ = transcript_store
                    .record_checkpoint(
                        &session_config.id,
                        compaction_checkpoint(0, bootstrap.current_mode, detail),
                    )
                    .await;

                let reason = match outcome.kind {
                    crate::context_runtime::ContextRuntimeAdvanceKind::Noop => None,
                    crate::context_runtime::ContextRuntimeAdvanceKind::ProjectionRefresh => {
                        Some("runtime_projection_refresh")
                    }
                    crate::context_runtime::ContextRuntimeAdvanceKind::StagedCollapse => {
                        Some("staged_collapse_applied")
                    }
                    crate::context_runtime::ContextRuntimeAdvanceKind::CommittedCollapse => {
                        Some("committed_collapse_applied")
                    }
                    crate::context_runtime::ContextRuntimeAdvanceKind::SessionMemoryCompaction => {
                        Some("session_memory_compaction_applied")
                    }
                };
                if let Some(reason) = reason {
                    let mut metadata = std::collections::BTreeMap::new();
                    metadata.insert("detail".to_string(), detail.to_string());
                    metadata.insert(
                        "freed_token_estimate".to_string(),
                        context_runtime_state
                            .last_projection_stats
                            .as_ref()
                            .map(|stats| stats.freed_token_estimate)
                            .unwrap_or_default()
                            .to_string(),
                    );
                    record_transition(
                        transition_trace,
                        0,
                        bootstrap.current_mode,
                        TransitionKind::ReplyBootstrap,
                        reason,
                        metadata,
                    )
                    .await;
                }

                if !matches!(
                    outcome.kind,
                    crate::context_runtime::ContextRuntimeAdvanceKind::Noop
                ) {
                    let freed_tokens = context_runtime_state
                        .last_projection_stats
                        .as_ref()
                        .map(|stats| stats.freed_token_estimate)
                        .unwrap_or_default();
                    events.push(AgentEvent::Message(
                        Message::assistant().with_system_notification(
                            SystemNotificationType::InlineMessage,
                            format!(
                                "Context runtime updated ({}, freed ~{} tokens)",
                                detail.replace('_', " "),
                                freed_tokens
                            ),
                        ),
                    ));
                }

                Ok(PreparedReplyConversation {
                    events,
                    conversation: Some(bootstrap.conversation.clone()),
                    should_enter_reply_loop: !bootstrap.is_manual_compact,
                })
            }
            Err(e) => {
                tracing::warn!(
                    session_id = %session_config.id,
                    error = %e,
                    "Context runtime update failed during reply preparation; continuing without a refreshed runtime projection"
                );
                Ok(PreparedReplyConversation {
                    events,
                    conversation: Some(bootstrap.conversation.clone()),
                    should_enter_reply_loop: true,
                })
            }
        }
    }

    pub(crate) async fn begin_turn(
        &self,
        transcript_store: &SessionHarnessStore,
        session_config: &SessionConfig,
        turns_taken: &mut u32,
        max_turns: u32,
        current_mode: HarnessMode,
    ) -> Result<TurnStartHandling> {
        let final_output_state = {
            let final_output_tool = self.final_output_tool.lock().await;
            snapshot_final_output_state(final_output_tool.as_ref())
        };
        if let Some(final_event) = final_output_state.into_assistant_message() {
            return Ok(TurnStartHandling::BreakWithMessage(final_event));
        }

        *turns_taken += 1;
        if *turns_taken > max_turns {
            return Ok(TurnStartHandling::BreakWithMessage(
                Message::assistant().with_text(
                    "I've reached the maximum number of actions I can do without user input. Would you like me to continue?",
                ),
            ));
        }

        let _ = transcript_store
            .record_checkpoint(
                &session_config.id,
                crate::agents::harness::HarnessCheckpoint::turn_start(*turns_taken, current_mode),
            )
            .await;

        Ok(TurnStartHandling::Continue)
    }

    pub(crate) async fn prepare_turn_input(
        &self,
        session_id: &str,
        conversation: &crate::conversation::Conversation,
        runtime_notifications: Option<&RuntimeNotificationInput>,
        runtime_compactions: u32,
        system_prompt: &str,
        current_mode: HarnessMode,
        coordinator_execution_mode: super::CoordinatorExecutionMode,
        delegation: &DelegationRuntimeState,
    ) -> Result<PreparedTurnInput> {
        let turn_conversation = conversation.clone();
        let mut context_runtime_state = load_context_runtime_state(session_id)
            .await
            .unwrap_or_default();
        let projection_changed =
            refresh_projection_only(&turn_conversation, &mut context_runtime_state);
        if projection_changed {
            let _ = save_context_runtime_state(session_id, &context_runtime_state).await;
        }
        let projected_conversation =
            project_prepared_view(&turn_conversation, &context_runtime_state);
        let (conversation_with_memory, memory_system_extra) = self
            .inject_runtime_memory_context(&projected_conversation, runtime_compactions)
            .await;

        let mut conversation_for_model =
            crate::agents::moim::inject_moim(conversation_with_memory, &self.extension_manager)
                .await;

        let effective_system_prompt = if let Some(extra) = &memory_system_extra {
            format!("{}\n\n{}", system_prompt, extra)
        } else {
            system_prompt.to_string()
        };
        if let Some(runtime_notifications) = runtime_notifications {
            if let Some(notification_message) =
                runtime_notifications.clone().into_agent_input_message()
            {
                conversation_for_model.push(notification_message);
            }
        }
        let effective_system_prompt = if let Some(mode_prompt) = mode_system_prompt(current_mode) {
            format!("{}\n\n{}", effective_system_prompt, mode_prompt)
        } else {
            effective_system_prompt
        };
        let effective_system_prompt =
            if let Some(leader_prompt) = Self::render_leader_swarm_prompt(delegation) {
                format!("{}\n\n{}", effective_system_prompt, leader_prompt)
            } else {
                effective_system_prompt
            };
        let effective_system_prompt = if let Some(addendum) = maybe_plan_swarm_upgrade(
            current_mode,
            coordinator_execution_mode,
            &turn_conversation,
            delegation,
        )
        .prompt_addendum
        {
            format!("{}\n\n{}", effective_system_prompt, addendum)
        } else {
            effective_system_prompt
        };

        Ok(PreparedTurnInput {
            conversation_for_model,
            effective_system_prompt,
        })
    }

    pub(crate) async fn refresh_tools_after_update(
        &self,
        tools_updated: bool,
        working_dir: &std::path::Path,
        tools: &mut Vec<Tool>,
        toolshim_tools: &mut Vec<Tool>,
        system_prompt: &mut String,
    ) -> Result<()> {
        if tools_updated {
            let (next_tools, next_toolshim_tools, next_system_prompt) =
                self.prepare_tools_and_prompt(working_dir).await?;
            *tools = next_tools;
            *toolshim_tools = next_toolshim_tools;
            *system_prompt = next_system_prompt;
        }
        Ok(())
    }
}
