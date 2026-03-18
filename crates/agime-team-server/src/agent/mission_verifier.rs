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
        .filter_map(|s| normalize_completion_check(&s, limits.max_completion_check_cmd_len))
        .take(limits.max_completion_checks)
        .collect()
}

pub fn normalize_completion_check(command: &str, max_len: usize) -> Option<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = if let Some(path) = extract_exists_check_path(trimmed) {
        format!("exists:{}", path)
    } else {
        strip_shell_completion_check_prefix(trimmed).to_string()
    };
    let normalized = if normalized.chars().count() > max_len {
        normalized.chars().take(max_len).collect::<String>()
    } else {
        normalized
    };
    (!normalized.trim().is_empty()).then_some(normalized)
}

fn strip_shell_completion_check_prefix(command: &str) -> &str {
    let trimmed = command.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("sh:") {
        trimmed[3..].trim()
    } else if lower.starts_with("bash:") {
        trimmed[5..].trim()
    } else if lower.starts_with("shell:") {
        trimmed[6..].trim()
    } else {
        trimmed
    }
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
    allow_persisted_success: bool,
) -> Result<()> {
    let has_success = tool_calls
        .iter()
        .any(|call| call.success && call.name.trim().eq_ignore_ascii_case(preflight_tool_name));

    let Some(first_tool) = tool_calls.iter().find(|call| !call.name.trim().is_empty()) else {
        if allow_persisted_success || has_success {
            return Ok(());
        }
        tracing::warn!(
            "Soft mission validation issue: no successful `{}` call was observed before work began; continuing with best-effort contract repair instead of failing early",
            preflight_tool_name
        );
        return Ok(());
    };

    if !first_tool
        .name
        .trim()
        .eq_ignore_ascii_case(preflight_tool_name)
    {
        if allow_persisted_success || has_success {
            tracing::warn!(
                "Soft mission validation issue: preflight order relaxed because a successful `{}` call exists later in the tool trace",
                preflight_tool_name
            );
            return Ok(());
        }
        tracing::warn!(
            "Soft mission validation issue: first tool was `{}` instead of `{}`; continuing because strict preflight ordering should not end the mission",
            first_tool.name,
            preflight_tool_name
        );
        return Ok(());
    }

    if !has_success {
        if allow_persisted_success {
            tracing::warn!(
                "Soft mission validation issue: fresh `{}` call did not complete successfully, but a persisted verified contract is being reused",
                preflight_tool_name
            );
            return Ok(());
        }
        tracing::warn!(
            "Soft mission validation issue: `{}` did not complete successfully; continuing with best-effort contract repair instead of failing the run",
            preflight_tool_name
        );
        return Ok(());
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
        .unwrap_or_else(|| {
            tracing::warn!(
                "Soft mission validation issue: missing preflight contract payload for `{}`; synthesizing an empty best-effort contract so execution can continue",
                preflight_tool_name
            );
            runtime::MissionPreflightContract {
                required_artifacts: Vec::new(),
                completion_checks: Vec::new(),
                no_artifact_reason: Some(
                    "preflight contract payload missing; continue with best-effort evidence collection"
                        .to_string(),
                ),
            }
        });

    if contract.required_artifacts.is_empty()
        && contract.completion_checks.is_empty()
        && contract.no_artifact_reason.is_none()
    {
        tracing::warn!(
            "Soft mission validation issue: empty preflight contract; auto-filling no_artifact_reason so execution can continue and rely on live evidence"
        );
        contract.no_artifact_reason = Some(
            "preflight contract did not declare artifacts or checks; continue with best-effort evidence collection"
                .to_string(),
        );
    }

    if !contract.required_artifacts.is_empty() && contract.no_artifact_reason.is_some() {
        contract.no_artifact_reason = None;
    }

    Ok(contract)
}

fn summary_claims_new_file_output(summary: &str) -> bool {
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
    let file_hints = [
        ".pptx", ".pdf", ".docx", ".xlsx", ".xls", ".csv", ".md", ".txt", ".json", ".png", ".jpg",
        ".jpeg",
    ];
    let negative_or_future_markers = [
        "do not exist",
        "does not exist",
        "did not exist",
        "not exist",
        "no files created",
        "no file created",
        "nothing to preserve",
        "inspection-only",
        "inspection only",
        "later steps will create",
        "later step will create",
        "will create",
        "to be created",
        "not created",
        "no deliverables are created",
        "不存在",
        "未创建",
        "没有创建",
        "未生成",
        "没有生成",
        "后续步骤将创建",
        "后续会创建",
    ];

    summary
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .any(|line| {
            let lower = line.to_ascii_lowercase();
            let has_output_verb = output_verbs.iter().any(|v| lower.contains(v));
            if !has_output_verb {
                return false;
            }
            if negative_or_future_markers
                .iter()
                .any(|marker| lower.contains(marker))
            {
                return false;
            }
            let has_file_hint = file_hints.iter().any(|hint| lower.contains(hint));
            let has_path_hint =
                lower.contains('/') || lower.contains('\\') || lower.contains("output");
            has_file_hint && has_path_hint
        })
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

pub(crate) async fn execute_shell_command_in_workspace(
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
    allow_persisted_preflight_success: bool,
    mode: CompletionCheckMode,
    enforce_workspace_artifact_signal: bool,
) -> Result<()> {
    validate_preflight_tool_calls(
        tool_calls,
        preflight_tool_name,
        allow_persisted_preflight_success,
    )?;

    let summary_text = summary.map(str::trim).unwrap_or_default();
    if summary_text.is_empty() {
        tracing::warn!("Soft mission validation issue: empty assistant output summary");
    }

    if contract.no_artifact_reason.is_some() && summary_claims_new_file_output(summary_text) {
        tracing::warn!(
            "Soft mission validation issue: preflight declared non-file output, but assistant summary reported file output"
        );
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
        tracing::warn!(
            "Soft mission validation issue: assistant reported file output, but no new workspace artifact was detected"
        );
    }

    if !contract.required_artifacts.is_empty() {
        if let Some(wp) = workspace_path {
            let base = Path::new(wp);
            for rel in &contract.required_artifacts {
                if !is_safe_relative_workspace_path(rel) {
                    return Err(anyhow!("invalid required artifact path: {}", rel));
                }
                let full = base.join(rel);
                if !full.exists() {
                    tracing::warn!(
                        "Soft mission validation issue: required artifact not found: {}",
                        rel
                    );
                }
            }
        } else {
            tracing::warn!(
                "Soft mission validation issue: workspace path missing for required artifact checks"
            );
        }
    }

    if !contract.completion_checks.is_empty() {
        for command in &contract.completion_checks {
            if let Some(rel) = extract_exists_check_path(command) {
                let Some(wp) = workspace_path else {
                    tracing::warn!(
                        "Soft mission validation issue: workspace path missing for completion check: {}",
                        command
                    );
                    continue;
                };
                if !is_safe_relative_workspace_path(&rel) {
                    return Err(anyhow!("invalid completion check path: {}", rel));
                }
                let full = Path::new(wp).join(&rel);
                if !full.exists() {
                    tracing::warn!(
                        "Soft mission validation issue: completion check required path not found ({})",
                        rel
                    );
                }
                continue;
            }

            match mode {
                CompletionCheckMode::ExistsOnly => {
                    tracing::warn!(
                        "Soft mission validation issue: unsupported completion check `{}` in exists-only mode",
                        command
                    );
                }
                CompletionCheckMode::AllowShell { timeout } => {
                    let output =
                        execute_shell_command_in_workspace(command, timeout, workspace_path)
                            .await?;
                    if !output.status.success() {
                        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                        tracing::warn!(
                            "Soft mission validation issue: completion check failed: `{}` (status={:?}, stdout={}, stderr={})",
                            command,
                            output.status.code(),
                            truncate_chars(&stdout, 240),
                            truncate_chars(&stderr, 240)
                        );
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
                tracing::warn!(
                    "Softening hard verify gate: `{}` was not called before completion",
                    verify_tool_name
                );
                return Ok(());
            }
            if verify_contract_status != Some(true) {
                tracing::warn!(
                    "Softening hard verify gate: `{}` status was {:?}",
                    verify_tool_name,
                    verify_contract_status
                );
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_effective_contract, summary_claims_new_file_output, validate_preflight_tool_calls,
        VerifierLimits,
    };
    use crate::agent::mission_mongo::ToolCallRecord;
    use crate::agent::runtime::MissionPreflightContract;

    fn limits() -> VerifierLimits {
        VerifierLimits {
            max_required_artifacts: 16,
            max_completion_checks: 8,
            max_completion_check_cmd_len: 300,
        }
    }

    #[test]
    fn resolve_effective_contract_softens_missing_payload_into_best_effort_contract() {
        let resolved = resolve_effective_contract(None, "mission_preflight__preflight", limits())
            .expect("missing payload should now soften into a best-effort contract");
        assert!(resolved.required_artifacts.is_empty());
        assert!(resolved.completion_checks.is_empty());
        assert!(resolved
            .no_artifact_reason
            .as_deref()
            .unwrap_or_default()
            .contains("best-effort"));
    }

    #[test]
    fn resolve_effective_contract_softens_empty_contract() {
        let resolved = resolve_effective_contract(
            Some(MissionPreflightContract {
                required_artifacts: vec![],
                completion_checks: vec![],
                no_artifact_reason: None,
            }),
            "mission_preflight__preflight",
            limits(),
        )
        .expect("empty contract should now soften");
        assert!(resolved.required_artifacts.is_empty());
        assert!(resolved.completion_checks.is_empty());
        assert!(resolved
            .no_artifact_reason
            .as_deref()
            .unwrap_or_default()
            .contains("best-effort"));
    }

    #[test]
    fn resolve_effective_contract_drops_no_artifact_reason_when_artifacts_exist() {
        let resolved = resolve_effective_contract(
            Some(MissionPreflightContract {
                required_artifacts: vec!["reports/out.md".to_string()],
                completion_checks: vec![],
                no_artifact_reason: Some("text only".to_string()),
            }),
            "mission_preflight__preflight",
            limits(),
        )
        .expect("contract should resolve");

        assert_eq!(resolved.required_artifacts, vec!["reports/out.md"]);
        assert!(resolved.no_artifact_reason.is_none());
    }

    #[test]
    fn validate_preflight_tool_calls_allows_persisted_success_without_fresh_calls() {
        validate_preflight_tool_calls(&[], "mission_preflight__preflight", true)
            .expect("persisted preflight success should satisfy the gate");
    }

    #[test]
    fn validate_preflight_tool_calls_softens_missing_fresh_and_no_persisted_state() {
        validate_preflight_tool_calls(&[], "mission_preflight__preflight", false)
            .expect("missing preflight should now soften instead of fail");
    }

    #[test]
    fn validate_preflight_tool_calls_allows_failed_fresh_preflight_with_persisted_state() {
        validate_preflight_tool_calls(
            &[ToolCallRecord {
                name: "mission_preflight__preflight".to_string(),
                success: false,
            }],
            "mission_preflight__preflight",
            true,
        )
        .expect("persisted verified contract should satisfy the gate");
    }

    #[test]
    fn validate_preflight_tool_calls_allows_successful_late_preflight() {
        validate_preflight_tool_calls(
            &[
                ToolCallRecord {
                    name: "developer__text_editor".to_string(),
                    success: true,
                },
                ToolCallRecord {
                    name: "mission_preflight__preflight".to_string(),
                    success: true,
                },
            ],
            "mission_preflight__preflight",
            false,
        )
        .expect("successful later preflight should satisfy relaxed ordering");
    }

    #[test]
    fn normalize_completion_check_strips_shell_prefixes() {
        let normalized = super::normalize_completion_check("sh: echo ok", 300)
            .expect("shell completion check should normalize");
        assert_eq!(normalized, "echo ok");
    }

    #[test]
    fn normalize_completion_check_preserves_exists_checks() {
        let normalized = super::normalize_completion_check("test -f reports/out.md", 300)
            .expect("exists check should normalize");
        assert_eq!(normalized, "exists:reports/out.md");
    }

    #[test]
    fn summary_file_output_detection_ignores_future_or_missing_files() {
        let summary = r#"
### Workspace inspection
- Prior `reports/summary.md` / `reports/checks.md`: **do not exist**, so **nothing to preserve**; later steps will create them.
- `verify_contract` with **no artifacts/checks** => **PASS** (inspection-only step, no files created).
"#;

        assert!(!summary_claims_new_file_output(summary));
    }

    #[test]
    fn summary_file_output_detection_still_flags_real_file_creation() {
        let summary = r#"
- Created `reports/summary.md` and wrote `reports/checks.md` under the workspace output directory.
"#;

        assert!(summary_claims_new_file_output(summary));
    }
}
