use crate::conversation::message::{Message, SystemNotificationType};

use super::{state::HarnessMode, tools::FinalOutputState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeTransition {
    pub from: HarnessMode,
    pub to: HarnessMode,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessModeCommand {
    pub mode: HarnessMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TransitionAction {
    pub follow_up_user_message: Option<String>,
    pub should_exit_chat: bool,
}

#[derive(Debug, Clone)]
pub struct PostTurnTransition {
    pub transition: ModeTransition,
    pub notification: Message,
    pub follow_up_user_message: Option<Message>,
    pub should_exit_chat: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoToolTurnAction {
    Noop,
    ContinueWithUserPrompt(String),
    ExitWithAssistantMessage(String),
    RunRetryLogic,
}

pub fn parse_harness_mode_command(input: &str) -> Option<HarnessModeCommand> {
    let trimmed = input.trim();
    let mode = match trimmed {
        "/plan" => HarnessMode::Plan,
        "/execute" => HarnessMode::Execute,
        "/repair" => HarnessMode::Repair,
        "/blocked" => HarnessMode::Blocked,
        "/complete" => HarnessMode::Complete,
        "/conversation" => HarnessMode::Conversation,
        _ => return None,
    };
    Some(HarnessModeCommand { mode })
}

pub fn mode_system_prompt(mode: HarnessMode) -> Option<&'static str> {
    match mode {
        HarnessMode::Conversation => None,
        HarnessMode::Plan => Some(
            "You are in explicit plan mode. Use read-only investigation and planning only. Do not perform mutating tool calls or delegate to subagents. Produce a concrete implementation plan or analysis.",
        ),
        HarnessMode::Execute => Some(
            "You are in execute mode. Make concrete tool-backed progress and prefer real artifact changes over broad discussion.",
        ),
        HarnessMode::Repair => Some(
            "You are in repair mode. Focus on fixing the previous execution gap, producing a concrete blocker, or restoring the task to a valid state.",
        ),
        HarnessMode::Blocked => Some(
            "You are in blocked mode. Do not continue broad execution. State the concrete blocker, what evidence you observed, and what user or environment action is required.",
        ),
        HarnessMode::Complete => Some(
            "You are in complete mode. If a final_output tool is available, call it now with the final structured answer. Do not start new tool chains.",
        ),
    }
}

pub fn build_mode_transition_notification(transition: &ModeTransition) -> String {
    format!(
        "Harness mode changed: {} -> {} ({})",
        transition.from, transition.to, transition.reason
    )
}

pub fn next_mode_after_turn(
    current_mode: HarnessMode,
    no_tools_called: bool,
    final_output_collected: bool,
    retry_requested: bool,
) -> Option<ModeTransition> {
    if final_output_collected && current_mode != HarnessMode::Complete {
        return Some(ModeTransition {
            from: current_mode,
            to: HarnessMode::Complete,
            reason: "final_output_collected".to_string(),
        });
    }

    if retry_requested {
        return None;
    }

    match current_mode {
        HarnessMode::Execute if no_tools_called => Some(ModeTransition {
            from: HarnessMode::Execute,
            to: HarnessMode::Repair,
            reason: "no_tool_backed_progress".to_string(),
        }),
        HarnessMode::Repair if no_tools_called => Some(ModeTransition {
            from: HarnessMode::Repair,
            to: HarnessMode::Blocked,
            reason: "repair_exhausted_without_progress".to_string(),
        }),
        _ => None,
    }
}

pub fn next_mode_after_turn_with_final_output(
    current_mode: HarnessMode,
    no_tools_called: bool,
    final_output: &FinalOutputState,
    retry_requested: bool,
) -> Option<ModeTransition> {
    next_mode_after_turn(
        current_mode,
        no_tools_called,
        final_output.is_collected(),
        retry_requested,
    )
}

pub fn transition_action(mode: HarnessMode, retry_requested: bool) -> TransitionAction {
    match mode {
        HarnessMode::Repair if !retry_requested => TransitionAction {
            follow_up_user_message: Some(
                "No tool-backed progress was made in execute mode. Repair the attempt now: use tools to make concrete progress, or state the blocker if execution cannot continue."
                    .to_string(),
            ),
            should_exit_chat: false,
        },
        HarnessMode::Blocked => TransitionAction {
            follow_up_user_message: Some(
                "State the concrete blocker, the evidence you observed, and what user or environment action is required before execution can continue."
                    .to_string(),
            ),
            should_exit_chat: false,
        },
        HarnessMode::Complete => TransitionAction {
            follow_up_user_message: None,
            should_exit_chat: true,
        },
        _ => TransitionAction::default(),
    }
}

pub fn no_tool_turn_action(
    no_tools_called: bool,
    did_recovery_compact_this_iteration: bool,
    final_output: Option<&str>,
    _final_output_tool_present: bool,
    _final_output_continuation_message: &str,
) -> NoToolTurnAction {
    if !no_tools_called {
        return NoToolTurnAction::Noop;
    }

    if let Some(final_output) = final_output {
        return NoToolTurnAction::ExitWithAssistantMessage(final_output.to_string());
    }

    if did_recovery_compact_this_iteration {
        return NoToolTurnAction::Noop;
    }

    NoToolTurnAction::RunRetryLogic
}

pub fn no_tool_turn_action_with_final_output(
    no_tools_called: bool,
    did_recovery_compact_this_iteration: bool,
    final_output: &FinalOutputState,
    final_output_continuation_message: &str,
) -> NoToolTurnAction {
    no_tool_turn_action(
        no_tools_called,
        did_recovery_compact_this_iteration,
        final_output.collected_output(),
        final_output.tool_present,
        final_output_continuation_message,
    )
}

pub fn resolve_post_turn_transition(
    current_mode: HarnessMode,
    no_tools_called: bool,
    final_output: &FinalOutputState,
    retry_requested: bool,
    _final_output_continuation_message: &str,
) -> Option<PostTurnTransition> {
    let transition = next_mode_after_turn_with_final_output(
        current_mode,
        no_tools_called,
        final_output,
        retry_requested,
    )?;
    let action = transition_action(transition.to, retry_requested);

    Some(PostTurnTransition {
        notification: Message::assistant().with_system_notification(
            SystemNotificationType::InlineMessage,
            build_mode_transition_notification(&transition),
        ),
        follow_up_user_message: action
            .follow_up_user_message
            .map(|message| Message::user().with_text(message)),
        should_exit_chat: action.should_exit_chat,
        transition,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plan_mode_command() {
        assert_eq!(
            parse_harness_mode_command("/plan"),
            Some(HarnessModeCommand {
                mode: HarnessMode::Plan,
            })
        );
        assert!(parse_harness_mode_command("/plan feature").is_none());
    }

    #[test]
    fn transitions_execute_to_repair_on_empty_turn() {
        let transition = next_mode_after_turn(HarnessMode::Execute, true, false, false)
            .expect("transition expected");
        assert_eq!(transition.to, HarnessMode::Repair);
    }

    #[test]
    fn blocked_mode_transition_requests_blocker() {
        let action = transition_action(HarnessMode::Blocked, false);
        assert!(action.follow_up_user_message.is_some());
        assert!(!action.should_exit_chat);
    }

    #[test]
    fn no_tool_turn_prefers_final_output_exit() {
        let action = no_tool_turn_action(true, false, Some("{\"ok\":true}"), true, "continue");
        assert_eq!(
            action,
            NoToolTurnAction::ExitWithAssistantMessage("{\"ok\":true}".to_string())
        );
    }

    #[test]
    fn no_tool_turn_requests_final_output_when_tool_present() {
        let action = no_tool_turn_action(true, false, None, true, "continue now");
        assert_eq!(action, NoToolTurnAction::RunRetryLogic);
    }

    #[test]
    fn next_mode_after_turn_uses_final_output_state() {
        let transition = next_mode_after_turn_with_final_output(
            HarnessMode::Execute,
            false,
            &FinalOutputState {
                tool_present: true,
                collected_output: Some("{\"ok\":true}".to_string()),
            },
            false,
        )
        .expect("transition expected");
        assert_eq!(transition.to, HarnessMode::Complete);
    }

    #[test]
    fn final_output_tool_prevents_execute_to_repair_transition() {
        let transition = next_mode_after_turn_with_final_output(
            HarnessMode::Execute,
            true,
            &FinalOutputState {
                tool_present: true,
                collected_output: None,
            },
            false,
        );
        assert_eq!(
            transition.expect("transition expected").to,
            HarnessMode::Repair
        );
    }

    #[test]
    fn final_output_tool_prevents_repair_to_blocked_transition() {
        let transition = next_mode_after_turn_with_final_output(
            HarnessMode::Repair,
            true,
            &FinalOutputState {
                tool_present: true,
                collected_output: None,
            },
            false,
        );
        assert_eq!(
            transition.expect("transition expected").to,
            HarnessMode::Blocked
        );
    }

    #[test]
    fn resolve_post_turn_transition_builds_notification_and_follow_up() {
        let transition = resolve_post_turn_transition(
            HarnessMode::Execute,
            true,
            &FinalOutputState::default(),
            false,
            "continue",
        )
        .expect("transition expected");
        assert_eq!(transition.transition.to, HarnessMode::Repair);
        assert!(transition.follow_up_user_message.is_some());
    }

    #[test]
    fn execute_mode_without_final_output_no_longer_transitions_to_complete() {
        let transition = resolve_post_turn_transition(
            HarnessMode::Execute,
            false,
            &FinalOutputState {
                tool_present: true,
                collected_output: None,
            },
            false,
            "call final_output now",
        );
        assert!(transition.is_none());
    }
}
