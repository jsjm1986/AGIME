use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use async_stream::try_stream;
use futures::stream::BoxStream;
use futures::{Stream, StreamExt};
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
