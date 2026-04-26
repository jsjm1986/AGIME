import { fetchApi } from "./client";

const API_BASE = "/api/team/automation";

export type IntegrationSpecKind =
  | "openapi"
  | "postman"
  | "curl"
  | "markdown"
  | "json"
  | "text";

export type IntegrationAuthType =
  | "none"
  | "bearer"
  | "header"
  | "basic"
  | "api_key";

export type DraftStatus = "draft" | "probing" | "ready" | "failed" | "archived";
export type RunMode = "chat" | "run_once" | "schedule" | "monitor";
export type RunStatus =
  | "draft"
  | "running"
  | "waiting_input"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "paused"
  | "canceled";
export type ScheduleMode = "schedule" | "monitor";
export type ScheduleStatus = "active" | "paused" | "deleted";
export type ArtifactKind = "deliverable" | "supporting";

export interface AutomationProject {
  project_id: string;
  team_id: string;
  name: string;
  description?: string | null;
  created_by: string;
  created_at: string;
  updated_at: string;
  stats: {
    integrations: number;
    drafts: number;
    modules: number;
    runs: number;
    schedules: number;
  };
}

export interface AutomationIntegration {
  integration_id: string;
  team_id: string;
  project_id: string;
  name: string;
  spec_kind: IntegrationSpecKind;
  source_file_name?: string | null;
  base_url?: string | null;
  auth_type: IntegrationAuthType;
  auth_config: Record<string, unknown>;
  connection_status: "unknown" | "success" | "failed";
  last_tested_at?: string | null;
  last_test_message?: string | null;
  created_by: string;
  created_at: string;
  updated_at: string;
  spec_excerpt: string;
}

export interface AgentAppDraft {
  draft_id: string;
  app_draft_id?: string;
  team_id: string;
  project_id: string;
  name: string;
  driver_agent_id: string;
  integration_ids: string[];
  goal: string;
  constraints: string[];
  success_criteria: string[];
  risk_preference: string;
  status: DraftStatus;
  builder_session_id?: string | null;
  candidate_plan: Record<string, unknown>;
  probe_report: Record<string, unknown>;
  candidate_endpoints: unknown[];
  latest_probe_prompt?: string | null;
  linked_module_id?: string | null;
  publish_readiness?: {
    ready: boolean;
    state: DraftStatus;
    label: string;
    issues?: string[];
    warnings?: string[];
    verification?: {
      total_actions: number;
      valid_actions: number;
      structured_actions: number;
      shell_fallback_actions: number;
    };
  };
  summary?: {
    integration_count: number;
    has_builder_session: boolean;
    has_candidate_plan: boolean;
    has_probe_report: boolean;
    verified_http_actions?: number;
    structured_verified_http_actions?: number;
    shell_fallback_verified_http_actions?: number;
  };
  created_by: string;
  created_at: string;
  updated_at: string;
}

export interface AgentApp {
  module_id: string;
  app_id?: string;
  team_id: string;
  project_id: string;
  name: string;
  version: number;
  source_draft_id: string;
  driver_agent_id: string;
  integration_ids: string[];
  intent_snapshot: Record<string, unknown>;
  capability_pack_snapshot: Record<string, unknown>;
  execution_contract: Record<string, unknown>;
  latest_builder_session_id?: string | null;
  runtime_session_id?: string | null;
  summary?: {
    integration_count: number;
    has_execution_contract: boolean;
    native_execution_ready?: boolean;
    verified_http_actions?: number;
    structured_verified_http_actions?: number;
    shell_fallback_verified_http_actions?: number;
  };
  created_by: string;
  created_at: string;
  updated_at: string;
}

export type AutomationTaskDraft = AgentAppDraft;
export type AutomationModule = AgentApp;

export interface AutomationRun {
  run_id: string;
  team_id: string;
  project_id: string;
  module_id: string;
  module_version: number;
  mode: RunMode;
  status: RunStatus;
  schedule_id?: string | null;
  session_id?: string | null;
  summary?: string | null;
  error?: string | null;
  started_at?: string | null;
  finished_at?: string | null;
  created_by: string;
  created_at: string;
  updated_at: string;
}

export interface AutomationArtifact {
  artifact_id: string;
  team_id: string;
  project_id: string;
  run_id: string;
  kind: ArtifactKind;
  name: string;
  content_type?: string | null;
  text_content?: string | null;
  metadata: Record<string, unknown>;
  created_at: string;
}

export interface AutomationSchedule {
  schedule_id: string;
  team_id: string;
  project_id: string;
  module_id: string;
  module_version: number;
  mode: ScheduleMode;
  status: ScheduleStatus;
  cron_expression?: string | null;
  poll_interval_seconds?: number | null;
  monitor_instruction?: string | null;
  next_run_at?: string | null;
  last_run_at?: string | null;
  last_run_id?: string | null;
  created_by: string;
  created_at: string;
  updated_at: string;
}

export interface AgentAppRuntime {
  app: AgentApp;
  runtime_session_id: string;
  recent_runs: AutomationRun[];
  recent_artifacts: AutomationArtifact[];
  active_schedules: AutomationSchedule[];
}

export const automationApi = {
  listProjects(teamId: string) {
    return fetchApi<{ projects: AutomationProject[] }>(`${API_BASE}/projects?team_id=${teamId}`);
  },
  createProject(payload: { team_id: string; name: string; description?: string | null }) {
    return fetchApi<{ project: AutomationProject }>(`${API_BASE}/projects`, {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },
  deleteProject(teamId: string, projectId: string) {
    return fetchApi<{ deleted: boolean }>(`${API_BASE}/projects/${projectId}?team_id=${teamId}`, {
      method: "DELETE",
    });
  },
  listIntegrations(teamId: string, projectId: string) {
    return fetchApi<{ integrations: AutomationIntegration[] }>(
      `${API_BASE}/projects/${projectId}/integrations?team_id=${teamId}`,
    );
  },
  createIntegration(
    projectId: string,
    payload: {
      team_id: string;
      project_id: string;
      name: string;
      spec_kind: IntegrationSpecKind;
      spec_content: string;
      source_file_name?: string | null;
      base_url?: string | null;
      auth_type?: IntegrationAuthType;
      auth_config?: Record<string, unknown>;
    },
  ) {
    return fetchApi<{ integration: AutomationIntegration }>(
      `${API_BASE}/projects/${projectId}/integrations?team_id=${payload.team_id}`,
      {
        method: "POST",
        body: JSON.stringify(payload),
      },
    );
  },
  testIntegration(teamId: string, integrationId: string, probe_path?: string) {
    return fetchApi<{ integration?: AutomationIntegration | null }>(
      `${API_BASE}/integrations/${integrationId}/test?team_id=${teamId}`,
      {
        method: "POST",
        body: JSON.stringify({ probe_path }),
      },
    );
  },
  listTaskDrafts(teamId: string, projectId: string) {
    return fetchApi<{ tasks: AutomationTaskDraft[] }>(
      `${API_BASE}/projects/${projectId}/tasks?team_id=${teamId}`,
    );
  },
  listAppDrafts(teamId: string, projectId: string) {
    return fetchApi<{ app_drafts: AgentAppDraft[] }>(
      `${API_BASE}/projects/${projectId}/app-drafts?team_id=${teamId}`,
    );
  },
  createTaskDraft(
    projectId: string,
    payload: {
      team_id: string;
      project_id: string;
      name: string;
      driver_agent_id: string;
      integration_ids: string[];
      goal: string;
      constraints: string[];
      success_criteria: string[];
      risk_preference?: string;
      create_builder_session?: boolean;
    },
  ) {
    return fetchApi<{ task: AutomationTaskDraft; builder_session_id?: string | null }>(
      `${API_BASE}/projects/${projectId}/tasks?team_id=${payload.team_id}`,
      {
        method: "POST",
        body: JSON.stringify(payload),
      },
    );
  },
  createAppDraft(
    projectId: string,
    payload: {
      team_id: string;
      project_id: string;
      name: string;
      driver_agent_id: string;
      integration_ids: string[];
      goal: string;
      constraints: string[];
      success_criteria: string[];
      risk_preference?: string;
      create_builder_session?: boolean;
    },
  ) {
    return fetchApi<{ app_draft: AgentAppDraft; builder_session_id?: string | null }>(
      `${API_BASE}/projects/${projectId}/app-drafts?team_id=${payload.team_id}`,
      {
        method: "POST",
        body: JSON.stringify(payload),
      },
    );
  },
  getTaskDraft(teamId: string, draftId: string) {
    return fetchApi<{ task: AutomationTaskDraft }>(`${API_BASE}/tasks/${draftId}?team_id=${teamId}`);
  },
  getAppDraft(teamId: string, draftId: string) {
    return fetchApi<{ app_draft: AgentAppDraft }>(`${API_BASE}/app-drafts/${draftId}?team_id=${teamId}`);
  },
  updateTaskDraft(teamId: string, draftId: string, payload: Record<string, unknown>) {
    return fetchApi<{ task: AutomationTaskDraft }>(
      `${API_BASE}/tasks/${draftId}?team_id=${teamId}`,
      {
        method: "PATCH",
        body: JSON.stringify(payload),
      },
    );
  },
  updateAppDraft(teamId: string, draftId: string, payload: Record<string, unknown>) {
    return fetchApi<{ app_draft: AgentAppDraft }>(
      `${API_BASE}/app-drafts/${draftId}?team_id=${teamId}`,
      {
        method: "PATCH",
        body: JSON.stringify(payload),
      },
    );
  },
  probeTaskDraft(teamId: string, draftId: string) {
    return fetchApi<{ task: AutomationTaskDraft; builder_session_id: string }>(
      `${API_BASE}/tasks/${draftId}/probe?team_id=${teamId}`,
      { method: "POST" },
    );
  },
  probeAppDraft(teamId: string, draftId: string) {
    return fetchApi<{ app_draft: AgentAppDraft; builder_session_id: string }>(
      `${API_BASE}/app-drafts/${draftId}/probe?team_id=${teamId}`,
      { method: "POST" },
    );
  },
  syncTaskDraftFromBuilder(teamId: string, draftId: string) {
    return fetchApi<{ task: AutomationTaskDraft; sync_state: "processing" | "updated" }>(
      `${API_BASE}/tasks/${draftId}/sync-builder?team_id=${teamId}`,
      { method: "POST" },
    );
  },
  syncAppDraftFromBuilder(teamId: string, draftId: string) {
    return fetchApi<{ app_draft: AgentAppDraft; sync_state: "processing" | "updated" }>(
      `${API_BASE}/app-drafts/${draftId}/sync-builder?team_id=${teamId}`,
      { method: "POST" },
    );
  },
  ensureBuilderSession(teamId: string, draftId: string) {
    return fetchApi<{ task: AutomationTaskDraft; builder_session_id: string }>(
      `${API_BASE}/tasks/${draftId}/builder-session?team_id=${teamId}`,
      { method: "POST" },
    );
  },
  ensureAppDraftBuilderSession(teamId: string, draftId: string) {
    return fetchApi<{ app_draft: AgentAppDraft; builder_session_id: string }>(
      `${API_BASE}/app-drafts/${draftId}/builder-session?team_id=${teamId}`,
      { method: "POST" },
    );
  },
  saveModule(teamId: string, draftId: string, name?: string) {
    return fetchApi<{ module: AutomationModule }>(
      `${API_BASE}/tasks/${draftId}/module?team_id=${teamId}`,
      {
        method: "POST",
        body: JSON.stringify({ name }),
      },
    );
  },
  publishApp(teamId: string, draftId: string, name?: string) {
    return fetchApi<{ app: AgentApp }>(
      `${API_BASE}/app-drafts/${draftId}/publish?team_id=${teamId}`,
      {
        method: "POST",
        body: JSON.stringify({ name }),
      },
    );
  },
  listModules(teamId: string, projectId: string) {
    return fetchApi<{ modules: AutomationModule[] }>(
      `${API_BASE}/projects/${projectId}/modules?team_id=${teamId}`,
    );
  },
  listApps(teamId: string, projectId: string) {
    return fetchApi<{ apps: AgentApp[] }>(
      `${API_BASE}/projects/${projectId}/apps?team_id=${teamId}`,
    );
  },
  deleteApp(teamId: string, appId: string) {
    return fetchApi<{ deleted: boolean }>(
      `${API_BASE}/apps/${appId}?team_id=${teamId}`,
      { method: "DELETE" },
    );
  },
  startModuleRun(teamId: string, moduleId: string, mode: RunMode) {
    return fetchApi<{ run: AutomationRun }>(
      `${API_BASE}/modules/${moduleId}/runs?team_id=${teamId}`,
      {
        method: "POST",
        body: JSON.stringify({ mode }),
      },
    );
  },
  startAppRun(teamId: string, appId: string, mode: RunMode) {
    return fetchApi<{ run: AutomationRun; app_id: string }>(
      `${API_BASE}/apps/${appId}/runs?team_id=${teamId}`,
      {
        method: "POST",
        body: JSON.stringify({ mode }),
      },
    );
  },
  getAppRuntime(teamId: string, appId: string) {
    return fetchApi<AgentAppRuntime>(
      `${API_BASE}/apps/${appId}/runtime?team_id=${teamId}`,
    );
  },
  listRuns(teamId: string, projectId: string) {
    return fetchApi<{ runs: AutomationRun[] }>(
      `${API_BASE}/projects/${projectId}/runs?team_id=${teamId}`,
    );
  },
  listArtifacts(teamId: string, projectId: string) {
    return fetchApi<{ artifacts: AutomationArtifact[] }>(
      `${API_BASE}/projects/${projectId}/artifacts?team_id=${teamId}`,
    );
  },
  listSchedules(teamId: string, projectId: string) {
    return fetchApi<{ schedules: AutomationSchedule[] }>(
      `${API_BASE}/projects/${projectId}/schedules?team_id=${teamId}`,
    );
  },
  createSchedule(
    teamId: string,
    moduleId: string,
    payload: {
      mode: ScheduleMode;
      cron_expression?: string | null;
      poll_interval_seconds?: number | null;
      monitor_instruction?: string | null;
    },
  ) {
    return fetchApi<{ schedule: AutomationSchedule }>(
      `${API_BASE}/modules/${moduleId}/schedules?team_id=${teamId}`,
      {
        method: "POST",
        body: JSON.stringify(payload),
      },
    );
  },
  createAppSchedule(
    teamId: string,
    appId: string,
    payload: {
      mode: ScheduleMode;
      cron_expression?: string | null;
      poll_interval_seconds?: number | null;
      monitor_instruction?: string | null;
    },
  ) {
    return fetchApi<{ schedule: AutomationSchedule; app_id: string }>(
      `${API_BASE}/apps/${appId}/schedules?team_id=${teamId}`,
      {
        method: "POST",
        body: JSON.stringify(payload),
      },
    );
  },
  updateSchedule(teamId: string, scheduleId: string, payload: Record<string, unknown>) {
    return fetchApi<{ schedule: AutomationSchedule }>(
      `${API_BASE}/schedules/${scheduleId}?team_id=${teamId}`,
      {
        method: "PATCH",
        body: JSON.stringify(payload),
      },
    );
  },
  deleteSchedule(teamId: string, scheduleId: string) {
    return fetchApi<{ deleted: boolean }>(
      `${API_BASE}/schedules/${scheduleId}?team_id=${teamId}`,
      { method: "DELETE" },
    );
  },
};
