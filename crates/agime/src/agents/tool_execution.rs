use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use async_stream::try_stream;
use futures::stream::BoxStream;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::config::permission::PermissionLevel;
use crate::mcp_utils::ToolResult;
use crate::permission::Permission;
use rmcp::model::{Content, ServerNotification};

// ToolCallResult combines the result of a tool call with an optional notification stream that
// can be used to receive notifications from the tool.
pub struct ToolCallResult {
    pub result: Box<dyn Future<Output = ToolResult<rmcp::model::CallToolResult>> + Send + Unpin>,
    pub notification_stream: Option<Box<dyn Stream<Item = ServerNotification> + Send + Unpin>>,
}

impl From<ToolResult<rmcp::model::CallToolResult>> for ToolCallResult {
    fn from(result: ToolResult<rmcp::model::CallToolResult>) -> Self {
        Self {
            result: Box::new(futures::future::ready(result)),
            notification_stream: None,
        }
    }
}

use crate::agents::Agent;
use crate::conversation::message::{Message, ToolRequest};
use crate::tool_inspection::get_security_finding_id_from_results;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NormalizedToolResultStatus {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NormalizedToolErrorClass {
    PermissionDenied,
    ValidationError,
    ExecutionError,
    TransportError,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedToolResult {
    pub status: NormalizedToolResultStatus,
    pub success: bool,
    pub display_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_class: Option<NormalizedToolErrorClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_input: Option<Value>,
}

pub const DECLINED_RESPONSE: &str = "The user has declined to run this tool. \
    DO NOT attempt to call this tool again. \
    If there are no alternative methods to proceed, clearly explain the situation and STOP.";

pub const CHAT_MODE_TOOL_SKIPPED_RESPONSE: &str = "Let the user know the tool call was skipped in AGIME chat mode. \
                                        DO NOT apologize for skipping the tool call. DO NOT say sorry. \
                                        Provide an explanation of what the tool call would do, structured as a \
                                        plan for the user. Again, DO NOT apologize. \
                                        **Example Plan:**\n \
                                        1. **Identify Task Scope** - Determine the purpose and expected outcome.\n \
                                        2. **Outline Steps** - Break down the steps.\n \
                                        If needed, adjust the explanation based on user preferences or questions.";

pub fn tool_execution_cancelled_error_text(tool_name: &str) -> String {
    format!(
        "[step:tool_cancelled] Error: tool '{}' cancelled",
        tool_name
    )
}

pub fn tool_execution_was_cancelled(error_text: &str) -> bool {
    error_text
        .to_ascii_lowercase()
        .contains("[step:tool_cancelled]")
        || error_text.to_ascii_lowercase().contains("tool cancelled")
}

fn extract_tool_result_display_text(result: &rmcp::model::CallToolResult) -> String {
    let text = result
        .content
        .iter()
        .filter_map(|content| content.as_text().map(|text| text.text.trim().to_string()))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if !text.is_empty() {
        return text;
    }
    result
        .structured_content
        .as_ref()
        .map(Value::to_string)
        .unwrap_or_default()
}

fn classify_tool_error_text(
    text: &str,
    structured_output: Option<&Value>,
) -> NormalizedToolErrorClass {
    let lower = text.to_ascii_lowercase();
    let reason_code = structured_output
        .and_then(|value| value.get("reason_code"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let combined = if reason_code.is_empty() {
        lower.clone()
    } else {
        format!("{} {}", lower, reason_code)
    };

    if combined.contains("tool_cancelled")
        || combined.contains("cancelled")
        || combined.contains("aborted")
    {
        NormalizedToolErrorClass::Cancelled
    } else if combined.contains("permission")
        || combined.contains("declined")
        || combined.contains("denied")
        || combined.contains("approval")
    {
        NormalizedToolErrorClass::PermissionDenied
    } else if combined.contains("invalid")
        || combined.contains("schema")
        || combined.contains("missing field")
        || combined.contains("unknown variant")
        || combined.contains("parameter")
        || combined.contains("contract")
        || combined.contains("verifiable file effect")
    {
        NormalizedToolErrorClass::ValidationError
    } else if combined.contains("timeout")
        || combined.contains("transport")
        || combined.contains("connection")
        || combined.contains("network")
        || combined.contains("closed")
    {
        NormalizedToolErrorClass::TransportError
    } else if !combined.trim().is_empty() {
        NormalizedToolErrorClass::ExecutionError
    } else {
        NormalizedToolErrorClass::Unknown
    }
}

pub fn normalize_call_tool_result(
    raw_tool_name: Option<&str>,
    final_input: Option<&Value>,
    result: &rmcp::model::CallToolResult,
) -> NormalizedToolResult {
    let display_text = extract_tool_result_display_text(result);
    let structured_output = result.structured_content.clone();
    let error_class = result
        .is_error
        .unwrap_or(false)
        .then(|| classify_tool_error_text(&display_text, structured_output.as_ref()));
    let status = match error_class {
        Some(NormalizedToolErrorClass::Cancelled) => NormalizedToolResultStatus::Cancelled,
        Some(_) => NormalizedToolResultStatus::Failed,
        None => NormalizedToolResultStatus::Succeeded,
    };
    NormalizedToolResult {
        status,
        success: matches!(status, NormalizedToolResultStatus::Succeeded),
        display_text,
        structured_output,
        error_class,
        raw_tool_name: raw_tool_name.map(str::to_string),
        final_input: final_input.cloned(),
    }
}

pub fn normalize_error_data(
    raw_tool_name: Option<&str>,
    final_input: Option<&Value>,
    error: &rmcp::model::ErrorData,
) -> NormalizedToolResult {
    let display_text = error.message.to_string();
    let error_class = classify_tool_error_text(&display_text, error.data.as_ref());
    let status = if error_class == NormalizedToolErrorClass::Cancelled {
        NormalizedToolResultStatus::Cancelled
    } else {
        NormalizedToolResultStatus::Failed
    };
    NormalizedToolResult {
        status,
        success: false,
        display_text,
        structured_output: error.data.clone(),
        error_class: Some(error_class),
        raw_tool_name: raw_tool_name.map(str::to_string),
        final_input: final_input.cloned(),
    }
}

pub fn normalize_tool_execution_error_text(
    raw_tool_name: Option<&str>,
    final_input: Option<&Value>,
    error_text: &str,
) -> NormalizedToolResult {
    let error_class = classify_tool_error_text(error_text, None);
    let status = if error_class == NormalizedToolErrorClass::Cancelled {
        NormalizedToolResultStatus::Cancelled
    } else {
        NormalizedToolResultStatus::Failed
    };
    NormalizedToolResult {
        status,
        success: false,
        display_text: error_text.trim().to_string(),
        structured_output: None,
        error_class: Some(error_class),
        raw_tool_name: raw_tool_name.map(str::to_string),
        final_input: final_input.cloned(),
    }
}

impl Agent {
    pub(crate) fn handle_approval_tool_requests<'a>(
        &'a self,
        tool_requests: &'a [ToolRequest],
        approved_requests: Arc<Mutex<Vec<ToolRequest>>>,
        request_to_response_map: &'a HashMap<String, Arc<Mutex<Message>>>,
        _cancellation_token: Option<CancellationToken>,
        inspection_results: &'a [crate::tool_inspection::InspectionResult],
    ) -> BoxStream<'a, anyhow::Result<Message>> {
        try_stream! {
        for request in tool_requests.iter() {
            if let Ok(tool_call) = request.tool_call.clone() {
                // Find the corresponding inspection result for this tool request
                let security_message = inspection_results.iter()
                    .find(|result| result.tool_request_id == request.id)
                    .and_then(|result| {
                        if let crate::tool_inspection::InspectionAction::RequireApproval(Some(message)) = &result.action {
                            Some(message.clone())
                        } else {
                            None
                        }
                    });

                let confirmation = Message::assistant()
                    .with_action_required(
                        request.id.clone(),
                        tool_call.name.to_string().clone(),
                        tool_call.arguments.clone().unwrap_or_default(),
                        security_message,
                    )
                    .user_only();
                yield confirmation;

                let mut rx = self.confirmation_rx.lock().await;
                while let Some((req_id, confirmation)) = rx.recv().await {
                    if req_id == request.id {
                        // Log user decision if this was a security alert
                        if let Some(finding_id) = get_security_finding_id_from_results(&request.id, inspection_results) {
                            tracing::info!(
                                counter.goose.prompt_injection_user_decisions = 1,
                                decision = ?confirmation.permission,
                                finding_id = %finding_id,
                                "User security decision"
                            );
                        }

                        if confirmation.permission == Permission::AllowOnce || confirmation.permission == Permission::AlwaysAllow {
                            approved_requests.lock().await.push(request.clone());

                            // Update the shared permission manager when user selects "Always Allow"
                            if confirmation.permission == Permission::AlwaysAllow {
                                self.tool_inspection_manager
                                    .update_permission_manager(&tool_call.name, PermissionLevel::AlwaysAllow)
                                    .await;
                            }
                        } else {
                            // User declined - update the specific response message for this request
                            if let Some(response_msg) = request_to_response_map.get(&request.id) {
                                let mut response = response_msg.lock().await;
                                *response = response.clone().with_tool_response(
                                    request.id.clone(),
                                    Ok(rmcp::model::CallToolResult {
                                        content: vec![Content::text(DECLINED_RESPONSE)],
                                        structured_content: None,
                                        is_error: Some(true),
                                        meta: None,
                                    }),
                                );
                            }
                        }
                        break; // Exit the loop once the matching `req_id` is found
                    }
                }
            }
        }
    }.boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_call_tool_result, normalize_error_data, normalize_tool_execution_error_text,
        tool_execution_cancelled_error_text, NormalizedToolErrorClass, NormalizedToolResultStatus,
    };
    use rmcp::model::{CallToolResult, Content, ErrorCode, ErrorData};
    use serde_json::json;

    #[test]
    fn normalize_call_tool_result_preserves_structured_output() {
        let result = CallToolResult {
            content: vec![Content::text("document read".to_string())],
            structured_content: Some(json!({
                "doc_id": "doc-1",
                "content_accessed": true
            })),
            is_error: Some(false),
            meta: None,
        };

        let normalized =
            normalize_call_tool_result(Some("document_tools__read_document"), None, &result);
        assert_eq!(normalized.status, NormalizedToolResultStatus::Succeeded);
        assert!(normalized.success);
        assert_eq!(normalized.display_text, "document read");
        assert_eq!(
            normalized.structured_output,
            Some(json!({
                "doc_id": "doc-1",
                "content_accessed": true
            }))
        );
    }

    #[test]
    fn normalize_error_data_classifies_permission_denied() {
        let error = ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            "Leader permission bridge denied this worker request".to_string(),
            None,
        );

        let normalized = normalize_error_data(Some("developer__shell"), None, &error);
        assert_eq!(normalized.status, NormalizedToolResultStatus::Failed);
        assert_eq!(
            normalized.error_class,
            Some(NormalizedToolErrorClass::PermissionDenied)
        );
    }

    #[test]
    fn normalize_tool_execution_error_text_classifies_cancelled() {
        let normalized = normalize_tool_execution_error_text(
            Some("developer__shell"),
            None,
            &tool_execution_cancelled_error_text("developer__shell"),
        );
        assert_eq!(normalized.status, NormalizedToolResultStatus::Cancelled);
        assert_eq!(
            normalized.error_class,
            Some(NormalizedToolErrorClass::Cancelled)
        );
    }
}
