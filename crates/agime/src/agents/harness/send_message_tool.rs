use std::borrow::Cow;
use std::collections::HashMap;

use rmcp::model::{CallToolResult, Content, ErrorCode, ErrorData, Tool, ToolAnnotations};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::mailbox::{
    write_to_mailbox_from_root, MailboxLineage, MailboxMessage, MailboxMessageKind,
    SwarmProtocolKind, SwarmProtocolMessage,
};
use super::runtime_state::{load_worker_runtime_state, HarnessWorkerRuntimeState};
use super::task_runtime::WorkerAttemptIdentity;
use crate::agents::tool_execution::ToolCallResult;

pub const SEND_MESSAGE_TOOL_NAME: &str = "send_message";

#[derive(Debug, Deserialize)]
struct SendMessageParams {
    to: String,
    message: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    referenced_task_ids: Option<Vec<String>>,
}

pub fn create_send_message_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "to": {
                "type": "string",
                "description": "Recipient worker logical id, or `leader`."
            },
            "message": {
                "type": "string",
                "description": "Bounded coordination message for another swarm worker or the leader."
            },
            "summary": {
                "type": "string",
                "description": "Optional short summary preview for the mailbox."
            },
            "thread_id": {
                "type": "string",
                "description": "Optional coordination thread id. Reuse this when replying in the same thread."
            },
            "referenced_task_ids": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional task ids this message refers to."
            }
        },
        "required": ["to", "message"]
    });

    Tool::new(
        SEND_MESSAGE_TOOL_NAME.to_string(),
        "Send a bounded coordination message to another swarm worker or the leader. Only available inside swarm worker runs when worker peer messaging is enabled.",
        schema.as_object().expect("tool schema object").clone(),
    )
    .annotate(ToolAnnotations {
        title: Some("Send Message".to_string()),
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(false),
        open_world_hint: Some(false),
    })
}

pub async fn handle_send_message_tool(arguments: Value, session_id: &str) -> ToolCallResult {
    let params: SendMessageParams = match serde_json::from_value(arguments) {
        Ok(value) => value,
        Err(error) => {
            return ToolCallResult::from(Err(ErrorData {
                code: ErrorCode::INVALID_PARAMS,
                message: Cow::from(format!("Invalid parameters: {}", error)),
                data: None,
            }));
        }
    };

    let runtime_state = match load_worker_runtime_state(session_id).await {
        Ok(Some(state)) if state.is_configured() => state,
        Ok(_) => {
            return ToolCallResult::from(Err(ErrorData {
                code: ErrorCode::INVALID_REQUEST,
                message: Cow::from(
                    "send_message is only available inside an active swarm worker runtime."
                        .to_string(),
                ),
                data: None,
            }));
        }
        Err(error) => {
            return ToolCallResult::from(Err(ErrorData {
                code: ErrorCode::INTERNAL_ERROR,
                message: Cow::from(format!("Failed to load worker runtime state: {}", error)),
                data: None,
            }));
        }
    };

    if !runtime_state.allow_worker_messaging {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INVALID_REQUEST,
            message: Cow::from(
                "Worker peer messaging is disabled by the current swarm runtime policy."
                    .to_string(),
            ),
            data: None,
        }));
    }

    let Some(mailbox_root) = runtime_state.mailbox_dir.clone() else {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from("Worker mailbox root is missing from runtime state.".to_string()),
            data: None,
        }));
    };

    let Some(from_address) = sender_identity(&runtime_state) else {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(
                "Worker logical identity is missing from runtime state.".to_string(),
            ),
            data: None,
        }));
    };

    let to = params.to.trim().to_string();
    if to.is_empty() {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from("`to` must not be empty.".to_string()),
            data: None,
        }));
    }
    if params.message.trim().is_empty() {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from("`message` must not be empty.".to_string()),
            data: None,
        }));
    }
    if to == from_address {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from("A worker cannot send a peer message to itself.".to_string()),
            data: None,
        }));
    }

    let mut allowed_recipients = runtime_state.peer_worker_addresses.clone();
    if let Some(leader_address) = runtime_state.leader_name.clone() {
        allowed_recipients.push(leader_address);
    }
    allowed_recipients.sort();
    allowed_recipients.dedup();

    if !allowed_recipients.iter().any(|candidate| candidate == &to) {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INVALID_REQUEST,
            message: Cow::from(format!(
                "Recipient `{}` is outside this swarm run. Allowed recipients: {}",
                to,
                if allowed_recipients.is_empty() {
                    "none".to_string()
                } else {
                    allowed_recipients.join(", ")
                }
            )),
            data: None,
        }));
    }

    let thread_id = params
        .thread_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("peer-{}", Uuid::new_v4().simple()));
    let message_id = format!("peer-msg-{}", Uuid::new_v4().simple());

    let mailbox_message = MailboxMessage {
        id: message_id.clone(),
        from: from_address.clone(),
        to: to.clone(),
        kind: MailboxMessageKind::PeerMessage,
        text: params.message.clone(),
        summary: params
            .summary
            .filter(|value| !value.trim().is_empty())
            .or_else(|| Some(default_summary(&params.message))),
        timestamp: String::new(),
        read: false,
        protocol: Some(SwarmProtocolMessage {
            kind: SwarmProtocolKind::PeerMessage,
            thread_id: Some(thread_id.clone()),
            directive_kind: None,
            reason_code: None,
            target_artifacts: Vec::new(),
            referenced_task_ids: params.referenced_task_ids.clone().unwrap_or_default(),
            lineage: Some(MailboxLineage {
                logical_worker_id: runtime_state.logical_worker_id.clone(),
                attempt_id: runtime_state.attempt_id.clone(),
                attempt_index: runtime_state.attempt_index,
                previous_task_id: runtime_state.previous_task_id.clone(),
            }),
        }),
        metadata: build_metadata(&runtime_state, &thread_id, &to, params.referenced_task_ids),
    };

    if let Err(error) = write_to_mailbox_from_root(&mailbox_root, &to, mailbox_message) {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to write peer mailbox message: {}", error)),
            data: None,
        }));
    }

    let structured = json!({
        "message_id": message_id,
        "thread_id": thread_id,
        "from": from_address,
        "to": to,
        "status": "sent",
    });

    ToolCallResult::from(Ok(CallToolResult {
        content: vec![Content::text("Peer message sent.".to_string())],
        structured_content: Some(structured),
        is_error: Some(false),
        meta: None,
    }))
}

fn sender_identity(state: &HarnessWorkerRuntimeState) -> Option<String> {
    state
        .logical_worker_id
        .clone()
        .or_else(|| state.worker_name.clone())
}

fn build_metadata(
    state: &HarnessWorkerRuntimeState,
    thread_id: &str,
    recipient: &str,
    referenced_task_ids: Option<Vec<String>>,
) -> HashMap<String, String> {
    let mut metadata = HashMap::new();
    metadata.insert("thread_id".to_string(), thread_id.to_string());
    metadata.insert("recipient".to_string(), recipient.to_string());
    if let Some(referenced) = referenced_task_ids {
        if !referenced.is_empty() {
            metadata.insert(
                "referenced_task_ids".to_string(),
                serde_json::to_string(&referenced).unwrap_or_default(),
            );
        }
    }
    if let (Some(logical_worker_id), Some(attempt_id)) =
        (state.logical_worker_id.clone(), state.attempt_id.clone())
    {
        WorkerAttemptIdentity {
            logical_worker_id,
            attempt_id,
            attempt_index: state.attempt_index.unwrap_or(0),
            followup_kind: "peer_message".to_string(),
            previous_task_id: state.previous_task_id.clone(),
        }
        .write_to_metadata(&mut metadata);
    }
    metadata
}

fn default_summary(message: &str) -> String {
    message
        .split_whitespace()
        .take(12)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::harness::save_worker_runtime_state;
    use crate::agents::harness::CoordinatorRole;
    use crate::session::{SessionManager, SessionType};

    #[tokio::test]
    async fn send_message_writes_peer_message_to_recipient_inbox() {
        let root = tempfile::tempdir().expect("tempdir");
        let session_id = format!("worker-{}", Uuid::new_v4().simple());
        SessionManager::create_session_with_id(
            session_id.clone(),
            root.path().to_path_buf(),
            "worker".to_string(),
            SessionType::SubAgent,
        )
        .await
        .expect("session");

        let mailbox_root = root
            .path()
            .join(".agime")
            .join("swarm")
            .join("run-1")
            .join("mailboxes");
        save_worker_runtime_state(
            &session_id,
            &HarnessWorkerRuntimeState {
                task_board_session_id: None,
                swarm_run_id: Some("run-1".to_string()),
                worker_name: Some("worker-a".to_string()),
                leader_name: Some("leader".to_string()),
                logical_worker_id: Some("worker-a".to_string()),
                attempt_id: Some("attempt-1".to_string()),
                attempt_index: Some(0),
                previous_task_id: None,
                coordinator_role: CoordinatorRole::Worker,
                mailbox_dir: Some(mailbox_root.clone()),
                permission_dir: None,
                scratchpad_dir: None,
                enable_permission_bridge: false,
                allow_worker_messaging: true,
                peer_worker_addresses: vec!["worker-b".to_string()],
                peer_worker_catalog: vec!["worker-b => docs/b.md".to_string()],
                validation_mode: false,
            },
        )
        .await
        .expect("save state");

        let result = handle_send_message_tool(
            json!({
                "to": "worker-b",
                "message": "Need the parsed findings from docs/b.md before I finalize docs/a.md",
                "thread_id": "thread-1",
                "referenced_task_ids": ["task-a"]
            }),
            &session_id,
        )
        .await;

        let tool_result = result.result.await.expect("tool result");
        assert!(!tool_result.is_error.unwrap_or(false));

        let inbox_path = mailbox_root.join("worker-b.json");
        let messages: Vec<MailboxMessage> =
            serde_json::from_str(&std::fs::read_to_string(inbox_path).expect("read inbox"))
                .expect("parse inbox");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].kind, MailboxMessageKind::PeerMessage);
        assert_eq!(
            messages[0].protocol.as_ref().map(|protocol| &protocol.kind),
            Some(&SwarmProtocolKind::PeerMessage)
        );
        assert_eq!(messages[0].from, "worker-a");
        assert_eq!(messages[0].to, "worker-b");
        assert_eq!(
            messages[0].metadata.get("thread_id").map(String::as_str),
            Some("thread-1")
        );
    }
}
