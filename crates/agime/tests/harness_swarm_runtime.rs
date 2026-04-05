use agime::agents::harness::{
    bootstrap_validation_workers, build_bounded_swarm_plan, decide_bounded_swarm_outcome,
    DelegationMode, DelegationRuntimeState, SwarmBudget, SwarmPlan, TaskRuntime,
};

#[tokio::test]
async fn bounded_swarm_plan_respects_budget() {
    let delegation = DelegationRuntimeState::new(
        DelegationMode::Swarm,
        0,
        2,
        vec!["src/".to_string()],
        vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
        vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
    );
    let plan = SwarmPlan {
        budget: SwarmBudget {
            parallelism_budget: Some(1),
        },
        targets: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
        write_scope: vec!["src/".to_string()],
        result_contract: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
        validation_mode: false,
    };

    let runtime_plan = build_bounded_swarm_plan("session-1", &delegation, &plan, false);
    assert_eq!(runtime_plan.parallelism_budget, 1);
    assert_eq!(runtime_plan.workers.len(), 2);
    assert_eq!(
        runtime_plan.workers[0].write_scope,
        vec!["src/a.rs".to_string()]
    );
    assert_eq!(
        runtime_plan.workers[0].result_contract,
        vec!["src/a.rs".to_string()]
    );
    assert_eq!(
        runtime_plan.workers[1].write_scope,
        vec!["src/b.rs".to_string()]
    );
    assert_eq!(
        runtime_plan.workers[1].result_contract,
        vec!["src/b.rs".to_string()]
    );
}

#[tokio::test]
async fn validation_workers_can_be_bootstrapped() {
    let delegation = DelegationRuntimeState::new(
        DelegationMode::Swarm,
        0,
        2,
        vec!["docs/".to_string()],
        vec!["docs/out.md".to_string()],
        vec!["docs/out.md".to_string()],
    );
    let plan = SwarmPlan {
        budget: SwarmBudget {
            parallelism_budget: Some(2),
        },
        targets: vec!["docs/out.md".to_string()],
        write_scope: vec!["docs/".to_string()],
        result_contract: vec!["docs/out.md".to_string()],
        validation_mode: true,
    };
    let runtime_plan = build_bounded_swarm_plan("session-2", &delegation, &plan, true);
    let runtime = TaskRuntime::default();
    let spawned = bootstrap_validation_workers(&runtime, "session-2", 1, &runtime_plan)
        .await
        .expect("spawned");
    assert_eq!(spawned.len(), 1);
}

#[test]
fn swarm_outcome_downgrades_when_no_delta() {
    let outcome = decide_bounded_swarm_outcome(
        &["src/a.rs".to_string()],
        &[],
        Some("no files changed".to_string()),
    );
    assert!(!outcome.produced_delta);
    assert!(outcome.accepted_targets.is_empty());
    assert!(outcome.downgrade_message.is_some());
}
