use crate::agents::agent::{
    build_cfpm_runtime_inline_message, current_cfpm_runtime_visibility,
    should_emit_cfpm_runtime_notification, Agent, AgentEvent, HistoryCapturePolicy,
    NoToolTurnHandling, TurnFinalization,
};
use crate::agents::types::SessionConfig;
use crate::context_mgmt::ContextCompactionStrategy;
use crate::conversation::{
    message::{Message, MessageContent, MessageMetadata, SystemNotificationType},
    Conversation,
};
use crate::session::SessionManager;
use anyhow::Result;

use super::{
    derive_execute_completion_outcome, no_tool_turn_action_with_final_output, record_transition,
    resolve_post_turn_transition, snapshot_final_output_state, update_host_completion_outcome,
    update_host_signal_summary, CoordinatorExecutionMode, ExecuteCompletionState,
    HarnessCheckpointStore, HarnessMode, HarnessTranscriptStore, NoToolTurnAction,
    SessionHarnessStore, SharedCoordinatorSignalStore, SharedTransitionTrace, TaskRuntime,
    TransitionKind,
};

impl Agent {
    const POST_TOOL_CONVERSATION_FOLLOW_UP: &str = "Tool execution for this turn has completed. Now answer the user directly in natural language using the completed tool results. Do not call more tools unless another tool is strictly required to answer.";
    const EXECUTE_STRUCTURED_COMPLETION_FOLLOW_UP: &str = "Execution output for this turn is present, but the runtime completion contract is still missing. Call the `final_output` tool now with the structured terminal completion report for this execute surface. Do not start a new tool chain.";

    fn has_user_visible_assistant_text(messages: &Conversation) -> bool {
        messages.messages().iter().any(|message| {
            message.role == rmcp::model::Role::Assistant
                && message.metadata.user_visible
                && message.content.iter().any(|content| {
                    matches!(
                        content,
                        crate::conversation::message::MessageContent::Text(text)
                            if !text.text.trim().is_empty()
                    )
                })
        })
    }

    fn has_user_visible_tool_response(messages: &Conversation) -> bool {
        messages.messages().iter().any(|message| {
            message.metadata.user_visible
                && message
                    .content
                    .iter()
                    .any(|content| matches!(content, MessageContent::ToolResponse(_)))
        })
    }

    fn has_user_visible_assistant_text_after_last_tool_response(messages: &Conversation) -> bool {
        let all_messages = messages.messages();
        let Some(last_tool_response_idx) = all_messages.iter().rposition(|message| {
            message.metadata.user_visible
                && message
                    .content
                    .iter()
                    .any(|content| matches!(content, MessageContent::ToolResponse(_)))
        }) else {
            return Self::has_user_visible_assistant_text(messages);
        };

        all_messages
            .iter()
            .skip(last_tool_response_idx + 1)
            .any(|message| {
                message.role == rmcp::model::Role::Assistant
                    && message.metadata.user_visible
                    && message.content.iter().any(|content| {
                        matches!(
                            content,
                            MessageContent::Text(text) if !text.text.trim().is_empty()
                        )
                    })
            })
    }

    fn build_post_tool_conversation_follow_up_message() -> Message {
        Message::user()
            .with_text(Self::POST_TOOL_CONVERSATION_FOLLOW_UP)
            .with_metadata(MessageMetadata::agent_only())
    }

    fn build_execute_structured_completion_follow_up_message() -> Message {
        Message::user()
            .with_text(Self::EXECUTE_STRUCTURED_COMPLETION_FOLLOW_UP)
            .with_metadata(MessageMetadata::agent_only())
    }

    pub(crate) fn should_complete_coordinator_turn(
        current_mode: HarnessMode,
        coordinator_execution_mode: CoordinatorExecutionMode,
        has_completion_ready: bool,
        required_tools_satisfied: bool,
        has_user_visible_assistant_text: bool,
        has_active_child_tasks: bool,
        has_blocking_signals: bool,
        has_user_visible_tool_response: bool,
    ) -> bool {
        if coordinator_execution_mode == CoordinatorExecutionMode::SingleWorker
            || has_active_child_tasks
        {
            return false;
        }

        if matches!(current_mode, HarnessMode::Execute) {
            return has_completion_ready && required_tools_satisfied;
        }

        if has_user_visible_tool_response {
            return has_user_visible_assistant_text && !has_blocking_signals;
        }

        has_completion_ready || (has_user_visible_assistant_text && !has_blocking_signals)
    }

    pub(crate) async fn handle_no_tool_turn(
        &self,
        no_tools_called: bool,
        did_recovery_compact_this_iteration: bool,
        conversation: &mut Conversation,
        session_config: &SessionConfig,
        initial_messages: &[Message],
    ) -> Result<NoToolTurnHandling> {
        let mut events = Vec::new();
        let mut exit_chat = false;
        let mut retry_requested = false;

        let final_output_state = {
            let final_output_tool = self.final_output_tool.lock().await;
            snapshot_final_output_state(final_output_tool.as_ref())
        };

        match no_tool_turn_action_with_final_output(
            no_tools_called,
            did_recovery_compact_this_iteration,
            &final_output_state,
            "",
        ) {
            NoToolTurnAction::Noop => {}
            NoToolTurnAction::ContinueWithUserPrompt(prompt) => {
                tracing::warn!("Final output tool has not been called yet. Continuing agent loop.");
                events.push(AgentEvent::Message(Message::user().with_text(prompt)));
            }
            NoToolTurnAction::ExitWithAssistantMessage(message) => {
                events.push(AgentEvent::Message(Message::assistant().with_text(message)));
                exit_chat = true;
            }
            NoToolTurnAction::RunRetryLogic => {
                match self
                    .handle_retry_logic(conversation, session_config, initial_messages)
                    .await
                {
                    Ok(should_retry) => {
                        if should_retry {
                            retry_requested = true;
                            tracing::info!("Retry logic triggered, restarting agent loop");
                        } else {
                            exit_chat = true;
                        }
                    }
                    Err(e) => {
                        tracing::error!("Retry logic failed: {}", e);
                        events
                            .push(AgentEvent::Message(Message::assistant().with_text(
                                format!("Retry logic encountered an error: {}", e),
                            )));
                        exit_chat = true;
                    }
                }
            }
        }

        Ok(NoToolTurnHandling {
            events,
            exit_chat,
            retry_requested,
        })
    }

    pub(crate) async fn refresh_cfpm_runtime_after_turn(
        &self,
        session_config: &SessionConfig,
        conversation: &Conversation,
        messages_to_add: &Conversation,
        active_compaction_strategy: ContextCompactionStrategy,
    ) -> Result<Vec<AgentEvent>> {
        let mut events = Vec::new();
        if !active_compaction_strategy.is_cfpm() || messages_to_add.is_empty() {
            return Ok(events);
        }

        if matches!(
            active_compaction_strategy,
            ContextCompactionStrategy::CfpmMemoryV2
        ) {
            let sem = self.cfpm_v2_extract_semaphore.clone();
            if let Ok(permit) = sem.clone().try_acquire_owned() {
                if let Ok(provider) = self.provider().await {
                    let sid = session_config.id.clone();
                    let msgs = conversation.messages().to_vec();
                    let new_msgs: Vec<Message> = messages_to_add.messages().to_vec();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let mut all_msgs = msgs;
                        all_msgs.extend(new_msgs.clone());
                        match crate::session::cfpm_extract_v2::extract_memory_facts_via_llm(
                            provider.as_ref(),
                            &all_msgs,
                        )
                        .await
                        {
                            Ok(drafts) if !drafts.is_empty() => {
                                if let Err(e) = SessionManager::merge_cfpm_memory_facts(
                                    &sid,
                                    drafts,
                                    "v2_llm_extract",
                                )
                                .await
                                {
                                    tracing::warn!("V2 LLM merge failed: {}", e);
                                }
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!(
                                    "V2 LLM extraction failed, falling back to V1: {}",
                                    e
                                );
                                let _ = SessionManager::refresh_cfpm_memory_facts_from_recent_messages_with_report(
                                    &sid,
                                    &new_msgs,
                                    "v2_fallback",
                                )
                                .await;
                            }
                        }
                    });
                }
            }
            return Ok(events);
        }

        match SessionManager::refresh_cfpm_memory_facts_from_recent_messages_with_report(
            &session_config.id,
            messages_to_add.messages(),
            "turn_checkpoint",
        )
        .await
        {
            Ok(report) => {
                let visibility = current_cfpm_runtime_visibility();
                if should_emit_cfpm_runtime_notification(visibility, &report) {
                    let msg = build_cfpm_runtime_inline_message(&report, visibility);
                    events.push(AgentEvent::Message(
                        Message::assistant()
                            .with_system_notification(SystemNotificationType::InlineMessage, msg),
                    ));
                }
            }
            Err(err) => {
                tracing::warn!(
                    "Failed to refresh CFPM memory facts from recent messages: {}",
                    err
                );
            }
        }

        Ok(events)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn finalize_turn(
        &self,
        conversation: &mut Conversation,
        messages_to_add: &mut Conversation,
        no_tools_called: bool,
        did_recovery_compact_this_iteration: bool,
        session_config: &SessionConfig,
        initial_messages: &[Message],
        terminal_provider_error: bool,
        transcript_store: &SessionHarnessStore,
        active_compaction_strategy: ContextCompactionStrategy,
        turns_taken: u32,
        current_mode: HarnessMode,
        compaction_count: u32,
        delegation_depth: u32,
        coordinator_execution_mode: CoordinatorExecutionMode,
        required_tool_prefixes: &[String],
        task_runtime: &TaskRuntime,
        coordinator_signals: &SharedCoordinatorSignalStore,
        transition_trace: &SharedTransitionTrace,
    ) -> Result<TurnFinalization> {
        let mut events = Vec::new();
        let mut exit_chat = false;
        tracing::info!(
            session_id = %session_config.id,
            turns_taken,
            current_mode = ?current_mode,
            no_tools_called,
            terminal_provider_error,
            coordinator_execution_mode = %coordinator_execution_mode,
            "finalize_turn: start"
        );

        let retry_requested = if terminal_provider_error {
            exit_chat = true;
            tracing::info!(
                session_id = %session_config.id,
                "finalize_turn: terminal provider error, skipping no-tool repair"
            );
            false
        } else {
            let no_tool_handling = self
                .handle_no_tool_turn(
                    no_tools_called,
                    did_recovery_compact_this_iteration,
                    conversation,
                    session_config,
                    initial_messages,
                )
                .await?;
            tracing::info!(
                session_id = %session_config.id,
                retry_requested = no_tool_handling.retry_requested,
                exit_chat = no_tool_handling.exit_chat,
                event_count = no_tool_handling.events.len(),
                "finalize_turn: no-tool handling computed"
            );
            if no_tool_handling.retry_requested {
                record_transition(
                    transition_trace,
                    turns_taken,
                    current_mode,
                    TransitionKind::NoToolRepair,
                    "retry_logic_requested",
                    std::collections::BTreeMap::new(),
                )
                .await;
            }
            for event in no_tool_handling.events {
                Self::capture_event_message(
                    messages_to_add,
                    &event,
                    HistoryCapturePolicy::AllMessages,
                );
                events.push(event);
            }
            exit_chat |= no_tool_handling.exit_chat;
            no_tool_handling.retry_requested
        };

        let final_output_state = {
            let final_output_tool = self.final_output_tool.lock().await;
            snapshot_final_output_state(final_output_tool.as_ref())
        };
        let signal_summary = coordinator_signals.summarize().await;
        let _ = update_host_signal_summary(&session_config.id, signal_summary.clone()).await;
        let active_child_tasks =
            task_runtime.has_active_tasks_for_parent_session(&session_config.id);
        let completion_outcome = derive_execute_completion_outcome(
            &signal_summary,
            &final_output_state,
            required_tool_prefixes,
            active_child_tasks,
        );
        let _ =
            update_host_completion_outcome(&session_config.id, completion_outcome.clone()).await;
        let has_completion_ready = completion_outcome.completion_ready;
        let required_tools_satisfied = completion_outcome.required_tools_satisfied;
        let has_blocking_signals = completion_outcome.has_blocking_signals;
        let has_user_visible_tool_response = Self::has_user_visible_tool_response(messages_to_add);
        let has_terminal_user_visible_assistant_text =
            Self::has_user_visible_assistant_text_after_last_tool_response(messages_to_add);
        let coordinator_should_complete = if matches!(current_mode, HarnessMode::Execute) {
            completion_outcome.state.is_terminal()
        } else {
            Self::should_complete_coordinator_turn(
                current_mode,
                coordinator_execution_mode,
                has_completion_ready,
                required_tools_satisfied,
                has_terminal_user_visible_assistant_text,
                active_child_tasks,
                has_blocking_signals,
                has_user_visible_tool_response,
            )
        };
        let needs_post_tool_conversation_reply = matches!(current_mode, HarnessMode::Conversation)
            && has_user_visible_tool_response
            && !has_terminal_user_visible_assistant_text
            && !active_child_tasks
            && !has_blocking_signals;
        let needs_execute_structured_completion_follow_up =
            matches!(current_mode, HarnessMode::Execute)
                && final_output_state.tool_present
                && !completion_outcome.state.is_terminal()
                && !active_child_tasks
                && !has_blocking_signals
                && has_terminal_user_visible_assistant_text;
        if needs_post_tool_conversation_reply {
            let follow_up = Self::build_post_tool_conversation_follow_up_message();
            messages_to_add.push(follow_up.clone());
            events.push(AgentEvent::Message(follow_up));
            record_transition(
                transition_trace,
                turns_taken,
                current_mode,
                TransitionKind::CoordinatorCompletion,
                "conversation_post_tool_reply_required",
                std::collections::BTreeMap::new(),
            )
            .await;
        }
        if needs_execute_structured_completion_follow_up {
            let follow_up = Self::build_execute_structured_completion_follow_up_message();
            messages_to_add.push(follow_up.clone());
            events.push(AgentEvent::Message(follow_up));
            record_transition(
                transition_trace,
                turns_taken,
                current_mode,
                TransitionKind::CoordinatorCompletion,
                "execute_structured_completion_required",
                std::collections::BTreeMap::new(),
            )
            .await;
        }
        if coordinator_should_complete {
            let reason = if matches!(current_mode, HarnessMode::Execute) {
                match completion_outcome.state {
                    ExecuteCompletionState::Completed => {
                        "execute_host_completed_with_terminal_completion_outcome"
                    }
                    ExecuteCompletionState::Blocked => {
                        "execute_host_blocked_with_terminal_completion_outcome"
                    }
                    _ => "execute_host_terminal_outcome",
                }
            } else {
                "coordinator_signals_settled_and_terminal_output_available"
            };
            record_transition(
                transition_trace,
                turns_taken,
                current_mode,
                TransitionKind::CoordinatorCompletion,
                reason,
                std::collections::BTreeMap::new(),
            )
            .await;
            exit_chat = true;
        }
        tracing::info!(
            session_id = %session_config.id,
            coordinator_should_complete,
            has_completion_ready,
            required_tools_satisfied,
            has_blocking_signals,
            has_user_visible_tool_response,
            has_terminal_user_visible_assistant_text,
            needs_post_tool_conversation_reply,
            needs_execute_structured_completion_follow_up,
            completion_state = ?completion_outcome.state,
            worker_completed = signal_summary.worker_completed,
            validation_passed = signal_summary.validation_passed,
            fallback_requested = signal_summary.fallback_requested,
            exit_chat,
            "finalize_turn: coordinator completion evaluated"
        );

        let mut next_mode = current_mode;
        if let Some(post_turn_transition) = resolve_post_turn_transition(
            current_mode,
            no_tools_called,
            &final_output_state,
            retry_requested,
            "",
        ) {
            next_mode = post_turn_transition.transition.to;
            record_transition(
                transition_trace,
                turns_taken,
                next_mode,
                TransitionKind::ModeTransition,
                post_turn_transition.transition.reason.clone(),
                std::collections::BTreeMap::new(),
            )
            .await;
            transcript_store
                .save_mode(&session_config.id, post_turn_transition.transition.to)
                .await?;
            messages_to_add.push(post_turn_transition.notification.clone());
            events.push(AgentEvent::Message(post_turn_transition.notification));
            let _ = transcript_store
                .record_checkpoint(
                    &session_config.id,
                    crate::agents::harness::HarnessCheckpoint::mode_transition(
                        turns_taken,
                        next_mode,
                        post_turn_transition.transition.reason.clone(),
                    ),
                )
                .await;

            if let Some(follow_up) = post_turn_transition.follow_up_user_message {
                messages_to_add.push(follow_up.clone());
                events.push(AgentEvent::Message(follow_up));
            }
            exit_chat |= post_turn_transition.should_exit_chat;
        }

        tracing::info!(
            session_id = %session_config.id,
            message_count = messages_to_add.len(),
            "finalize_turn: appending messages"
        );
        transcript_store
            .append_messages(&session_config.id, messages_to_add.messages())
            .await?;
        tracing::info!(
            session_id = %session_config.id,
            "finalize_turn: append_messages complete"
        );

        let cfpm_events = self
            .refresh_cfpm_runtime_after_turn(
                session_config,
                conversation,
                messages_to_add,
                active_compaction_strategy,
            )
            .await?;
        tracing::info!(
            session_id = %session_config.id,
            cfpm_event_count = cfpm_events.len(),
            "finalize_turn: cfpm refresh complete"
        );
        for event in cfpm_events {
            events.push(event);
        }

        conversation.extend(messages_to_add.clone());
        let _ = transcript_store
            .record_checkpoint(
                &session_config.id,
                crate::agents::harness::HarnessCheckpoint::turn_end(
                    turns_taken,
                    next_mode,
                    no_tools_called,
                ),
            )
            .await;
        let _ = transcript_store
            .persist_runtime_snapshot(
                &session_config.id,
                next_mode,
                turns_taken,
                compaction_count,
                delegation_depth,
            )
            .await;
        tracing::info!(
            session_id = %session_config.id,
            next_mode = ?next_mode,
            exit_chat,
            "finalize_turn: complete"
        );

        Ok(TurnFinalization {
            events,
            next_mode,
            exit_chat,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinator_turn_completes_when_text_emitted_and_tasks_settled() {
        assert!(Agent::should_complete_coordinator_turn(
            HarnessMode::Conversation,
            CoordinatorExecutionMode::ExplicitSwarm,
            false,
            true,
            true,
            false,
            false,
            false,
        ));
    }

    #[test]
    fn coordinator_turn_does_not_complete_for_single_worker_mode() {
        assert!(!Agent::should_complete_coordinator_turn(
            HarnessMode::Conversation,
            CoordinatorExecutionMode::SingleWorker,
            true,
            true,
            true,
            false,
            false,
            false,
        ));
    }

    #[test]
    fn coordinator_turn_does_not_complete_while_child_tasks_are_running() {
        assert!(!Agent::should_complete_coordinator_turn(
            HarnessMode::Conversation,
            CoordinatorExecutionMode::AutoSwarm,
            true,
            true,
            true,
            true,
            false,
            false,
        ));
    }

    #[test]
    fn execute_host_requires_completion_ready_signal() {
        assert!(Agent::should_complete_coordinator_turn(
            HarnessMode::Execute,
            CoordinatorExecutionMode::ExplicitSwarm,
            true,
            true,
            false,
            false,
            false,
            true,
        ));
        assert!(!Agent::should_complete_coordinator_turn(
            HarnessMode::Execute,
            CoordinatorExecutionMode::ExplicitSwarm,
            false,
            true,
            true,
            false,
            false,
            true,
        ));
        assert!(!Agent::should_complete_coordinator_turn(
            HarnessMode::Execute,
            CoordinatorExecutionMode::ExplicitSwarm,
            true,
            false,
            true,
            false,
            false,
            true,
        ));
    }

    #[test]
    fn conversation_turn_with_tool_response_requires_post_tool_assistant_reply() {
        assert!(!Agent::should_complete_coordinator_turn(
            HarnessMode::Conversation,
            CoordinatorExecutionMode::ExplicitSwarm,
            true,
            true,
            false,
            false,
            false,
            true,
        ));
    }

    #[test]
    fn detects_assistant_reply_only_after_last_tool_response() {
        let mut conversation = Conversation::default();
        conversation.push(Message::assistant().with_text("Starting swarm now."));
        conversation.push(Message::user().with_tool_response(
            "tool-1",
            Ok(rmcp::model::CallToolResult {
                content: vec![rmcp::model::Content::text("tool completed")],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        ));
        assert!(Agent::has_user_visible_tool_response(&conversation));
        assert!(!Agent::has_user_visible_assistant_text_after_last_tool_response(&conversation));

        conversation.push(Message::assistant().with_text("Swarm completed successfully."));
        assert!(Agent::has_user_visible_assistant_text_after_last_tool_response(&conversation));
    }

    #[test]
    fn post_tool_follow_up_message_is_agent_only() {
        let message = Agent::build_post_tool_conversation_follow_up_message();
        assert_eq!(message.role, rmcp::model::Role::User);
        assert_eq!(message.metadata, MessageMetadata::agent_only());
        assert!(message
            .as_concat_text()
            .contains("Tool execution for this turn has completed"));
    }

    #[test]
    fn execute_structured_completion_follow_up_message_is_agent_only() {
        let message = Agent::build_execute_structured_completion_follow_up_message();
        assert_eq!(message.role, rmcp::model::Role::User);
        assert_eq!(message.metadata, MessageMetadata::agent_only());
        assert!(message
            .as_concat_text()
            .contains("Call the `final_output` tool now"));
    }
}
