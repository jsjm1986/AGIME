use agime::agents::harness::{
    apply_tool_result_budget, ContentReplacementState, InMemoryToolResultStore, ResultBudgetAction,
    ToolResultBudget, ToolResultBudgetBucket, ToolResultStore,
};
use rmcp::model::{CallToolResult, Content};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::test]
async fn small_result_stays_inline() {
    let store = InMemoryToolResultStore::default();
    let replacement_state = Arc::new(Mutex::new(ContentReplacementState::default()));
    let budget = ToolResultBudget::default();
    let result = Ok(CallToolResult::success(vec![Content::text(
        "short".to_string(),
    )]));

    let budgeted = apply_tool_result_budget(
        "req-1",
        "search",
        ToolResultBudgetBucket::Standard,
        Some(64),
        result,
        &budget,
        &store,
        &replacement_state,
    )
    .await;

    assert!(budgeted.handle.is_none());
    assert_eq!(budgeted.action, ResultBudgetAction::Inline);
}

#[tokio::test]
async fn long_result_gets_handle_and_replacement() {
    let store = InMemoryToolResultStore::default();
    let replacement_state = Arc::new(Mutex::new(ContentReplacementState::default()));
    let budget = ToolResultBudget::default();
    let large = "x".repeat(5000);
    let result = Ok(CallToolResult::success(vec![Content::text(large.clone())]));

    let budgeted = apply_tool_result_budget(
        "req-2",
        "search",
        ToolResultBudgetBucket::Small,
        Some(128),
        result,
        &budget,
        &store,
        &replacement_state,
    )
    .await;

    let handle = budgeted.handle.expect("handle");
    let stored = store.load(&handle.id).expect("stored original");
    assert_eq!(stored, large);
    assert!(matches!(
        budgeted.action,
        ResultBudgetAction::Truncate
            | ResultBudgetAction::Summarize
            | ResultBudgetAction::ReplaceWithHandle
    ));
}
