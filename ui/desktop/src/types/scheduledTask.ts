/**
 * TypeScript types mirroring Rust `crates/agime-server/src/scheduled_tasks/models.rs`.
 * These types correspond to the desktop harness host scheduled tasks API.
 */

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

export type ScheduledTaskKind = 'one_shot' | 'cron';

export type ScheduledTaskStatus = 'draft' | 'active' | 'paused' | 'completed' | 'deleted';

export type ScheduledTaskRunStatus =
  | 'running'
  | 'completed'
  | 'failed'
  | 'cancelled'
  | 'waiting_input'
  | 'waiting_approval';

export type ScheduledTaskDeliveryTier = 'durable' | 'session_scoped';

export type ScheduledTaskScheduleMode =
  | 'every_minutes'
  | 'every_hours'
  | 'daily_at'
  | 'weekdays_at'
  | 'weekly_on'
  | 'custom';

export type ScheduledTaskProfile =
  | 'document_task'
  | 'workspace_task'
  | 'hybrid_task'
  | 'retrieval_task';

export type ScheduledTaskOutputMode = 'summary_only' | 'summary_and_artifact';

export type ScheduledTaskPublishBehavior =
  | 'none'
  | 'publish_workspace_artifact'
  | 'create_document_from_file';

export type ScheduledTaskSourceScope =
  | 'team_documents'
  | 'workspace_only'
  | 'mixed'
  | 'external_retrieval';

export type ScheduledTaskSourcePolicy =
  | 'official_first'
  | 'domestic_preferred'
  | 'global_preferred'
  | 'mixed';

export type ScheduledTaskRunOutcomeReason =
  | 'completed'
  | 'completed_with_warnings'
  | 'failed_no_final_answer'
  | 'failed_contract_violation'
  | 'blocked_capability_policy'
  | 'cancelled';

export type ScheduledTaskSelfEvaluationGrade =
  | 'excellent'
  | 'good'
  | 'acceptable'
  | 'weak'
  | 'failed';

export type ScheduledTaskScheduleSpecKind = 'one_shot' | 'every' | 'calendar';

export type ScheduledTaskPayloadKind =
  | 'system_summary'
  | 'artifact_task'
  | 'document_pipeline'
  | 'retrieval_pipeline';

export type ScheduledTaskSessionBinding = 'isolated_task' | 'bound_session';

export type ScheduledTaskDeliveryPlanKind =
  | 'channel_only'
  | 'channel_and_artifact'
  | 'channel_and_publish';

// ---------------------------------------------------------------------------
// Schedule config
// ---------------------------------------------------------------------------

export interface ScheduledTaskScheduleConfig {
  mode: ScheduledTaskScheduleMode;
  every_minutes?: number;
  every_hours?: number;
  daily_time?: string;
  weekly_days?: string[];
  cron_expression?: string;
}

export interface ScheduledTaskExecutionContract {
  output_mode: ScheduledTaskOutputMode;
  must_return_final_text: boolean;
  allow_partial_result: boolean;
  artifact_path?: string;
  publish_behavior: ScheduledTaskPublishBehavior;
  source_scope: ScheduledTaskSourceScope;
  source_policy?: ScheduledTaskSourcePolicy;
  minimum_source_attempts?: number;
  minimum_successful_sources?: number;
  prefer_structured_sources?: boolean;
  allow_query_retry?: boolean;
  fallback_to_secondary_sources?: boolean;
  required_sections?: string[];
}

export interface ScheduledTaskScheduleSpec {
  kind: ScheduledTaskScheduleSpecKind;
  one_shot_at?: string;
  schedule_config?: ScheduledTaskScheduleConfig;
  cron_expression?: string;
  timezone: string;
}

// ---------------------------------------------------------------------------
// Document types (JSON-file backed)
// ---------------------------------------------------------------------------

/**
 * Scheduled task summary for list views.
 * Mirrors Rust `ScheduledTaskSummaryResponse` (the serialized DTO), NOT the
 * internal `ScheduledTaskDoc`. Fields marked optional use
 * `skip_serializing_if = "Option::is_none"` on the backend, so they are absent
 * (not null) when unset.
 */
export interface ScheduledTaskSummary {
  task_id: string;
  agent_id: string;
  title: string;
  task_kind: ScheduledTaskKind;
  task_profile: ScheduledTaskProfile;
  payload_kind: ScheduledTaskPayloadKind;
  session_binding: ScheduledTaskSessionBinding;
  delivery_plan: ScheduledTaskDeliveryPlanKind;
  execution_contract: ScheduledTaskExecutionContract;
  delivery_tier: ScheduledTaskDeliveryTier;
  timezone: string;
  status: ScheduledTaskStatus;
  human_schedule: string;
  schedule_config?: ScheduledTaskScheduleConfig;
  one_shot_at?: string;
  cron_expression?: string;
  next_fire_at?: string;
  next_fire_preview?: string;
  last_fire_at?: string;
  last_run_id?: string;
  owner_session_id?: string;
  last_expected_fire_at?: string;
  last_missed_at?: string;
  missed_fire_count: number;
  can_resume: boolean;
  resume_hint?: string;
  created_at: string;
  updated_at: string;
}

/**
 * Scheduled task run record. Mirrors Rust `ScheduledTaskRunResponse`.
 * Note: the run DTO does NOT carry `created_at`/`updated_at` — only
 * `started_at`/`finished_at`.
 */
export interface ScheduledTaskRun {
  run_id: string;
  task_id: string;
  status: ScheduledTaskRunStatus;
  outcome_reason?: ScheduledTaskRunOutcomeReason;
  warning_count: number;
  runtime_session_id?: string;
  fire_message_id?: string;
  summary?: string;
  error?: string;
  self_evaluation?: ScheduledTaskSelfEvaluation;
  initial_self_evaluation?: ScheduledTaskSelfEvaluation;
  improvement_loop_applied: boolean;
  improvement_loop_count: number;
  trigger_source: string;
  started_at: string;
  finished_at?: string;
}

/** Mirrors Rust `ScheduledTaskSelfEvaluation`. */
export interface ScheduledTaskSelfEvaluation {
  score: number;
  grade: ScheduledTaskSelfEvaluationGrade;
  goal_completion: number;
  result_quality: number;
  evidence_quality: number;
  execution_stability: number;
  contract_compliance: number;
  summary: string;
  completed_steps?: string[];
  failed_steps?: string[];
  risks?: string[];
  confidence: number;
}

// ---------------------------------------------------------------------------
// Parse result
// ---------------------------------------------------------------------------

/** Result from natural language parsing. Mirrors Rust `ScheduledTaskParseResult`. */
export interface ScheduledTaskParseResult {
  title: string;
  prompt: string;
  task_kind: ScheduledTaskKind;
  task_profile: ScheduledTaskProfile;
  payload_kind: ScheduledTaskPayloadKind;
  session_binding: ScheduledTaskSessionBinding;
  delivery_plan: ScheduledTaskDeliveryPlanKind;
  schedule_spec: ScheduledTaskScheduleSpec;
  execution_contract: ScheduledTaskExecutionContract;
  human_schedule: string;
  warnings: string[];
  advanced_mode: boolean;
  confidence: number;
  ready_to_create: boolean;
  agent_id?: string;
}

// ---------------------------------------------------------------------------
// API request types
// ---------------------------------------------------------------------------

export interface ParseScheduledTaskRequest {
  text: string;
  timezone?: string;
  agent_id?: string;
}

export interface CreateTaskRequest {
  agent_id: string;
  title: string;
  prompt: string;
  task_kind: ScheduledTaskKind;
  one_shot_at?: string;
  cron_expression?: string;
  timezone?: string;
  delivery_tier?: ScheduledTaskDeliveryTier;
  owner_session_id?: string;
  schedule_config?: ScheduledTaskScheduleConfig;
  task_profile?: ScheduledTaskProfile;
  payload_kind?: ScheduledTaskPayloadKind;
  session_binding?: ScheduledTaskSessionBinding;
  delivery_plan?: ScheduledTaskDeliveryPlanKind;
  execution_contract?: ScheduledTaskExecutionContract;
}

export interface CreateFromParseOverrides {
  title?: string;
  prompt?: string;
  agent_id?: string;
  timezone?: string;
  one_shot_at?: string;
  schedule_config?: ScheduledTaskScheduleConfig;
  artifact_path?: string;
  publish_behavior?: ScheduledTaskPublishBehavior;
  delivery_tier?: ScheduledTaskDeliveryTier;
}

export interface CreateFromParseRequest {
  preview: ScheduledTaskParseResult;
  overrides: CreateFromParseOverrides;
}

export interface UpdateTaskRequest {
  title?: string;
  prompt?: string;
  agent_id?: string;
  task_kind?: ScheduledTaskKind;
  one_shot_at?: string;
  cron_expression?: string;
  timezone?: string;
  schedule_config?: ScheduledTaskScheduleConfig;
  delivery_tier?: ScheduledTaskDeliveryTier;
  owner_session_id?: string;
  task_profile?: ScheduledTaskProfile;
  execution_contract?: ScheduledTaskExecutionContract;
  payload_kind?: ScheduledTaskPayloadKind;
  session_binding?: ScheduledTaskSessionBinding;
  delivery_plan?: ScheduledTaskDeliveryPlanKind;
}

// ---------------------------------------------------------------------------
// API response types
// ---------------------------------------------------------------------------

/** List tasks response. */
export interface ListTasksResponse {
  tasks: ScheduledTaskSummary[];
}

/** Create/Update/Get task response. */
export interface TaskDetailResponse {
  task: ScheduledTaskDetail;
}

/** Parse task response. */
export interface ParseTaskResponse {
  preview: ScheduledTaskParseResult;
}

/** Run task now response. */
export interface RunTaskResponse {
  run: ScheduledTaskRun;
}

/** List runs response. */
export interface ListRunsResponse {
  runs: ScheduledTaskRun[];
}

/**
 * Full task detail with runs. Mirrors Rust `ScheduledTaskDetailResponse`,
 * which flattens the summary fields and adds `prompt` + `runs`.
 */
export interface ScheduledTaskDetail extends ScheduledTaskSummary {
  prompt: string;
  runs: ScheduledTaskRun[];
}
