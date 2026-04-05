use std::collections::HashMap;

use rmcp::model::Tool;

use crate::conversation::message::ToolRequest;

use super::tools::{infer_runtime_tool_meta, ScheduledToolCall, ToolExecutionMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolBatchMode {
    ConcurrentReadOnly,
    SerialMutating,
    StatefulSerial,
}

impl From<ToolExecutionMode> for ToolBatchMode {
    fn from(value: ToolExecutionMode) -> Self {
        match value {
            ToolExecutionMode::ConcurrentReadOnly => Self::ConcurrentReadOnly,
            ToolExecutionMode::SerialMutating => Self::SerialMutating,
            ToolExecutionMode::StatefulSerial => Self::StatefulSerial,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolBatch {
    pub mode: ToolBatchMode,
    pub calls: Vec<ScheduledToolCall>,
}

#[derive(Debug, Clone, Default)]
pub struct ToolBatchResult {
    pub batch_count: usize,
    pub concurrent_batch_count: usize,
    pub serial_batch_count: usize,
}

pub fn build_scheduled_tool_calls<F>(
    requests: &[ToolRequest],
    tools: &[Tool],
    mut is_frontend_tool: F,
) -> Vec<ScheduledToolCall>
where
    F: FnMut(&str) -> bool,
{
    let tool_index: HashMap<String, Tool> = tools
        .iter()
        .cloned()
        .map(|tool| (tool.name.to_string(), tool))
        .collect();

    requests
        .iter()
        .filter_map(|request| {
            let tool_call = request.tool_call.clone().ok()?;
            let name = tool_call.name.to_string();
            let tool = tool_index.get(&name)?;
            Some(ScheduledToolCall {
                request: request.clone(),
                meta: infer_runtime_tool_meta(tool, request, is_frontend_tool(&name)),
            })
        })
        .collect()
}

pub fn partition_tool_calls(calls: Vec<ScheduledToolCall>) -> (Vec<ToolBatch>, ToolBatchResult) {
    let mut batches: Vec<ToolBatch> = Vec::new();
    let mut result = ToolBatchResult::default();

    for call in calls {
        let next_mode = ToolBatchMode::from(call.meta.execution_mode);
        match batches.last_mut() {
            Some(last)
                if last.mode == next_mode
                    && matches!(next_mode, ToolBatchMode::ConcurrentReadOnly) =>
            {
                last.calls.push(call);
            }
            _ => {
                batches.push(ToolBatch {
                    mode: next_mode,
                    calls: vec![call],
                });
            }
        }
    }

    result.batch_count = batches.len();
    for batch in &batches {
        match batch.mode {
            ToolBatchMode::ConcurrentReadOnly => result.concurrent_batch_count += 1,
            ToolBatchMode::SerialMutating | ToolBatchMode::StatefulSerial => {
                result.serial_batch_count += 1
            }
        }
    }

    (batches, result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{Tool, ToolAnnotations};
    use rmcp::object;

    fn tool(name: &str, read_only: bool) -> Tool {
        Tool::new(
            name.to_string(),
            "".to_string(),
            object!({"type": "object"}),
        )
        .annotate(ToolAnnotations {
            title: None,
            read_only_hint: Some(read_only),
            destructive_hint: Some(!read_only),
            idempotent_hint: Some(read_only),
            open_world_hint: Some(false),
        })
    }

    fn request(id: &str, tool_name: &str) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(rmcp::model::CallToolRequestParams {
                name: tool_name.to_string().into(),
                arguments: Some(object!({})),
                meta: None,
                task: None,
            }),
            thought_signature: None,
        }
    }

    #[test]
    fn partitions_consecutive_read_only_tools_together() {
        let tools = vec![
            tool("read_a", true),
            tool("read_b", true),
            tool("write_c", false),
        ];
        let calls = build_scheduled_tool_calls(
            &[
                request("1", "read_a"),
                request("2", "read_b"),
                request("3", "write_c"),
            ],
            &tools,
            |_| false,
        );
        let (batches, result) = partition_tool_calls(calls);
        assert_eq!(batches.len(), 2);
        assert_eq!(result.concurrent_batch_count, 1);
        assert_eq!(result.serial_batch_count, 1);
        assert_eq!(batches[0].calls.len(), 2);
        assert_eq!(batches[1].calls.len(), 1);
    }
}
