//! Traits the runtime layer asks the surface servers to implement.
//!
//! Each batch of the migration adds traits to this module. For batch 1 we only
//! introduce the minimal seam needed by the standard tool-call dispatcher.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::tool_content::{ToolContentBlock, ToolTaskProgressCallback};

/// Sink used by the dispatcher to forward MCP tool task progress events back
/// to the active stream (chat / channel / task / silent).
#[async_trait]
pub trait ToolDispatchProgressSink: Send + Sync {
    /// Called from the MCP progress callback. Implementors decide how to
    /// surface the payload to the running stream (broadcast on a channel,
    /// log only, drop, ...).
    async fn on_progress(&self, payload_json: String);
}

/// Extension state seen by [`crate::tool_dispatch::execute_standard_tool_call`].
///
/// The dispatcher only needs the ability to:
/// 1. ask whether a platform extension can handle a given tool name,
/// 2. invoke a platform extension by name, and
/// 3. invoke an MCP tool with a progress callback + cancellation token.
///
/// Surface servers (team / desktop) provide their own concrete state object —
/// usually wrapped in `Arc<RwLock<...>>` — and implement this trait on it.
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
