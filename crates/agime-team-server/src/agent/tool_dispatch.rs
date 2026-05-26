//! Standard tool-call dispatcher (team-server side).
//!
//! The actual dispatcher lives in [`agime_runtime::tool_dispatch`]; this file
//! adapts the team-server-specific runtime types (`DynamicExtensionState`,
//! `TaskManager`, the team-side `ToolContentBlock`) to the runtime crate's
//! traits. The server harness keeps calling the same
//! `execute_standard_tool_call` symbol with the same argument shape — only the
//! internals were moved.

use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use agime_runtime::tool_content::{
    ToolContentBlock as RtToolContentBlock, ToolTaskProgress as RtToolTaskProgress,
    ToolTaskProgressCallback as RtToolTaskProgressCallback,
};
use agime_runtime::traits::{ToolDispatchExtensionState, ToolDispatchProgressSink};

use super::extension_manager_client::DynamicExtensionState;
use super::mcp_connector::{
    ToolContentBlock, ToolTaskProgress as LocalToolTaskProgress,
    ToolTaskProgressCallback as LocalToolTaskProgressCallback,
};
use super::task_manager::{StreamEvent, TaskManager};

fn rt_block_to_local(block: RtToolContentBlock) -> ToolContentBlock {
    match block {
        RtToolContentBlock::Text(text) => ToolContentBlock::Text(text),
        RtToolContentBlock::Image { mime_type, data } => {
            ToolContentBlock::Image { mime_type, data }
        }
        RtToolContentBlock::Resource {
            uri,
            mime_type,
            text,
            blob,
        } => ToolContentBlock::Resource {
            uri,
            mime_type,
            text,
            blob,
        },
        RtToolContentBlock::StructuredJson(value) => ToolContentBlock::StructuredJson(value),
    }
}

fn local_block_to_rt(block: ToolContentBlock) -> RtToolContentBlock {
    match block {
        ToolContentBlock::Text(text) => RtToolContentBlock::Text(text),
        ToolContentBlock::Image { mime_type, data } => {
            RtToolContentBlock::Image { mime_type, data }
        }
        ToolContentBlock::Resource {
            uri,
            mime_type,
            text,
            blob,
        } => RtToolContentBlock::Resource {
            uri,
            mime_type,
            text,
            blob,
        },
        ToolContentBlock::StructuredJson(value) => RtToolContentBlock::StructuredJson(value),
    }
}

struct DynamicExtensionStateAdapter(Arc<RwLock<DynamicExtensionState>>);

#[async_trait::async_trait]
impl ToolDispatchExtensionState for DynamicExtensionStateAdapter {
    async fn platform_can_handle(&self, name: &str) -> bool {
        let state = self.0.read().await;
        state.platform.can_handle(name)
    }

    async fn platform_call_tool(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> anyhow::Result<Vec<RtToolContentBlock>> {
        let state = self.0.read().await;
        let blocks = state.platform.call_tool_rich(name, args).await?;
        Ok(blocks.into_iter().map(local_block_to_rt).collect())
    }

    async fn mcp_call_tool(
        &self,
        name: &str,
        args: serde_json::Value,
        progress_cb: Option<RtToolTaskProgressCallback>,
        cancel_token: CancellationToken,
    ) -> Option<anyhow::Result<Vec<RtToolContentBlock>>> {
        let state = self.0.read().await;
        let mcp = state.mcp.as_ref()?;

        let local_cb: Option<LocalToolTaskProgressCallback> = progress_cb.map(|cb| {
            let forward: LocalToolTaskProgressCallback =
                Arc::new(move |progress: LocalToolTaskProgress| {
                    cb(RtToolTaskProgress {
                        tool_name: progress.tool_name,
                        server_name: progress.server_name,
                        task_id: progress.task_id,
                        status: progress.status,
                        status_message: progress.status_message,
                        poll_count: progress.poll_count,
                    });
                });
            forward
        });

        let result = mcp
            .call_tool_rich_with_progress(name, args, local_cb, cancel_token)
            .await
            .map(|blocks| blocks.into_iter().map(local_block_to_rt).collect());
        Some(result)
    }
}

struct TaskBroadcastSink {
    task_manager: Arc<TaskManager>,
    task_id: String,
}

#[async_trait::async_trait]
impl ToolDispatchProgressSink for TaskBroadcastSink {
    async fn on_progress(&self, payload_json: String) {
        self.task_manager
            .broadcast(
                &self.task_id,
                StreamEvent::Status {
                    status: payload_json,
                },
            )
            .await;
    }
}

pub(crate) async fn execute_standard_tool_call(
    dynamic_state: Arc<RwLock<DynamicExtensionState>>,
    task_manager: Arc<TaskManager>,
    task_id: String,
    cancel_token: CancellationToken,
    tool_timeout_secs: Option<u64>,
    name: String,
    args: serde_json::Value,
) -> (u64, Result<Vec<ToolContentBlock>, String>) {
    let ext_state: Arc<dyn ToolDispatchExtensionState> =
        Arc::new(DynamicExtensionStateAdapter(dynamic_state));
    let sink: Arc<dyn ToolDispatchProgressSink> = Arc::new(TaskBroadcastSink {
        task_manager,
        task_id,
    });

    let (duration_ms, result) = agime_runtime::execute_standard_tool_call(
        ext_state,
        sink,
        cancel_token,
        tool_timeout_secs,
        name,
        args,
    )
    .await;

    let result = result.map(|blocks| blocks.into_iter().map(rt_block_to_local).collect());
    (duration_ms, result)
}
