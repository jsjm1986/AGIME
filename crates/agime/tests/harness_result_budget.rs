use agime::agents::harness::{
    apply_tool_result_budget, ResultBudgetAction, ToolResultBudget, ToolResultBudgetBucket,
};
use rmcp::model::{CallToolResult, Content};

#[tokio::test]
async fn small_result_stays_inline() {
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
    )
    .await;

    assert!(budgeted.handle.is_none());
    assert_eq!(budgeted.action, ResultBudgetAction::Inline);
}

#[tokio::test]
async fn long_result_gets_handle_and_replacement() {
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
    )
    .await;

    let handle = budgeted.handle.expect("handle");
    assert_eq!(handle.original_length, large.chars().count());
    assert!(matches!(
        budgeted.action,
        ResultBudgetAction::Truncate
            | ResultBudgetAction::Summarize
            | ResultBudgetAction::ReplaceWithHandle
    ));
}

#[tokio::test]
async fn truncate_preview_respects_effective_limit() {
    let budget = ToolResultBudget::default();
    let large = "x".repeat(300);
    let result = Ok(CallToolResult::success(vec![Content::text(large)]));

    let budgeted = apply_tool_result_budget(
        "req-3",
        "search",
        ToolResultBudgetBucket::Small,
        Some(128),
        result,
        &budget,
    )
    .await;

    assert_eq!(budgeted.action, ResultBudgetAction::Truncate);

    let preview = budgeted
        .result
        .as_ref()
        .ok()
        .and_then(|result| result.content.iter().find_map(|content| content.as_text()))
        .map(|text| text.text.clone())
        .expect("preview text");

    assert!(preview.contains("Preview:"));
    assert!(!preview.contains("Handle:"));
    assert!(preview.len() < 512);
}

#[tokio::test]
async fn oversized_result_prefers_summary_over_dead_handle_only_fallback() {
    let budget = ToolResultBudget::default();
    let huge = "x".repeat(100_000);
    let result = Ok(CallToolResult::success(vec![Content::text(huge)]));

    let budgeted = apply_tool_result_budget(
        "req-4",
        "search",
        ToolResultBudgetBucket::Small,
        Some(128),
        result,
        &budget,
    )
    .await;

    assert_eq!(budgeted.action, ResultBudgetAction::Summarize);

    let preview = budgeted
        .result
        .as_ref()
        .ok()
        .and_then(|result| result.content.iter().find_map(|content| content.as_text()))
        .map(|text| text.text.clone())
        .expect("summary text");

    assert!(preview.contains("Summary:"));
    assert!(!preview.contains("Handle:"));
    assert!(!preview.contains("replaced with stored handle"));
}
