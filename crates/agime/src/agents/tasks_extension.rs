use crate::agents::extension::PlatformExtensionContext;
use crate::agents::harness::runtime_state::HarnessWorkerRuntimeState;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::task_board::{
    TaskBoardContext, TaskBoardMutation, TaskBoardScope, TaskBoardStore, TaskBoardUpdate,
};
use crate::session::extension_data::{TaskItem, TaskProjectionEvent, TaskStatus};
use crate::session::{ExtensionState, SessionManager};
use anyhow::Result;
use async_trait::async_trait;
use indoc::indoc;
use rmcp::model::{
    CallToolResult, Content, GetPromptResult, Implementation, InitializeResult, JsonObject,
    ListPromptsResult, ListResourcesResult, ListToolsResult, ProtocolVersion, ReadResourceResult,
    ServerCapabilities, ServerNotification, Tool, ToolAnnotations, ToolsCapability,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "tasks";

const TOOL_TASK_CREATE: &str = "TaskCreate";
const TOOL_TASK_UPDATE: &str = "TaskUpdate";
const TOOL_TASK_LIST: &str = "TaskList";
const TOOL_TASK_GET: &str = "TaskGet";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TaskCreateParams {
    subject: String,
    description: String,
    active_form: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<HashMap<String, Value>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Default)]
struct TaskUpdateParams {
    task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_form: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<TaskStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    add_blocks: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    add_blocked_by: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<HashMap<String, Value>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TaskGetParams {
    task_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TaskMutationResult {
    board_id: String,
    scope: TaskBoardScope,
    task: TaskItem,
    task_count: usize,
    in_progress_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    projection: Option<TaskProjectionEvent>,
}

pub struct TasksClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
}

impl TasksClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {
                    list_changed: Some(false),
                }),
                resources: None,
                prompts: None,
                completions: None,
                experimental: None,
                extensions: None,
                logging: None,
                tasks: None,
            },
            server_info: Implementation {
                name: EXTENSION_NAME.to_string(),
                title: Some("Tasks".to_string()),
                version: "2.0.0".to_string(),
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(indoc! {r#"
                Structured Task Management

                Use TaskCreate / TaskUpdate / TaskList / TaskGet to manage the task board for the current session.
                This replaces the old text TODO workflow.

                Rules:
                - Create tasks proactively for multi-step work
                - Keep task descriptions specific and actionable
                - active_form must describe the task in present continuous form
                - Maintain at most one task in_progress at a time
                - Update status immediately when work changes
                - Do not mark tasks completed when work is partial or blocked
                - Worker sessions maintain local task truth; the leader board receives projected shadow tasks
            "#}.to_string()),
        };

        Ok(Self { info, context })
    }

    async fn task_board_context(&self) -> Result<TaskBoardContext, String> {
        if let Some(session_id) = self.context.session_id.as_ref() {
            if let Ok(session) =
                crate::session::SessionManager::get_session(session_id, false).await
            {
                let worker_runtime =
                    HarnessWorkerRuntimeState::from_extension_data(&session.extension_data);
                return Ok(TaskBoardContext::from_runtime(
                    session_id.clone(),
                    worker_runtime.as_ref(),
                ));
            }
        }
        self.context
            .task_board_context
            .clone()
            .ok_or_else(|| "Tasks require a bound session".to_string())
    }

    fn render_tasks_markdown(items: &[TaskItem], board_id: &str) -> String {
        if items.is_empty() {
            return "No active tasks.".to_string();
        }
        let mut lines = vec![format!("Task board: {}", board_id)];
        for item in items {
            let status = match item.status {
                TaskStatus::Pending => "pending",
                TaskStatus::InProgress => "in_progress",
                TaskStatus::Completed => "completed",
            };
            let owner = item
                .owner
                .as_deref()
                .map(|value| format!(" owner={}", value))
                .unwrap_or_default();
            lines.push(format!(
                "- [{}] #{} {}{}",
                status, item.id, item.subject, owner
            ));
        }
        lines.join("\n")
    }

    fn current_worker_owner(runtime: Option<&HarnessWorkerRuntimeState>) -> Option<String> {
        runtime
            .and_then(|value| value.logical_worker_id.clone())
            .or_else(|| runtime.and_then(|value| value.worker_name.clone()))
    }

    fn mutation_result(mutation: &TaskBoardMutation) -> TaskMutationResult {
        TaskMutationResult {
            board_id: mutation.snapshot.board_id.0.clone(),
            scope: mutation.snapshot.scope.into(),
            task: mutation.task.clone(),
            task_count: mutation.snapshot.items.len(),
            in_progress_count: mutation
                .snapshot
                .items
                .iter()
                .filter(|item| item.status == TaskStatus::InProgress)
                .count(),
            projection: mutation.projection.clone(),
        }
    }

    fn success_with_structure(message: String, structured: Value) -> CallToolResult {
        let mut result = CallToolResult::success(vec![Content::text(message)]);
        result.structured_content = Some(structured);
        result
    }

    async fn handle_task_create(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let params: TaskCreateParams =
            serde_json::from_value(Value::Object(arguments.ok_or("Missing arguments")?))
                .map_err(|e| format!("Invalid TaskCreate arguments: {}", e))?;

        if params.subject.trim().is_empty()
            || params.description.trim().is_empty()
            || params.active_form.trim().is_empty()
        {
            return Err("subject, description and active_form are required".to_string());
        }
        let board_context = self.task_board_context().await?;
        let owner = if self.context.task_board_context.is_some() {
            board_context.logical_worker_id.clone()
        } else if let Some(session_id) = self.context.session_id.as_deref() {
            let runtime = SessionManager::get_session(session_id, false)
                .await
                .ok()
                .and_then(|session| {
                    HarnessWorkerRuntimeState::from_extension_data(&session.extension_data)
                });
            Self::current_worker_owner(runtime.as_ref())
        } else {
            None
        };
        let mutation = TaskBoardStore::create_task(
            &board_context,
            params.subject.trim().to_string(),
            params.description.trim().to_string(),
            params.active_form.trim().to_string(),
            owner,
            params.metadata.unwrap_or_default(),
        )
        .await?;
        Ok(Self::success_with_structure(
            format!(
                "Task #{} created: {}",
                mutation.task.id, mutation.task.subject
            ),
            json!(Self::mutation_result(&mutation)),
        ))
    }

    async fn handle_task_update(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let params: TaskUpdateParams =
            serde_json::from_value(Value::Object(arguments.ok_or("Missing arguments")?))
                .map_err(|e| format!("Invalid TaskUpdate arguments: {}", e))?;

        let task_id = params.task_id.trim().to_string();
        if task_id.is_empty() {
            return Err("task_id is required".to_string());
        }

        let board_context = self.task_board_context().await?;
        let owner = if let Some(owner) = params.owner.as_deref() {
            if owner.trim().is_empty() {
                None
            } else {
                Some(owner.trim().to_string())
            }
        } else {
            board_context.logical_worker_id.clone()
        };
        let mutation = TaskBoardStore::update_task(
            &board_context,
            TaskBoardUpdate {
                task_id,
                subject: params.subject.map(|value| value.trim().to_string()),
                description: params.description.map(|value| value.trim().to_string()),
                active_form: params.active_form.map(|value| value.trim().to_string()),
                owner,
                status: params.status,
                add_blocks: params.add_blocks,
                add_blocked_by: params.add_blocked_by,
                metadata: params.metadata,
            },
        )
        .await?;
        Ok(Self::success_with_structure(
            format!(
                "Task #{} updated: {}",
                mutation.task.id, mutation.task.subject
            ),
            json!(Self::mutation_result(&mutation)),
        ))
    }

    async fn handle_task_list(&self) -> Result<CallToolResult, String> {
        let board_context = self.task_board_context().await?;
        let snapshot = TaskBoardStore::list(&board_context).await?;
        Ok(Self::success_with_structure(
            Self::render_tasks_markdown(&snapshot.items, snapshot.board_id.as_str()),
            json!({
                "board_id": snapshot.board_id,
                "scope": snapshot.scope,
                "items": snapshot.items,
            }),
        ))
    }

    async fn handle_task_get(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let params: TaskGetParams =
            serde_json::from_value(Value::Object(arguments.ok_or("Missing arguments")?))
                .map_err(|e| format!("Invalid TaskGet arguments: {}", e))?;

        let task_id = params.task_id.trim();
        if task_id.is_empty() {
            return Err("task_id is required".to_string());
        }

        let board_context = self.task_board_context().await?;
        let (snapshot, task) = TaskBoardStore::get(&board_context, task_id).await?;
        let task = task.ok_or_else(|| format!("Task {} not found", task_id))?;
        Ok(Self::success_with_structure(
            format!("Task #{}: {}", task.id, task.subject),
            json!({
                "board_id": snapshot.board_id,
                "scope": snapshot.scope,
                "task": task,
            }),
        ))
    }

    fn get_tools() -> Vec<Tool> {
        let create_schema = serde_json::to_value(schema_for!(TaskCreateParams))
            .expect("Failed to serialize TaskCreateParams schema");
        let update_schema = serde_json::to_value(schema_for!(TaskUpdateParams))
            .expect("Failed to serialize TaskUpdateParams schema");
        let get_schema = serde_json::to_value(schema_for!(TaskGetParams))
            .expect("Failed to serialize TaskGetParams schema");

        vec![
            Tool::new(
                TOOL_TASK_CREATE.to_string(),
                "Create a structured task in the current session task board.".to_string(),
                create_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("Create Task".to_string()),
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
                open_world_hint: Some(false),
            }),
            Tool::new(
                TOOL_TASK_UPDATE.to_string(),
                "Update a task in the current session task board.".to_string(),
                update_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("Update Task".to_string()),
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
                open_world_hint: Some(false),
            }),
            Tool::new(
                TOOL_TASK_LIST.to_string(),
                "List the current structured tasks for this session.".to_string(),
                JsonObject::new(),
            )
            .annotate(ToolAnnotations {
                title: Some("List Tasks".to_string()),
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            }),
            Tool::new(
                TOOL_TASK_GET.to_string(),
                "Get one structured task by id.".to_string(),
                get_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("Get Task".to_string()),
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            }),
        ]
    }
}

#[async_trait]
impl McpClientTrait for TasksClient {
    async fn list_resources(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListResourcesResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn read_resource(
        &self,
        _uri: &str,
        _cancellation_token: CancellationToken,
    ) -> Result<ReadResourceResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn list_tools(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        _cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let result = match name {
            TOOL_TASK_CREATE => self.handle_task_create(arguments).await,
            TOOL_TASK_UPDATE => self.handle_task_update(arguments).await,
            TOOL_TASK_LIST => self.handle_task_list().await,
            TOOL_TASK_GET => self.handle_task_get(arguments).await,
            _ => Err(format!("Unknown tool: {}", name)),
        };

        match result {
            Ok(content) => Ok(content),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {}",
                error
            ))])),
        }
    }

    async fn list_tasks(
        &self,
        _cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<rmcp::model::ListTasksResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn get_task_info(
        &self,
        _task_id: &str,
        _cancellation_token: CancellationToken,
    ) -> Result<rmcp::model::GetTaskInfoResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn get_task_result(
        &self,
        _task_id: &str,
        _cancellation_token: CancellationToken,
    ) -> Result<rmcp::model::TaskResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn cancel_task(
        &self,
        _task_id: &str,
        _cancellation_token: CancellationToken,
    ) -> Result<(), Error> {
        Err(Error::TransportClosed)
    }

    async fn list_prompts(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListPromptsResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn get_prompt(
        &self,
        _name: &str,
        _arguments: Value,
        _cancellation_token: CancellationToken,
    ) -> Result<GetPromptResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        mpsc::channel(1).1
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }

    async fn get_moim(&self) -> Option<String> {
        let board_context = self.task_board_context().await.ok()?;
        let snapshot = TaskBoardStore::list(&board_context).await.ok()?;
        if snapshot.items.is_empty() {
            return None;
        }
        Some(format!(
            "Current tasks:\n{}\n",
            Self::render_tasks_markdown(&snapshot.items, snapshot.board_id.as_str())
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::harness::{
        save_worker_runtime_state, CoordinatorRole, HarnessWorkerRuntimeState,
    };
    use crate::session::SessionType;

    async fn test_client() -> TasksClient {
        let session = SessionManager::create_session(
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            "tasks-test".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create session");
        TasksClient::new(PlatformExtensionContext {
            session_id: Some(session.id.clone()),
            task_board_context: Some(TaskBoardContext::standalone(session.id)),
            extension_manager: None,
            tool_route_manager: None,
        })
        .expect("client")
    }

    #[tokio::test]
    async fn task_create_and_list_work_in_bound_session() {
        let client = test_client().await;
        let created = client
            .handle_task_create(Some(
                serde_json::json!({
                    "subject": "Implement Tasks V2",
                    "description": "Replace legacy tasks flow",
                    "active_form": "Implementing Tasks V2"
                })
                .as_object()
                .cloned()
                .unwrap(),
            ))
            .await
            .expect("create");
        assert!(created.structured_content.is_some());

        let listed = client.handle_task_list().await.expect("list");
        let structured = listed.structured_content.expect("structured");
        let items = structured["items"].as_array().expect("items");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["subject"], "Implement Tasks V2");
        assert_eq!(items[0]["status"], "pending");
    }

    #[tokio::test]
    async fn task_update_moves_single_item_to_in_progress() {
        let client = test_client().await;
        client
            .handle_task_create(Some(
                serde_json::json!({
                    "subject": "Step one",
                    "description": "First step",
                    "active_form": "Working on step one"
                })
                .as_object()
                .cloned()
                .unwrap(),
            ))
            .await
            .expect("create one");
        client
            .handle_task_create(Some(
                serde_json::json!({
                    "subject": "Step two",
                    "description": "Second step",
                    "active_form": "Working on step two"
                })
                .as_object()
                .cloned()
                .unwrap(),
            ))
            .await
            .expect("create two");

        client
            .handle_task_update(Some(
                serde_json::json!({
                    "task_id": "2",
                    "status": "in_progress"
                })
                .as_object()
                .cloned()
                .unwrap(),
            ))
            .await
            .expect("update");

        let listed = client.handle_task_list().await.expect("list");
        let items = listed.structured_content.expect("structured")["items"]
            .as_array()
            .cloned()
            .expect("items");
        assert_eq!(items[0]["status"], "pending");
        assert_eq!(items[1]["status"], "in_progress");
    }

    #[tokio::test]
    async fn worker_runtime_session_derives_worker_task_board_context_without_explicit_context() {
        let session = SessionManager::create_session(
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            "tasks-worker-runtime".to_string(),
            SessionType::SubAgent,
        )
        .await
        .expect("create worker session");

        save_worker_runtime_state(
            &session.id,
            &HarnessWorkerRuntimeState {
                task_board_session_id: Some("leader-session-1".to_string()),
                swarm_run_id: Some("swarm-run-1".to_string()),
                worker_name: Some("worker-a".to_string()),
                leader_name: Some("leader".to_string()),
                logical_worker_id: Some("worker-a".to_string()),
                attempt_id: Some("attempt-1".to_string()),
                attempt_index: Some(0),
                previous_task_id: None,
                coordinator_role: CoordinatorRole::Worker,
                mailbox_dir: None,
                permission_dir: None,
                scratchpad_dir: None,
                enable_permission_bridge: false,
                allow_worker_messaging: false,
                peer_worker_addresses: Vec::new(),
                peer_worker_catalog: Vec::new(),
                validation_mode: false,
            },
        )
        .await
        .expect("save runtime state");

        let client = TasksClient::new(PlatformExtensionContext {
            session_id: Some(session.id.clone()),
            task_board_context: None,
            extension_manager: None,
            tool_route_manager: None,
        })
        .expect("client");

        let context = client
            .task_board_context()
            .await
            .expect("task board context");
        assert_eq!(context.board_id.as_str(), "leader-session-1:worker-a");
        assert_eq!(context.storage_session_id, "leader-session-1:worker-a");
        assert_eq!(
            context.projection_target.as_deref(),
            Some("leader-session-1")
        );
        assert_eq!(context.logical_worker_id.as_deref(), Some("worker-a"));
        assert_eq!(context.scope, TaskBoardScope::Worker);
    }

    #[tokio::test]
    async fn explicit_task_board_context_is_used_when_session_id_is_not_a_core_session() {
        let client = TasksClient::new(PlatformExtensionContext {
            session_id: Some("logical-chat-session".to_string()),
            task_board_context: Some(TaskBoardContext::standalone("logical-chat-session")),
            extension_manager: None,
            tool_route_manager: None,
        })
        .expect("client");

        let context = client
            .task_board_context()
            .await
            .expect("task board context");
        assert_eq!(context.board_id.as_str(), "logical-chat-session");
        assert_eq!(context.storage_session_id, "logical-chat-session");
        assert_eq!(context.scope, TaskBoardScope::Standalone);
    }
}
