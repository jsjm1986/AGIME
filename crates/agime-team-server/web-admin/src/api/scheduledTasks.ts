import { fetchApi } from "./client";

const API_BASE = "/api/team/scheduled-tasks";

export type ScheduledTaskKind = "one_shot" | "cron";
export type ScheduledTaskStatus = "draft" | "active" | "paused" | "completed" | "deleted";
export type ScheduledTaskDeliveryTier = "durable" | "session_scoped";
export type ScheduledTaskListView = "mine" | "all_visible";
export type ScheduledTaskScheduleMode =
  | "every_minutes"
  | "every_hours"
  | "daily_at"
  | "weekdays_at"
  | "weekly_on"
  | "custom";
export type ScheduledTaskProfile =
  | "document_task"
  | "workspace_task"
  | "hybrid_task"
  | "retrieval_task";
export type ScheduledTaskOutputMode = "summary_only" | "summary_and_artifact";
export type ScheduledTaskPublishBehavior =
  | "none"
  | "publish_workspace_artifact"
  | "create_document_from_file";
export type ScheduledTaskSourceScope =
  | "team_documents"
  | "workspace_only"
  | "mixed"
  | "external_retrieval";
export type ScheduledTaskSourcePolicy =
  | "official_first"
  | "domestic_preferred"
  | "global_preferred"
  | "mixed";
export type ScheduledTaskPayloadKind =
  | "system_summary"
  | "artifact_task"
  | "document_pipeline"
  | "retrieval_pipeline";
export type ScheduledTaskSessionBinding = "isolated_task" | "bound_session";
export type ScheduledTaskDeliveryPlanKind =
  | "channel_only"
  | "channel_and_artifact"
  | "channel_and_publish";
export type ScheduledTaskScheduleSpecKind = "one_shot" | "every" | "calendar";
export type ScheduledTaskRunStatus =
  | "running"
  | "completed"
  | "failed"
  | "cancelled"
  | "waiting_input"
  | "waiting_approval";
export type ScheduledTaskRunOutcomeReason =
  | "completed"
  | "completed_with_warnings"
  | "failed_no_final_answer"
  | "failed_contract_violation"
  | "blocked_capability_policy"
  | "cancelled";
export type ScheduledTaskSelfEvaluationGrade =
  | "excellent"
  | "good"
  | "acceptable"
  | "weak"
  | "failed";

export interface ScheduledTaskSelfEvaluation {
  score: number;
  grade: ScheduledTaskSelfEvaluationGrade;
  goal_completion: number;
  result_quality: number;
  evidence_quality: number;
  execution_stability: number;
  contract_compliance: number;
  summary: string;
  completed_steps?: string[] | null;
  failed_steps?: string[] | null;
  risks?: string[] | null;
  confidence: number;
}

export interface ScheduledTaskRun {
  run_id: string;
  task_id: string;
  channel_id: string;
  status: ScheduledTaskRunStatus;
  outcome_reason?: ScheduledTaskRunOutcomeReason | null;
  warning_count: number;
  runtime_session_id?: string | null;
  fire_message_id?: string | null;
  summary?: string | null;
  error?: string | null;
  self_evaluation?: ScheduledTaskSelfEvaluation | null;
  initial_self_evaluation?: ScheduledTaskSelfEvaluation | null;
  improvement_loop_applied: boolean;
  improvement_loop_count: number;
  trigger_source: string;
  started_at: string;
  finished_at?: string | null;
}

export interface ScheduledTaskScheduleConfig {
  mode: ScheduledTaskScheduleMode;
  every_minutes?: number | null;
  every_hours?: number | null;
  daily_time?: string | null;
  weekly_days?: string[] | null;
  cron_expression?: string | null;
}

export interface ScheduledTaskExecutionContract {
  output_mode: ScheduledTaskOutputMode;
  must_return_final_text: boolean;
  allow_partial_result: boolean;
  artifact_path?: string | null;
  publish_behavior: ScheduledTaskPublishBehavior;
  source_scope: ScheduledTaskSourceScope;
  source_policy?: ScheduledTaskSourcePolicy | null;
  minimum_source_attempts?: number | null;
  minimum_successful_sources?: number | null;
  prefer_structured_sources?: boolean | null;
  allow_query_retry?: boolean | null;
  fallback_to_secondary_sources?: boolean | null;
  required_sections?: string[] | null;
}

export interface ScheduledTaskScheduleSpec {
  kind: ScheduledTaskScheduleSpecKind;
  one_shot_at?: string | null;
  schedule_config?: ScheduledTaskScheduleConfig | null;
  cron_expression?: string | null;
  timezone: string;
}

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
  agent_id?: string | null;
}

export interface ScheduledTaskParseOverrides {
  title?: string;
  prompt?: string;
  agent_id?: string;
  timezone?: string;
  delivery_tier?: ScheduledTaskDeliveryTier;
  one_shot_at?: string | null;
  schedule_config?: ScheduledTaskScheduleConfig | null;
  artifact_path?: string;
  publish_behavior?: ScheduledTaskPublishBehavior;
}

export interface ScheduledTaskSummary {
  task_id: string;
  team_id: string;
  channel_id: string;
  owner_user_id: string;
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
  schedule_config?: ScheduledTaskScheduleConfig | null;
  one_shot_at?: string | null;
  cron_expression?: string | null;
  next_fire_at?: string | null;
  next_fire_preview?: string | null;
  last_fire_at?: string | null;
  last_run_id?: string | null;
  owner_session_id?: string | null;
  last_expected_fire_at?: string | null;
  last_missed_at?: string | null;
  missed_fire_count: number;
  can_resume: boolean;
  resume_hint?: string | null;
  created_at: string;
  updated_at: string;
}

export type ScheduledTaskRepairReason = "missing_channel";

export interface ScheduledTaskRepairEvent {
  task_id: string;
  channel_id: string;
  title: string;
  owner_user_id: string;
  delivery_tier: ScheduledTaskDeliveryTier;
  reason: ScheduledTaskRepairReason;
  user_message: string;
}

export interface ScheduledTaskListResponse {
  tasks: ScheduledTaskSummary[];
  repair_events?: ScheduledTaskRepairEvent[];
}

export interface ScheduledTaskDetail extends ScheduledTaskSummary {
  prompt: string;
  runs: ScheduledTaskRun[];
}

export const scheduledTasksApi = {
  async list(teamId: string, view: ScheduledTaskListView = "mine"): Promise<ScheduledTaskListResponse> {
    const res = await fetchApi<ScheduledTaskListResponse>(
      `${API_BASE}?team_id=${encodeURIComponent(teamId)}&view=${encodeURIComponent(view)}`,
    );
    return {
      tasks: res.tasks || [],
      repair_events: res.repair_events || [],
    };
  },

  async create(
    teamId: string,
    payload: {
      agent_id: string;
      title: string;
      prompt: string;
      task_kind: ScheduledTaskKind;
      delivery_tier?: ScheduledTaskDeliveryTier;
      owner_session_id?: string | null;
      one_shot_at?: string | null;
      cron_expression?: string | null;
      timezone?: string | null;
      schedule_config?: ScheduledTaskScheduleConfig | null;
    },
  ): Promise<ScheduledTaskDetail> {
    const res = await fetchApi<{ task: ScheduledTaskDetail }>(
      `${API_BASE}?team_id=${encodeURIComponent(teamId)}`,
      {
        method: "POST",
        body: JSON.stringify(payload),
      },
    );
    return res.task;
  },

  async parse(
    teamId: string,
    payload: { text: string; timezone?: string | null; agent_id?: string | null },
  ): Promise<ScheduledTaskParseResult> {
    const res = await fetchApi<{ preview: ScheduledTaskParseResult }>(
      `${API_BASE}/parse?team_id=${encodeURIComponent(teamId)}`,
      {
        method: "POST",
        body: JSON.stringify(payload),
      },
    );
    return res.preview;
  },

  async createFromParse(
    teamId: string,
    payload: { preview: ScheduledTaskParseResult; overrides?: ScheduledTaskParseOverrides },
  ): Promise<ScheduledTaskDetail> {
    const res = await fetchApi<{ task: ScheduledTaskDetail }>(
      `${API_BASE}/create-from-parse?team_id=${encodeURIComponent(teamId)}`,
      {
        method: "POST",
        body: JSON.stringify(payload),
      },
    );
    return res.task;
  },

  async get(teamId: string, taskId: string): Promise<ScheduledTaskDetail> {
    const res = await fetchApi<{ task: ScheduledTaskDetail }>(
      `${API_BASE}/${encodeURIComponent(taskId)}?team_id=${encodeURIComponent(teamId)}`,
    );
    return res.task;
  },

  async update(
    teamId: string,
    taskId: string,
    payload: {
      agent_id?: string;
      title?: string;
      prompt?: string;
      task_kind?: ScheduledTaskKind;
      delivery_tier?: ScheduledTaskDeliveryTier;
      owner_session_id?: string | null;
      one_shot_at?: string | null;
      cron_expression?: string | null;
      timezone?: string | null;
      schedule_config?: ScheduledTaskScheduleConfig | null;
    },
  ): Promise<ScheduledTaskDetail> {
    const res = await fetchApi<{ task: ScheduledTaskDetail }>(
      `${API_BASE}/${encodeURIComponent(taskId)}?team_id=${encodeURIComponent(teamId)}`,
      {
        method: "PATCH",
        body: JSON.stringify(payload),
      },
    );
    return res.task;
  },

  async publish(teamId: string, taskId: string): Promise<ScheduledTaskDetail> {
    const res = await fetchApi<{ task: ScheduledTaskDetail }>(
      `${API_BASE}/${encodeURIComponent(taskId)}/publish?team_id=${encodeURIComponent(teamId)}`,
      { method: "POST" },
    );
    return res.task;
  },

  async pause(teamId: string, taskId: string): Promise<ScheduledTaskDetail> {
    const res = await fetchApi<{ task: ScheduledTaskDetail }>(
      `${API_BASE}/${encodeURIComponent(taskId)}/pause?team_id=${encodeURIComponent(teamId)}`,
      { method: "POST" },
    );
    return res.task;
  },

  async resume(teamId: string, taskId: string): Promise<ScheduledTaskDetail> {
    const res = await fetchApi<{ task: ScheduledTaskDetail }>(
      `${API_BASE}/${encodeURIComponent(taskId)}/resume?team_id=${encodeURIComponent(teamId)}`,
      { method: "POST" },
    );
    return res.task;
  },

  async runNow(teamId: string, taskId: string): Promise<ScheduledTaskRun> {
    const res = await fetchApi<{ run: ScheduledTaskRun }>(
      `${API_BASE}/${encodeURIComponent(taskId)}/run-now?team_id=${encodeURIComponent(teamId)}`,
      { method: "POST" },
    );
    return res.run;
  },

  async cancel(teamId: string, taskId: string): Promise<void> {
    await fetchApi(
      `${API_BASE}/${encodeURIComponent(taskId)}/cancel?team_id=${encodeURIComponent(teamId)}`,
      { method: "POST" },
    );
  },

  async remove(teamId: string, taskId: string): Promise<void> {
    await fetchApi(
      `${API_BASE}/${encodeURIComponent(taskId)}?team_id=${encodeURIComponent(teamId)}`,
      { method: "DELETE" },
    );
  },
};
