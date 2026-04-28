use agime::agents::tool_execution_cancelled_error_text;
use anyhow::anyhow;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use super::extension_manager_client::DynamicExtensionState;
use super::mcp_connector::{ToolContentBlock, ToolTaskProgressCallback};
use super::task_manager::{StreamEvent, TaskManager};

pub(crate) async fn execute_standard_tool_call(
    dynamic_state: Arc<RwLock<DynamicExtensionState>>,
    task_manager: Arc<TaskManager>,
    task_id: String,
    cancel_token: CancellationToken,
    tool_timeout_secs: Option<u64>,
    name: String,
    args: serde_json::Value,
) -> (u64, Result<Vec<ToolContentBlock>, String>) {
    fn tag_tool_error(error: String) -> String {
        let lower = error.to_ascii_lowercase();
        if lower.contains("failed to deserialize parameters")
            || lower.contains("unknown variant")
            || lower.contains("missing field")
            || lower.contains("invalid type")
        {
            format!("[step:tool_parameter_schema] {}", error)
        } else {
            format!("[step:tool_execution] {}", error)
        }
    }

    let started_at = Instant::now();
    if cancel_token.is_cancelled() {
        return (
            started_at.elapsed().as_millis() as u64,
            Err(tool_execution_cancelled_error_text(&name)),
        );
    }

    let call_tool = |dynamic_state: Arc<RwLock<DynamicExtensionState>>,
                     task_manager: Arc<TaskManager>,
                     task_id: String,
                     cancel_token: CancellationToken,
                     name: String,
                     args: serde_json::Value| async move {
        let state = dynamic_state.read().await;
        if state.platform.can_handle(&name) {
            return state.platform.call_tool_rich(&name, args).await;
        }
        if let Some(ref mcp) = state.mcp {
            let progress_task_id = task_id.clone();
            let progress_mgr = task_manager.clone();
            let progress_cb: ToolTaskProgressCallback = Arc::new(move |p| {
                let payload = serde_json::json!({
                    "type": "tool_task_progress",
                    "tool_name": p.tool_name,
                    "server_name": p.server_name,
                    "task_id": p.task_id,
                    "status": p.status,
                    "status_message": p.status_message,
                    "poll_count": p.poll_count,
                })
                .to_string();
                let tm = progress_mgr.clone();
                let tid = progress_task_id.clone();
                tokio::spawn(async move {
                    tm.broadcast(&tid, StreamEvent::Status { status: payload })
                        .await;
                });
            });
            return mcp
                .call_tool_rich_with_progress(&name, args, Some(progress_cb), cancel_token)
                .await;
        }
        Err(anyhow!("No handler for tool: {}", name))
    };

    let result = if let Some(timeout_secs) = tool_timeout_secs {
        tokio::select! {
            _ = cancel_token.cancelled() => Err(tool_execution_cancelled_error_text(&name)),
            outcome = tokio::time::timeout(
                Duration::from_secs(timeout_secs),
                call_tool(
                    dynamic_state,
                    task_manager,
                    task_id,
                    cancel_token.clone(),
                    name.clone(),
                    args,
                ),
            ) => {
                match outcome {
                    Ok(Ok(blocks)) => Ok(blocks),
                    Ok(Err(error)) => Err(tag_tool_error(format!("Error: {}", error))),
                    Err(_) => Err(format!(
                        "[step:tool_execution] Error: tool '{}' timed out after {}s",
                        name, timeout_secs
                    )),
                }
            }
        }
    } else {
        tokio::select! {
            _ = cancel_token.cancelled() => Err(tool_execution_cancelled_error_text(&name)),
            outcome = call_tool(
                dynamic_state,
                task_manager,
                task_id,
                cancel_token.clone(),
                name.clone(),
                args,
            ) => outcome.map_err(|error| tag_tool_error(format!("Error: {}", error)))
        }
    };

    let duration_ms = started_at.elapsed().as_millis() as u64;
    let result = if cancel_token.is_cancelled() && result.is_ok() {
        Err(tool_execution_cancelled_error_text(&name))
    } else {
        result
    };
    match result {
        Ok(blocks) => (duration_ms, Ok(blocks)),
        Err(err) => {
            tracing::warn!("{}", err);
            (duration_ms, Err(err))
        }
    }
}
