use agime::agents::harness::{
    build_subagent_bootstrap_call, build_subagent_downgrade_message, build_swarm_downgrade_message,
    SubagentBootstrapRequest,
};

#[test]
fn subagent_bootstrap_prefers_locked_target_and_scopes_to_it() {
    let request = SubagentBootstrapRequest {
        locked_target: Some("src/report.md".to_string()),
        node_target_artifacts: vec!["src/ignored.md".to_string()],
        node_result_contract: vec!["src/report.md".to_string()],
        parent_write_scope: vec!["src/".to_string()],
        ..Default::default()
    };

    let call = build_subagent_bootstrap_call(&request).expect("call");
    assert_eq!(call.target_artifact.as_deref(), Some("src/report.md"));
    assert_eq!(call.write_scope, vec!["src/report.md".to_string()]);
}

#[test]
fn subagent_downgrade_message_uses_missing_artifact_hint() {
    let message =
        build_subagent_downgrade_message(None, &["docs/out.md".to_string()], "worker crashed");
    assert!(message.contains("docs/out.md"));
}

#[test]
fn swarm_downgrade_message_uses_locked_target_hint() {
    let message =
        build_swarm_downgrade_message(Some("src/a.rs"), &["docs/out.md".to_string()], "no delta");
    assert!(message.contains("src/a.rs"));
}
