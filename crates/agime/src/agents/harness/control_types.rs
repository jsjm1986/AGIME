use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HarnessControlEnvelope {
    pub session_id: String,
    pub runtime_session_id: String,
    pub sequence: u64,
    pub timestamp_ms: i64,
    pub payload: HarnessControlMessage,
}

impl HarnessControlEnvelope {
    pub fn new(
        session_id: impl Into<String>,
        runtime_session_id: impl Into<String>,
        sequence: u64,
        timestamp_ms: i64,
        payload: HarnessControlMessage,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            runtime_session_id: runtime_session_id.into(),
            sequence,
            timestamp_ms,
            payload,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "channel", content = "event", rename_all = "snake_case")]
pub enum HarnessControlMessage {
    Session(SessionControlEvent),
    Tool(ToolControlEvent),
    Permission(PermissionControlEvent),
    Worker(WorkerControlEvent),
    Completion(CompletionControlEvent),
    Runtime(RuntimeControlEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionControlEvent {
    Started {
        mode: String,
        surface: String,
    },
    StateChanged {
        state: String,
        reason: Option<String>,
    },
    Interrupted {
        source: String,
    },
    CancelRequested {
        source: String,
    },
    Finished {
        final_status: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolControlEvent {
    TransportRequested {
        request_id: String,
        tool_name: String,
        transport: String,
        surface: String,
    },
    Started {
        request_id: String,
        tool_name: String,
    },
    Progress {
        request_id: String,
        tool_name: String,
        message: String,
        percent: Option<u8>,
    },
    Finished {
        request_id: String,
        tool_name: String,
        success: bool,
        summary: Option<String>,
        duration_ms: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionControlEvent {
    Requested {
        request_id: String,
        tool_name: String,
        worker_name: Option<String>,
        logical_worker_id: Option<String>,
        attempt_id: Option<String>,
    },
    Resolved {
        request_id: String,
        tool_name: String,
        decision: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        feedback: Option<String>,
        worker_name: Option<String>,
        logical_worker_id: Option<String>,
        attempt_id: Option<String>,
    },
    TimedOut {
        request_id: String,
        tool_name: String,
        timeout_ms: u64,
        worker_name: Option<String>,
        logical_worker_id: Option<String>,
        attempt_id: Option<String>,
    },
    RequiresAction {
        request_id: String,
        action_type: String,
        title: String,
        details: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerControlEvent {
    Started {
        task_id: String,
        kind: String,
        target: Option<String>,
        logical_worker_id: Option<String>,
        attempt_id: Option<String>,
        attempt_index: Option<u32>,
        previous_task_id: Option<String>,
    },
    Progress {
        task_id: String,
        message: String,
        percent: Option<u8>,
    },
    Idle {
        task_id: String,
        message: String,
        logical_worker_id: Option<String>,
        attempt_id: Option<String>,
    },
    FollowupRequested {
        task_id: String,
        kind: String,
        reason: String,
        logical_worker_id: Option<String>,
        attempt_id: Option<String>,
        attempt_index: Option<u32>,
        previous_task_id: Option<String>,
    },
    Finished {
        task_id: String,
        kind: String,
        status: String,
        summary: String,
        produced_delta: Option<bool>,
        logical_worker_id: Option<String>,
        attempt_id: Option<String>,
        attempt_index: Option<u32>,
        previous_task_id: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompletionControlEvent {
    StructuredPublished {
        status: String,
        summary: Option<String>,
        blocking_reason: Option<String>,
        metadata: Option<Value>,
    },
    OutcomeObserved {
        status: String,
        summary: Option<String>,
        blocking_reason: Option<String>,
        reason_code: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeControlEvent {
    CompactionObserved {
        strategy: Option<String>,
        reason: Option<String>,
        before_tokens: Option<usize>,
        after_tokens: Option<usize>,
        phase: Option<String>,
    },
    Notification {
        level: String,
        code: Option<String>,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        CompletionControlEvent, HarnessControlEnvelope, HarnessControlMessage, RuntimeControlEvent,
    };

    #[test]
    fn envelope_preserves_sequence_and_payload() {
        let envelope = HarnessControlEnvelope::new(
            "logical-1",
            "runtime-1",
            7,
            1_727_000_123,
            HarnessControlMessage::Completion(CompletionControlEvent::OutcomeObserved {
                status: "completed".to_string(),
                summary: Some("ok".to_string()),
                blocking_reason: None,
                reason_code: None,
            }),
        );

        assert_eq!(envelope.sequence, 7);
        assert_eq!(envelope.session_id, "logical-1");
        assert_eq!(envelope.runtime_session_id, "runtime-1");
    }

    #[test]
    fn envelope_serializes_stable_channel_shape() {
        let envelope = HarnessControlEnvelope::new(
            "logical-2",
            "runtime-2",
            8,
            1_727_000_456,
            HarnessControlMessage::Runtime(RuntimeControlEvent::CompactionObserved {
                strategy: Some("recovery".to_string()),
                reason: Some("context_overflow".to_string()),
                before_tokens: Some(1200),
                after_tokens: Some(800),
                phase: Some("recover".to_string()),
            }),
        );

        let value = serde_json::to_value(&envelope).expect("serialize envelope");
        assert_eq!(value["payload"]["channel"], "runtime");
        assert_eq!(value["payload"]["event"]["type"], "compaction_observed");
    }
}
