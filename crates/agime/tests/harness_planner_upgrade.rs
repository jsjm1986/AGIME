use agime::agents::harness::{
    auto_resolve_request, build_auto_swarm_tool_request, maybe_plan_swarm_upgrade,
    CoordinatorExecutionMode, DelegationMode, DelegationRuntimeState, HarnessMode,
    PermissionBridgeRequest, SWARM_TOOL_NAME,
};
use agime::conversation::message::Message;
use agime::conversation::Conversation;
use agime::permission::Permission;
use rmcp::model::JsonObject;

#[test]
fn planner_upgrade_prefers_stable_result_contract_targets() {
    std::env::set_var("AGIME_ENABLE_SWARM_PLANNER_AUTO", "true");
    let delegation = DelegationRuntimeState::new(
        DelegationMode::Swarm,
        0,
        1,
        vec!["docs".to_string()],
        vec!["draft alpha".to_string(), "draft beta".to_string()],
        vec!["docs/a.md".to_string(), "docs/b.md".to_string()],
    );
    let decision = maybe_plan_swarm_upgrade(
        HarnessMode::Conversation,
        CoordinatorExecutionMode::AutoSwarm,
        &Conversation::new_unvalidated(vec![
            Message::user().with_text("split the bounded docs work")
        ]),
        &delegation,
    );
    let plan = decision.plan.expect("expected swarm plan");
    assert_eq!(
        plan.targets,
        vec!["docs/a.md".to_string(), "docs/b.md".to_string()]
    );
    std::env::remove_var("AGIME_ENABLE_SWARM_PLANNER_AUTO");
}

#[test]
fn planner_upgrade_stays_disabled_after_swarm_fallback() {
    std::env::set_var("AGIME_ENABLE_SWARM_PLANNER_AUTO", "true");
    let mut delegation = DelegationRuntimeState::new(
        DelegationMode::Swarm,
        0,
        1,
        vec!["docs".to_string()],
        vec!["docs/a.md".to_string(), "docs/b.md".to_string()],
        vec!["docs/a.md".to_string(), "docs/b.md".to_string()],
    );
    delegation.note_swarm_fallback("previous fan-out produced no delta");
    let decision = maybe_plan_swarm_upgrade(
        HarnessMode::Conversation,
        CoordinatorExecutionMode::AutoSwarm,
        &Conversation::new_unvalidated(vec![Message::user().with_text("try again")]),
        &delegation,
    );
    assert!(decision.plan.is_none());
    std::env::remove_var("AGIME_ENABLE_SWARM_PLANNER_AUTO");
}

#[test]
fn validation_worker_permission_policy_denies_mutation() {
    let resolution = auto_resolve_request(&PermissionBridgeRequest {
        request_id: "req-1".to_string(),
        run_id: "run-1".to_string(),
        worker_name: "validator-1".to_string(),
        logical_worker_id: None,
        attempt_id: None,
        attempt_index: None,
        previous_task_id: None,
        tool_name: "write_file".to_string(),
        tool_use_id: "tool-1".to_string(),
        description: "write".to_string(),
        arguments: JsonObject::default(),
        tool_input_snapshot: None,
        write_scope: vec!["docs/out.md".to_string()],
        validation_mode: true,
        permission_policy: None,
        created_at: chrono::Utc::now().to_rfc3339(),
    });
    assert_eq!(resolution.permission, Permission::DenyOnce);
}

#[test]
fn planner_upgrade_can_build_auto_swarm_tool_request() {
    std::env::set_var("AGIME_ENABLE_SWARM_PLANNER_AUTO", "true");
    let delegation = DelegationRuntimeState::new(
        DelegationMode::Swarm,
        0,
        1,
        vec!["docs".to_string()],
        vec!["docs/a.md".to_string(), "docs/b.md".to_string()],
        vec!["docs/a.md".to_string(), "docs/b.md".to_string()],
    );
    let conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("请并行产出 docs/a.md 和 docs/b.md")
    ]);
    let decision = maybe_plan_swarm_upgrade(
        HarnessMode::Conversation,
        CoordinatorExecutionMode::AutoSwarm,
        &conversation,
        &delegation,
    );
    let request = build_auto_swarm_tool_request(
        &conversation,
        &Message::assistant().with_text("我会并行推进这两个目标。"),
        &decision,
    )
    .expect("auto request");
    let tool_call = request.tool_call.expect("tool call");
    assert_eq!(tool_call.name.as_ref(), SWARM_TOOL_NAME);
    std::env::remove_var("AGIME_ENABLE_SWARM_PLANNER_AUTO");
}

#[test]
fn planner_upgrade_requires_auto_swarm_execution_mode() {
    std::env::set_var("AGIME_ENABLE_SWARM_PLANNER_AUTO", "true");
    let delegation = DelegationRuntimeState::new(
        DelegationMode::Swarm,
        0,
        1,
        vec!["docs".to_string()],
        vec!["docs/a.md".to_string(), "docs/b.md".to_string()],
        vec!["docs/a.md".to_string(), "docs/b.md".to_string()],
    );
    let conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("请并行产出 docs/a.md 和 docs/b.md")
    ]);
    let decision = maybe_plan_swarm_upgrade(
        HarnessMode::Conversation,
        CoordinatorExecutionMode::ExplicitSwarm,
        &conversation,
        &delegation,
    );
    assert!(decision.plan.is_none());
    std::env::remove_var("AGIME_ENABLE_SWARM_PLANNER_AUTO");
}
