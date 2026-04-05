use agime::agents::harness::{
    record_transition, shared_transition_trace, HarnessMode, TransitionKind,
};
use std::collections::BTreeMap;

#[tokio::test]
async fn transition_trace_records_mode_and_budget_events() {
    let trace = shared_transition_trace();
    record_transition(
        &trace,
        1,
        HarnessMode::Execute,
        TransitionKind::ModeTransition,
        "execute_to_repair",
        BTreeMap::new(),
    )
    .await;
    record_transition(
        &trace,
        2,
        HarnessMode::Repair,
        TransitionKind::ToolBudgetFallback,
        "tool_result_budget_applied",
        BTreeMap::new(),
    )
    .await;

    let trace = trace.lock().await;
    assert_eq!(trace.records.len(), 2);
    assert_eq!(
        trace.latest().expect("latest").kind,
        TransitionKind::ToolBudgetFallback
    );
}
