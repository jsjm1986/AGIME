use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use rmcp::model::Role;

use crate::agents::agent::ToolTransportRequestEvent;
use crate::agents::AgentEvent;
use crate::conversation::message::{
    ActionRequiredData, Message, MessageContent, SystemNotificationType,
};

use super::completion::{
    CompletionSurfacePolicy, ExecuteCompletionOutcome, ExecutionHostCompletionReport,
};
use super::control_types::{
    CompletionControlEvent, HarnessControlEnvelope, HarnessControlMessage, PermissionControlEvent,
    RuntimeControlEvent, SessionControlEvent, ToolControlEvent,
};
use super::state::HarnessMode;
use super::task_runtime::{TaskKind, TaskSnapshot, WorkerAttemptIdentity};
use super::tools::{ToolInvocationSurface, ToolTransportKind};

static SESSION_CONTROL_SEQUENCERS: Lazy<DashMap<String, Arc<HarnessControlSequencer>>> =
    Lazy::new(DashMap::new);

pub fn control_timestamp_ms() -> i64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    duration.as_millis().min(i64::MAX as u128) as i64
}

pub struct HarnessControlSequencer {
    session_id: String,
    runtime_session_id: String,
    next_sequence: AtomicU64,
}

impl HarnessControlSequencer {
    pub fn new(session_id: impl Into<String>, runtime_session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            runtime_session_id: runtime_session_id.into(),
            next_sequence: AtomicU64::new(1),
        }
    }

    pub fn next(&self, payload: HarnessControlMessage) -> HarnessControlEnvelope {
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        HarnessControlEnvelope::new(
            self.session_id.clone(),
            self.runtime_session_id.clone(),
            sequence,
            control_timestamp_ms(),
            payload,
        )
    }
}

pub fn register_harness_control_sequencer(
    runtime_session_id: &str,
    sequencer: Arc<HarnessControlSequencer>,
) {
    SESSION_CONTROL_SEQUENCERS.insert(runtime_session_id.to_string(), sequencer);
}

pub fn control_sequencer_for_session(
    runtime_session_id: &str,
) -> Option<Arc<HarnessControlSequencer>> {
    SESSION_CONTROL_SEQUENCERS
        .get(runtime_session_id)
        .map(|value| value.clone())
}

pub fn unregister_harness_control_sequencer(runtime_session_id: &str) {
    SESSION_CONTROL_SEQUENCERS.remove(runtime_session_id);
}

#[async_trait]
pub trait HarnessControlSink: Send + Sync {
    async fn handle(&self, envelope: &HarnessControlEnvelope) -> Result<()>;
}

#[derive(Clone, Default)]
pub struct NoopHarnessControlSink;

#[async_trait]
impl HarnessControlSink for NoopHarnessControlSink {
    async fn handle(&self, _envelope: &HarnessControlEnvelope) -> Result<()> {
        Ok(())
    }
}

pub fn completion_surface_label(policy: CompletionSurfacePolicy) -> &'static str {
    match policy {
        CompletionSurfacePolicy::Conversation => "conversation",
        CompletionSurfacePolicy::Execute => "execute",
        CompletionSurfacePolicy::SystemDocumentAnalysis => "system_document_analysis",
    }
}

pub fn tool_transport_label(transport: ToolTransportKind) -> &'static str {
    match transport {
        ToolTransportKind::ExternalFrontend => "external_frontend",
        ToolTransportKind::ServerLocal => "server_local",
        ToolTransportKind::WorkerLocal => "worker_local",
    }
}

pub fn tool_invocation_surface_label(surface: ToolInvocationSurface) -> &'static str {
    match surface {
        ToolInvocationSurface::Conversation => "conversation",
        ToolInvocationSurface::ExecuteHost => "execute_host",
        ToolInvocationSurface::Worker => "worker",
    }
}

pub fn build_session_started_message(
    mode: HarnessMode,
    surface: CompletionSurfacePolicy,
) -> HarnessControlMessage {
    HarnessControlMessage::Session(SessionControlEvent::Started {
        mode: mode.to_string(),
        surface: completion_surface_label(surface).to_string(),
    })
}

pub fn build_session_finished_message(final_status: impl Into<String>) -> HarnessControlMessage {
    HarnessControlMessage::Session(SessionControlEvent::Finished {
        final_status: final_status.into(),
    })
}

pub fn build_completion_control_messages(
    report: &ExecutionHostCompletionReport,
    outcome: Option<&ExecuteCompletionOutcome>,
) -> Vec<HarnessControlMessage> {
    let mut messages = vec![HarnessControlMessage::Completion(
        CompletionControlEvent::StructuredPublished {
            status: report.status.clone(),
            summary: Some(report.summary.clone()),
            blocking_reason: report.blocking_reason.clone(),
            metadata: serde_json::to_value(report).ok(),
        },
    )];

    if let Some(outcome) = outcome {
        messages.push(HarnessControlMessage::Completion(
            CompletionControlEvent::OutcomeObserved {
                status: outcome.status.clone(),
                summary: outcome.summary.clone(),
                blocking_reason: outcome.blocking_reason.clone(),
                reason_code: Some(format!("{:?}", outcome.state).to_ascii_lowercase()),
            },
        ));
    }

    messages
}

pub fn build_tool_started_control_message(
    request_id: impl Into<String>,
    tool_name: impl Into<String>,
) -> HarnessControlMessage {
    HarnessControlMessage::Tool(ToolControlEvent::Started {
        request_id: request_id.into(),
        tool_name: tool_name.into(),
    })
}

pub fn build_tool_finished_control_message(
    request_id: impl Into<String>,
    tool_name: impl Into<String>,
    success: bool,
    summary: Option<String>,
    duration_ms: Option<u64>,
) -> HarnessControlMessage {
    HarnessControlMessage::Tool(ToolControlEvent::Finished {
        request_id: request_id.into(),
        tool_name: tool_name.into(),
        success,
        summary,
        duration_ms,
    })
}

fn task_kind_label(kind: TaskKind) -> String {
    match kind {
        TaskKind::Subagent => "subagent".to_string(),
        TaskKind::SwarmWorker => "swarm_worker".to_string(),
        TaskKind::ValidationWorker => "validation_worker".to_string(),
    }
}

pub fn build_worker_started_control_message(snapshot: &TaskSnapshot) -> HarnessControlMessage {
    let attempt_identity = WorkerAttemptIdentity::from_metadata(&snapshot.metadata);
    HarnessControlMessage::Worker(super::control_types::WorkerControlEvent::Started {
        task_id: snapshot.task_id.clone(),
        kind: task_kind_label(snapshot.kind),
        target: snapshot.target_artifacts.first().cloned(),
        logical_worker_id: attempt_identity
            .as_ref()
            .map(|value| value.logical_worker_id.clone()),
        attempt_id: attempt_identity
            .as_ref()
            .map(|value| value.attempt_id.clone()),
        attempt_index: attempt_identity.as_ref().map(|value| value.attempt_index),
        previous_task_id: attempt_identity
            .as_ref()
            .and_then(|value| value.previous_task_id.clone()),
    })
}

pub fn build_worker_progress_control_message(
    task_id: impl Into<String>,
    message: impl Into<String>,
    percent: Option<u8>,
) -> HarnessControlMessage {
    HarnessControlMessage::Worker(super::control_types::WorkerControlEvent::Progress {
        task_id: task_id.into(),
        message: message.into(),
        percent,
    })
}

pub fn build_worker_followup_requested_control_message(
    task_id: impl Into<String>,
    kind: impl Into<String>,
    reason: impl Into<String>,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    HarnessControlMessage::Worker(
        super::control_types::WorkerControlEvent::FollowupRequested {
            task_id: task_id.into(),
            kind: kind.into(),
            reason: reason.into(),
            logical_worker_id: attempt_identity.map(|value| value.logical_worker_id.clone()),
            attempt_id: attempt_identity.map(|value| value.attempt_id.clone()),
            attempt_index: attempt_identity.map(|value| value.attempt_index),
            previous_task_id: attempt_identity.and_then(|value| value.previous_task_id.clone()),
        },
    )
}

pub fn build_worker_idle_control_message(
    task_id: impl Into<String>,
    message: impl Into<String>,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    HarnessControlMessage::Worker(super::control_types::WorkerControlEvent::Idle {
        task_id: task_id.into(),
        message: message.into(),
        logical_worker_id: attempt_identity.map(|value| value.logical_worker_id.clone()),
        attempt_id: attempt_identity.map(|value| value.attempt_id.clone()),
    })
}

pub fn build_permission_requested_control_message(
    request_id: impl Into<String>,
    tool_name: impl Into<String>,
    worker_name: Option<String>,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    HarnessControlMessage::Permission(PermissionControlEvent::Requested {
        request_id: request_id.into(),
        tool_name: tool_name.into(),
        worker_name,
        logical_worker_id: attempt_identity.map(|value| value.logical_worker_id.clone()),
        attempt_id: attempt_identity.map(|value| value.attempt_id.clone()),
    })
}

pub fn build_permission_resolved_control_message(
    request_id: impl Into<String>,
    tool_name: impl Into<String>,
    decision: impl Into<String>,
    source: Option<String>,
    feedback: Option<String>,
    worker_name: Option<String>,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    HarnessControlMessage::Permission(PermissionControlEvent::Resolved {
        request_id: request_id.into(),
        tool_name: tool_name.into(),
        decision: decision.into(),
        source,
        feedback,
        worker_name,
        logical_worker_id: attempt_identity.map(|value| value.logical_worker_id.clone()),
        attempt_id: attempt_identity.map(|value| value.attempt_id.clone()),
    })
}

pub fn build_permission_timed_out_control_message(
    request_id: impl Into<String>,
    tool_name: impl Into<String>,
    timeout_ms: u64,
    worker_name: Option<String>,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    HarnessControlMessage::Permission(PermissionControlEvent::TimedOut {
        request_id: request_id.into(),
        tool_name: tool_name.into(),
        timeout_ms,
        worker_name,
        logical_worker_id: attempt_identity.map(|value| value.logical_worker_id.clone()),
        attempt_id: attempt_identity.map(|value| value.attempt_id.clone()),
    })
}

pub fn build_worker_finished_control_message(
    task_id: impl Into<String>,
    kind: TaskKind,
    status: impl Into<String>,
    summary: impl Into<String>,
    produced_delta: bool,
    attempt_identity: Option<&WorkerAttemptIdentity>,
) -> HarnessControlMessage {
    HarnessControlMessage::Worker(super::control_types::WorkerControlEvent::Finished {
        task_id: task_id.into(),
        kind: task_kind_label(kind),
        status: status.into(),
        summary: summary.into(),
        produced_delta: Some(produced_delta),
        logical_worker_id: attempt_identity.map(|value| value.logical_worker_id.clone()),
        attempt_id: attempt_identity.map(|value| value.attempt_id.clone()),
        attempt_index: attempt_identity.map(|value| value.attempt_index),
        previous_task_id: attempt_identity.and_then(|value| value.previous_task_id.clone()),
    })
}

pub fn control_messages_for_agent_event(event: &AgentEvent) -> Vec<HarnessControlMessage> {
    match event {
        AgentEvent::ToolTransportRequest(request) => {
            vec![tool_transport_request_to_control_message(request)]
        }
        AgentEvent::Message(message) => control_messages_for_message(message),
        AgentEvent::HistoryReplaced(_) => vec![HarnessControlMessage::Runtime(
            RuntimeControlEvent::CompactionObserved {
                strategy: Some("history_replaced".to_string()),
                reason: Some("agent_event_history_replaced".to_string()),
                before_tokens: None,
                after_tokens: None,
                phase: None,
            },
        )],
        AgentEvent::ModelChange { model, mode } => vec![HarnessControlMessage::Runtime(
            RuntimeControlEvent::Notification {
                level: "info".to_string(),
                code: Some("model_change".to_string()),
                message: format!("model changed to '{}' in mode '{}'", model, mode),
            },
        )],
        AgentEvent::McpNotification((server_name, notification)) => {
            vec![HarnessControlMessage::Runtime(
                RuntimeControlEvent::Notification {
                    level: "info".to_string(),
                    code: Some("mcp_notification".to_string()),
                    message: format!("mcp notification from {}: {:?}", server_name, notification),
                },
            )]
        }
    }
}

fn tool_transport_request_to_control_message(
    event: &ToolTransportRequestEvent,
) -> HarnessControlMessage {
    let tool_name = event
        .request
        .tool_call
        .as_ref()
        .ok()
        .map(|call| call.name.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    HarnessControlMessage::Tool(ToolControlEvent::TransportRequested {
        request_id: event.request.id.clone(),
        tool_name,
        transport: tool_transport_label(event.transport).to_string(),
        surface: tool_invocation_surface_label(event.surface).to_string(),
    })
}

fn control_messages_for_message(message: &Message) -> Vec<HarnessControlMessage> {
    let mut events = Vec::new();

    for content in &message.content {
        match content {
            MessageContent::Text(text)
                if message.role == Role::Assistant && message.metadata.user_visible =>
            {
                events.push(HarnessControlMessage::Runtime(
                    RuntimeControlEvent::Notification {
                        level: "assistant_text".to_string(),
                        code: Some("assistant_text".to_string()),
                        message: text.text.clone(),
                    },
                ));
            }
            MessageContent::Thinking(thinking) if message.metadata.user_visible => {
                events.push(HarnessControlMessage::Session(
                    SessionControlEvent::StateChanged {
                        state: "thinking".to_string(),
                        reason: Some(thinking.thinking.clone()),
                    },
                ));
            }
            MessageContent::SystemNotification(notification) if message.metadata.user_visible => {
                events.push(system_notification_to_control_message(
                    notification.notification_type.clone(),
                    &notification.msg,
                ));
            }
            MessageContent::ActionRequired(action_required) => {
                if let Some(event) = action_required_to_control_message(&action_required.data) {
                    events.push(event);
                }
            }
            _ => {}
        }
    }

    events
}

fn system_notification_to_control_message(
    notification_type: SystemNotificationType,
    message: &str,
) -> HarnessControlMessage {
    match notification_type {
        SystemNotificationType::ThinkingMessage => {
            HarnessControlMessage::Session(SessionControlEvent::StateChanged {
                state: "thinking".to_string(),
                reason: Some(message.to_string()),
            })
        }
        SystemNotificationType::InlineMessage => {
            HarnessControlMessage::Runtime(RuntimeControlEvent::Notification {
                level: "info".to_string(),
                code: Some("inline_message".to_string()),
                message: message.to_string(),
            })
        }
        SystemNotificationType::RuntimeNotificationAttachment => {
            HarnessControlMessage::Runtime(RuntimeControlEvent::Notification {
                level: "info".to_string(),
                code: Some("runtime_notification_attachment".to_string()),
                message: message.to_string(),
            })
        }
    }
}

fn action_required_to_control_message(data: &ActionRequiredData) -> Option<HarnessControlMessage> {
    match data {
        ActionRequiredData::ToolConfirmation {
            id,
            tool_name,
            prompt,
            ..
        } => Some(HarnessControlMessage::Permission(
            PermissionControlEvent::RequiresAction {
                request_id: id.clone(),
                action_type: "tool_confirmation".to_string(),
                title: tool_name.clone(),
                details: prompt.clone(),
            },
        )),
        ActionRequiredData::Elicitation {
            id,
            message,
            requested_schema,
        } => Some(HarnessControlMessage::Permission(
            PermissionControlEvent::RequiresAction {
                request_id: id.clone(),
                action_type: "elicitation".to_string(),
                title: message.clone(),
                details: Some(requested_schema.to_string()),
            },
        )),
        ActionRequiredData::ElicitationResponse { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    use rmcp::model::{CallToolRequestParams, JsonObject, Role};

    use super::{
        build_completion_control_messages, build_permission_requested_control_message,
        build_permission_resolved_control_message, build_session_started_message,
        build_tool_finished_control_message, build_tool_started_control_message,
        build_worker_finished_control_message, build_worker_idle_control_message,
        build_worker_started_control_message, control_messages_for_agent_event,
        control_sequencer_for_session, register_harness_control_sequencer,
        unregister_harness_control_sequencer, HarnessControlSequencer,
    };
    use crate::agents::agent::{AgentEvent, ToolTransportRequestEvent};
    use crate::agents::harness::{
        CompletionSurfacePolicy, ExecuteCompletionOutcome, ExecuteCompletionState,
        ExecutionHostCompletionReport, HarnessControlMessage, NoopHarnessControlSink,
        RuntimeControlEvent, TaskKind, ToolInvocationSurface, ToolTransportKind,
        WorkerAttemptIdentity,
    };
    use crate::conversation::message::{
        Message, MessageContent, SystemNotificationType, ToolRequest,
    };
    use crate::conversation::Conversation;

    #[test]
    fn sequencer_allocates_monotonic_sequence_numbers() {
        let sequencer = HarnessControlSequencer::new("logical-1", "runtime-1");
        let first = sequencer.next(HarnessControlMessage::Runtime(
            RuntimeControlEvent::Notification {
                level: "info".to_string(),
                code: None,
                message: "boot".to_string(),
            },
        ));
        let second = sequencer.next(HarnessControlMessage::Runtime(
            RuntimeControlEvent::Notification {
                level: "info".to_string(),
                code: None,
                message: "ready".to_string(),
            },
        ));

        assert_eq!(first.sequence, 1);
        assert_eq!(second.sequence, 2);
    }

    #[test]
    fn registry_roundtrips_registered_sequencer() {
        let sequencer = Arc::new(HarnessControlSequencer::new("logical-3", "runtime-3"));
        register_harness_control_sequencer("runtime-3", sequencer.clone());

        let loaded = control_sequencer_for_session("runtime-3").expect("sequencer should exist");
        assert_eq!(loaded.next_sequence.load(Ordering::Relaxed), 1);

        unregister_harness_control_sequencer("runtime-3");
        assert!(control_sequencer_for_session("runtime-3").is_none());
    }

    #[test]
    fn session_started_message_uses_completion_surface_label() {
        let message = build_session_started_message(
            crate::agents::HarnessMode::Conversation,
            CompletionSurfacePolicy::Execute,
        );
        let value = serde_json::to_value(message).expect("serialize message");
        assert_eq!(value["event"]["surface"], "execute");
    }

    #[test]
    fn tool_transport_request_maps_to_tool_control_message() {
        let request = ToolTransportRequestEvent {
            request: ToolRequest {
                id: "req-1".to_string(),
                tool_call: Ok(CallToolRequestParams {
                    name: "read_file".into(),
                    arguments: Some(JsonObject::default()),
                    meta: None,
                    task: None,
                }),
                thought_signature: None,
            },
            transport: ToolTransportKind::ServerLocal,
            surface: ToolInvocationSurface::ExecuteHost,
        };

        let messages = control_messages_for_agent_event(&AgentEvent::ToolTransportRequest(request));
        let value = serde_json::to_value(&messages[0]).expect("serialize tool control message");
        assert_eq!(value["channel"], "tool");
        assert_eq!(value["event"]["type"], "transport_requested");
        assert_eq!(value["event"]["transport"], "server_local");
        assert_eq!(value["event"]["surface"], "execute_host");
    }

    #[test]
    fn assistant_text_maps_to_runtime_notification() {
        let mut visible_message = Message::assistant().with_text("hello control");
        visible_message.role = Role::Assistant;
        visible_message.metadata.user_visible = true;

        let messages = control_messages_for_agent_event(&AgentEvent::Message(visible_message));
        let value =
            serde_json::to_value(&messages[0]).expect("serialize runtime notification message");
        assert_eq!(value["channel"], "runtime");
        assert_eq!(value["event"]["type"], "notification");
    }

    #[test]
    fn thinking_notification_maps_to_session_state_changed() {
        let mut visible_message =
            Message::assistant().with_content(MessageContent::system_notification(
                SystemNotificationType::ThinkingMessage,
                "thinking about next step",
            ));
        visible_message.metadata.user_visible = true;

        let messages = control_messages_for_agent_event(&AgentEvent::Message(visible_message));
        let value = serde_json::to_value(&messages[0]).expect("serialize session control message");
        assert_eq!(value["channel"], "session");
        assert_eq!(value["event"]["type"], "state_changed");
        assert_eq!(value["event"]["state"], "thinking");
    }

    #[test]
    fn action_required_tool_confirmation_maps_to_permission_requires_action() {
        let mut visible_message = Message::assistant().with_action_required(
            "req-1",
            "document_tools__read_document".to_string(),
            JsonObject::default(),
            Some("approve read".to_string()),
        );
        visible_message.metadata.user_visible = true;

        let messages = control_messages_for_agent_event(&AgentEvent::Message(visible_message));
        let value =
            serde_json::to_value(&messages[0]).expect("serialize permission control message");
        assert_eq!(value["channel"], "permission");
        assert_eq!(value["event"]["type"], "requires_action");
        assert_eq!(value["event"]["action_type"], "tool_confirmation");
        assert_eq!(value["event"]["title"], "document_tools__read_document");
    }

    #[test]
    fn action_required_elicitation_maps_to_permission_requires_action() {
        let mut visible_message =
            Message::assistant().with_content(MessageContent::action_required_elicitation(
                "req-2",
                "please choose".to_string(),
                serde_json::json!({ "type": "object" }),
            ));
        visible_message.metadata.user_visible = true;

        let messages = control_messages_for_agent_event(&AgentEvent::Message(visible_message));
        let value =
            serde_json::to_value(&messages[0]).expect("serialize permission control message");
        assert_eq!(value["channel"], "permission");
        assert_eq!(value["event"]["type"], "requires_action");
        assert_eq!(value["event"]["action_type"], "elicitation");
        assert_eq!(value["event"]["title"], "please choose");
    }

    #[test]
    fn history_replaced_maps_to_runtime_compaction_observed() {
        let messages =
            control_messages_for_agent_event(&AgentEvent::HistoryReplaced(Conversation::empty()));
        let value =
            serde_json::to_value(&messages[0]).expect("serialize runtime compaction message");
        assert_eq!(value["channel"], "runtime");
        assert_eq!(value["event"]["type"], "compaction_observed");
        assert_eq!(value["event"]["strategy"], "history_replaced");
    }

    #[test]
    fn model_change_maps_to_runtime_notification_with_code() {
        let messages = control_messages_for_agent_event(&AgentEvent::ModelChange {
            model: "claude-test".to_string(),
            mode: "execute".to_string(),
        });
        let value =
            serde_json::to_value(&messages[0]).expect("serialize model change notification");
        assert_eq!(value["channel"], "runtime");
        assert_eq!(value["event"]["type"], "notification");
        assert_eq!(value["event"]["code"], "model_change");
    }

    #[test]
    fn worker_started_helper_reads_attempt_identity_from_metadata() {
        let mut snapshot = crate::agents::TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: "runtime-1".to_string(),
            depth: 1,
            kind: TaskKind::SwarmWorker,
            status: crate::agents::TaskStatus::Running,
            description: Some("worker".to_string()),
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: std::collections::HashMap::new(),
            started_at: 1,
            updated_at: 1,
            finished_at: None,
        };
        WorkerAttemptIdentity::followup("worker-1", "attempt-2", 2, "correction", "task-0")
            .write_to_metadata(&mut snapshot.metadata);

        let message = build_worker_started_control_message(&snapshot);
        let value = serde_json::to_value(message).expect("serialize worker started");
        assert_eq!(value["event"]["logical_worker_id"], "worker-1");
        assert_eq!(value["event"]["attempt_id"], "attempt-2");
        assert_eq!(value["event"]["target"], "docs/a.md");
    }

    #[test]
    fn worker_finished_helper_preserves_previous_task_id() {
        let attempt_identity =
            WorkerAttemptIdentity::followup("worker-1", "attempt-2", 2, "correction", "task-0");
        let message = build_worker_finished_control_message(
            "task-1",
            TaskKind::SwarmWorker,
            "completed",
            "done",
            true,
            Some(&attempt_identity),
        );
        let value = serde_json::to_value(message).expect("serialize worker finished");
        assert_eq!(value["event"]["status"], "completed");
        assert_eq!(value["event"]["attempt_index"], 2);
        assert_eq!(value["event"]["previous_task_id"], "task-0");
    }

    #[test]
    fn permission_requested_helper_carries_attempt_identity() {
        let attempt_identity = WorkerAttemptIdentity::fresh("worker-1", "attempt-1");
        let message = build_permission_requested_control_message(
            "req-1",
            "developer__shell",
            Some("worker-a".to_string()),
            Some(&attempt_identity),
        );
        let value = serde_json::to_value(message).expect("serialize permission requested");
        assert_eq!(value["event"]["type"], "requested");
        assert_eq!(value["event"]["logical_worker_id"], "worker-1");
        assert_eq!(value["event"]["attempt_id"], "attempt-1");
    }

    #[test]
    fn permission_resolved_helper_carries_worker_identity() {
        let attempt_identity = WorkerAttemptIdentity::fresh("worker-1", "attempt-1");
        let message = build_permission_resolved_control_message(
            "req-1",
            "developer__shell",
            "allow_once",
            Some("config".to_string()),
            Some("approved".to_string()),
            Some("worker-a".to_string()),
            Some(&attempt_identity),
        );
        let value = serde_json::to_value(message).expect("serialize permission resolved");
        assert_eq!(value["event"]["worker_name"], "worker-a");
        assert_eq!(value["event"]["logical_worker_id"], "worker-1");
        assert_eq!(value["event"]["attempt_id"], "attempt-1");
        assert_eq!(value["event"]["source"], "config");
    }

    #[test]
    fn worker_idle_helper_carries_attempt_identity() {
        let attempt_identity = WorkerAttemptIdentity::fresh("worker-1", "attempt-1");
        let message =
            build_worker_idle_control_message("task-1", "waiting", Some(&attempt_identity));
        let value = serde_json::to_value(message).expect("serialize worker idle");
        assert_eq!(value["event"]["logical_worker_id"], "worker-1");
        assert_eq!(value["event"]["attempt_id"], "attempt-1");
    }

    #[test]
    fn tool_lifecycle_helpers_build_stable_shapes() {
        let started = build_tool_started_control_message("req-1", "developer__shell");
        let finished = build_tool_finished_control_message(
            "req-1",
            "developer__shell",
            true,
            Some("shell finished".to_string()),
            Some(42),
        );

        let started_value = serde_json::to_value(started).expect("serialize tool started");
        let finished_value = serde_json::to_value(finished).expect("serialize tool finished");
        assert_eq!(started_value["channel"], "tool");
        assert_eq!(started_value["event"]["type"], "started");
        assert_eq!(finished_value["event"]["type"], "finished");
        assert_eq!(finished_value["event"]["success"], true);
        assert_eq!(finished_value["event"]["duration_ms"], 42);
    }

    #[test]
    fn completion_messages_publish_report_and_outcome() {
        let report = ExecutionHostCompletionReport {
            status: "completed".to_string(),
            summary: "done".to_string(),
            produced_artifacts: Vec::new(),
            accepted_artifacts: Vec::new(),
            next_steps: Vec::new(),
            validation_status: None,
            blocking_reason: None,
            reason_code: None,
            content_accessed: None,
            analysis_complete: None,
        };
        let outcome = ExecuteCompletionOutcome {
            state: ExecuteCompletionState::Completed,
            status: "completed".to_string(),
            summary: Some("done".to_string()),
            blocking_reason: None,
            completion_ready: true,
            required_tools_satisfied: true,
            has_blocking_signals: false,
            active_child_tasks: false,
        };

        let messages = build_completion_control_messages(&report, Some(&outcome));
        assert_eq!(messages.len(), 2);
        let value = serde_json::to_value(&messages).expect("serialize completion messages");
        assert_eq!(value[0]["event"]["type"], "structured_published");
        assert_eq!(value[1]["event"]["type"], "outcome_observed");
    }

    #[tokio::test]
    async fn noop_sink_accepts_envelope() {
        use crate::agents::harness::HarnessControlSink;

        let sequencer = HarnessControlSequencer::new("logical-2", "runtime-2");
        let envelope = sequencer.next(HarnessControlMessage::Runtime(
            RuntimeControlEvent::Notification {
                level: "debug".to_string(),
                code: Some("mirror_placeholder".to_string()),
                message: "ready".to_string(),
            },
        ));

        let sink = NoopHarnessControlSink;
        sink.handle(&envelope)
            .await
            .expect("noop sink should succeed");
    }
}
