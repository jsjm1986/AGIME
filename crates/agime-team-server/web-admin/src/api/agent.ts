// Agent API client

import { fetchApi } from './client';

const API_BASE = '/api/team/agent';

// Types
export type ApiFormat = 'openai' | 'anthropic' | 'local';

// Built-in extension types
export type BuiltinExtension =
  | 'skills'
  | 'todo'
  | 'extension_manager'
  | 'team'
  | 'chat_recall'
  | 'document_tools'
  | 'developer'
  | 'memory'
  | 'computer_controller'
  | 'auto_visualiser'
  | 'tutorial';

// Extension configuration
export interface AgentExtensionConfig {
  extension: BuiltinExtension;
  enabled: boolean;
}

// Custom extension configuration
export interface CustomExtensionConfig {
  name: string;
  type: 'sse' | 'stdio';
  uri_or_cmd: string;
  args?: string[];
  envs?: Record<string, string>;
  enabled: boolean;
  source?: string;
  source_extension_id?: string;
}

// Skill assigned to an agent from team shared skills
export interface AgentSkillConfig {
  skill_id: string;
  name: string;
  description?: string;
  enabled: boolean;
  version: string;
}

// All available built-in extensions with metadata
export const BUILTIN_EXTENSIONS: {
  id: BuiltinExtension;
  name: string;
  description: string;
  isPlatform: boolean;
}[] = [
  { id: 'skills', name: 'Skills', description: 'Load and use skills', isPlatform: true },
  { id: 'todo', name: 'Todo', description: 'Task tracking', isPlatform: true },
  { id: 'extension_manager', name: 'Extension Manager', description: 'Extension management', isPlatform: true },
  { id: 'team', name: 'Team', description: 'Team collaboration', isPlatform: true },
  { id: 'chat_recall', name: 'Chat Recall', description: 'Conversation memory', isPlatform: true },
  { id: 'document_tools', name: 'Document Tools', description: 'Read, create, search and list team documents', isPlatform: true },
  { id: 'developer', name: 'Developer', description: 'File editing and shell commands', isPlatform: false },
  { id: 'memory', name: 'Memory', description: 'Knowledge base', isPlatform: false },
  { id: 'computer_controller', name: 'Computer Controller', description: 'Computer control', isPlatform: false },
  { id: 'auto_visualiser', name: 'Auto Visualiser', description: 'Auto visualization', isPlatform: false },
  { id: 'tutorial', name: 'Tutorial', description: 'Tutorials', isPlatform: false },
];

// Default enabled extensions
export const DEFAULT_EXTENSIONS: BuiltinExtension[] = ['skills', 'todo', 'developer', 'extension_manager', 'document_tools'];

// Built-in skills managed by the Skills MCP extension
export const BUILTIN_SKILLS: {
  id: string;
  name: string;
  description: string;
}[] = [
  { id: 'team-onboarding', name: 'Team Onboarding', description: '团队协作入职指南 - 如何使用团队功能搜索、安装和分享资源' },
  { id: 'extension-security-review', name: 'Extension Security Review', description: 'MCP Extension 安全审核指南 - 评估和审核团队共享的扩展' },
];

export type AgentAccessMode = 'all' | 'allowlist' | 'denylist';

export interface TeamAgent {
  id: string;
  team_id: string;
  name: string;
  description?: string;
  system_prompt?: string;
  api_url?: string;
  model?: string;
  api_format: ApiFormat;
  enabled_extensions: AgentExtensionConfig[];
  custom_extensions: CustomExtensionConfig[];
  status: 'idle' | 'running' | 'paused' | 'error';
  last_error?: string;
  access_mode: AgentAccessMode;
  allowed_groups: string[];
  denied_groups: string[];
  max_concurrent_tasks: number;
  temperature?: number;
  max_tokens?: number;
  context_limit?: number;
  assigned_skills: AgentSkillConfig[];
  auto_approve_chat: boolean;
  created_at: string;
  updated_at: string;
}

export interface AgentTask {
  id: string;
  team_id: string;
  agent_id: string;
  submitter_id: string;
  approver_id?: string;
  task_type: 'chat' | 'recipe' | 'skill';
  content: unknown;
  status: 'pending' | 'approved' | 'rejected' | 'running' | 'completed' | 'failed' | 'cancelled';
  priority: number;
  submitted_at: string;
  approved_at?: string;
  started_at?: string;
  completed_at?: string;
  error_message?: string;
}

export interface TaskResult {
  id: string;
  task_id: string;
  result_type: 'message' | 'tool_call' | 'error';
  content: unknown;
  created_at: string;
}

export interface PaginatedResponse<T> {
  items: T[];
  total: number;
  page: number;
  limit: number;
  total_pages: number;
}

// Request types
export interface CreateAgentRequest {
  team_id: string;
  name: string;
  description?: string;
  system_prompt?: string;
  api_url?: string;
  model?: string;
  api_key?: string;
  api_format?: ApiFormat;
  enabled_extensions?: AgentExtensionConfig[];
  custom_extensions?: CustomExtensionConfig[];
  access_mode?: AgentAccessMode;
  allowed_groups?: string[];
  denied_groups?: string[];
  max_concurrent_tasks?: number;
  temperature?: number;
  max_tokens?: number;
  context_limit?: number;
}

export interface UpdateAgentRequest {
  name?: string;
  description?: string;
  system_prompt?: string;
  api_url?: string;
  model?: string;
  api_key?: string;
  api_format?: ApiFormat;
  status?: 'idle' | 'running' | 'paused' | 'error';
  enabled_extensions?: AgentExtensionConfig[];
  custom_extensions?: CustomExtensionConfig[];
  access_mode?: AgentAccessMode;
  allowed_groups?: string[];
  denied_groups?: string[];
  max_concurrent_tasks?: number;
  temperature?: number;
  max_tokens?: number;
  context_limit?: number;
  assigned_skills?: AgentSkillConfig[];
  auto_approve_chat?: boolean;
}

export interface SubmitTaskRequest {
  team_id: string;
  agent_id: string;
  task_type: 'chat' | 'recipe' | 'skill';
  content: unknown;
  priority?: number;
}

// Agent API
export const agentApi = {
  // List agents for a team
  listAgents: (teamId: string, page = 1, limit = 20) =>
    fetchApi<PaginatedResponse<TeamAgent>>(
      `${API_BASE}/agents?team_id=${teamId}&page=${page}&limit=${limit}`
    ),

  // Create agent
  createAgent: (req: CreateAgentRequest) =>
    fetchApi<TeamAgent>(`${API_BASE}/agents`, {
      method: 'POST',
      body: JSON.stringify(req),
    }),

  // Get agent
  getAgent: (id: string) =>
    fetchApi<TeamAgent>(`${API_BASE}/agents/${id}`),

  // Update agent
  updateAgent: (id: string, req: UpdateAgentRequest) =>
    fetchApi<TeamAgent>(`${API_BASE}/agents/${id}`, {
      method: 'PUT',
      body: JSON.stringify(req),
    }),

  // Delete agent
  deleteAgent: (id: string) =>
    fetchApi<void>(`${API_BASE}/agents/${id}`, { method: 'DELETE' }),

  // Update agent access control
  updateAccess: (id: string, req: {
    access_mode: AgentAccessMode;
    allowed_groups?: string[];
    denied_groups?: string[];
  }) =>
    fetchApi<TeamAgent>(`${API_BASE}/agents/${id}/access`, {
      method: 'PUT',
      body: JSON.stringify(req),
    }),

  // Update agent extensions (MCP real-time load/unload)
  updateExtensions: (id: string, req: {
    enabled_extensions?: AgentExtensionConfig[];
    custom_extensions?: CustomExtensionConfig[];
  }) =>
    fetchApi<TeamAgent>(`${API_BASE}/agents/${id}/extensions`, {
      method: 'PUT',
      body: JSON.stringify(req),
    }),

  // Reload agent extensions
  reloadExtensions: (id: string) =>
    fetchApi<{ success: boolean; message: string }>(
      `${API_BASE}/agents/${id}/extensions/reload`,
      { method: 'POST' }
    ),

  // Update agent skills
  updateSkills: (id: string, req: {
    enabled_extensions?: AgentExtensionConfig[];
  }) =>
    fetchApi<TeamAgent>(`${API_BASE}/agents/${id}/skills`, {
      method: 'PUT',
      body: JSON.stringify(req),
    }),

  // Add a team shared extension to an agent
  addTeamExtension: (agentId: string, extensionId: string, teamId: string) =>
    fetchApi<TeamAgent>(`${API_BASE}/agents/${agentId}/extensions/add-team`, {
      method: 'POST',
      body: JSON.stringify({ extension_id: extensionId, team_id: teamId }),
    }),

  // Add a team shared skill to an agent
  addTeamSkill: (agentId: string, skillId: string, teamId: string) =>
    fetchApi<TeamAgent>(`${API_BASE}/agents/${agentId}/skills/add-team`, {
      method: 'POST',
      body: JSON.stringify({ skill_id: skillId, team_id: teamId }),
    }),

  // Remove a skill from an agent
  removeSkill: (agentId: string, skillId: string) =>
    fetchApi<TeamAgent>(`${API_BASE}/agents/${agentId}/skills/${skillId}`, {
      method: 'DELETE',
    }),

  // List available team skills not yet assigned to the agent
  listAvailableSkills: (agentId: string, teamId: string) =>
    fetchApi<{ id: string; name: string; description?: string; version: string }[]>(
      `${API_BASE}/agents/${agentId}/skills/available?team_id=${teamId}`
    ),
};

// Task API
export const taskApi = {
  // List tasks
  listTasks: (teamId: string, page = 1, limit = 20, status?: string) => {
    let url = `${API_BASE}/tasks?team_id=${teamId}&page=${page}&limit=${limit}`;
    if (status) url += `&status=${status}`;
    return fetchApi<PaginatedResponse<AgentTask>>(url);
  },

  // Submit task
  submitTask: (req: SubmitTaskRequest) =>
    fetchApi<AgentTask>(`${API_BASE}/tasks`, {
      method: 'POST',
      body: JSON.stringify(req),
    }),

  // Get task
  getTask: (id: string) =>
    fetchApi<AgentTask>(`${API_BASE}/tasks/${id}`),

  // Approve task
  approveTask: (id: string) =>
    fetchApi<AgentTask>(`${API_BASE}/tasks/${id}/approve`, {
      method: 'POST',
    }),

  // Reject task
  rejectTask: (id: string) =>
    fetchApi<AgentTask>(`${API_BASE}/tasks/${id}/reject`, {
      method: 'POST',
    }),

  // Cancel task
  cancelTask: (id: string) =>
    fetchApi<AgentTask>(`${API_BASE}/tasks/${id}/cancel`, {
      method: 'POST',
    }),

  // Get task results
  getTaskResults: (id: string) =>
    fetchApi<TaskResult[]>(`${API_BASE}/tasks/${id}/results`),

  // Stream task results via SSE
  streamTaskResults: (id: string): EventSource =>
    new EventSource(`${API_BASE}/tasks/${id}/stream`, {
      withCredentials: true,
    }),
};
