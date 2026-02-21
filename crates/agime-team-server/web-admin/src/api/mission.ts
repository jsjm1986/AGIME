/**
 * Mission API client (Phase 2 - Mission Track)
 * Goal-driven multi-step autonomous task system.
 */

import { fetchApi } from './client';

const API_BASE = '/api/team/agent/mission';

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
export type ProgressSignal = 'advancing' | 'stalled' | 'blocked';
export type GoalStatus =
  | 'pending' | 'running' | 'awaiting_approval'
  | 'completed' | 'pivoting' | 'abandoned' | 'failed';

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
  pivot_reason?: string;
  is_checkpoint: boolean;
  created_at?: string;
  completed_at?: string;
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
  tokens_used: number;
  output_summary?: string;
  retry_count: number;
  max_retries: number;
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
  goal_count: number;
  completed_goals: number;
  pivots: number;
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
  goal_tree?: GoalNode[];
  current_goal_id?: string;
  total_pivots: number;
  total_abandoned: number;
  created_at: string;
  updated_at: string;
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
  created_at: string;
}

export interface CreateMissionRequest {
  agent_id: string;
  goal: string;
  context?: string;
  approval_policy?: ApprovalPolicy;
  token_budget?: number;
  priority?: number;
  source_chat_session_id?: string;
  execution_mode?: ExecutionMode;
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
  async createMission(req: CreateMissionRequest): Promise<{ mission_id: string; status: string }> {
    return fetchApi(`${API_BASE}/missions`, {
      method: 'POST',
      body: JSON.stringify(req),
    });
  },

  /** Get mission detail */
  async getMission(missionId: string): Promise<MissionDetail> {
    return fetchApi(`${API_BASE}/missions/${missionId}`);
  },

  /** Delete a mission (only draft/cancelled) */
  async deleteMission(missionId: string): Promise<void> {
    await fetchApi(`${API_BASE}/missions/${missionId}`, { method: 'DELETE' });
  },

  /** Start mission execution */
  async startMission(missionId: string): Promise<{ mission_id: string; status: string }> {
    return fetchApi(`${API_BASE}/missions/${missionId}/start`, { method: 'POST' });
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
  streamMission(missionId: string): EventSource {
    return new EventSource(`${API_BASE}/missions/${missionId}/stream`, {
      withCredentials: true,
    } as EventSourceInit);
  },

  /** List mission artifacts */
  async listArtifacts(missionId: string): Promise<MissionArtifact[]> {
    return fetchApi(`${API_BASE}/missions/${missionId}/artifacts`);
  },

  /** Get a single artifact */
  async getArtifact(artifactId: string): Promise<MissionArtifact> {
    return fetchApi(`${API_BASE}/artifacts/${artifactId}`);
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
