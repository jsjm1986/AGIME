use std::borrow::Cow;
use std::path::PathBuf;

use futures::FutureExt;
use rmcp::model::{CallToolResult, Content, ErrorCode, ErrorData};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::agents::subagent_task_config::TaskConfig;
use crate::agents::tool_execution::ToolCallResult;

use super::coordinator::{native_swarm_tool_enabled, SwarmExecutionRequest};
use super::coordinator_runtime::execute_swarm_request;

pub const SWARM_TOOL_NAME: &str = "swarm";

#[derive(Debug, Clone, Deserialize)]
pub struct SwarmParams {
    pub instructions: String,
    pub targets: Vec<String>,
    #[serde(default)]
    pub write_scope: Vec<String>,
    #[serde(default)]
    pub result_contract: Vec<String>,
    #[serde(default)]
    pub parallelism_budget: Option<u32>,
    #[serde(default)]
    pub validation_mode: bool,
    #[serde(default = "default_summary")]
    pub summary: bool,
}

fn default_summary() -> bool {
    true
}

fn normalize_non_empty(values: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let candidate = trimmed.replace('\\', "/");
        if !normalized.iter().any(|existing| existing == &candidate) {
            normalized.push(candidate);
        }
    }
    normalized
}

pub fn create_swarm_tool() -> rmcp::model::Tool {
    let schema = json!({
        "type": "object",
        "required": ["instructions", "targets"],
        "properties": {
            "instructions": {
                "type": "string",
                "description": "Shared worker instructions describing the multi-target task."
            },
            "targets": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Bounded target artifacts or work items to parallelize."
            },
            "write_scope": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Optional bounded write scope for the swarm run."
            },
            "result_contract": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Optional result contract that workers and validators must satisfy."
            },
            "parallelism_budget": {
                "type": "integer",
                "minimum": 1,
                "maximum": 4,
                "description": "Maximum number of bounded workers to launch in parallel."
            },
            "validation_mode": {
                "type": "boolean",
                "default": false,
                "description": "Whether to run bounded validation workers after primary workers complete."
            },
            "summary": {
                "type": "boolean",
                "default": true,
                "description": "Return a concise leader summary instead of raw worker output."
            }
        }
    });

    rmcp::model::Tool::new(
        SWARM_TOOL_NAME,
        "Coordinate multiple bounded workers for a multi-target task. Use this when you have at least two concrete deliverables that can be advanced in parallel.",
        schema.as_object().cloned().unwrap_or_default(),
    )
}

pub fn handle_swarm_tool(
    params: Value,
    task_config: TaskConfig,
    working_dir: PathBuf,
    cancellation_token: Option<CancellationToken>,
) -> ToolCallResult {
    if !native_swarm_tool_enabled() {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INVALID_REQUEST,
            message: Cow::from("Native swarm tool is disabled"),
            data: None,
        }));
    }

    let parsed: SwarmParams = match serde_json::from_value(params) {
        Ok(value) => value,
        Err(error) => {
            return ToolCallResult::from(Err(ErrorData {
                code: ErrorCode::INVALID_PARAMS,
                message: Cow::from(format!("Invalid swarm parameters: {}", error)),
                data: None,
            }));
        }
    };

    if parsed.instructions.trim().is_empty() {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from("Swarm instructions cannot be empty"),
            data: None,
        }));
    }
    let normalized_targets = normalize_non_empty(parsed.targets);
    let normalized_write_scope = normalize_non_empty(parsed.write_scope);
    let normalized_result_contract = normalize_non_empty(parsed.result_contract);
    if normalized_targets.len() < 2 {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from("Swarm requires at least two non-empty targets"),
            data: None,
        }));
    }
    if task_config.current_depth >= task_config.max_depth {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INVALID_REQUEST,
            message: Cow::from(format!(
                "Swarm delegation depth {} reached max depth {}",
                task_config.current_depth, task_config.max_depth
            )),
            data: None,
        }));
    }

    let request = SwarmExecutionRequest {
        instructions: parsed.instructions,
        targets: normalized_targets,
        write_scope: normalized_write_scope,
        result_contract: normalized_result_contract,
        parallelism_budget: parsed.parallelism_budget,
        validation_mode: parsed.validation_mode,
        summary_only: parsed.summary,
    };

    ToolCallResult {
        notification_stream: None,
        result: Box::new(
            async move {
                let result =
                    execute_swarm_request(request, task_config, working_dir, cancellation_token)
                        .await
                        .map_err(|error| ErrorData {
                            code: ErrorCode::INTERNAL_ERROR,
                            message: Cow::from(error.to_string()),
                            data: None,
                        })?;
                let mut lines = vec![format!("Swarm run `{}` completed.", result.run_id)];
                if let Some(path) = result.scratchpad_path.as_deref() {
                    lines.push(format!("Scratchpad: {}", path));
                }
                if let Some(path) = result.mailbox_path.as_deref() {
                    lines.push(format!("Mailbox root: {}", path));
                }
                if let Some(message) = result.downgrade_message.as_deref() {
                    lines.push(format!("Fallback: {}", message));
                }
                if !result.worker_summaries.is_empty() {
                    lines.push("Worker summaries:".to_string());
                    lines.extend(
                        result
                            .worker_summaries
                            .iter()
                            .map(|value| format!("- {}", value)),
                    );
                }
                if !result.validation_summaries.is_empty() {
                    lines.push("Validation summaries:".to_string());
                    lines.extend(
                        result
                            .validation_summaries
                            .iter()
                            .map(|value| format!("- {}", value)),
                    );
                }
                Ok(CallToolResult {
                    content: vec![Content::text(lines.join("\n"))],
                    structured_content: Some(json!({
                        "run_id": result.run_id,
                        "produced_targets": result.produced_targets,
                        "downgraded": result.downgraded,
                        "downgrade_message": result.downgrade_message,
                        "scratchpad_path": result.scratchpad_path,
                        "mailbox_path": result.mailbox_path,
                        "validation_summary_count": result.validation_summaries.len(),
                    })),
                    is_error: Some(false),
                    meta: None,
                })
            }
            .boxed(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swarm_tool_schema_requires_targets() {
        let tool = create_swarm_tool();
        assert_eq!(tool.name.as_ref(), SWARM_TOOL_NAME);
        let required = tool
            .input_schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(required.iter().any(|value| value == "targets"));
    }
}
