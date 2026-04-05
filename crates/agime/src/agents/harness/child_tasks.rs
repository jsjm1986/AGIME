use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::delegation::DelegationMode;
use super::task_runtime::TaskKind;

#[derive(Debug, Clone, Default)]
pub struct ChildTaskRequest {
    pub instructions: String,
    pub spec_name: Option<String>,
    pub summary_only: bool,
    pub validation_mode: bool,
    pub requested_extensions: Option<Vec<String>>,
    pub requested_write_scope: Vec<String>,
    pub delegation_mode: DelegationMode,
    pub target_artifacts: Vec<String>,
    pub result_contract: Vec<String>,
    pub parallelism_budget: Option<u32>,
    pub swarm_budget: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Passed,
    Failed,
    #[default]
    NotRun,
}

impl ValidationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::NotRun => "not_run",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub status: ValidationStatus,
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    pub validator_task_id: Option<String>,
    pub target_artifacts: Vec<String>,
    pub evidence_summary: Option<String>,
    #[serde(default)]
    pub content_accessed: bool,
    #[serde(default)]
    pub analysis_complete: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct StructuredValidationResult {
    status: String,
    summary: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    reason_code: Option<String>,
    #[serde(default)]
    target_artifacts: Vec<String>,
    #[serde(default)]
    content_accessed: bool,
    #[serde(default)]
    analysis_complete: bool,
}

impl ValidationReport {
    pub fn passed(&self) -> bool {
        self.status == ValidationStatus::Passed
    }

    pub fn failure_reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    pub fn evidence_summary(&self) -> Option<&str> {
        self.evidence_summary.as_deref()
    }

    pub fn with_validator_context(
        mut self,
        validator_task_id: Option<String>,
        target_artifacts: Vec<String>,
    ) -> Self {
        self.validator_task_id = validator_task_id;
        self.target_artifacts = target_artifacts;
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChildTaskResultClassification {
    pub accepted_targets: Vec<String>,
    pub produced_delta: bool,
    pub validation_outcome: Option<ValidationReport>,
}

pub fn build_swarm_worker_instructions(
    base_instructions: &str,
    target_artifact: &str,
    result_contract: &[String],
) -> String {
    let mut lines = vec![format!(
        "Bounded worker objective: create or materially update `{}` in this run.",
        target_artifact
    )];
    if !result_contract.is_empty() {
        lines.push(format!(
            "Result contract for this worker: {}",
            result_contract.join(", ")
        ));
    }
    lines.push(
        "Work only on the bounded target for this worker. Return a concise final summary of concrete output."
            .to_string(),
    );
    lines.push(base_instructions.trim().to_string());
    lines.join("\n\n")
}

pub fn build_validation_worker_instructions(
    target_artifact: &str,
    result_contract: &[String],
    base_summary: &str,
) -> String {
    let mut lines = vec![format!(
        "Act as an independent validation worker for `{}`.",
        target_artifact
    )];
    if !result_contract.is_empty() {
        lines.push(format!(
            "Validate this result contract: {}",
            result_contract.join(", ")
        ));
    }
    if !base_summary.trim().is_empty() {
        lines.push(format!(
            "Primary worker summary to cross-check:\n{}",
            base_summary.trim()
        ));
    }
    lines.push(
        "Inspect the artifact independently. Begin your final response with either `PASS:` or `FAIL:`. Prefer read-only verification and only suggest fixes when necessary."
            .to_string(),
    );
    lines.join("\n\n")
}

pub fn parse_validation_outcome(summary: &str) -> ValidationReport {
    let trimmed = summary.trim();
    if let Ok(structured) = serde_json::from_str::<StructuredValidationResult>(trimmed) {
        let status = if structured.status.eq_ignore_ascii_case("passed") {
            ValidationStatus::Passed
        } else if structured.status.eq_ignore_ascii_case("failed") {
            ValidationStatus::Failed
        } else {
            ValidationStatus::NotRun
        };
        return ValidationReport {
            status,
            reason: if status == ValidationStatus::Passed {
                None
            } else {
                structured
                    .reason
                    .or_else(|| Some(structured.summary.clone()))
            },
            reason_code: structured.reason_code,
            validator_task_id: None,
            target_artifacts: structured.target_artifacts,
            evidence_summary: Some(structured.summary),
            content_accessed: structured.content_accessed,
            analysis_complete: structured.analysis_complete,
        };
    }
    let verdict_line = trimmed.lines().find_map(|line| {
        let normalized = line
            .trim()
            .trim_start_matches(|ch: char| {
                ch.is_whitespace() || matches!(ch, '*' | '_' | '`' | '#' | '>' | '-')
            })
            .trim_start();
        if normalized
            .get(..5)
            .map(|prefix| prefix.eq_ignore_ascii_case("PASS:"))
            .unwrap_or(false)
        {
            Some("pass")
        } else if normalized
            .get(..5)
            .map(|prefix| prefix.eq_ignore_ascii_case("FAIL:"))
            .unwrap_or(false)
        {
            Some("fail")
        } else {
            None
        }
    });
    let pass = verdict_line == Some("pass");
    let fail = verdict_line == Some("fail");

    ValidationReport {
        status: if pass {
            ValidationStatus::Passed
        } else if fail {
            ValidationStatus::Failed
        } else {
            ValidationStatus::NotRun
        },
        reason: if pass {
            None
        } else if fail {
            Some(trimmed.to_string())
        } else {
            Some(format!(
                "Validation worker returned malformed summary: {}",
                trimmed
            ))
        },
        reason_code: if pass {
            Some("validated".to_string())
        } else if fail {
            Some("validation_failed".to_string())
        } else {
            Some("validation_malformed_summary".to_string())
        },
        validator_task_id: None,
        target_artifacts: Vec::new(),
        evidence_summary: Some(trimmed.to_string()),
        content_accessed: false,
        analysis_complete: pass,
    }
}

pub fn validation_response_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "status": {
                "type": "string",
                "enum": ["passed", "failed", "not_run"]
            },
            "summary": { "type": "string" },
            "reason": { "type": "string" },
            "reason_code": { "type": "string" },
            "content_accessed": { "type": "boolean" },
            "analysis_complete": { "type": "boolean" },
            "target_artifacts": {
                "type": "array",
                "items": { "type": "string" }
            }
        },
        "required": ["status", "summary", "target_artifacts", "content_accessed", "analysis_complete"]
    })
}

pub fn summary_indicates_material_progress(summary: &str) -> bool {
    let lower = summary.to_ascii_lowercase();
    let negative_markers = [
        "no changes",
        "did not change",
        "did not modify",
        "analysis only",
        "unable to complete",
        "blocked",
        "no material update",
        "no file changes",
        "dry run",
        "pass:",
        "fail:",
    ];
    !negative_markers.iter().any(|marker| lower.contains(marker))
}

pub fn classify_child_task_result(
    kind: TaskKind,
    target_artifacts: &[String],
    effective_write_scope: &[String],
    summary: &str,
) -> ChildTaskResultClassification {
    if kind == TaskKind::ValidationWorker {
        return ChildTaskResultClassification {
            accepted_targets: Vec::new(),
            produced_delta: false,
            validation_outcome: Some(parse_validation_outcome(summary)),
        };
    }

    let produced_delta = summary_indicates_material_progress(summary);
    let accepted_targets = if produced_delta {
        if target_artifacts.is_empty() {
            effective_write_scope.to_vec()
        } else {
            target_artifacts.to_vec()
        }
    } else {
        Vec::new()
    };

    ChildTaskResultClassification {
        accepted_targets,
        produced_delta,
        validation_outcome: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_validation_outcome_accepts_markdown_wrapped_pass_prefix() {
        let outcome = parse_validation_outcome("**PASS:** artifact verified");
        assert!(outcome.passed());
        assert!(outcome.failure_reason().is_none());
    }

    #[test]
    fn parse_validation_outcome_accepts_markdown_wrapped_fail_prefix() {
        let outcome = parse_validation_outcome("> FAIL: artifact missing");
        assert!(!outcome.passed());
        assert!(outcome.failure_reason().is_some());
    }

    #[test]
    fn parse_validation_outcome_accepts_pass_on_later_line() {
        let outcome =
            parse_validation_outcome("Independent inspection complete.\n\nPASS: artifact verified");
        assert!(outcome.passed());
        assert!(outcome.failure_reason().is_none());
    }

    #[test]
    fn parse_validation_outcome_accepts_structured_json() {
        let outcome = parse_validation_outcome(
            r#"{"status":"failed","summary":"validator blocked","reason":"missing section","reason_code":"missing_required_section","target_artifacts":["docs/a.md"],"content_accessed":true,"analysis_complete":false}"#,
        );
        assert_eq!(outcome.status, ValidationStatus::Failed);
        assert_eq!(outcome.failure_reason(), Some("missing section"));
        assert_eq!(
            outcome.reason_code.as_deref(),
            Some("missing_required_section")
        );
        assert_eq!(outcome.target_artifacts, vec!["docs/a.md".to_string()]);
        assert_eq!(outcome.evidence_summary(), Some("validator blocked"));
        assert!(outcome.content_accessed);
        assert!(!outcome.analysis_complete);
    }

    #[test]
    fn classify_child_task_result_does_not_accept_targets_without_material_delta() {
        let result = classify_child_task_result(
            TaskKind::SwarmWorker,
            &["docs/out.md".to_string()],
            &["docs".to_string()],
            "analysis only, no changes",
        );
        assert!(result.accepted_targets.is_empty());
        assert!(!result.produced_delta);
    }
}
