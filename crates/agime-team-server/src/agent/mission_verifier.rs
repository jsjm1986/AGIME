//! Shared mission contract verification utilities.
//!
//! Keeps execution orchestration (executor) and completion validation
//! (contract verifier) separated for easier evolution and testing.

use super::mission_mongo::ToolCallRecord;
use super::runtime;
use anyhow::{anyhow, Result};
use std::path::{Component, Path};
use std::process::{Output, Stdio};
use std::time::Duration;
use tokio::process::Command;

#[derive(Debug, Clone, Copy)]
pub struct VerifierLimits {
    pub max_required_artifacts: usize,
    pub max_completion_checks: usize,
    pub max_completion_check_cmd_len: usize,
}

#[derive(Debug, Clone, Copy)]
pub enum CompletionCheckMode {
    ExistsOnly,
    AllowShell { timeout: Duration },
}

pub fn from_tool_tuples(items: Vec<(String, bool)>) -> Vec<ToolCallRecord> {
    items
        .into_iter()
        .map(|(name, success)| ToolCallRecord { name, success })
        .collect()
}

fn normalize_required_artifacts(items: Vec<String>, limits: VerifierLimits) -> Vec<String> {
    items
        .into_iter()
        .map(|s| s.trim().replace('\\', "/"))
        .filter(|s| !s.is_empty())
        .take(limits.max_required_artifacts)
        .collect()
}

fn normalize_completion_checks(items: Vec<String>, limits: VerifierLimits) -> Vec<String> {
    items
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| {
            if let Some(path) = extract_exists_check_path(&s) {
                format!("exists:{}", path)
            } else {
                s
            }
        })
        .map(|s| {
            if s.chars().count() > limits.max_completion_check_cmd_len {
                s.chars()
                    .take(limits.max_completion_check_cmd_len)
                    .collect::<String>()
            } else {
                s
            }
        })
        .take(limits.max_completion_checks)
        .collect()
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

pub fn extract_exists_check_path(command: &str) -> Option<String> {
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

    let path = trim_wrapping_quotes(raw).replace('\\', "/");
    (!path.is_empty()).then_some(path)
}

pub fn is_safe_relative_workspace_path(value: &str) -> bool {
    let p = Path::new(value);
    if value.is_empty() || p.is_absolute() {
        return false;
    }
    p.components().all(|c| matches!(c, Component::Normal(_)))
}

pub fn validate_preflight_tool_calls(
    tool_calls: &[ToolCallRecord],
    preflight_tool_name: &str,
) -> Result<()> {
    let first_tool = tool_calls
        .iter()
        .find(|call| !call.name.trim().is_empty())
        .ok_or_else(|| {
            anyhow!(
                "mandatory preflight missing: call {} before any other tool",
                preflight_tool_name
            )
        })?;

    if !first_tool
        .name
        .trim()
        .eq_ignore_ascii_case(preflight_tool_name)
    {
        return Err(anyhow!(
            "mandatory preflight order violation: first tool was `{}`, expected `{}`",
            first_tool.name,
            preflight_tool_name
        ));
    }

    let has_success = tool_calls
        .iter()
        .any(|call| call.success && call.name.trim().eq_ignore_ascii_case(preflight_tool_name));
    if !has_success {
        return Err(anyhow!(
            "mandatory preflight failed: `{}` did not complete successfully",
            preflight_tool_name
        ));
    }

    Ok(())
}

pub fn resolve_effective_contract(
    preflight_contract: Option<runtime::MissionPreflightContract>,
    preflight_tool_name: &str,
    limits: VerifierLimits,
) -> Result<runtime::MissionPreflightContract> {
    let mut contract = preflight_contract
        .map(|contract| runtime::MissionPreflightContract {
            required_artifacts: normalize_required_artifacts(contract.required_artifacts, limits),
            completion_checks: normalize_completion_checks(contract.completion_checks, limits),
            no_artifact_reason: contract
                .no_artifact_reason
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        })
        .ok_or_else(|| {
            anyhow!(
                "missing preflight contract payload: call {} with required_artifacts/completion_checks/no_artifact_reason",
                preflight_tool_name
            )
        })?;

    if contract.required_artifacts.is_empty()
        && contract.completion_checks.is_empty()
        && contract.no_artifact_reason.is_none()
    {
        return Err(anyhow!(
            "missing preflight contract: declare required_artifacts/completion_checks, or set no_artifact_reason"
        ));
    }

    if !contract.required_artifacts.is_empty() && contract.no_artifact_reason.is_some() {
        contract.no_artifact_reason = None;
    }

    Ok(contract)
}

fn summary_claims_new_file_output(summary: &str) -> bool {
    let lower = summary.to_ascii_lowercase();
    let output_verbs = [
        "saved",
        "generated",
        "created",
        "written",
        "exported",
        "produced",
        "output",
        "生成",
        "保存",
        "写入",
        "导出",
        "产出",
    ];
    let has_output_verb = output_verbs.iter().any(|v| lower.contains(v));
    if !has_output_verb {
        return false;
    }

    let file_hints = [
        ".pptx", ".pdf", ".docx", ".xlsx", ".xls", ".csv", ".md", ".txt", ".json", ".png", ".jpg",
        ".jpeg",
    ];
    let has_file_hint = file_hints.iter().any(|hint| lower.contains(hint));
    let has_path_hint = lower.contains('/') || lower.contains('\\') || lower.contains("output");
    has_file_hint && has_path_hint
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        let mut out = value
            .chars()
            .take(max_chars.saturating_sub(3))
            .collect::<String>();
        out.push_str("...");
        out
    }
}

async fn execute_shell_command_in_workspace(
    command: &str,
    timeout: Duration,
    workspace_path: Option<&str>,
) -> Result<Output> {
    let mut cmd = if cfg!(target_os = "windows") {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", command]);
        cmd
    } else {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", command]);
        cmd
    };
    if let Some(path) = workspace_path {
        cmd.current_dir(path);
    }
    let fut = async move {
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(anyhow::Error::from)
    };
    tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| anyhow!("Command timed out after {:?}", timeout))?
}

#[allow(clippy::too_many_arguments)]
pub async fn validate_contract_outputs(
    contract: &runtime::MissionPreflightContract,
    workspace_path: Option<&str>,
    summary: Option<&str>,
    tool_calls: &[ToolCallRecord],
    workspace_artifact_count: usize,
    preflight_tool_name: &str,
    mode: CompletionCheckMode,
    enforce_workspace_artifact_signal: bool,
) -> Result<()> {
    validate_preflight_tool_calls(tool_calls, preflight_tool_name)?;

    let summary_text = summary.map(str::trim).unwrap_or_default();
    if summary_text.is_empty() {
        return Err(anyhow!("empty assistant output summary"));
    }

    if contract.no_artifact_reason.is_some() && summary_claims_new_file_output(summary_text) {
        return Err(anyhow!(
            "preflight declared non-file output, but assistant summary reported file output; update preflight contract with real required_artifacts"
        ));
    }

    if enforce_workspace_artifact_signal
        && workspace_path.is_some()
        && workspace_artifact_count == 0
        && summary_claims_new_file_output(summary_text)
        && !contract
            .required_artifacts
            .iter()
            .any(|rel| is_safe_relative_workspace_path(rel))
        && contract.no_artifact_reason.is_none()
        && tool_calls
            .iter()
            .any(|call| call.success && !call.name.trim().is_empty())
    {
        return Err(anyhow!(
            "assistant reported file output, but no new workspace artifact was detected; write deliverables under workspace-relative paths (for example output/...)"
        ));
    }

    if !contract.required_artifacts.is_empty() {
        let wp = workspace_path
            .ok_or_else(|| anyhow!("workspace path missing for required artifact checks"))?;
        let base = Path::new(wp);
        for rel in &contract.required_artifacts {
            if !is_safe_relative_workspace_path(rel) {
                return Err(anyhow!("invalid required artifact path: {}", rel));
            }
            let full = base.join(rel);
            if !full.exists() {
                return Err(anyhow!("required artifact not found: {}", rel));
            }
        }
    }

    if !contract.completion_checks.is_empty() {
        for command in &contract.completion_checks {
            if let Some(rel) = extract_exists_check_path(command) {
                let wp = workspace_path.ok_or_else(|| {
                    anyhow!("workspace path missing for completion check: {}", command)
                })?;
                if !is_safe_relative_workspace_path(&rel) {
                    return Err(anyhow!("invalid completion check path: {}", rel));
                }
                let full = Path::new(wp).join(&rel);
                if !full.exists() {
                    return Err(anyhow!(
                        "completion check failed: required path not found ({})",
                        rel
                    ));
                }
                continue;
            }

            match mode {
                CompletionCheckMode::ExistsOnly => {
                    return Err(anyhow!(
                        "unsupported completion check `{}` in adaptive mode; use exists:<relative_path> or declare required_artifacts",
                        command
                    ));
                }
                CompletionCheckMode::AllowShell { timeout } => {
                    let output =
                        execute_shell_command_in_workspace(command, timeout, workspace_path)
                            .await?;
                    if !output.status.success() {
                        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                        return Err(anyhow!(
                            "completion check failed: `{}` (status={:?}, stdout={}, stderr={})",
                            command,
                            output.status.code(),
                            truncate_chars(&stdout, 240),
                            truncate_chars(&stderr, 240)
                        ));
                    }
                }
            }
        }
    }

    Ok(())
}

pub fn has_verify_contract_tool_call(
    tool_calls: &[ToolCallRecord],
    verify_tool_name: &str,
) -> bool {
    tool_calls
        .iter()
        .any(|call| call.success && call.name.trim().eq_ignore_ascii_case(verify_tool_name))
}

pub fn verify_contract_status_label(
    verify_tool_called: bool,
    verify_contract_status: Option<bool>,
) -> &'static str {
    match (verify_tool_called, verify_contract_status) {
        (false, _) => "not_called",
        (true, Some(true)) => "pass",
        (true, Some(false)) => "fail",
        (true, None) => "unknown",
    }
}

pub fn enforce_verify_contract_gate(
    gate_mode: runtime::ContractVerifyGateMode,
    verify_tool_called: bool,
    verify_contract_status: Option<bool>,
    verify_tool_name: &str,
) -> Result<()> {
    match gate_mode {
        runtime::ContractVerifyGateMode::Off => Ok(()),
        runtime::ContractVerifyGateMode::Soft => Ok(()),
        runtime::ContractVerifyGateMode::Hard => {
            if !verify_tool_called {
                return Err(anyhow!(
                    "hard gate requires calling `{}` before completion",
                    verify_tool_name
                ));
            }
            match verify_contract_status {
                Some(true) => Ok(()),
                Some(false) => Err(anyhow!("`{}` returned fail", verify_tool_name)),
                None => Err(anyhow!(
                    "`{}` status unknown; ensure tool returns parseable status",
                    verify_tool_name
                )),
            }
        }
    }
}
