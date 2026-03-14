//! Mission Preflight tools provider.
//!
//! This extension is loaded only for mission/task execution flow and exposes
//! a mandatory preflight tool that turns soft prompt rules into explicit,
//! machine-readable execution contracts.

use super::mission_verifier;
use agime::agents::mcp_client::McpClientTrait;
use anyhow::{anyhow, Result};
use rmcp::model::*;
use rmcp::ServiceError;
use serde_json::json;
use std::fs;
use std::path::{Component, Path};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub struct MissionPreflightToolsProvider {
    info: InitializeResult,
}

impl MissionPreflightToolsProvider {
    pub fn new() -> Self {
        let info = InitializeResult {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {
                    list_changed: Some(false),
                }),
                resources: None,
                prompts: None,
                completions: None,
                experimental: None,
                logging: None,
                extensions: None,
                tasks: None,
            },
            server_info: Implementation {
                name: "mission_preflight".to_string(),
                title: Some("Mission Preflight".to_string()),
                version: "1.0.0".to_string(),
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "For mission execution, call preflight first before any other tool to validate step contract, workspace, artifacts, and checks. Use workspace_overview/verify_contract to inspect and self-verify."
                    .to_string(),
            ),
        };
        Self { info }
    }

    fn verify_contract_shell_timeout() -> Duration {
        Duration::from_secs(20)
    }

    fn truncate_chars(text: &str, max_chars: usize) -> String {
        if text.chars().count() <= max_chars {
            return text.to_string();
        }
        let mut out = text
            .chars()
            .take(max_chars.saturating_sub(3))
            .collect::<String>();
        out.push_str("...");
        out
    }

    fn tool_definitions() -> Vec<Tool> {
        vec![
            Tool {
                name: "preflight".into(),
                title: None,
                description: Some(
                    "MANDATORY first call in mission mode. Validate step contract and return an execution checklist."
                        .into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "step_title": { "type": "string", "description": "Current step title" },
                        "step_goal": { "type": "string", "description": "Current step goal/description" },
                        "workspace_path": { "type": "string", "description": "Absolute workspace path for this mission run" },
                        "required_artifacts": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Expected workspace-relative output paths"
                        },
                        "completion_checks": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Completion checks that must pass. Supports exists:<relative_path> or deterministic shell commands executed in the workspace"
                        },
                        "no_artifact_reason": {
                            "type": "string",
                            "description": "When no file artifact is expected, explain why this step is text/analysis-only"
                        },
                        "attempt": { "type": "integer", "description": "Current attempt number (1-based)" },
                        "last_error": { "type": "string", "description": "Last failure reason for retries" }
                    },
                    "required": ["step_goal", "workspace_path"]
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "workspace_overview".into(),
                title: None,
                description: Some(
                    "Inspect workspace files/directories to help planning and path selection."
                        .into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "workspace_path": { "type": "string", "description": "Absolute workspace path for this mission run" },
                        "max_depth": { "type": "integer", "description": "Directory recursion depth (default 2, max 6)" },
                        "max_entries": { "type": "integer", "description": "Maximum entries returned (default 200, max 1000)" },
                        "include_hidden": { "type": "boolean", "description": "Whether to include hidden files (default false)" }
                    },
                    "required": ["workspace_path"]
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "verify_contract".into(),
                title: None,
                description: Some(
                    "Verify contract against workspace state. Use before final completion response."
                        .into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "workspace_path": { "type": "string", "description": "Absolute workspace path for this mission run" },
                        "required_artifacts": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Expected workspace-relative output paths"
                        },
                        "completion_checks": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Completion checks (exists:<path> preferred)"
                        },
                        "no_artifact_reason": {
                            "type": "string",
                            "description": "For non-file outcomes, explain why no artifact is expected"
                        }
                    },
                    "required": ["workspace_path"]
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
        ]
    }

    fn parse_string_list(args: &JsonObject, key: &str) -> Vec<String> {
        args.get(key)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn is_safe_relative_workspace_path(path: &str) -> bool {
        let normalized = path.trim().replace('\\', "/");
        if normalized.is_empty() {
            return false;
        }
        let parsed = Path::new(&normalized);
        if parsed.is_absolute() {
            return false;
        }
        parsed
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    }

    fn trim_wrapping_quotes(value: &str) -> String {
        let trimmed = value.trim();
        let bytes = trimmed.as_bytes();
        if bytes.len() >= 2 {
            let first = bytes[0];
            let last = bytes[bytes.len() - 1];
            if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
                return trimmed[1..trimmed.len() - 1].trim().to_string();
            }
        }
        trimmed.to_string()
    }

    fn extract_exists_check_path(command: &str) -> Option<String> {
        let trimmed = command.trim();
        let lower = trimmed.to_ascii_lowercase();
        let raw = trimmed
            .strip_prefix("exists:")
            .or_else(|| {
                if lower.starts_with("test -f ") || lower.starts_with("test -e ") {
                    trimmed.get(8..)
                } else {
                    None
                }
            })
            .or_else(|| {
                trimmed
                    .strip_prefix("[ -f ")
                    .or_else(|| trimmed.strip_prefix("[ -e "))
                    .and_then(|s| s.strip_suffix(" ]"))
            })?;

        let path = Self::trim_wrapping_quotes(raw).replace('\\', "/");
        (!path.is_empty()).then_some(path)
    }

    fn parse_u64(args: &JsonObject, key: &str, default: u64, min: u64, max: u64) -> u64 {
        args.get(key)
            .and_then(|v| v.as_u64())
            .unwrap_or(default)
            .clamp(min, max)
    }

    fn parse_bool(args: &JsonObject, key: &str, default: bool) -> bool {
        args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    fn collect_workspace_entries(
        base: &Path,
        current: &Path,
        depth: usize,
        max_depth: usize,
        include_hidden: bool,
        max_entries: usize,
        out: &mut Vec<serde_json::Value>,
    ) {
        if depth > max_depth || out.len() >= max_entries {
            return;
        }

        let read_dir = match fs::read_dir(current) {
            Ok(rd) => rd,
            Err(_) => return,
        };

        let mut paths = Vec::new();
        for entry in read_dir.flatten() {
            paths.push(entry.path());
        }
        paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

        for path in paths {
            if out.len() >= max_entries {
                break;
            }

            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if name.is_empty() {
                continue;
            }
            if !include_hidden && name.starts_with('.') {
                continue;
            }

            let rel = match path.strip_prefix(base) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if rel.is_empty() {
                continue;
            }

            let meta = match fs::metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            if meta.is_dir() {
                out.push(json!({
                    "path": rel,
                    "kind": "dir"
                }));
                if depth < max_depth {
                    Self::collect_workspace_entries(
                        base,
                        &path,
                        depth + 1,
                        max_depth,
                        include_hidden,
                        max_entries,
                        out,
                    );
                }
            } else if meta.is_file() {
                out.push(json!({
                    "path": rel,
                    "kind": "file",
                    "size_bytes": meta.len()
                }));
            }
        }
    }

    async fn handle_workspace_overview(&self, args: &JsonObject) -> Result<String> {
        let workspace_path = args
            .get("workspace_path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("workspace_path is required"))?
            .to_string();

        let max_depth = Self::parse_u64(args, "max_depth", 2, 0, 6) as usize;
        let max_entries = Self::parse_u64(args, "max_entries", 200, 1, 1000) as usize;
        let include_hidden = Self::parse_bool(args, "include_hidden", false);

        let base = Path::new(&workspace_path);
        if !base.exists() {
            return Err(anyhow!("workspace_path does not exist: {}", workspace_path));
        }
        if !base.is_dir() {
            return Err(anyhow!(
                "workspace_path is not a directory: {}",
                workspace_path
            ));
        }

        let mut entries = Vec::new();
        Self::collect_workspace_entries(
            base,
            base,
            0,
            max_depth,
            include_hidden,
            max_entries,
            &mut entries,
        );

        let dir_count = entries
            .iter()
            .filter(|e| e.get("kind").and_then(|v| v.as_str()) == Some("dir"))
            .count();
        let file_count = entries
            .iter()
            .filter(|e| e.get("kind").and_then(|v| v.as_str()) == Some("file"))
            .count();
        let output_exists = base.join("output").exists();
        let documents_exists = base.join("documents").exists();

        Ok(json!({
            "status": "ok",
            "workspace": {
                "path": workspace_path,
                "exists": true,
                "output_dir_exists": output_exists,
                "documents_dir_exists": documents_exists,
            },
            "summary": {
                "max_depth": max_depth,
                "max_entries": max_entries,
                "returned_entries": entries.len(),
                "dir_count": dir_count,
                "file_count": file_count,
            },
            "entries": entries,
        })
        .to_string())
    }

    async fn handle_verify_contract(&self, args: &JsonObject) -> Result<String> {
        let workspace_path = args
            .get("workspace_path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("workspace_path is required"))?
            .to_string();
        let required_artifacts = Self::parse_string_list(args, "required_artifacts");
        let completion_checks = Self::parse_string_list(args, "completion_checks");
        let no_artifact_reason = args
            .get("no_artifact_reason")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let base = Path::new(&workspace_path);
        if !base.exists() {
            return Err(anyhow!("workspace_path does not exist: {}", workspace_path));
        }
        if !base.is_dir() {
            return Err(anyhow!(
                "workspace_path is not a directory: {}",
                workspace_path
            ));
        }
        if required_artifacts.is_empty()
            && completion_checks.is_empty()
            && no_artifact_reason.is_none()
        {
            return Err(anyhow!(
                "empty contract: provide required_artifacts/completion_checks, or set no_artifact_reason for non-file outcomes"
            ));
        }

        let mut missing: Vec<String> = Vec::new();
        let mut invalid_paths: Vec<String> = Vec::new();
        let mut required_results: Vec<serde_json::Value> = Vec::new();
        for rel in &required_artifacts {
            let safe = Self::is_safe_relative_workspace_path(rel);
            if !safe {
                invalid_paths.push(rel.clone());
                required_results.push(json!({
                    "path": rel,
                    "ok": false,
                    "reason": "invalid_workspace_relative_path"
                }));
                continue;
            }
            let full = base.join(rel);
            let exists = full.exists();
            let is_file = full.is_file();
            let size = if exists && is_file {
                fs::metadata(&full).ok().map(|m| m.len())
            } else {
                None
            };
            if !exists || !is_file {
                missing.push(rel.clone());
            }
            required_results.push(json!({
                "path": rel,
                "ok": exists && is_file,
                "exists": exists,
                "is_file": is_file,
                "size_bytes": size,
            }));
        }

        let mut check_failures: Vec<String> = Vec::new();
        let mut check_results: Vec<serde_json::Value> = Vec::new();
        for check in &completion_checks {
            if let Some(rel) = Self::extract_exists_check_path(check) {
                if !Self::is_safe_relative_workspace_path(&rel) {
                    check_failures.push(check.clone());
                    check_results.push(json!({
                        "check": check,
                        "ok": false,
                        "reason": "invalid_workspace_relative_path",
                        "path": rel
                    }));
                    continue;
                }
                let ok = base.join(&rel).exists();
                if !ok {
                    check_failures.push(check.clone());
                }
                check_results.push(json!({
                    "check": check,
                    "ok": ok,
                    "mode": "exists",
                    "path": rel
                }));
            } else {
                match mission_verifier::execute_shell_command_in_workspace(
                    check,
                    Self::verify_contract_shell_timeout(),
                    Some(&workspace_path),
                )
                .await
                {
                    Ok(output) => {
                        let ok = output.status.success();
                        if !ok {
                            check_failures.push(check.clone());
                        }
                        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                        check_results.push(json!({
                            "check": check,
                            "ok": ok,
                            "mode": "shell",
                            "status": output.status.code(),
                            "stdout": Self::truncate_chars(&stdout, 240),
                            "stderr": Self::truncate_chars(&stderr, 240),
                        }));
                    }
                    Err(e) => {
                        check_failures.push(check.clone());
                        check_results.push(json!({
                            "check": check,
                            "ok": false,
                            "mode": "shell",
                            "reason": e.to_string(),
                        }));
                    }
                }
            }
        }

        let pass = missing.is_empty() && invalid_paths.is_empty() && check_failures.is_empty();
        let advice = if pass {
            vec![
                "Contract verification passed.".to_string(),
                "You can now provide final completion response with exact output paths."
                    .to_string(),
            ]
        } else {
            vec![
                "Contract verification failed.".to_string(),
                "Create/fix missing artifacts and rerun verify_contract before final response."
                    .to_string(),
            ]
        };

        Ok(json!({
            "status": if pass { "pass" } else { "fail" },
            "workspace": {
                "path": workspace_path,
                "exists": true
            },
            "contract": {
                "required_artifacts": required_artifacts,
                "completion_checks": completion_checks,
                "no_artifact_reason": no_artifact_reason,
            },
            "results": {
                "required_artifacts": required_results,
                "completion_checks": check_results,
            },
            "missing_required_artifacts": missing,
            "invalid_paths": invalid_paths,
            "failed_checks": check_failures,
            "advice": advice,
        })
        .to_string())
    }

    async fn handle_preflight(&self, args: &JsonObject) -> Result<String> {
        let step_title = args
            .get("step_title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled Step")
            .trim()
            .to_string();
        let step_goal = args
            .get("step_goal")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("step_goal is required"))?
            .to_string();
        let workspace_path = args
            .get("workspace_path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("workspace_path is required"))?
            .to_string();
        let required_artifacts = Self::parse_string_list(args, "required_artifacts");
        let completion_checks = Self::parse_string_list(args, "completion_checks");
        let no_artifact_reason = args
            .get("no_artifact_reason")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let attempt = args
            .get("attempt")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .max(1);
        let last_error = args
            .get("last_error")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let workspace_exists = Path::new(&workspace_path).exists();
        if !workspace_exists {
            return Err(anyhow!("workspace_path does not exist: {}", workspace_path));
        }

        let mut artifact_path_errors = Vec::new();
        for rel in &required_artifacts {
            if !Self::is_safe_relative_workspace_path(rel) {
                artifact_path_errors.push(rel.clone());
            }
        }
        if !artifact_path_errors.is_empty() {
            return Err(anyhow!(
                "required_artifacts contain invalid workspace-relative paths: {}",
                artifact_path_errors.join(", ")
            ));
        }

        if required_artifacts.is_empty()
            && completion_checks.is_empty()
            && no_artifact_reason.is_none()
        {
            return Err(anyhow!(
                "empty contract: provide required_artifacts/completion_checks, or set no_artifact_reason for non-file outcomes"
            ));
        }

        let mut plan = vec![
            "Understand the requested output and constraints from this step.".to_string(),
            "Execute concrete actions to produce real deliverables (no placeholders).".to_string(),
            "Verify outputs against required artifacts and completion checks.".to_string(),
            "Return concise completion evidence with exact output paths.".to_string(),
        ];
        if no_artifact_reason.is_some() {
            plan.push(
                "This step is declared as non-file output; provide concise, verifiable textual completion evidence."
                    .to_string(),
            );
        }
        if let Some(err) = &last_error {
            plan.insert(
                0,
                format!(
                    "Address previous failure before new actions. Root failure: {}",
                    err
                ),
            );
        }

        let preflight_id = Uuid::new_v4().to_string();
        Ok(json!({
            "status": "ready",
            "preflight_id": preflight_id,
            "mode": "mission",
            "attempt": attempt,
            "step": {
                "title": step_title,
                "goal": step_goal,
            },
            "workspace": {
                "path": workspace_path,
                "exists": workspace_exists,
            },
            "contract": {
                "required_artifacts": required_artifacts,
                "completion_checks": completion_checks,
                "no_artifact_reason": no_artifact_reason,
                "rules": [
                    "call_preflight_before_other_tools",
                    "real_outputs_only_no_placeholders",
                    "report_workspace_relative_output_paths"
                ]
            },
            "execution_plan": plan,
            "verification": {
                "artifact_count_expected": required_artifacts.len(),
                "completion_check_count": completion_checks.len(),
            }
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value};

    fn verify_contract_args(
        workspace_path: &Path,
        completion_checks: Vec<&str>,
    ) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert(
            "workspace_path".into(),
            Value::String(workspace_path.to_string_lossy().to_string()),
        );
        args.insert("required_artifacts".into(), Value::Array(vec![]));
        args.insert(
            "completion_checks".into(),
            Value::Array(
                completion_checks
                    .into_iter()
                    .map(|s| Value::String(s.to_string()))
                    .collect(),
            ),
        );
        args.insert(
            "no_artifact_reason".into(),
            Value::String("runtime verification step".to_string()),
        );
        args
    }

    #[tokio::test]
    async fn verify_contract_accepts_successful_shell_completion_checks() {
        let workspace = std::env::temp_dir().join(format!("mission-preflight-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let provider = MissionPreflightToolsProvider::new();
        let result = provider
            .handle_verify_contract(&verify_contract_args(&workspace, vec!["echo ok"]))
            .await
            .unwrap();
        let payload: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(payload.get("status").and_then(Value::as_str), Some("pass"));
        std::fs::remove_dir_all(&workspace).ok();
    }

    #[tokio::test]
    async fn verify_contract_reports_failing_shell_completion_checks() {
        let workspace = std::env::temp_dir().join(format!("mission-preflight-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let provider = MissionPreflightToolsProvider::new();
        #[cfg(windows)]
        let failing_check = "exit /b 1";
        #[cfg(not(windows))]
        let failing_check = "false";
        let result = provider
            .handle_verify_contract(&verify_contract_args(&workspace, vec![failing_check]))
            .await
            .unwrap();
        let payload: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(payload.get("status").and_then(Value::as_str), Some("fail"));
        assert!(payload
            .get("failed_checks")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|v| v.as_str() == Some(failing_check))));
        std::fs::remove_dir_all(&workspace).ok();
    }
}

#[async_trait::async_trait]
impl McpClientTrait for MissionPreflightToolsProvider {
    async fn list_resources(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListResourcesResult, ServiceError> {
        Ok(ListResourcesResult {
            resources: vec![],
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        _uri: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ReadResourceResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn list_tools(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListToolsResult, ServiceError> {
        Ok(ListToolsResult {
            tools: Self::tool_definitions(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<CallToolResult, ServiceError> {
        let args = arguments.unwrap_or_default();
        let result = match name {
            "preflight" => self.handle_preflight(&args).await,
            "workspace_overview" => self.handle_workspace_overview(&args).await,
            "verify_contract" => self.handle_verify_contract(&args).await,
            _ => Err(anyhow!("Unknown tool: {}", name)),
        };

        match result {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    async fn list_tasks(
        &self,
        _cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListTasksResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn get_task_info(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<GetTaskInfoResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn get_task_result(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<TaskResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn cancel_task(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<(), ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn list_prompts(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListPromptsResult, ServiceError> {
        Ok(ListPromptsResult {
            prompts: vec![],
            next_cursor: None,
            meta: None,
        })
    }

    async fn get_prompt(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<GetPromptResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        let (_tx, rx) = mpsc::channel(1);
        rx
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}
