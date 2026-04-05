use std::time::Instant;

use anyhow::{anyhow, Result};
use rmcp::model::Tool;
use serde::Serialize;

use crate::agents::agent::{
    Agent, AgentEvent, PreparedReplyConversation, PreparedTurnInput, ReplyBootstrap,
    TurnStartHandling,
};
use crate::agents::types::SessionConfig;
use crate::context_mgmt::{
    check_if_compaction_needed, compact_messages_with_active_strategy, current_compaction_strategy,
    DEFAULT_COMPACTION_THRESHOLD,
};
use crate::conversation::message::{Message, SystemNotificationType};
use crate::prompt_template::render_global_file;
use crate::session::SessionManager;

use super::{
    auto_compaction_inline_message, compaction_checkpoint, compaction_strategy_label,
    maybe_plan_swarm_upgrade, mode_system_prompt, snapshot_final_output_state,
    DelegationRuntimeState, HarnessCheckpointStore, HarnessMode, HarnessTranscriptStore,
    RuntimeNotificationInput, SessionHarnessStore,
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
        let needs_auto_compact = !is_manual_compact
            && check_if_compaction_needed(
                self.provider().await?.as_ref(),
                &conversation,
                None,
                &session,
            )
            .await?;
        tracing::info!(
            "[PERF] check_compaction done, elapsed: {:?}",
            reply_start.elapsed()
        );

        Ok(ReplyBootstrap {
            current_mode: transcript_store.load_mode(&session_config.id).await?,
            active_compaction_strategy: current_compaction_strategy(),
            needs_auto_compact,
            is_manual_compact,
            session,
            conversation,
        })
    }

    pub(crate) async fn prepare_conversation_for_reply_loop(
        &self,
        transcript_store: &SessionHarnessStore,
        session_config: &SessionConfig,
        bootstrap: &ReplyBootstrap,
    ) -> Result<PreparedReplyConversation> {
        if !bootstrap.needs_auto_compact && !bootstrap.is_manual_compact {
            return Ok(PreparedReplyConversation {
                events: Vec::new(),
                conversation: Some(bootstrap.conversation.clone()),
                should_enter_reply_loop: true,
            });
        }

        let mut events = Vec::new();
        if !bootstrap.is_manual_compact {
            let config = crate::config::Config::global();
            let threshold = config
                .get_param::<f64>("AGIME_AUTO_COMPACT_THRESHOLD")
                .unwrap_or(DEFAULT_COMPACTION_THRESHOLD);
            let threshold_percentage = (threshold * 100.0) as u32;

            events.push(AgentEvent::Message(
                Message::assistant().with_system_notification(
                    SystemNotificationType::InlineMessage,
                    auto_compaction_inline_message(threshold_percentage),
                ),
            ));
        }

        events.push(AgentEvent::Message(
            Message::assistant().with_system_notification(
                SystemNotificationType::ThinkingMessage,
                "AGIME is compacting the conversation...",
            ),
        ));

        match compact_messages_with_active_strategy(
            self.provider().await?.as_ref(),
            &bootstrap.conversation,
            bootstrap.is_manual_compact,
        )
        .await
        {
            Ok((compacted_conversation, summarization_usage)) => {
                transcript_store
                    .replace_conversation(&session_config.id, &compacted_conversation)
                    .await?;
                if bootstrap.active_compaction_strategy.is_cfpm() {
                    let reason = if bootstrap.is_manual_compact {
                        "manual_compaction"
                    } else if bootstrap.needs_auto_compact {
                        "auto_compaction"
                    } else {
                        "compaction"
                    };
                    if let Err(err) = SessionManager::replace_cfpm_memory_facts_from_conversation(
                        &session_config.id,
                        &compacted_conversation,
                        reason,
                    )
                    .await
                    {
                        tracing::warn!("Failed to refresh CFPM memory facts: {}", err);
                    }
                }
                Self::update_session_metrics(session_config, &summarization_usage, true).await?;

                events.push(AgentEvent::HistoryReplaced(compacted_conversation.clone()));
                let _ = transcript_store
                    .record_checkpoint(
                        &session_config.id,
                        compaction_checkpoint(
                            0,
                            bootstrap.current_mode,
                            bootstrap.active_compaction_strategy,
                            "pre_loop_compaction",
                        ),
                    )
                    .await;

                events.push(AgentEvent::Message(
                    Message::assistant().with_system_notification(
                        SystemNotificationType::InlineMessage,
                        format!(
                            "Compaction complete (strategy: {})",
                            compaction_strategy_label(bootstrap.active_compaction_strategy)
                        ),
                    ),
                ));

                Ok(PreparedReplyConversation {
                    events,
                    conversation: Some(compacted_conversation),
                    should_enter_reply_loop: !bootstrap.is_manual_compact,
                })
            }
            Err(e) => Ok(PreparedReplyConversation {
                events: vec![AgentEvent::Message(Message::assistant().with_text(format!(
                    "Ran into this error trying to compact: {e}.\n\nPlease try again or create a new session"
                )))],
                conversation: None,
                should_enter_reply_loop: false,
            }),
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
        compaction_count: u32,
        system_prompt: &str,
        current_mode: HarnessMode,
        coordinator_execution_mode: super::CoordinatorExecutionMode,
        delegation: &DelegationRuntimeState,
    ) -> Result<PreparedTurnInput> {
        let turn_conversation = conversation.clone();
        let (conversation_with_memory, memory_system_extra) = self
            .inject_runtime_memory_context(session_id, &turn_conversation, compaction_count)
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
