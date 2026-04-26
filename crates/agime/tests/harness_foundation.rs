use agime::agents::final_output_tool::{FinalOutputTool, FINAL_OUTPUT_CONTINUATION_MESSAGE};
use agime::agents::harness::{
    apply_runtime_policy, build_scheduled_tool_calls, next_mode_after_turn,
    next_mode_after_turn_with_final_output, no_tool_turn_action_with_final_output,
    parse_harness_mode_command, partition_tool_calls, resolve_post_turn_transition,
    snapshot_final_output_state, transition_action, CoordinatorExecutionMode,
    DelegationRuntimeState, FinalOutputState, HarnessCheckpoint, HarnessCheckpointKind,
    HarnessMode, HarnessPolicy, HarnessSessionState, NoToolTurnAction, PolicyDecision,
    ToolBatchMode, ToolResponsePlan,
};
use agime::agents::{validate_subagent_runtime_preconditions, SubagentParams, TaskConfig};
use agime::conversation::message::Message;
use agime::conversation::message::ToolRequest;
use agime::model::ModelConfig;
use agime::providers::base::{Provider, ProviderMetadata, ProviderUsage};
use agime::providers::errors::ProviderError;
use agime::recipe::Response;
use rmcp::model::{CallToolRequestParams, Tool, ToolAnnotations};
use rmcp::object;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug)]
struct DummyProvider {
    model_config: ModelConfig,
}

#[async_trait::async_trait]
impl Provider for DummyProvider {
    fn metadata() -> ProviderMetadata
    where
        Self: Sized,
    {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "dummy"
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        panic!(
            "DummyProvider::complete_with_model should not be called in harness foundation tests"
        )
    }

    fn get_model_config(&self) -> ModelConfig {
        self.model_config.clone()
    }
}

fn make_tool(name: &str, read_only: bool, destructive: bool) -> Tool {
    Tool::new(
        name.to_string(),
        "test tool".to_string(),
        object!({"type": "object"}),
    )
    .annotate(ToolAnnotations {
        title: None,
        read_only_hint: Some(read_only),
        destructive_hint: Some(destructive),
        idempotent_hint: Some(read_only && !destructive),
        open_world_hint: Some(false),
    })
}

fn make_request(id: &str, tool_name: &str) -> ToolRequest {
    ToolRequest {
        id: id.to_string(),
        tool_call: Ok(CallToolRequestParams {
            name: tool_name.to_string().into(),
            arguments: Some(object!({})),
            meta: None,
            task: None,
        }),
        thought_signature: None,
    }
}

fn make_task_config() -> TaskConfig {
    TaskConfig::new(
        Arc::new(DummyProvider {
            model_config: ModelConfig::new_or_fail("gpt-4o"),
        }),
        "parent-session",
        Path::new("."),
        Vec::new(),
    )
}

#[test]
fn parses_explicit_mode_commands() {
    assert_eq!(
        parse_harness_mode_command("/plan")
            .expect("plan command should parse")
            .mode,
        HarnessMode::Plan
    );
    assert_eq!(
        parse_harness_mode_command("/execute")
            .expect("execute command should parse")
            .mode,
        HarnessMode::Execute
    );
    assert!(parse_harness_mode_command("/plan now").is_none());
}

#[test]
fn partitions_read_only_tools_into_concurrent_batch() {
    let tools = vec![
        make_tool("read_a", true, false),
        make_tool("read_b", true, false),
        make_tool("write_c", false, true),
    ];
    let requests = vec![
        make_request("1", "read_a"),
        make_request("2", "read_b"),
        make_request("3", "write_c"),
    ];

    let scheduled = build_scheduled_tool_calls(&requests, &tools, |_| false);
    let (batches, meta) = partition_tool_calls(scheduled);

    assert_eq!(batches.len(), 2);
    assert_eq!(meta.concurrent_batch_count, 1);
    assert_eq!(meta.serial_batch_count, 1);
    assert_eq!(batches[0].mode, ToolBatchMode::ConcurrentReadOnly);
    assert_eq!(batches[0].calls.len(), 2);
}

#[test]
fn plan_mode_denies_mutating_tool_calls() {
    let tools = vec![make_tool("write_c", false, true)];
    let requests = vec![make_request("1", "write_c")];
    let scheduled = build_scheduled_tool_calls(&requests, &tools, |_| false);
    let decision = apply_runtime_policy(
        &HarnessPolicy::new(HarnessMode::Plan),
        CoordinatorExecutionMode::SingleWorker,
        &DelegationRuntimeState::default(),
        &scheduled[0].meta,
        &scheduled[0].request,
        0,
    );
    assert!(matches!(decision, PolicyDecision::Deny { .. }));
}

#[test]
fn complete_mode_allows_only_final_output() {
    let tools = vec![
        make_tool("recipe__final_output", false, false),
        make_tool("search", true, false),
    ];
    let requests = vec![
        make_request("1", "recipe__final_output"),
        make_request("2", "search"),
    ];
    let scheduled = build_scheduled_tool_calls(&requests, &tools, |_| false);

    let allow = apply_runtime_policy(
        &HarnessPolicy::new(HarnessMode::Complete),
        CoordinatorExecutionMode::SingleWorker,
        &DelegationRuntimeState::default(),
        &scheduled[0].meta,
        &scheduled[0].request,
        0,
    );
    let deny = apply_runtime_policy(
        &HarnessPolicy::new(HarnessMode::Complete),
        CoordinatorExecutionMode::SingleWorker,
        &DelegationRuntimeState::default(),
        &scheduled[1].meta,
        &scheduled[1].request,
        0,
    );

    assert!(matches!(allow, PolicyDecision::Allow));
    assert!(matches!(deny, PolicyDecision::Deny { .. }));
}

#[test]
fn mode_transition_requests_follow_up_actions() {
    let transition = next_mode_after_turn(HarnessMode::Execute, true, false, false)
        .expect("should transition to repair");
    assert_eq!(transition.to, HarnessMode::Repair);

    let action = transition_action(transition.to, false);
    assert!(action.follow_up_user_message.is_some());
    assert!(!action.should_exit_chat);

    let complete_action = transition_action(HarnessMode::Complete, false);
    assert!(complete_action.should_exit_chat);
}

#[test]
fn final_output_state_snapshot_detects_collected_output() {
    let mut final_output_tool = FinalOutputTool::new(Response {
        json_schema: Some(json!({
            "type": "object",
            "properties": { "ok": { "type": "boolean" } },
            "required": ["ok"]
        })),
    });
    final_output_tool.final_output = Some("{\"ok\":true}".to_string());

    let state = snapshot_final_output_state(Some(&final_output_tool));
    assert!(state.tool_present);
    assert!(state.is_collected());
    assert_eq!(state.collected_output(), Some("{\"ok\":true}"));
}

#[test]
fn no_tool_turn_uses_final_output_state() {
    let action = no_tool_turn_action_with_final_output(
        true,
        false,
        &FinalOutputState {
            tool_present: true,
            collected_output: Some("{\"ok\":true}".to_string()),
        },
        FINAL_OUTPUT_CONTINUATION_MESSAGE,
    );
    assert_eq!(
        action,
        NoToolTurnAction::ExitWithAssistantMessage("{\"ok\":true}".to_string())
    );
}

#[test]
fn next_mode_after_turn_uses_final_output_state() {
    let transition = next_mode_after_turn_with_final_output(
        HarnessMode::Execute,
        false,
        &FinalOutputState {
            tool_present: true,
            collected_output: Some("{\"ok\":true}".to_string()),
        },
        false,
    )
    .expect("should transition to complete");
    assert_eq!(transition.to, HarnessMode::Complete);
}

#[test]
fn resolve_post_turn_transition_returns_notification_and_follow_up() {
    let transition = resolve_post_turn_transition(
        HarnessMode::Execute,
        true,
        &FinalOutputState::default(),
        false,
        FINAL_OUTPUT_CONTINUATION_MESSAGE,
    )
    .expect("should transition to repair");
    assert_eq!(transition.transition.to, HarnessMode::Repair);
    assert!(transition.follow_up_user_message.is_some());
}

#[test]
fn checkpoint_helpers_encode_expected_details() {
    let tool_batch = HarnessCheckpoint::tool_batch(3, HarnessMode::Execute, 2, 1, 1);
    assert_eq!(tool_batch.kind, HarnessCheckpointKind::ToolBatch);
    assert!(tool_batch.detail.contains("batches=2"));

    let turn_end = HarnessCheckpoint::turn_end(4, HarnessMode::Repair, true);
    assert_eq!(turn_end.kind, HarnessCheckpointKind::TurnEnd);
    assert_eq!(turn_end.detail, "turn_end:no_tools_called");
}

#[test]
fn harness_session_state_snapshot_tracks_runtime_fields() {
    let state = HarnessSessionState::snapshot(HarnessMode::Execute, 5, 2, 1);
    assert_eq!(state.mode, HarnessMode::Execute);
    assert_eq!(state.turns_taken, 5);
    assert_eq!(state.runtime_compaction_count, 2);
    assert_eq!(state.delegation_depth, 1);
}

#[test]
fn tool_response_plan_preserves_order_and_lookup() {
    let frontend_requests = vec![make_request("front", "frontend_tool")];
    let remaining_requests = vec![make_request("back", "backend_tool")];
    let plan = ToolResponsePlan::new(&frontend_requests, &remaining_requests);

    let ordered_ids = plan
        .ordered_pairs()
        .into_iter()
        .map(|(request, _)| request.id)
        .collect::<Vec<_>>();

    assert_eq!(ordered_ids, vec!["front".to_string(), "back".to_string()]);
    assert!(plan.response_message_for("front").is_some());
    assert!(plan.response_message_for("back").is_some());
}

#[test]
fn subagent_runtime_preconditions_enforce_depth_limit() {
    let config = make_task_config().with_delegation_depth(1, 1);
    let params = SubagentParams {
        instructions: Some("do work".to_string()),
        subrecipe: None,
        parameters: None,
        extensions: None,
        settings: None,
        summary: true,
    };
    let result = validate_subagent_runtime_preconditions(&config, &params);
    assert!(result.is_err());
}

#[test]
fn subagent_runtime_preconditions_enforce_summary_only() {
    let config = make_task_config().with_summary_only(true);
    let params = SubagentParams {
        instructions: Some("do work".to_string()),
        subrecipe: None,
        parameters: None,
        extensions: None,
        settings: None,
        summary: false,
    };
    let result = validate_subagent_runtime_preconditions(&config, &params);
    assert!(result.is_err());
}

#[test]
fn subagent_runtime_preconditions_reject_paths_outside_write_scope() {
    let config = make_task_config()
        .with_write_scope(vec!["src/generated".to_string()])
        .with_runtime_contract(
            vec!["src/other/output.md".to_string()],
            vec!["src/other/output.md".to_string()],
        );
    let params = SubagentParams {
        instructions: Some("do work".to_string()),
        subrecipe: None,
        parameters: None,
        extensions: None,
        settings: None,
        summary: true,
    };
    let result = validate_subagent_runtime_preconditions(&config, &params);
    assert!(result.is_err());
}

#[test]
fn subagent_runtime_preconditions_allow_paths_inside_write_scope() {
    let config = make_task_config()
        .with_write_scope(vec!["src/generated".to_string()])
        .with_runtime_contract(
            vec!["src/generated/output.md".to_string()],
            vec!["src/generated/output.md".to_string()],
        );
    let params = SubagentParams {
        instructions: Some("do work".to_string()),
        subrecipe: None,
        parameters: None,
        extensions: None,
        settings: None,
        summary: true,
    };
    let result = validate_subagent_runtime_preconditions(&config, &params);
    assert!(result.is_ok());
}
