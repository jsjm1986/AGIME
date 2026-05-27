//! Standard tool-call dispatcher.
//!
//! Migrated from `agime-team-server::agent::tool_dispatch`. The team server
//! now wraps this function with adapters that implement
//! [`ToolDispatchExtensionState`] and [`ToolDispatchProgressSink`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use agime::agents::tool_execution_cancelled_error_text;
use anyhow::anyhow;
use tokio_util::sync::CancellationToken;

use crate::tool_content::{ToolContentBlock, ToolTaskProgress, ToolTaskProgressCallback};
use crate::traits::{ToolDispatchExtensionState, ToolDispatchProgressSink};

/// Run a single tool call against the active extension state.
///
/// Returns `(duration_ms, Ok(blocks) | Err(error_text))`. The error text uses
/// the canonical `[step:tool_*]` tags so the agent loop can classify the
/// failure (parameter schema vs execution vs cancellation vs timeout).
pub async fn execute_standard_tool_call(
    dynamic_state: Arc<dyn ToolDispatchExtensionState>,
    progress_sink: Arc<dyn ToolDispatchProgressSink>,
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

    let call_tool = |dynamic_state: Arc<dyn ToolDispatchExtensionState>,
                     progress_sink: Arc<dyn ToolDispatchProgressSink>,
                     cancel_token: CancellationToken,
                     name: String,
                     args: serde_json::Value| async move {
        if dynamic_state.platform_can_handle(&name).await {
            return dynamic_state.platform_call_tool(&name, args).await;
        }

        let progress_cb: ToolTaskProgressCallback = {
            let progress_sink = progress_sink.clone();
            Arc::new(move |p: ToolTaskProgress| {
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
                let sink = progress_sink.clone();
                tokio::spawn(async move {
                    sink.on_progress(payload).await;
                });
            })
        };

        match dynamic_state
            .mcp_call_tool(&name, args, Some(progress_cb), cancel_token)
            .await
        {
            Some(result) => result,
            None => Err(anyhow!("No handler for tool: {}", name)),
        }
    };

    let result = if let Some(timeout_secs) = tool_timeout_secs {
        tokio::select! {
            _ = cancel_token.cancelled() => Err(tool_execution_cancelled_error_text(&name)),
            outcome = tokio::time::timeout(
                Duration::from_secs(timeout_secs),
                call_tool(
                    dynamic_state,
                    progress_sink,
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
                progress_sink,
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
