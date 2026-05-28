//! Workspace execution rules (desktop side).
//!
//! Mirror of the workspace boundary helpers from
//! `agime-team-server::agent::server_harness_host` and the
//! `WorkspaceExecutionContext` value type from
//! `agime-team-server::agent::workspace_service`. Per the dual-track plan,
//! desktop owns its own copy so it can stay independent of the team crate.
//!
//! What this provides:
//! - [`WorkspaceExecutionContext`] — value type carrying the workspace root,
//!   run id, run directory, and allowed read/write roots for the current
//!   session. Desktop callers populate it from session metadata.
//! - [`workspace_write_allowed`] — checks a candidate write path against the
//!   `allowed_write_roots` whitelist.
//! - [`workspace_execution_instruction`] — renders the canonical workspace
//!   boundary instruction shown to the model.
//! - [`merge_workspace_execution_instruction`] — merges a workspace overlay
//!   into a caller-supplied turn-system instruction string.
//! - [`execution_host_completion_response`] /
//!   [`document_analysis_completion_response`] — return the JSON-schema
//!   completion contracts the team-server attaches to recipe responses, so
//!   desktop can match the contract verbatim once it grows recipe-driven
//!   completion enforcement.
//!
//! SOURCE: crates/agime-team-server/src/agent/server_harness_host.rs and
//! crates/agime-team-server/src/agent/workspace_service.rs at commit 961109f.
//! Keep in sync manually — see CLAUDE.md long-term maintenance strategy.

#![allow(dead_code)]

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkspaceKind {
    #[default]
    Conversation,
    Document,
    Skill,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceExecutionContext {
    pub workspace_root: String,
    pub run_id: String,
    pub run_dir: String,
    pub allowed_read_roots: Vec<String>,
    pub allowed_write_roots: Vec<String>,
    pub artifact_dirs: Vec<String>,
    pub workspace_kind: WorkspaceKind,
}

pub fn workspace_write_allowed(context: &WorkspaceExecutionContext, path: &Path) -> bool {
    context
        .allowed_write_roots
        .iter()
        .any(|root| path.starts_with(Path::new(root)))
}

pub fn workspace_execution_instruction(context: &WorkspaceExecutionContext) -> String {
    format!(
        "Workspace execution boundary:\n- Workspace root: `{}`\n- Current run directory: `{}`\n- Treat `attachments/` as read-only input material.\n- Write final user-visible artifacts under `artifacts/`.\n- Write durable notes under `notes/`.\n- Use `runs/{}` only for this turn's temporary execution files.\n- Do not describe paths outside the workspace as previewable, downloadable, or shareable workspace artifacts.",
        context.workspace_root, context.run_dir, context.run_id
    )
}

pub fn merge_workspace_execution_instruction(
    turn_system_instruction: Option<&str>,
    context: Option<&WorkspaceExecutionContext>,
) -> Option<String> {
    let existing = turn_system_instruction
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let workspace_instruction = context.map(workspace_execution_instruction);
    match (existing, workspace_instruction) {
        (Some(existing), Some(workspace_instruction)) => {
            Some(format!("{existing}\n\n{workspace_instruction}"))
        }
        (Some(existing), None) => Some(existing.to_string()),
        (None, Some(workspace_instruction)) => Some(workspace_instruction),
        (None, None) => None,
    }
}

pub fn execution_host_completion_response() -> agime::recipe::Response {
    agime::recipe::Response {
        json_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["completed", "blocked"]
                },
                "summary": {
                    "type": "string",
                    "minLength": 1
                },
                "produced_artifacts": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "accepted_artifacts": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "next_steps": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "validation_status": {
                    "type": "string",
                    "enum": ["passed", "failed", "not_run"]
                },
                "blocking_reason": {
                    "type": "string"
                }
            },
            "required": ["status", "summary", "produced_artifacts", "accepted_artifacts", "next_steps"],
            "additionalProperties": false
        })),
    }
}

pub fn document_analysis_completion_response() -> agime::recipe::Response {
    agime::recipe::Response {
        json_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["completed", "blocked"]
                },
                "summary": {
                    "type": "string",
                    "minLength": 1
                },
                "produced_artifacts": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "accepted_artifacts": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "next_steps": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "validation_status": {
                    "type": "string",
                    "enum": ["passed", "failed", "not_run"]
                },
                "blocking_reason": { "type": "string" },
                "reason_code": { "type": "string" },
                "content_accessed": { "type": "boolean" },
                "analysis_complete": { "type": "boolean" }
            },
            "required": [
                "status",
                "summary",
                "produced_artifacts",
                "accepted_artifacts",
                "next_steps",
                "content_accessed",
                "analysis_complete",
                "reason_code"
            ],
            "additionalProperties": false
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> WorkspaceExecutionContext {
        WorkspaceExecutionContext {
            workspace_root: "/work".to_string(),
            run_id: "run-42".to_string(),
            run_dir: "/work/runs/run-42".to_string(),
            allowed_read_roots: vec!["/work".to_string(), "/data/in".to_string()],
            allowed_write_roots: vec!["/work/artifacts".to_string(), "/work/notes".to_string()],
            artifact_dirs: vec!["/work/artifacts".to_string()],
            workspace_kind: WorkspaceKind::Conversation,
        }
    }

    #[test]
    fn write_allowed_matches_prefix_roots() {
        let context = ctx();
        assert!(workspace_write_allowed(
            &context,
            Path::new("/work/artifacts/output.txt")
        ));
        assert!(workspace_write_allowed(
            &context,
            Path::new("/work/notes/diary.md")
        ));
        assert!(!workspace_write_allowed(&context, Path::new("/etc/passwd")));
        assert!(!workspace_write_allowed(
            &context,
            Path::new("/work/runs/run-42/scratch")
        ));
    }

    #[test]
    fn execution_instruction_mentions_root_run_id_run_dir() {
        let context = ctx();
        let instruction = workspace_execution_instruction(&context);
        assert!(instruction.contains("/work"));
        assert!(instruction.contains("run-42"));
        assert!(instruction.contains("/work/runs/run-42"));
        assert!(instruction.contains("Workspace execution boundary"));
    }

    #[test]
    fn merge_returns_existing_when_no_context() {
        let merged = merge_workspace_execution_instruction(Some("turn note"), None);
        assert_eq!(merged.as_deref(), Some("turn note"));
    }

    #[test]
    fn merge_returns_workspace_when_no_existing() {
        let context = ctx();
        let merged = merge_workspace_execution_instruction(None, Some(&context));
        assert!(merged.unwrap().contains("Workspace execution boundary"));
    }

    #[test]
    fn merge_concatenates_existing_then_workspace() {
        let context = ctx();
        let merged =
            merge_workspace_execution_instruction(Some("turn note"), Some(&context)).unwrap();
        let existing_pos = merged.find("turn note").unwrap();
        let workspace_pos = merged.find("Workspace execution boundary").unwrap();
        assert!(existing_pos < workspace_pos);
    }

    #[test]
    fn merge_treats_blank_existing_as_absent() {
        let context = ctx();
        let merged =
            merge_workspace_execution_instruction(Some("   \n  "), Some(&context)).unwrap();
        assert!(merged.starts_with("Workspace execution boundary"));
    }

    #[test]
    fn merge_returns_none_when_both_absent() {
        assert!(merge_workspace_execution_instruction(None, None).is_none());
        assert!(merge_workspace_execution_instruction(Some("   "), None).is_none());
    }

    #[test]
    fn execution_host_completion_response_requires_summary_and_status() {
        let response = execution_host_completion_response();
        let schema = response.json_schema.unwrap();
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"status"));
        assert!(names.contains(&"summary"));
        assert!(names.contains(&"produced_artifacts"));
        assert_eq!(
            schema.get("additionalProperties").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn document_analysis_completion_response_requires_content_accessed() {
        let response = document_analysis_completion_response();
        let schema = response.json_schema.unwrap();
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"content_accessed"));
        assert!(names.contains(&"analysis_complete"));
        assert!(names.contains(&"reason_code"));
    }
}
