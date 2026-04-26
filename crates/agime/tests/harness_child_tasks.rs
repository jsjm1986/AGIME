use agime::agents::harness::{
    build_swarm_worker_instructions, build_validation_worker_instructions,
    classify_child_task_result, parse_validation_outcome, ChildExecutionExpectation,
    ValidationStatus,
};

#[test]
fn validation_outcome_requires_pass_prefix() {
    assert!(parse_validation_outcome("PASS: looks good").passed());
    assert!(parse_validation_outcome("**PASS:** looks good").passed());
    assert!(parse_validation_outcome("Verification complete.\nPASS: looks good").passed());
    assert!(!parse_validation_outcome("FAIL: missing section").passed());
    assert!(!parse_validation_outcome("> FAIL: missing section").passed());
    assert!(!parse_validation_outcome("looks good").passed());
}

#[test]
fn structured_validation_result_takes_priority_over_text_fallback() {
    let report = parse_validation_outcome(
        r#"{"status":"failed","summary":"validator blocked","reason":"missing contract","target_artifacts":["docs/a.md"]}"#,
    );
    assert_eq!(report.status, ValidationStatus::Failed);
    assert_eq!(report.failure_reason(), Some("missing contract"));
    assert_eq!(report.target_artifacts, vec!["docs/a.md".to_string()]);
}

#[test]
fn classify_validation_worker_never_produces_delta() {
    let classification = classify_child_task_result(
        ChildExecutionExpectation::Validation,
        &["src/out.md".to_string()],
        &["src/".to_string()],
        "PASS: verified",
    );
    assert!(!classification.produced_delta);
    assert!(classification.accepted_targets.is_empty());
    assert!(classification.validation_outcome.is_some());
}

#[test]
fn worker_instruction_helpers_include_contract() {
    let worker = build_swarm_worker_instructions("base", "src/out.md", &["src/out.md".to_string()]);
    let validation =
        build_validation_worker_instructions("src/out.md", &["src/out.md".to_string()], "PASS: ok");
    assert!(worker.contains("src/out.md"));
    assert!(validation.contains("PASS: ok"));
}

#[test]
fn validation_worker_instructions_explain_document_targets() {
    let validation = build_validation_worker_instructions(
        "document:doc-smoke",
        &["document:doc-smoke".to_string()],
        "The primary worker read the document.",
    );

    assert!(validation.contains("document_tools__read_document"));
    assert!(validation.contains("read-only document tools"));
    assert!(validation.contains("PASS:"));
    assert!(validation.contains("FAIL:"));
}
