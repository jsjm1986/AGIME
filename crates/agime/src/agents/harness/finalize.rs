use crate::agents::agent::{
    Agent, AgentEvent, HistoryCapturePolicy, NoToolTurnHandling, TurnFinalization,
};
use crate::agents::types::SessionConfig;
use crate::conversation::{
    message::{Message, MessageContent, MessageMetadata},
    Conversation,
};
use crate::utils::{normalize_delegation_summary_text, safe_truncate};
use anyhow::Result;

use super::{
    derive_execute_completion_outcome, has_active_persisted_tasks,
    no_tool_turn_action_with_final_output, record_transition, resolve_post_turn_transition,
    snapshot_final_output_state, update_host_completion_outcome, update_host_signal_summary,
    CompletionSurfacePolicy, CoordinatorExecutionMode, ExecuteCompletionState,
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
                            if super::completion::assistant_text_counts_as_terminal_reply(
                                &text.text
                            )
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
                            MessageContent::Text(text)
                                if super::completion::assistant_text_counts_as_terminal_reply(
                                    &text.text
                                )
                        )
                    })
            })
    }

    fn latest_user_visible_tool_response_text(messages: &Conversation) -> Option<String> {
        messages
            .messages()
            .iter()
            .rev()
            .filter(|message| message.metadata.user_visible)
            .find_map(|message| {
                message
                    .content
                    .iter()
                    .rev()
                    .find_map(MessageContent::as_tool_response_text)
            })
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    }

    fn build_tool_response_summary_fallback(messages: &Conversation) -> Option<String> {
        let raw = Self::latest_user_visible_tool_response_text(messages)?;
        let normalized = normalize_delegation_summary_text(&raw);
        if normalized.is_empty() {
            return None;
        }
        Some(format!(
            "Tool execution completed. Result:\n{}",
            safe_truncate(&normalized, 500)
        ))
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

    fn should_accept_bounded_child_terminal_assistant_text(
        no_tools_called: bool,
        delegation_depth: u32,
        current_mode: HarnessMode,
        messages_to_add: &Conversation,
    ) -> bool {
        no_tools_called
            && delegation_depth > 0
            && matches!(current_mode, HarnessMode::Conversation)
            && Self::has_user_visible_assistant_text(messages_to_add)
    }

    fn should_accept_bounded_child_terminal_tool_result(
        turns_taken: u32,
        delegation_depth: u32,
        completion_surface_policy: CompletionSurfacePolicy,
        has_user_visible_tool_response: bool,
        active_child_tasks: bool,
        has_blocking_signals: bool,
    ) -> bool {
        turns_taken > 1
            && delegation_depth > 0
            && matches!(
                completion_surface_policy,
                CompletionSurfacePolicy::Conversation
            )
            && has_user_visible_tool_response
            && !active_child_tasks
            && !has_blocking_signals
    }

    fn build_delegation_summary_fallback(
        signal_summary: &super::signals::CoordinatorSignalSummary,
    ) -> Option<String> {
        signal_summary
            .latest_completion_summary
            .as_deref()
            .map(str::trim)
            .filter(|value| super::completion::assistant_text_counts_as_terminal_reply(value))
            .map(ToString::to_string)
            .or_else(|| {
                signal_summary.worker_outcomes.iter().find_map(|outcome| {
                    let summary = outcome.summary.trim();
                    super::completion::assistant_text_counts_as_terminal_reply(summary).then(|| {
                        if signal_summary.worker_outcomes.len() == 1 {
                            format!("Worker completed. Summary:\n{}", summary)
                        } else {
                            format!("Delegation completed. Summary:\n{}", summary)
                        }
                    })
                })
            })
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

        let _ = has_completion_ready;
        let _ = required_tools_satisfied;
        let _ = has_user_visible_tool_response;
        has_user_visible_assistant_text && !has_blocking_signals
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
        turns_taken: u32,
        current_mode: HarnessMode,
        runtime_compaction_count: u32,
        completion_surface_policy: CompletionSurfacePolicy,
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
        } else if Self::should_accept_bounded_child_terminal_assistant_text(
            no_tools_called,
            delegation_depth,
            current_mode,
            messages_to_add,
        ) {
            exit_chat = true;
            tracing::info!(
                session_id = %session_config.id,
                "finalize_turn: accepting bounded child terminal assistant text without retry"
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
                    TransitionKind::PostTurnAdjudication,
                    "no_tool_repair_transition",
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
        let active_child_tasks = task_runtime
            .has_active_tasks_for_parent_session(&session_config.id)
            || has_active_persisted_tasks(&session_config.id)
                .await
                .unwrap_or(false);
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
        let mut visible_conversation = conversation.clone();
        visible_conversation.extend(messages_to_add.messages().clone());
        let has_user_visible_tool_response =
            Self::has_user_visible_tool_response(&visible_conversation);
        let mut has_terminal_user_visible_assistant_text =
            Self::has_user_visible_assistant_text_after_last_tool_response(messages_to_add);
        if !has_terminal_user_visible_assistant_text {
            if let Some(summary) = Self::build_delegation_summary_fallback(&signal_summary) {
                let assistant_summary = Message::assistant().with_text(summary);
                messages_to_add.push(assistant_summary.clone());
                events.push(AgentEvent::Message(assistant_summary.clone()));
                visible_conversation.push(assistant_summary);
                has_terminal_user_visible_assistant_text = true;
            }
        }
        if no_tools_called
            && !has_terminal_user_visible_assistant_text
            && matches!(
                completion_surface_policy,
                CompletionSurfacePolicy::Conversation
            )
            && has_user_visible_tool_response
            && !active_child_tasks
            && !has_blocking_signals
        {
            if let Some(summary) = Self::build_tool_response_summary_fallback(&visible_conversation)
            {
                let assistant_summary = Message::assistant().with_text(summary);
                messages_to_add.push(assistant_summary.clone());
                events.push(AgentEvent::Message(assistant_summary.clone()));
                visible_conversation.push(assistant_summary);
                has_terminal_user_visible_assistant_text = true;
            }
        }
        let bounded_child_tool_result_is_terminal =
            Self::should_accept_bounded_child_terminal_tool_result(
                turns_taken,
                delegation_depth,
                completion_surface_policy,
                has_user_visible_tool_response,
                active_child_tasks,
                has_blocking_signals,
            );
        let coordinator_should_complete = match completion_surface_policy {
            CompletionSurfacePolicy::Execute | CompletionSurfacePolicy::SystemDocumentAnalysis => {
                if matches!(current_mode, HarnessMode::Execute) {
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
                }
            }
            CompletionSurfacePolicy::Conversation => {
                let _ = current_mode;
                let _ = coordinator_execution_mode;
                let _ = has_completion_ready;
                let _ = required_tools_satisfied;
                let _ = has_user_visible_tool_response;
                (has_terminal_user_visible_assistant_text || bounded_child_tool_result_is_terminal)
                    && !active_child_tasks
                    && !has_blocking_signals
            }
        };
        let needs_post_tool_conversation_reply = matches!(
            completion_surface_policy,
            CompletionSurfacePolicy::Conversation
        ) && has_user_visible_tool_response
            && !has_terminal_user_visible_assistant_text
            && !bounded_child_tool_result_is_terminal
            && !active_child_tasks
            && !has_blocking_signals;
        let needs_execute_structured_completion_follow_up =
            matches!(completion_surface_policy, CompletionSurfacePolicy::Execute)
                && matches!(current_mode, HarnessMode::Execute)
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
                TransitionKind::PostTurnAdjudication,
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
                TransitionKind::PostTurnAdjudication,
                "execute_structured_completion_required",
                std::collections::BTreeMap::new(),
            )
            .await;
        }
        if coordinator_should_complete {
            record_transition(
                transition_trace,
                turns_taken,
                current_mode,
                TransitionKind::PostTurnAdjudication,
                if matches!(completion_outcome.state, ExecuteCompletionState::Blocked) {
                    "completion_blocked"
                } else {
                    "completion_completed"
                },
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
        let allow_post_turn_transition = !coordinator_should_complete && !terminal_provider_error;
        if let Some(post_turn_transition) = allow_post_turn_transition
            .then(|| {
                resolve_post_turn_transition(
                    current_mode,
                    no_tools_called,
                    &final_output_state,
                    retry_requested,
                    "",
                )
            })
            .flatten()
        {
            next_mode = post_turn_transition.transition.to;
            let mut metadata = std::collections::BTreeMap::new();
            metadata.insert(
                "transition_reason".to_string(),
                post_turn_transition.transition.reason.clone(),
            );
            metadata.insert(
                "next_mode".to_string(),
                post_turn_transition.transition.to.to_string(),
            );
            record_transition(
                transition_trace,
                turns_taken,
                next_mode,
                TransitionKind::PostTurnAdjudication,
                "mode_transition",
                metadata,
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
                runtime_compaction_count,
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
    fn conversation_turn_without_terminal_assistant_reply_does_not_complete_from_runtime_evidence()
    {
        assert!(!Agent::should_complete_coordinator_turn(
            HarnessMode::Conversation,
            CoordinatorExecutionMode::ExplicitSwarm,
            true,
            true,
            false,
            false,
            false,
            false,
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
    fn runtime_only_summary_after_tool_response_is_not_terminal_reply() {
        let mut conversation = Conversation::default();
        conversation.push(Message::assistant().with_text("Checking documents."));
        conversation.push(Message::user().with_tool_response(
            "tool-1",
            Ok(rmcp::model::CallToolResult {
                content: vec![rmcp::model::Content::text("tool completed")],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        ));
        conversation.push(
            Message::assistant().with_text("tool `document_tools__document_inventory` completed"),
        );

        assert!(!Agent::has_user_visible_assistant_text_after_last_tool_response(&conversation));
    }

    #[test]
    fn document_workspace_handoff_is_not_considered_user_visible_terminal_text() {
        let mut conversation = Conversation::default();
        conversation.push(Message::assistant().with_text(
            "Document access established and the file was materialized into the workspace. Use developer shell, MCP, or another local tool to read the file content from the workspace path.",
        ));

        assert!(!Agent::has_user_visible_assistant_text(&conversation));
    }

    #[test]
    fn delegation_summary_fallback_rejects_document_workspace_handoff() {
        let signal_summary = crate::agents::harness::signals::CoordinatorSignalSummary {
            latest_completion_summary: Some(
                "Document access established and the file was materialized into the workspace. Use developer shell, MCP, or another local tool to read the file content from the workspace path."
                    .to_string(),
            ),
            ..Default::default()
        };

        assert!(Agent::build_delegation_summary_fallback(&signal_summary).is_none());
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

    #[test]
    fn bounded_child_terminal_assistant_text_short_circuits_no_tool_retry() {
        let mut messages = Conversation::default();
        messages.push(
            Message::assistant().with_text("Scope: inspect README\nResult: README.md is missing."),
        );
        assert!(Agent::should_accept_bounded_child_terminal_assistant_text(
            true,
            1,
            HarnessMode::Conversation,
            &messages,
        ));
        assert!(!Agent::should_accept_bounded_child_terminal_assistant_text(
            true,
            0,
            HarnessMode::Conversation,
            &messages,
        ));
        assert!(!Agent::should_accept_bounded_child_terminal_assistant_text(
            false,
            1,
            HarnessMode::Conversation,
            &messages,
        ));
    }

    #[test]
    fn bounded_child_tool_result_can_complete_without_extra_assistant_turn() {
        assert!(Agent::should_accept_bounded_child_terminal_tool_result(
            2,
            1,
            CompletionSurfacePolicy::Conversation,
            true,
            false,
            false,
        ));
        assert!(!Agent::should_accept_bounded_child_terminal_tool_result(
            2,
            0,
            CompletionSurfacePolicy::Conversation,
            true,
            false,
            false,
        ));
        assert!(!Agent::should_accept_bounded_child_terminal_tool_result(
            2,
            1,
            CompletionSurfacePolicy::Conversation,
            true,
            true,
            false,
        ));
        assert!(!Agent::should_accept_bounded_child_terminal_tool_result(
            1,
            1,
            CompletionSurfacePolicy::Conversation,
            true,
            false,
            false,
        ));
    }

    #[test]
    fn builds_tool_response_summary_fallback_from_latest_user_visible_tool_result() {
        let mut conversation = Conversation::default();
        conversation.push(Message::assistant().with_text("Checking health."));
        conversation.push(Message::user().with_tool_response(
            "tool-1",
            Ok(rmcp::model::CallToolResult {
                content: vec![rmcp::model::Content::text(
                    "{\"status\":\"healthy\",\"database_connected\":true,\"version\":\"2.8.0\"}",
                )],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        ));

        let summary =
            Agent::build_tool_response_summary_fallback(&conversation).expect("summary fallback");
        assert!(summary.starts_with("Tool execution completed. Result:"));
        assert!(summary.contains("2.8.0"));
    }

    #[test]
    fn preexisting_assistant_notice_does_not_count_as_terminal_reply_for_new_turn() {
        let mut existing = Conversation::default();
        existing.push(Message::assistant().with_text("已初始化应用。"));
        assert!(Agent::has_user_visible_assistant_text(&existing));

        let new_turn = Conversation::default();
        assert!(!Agent::has_user_visible_assistant_text_after_last_tool_response(&new_turn));
    }
}
