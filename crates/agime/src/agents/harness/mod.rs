pub mod checkpoints;
pub mod child_tasks;
pub mod compaction;
pub mod completion;
pub mod context;
pub mod control_protocol;
pub mod control_types;
pub mod coordinator;
pub mod coordinator_runtime;
pub mod delegation;
pub mod finalize;
pub mod host;
pub mod host_state;
pub mod loop_;
pub mod mailbox;
pub mod permission_bridge;
pub mod planner;
pub mod planner_upgrade;
pub mod policy;
pub mod provider_turn;
pub mod reply_bootstrap;
pub mod result_budget;
pub mod runtime_state;
pub mod scratchpad;
pub mod send_message_tool;
pub mod signals;
pub mod state;
pub mod swarm_runtime;
pub mod swarm_tool;
pub mod task_runtime;
pub mod tool_scheduler;
pub mod tools;
pub mod transcript;
pub mod transition;

pub use checkpoints::{HarnessCheckpoint, HarnessCheckpointKind};
pub use child_tasks::{
    build_swarm_worker_instructions, build_validation_worker_instructions,
    classify_child_task_result, parse_validation_outcome, summary_indicates_material_progress,
    ChildExecutionExpectation, ChildTaskRequest, ChildTaskResultClassification, ValidationReport,
    ValidationStatus,
};
pub use compaction::{
    auto_compaction_inline_message, compaction_checkpoint, compaction_strategy_label,
    execute_recovery_compaction, max_compaction_attempts_for_context_runtime,
    recovery_compaction_abort_message, recovery_compaction_inline_message,
    recovery_compaction_prelude_messages, RecoveryCompactionExecution,
};
pub use completion::{
    build_conversation_completion_report, build_execute_completion_report,
    build_system_document_analysis_completion_report, derive_execute_completion_outcome,
    format_execution_host_completion_text, normalize_execution_host_completion_report,
    normalize_system_document_analysis_completion_report, parse_execution_host_completion_report,
    CompletionSurfacePolicy, ExecuteCompletionOutcome, ExecuteCompletionState,
    ExecutionHostCompletionReport,
};
pub use context::{HarnessContext, HarnessWorkerRuntimeContext};
pub use control_protocol::{
    build_completion_control_messages, build_permission_requested_control_message,
    build_permission_resolved_control_message, build_permission_timed_out_control_message,
    build_session_finished_message, build_session_started_message,
    build_tool_finished_control_message, build_tool_started_control_message,
    build_worker_finished_control_message, build_worker_followup_requested_control_message,
    build_worker_idle_control_message, build_worker_progress_control_message,
    build_worker_started_control_message, completion_surface_label,
    control_messages_for_agent_event, control_sequencer_for_session, control_timestamp_ms,
    register_harness_control_sequencer, tool_invocation_surface_label, tool_transport_label,
    unregister_harness_control_sequencer, HarnessControlSequencer, HarnessControlSink,
    NoopHarnessControlSink,
};
pub use control_types::{
    CompletionControlEvent, HarnessControlEnvelope, HarnessControlMessage, PermissionControlEvent,
    RuntimeControlEvent, SessionControlEvent, ToolControlEvent, WorkerControlEvent,
};
pub use coordinator::{
    leader_permission_bridge_enabled, native_swarm_tool_enabled, planner_auto_swarm_enabled,
    sanitize_worker_name, swarm_scratchpad_enabled, worker_name_for_target, CoordinatorRole,
    SwarmExecutionRequest, SwarmExecutionResult, WorkerIdentity,
};
pub use coordinator_runtime::execute_swarm_request;
pub use delegation::{
    bounded_subagent_depth_from_env, build_subagent_bootstrap_call,
    build_subagent_downgrade_message, build_swarm_downgrade_message,
    detect_explicit_delegation_intent, maybe_build_explicit_delegation_bootstrap_request,
    DelegationMode, DelegationRuntimeState, SubagentBootstrapCall, SubagentBootstrapRequest,
    SwarmBudget, SwarmOutcome, SwarmPlan,
};
pub use host::{
    run_harness_host, HarnessEventSink, HarnessHostDependencies, HarnessHostRequest,
    HarnessHostResult, HarnessPersistenceAdapter, NoopHarnessEventSink,
    NoopHarnessPersistenceAdapter,
};
pub use host_state::{
    load_host_session_state, save_host_session_state, update_host_completion_outcome,
    update_host_notification_summary, update_host_signal_summary, update_host_transition_trace,
    HarnessHostSessionState,
};
pub use loop_::HarnessRunLoop;
pub use mailbox::{
    drain_unread_messages_from_root, mailbox_dir, mailbox_path, mark_all_read, read_mailbox,
    read_unread_messages, read_unread_messages_from_root, write_to_mailbox,
    write_to_mailbox_from_root, MailboxMessage, MailboxMessageKind,
};
pub use permission_bridge::{
    auto_resolve_request, await_permission_resolution, clear_permission_bridge_resolver,
    list_pending_requests, permission_decision_source_label, permission_resolution_feedback,
    resolve_permission_request, resolve_permission_request_via_policy,
    set_permission_bridge_resolver, to_permission_confirmation, write_permission_request,
    PermissionBridgeRequest, PermissionBridgeResolution, PermissionBridgeResolver,
    PermissionDecisionSource,
};
pub use planner::{
    build_mode_transition_notification, mode_system_prompt, next_mode_after_turn,
    next_mode_after_turn_with_final_output, no_tool_turn_action,
    no_tool_turn_action_with_final_output, parse_harness_mode_command,
    resolve_post_turn_transition, transition_action, HarnessModeCommand, ModeTransition,
    NoToolTurnAction, PostTurnTransition, TransitionAction,
};
pub use planner_upgrade::{
    build_auto_swarm_instructions, build_auto_swarm_tool_request, maybe_plan_swarm_upgrade,
    PlannerUpgradeDecision,
};
pub use policy::{
    apply_runtime_policy, format_runtime_policy_denial, HarnessPolicy, PolicyDecision,
};
pub use result_budget::{
    apply_tool_result_budget, ResultBudgetAction, ToolResultBudget, ToolResultBudgetBucket,
    ToolResultHandle,
};
pub use runtime_state::{
    load_worker_runtime_state, save_worker_runtime_state, HarnessWorkerRuntimeState,
};
pub use scratchpad::SwarmScratchpad;
pub use send_message_tool::{
    create_send_message_tool, handle_send_message_tool, SEND_MESSAGE_TOOL_NAME,
};
pub use signals::{
    mailbox_message_to_notification, spawn_task_runtime_signal_bridge, CoordinatorNotification,
    CoordinatorSignal, CoordinatorSignalStore, CoordinatorSignalSummary, NotificationDrainResult,
    NotificationQueue, RuntimeNotificationInput, SharedCoordinatorSignalStore,
    StructuredCompletionSignal, SwarmCompletionEvidence, ValidationSignalOutcome, WorkerOutcome,
};
pub use state::{CoordinatorExecutionMode, HarnessMode, HarnessState, ProviderTurnMode};
pub use swarm_runtime::{
    bootstrap_validation_workers, build_bounded_swarm_plan, decide_bounded_swarm_outcome,
    worker_task_spec, SwarmRuntimePlan, SwarmWorkerSpec,
};
pub use swarm_tool::{create_swarm_tool, handle_swarm_tool, SwarmParams, SWARM_TOOL_NAME};
pub use task_runtime::{
    register_task_runtime, task_runtime_for_session, unregister_task_runtime, TaskHandle, TaskKind,
    TaskResultEnvelope, TaskRuntime, TaskRuntimeEvent, TaskRuntimeHost, TaskSnapshot, TaskSpec,
    TaskStatus, WorkerAttemptIdentity,
};
pub use tool_scheduler::{
    build_scheduled_tool_calls, partition_tool_calls, ToolBatch, ToolBatchMode, ToolBatchResult,
};
pub use tools::{
    classify_tool_transport_kind, infer_runtime_tool_meta, partition_frontend_transports,
    snapshot_final_output_state, tool_invocation_surface, FinalOutputState,
    FrontendTransportPartition, HarnessToolResult, InterruptBehavior, RuntimeToolMeta,
    ScheduledToolCall, ToolExecutionMode, ToolInvocationSurface, ToolResponsePlan,
    ToolTransportDispatch, ToolTransportKind, TransportDispatchPlan, TransportDispatcher,
};
pub use transcript::{
    annotate_task_ledger_child_evidence_view, annotate_task_ledger_child_session,
    annotate_task_ledger_child_session_excerpt, annotate_task_ledger_child_session_preview,
    annotate_task_ledger_child_transcript_resume, build_child_transcript_excerpt,
    build_child_transcript_resume_lines, has_active_persisted_tasks,
    load_task_ledger_evidence_view, load_task_ledger_state, load_task_ledger_summary,
    load_task_ledger_transcript_resume_view, preferred_child_evidence_detail,
    render_child_evidence_line, select_replayable_child_evidence,
    select_replayable_child_transcript_resume, summarize_task_ledger_state,
    upsert_task_ledger_snapshot, ChildTranscriptResumeSelection, HarnessCheckpointStore,
    HarnessSessionState, HarnessTaskLedgerState, HarnessTranscriptStore,
    PersistedChildEvidenceItem, PersistedChildTranscriptView, PersistedTaskLedgerSummary,
    SessionHarnessStore,
};
pub use transition::{
    record_transition, shared_transition_trace, QueryTrackingState, SharedTransitionTrace,
    TransitionKind, TransitionRecord, TransitionTrace,
};
