use agime::agents::ExecutionHostCompletionReport;
use serde_json::Value;
use tracing::info;

use super::harness_core::{HookEventKind, HookPayload, HookSpec};
use super::workspace_runtime;

fn normalized_scope(scope: &[String]) -> Vec<String> {
    scope
        .iter()
        .filter_map(|item| workspace_runtime::normalize_relative_workspace_path(item))
        .collect()
}

fn normalized_editor_path(args: &Value) -> Option<String> {
    args.get("path")
        .and_then(Value::as_str)
        .and_then(workspace_runtime::normalize_relative_workspace_path)
}

pub fn builtin_hook_specs() -> Vec<HookSpec> {
    vec![
        HookSpec {
            hook_id: "builtin:pre_tool_use".to_string(),
            event: HookEventKind::PreToolUse,
            description: Some(
                "Enforce deterministic tool policies such as bounded subagent write scopes."
                    .to_string(),
            ),
            blocking: true,
            write_scope: Vec::new(),
            required_tools: vec!["subagent".to_string()],
            enabled: true,
        },
        HookSpec {
            hook_id: "builtin:post_tool_use".to_string(),
            event: HookEventKind::PostToolUse,
            description: Some(
                "Emit structured payloads after successful tool execution.".to_string(),
            ),
            blocking: false,
            write_scope: Vec::new(),
            required_tools: Vec::new(),
            enabled: true,
        },
        HookSpec {
            hook_id: "builtin:post_tool_use_failure".to_string(),
            event: HookEventKind::PostToolUseFailure,
            description: Some("Emit structured payloads after failed tool execution.".to_string()),
            blocking: false,
            write_scope: Vec::new(),
            required_tools: Vec::new(),
            enabled: true,
        },
        HookSpec {
            hook_id: "builtin:subagent_stop".to_string(),
            event: HookEventKind::SubagentStop,
            description: Some(
                "Emit structured payloads when bounded subagents settle.".to_string(),
            ),
            blocking: false,
            write_scope: Vec::new(),
            required_tools: vec!["subagent".to_string()],
            enabled: true,
        },
        HookSpec {
            hook_id: "builtin:run_settle".to_string(),
            event: HookEventKind::RunSettle,
            description: Some(
                "Emit structured payloads when a harness run reaches canonical completion."
                    .to_string(),
            ),
            blocking: false,
            write_scope: Vec::new(),
            required_tools: Vec::new(),
            enabled: true,
        },
    ]
}

pub fn apply_pre_tool_use_hooks(
    tool_name: &str,
    args: &Value,
    allowed_write_scope: Option<&[String]>,
) -> Result<Value, String> {
    let allowed_scope = allowed_write_scope
        .map(normalized_scope)
        .unwrap_or_default();
    if !allowed_scope.is_empty() && tool_name == "developer__text_editor" {
        let mut adjusted = args.clone();
        let path = if let Some(path) = normalized_editor_path(args) {
            path
        } else if allowed_scope.len() == 1 {
            let inferred = allowed_scope[0].clone();
            adjusted["path"] = Value::String(inferred.clone());
            inferred
        } else {
            return Err(
                "developer__text_editor writes must use a workspace-relative `path` that stays inside the current task write scope"
                    .to_string(),
            );
        };
        if !allowed_scope
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&path))
        {
            return Err(format!(
                "developer__text_editor attempted to write `{}` outside the current task write scope ({})",
                path,
                allowed_scope.join(", ")
            ));
        }
        return Ok(adjusted);
    }
    if tool_name != "subagent" {
        return Ok(args.clone());
    }
    let Some(parent_scope) = allowed_write_scope else {
        return Ok(args.clone());
    };
    let requested_scope = args
        .get("write_scope")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let constrained =
        workspace_runtime::constrain_subagent_write_scope(parent_scope, &requested_scope);
    let mut adjusted = args.clone();
    adjusted["write_scope"] = Value::Array(
        constrained
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>(),
    );
    Ok(adjusted)
}

fn emit_builtin_hook_payload(event: HookEventKind, payload: &HookPayload) {
    match serde_json::to_string(payload) {
        Ok(serialized) => {
            info!(hook_event = ?event, hook_payload = %serialized, "builtin hook payload emitted");
        }
        Err(error) => {
            info!(hook_event = ?event, error = %error, "failed to serialize builtin hook payload");
        }
    }
}

pub fn build_post_tool_use_payload(
    session_id: &str,
    run_id: Option<String>,
    request_id: &str,
    tool_name: &str,
    accepted_targets: Vec<String>,
    produced_delta: bool,
    validation_status: Option<String>,
) -> HookPayload {
    HookPayload {
        session_id: session_id.to_string(),
        run_id,
        task_id: None,
        request_id: Some(request_id.to_string()),
        tool_name: Some(tool_name.to_string()),
        accepted_targets,
        produced_delta: Some(produced_delta),
        validation_status,
        completion_status: Some("completed".to_string()),
        blocking_reason: None,
    }
}

pub fn build_post_tool_use_failure_payload(
    session_id: &str,
    run_id: Option<String>,
    request_id: &str,
    tool_name: &str,
    blocking_reason: String,
) -> HookPayload {
    HookPayload {
        session_id: session_id.to_string(),
        run_id,
        task_id: None,
        request_id: Some(request_id.to_string()),
        tool_name: Some(tool_name.to_string()),
        accepted_targets: Vec::new(),
        produced_delta: Some(false),
        validation_status: None,
        completion_status: Some("blocked".to_string()),
        blocking_reason: Some(blocking_reason),
    }
}

pub fn build_subagent_stop_payload(
    session_id: &str,
    run_id: Option<String>,
    task_id: &str,
    completion_status: &str,
    accepted_targets: Vec<String>,
    produced_delta: bool,
    validation_status: Option<String>,
    blocking_reason: Option<String>,
) -> HookPayload {
    HookPayload {
        session_id: session_id.to_string(),
        run_id,
        task_id: Some(task_id.to_string()),
        request_id: None,
        tool_name: Some("subagent".to_string()),
        accepted_targets,
        produced_delta: Some(produced_delta),
        validation_status,
        completion_status: Some(completion_status.to_string()),
        blocking_reason,
    }
}

pub fn build_run_settle_payload(
    session_id: &str,
    run_id: Option<String>,
    report: &ExecutionHostCompletionReport,
) -> HookPayload {
    HookPayload {
        session_id: session_id.to_string(),
        run_id,
        task_id: None,
        request_id: None,
        tool_name: None,
        accepted_targets: report.accepted_artifacts.clone(),
        produced_delta: Some(!report.produced_artifacts.is_empty()),
        validation_status: report.validation_status.clone(),
        completion_status: Some(report.status.clone()),
        blocking_reason: report.blocking_reason.clone(),
    }
}

pub fn emit_post_tool_use_payload(payload: &HookPayload) {
    emit_builtin_hook_payload(HookEventKind::PostToolUse, payload);
}

pub fn emit_post_tool_use_failure_payload(payload: &HookPayload) {
    emit_builtin_hook_payload(HookEventKind::PostToolUseFailure, payload);
}

pub fn emit_subagent_stop_payload(payload: &HookPayload) {
    emit_builtin_hook_payload(HookEventKind::SubagentStop, payload);
}

pub fn emit_run_settle_payload(payload: &HookPayload) {
    emit_builtin_hook_payload(HookEventKind::RunSettle, payload);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_hooks_cover_runtime_lifecycle_events() {
        let specs = builtin_hook_specs();
        assert!(specs
            .iter()
            .any(|spec| spec.event == HookEventKind::PreToolUse));
        assert!(specs
            .iter()
            .any(|spec| spec.event == HookEventKind::PostToolUse));
        assert!(specs
            .iter()
            .any(|spec| spec.event == HookEventKind::PostToolUseFailure));
        assert!(specs
            .iter()
            .any(|spec| spec.event == HookEventKind::SubagentStop));
        assert!(specs
            .iter()
            .any(|spec| spec.event == HookEventKind::RunSettle));
    }

    #[test]
    fn run_settle_payload_reflects_canonical_completion_report() {
        let payload = build_run_settle_payload(
            "session-1",
            Some("run-1".to_string()),
            &ExecutionHostCompletionReport {
                status: "blocked".to_string(),
                summary: "blocked".to_string(),
                produced_artifacts: vec!["docs/a.md".to_string()],
                accepted_artifacts: vec!["docs/a.md".to_string()],
                next_steps: Vec::new(),
                validation_status: Some("failed".to_string()),
                blocking_reason: Some("validation failed".to_string()),
                reason_code: Some("validation_failed".to_string()),
                content_accessed: Some(false),
                analysis_complete: Some(false),
            },
        );

        assert_eq!(payload.session_id, "session-1");
        assert_eq!(payload.run_id.as_deref(), Some("run-1"));
        assert_eq!(payload.completion_status.as_deref(), Some("blocked"));
        assert_eq!(payload.validation_status.as_deref(), Some("failed"));
        assert_eq!(
            payload.blocking_reason.as_deref(),
            Some("validation failed")
        );
        assert_eq!(payload.accepted_targets, vec!["docs/a.md".to_string()]);
        assert_eq!(payload.produced_delta, Some(true));
    }
}
