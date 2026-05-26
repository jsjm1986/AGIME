//! Tool-call content blocks shared between the runtime layer and the surface
//! servers. Mirrors the team-server `ToolContentBlock` definition so this
//! crate can hand back tool results without depending on team-only types.

use std::sync::Arc;

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
