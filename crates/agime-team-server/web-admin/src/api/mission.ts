/**
 * Mission API client (Phase 2 - Mission Track)
 * Goal-driven multi-step autonomous task system.
 */

import { ApiError, fetchApi } from './client';

const API_BASE = '/api/team/agent/mission';

function parseMissionConflictStatus(e: unknown): string | null {
  if (!(e instanceof ApiError) || e.status !== 409 || !e.body) return null;
  try {
    const parsed = JSON.parse(e.body);
    return typeof parsed?.status === 'string' ? parsed.status : null;
  } catch {
    return null;
  }
}

// ─── Types ───────────────────────────────────────────

export type MissionStatus =
  | 'draft' | 'planning' | 'planned' | 'running'
  | 'paused' | 'completed' | 'failed' | 'cancelled';

export type ApprovalPolicy = 'auto' | 'checkpoint' | 'manual';

export type StepStatus =
  | 'pending' | 'awaiting_approval' | 'running'
  | 'completed' | 'failed' | 'skipped';

// ─── AGE Types ──────────────────────────────────────

export type ExecutionMode = 'sequential' | 'adaptive';
export type ExecutionProfile = 'auto' | 'fast' | 'full';
export type ProgressSignal = 'advancing' | 'stalled' | 'blocked';
export type GoalStatus =
  | 'pending' | 'running' | 'awaiting_approval'
  | 'completed' | 'pivoting' | 'abandoned' | 'failed';

export interface RuntimeContract {
  required_artifacts?: string[];
  completion_checks?: string[];
  no_artifact_reason?: string;
  source?: string;
  captured_at?: string;
}

export interface RuntimeContractVerification {
  tool_called: boolean;
  status?: string;
  gate_mode?: string;
  accepted?: boolean;
  reason?: string;
  checked_at?: string;
}

export interface AttemptRecord {
  attempt_number: number;
  approach: string;
  signal: ProgressSignal;
  learnings: string;
  tokens_used: number;
  started_at?: string;
  completed_at?: string;
}

export interface GoalNode {
  goal_id: string;
  parent_id?: string;
  title: string;
  description: string;
  success_criteria: string;
  status: GoalStatus;
  depth: number;
  order: number;
  exploration_budget: number;
  attempts: AttemptRecord[];
  output_summary?: string;
  runtime_contract?: RuntimeContract;
  contract_verification?: RuntimeContractVerification;
  pivot_reason?: string;
  is_checkpoint: boolean;
  created_at?: string;
  completed_at?: string;
}

export interface ToolCallRecord {
  name: string;
  success: boolean;
}

export type StepSupervisorState = 'healthy' | 'busy' | 'drifting' | 'stalled';

export type StepProgressEventKind =
  | 'step_started'
  | 'activity_observed'
  | 'work_progress_observed'
  | 'summary_observed'
  | 'artifact_observed'
  | 'required_artifact_satisfied'
  | 'planning_evidence_observed'
  | 'quality_evidence_observed'
  | 'runtime_evidence_observed'
  | 'deployment_evidence_observed'
  | 'review_evidence_observed'
  | 'risk_evidence_observed'
  | 'runtime_contract_captured'
  | 'contract_verified'
  | 'retry_scheduled'
  | 'supervisor_intervention'
  | 'step_completed'
  | 'step_failed';

export type StepProgressEventSource =
  | 'executor'
  | 'workspace'
  | 'verifier'
  | 'supervisor'
  | 'ai_supervisor';

export type StepProgressLayer =
  | 'activity'
  | 'work_progress'
  | 'delivery_progress'
  | 'recovery';

export interface StepProgressEvent {
  kind: StepProgressEventKind;
  message: string;
  source?: StepProgressEventSource;
  layer?: StepProgressLayer;
  semantic_tags?: string[];
  ai_annotation?: string;
  paths?: string[];
  checks?: string[];
  score_delta?: number;
  recorded_at?: string;
}

export interface StepEvidenceBundle {
  artifact_paths?: string[];
  required_artifact_paths?: string[];
  planning_evidence_paths?: string[];
  planning_signals?: string[];
  quality_evidence_paths?: string[];
  quality_signals?: string[];
  runtime_evidence_paths?: string[];
  runtime_signals?: string[];
  deployment_evidence_paths?: string[];
  deployment_signals?: string[];
  review_evidence_paths?: string[];
  review_signals?: string[];
  risk_evidence_paths?: string[];
  risk_signals?: string[];
  latest_summary?: string;
  updated_at?: string;
}

export interface MissionStep {
  index: number;
  title: string;
  description: string;
  status: StepStatus;
  is_checkpoint: boolean;
  approved_by?: string;
  started_at?: string;
  completed_at?: string;
  error_message?: string;
  supervisor_state?: StepSupervisorState;
  last_activity_at?: string;
  last_progress_at?: string;
  progress_score?: number;
  current_blocker?: string;
  last_supervisor_hint?: string;
  stall_count?: number;
  recent_progress_events?: StepProgressEvent[];
  evidence_bundle?: StepEvidenceBundle;
  tokens_used: number;
  output_summary?: string;
  retry_count: number;
  max_retries: number;
  required_artifacts?: string[];
  completion_checks?: string[];
  runtime_contract?: RuntimeContract;
  contract_verification?: RuntimeContractVerification;
  use_subagent?: boolean;
  tool_calls?: ToolCallRecord[];
}

export interface MissionListItem {
  mission_id: string;
  agent_id: string;
  agent_name: string;
  goal: string;
  status: MissionStatus;
  approval_policy: ApprovalPolicy;
  step_count: number;
  completed_steps: number;
  current_step?: number;
  total_tokens_used: number;
  created_at: string;
  updated_at: string;
  // AGE fields
  execution_mode: ExecutionMode;
  execution_profile: ExecutionProfile;
  resolved_execution_profile?: ExecutionProfile;
  goal_count: number;
  completed_goals: number;
  pivots: number;
  attached_doc_count: number;
}

export interface MissionDetail {
  mission_id: string;
  team_id: string;
  agent_id: string;
  creator_id: string;
  goal: string;
  context?: string;
  status: MissionStatus;
  approval_policy: ApprovalPolicy;
  steps: MissionStep[];
  current_step?: number;
  session_id?: string;
  token_budget: number;
  total_tokens_used: number;
  priority: number;
  error_message?: string;
  final_summary?: string;
  // AGE fields
  execution_mode: ExecutionMode;
  execution_profile: ExecutionProfile;
  resolved_execution_profile?: ExecutionProfile;
  goal_tree?: GoalNode[];
  current_goal_id?: string;
  total_pivots: number;
  total_abandoned: number;
  created_at: string;
  updated_at: string;
  started_at?: string;
  completed_at?: string;
  attached_document_ids: string[];
  current_run_id?: string;
}

export interface MonitorAssessmentSnapshot {
  status_assessment?: string;
  evidence_sufficient: boolean;
  observed_evidence?: string[];
  missing_evidence?: string[];
  quality_summary?: string;
  risk_summary?: string;
}

export interface MonitorInterventionSnapshot {
  action: string;
  feedback?: string;
  semantic_tags?: string[];
  observed_evidence?: string[];
  requested_at?: string;
  applied_at?: string;
}

export interface MonitorStepSnapshot {
  index: number;
  title: string;
  description: string;
  status: StepStatus;
  supervisor_state?: StepSupervisorState;
  last_activity_at?: string;
  last_progress_at?: string;
  progress_score?: number;
  current_blocker?: string;
  last_supervisor_hint?: string;
  stall_count: number;
  retry_count: number;
  output_summary_present: boolean;
  required_artifacts?: string[];
  completion_checks?: string[];
  recent_progress_events?: StepProgressEvent[];
  evidence_bundle?: StepEvidenceBundle;
  assessment?: MonitorAssessmentSnapshot;
}

export interface MonitorGoalSnapshot {
  goal_id: string;
  parent_id?: string;
  title: string;
  description: string;
  success_criteria: string;
  status: GoalStatus;
  attempt_count: number;
  output_summary_present: boolean;
  has_runtime_contract: boolean;
  contract_verified?: boolean;
  pivot_reason?: string;
  last_activity_at?: string;
  last_progress_at?: string;
  assessment?: MonitorAssessmentSnapshot;
}

export interface MissionMonitorSnapshot {
  mission_id: string;
  status: MissionStatus;
  execution_mode: ExecutionMode;
  execution_profile: ExecutionProfile;
  is_active: boolean;
  current_run_id?: string;
  current_step?: number;
  current_goal_id?: string;
  error_message?: string;
  context?: string;
  pending_intervention?: MonitorInterventionSnapshot;
  last_applied_intervention?: MonitorInterventionSnapshot;
  step?: MonitorStepSnapshot;
  goal?: MonitorGoalSnapshot;
}

export interface MissionArtifact {
  artifact_id: string;
  mission_id: string;
  step_index: number;
  name: string;
  artifact_type: string;
  content?: string;
  file_path?: string;
  mime_type?: string;
  size: number;
  archived_document_id?: string;
  archived_document_status?: string;
  archived_at?: string;
  created_at: string;
}

export interface ArchiveArtifactRequest {
  name?: string;
  folder_path?: string;
  category?: 'general' | 'report' | 'translation' | 'summary' | 'review' | 'code' | 'other';
}

export interface ArchiveArtifactResponse {
  artifact: MissionArtifact;
  document: {
    id: string;
    name: string;
    status: string;
    source_mission_id?: string | null;
  };
  created: boolean;
}

export interface MissionEvent {
  mission_id: string;
  run_id?: string;
  event_id: number;
  event_type: string;
  payload: Record<string, unknown>;
  created_at: string;
}

export interface CreateMissionRequest {
  agent_id: string;
  goal: string;
  context?: string;
  route_mode?: 'auto' | 'mission' | 'direct';
  approval_policy?: ApprovalPolicy;
  token_budget?: number;
  priority?: number;
  source_chat_session_id?: string;
  execution_mode?: ExecutionMode;
  execution_profile?: ExecutionProfile;
  attached_document_ids?: string[];
}

// ─── API ─────────────────────────────────────────────

export const missionApi = {
  /** List missions for a team */
  async listMissions(
    teamId: string,
    agentId?: string,
    status?: string,
    page = 1,
    limit = 20,
  ): Promise<MissionListItem[]> {
    const params = new URLSearchParams({
      team_id: teamId,
      page: String(page),
      limit: String(limit),
    });
    if (agentId) params.set('agent_id', agentId);
    if (status) params.set('status', status);
    return fetchApi(`${API_BASE}/missions?${params}`);
  },

  /** Create a new mission */
  async createMission(req: CreateMissionRequest): Promise<{
    route?: 'mission' | 'direct';
    mission_id?: string;
    session_id?: string;
    agent_id?: string;
    status: string;
    message?: string;
  }> {
    return fetchApi(`${API_BASE}/missions`, {
      method: 'POST',
      body: JSON.stringify(req),
    });
  },

  /** Get mission detail */
  async getMission(missionId: string): Promise<MissionDetail> {
    return fetchApi(`${API_BASE}/missions/${missionId}`);
  },

  /** Get monitor snapshot for live mission state */
  async getMonitorSnapshot(missionId: string): Promise<MissionMonitorSnapshot> {
    return fetchApi(`${API_BASE}/missions/${missionId}/monitor-snapshot`);
  },

  /** Delete a mission (only draft/cancelled) */
  async deleteMission(missionId: string): Promise<void> {
    await fetchApi(`${API_BASE}/missions/${missionId}`, { method: 'DELETE' });
  },

  /** Start mission execution */
  async startMission(missionId: string): Promise<{ mission_id: string; status: string }> {
    try {
      return await fetchApi(`${API_BASE}/missions/${missionId}/start`, { method: 'POST' });
    } catch (e) {
      const status = parseMissionConflictStatus(e);
      if (status === 'already_running') {
        return { mission_id: missionId, status: 'already_running' };
      }
      throw e;
    }
  },

  /** Resume a paused/failed mission, optional feedback guides the retry run */
  async resumeMission(
    missionId: string,
    feedback?: string,
  ): Promise<{ mission_id: string; status: string }> {
    try {
      return await fetchApi(`${API_BASE}/missions/${missionId}/resume`, {
        method: 'POST',
        body: JSON.stringify({ feedback }),
      });
    } catch (e) {
      const status = parseMissionConflictStatus(e);
      if (status === 'already_running' || status === 'pause_in_progress') {
        return { mission_id: missionId, status };
      }
      throw e;
    }
  },

  /** Pause a running mission */
  async pauseMission(missionId: string): Promise<void> {
    await fetchApi(`${API_BASE}/missions/${missionId}/pause`, { method: 'POST' });
  },

  /** Cancel a mission */
  async cancelMission(missionId: string): Promise<void> {
    await fetchApi(`${API_BASE}/missions/${missionId}/cancel`, { method: 'POST' });
  },

  /** Approve a step */
  async approveStep(missionId: string, stepIndex: number, feedback?: string): Promise<void> {
    await fetchApi(`${API_BASE}/missions/${missionId}/steps/${stepIndex}/approve`, {
      method: 'POST',
      body: JSON.stringify({ feedback }),
    });
  },

  /** Reject a step */
  async rejectStep(missionId: string, stepIndex: number, feedback?: string): Promise<void> {
    await fetchApi(`${API_BASE}/missions/${missionId}/steps/${stepIndex}/reject`, {
      method: 'POST',
      body: JSON.stringify({ feedback }),
    });
  },

  /** Skip a step */
  async skipStep(missionId: string, stepIndex: number): Promise<void> {
    await fetchApi(`${API_BASE}/missions/${missionId}/steps/${stepIndex}/skip`, {
      method: 'POST',
    });
  },

  // ─── AGE Goal Operations ──────────────────────────

  /** Approve a goal (resume execution) */
  async approveGoal(missionId: string, goalId: string, feedback?: string): Promise<void> {
    await fetchApi(`${API_BASE}/missions/${missionId}/goals/${goalId}/approve`, {
      method: 'POST',
      body: JSON.stringify({ feedback }),
    });
  },

  /** Reject a goal (fail mission) */
  async rejectGoal(missionId: string, goalId: string, feedback?: string): Promise<void> {
    await fetchApi(`${API_BASE}/missions/${missionId}/goals/${goalId}/reject`, {
      method: 'POST',
      body: JSON.stringify({ feedback }),
    });
  },

  /** Pivot a goal to a new approach */
  async pivotGoal(missionId: string, goalId: string, approach: string): Promise<void> {
    await fetchApi(`${API_BASE}/missions/${missionId}/goals/${goalId}/pivot`, {
      method: 'POST',
      body: JSON.stringify({ alternative_approach: approach }),
    });
  },

  /** Abandon a goal */
  async abandonGoal(missionId: string, goalId: string, reason?: string): Promise<void> {
    await fetchApi(`${API_BASE}/missions/${missionId}/goals/${goalId}/abandon`, {
      method: 'POST',
      body: JSON.stringify({ feedback: reason }),
    });
  },

  /** Subscribe to SSE stream for a mission */
  streamMission(missionId: string, lastEventId?: number | null): EventSource {
    const q = lastEventId && lastEventId > 0 ? `?last_event_id=${lastEventId}` : '';
    return new EventSource(`${API_BASE}/missions/${missionId}/stream${q}`, {
      withCredentials: true,
    } as EventSourceInit);
  },

  /** List mission artifacts */
  async listArtifacts(missionId: string): Promise<MissionArtifact[]> {
    return fetchApi(`${API_BASE}/missions/${missionId}/artifacts`);
  },

  /** List mission runtime events (persisted stream logs) */
  async listEvents(
    missionId: string,
    afterEventId?: number,
    runId?: string,
    limit = 500,
  ): Promise<MissionEvent[]> {
    const params = new URLSearchParams({ limit: String(limit) });
    if (afterEventId !== undefined && afterEventId !== null) {
      params.set('after_event_id', String(afterEventId));
    }
    // `run_id=__all__` tells backend to aggregate all runs.
    if (runId && runId.trim().length > 0) {
      params.set('run_id', runId.trim());
    }
    return fetchApi(`${API_BASE}/missions/${missionId}/events?${params.toString()}`);
  },

  /** Get a single artifact */
  async getArtifact(artifactId: string): Promise<MissionArtifact> {
    return fetchApi(`${API_BASE}/artifacts/${artifactId}`);
  },

  /** Download artifact file */
  getArtifactDownloadUrl(artifactId: string): string {
    return `${API_BASE}/artifacts/${artifactId}/download`;
  },

  /** Archive artifact into document store (creates draft document) */
  async archiveArtifactToDocument(
    artifactId: string,
    body: ArchiveArtifactRequest = {},
  ): Promise<ArchiveArtifactResponse> {
    return fetchApi(`${API_BASE}/artifacts/${artifactId}/archive`, {
      method: 'POST',
      body: JSON.stringify(body),
    });
  },

  /** Create mission from a chat session */
  async createFromChat(
    agentId: string,
    goal: string,
    chatSessionId: string,
    approvalPolicy?: ApprovalPolicy,
  ): Promise<{ mission_id: string; status: string }> {
    return fetchApi(`${API_BASE}/from-chat`, {
      method: 'POST',
      body: JSON.stringify({
        agent_id: agentId,
        goal,
        chat_session_id: chatSessionId,
        approval_policy: approvalPolicy,
      }),
    });
  },

  /** Attach documents to a mission */
  async attachDocuments(missionId: string, documentIds: string[]): Promise<void> {
    await fetchApi(`${API_BASE}/missions/${missionId}/documents`, {
      method: 'POST',
      body: JSON.stringify({ document_ids: documentIds }),
    });
  },

  /** Detach documents from a mission */
  async detachDocuments(missionId: string, documentIds: string[]): Promise<void> {
    await fetchApi(`${API_BASE}/missions/${missionId}/documents`, {
      method: 'DELETE',
      body: JSON.stringify({ document_ids: documentIds }),
    });
  },

  /** List attached documents */
  async listAttachedDocuments(missionId: string): Promise<string[]> {
    return fetchApi(`${API_BASE}/missions/${missionId}/documents`);
  },
};
