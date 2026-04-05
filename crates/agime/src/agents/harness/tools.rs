use std::collections::HashMap;
use std::sync::Arc;

use crate::agents::agent::ToolStream;
use crate::agents::final_output_tool::FinalOutputTool;
use crate::conversation::message::{Message, ToolRequest};
use rmcp::model::Tool;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::result_budget::ToolResultBudgetBucket;
use crate::agents::final_output_tool::FINAL_OUTPUT_TOOL_NAME;
use crate::agents::subagent_tool::SUBAGENT_TOOL_NAME;
use crate::agents::swarm_tool::SWARM_TOOL_NAME;
use crate::agents::HarnessMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecutionMode {
    ConcurrentReadOnly,
    SerialMutating,
    StatefulSerial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConcurrencyPolicy {
    Static(ToolExecutionMode),
    InputSensitive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptBehavior {
    Cancel,
    Block,
    ResumeSafe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolTransportKind {
    ExternalFrontend,
    ServerLocal,
    WorkerLocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolInvocationSurface {
    Conversation,
    ExecuteHost,
    Worker,
}

#[derive(Debug, Clone, Default)]
pub struct FrontendTransportPartition {
    pub server_local: Vec<ScheduledToolCall>,
    pub external_frontend: Vec<ScheduledToolCall>,
}

#[derive(Debug, Clone)]
pub struct ToolTransportDispatch {
    pub call: ScheduledToolCall,
    pub transport: ToolTransportKind,
    pub surface: ToolInvocationSurface,
}

#[derive(Debug, Clone, Default)]
pub struct TransportDispatchPlan {
    pub dispatches: Vec<ToolTransportDispatch>,
}

pub struct TransportDispatcher;

#[derive(Debug, Clone)]
pub struct RuntimeToolMeta {
    pub name: String,
    pub transport: ToolTransportKind,
    pub execution_mode: ToolExecutionMode,
    pub concurrency_policy: ConcurrencyPolicy,
    pub interrupt_behavior: InterruptBehavior,
    pub requires_permission: bool,
    pub supports_notifications: bool,
    pub supports_task_progress: bool,
    pub supports_streaming_result: bool,
    pub supports_child_tasks: bool,
    pub requires_explicit_scope: bool,
    pub max_result_chars: Option<usize>,
    pub result_budget_bucket: ToolResultBudgetBucket,
    pub is_frontend: bool,
    pub is_subagent: bool,
    pub is_final_output: bool,
}

impl RuntimeToolMeta {
    pub fn with_transport(mut self, transport: ToolTransportKind) -> Self {
        self.transport = transport;
        if matches!(transport, ToolTransportKind::ServerLocal) {
            self.is_frontend = false;
            self.supports_notifications = true;
        }
        self
    }
}

pub fn classify_tool_transport_kind(
    request: &ToolRequest,
    meta: &RuntimeToolMeta,
    server_local_tool_names: &[String],
) -> ToolTransportKind {
    let tool_name = request
        .tool_call
        .as_ref()
        .ok()
        .map(|call| call.name.to_string())
        .unwrap_or_default();

    if matches!(meta.transport, ToolTransportKind::ExternalFrontend) {
        if server_local_tool_names
            .iter()
            .any(|name| name == &tool_name)
        {
            ToolTransportKind::ServerLocal
        } else {
            ToolTransportKind::ExternalFrontend
        }
    } else {
        ToolTransportKind::WorkerLocal
    }
}

pub fn tool_invocation_surface(
    mode: HarnessMode,
    is_worker_session: bool,
) -> ToolInvocationSurface {
    if is_worker_session {
        ToolInvocationSurface::Worker
    } else if matches!(mode, HarnessMode::Execute) {
        ToolInvocationSurface::ExecuteHost
    } else {
        ToolInvocationSurface::Conversation
    }
}

pub fn partition_frontend_transports(
    calls: Vec<ScheduledToolCall>,
    server_local_tool_names: &[String],
) -> FrontendTransportPartition {
    let dispatch_plan = TransportDispatcher::dispatch(
        calls,
        ToolInvocationSurface::Conversation,
        server_local_tool_names,
    );
    let mut partition = FrontendTransportPartition::default();
    for dispatch in dispatch_plan.dispatches {
        match dispatch.transport {
            ToolTransportKind::ServerLocal => partition.server_local.push(dispatch.call),
            ToolTransportKind::ExternalFrontend | ToolTransportKind::WorkerLocal => {
                partition.external_frontend.push(dispatch.call)
            }
        }
    }
    partition
}

impl TransportDispatchPlan {
    pub fn pending_request_ids(&self) -> Vec<String> {
        self.dispatches
            .iter()
            .map(|dispatch| dispatch.call.request.id.clone())
            .collect()
    }
}

impl TransportDispatcher {
    pub fn dispatch(
        calls: Vec<ScheduledToolCall>,
        surface: ToolInvocationSurface,
        server_local_tool_names: &[String],
    ) -> TransportDispatchPlan {
        let dispatches = calls
            .into_iter()
            .map(|mut call| {
                let transport = classify_tool_transport_kind(
                    &call.request,
                    &call.meta,
                    server_local_tool_names,
                );
                call.meta = call.meta.clone().with_transport(transport);
                ToolTransportDispatch {
                    transport,
                    call,
                    surface,
                }
            })
            .collect();
        TransportDispatchPlan { dispatches }
    }
}

pub struct HarnessToolResult {
    pub request_id: String,
    pub stream: ToolStream,
}

pub struct ToolResponsePlan {
    ordered_requests: Vec<ToolRequest>,
    response_messages: Vec<Arc<Mutex<Message>>>,
    request_to_response_map: HashMap<String, Arc<Mutex<Message>>>,
}

impl ToolResponsePlan {
    pub fn new(frontend_requests: &[ToolRequest], remaining_requests: &[ToolRequest]) -> Self {
        let ordered_requests = frontend_requests
            .iter()
            .chain(remaining_requests.iter())
            .cloned()
            .collect::<Vec<_>>();
        let response_messages = (0..ordered_requests.len())
            .map(|_| {
                Arc::new(Mutex::new(
                    Message::user().with_id(format!("msg_{}", Uuid::new_v4())),
                ))
            })
            .collect::<Vec<_>>();
        let request_to_response_map = ordered_requests
            .iter()
            .zip(response_messages.iter())
            .map(|(request, response)| (request.id.clone(), response.clone()))
            .collect::<HashMap<_, _>>();

        Self {
            ordered_requests,
            response_messages,
            request_to_response_map,
        }
    }

    pub fn len(&self) -> usize {
        self.ordered_requests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ordered_requests.is_empty()
    }

    pub fn request_to_response_map(&self) -> &HashMap<String, Arc<Mutex<Message>>> {
        &self.request_to_response_map
    }

    pub fn response_message_for(&self, request_id: &str) -> Option<Arc<Mutex<Message>>> {
        self.request_to_response_map.get(request_id).cloned()
    }

    pub fn request_for(&self, request_id: &str) -> Option<&ToolRequest> {
        self.ordered_requests
            .iter()
            .find(|request| request.id == request_id)
    }

    pub fn ordered_pairs(&self) -> Vec<(ToolRequest, Arc<Mutex<Message>>)> {
        self.ordered_requests
            .iter()
            .cloned()
            .zip(self.response_messages.iter().cloned())
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FinalOutputState {
    pub tool_present: bool,
    pub collected_output: Option<String>,
}

impl FinalOutputState {
    pub fn is_collected(&self) -> bool {
        self.collected_output.is_some()
    }

    pub fn collected_output(&self) -> Option<&str> {
        self.collected_output.as_deref()
    }

    pub fn into_assistant_message(self) -> Option<Message> {
        self.collected_output
            .map(|message| Message::assistant().with_text(message))
    }
}

#[derive(Debug, Clone)]
pub struct ScheduledToolCall {
    pub request: ToolRequest,
    pub meta: RuntimeToolMeta,
}

pub fn snapshot_final_output_state(tool: Option<&FinalOutputTool>) -> FinalOutputState {
    let Some(tool) = tool else {
        return FinalOutputState::default();
    };

    FinalOutputState {
        tool_present: true,
        collected_output: tool.final_output.clone(),
    }
}

fn request_looks_read_only(request: &ToolRequest) -> bool {
    let Ok(tool_call) = request.tool_call.clone() else {
        return false;
    };
    let payload = serde_json::Value::Object(tool_call.arguments.unwrap_or_default())
        .to_string()
        .to_ascii_lowercase();
    [
        "query", "pattern", "glob", "search", "list", "read", "inspect",
    ]
    .iter()
    .any(|marker| payload.contains(marker))
}

pub fn infer_runtime_tool_meta(
    tool: &Tool,
    request: &ToolRequest,
    is_frontend: bool,
) -> RuntimeToolMeta {
    let is_final_output = tool.name.as_ref() == FINAL_OUTPUT_TOOL_NAME;
    let is_subagent = tool.name.as_ref() == SUBAGENT_TOOL_NAME;
    let is_swarm = tool.name.as_ref() == SWARM_TOOL_NAME;
    let read_only = tool
        .annotations
        .as_ref()
        .and_then(|a| a.read_only_hint)
        .unwrap_or(false);
    let destructive = tool
        .annotations
        .as_ref()
        .and_then(|a| a.destructive_hint)
        .unwrap_or(false);

    let concurrency_policy =
        if is_final_output || is_frontend || is_subagent || is_swarm || read_only {
            ConcurrencyPolicy::Static(if is_final_output || is_frontend {
                ToolExecutionMode::StatefulSerial
            } else if is_subagent || is_swarm {
                ToolExecutionMode::SerialMutating
            } else {
                ToolExecutionMode::ConcurrentReadOnly
            })
        } else if request_looks_read_only(request) {
            ConcurrencyPolicy::InputSensitive
        } else {
            ConcurrencyPolicy::Static(ToolExecutionMode::SerialMutating)
        };

    let execution_mode = if is_final_output || is_frontend {
        ToolExecutionMode::StatefulSerial
    } else if is_subagent || is_swarm {
        ToolExecutionMode::SerialMutating
    } else if matches!(concurrency_policy, ConcurrencyPolicy::InputSensitive)
        && request_looks_read_only(request)
    {
        ToolExecutionMode::ConcurrentReadOnly
    } else if read_only && !destructive {
        ToolExecutionMode::ConcurrentReadOnly
    } else {
        ToolExecutionMode::SerialMutating
    };

    RuntimeToolMeta {
        name: tool.name.to_string(),
        transport: if is_frontend {
            ToolTransportKind::ExternalFrontend
        } else {
            ToolTransportKind::WorkerLocal
        },
        execution_mode,
        concurrency_policy,
        interrupt_behavior: if is_subagent || is_swarm || tool.execution.is_some() {
            InterruptBehavior::ResumeSafe
        } else {
            match execution_mode {
                ToolExecutionMode::ConcurrentReadOnly => InterruptBehavior::Cancel,
                ToolExecutionMode::SerialMutating | ToolExecutionMode::StatefulSerial => {
                    InterruptBehavior::Block
                }
            }
        },
        requires_permission: !read_only || is_subagent || is_swarm,
        supports_notifications: !is_frontend,
        supports_task_progress: tool.execution.is_some(),
        supports_streaming_result: tool.execution.is_some() || is_subagent || is_swarm,
        supports_child_tasks: is_subagent || is_swarm,
        requires_explicit_scope: !read_only || is_subagent || is_swarm,
        max_result_chars: if is_final_output {
            None
        } else if is_subagent || is_swarm {
            Some(16_000)
        } else if read_only {
            Some(8_000)
        } else {
            Some(4_000)
        },
        result_budget_bucket: if is_final_output {
            ToolResultBudgetBucket::Unlimited
        } else if is_subagent || is_swarm {
            ToolResultBudgetBucket::Large
        } else if read_only {
            ToolResultBudgetBucket::Standard
        } else {
            ToolResultBudgetBucket::Small
        },
        is_frontend,
        is_subagent,
        is_final_output,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::Response;
    use serde_json::json;

    #[test]
    fn snapshot_final_output_state_defaults_when_tool_absent() {
        let state = snapshot_final_output_state(None);
        assert!(!state.tool_present);
        assert!(!state.is_collected());
        assert!(state.collected_output().is_none());
    }

    #[test]
    fn snapshot_final_output_state_reads_collected_output() {
        let response = Response {
            json_schema: Some(json!({
                "type": "object",
                "properties": { "ok": { "type": "boolean" } },
                "required": ["ok"]
            })),
        };
        let mut tool = FinalOutputTool::new(response);
        tool.final_output = Some("{\"ok\":true}".to_string());

        let state = snapshot_final_output_state(Some(&tool));
        assert!(state.tool_present);
        assert!(state.is_collected());
        assert_eq!(state.collected_output(), Some("{\"ok\":true}"));
    }

    #[test]
    fn tool_response_plan_preserves_request_order_and_lookup() {
        let frontend = vec![ToolRequest {
            id: "frontend".to_string(),
            tool_call: Ok(rmcp::model::CallToolRequestParams {
                name: "frontend_tool".to_string().into(),
                arguments: Some(rmcp::object!({})),
                meta: None,
                task: None,
            }),
            thought_signature: None,
        }];
        let remaining = vec![ToolRequest {
            id: "backend".to_string(),
            tool_call: Ok(rmcp::model::CallToolRequestParams {
                name: "backend_tool".to_string().into(),
                arguments: Some(rmcp::object!({})),
                meta: None,
                task: None,
            }),
            thought_signature: None,
        }];

        let plan = ToolResponsePlan::new(&frontend, &remaining);
        let ordered_ids = plan
            .ordered_pairs()
            .into_iter()
            .map(|(request, _)| request.id)
            .collect::<Vec<_>>();

        assert_eq!(
            ordered_ids,
            vec!["frontend".to_string(), "backend".to_string()]
        );
        assert!(plan.response_message_for("frontend").is_some());
        assert!(plan.response_message_for("backend").is_some());
    }

    #[test]
    fn classify_tool_transport_kind_distinguishes_server_local_from_external_frontend() {
        let request = ToolRequest {
            id: "front".to_string(),
            tool_call: Ok(rmcp::model::CallToolRequestParams {
                name: "developer__write_file".to_string().into(),
                arguments: Some(rmcp::object!({})),
                meta: None,
                task: None,
            }),
            thought_signature: None,
        };

        assert_eq!(
            classify_tool_transport_kind(
                &request,
                &RuntimeToolMeta {
                    name: "developer__write_file".to_string(),
                    transport: ToolTransportKind::ExternalFrontend,
                    execution_mode: ToolExecutionMode::StatefulSerial,
                    concurrency_policy: ConcurrencyPolicy::Static(
                        ToolExecutionMode::StatefulSerial,
                    ),
                    interrupt_behavior: InterruptBehavior::Block,
                    requires_permission: true,
                    supports_notifications: false,
                    supports_task_progress: false,
                    supports_streaming_result: false,
                    supports_child_tasks: false,
                    requires_explicit_scope: true,
                    max_result_chars: Some(1000),
                    result_budget_bucket: ToolResultBudgetBucket::Small,
                    is_frontend: true,
                    is_subagent: false,
                    is_final_output: false,
                },
                &["developer__write_file".to_string()]
            ),
            ToolTransportKind::ServerLocal
        );
        assert_eq!(
            classify_tool_transport_kind(
                &request,
                &RuntimeToolMeta {
                    name: "frontend__pick_file".to_string(),
                    transport: ToolTransportKind::ExternalFrontend,
                    execution_mode: ToolExecutionMode::StatefulSerial,
                    concurrency_policy: ConcurrencyPolicy::Static(
                        ToolExecutionMode::StatefulSerial,
                    ),
                    interrupt_behavior: InterruptBehavior::Block,
                    requires_permission: true,
                    supports_notifications: false,
                    supports_task_progress: false,
                    supports_streaming_result: false,
                    supports_child_tasks: false,
                    requires_explicit_scope: true,
                    max_result_chars: Some(1000),
                    result_budget_bucket: ToolResultBudgetBucket::Small,
                    is_frontend: true,
                    is_subagent: false,
                    is_final_output: false,
                },
                &[]
            ),
            ToolTransportKind::ExternalFrontend
        );
        assert_eq!(
            classify_tool_transport_kind(
                &request,
                &RuntimeToolMeta {
                    name: "developer__shell".to_string(),
                    transport: ToolTransportKind::WorkerLocal,
                    execution_mode: ToolExecutionMode::SerialMutating,
                    concurrency_policy: ConcurrencyPolicy::Static(
                        ToolExecutionMode::SerialMutating,
                    ),
                    interrupt_behavior: InterruptBehavior::Block,
                    requires_permission: true,
                    supports_notifications: true,
                    supports_task_progress: false,
                    supports_streaming_result: false,
                    supports_child_tasks: false,
                    requires_explicit_scope: true,
                    max_result_chars: Some(1000),
                    result_budget_bucket: ToolResultBudgetBucket::Small,
                    is_frontend: false,
                    is_subagent: false,
                    is_final_output: false,
                },
                &[]
            ),
            ToolTransportKind::WorkerLocal
        );
    }

    #[test]
    fn invocation_surface_tracks_execute_and_worker_contexts() {
        assert_eq!(
            tool_invocation_surface(HarnessMode::Conversation, false),
            ToolInvocationSurface::Conversation
        );
        assert_eq!(
            tool_invocation_surface(HarnessMode::Execute, false),
            ToolInvocationSurface::ExecuteHost
        );
        assert_eq!(
            tool_invocation_surface(HarnessMode::Execute, true),
            ToolInvocationSurface::Worker
        );
    }

    #[test]
    fn partition_frontend_transports_separates_server_local_and_external() {
        let server_local = ScheduledToolCall {
            request: ToolRequest {
                id: "server-local".to_string(),
                tool_call: Ok(rmcp::model::CallToolRequestParams {
                    name: "developer__write_file".to_string().into(),
                    arguments: Some(rmcp::object!({})),
                    meta: None,
                    task: None,
                }),
                thought_signature: None,
            },
            meta: RuntimeToolMeta {
                name: "developer__write_file".to_string(),
                transport: ToolTransportKind::ExternalFrontend,
                execution_mode: ToolExecutionMode::StatefulSerial,
                concurrency_policy: ConcurrencyPolicy::Static(ToolExecutionMode::StatefulSerial),
                interrupt_behavior: InterruptBehavior::Block,
                requires_permission: true,
                supports_notifications: false,
                supports_task_progress: false,
                supports_streaming_result: false,
                supports_child_tasks: false,
                requires_explicit_scope: true,
                max_result_chars: Some(1000),
                result_budget_bucket: ToolResultBudgetBucket::Small,
                is_frontend: true,
                is_subagent: false,
                is_final_output: false,
            },
        };
        let external = ScheduledToolCall {
            request: ToolRequest {
                id: "external".to_string(),
                tool_call: Ok(rmcp::model::CallToolRequestParams {
                    name: "frontend__pick_file".to_string().into(),
                    arguments: Some(rmcp::object!({})),
                    meta: None,
                    task: None,
                }),
                thought_signature: None,
            },
            meta: server_local.meta.clone(),
        };

        let partition = partition_frontend_transports(
            vec![server_local.clone(), external.clone()],
            &["developer__write_file".to_string()],
        );

        assert_eq!(partition.server_local.len(), 1);
        assert_eq!(partition.server_local[0].request.id, "server-local");
        assert_eq!(partition.external_frontend.len(), 1);
        assert_eq!(partition.external_frontend[0].request.id, "external");
    }

    #[test]
    fn transport_dispatcher_preserves_three_transport_kinds() {
        let frontend_meta = RuntimeToolMeta {
            name: "developer__write_file".to_string(),
            transport: ToolTransportKind::ExternalFrontend,
            execution_mode: ToolExecutionMode::StatefulSerial,
            concurrency_policy: ConcurrencyPolicy::Static(ToolExecutionMode::StatefulSerial),
            interrupt_behavior: InterruptBehavior::Block,
            requires_permission: true,
            supports_notifications: false,
            supports_task_progress: false,
            supports_streaming_result: false,
            supports_child_tasks: false,
            requires_explicit_scope: true,
            max_result_chars: Some(1000),
            result_budget_bucket: ToolResultBudgetBucket::Small,
            is_frontend: true,
            is_subagent: false,
            is_final_output: false,
        };
        let worker_meta = RuntimeToolMeta {
            name: "developer__shell".to_string(),
            transport: ToolTransportKind::WorkerLocal,
            is_frontend: false,
            ..frontend_meta.clone()
        };
        let dispatch = TransportDispatcher::dispatch(
            vec![
                ScheduledToolCall {
                    request: ToolRequest {
                        id: "server-local".to_string(),
                        tool_call: Ok(rmcp::model::CallToolRequestParams {
                            name: "developer__write_file".to_string().into(),
                            arguments: Some(rmcp::object!({})),
                            meta: None,
                            task: None,
                        }),
                        thought_signature: None,
                    },
                    meta: frontend_meta.clone(),
                },
                ScheduledToolCall {
                    request: ToolRequest {
                        id: "external".to_string(),
                        tool_call: Ok(rmcp::model::CallToolRequestParams {
                            name: "frontend__pick_file".to_string().into(),
                            arguments: Some(rmcp::object!({})),
                            meta: None,
                            task: None,
                        }),
                        thought_signature: None,
                    },
                    meta: frontend_meta,
                },
                ScheduledToolCall {
                    request: ToolRequest {
                        id: "worker-local".to_string(),
                        tool_call: Ok(rmcp::model::CallToolRequestParams {
                            name: "developer__shell".to_string().into(),
                            arguments: Some(rmcp::object!({})),
                            meta: None,
                            task: None,
                        }),
                        thought_signature: None,
                    },
                    meta: worker_meta,
                },
            ],
            ToolInvocationSurface::ExecuteHost,
            &["developer__write_file".to_string()],
        );

        assert_eq!(dispatch.dispatches.len(), 3);
        assert!(dispatch.dispatches.iter().any(|item| {
            item.call.request.id == "server-local"
                && item.transport == ToolTransportKind::ServerLocal
        }));
        assert!(dispatch.dispatches.iter().any(|item| {
            item.call.request.id == "external"
                && item.transport == ToolTransportKind::ExternalFrontend
        }));
        assert!(dispatch.dispatches.iter().any(|item| {
            item.call.request.id == "worker-local"
                && item.transport == ToolTransportKind::WorkerLocal
        }));
    }
}
