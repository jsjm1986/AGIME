//! Desktop harness host — Phase 5b: standard tool dispatch.
//!
//! Combined verbatim copy of three `agime-runtime` modules:
//! - `tool_content.rs`  → `ToolContentBlock`, `ToolTaskProgress`, `ToolTaskProgressCallback`
//! - `traits.rs`        → `ToolDispatchProgressSink`, `ToolDispatchExtensionState`,
//!                        `SharedToolDispatchState`
//! - `tool_dispatch.rs` → `execute_standard_tool_call`
//!
//! Squashed into one file to keep the desktop harness host's helpers flat at
//! `crates/agime-server/src/` per the long-term maintenance strategy
//! (CLAUDE.md): no new submodule directories. Behavior is bit-for-bit
//! identical to the runtime version — same `[step:tool_*]` error tagging, same
//! cancel/timeout handling, same MCP progress callback wiring.
//!
//! All callers are gated by the `desktop_harness_host` cargo feature.
//!
//! SOURCE:
//!   - crates/agime-runtime/src/tool_content.rs at commit 961109f
//!   - crates/agime-runtime/src/traits.rs       at commit 961109f
//!   - crates/agime-runtime/src/tool_dispatch.rs at commit 961109f

#![allow(dead_code)]
// Phase-5b verbatim runtime copy: full tool-dispatch surface mirrors the
// agime-runtime contract so the future tool-interception bridge plugs in
// unchanged. Items remain dead until the bridge is wired in Milestone B.

use std::sync::Arc;
use std::time::{Duration, Instant};

use agime::agents::tool_execution_cancelled_error_text;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Tool content blocks (mirrored from agime-runtime/src/tool_content.rs)
// ---------------------------------------------------------------------------

/// A single content block produced by a tool call.
#[derive(Debug, Clone)]
pub enum ToolContentBlock {
    Text(String),
    Image {
        mime_type: String,
        data: String,
    },
    Resource {
        uri: String,
        mime_type: Option<String>,
        text: Option<String>,
        blob: Option<String>,
    },
    StructuredJson(serde_json::Value),
}

/// Long-running MCP tool task progress information.
#[derive(Debug, Clone)]
pub struct ToolTaskProgress {
    pub tool_name: String,
    pub server_name: String,
    pub task_id: String,
    pub status: String,
    pub status_message: Option<String>,
    pub poll_count: u32,
}

/// Callback invoked by the MCP layer when a long-running tool reports progress.
pub type ToolTaskProgressCallback = Arc<dyn Fn(ToolTaskProgress) + Send + Sync>;

// ---------------------------------------------------------------------------
// Dispatch traits (mirrored from agime-runtime/src/traits.rs)
// ---------------------------------------------------------------------------

/// Sink used by the dispatcher to forward MCP tool task progress events back
/// to the active stream (chat / channel / task / silent).
#[async_trait]
pub trait ToolDispatchProgressSink: Send + Sync {
    /// Called from the MCP progress callback. Implementors decide how to
    /// surface the payload to the running stream (broadcast on a channel,
    /// log only, drop, ...).
    async fn on_progress(&self, payload_json: String);
}

/// Extension state seen by [`execute_standard_tool_call`].
///
/// The dispatcher only needs the ability to:
/// 1. ask whether a platform extension can handle a given tool name,
/// 2. invoke a platform extension by name, and
/// 3. invoke an MCP tool with a progress callback + cancellation token.
///
/// The desktop side provides its own concrete state object — usually wrapped
/// in `Arc<RwLock<...>>` — and implements this trait on it.
#[async_trait]
pub trait ToolDispatchExtensionState: Send + Sync {
    async fn platform_can_handle(&self, name: &str) -> bool;

    async fn platform_call_tool(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<Vec<ToolContentBlock>>;

    /// Returns `Ok(None)` when no MCP extension is configured (the dispatcher
    /// then surfaces a "no handler" error).
    async fn mcp_call_tool(
        &self,
        name: &str,
        args: serde_json::Value,
        progress_cb: Option<ToolTaskProgressCallback>,
        cancel_token: CancellationToken,
    ) -> Option<Result<Vec<ToolContentBlock>>>;
}

/// Convenience alias matching the team-server pattern of sharing the dynamic
/// extension state behind an `Arc`.
pub type SharedToolDispatchState = Arc<dyn ToolDispatchExtensionState>;

// ---------------------------------------------------------------------------
// Standard tool-call dispatcher (mirrored from agime-runtime/src/tool_dispatch.rs)
// ---------------------------------------------------------------------------

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    struct StubProgressSink {
        events: Mutex<Vec<String>>,
    }

    impl StubProgressSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                events: Mutex::new(Vec::new()),
            })
        }

        fn count(&self) -> usize {
            self.events.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl ToolDispatchProgressSink for StubProgressSink {
        async fn on_progress(&self, payload_json: String) {
            self.events.lock().unwrap().push(payload_json);
        }
    }

    struct StubExtensionState {
        platform_handles: bool,
        platform_calls: AtomicU32,
        mcp_calls: AtomicU32,
        platform_outcome: Mutex<Option<Result<Vec<ToolContentBlock>>>>,
        mcp_outcome: Mutex<Option<Option<Result<Vec<ToolContentBlock>>>>>,
    }

    impl StubExtensionState {
        fn new() -> Self {
            Self {
                platform_handles: false,
                platform_calls: AtomicU32::new(0),
                mcp_calls: AtomicU32::new(0),
                platform_outcome: Mutex::new(None),
                mcp_outcome: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl ToolDispatchExtensionState for StubExtensionState {
        async fn platform_can_handle(&self, _name: &str) -> bool {
            self.platform_handles
        }

        async fn platform_call_tool(
            &self,
            _name: &str,
            _args: serde_json::Value,
        ) -> Result<Vec<ToolContentBlock>> {
            self.platform_calls.fetch_add(1, Ordering::SeqCst);
            self.platform_outcome
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Ok(vec![ToolContentBlock::Text("ok".to_string())]))
        }

        async fn mcp_call_tool(
            &self,
            _name: &str,
            _args: serde_json::Value,
            _progress_cb: Option<ToolTaskProgressCallback>,
            _cancel_token: CancellationToken,
        ) -> Option<Result<Vec<ToolContentBlock>>> {
            self.mcp_calls.fetch_add(1, Ordering::SeqCst);
            self.mcp_outcome
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Some(Ok(vec![ToolContentBlock::Text("mcp".to_string())])))
        }
    }

    #[tokio::test]
    async fn platform_path_short_circuits_mcp() {
        let mut state = StubExtensionState::new();
        state.platform_handles = true;
        let state = Arc::new(state);
        let sink = StubProgressSink::new();

        let (_, result) = execute_standard_tool_call(
            state.clone(),
            sink.clone(),
            CancellationToken::new(),
            None,
            "developer__shell".to_string(),
            serde_json::json!({}),
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(state.platform_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.mcp_calls.load(Ordering::SeqCst), 0);
        assert_eq!(sink.count(), 0);
    }

    #[tokio::test]
    async fn no_handler_returns_tagged_error() {
        let state = Arc::new(StubExtensionState::new());
        // Force mcp_outcome to be Some(None) → "no handler"
        *state.mcp_outcome.lock().unwrap() = Some(None);
        let sink = StubProgressSink::new();

        let (_, result) = execute_standard_tool_call(
            state,
            sink,
            CancellationToken::new(),
            None,
            "missing__tool".to_string(),
            serde_json::json!({}),
        )
        .await;

        let err = result.expect_err("no handler should produce an error");
        assert!(err.starts_with("[step:tool_execution]"));
        assert!(err.contains("No handler for tool"));
    }

    #[tokio::test]
    async fn parameter_schema_errors_are_classified() {
        let state = Arc::new(StubExtensionState::new());
        *state.mcp_outcome.lock().unwrap() = Some(Some(Err(anyhow!(
            "Failed to deserialize parameters: missing field `path`"
        ))));
        let sink = StubProgressSink::new();

        let (_, result) = execute_standard_tool_call(
            state,
            sink,
            CancellationToken::new(),
            None,
            "developer__text_editor".to_string(),
            serde_json::json!({}),
        )
        .await;

        let err = result.expect_err("schema mismatch should error");
        assert!(err.starts_with("[step:tool_parameter_schema]"));
    }

    #[tokio::test]
    async fn cancelled_before_start_returns_cancelled() {
        let state = Arc::new(StubExtensionState::new());
        let sink = StubProgressSink::new();
        let cancel = CancellationToken::new();
        cancel.cancel();

        let (_, result) = execute_standard_tool_call(
            state,
            sink,
            cancel,
            None,
            "developer__shell".to_string(),
            serde_json::json!({}),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn timeout_returns_tagged_timeout_error() {
        struct SlowState;

        #[async_trait]
        impl ToolDispatchExtensionState for SlowState {
            async fn platform_can_handle(&self, _name: &str) -> bool {
                false
            }

            async fn platform_call_tool(
                &self,
                _name: &str,
                _args: serde_json::Value,
            ) -> Result<Vec<ToolContentBlock>> {
                unreachable!()
            }

            async fn mcp_call_tool(
                &self,
                _name: &str,
                _args: serde_json::Value,
                _progress_cb: Option<ToolTaskProgressCallback>,
                _cancel_token: CancellationToken,
            ) -> Option<Result<Vec<ToolContentBlock>>> {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Some(Ok(vec![]))
            }
        }

        let state = Arc::new(SlowState);
        let sink = StubProgressSink::new();

        let (_, result) = execute_standard_tool_call(
            state,
            sink,
            CancellationToken::new(),
            Some(0),
            "slow__tool".to_string(),
            serde_json::json!({}),
        )
        .await;

        let err = result.expect_err("should time out");
        assert!(err.starts_with("[step:tool_execution]"));
        assert!(err.contains("timed out"));
    }
}
