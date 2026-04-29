// Agent API client

import { ApiError, fetchApi } from './client';
import type { PaginatedResponse } from './types';

const API_BASE = '/api/team/agent';

// Types
export type ApiFormat = 'openai' | 'anthropic' | 'litellm' | 'local';
export type SkillBindingMode = 'assigned_only' | 'hybrid' | 'on_demand_only';
export type ApprovalMode = 'leader_owned' | 'headless_fallback';
export type RuntimeOptimizationMode = 'auto' | 'off' | 'prefer';

export interface DelegationPolicy {
  allowPlan: boolean;
  allowSubagent: boolean;
  allowSwarm: boolean;
  allowWorkerMessaging: boolean;
  allowAutoSwarm: boolean;
  allowValidationWorker: boolean;
  approvalMode: ApprovalMode;
  maxSubagentDepth: number;
  parallelismBudget?: number | null;
  swarmBudget?: number | null;
  requireFinalReport: boolean;
}

export interface AttachedTeamExtensionRef {
  extension_id: string;
  enabled: boolean;
  allowed_groups?: string[];
  allowedGroups?: string[];
  runtime_name?: string;
  display_name?: string;
  transport?: string;
}

type AttachedTeamExtensionRefWire = Partial<AttachedTeamExtensionRef> & {
  extensionId?: string;
  runtimeName?: string;
  displayName?: string;
  allowedGroups?: string[];
};

// Built-in extension types
export type BuiltinExtension =
  | 'skills'
  | 'skill_registry'
  | 'tasks'
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
  allowed_groups?: string[];
  allowedGroups?: string[];
}

// Custom extension configuration
export interface CustomExtensionConfig {
  name: string;
  type: 'sse' | 'stdio' | 'streamable_http';
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
  allowed_groups?: string[];
  allowedGroups?: string[];
  version: string;
}

// All available built-in extensions with metadata
export const BUILTIN_EXTENSIONS: {
  id: BuiltinExtension;
  name: string;
  description: string;
  isPlatform: boolean;
  editable?: boolean;
}[] = [
  { id: 'skills', name: 'Skills', description: 'Load and use skills', isPlatform: true, editable: true },
  { id: 'skill_registry', name: 'Skill Registry', description: 'Discover and import remote skills', isPlatform: true, editable: true },
  { id: 'tasks', name: 'Tasks', description: 'Structured task tracking', isPlatform: true, editable: true },
  { id: 'extension_manager', name: 'Extension Manager', description: 'System-managed extension control', isPlatform: true, editable: false },
  { id: 'chat_recall', name: 'Chat Recall', description: 'System-managed conversation memory', isPlatform: true, editable: false },
  { id: 'document_tools', name: 'Document Tools', description: 'Access and work with team documents within the current session scope', isPlatform: true, editable: true },
  { id: 'developer', name: 'Developer', description: 'File editing and shell commands', isPlatform: false, editable: true },
  { id: 'memory', name: 'Memory', description: 'Knowledge base', isPlatform: false, editable: true },
  { id: 'computer_controller', name: 'Computer Controller', description: 'Computer control', isPlatform: false, editable: true },
  { id: 'auto_visualiser', name: 'Auto Visualiser', description: 'Auto visualization', isPlatform: false, editable: true },
  { id: 'tutorial', name: 'Tutorial', description: 'Tutorials', isPlatform: false, editable: true },
];

// Default enabled extensions
export const DEFAULT_EXTENSIONS: BuiltinExtension[] = ['skills', 'tasks', 'developer', 'document_tools'];

export const DEFAULT_DELEGATION_POLICY: DelegationPolicy = {
  allowPlan: true,
  allowSubagent: true,
  allowSwarm: true,
  allowWorkerMessaging: true,
  allowAutoSwarm: true,
  allowValidationWorker: true,
  approvalMode: 'leader_owned',
  maxSubagentDepth: 1,
  parallelismBudget: undefined,
  swarmBudget: undefined,
  requireFinalReport: false,
};

type DelegationPolicyWire = Partial<DelegationPolicy> & {
  allow_plan?: boolean;
  allow_subagent?: boolean;
  allow_swarm?: boolean;
  allow_worker_messaging?: boolean;
  allow_auto_swarm?: boolean;
  allow_validation_worker?: boolean;
  approval_mode?: ApprovalMode;
  max_subagent_depth?: number;
  parallelism_budget?: number | null;
  swarm_budget?: number | null;
  require_final_report?: boolean;
};

export function normalizeDelegationPolicy(raw?: DelegationPolicyWire | null): DelegationPolicy {
  if (!raw) return { ...DEFAULT_DELEGATION_POLICY };
  return {
    allowPlan: raw.allowPlan ?? raw.allow_plan ?? DEFAULT_DELEGATION_POLICY.allowPlan,
    allowSubagent:
      raw.allowSubagent ?? raw.allow_subagent ?? DEFAULT_DELEGATION_POLICY.allowSubagent,
    allowSwarm: raw.allowSwarm ?? raw.allow_swarm ?? DEFAULT_DELEGATION_POLICY.allowSwarm,
    allowWorkerMessaging:
      raw.allowWorkerMessaging ??
      raw.allow_worker_messaging ??
      DEFAULT_DELEGATION_POLICY.allowWorkerMessaging,
    allowAutoSwarm:
      raw.allowAutoSwarm ?? raw.allow_auto_swarm ?? DEFAULT_DELEGATION_POLICY.allowAutoSwarm,
    allowValidationWorker:
      raw.allowValidationWorker ??
      raw.allow_validation_worker ??
      DEFAULT_DELEGATION_POLICY.allowValidationWorker,
    approvalMode:
      raw.approvalMode ??
      raw.approval_mode ??
      DEFAULT_DELEGATION_POLICY.approvalMode,
    maxSubagentDepth:
      raw.maxSubagentDepth ??
      raw.max_subagent_depth ??
      DEFAULT_DELEGATION_POLICY.maxSubagentDepth,
    parallelismBudget:
      raw.parallelismBudget ?? raw.parallelism_budget ?? DEFAULT_DELEGATION_POLICY.parallelismBudget,
    swarmBudget: raw.swarmBudget ?? raw.swarm_budget ?? DEFAULT_DELEGATION_POLICY.swarmBudget,
    requireFinalReport:
      raw.requireFinalReport ??
      raw.require_final_report ??
      DEFAULT_DELEGATION_POLICY.requireFinalReport,
  };
}

function normalizeAttachedTeamExtensionRef(
  raw?: AttachedTeamExtensionRefWire | null,
): AttachedTeamExtensionRef {
  return {
    extension_id: raw?.extension_id ?? raw?.extensionId ?? '',
    enabled: raw?.enabled ?? true,
    allowed_groups: raw?.allowed_groups ?? raw?.allowedGroups ?? [],
    runtime_name: raw?.runtime_name ?? raw?.runtimeName,
    display_name: raw?.display_name ?? raw?.displayName,
    transport: raw?.transport,
  };
}

function normalizeAgentExtensionConfig(raw: AgentExtensionConfig): AgentExtensionConfig {
  return {
    ...raw,
    allowed_groups: raw.allowed_groups ?? raw.allowedGroups ?? [],
  };
}

function normalizeAgentSkillConfig(raw: AgentSkillConfig): AgentSkillConfig {
  return {
    ...raw,
    allowed_groups: raw.allowed_groups ?? raw.allowedGroups ?? [],
  };
}

type TeamAgentWire = Partial<TeamAgent> & {
  teamId?: string;
  systemPrompt?: string;
  apiUrl?: string;
  apiFormat?: ApiFormat;
  enabledExtensions?: AgentExtensionConfig[];
  customExtensions?: CustomExtensionConfig[];
  agentDomain?: string;
  agentRole?: string;
  ownerManagerAgentId?: string;
  templateSourceAgentId?: string;
  lastError?: string;
  allowedGroups?: string[];
  maxConcurrentTasks?: number;
  activeExecutionSlots?: number;
  maxTokens?: number;
  contextLimit?: number;
  thinkingEnabled?: boolean;
  thinkingBudget?: number;
  reasoningEffort?: string;
  outputReserveTokens?: number;
  autoCompactThreshold?: number;
  supportsMultimodal?: boolean;
  promptCachingMode?: RuntimeOptimizationMode;
  cacheEditMode?: RuntimeOptimizationMode;
  assignedSkills?: AgentSkillConfig[];
  skillBindingMode?: SkillBindingMode;
  delegationPolicy?: DelegationPolicyWire;
  attachedTeamExtensions?: AttachedTeamExtensionRefWire[];
  autoApproveChat?: boolean;
  createdAt?: string;
  updatedAt?: string;
};

function normalizeAgent(raw: TeamAgentWire): TeamAgent {
  return {
    id: raw.id || '',
    team_id: raw.team_id ?? raw.teamId ?? '',
    name: raw.name || '',
    description: raw.description,
    avatar: raw.avatar,
    system_prompt: raw.system_prompt ?? raw.systemPrompt,
    api_url: raw.api_url ?? raw.apiUrl,
    model: raw.model,
    api_format: raw.api_format ?? raw.apiFormat ?? 'openai',
    enabled_extensions: (raw.enabled_extensions ?? raw.enabledExtensions ?? []).map(
      normalizeAgentExtensionConfig,
    ),
    custom_extensions: raw.custom_extensions ?? raw.customExtensions ?? [],
    agent_domain: raw.agent_domain ?? raw.agentDomain,
    agent_role: raw.agent_role ?? raw.agentRole,
    owner_manager_agent_id: raw.owner_manager_agent_id ?? raw.ownerManagerAgentId,
    template_source_agent_id: raw.template_source_agent_id ?? raw.templateSourceAgentId,
    status: raw.status ?? 'idle',
    last_error: raw.last_error ?? raw.lastError,
    allowed_groups: raw.allowed_groups ?? raw.allowedGroups ?? [],
    max_concurrent_tasks: raw.max_concurrent_tasks ?? raw.maxConcurrentTasks ?? 5,
    active_execution_slots: raw.active_execution_slots ?? raw.activeExecutionSlots ?? 0,
    temperature: raw.temperature,
    max_tokens: raw.max_tokens ?? raw.maxTokens,
    context_limit: raw.context_limit ?? raw.contextLimit,
    thinking_enabled: raw.thinking_enabled ?? raw.thinkingEnabled ?? false,
    thinking_budget: raw.thinking_budget ?? raw.thinkingBudget,
    reasoning_effort: raw.reasoning_effort ?? raw.reasoningEffort,
    output_reserve_tokens: raw.output_reserve_tokens ?? raw.outputReserveTokens,
    auto_compact_threshold: raw.auto_compact_threshold ?? raw.autoCompactThreshold,
    supports_multimodal: raw.supports_multimodal ?? raw.supportsMultimodal ?? false,
    prompt_caching_mode:
      raw.prompt_caching_mode ?? raw.promptCachingMode ?? 'auto',
    cache_edit_mode: raw.cache_edit_mode ?? raw.cacheEditMode ?? 'auto',
    assigned_skills: (raw.assigned_skills ?? raw.assignedSkills ?? []).map(
      normalizeAgentSkillConfig,
    ),
    skill_binding_mode: raw.skill_binding_mode ?? raw.skillBindingMode ?? 'assigned_only',
    delegation_policy: normalizeDelegationPolicy(
      raw.delegation_policy ?? raw.delegationPolicy,
    ),
    attached_team_extensions: (
      raw.attached_team_extensions ??
      raw.attachedTeamExtensions ??
      []
    ).map(normalizeAttachedTeamExtensionRef),
    auto_approve_chat: raw.auto_approve_chat ?? raw.autoApproveChat ?? false,
    created_at: raw.created_at ?? raw.createdAt ?? '',
    updated_at: raw.updated_at ?? raw.updatedAt ?? '',
  };
}

async function fetchAgent(url: string, init?: RequestInit) {
  const raw = await fetchApi<TeamAgentWire>(url, init);
  return normalizeAgent(raw);
}

async function fetchAgentPage(url: string) {
  const raw = await fetchApi<PaginatedResponse<TeamAgentWire>>(url);
  return {
    ...raw,
    items: (raw.items || []).map(normalizeAgent),
  } as PaginatedResponse<TeamAgent>;
}

// Built-in skills managed by the Skills MCP extension
export const BUILTIN_SKILLS: {
  id: string;
  name: string;
  description: string;
}[] = [
  { id: 'team-onboarding', name: 'Team Onboarding', description: '团队协作入职指南 - 如何使用团队功能搜索、安装和分享资源' },
  { id: 'extension-security-review', name: 'Extension Security Review', description: 'MCP Extension 安全审核指南 - 评估和审核团队共享的扩展' },
];

export interface TeamAgent {
  id: string;
  team_id: string;
  name: string;
  description?: string;
  avatar?: string;
  system_prompt?: string;
  api_url?: string;
  model?: string;
  api_format: ApiFormat;
  enabled_extensions: AgentExtensionConfig[];
  custom_extensions: CustomExtensionConfig[];
  agent_domain?: 'general' | 'digital_avatar' | 'ecosystem_portal' | string;
  agent_role?: 'manager' | 'service' | string;
  owner_manager_agent_id?: string;
  template_source_agent_id?: string;
  status: 'idle' | 'running' | 'paused' | 'error';
  last_error?: string;
  allowed_groups: string[];
  max_concurrent_tasks: number;
  active_execution_slots: number;
  temperature?: number;
  max_tokens?: number;
  context_limit?: number;
  thinking_enabled: boolean;
  thinking_budget?: number;
  reasoning_effort?: string;
  output_reserve_tokens?: number;
  auto_compact_threshold?: number;
  supports_multimodal: boolean;
  prompt_caching_mode: RuntimeOptimizationMode;
  cache_edit_mode: RuntimeOptimizationMode;
  assigned_skills: AgentSkillConfig[];
  skill_binding_mode: SkillBindingMode;
  delegation_policy: DelegationPolicy;
  attached_team_extensions: AttachedTeamExtensionRef[];
  auto_approve_chat: boolean;
  created_at: string;
  updated_at: string;
}

export interface RuntimeProfilePreview {
  model: string;
  userIntent?: {
    modelName: string;
    apiFormat?: string | null;
    apiUrl?: string | null;
    contextLimit?: number | null;
    maxTokens?: number | null;
    thinkingEnabled?: boolean | null;
    thinkingBudget?: number | null;
    reasoningEffort?: string | null;
    outputReserveTokens?: number | null;
    autoCompactThreshold?: number | null;
    promptCachingMode?: RuntimeOptimizationMode;
    cacheEditMode?: RuntimeOptimizationMode;
  };
  hintedCapabilities?: {
    matchedPattern?: string | null;
    provider?: string | null;
    contextLength?: number | null;
    maxCompletionTokens?: number | null;
    supportsTemperature?: boolean;
    supportsThinking?: boolean;
    supportsReasoning?: boolean;
    supportsTools?: boolean;
    useMaxCompletionTokens?: boolean;
  };
  runtimeCapabilities?: {
    contextLength?: number | null;
    maxCompletionTokens?: number | null;
    supportsTemperature?: boolean;
    supportsThinking?: boolean;
    supportsReasoning?: boolean;
    supportsPromptCaching?: boolean;
    supportsCacheEdit?: boolean;
    useMaxCompletionTokens?: boolean;
  };
  effectiveExecution?: {
    contextLimit?: number | null;
    maxCompletionTokens?: number | null;
    useMaxCompletionTokens?: boolean;
    outputReserveTokens?: number | null;
    autoCompactThreshold?: number | null;
    thinkingEnabled?: boolean;
    thinkingBudget?: number | null;
    reasoningEffort?: string | null;
    promptCachingMode?: RuntimeOptimizationMode;
    cacheEditMode?: RuntimeOptimizationMode;
  };
  downgrades?: Array<{
    field: string;
    from?: string | null;
    to?: string | null;
    reason: string;
    source: string;
  }>;
  warnings?: string[];
  sourceBreakdown?: {
    apiFormat?: string | null;
    apiUrl?: string | null;
    providerMode?: string;
    matchedPattern?: string | null;
  };
}

export interface AgentTask {
  id: string;
  team_id: string;
  agent_id: string;
  submitter_id: string;
  approver_id?: string;
  task_type: 'chat' | 'recipe' | 'skill';
  content: unknown;
  status: 'pending' | 'approved' | 'queued' | 'rejected' | 'running' | 'completed' | 'failed' | 'cancelled';
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

// Request types
export interface CreateAgentRequest {
  team_id: string;
  name: string;
  description?: string;
  avatar?: string;
  system_prompt?: string;
  api_url?: string;
  model?: string;
  api_key?: string;
  api_format?: ApiFormat;
  enabled_extensions?: AgentExtensionConfig[];
  custom_extensions?: CustomExtensionConfig[];
  agent_domain?: string;
  agent_role?: string;
  owner_manager_agent_id?: string;
  template_source_agent_id?: string;
  allowed_groups?: string[];
  max_concurrent_tasks?: number;
  temperature?: number;
  max_tokens?: number;
  context_limit?: number;
  thinking_enabled?: boolean;
  thinking_budget?: number;
  reasoning_effort?: string;
  output_reserve_tokens?: number;
  auto_compact_threshold?: number;
  supports_multimodal?: boolean;
  prompt_caching_mode?: RuntimeOptimizationMode;
  cache_edit_mode?: RuntimeOptimizationMode;
  assigned_skills?: AgentSkillConfig[];
  skill_binding_mode?: SkillBindingMode;
  delegation_policy?: DelegationPolicy;
  attached_team_extensions?: AttachedTeamExtensionRef[];
}

export interface UpdateAgentRequest {
  name?: string;
  description?: string;
  avatar?: string;
  system_prompt?: string;
  api_url?: string;
  model?: string;
  api_key?: string;
  api_format?: ApiFormat;
  status?: 'idle' | 'running' | 'paused' | 'error';
  enabled_extensions?: AgentExtensionConfig[];
  custom_extensions?: CustomExtensionConfig[];
  agent_domain?: string;
  agent_role?: string;
  owner_manager_agent_id?: string;
  template_source_agent_id?: string;
  allowed_groups?: string[];
  max_concurrent_tasks?: number;
  temperature?: number;
  max_tokens?: number;
  context_limit?: number;
  thinking_enabled?: boolean;
  thinking_budget?: number;
  reasoning_effort?: string;
  output_reserve_tokens?: number;
  auto_compact_threshold?: number;
  supports_multimodal?: boolean;
  prompt_caching_mode?: RuntimeOptimizationMode;
  cache_edit_mode?: RuntimeOptimizationMode;
  assigned_skills?: AgentSkillConfig[];
  skill_binding_mode?: SkillBindingMode;
  delegation_policy?: DelegationPolicy;
  attached_team_extensions?: AttachedTeamExtensionRef[];
  auto_approve_chat?: boolean;
}

export interface ProvisionFromTemplateRequest {
  name?: string;
  agent_domain?: string;
  agent_role?: string;
  owner_manager_agent_id?: string;
  template_source_agent_id?: string;
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
    fetchAgentPage(
      `${API_BASE}/agents?team_id=${teamId}&page=${page}&limit=${limit}`
    ),

  // Create agent
  createAgent: (req: CreateAgentRequest) =>
    fetchAgent(`${API_BASE}/agents`, {
      method: 'POST',
      body: JSON.stringify(req),
    }),

  // Get agent
  getAgent: (id: string) =>
    fetchAgent(`${API_BASE}/agents/${id}`),

  // Update agent
  updateAgent: (id: string, req: UpdateAgentRequest) =>
    fetchAgent(`${API_BASE}/agents/${id}`, {
      method: 'PUT',
      body: JSON.stringify(req),
    }),

  runtimeProfilePreview: (
    model: string,
    apiFormat?: ApiFormat,
    options?: {
      apiUrl?: string;
      contextLimit?: number;
      maxTokens?: number;
      thinkingEnabled?: boolean;
      thinkingBudget?: number;
      reasoningEffort?: string;
      outputReserveTokens?: number;
      autoCompactThreshold?: number;
      promptCachingMode?: RuntimeOptimizationMode;
      cacheEditMode?: RuntimeOptimizationMode;
    }
  ) => {
    const params = new URLSearchParams();
    params.set('model', model);
    if (apiFormat) params.set('api_format', apiFormat);
    if (options?.apiUrl) params.set('api_url', options.apiUrl);
    if (options?.contextLimit != null) params.set('context_limit', String(options.contextLimit));
    if (options?.maxTokens != null) params.set('max_tokens', String(options.maxTokens));
    if (options?.thinkingEnabled != null) params.set('thinking_enabled', String(options.thinkingEnabled));
    if (options?.thinkingBudget != null) params.set('thinking_budget', String(options.thinkingBudget));
    if (options?.reasoningEffort) params.set('reasoning_effort', options.reasoningEffort);
    if (options?.outputReserveTokens != null) params.set('output_reserve_tokens', String(options.outputReserveTokens));
    if (options?.autoCompactThreshold != null) params.set('auto_compact_threshold', String(options.autoCompactThreshold));
    if (options?.promptCachingMode) params.set('prompt_caching_mode', options.promptCachingMode);
    if (options?.cacheEditMode) params.set('cache_edit_mode', options.cacheEditMode);
    return fetchApi<RuntimeProfilePreview>(`${API_BASE}/runtime-profile-preview?${params.toString()}`);
  },

  // Delete agent
  deleteAgent: (id: string) =>
    fetchApi<void>(`${API_BASE}/agents/${id}`, { method: 'DELETE' }),

  // Clone agent from an existing template
  // Legacy endpoint kept for backward compatibility with older servers.
  cloneAgent: (id: string, req: ProvisionFromTemplateRequest) =>
    fetchAgent(`${API_BASE}/agents/${id}/clone`, {
      method: 'POST',
      body: JSON.stringify(req),
    }),

  // Preferred endpoint: provision dedicated agent from template configuration.
  // Falls back to legacy /clone for backward compatibility.
  provisionFromTemplate: async (id: string, req: ProvisionFromTemplateRequest) => {
    try {
      return await fetchAgent(`${API_BASE}/agents/${id}/provision-from-template`, {
        method: 'POST',
        body: JSON.stringify(req),
      });
    } catch (error) {
      if (error instanceof ApiError && error.status === 404) {
        return fetchAgent(`${API_BASE}/agents/${id}/clone`, {
          method: 'POST',
          body: JSON.stringify(req),
        });
      }
      throw error;
    }
  },

  // Update agent access control
  updateAccess: (id: string, req: {
    allowed_groups: string[];
  }) =>
    fetchAgent(`${API_BASE}/agents/${id}/access`, {
      method: 'PUT',
      body: JSON.stringify(req),
    }),

  // Update agent extensions (MCP real-time load/unload)
  updateExtensions: (id: string, req: {
    enabled_extensions?: AgentExtensionConfig[];
    custom_extensions?: CustomExtensionConfig[];
  }) =>
    fetchAgent(`${API_BASE}/agents/${id}/extensions`, {
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
    assigned_skills?: AgentSkillConfig[];
  }) =>
    fetchAgent(`${API_BASE}/agents/${id}/skills`, {
      method: 'PUT',
      body: JSON.stringify(req),
    }),

  // Add a team shared extension to an agent
  addTeamExtension: (agentId: string, extensionId: string, teamId: string) =>
    fetchAgent(`${API_BASE}/agents/${agentId}/extensions/add-team`, {
      method: 'POST',
      body: JSON.stringify({ extension_id: extensionId, team_id: teamId }),
    }),

  // Add a custom MCP extension directly to an agent
  addCustomExtension: (agentId: string, teamId: string, extension: CustomExtensionConfig) =>
    fetchAgent(`${API_BASE}/agents/${agentId}/extensions/custom`, {
      method: 'POST',
      body: JSON.stringify({ team_id: teamId, extension }),
    }),

  // Enable or disable an existing custom MCP extension on an agent
  setCustomExtensionEnabled: (
    agentId: string,
    extensionName: string,
    teamId: string,
    enabled: boolean,
  ) =>
    fetchAgent(
      `${API_BASE}/agents/${agentId}/extensions/custom/${encodeURIComponent(extensionName)}?team_id=${encodeURIComponent(teamId)}`,
      {
        method: 'PATCH',
        body: JSON.stringify({ enabled }),
      },
    ),

  // Remove a custom MCP extension from an agent
  removeCustomExtension: (agentId: string, extensionName: string, teamId: string) =>
    fetchAgent(
      `${API_BASE}/agents/${agentId}/extensions/custom/${encodeURIComponent(extensionName)}?team_id=${encodeURIComponent(teamId)}`,
      {
        method: 'DELETE',
      },
    ),

  // Add a team shared skill to an agent
  addTeamSkill: (agentId: string, skillId: string, teamId: string) =>
    fetchAgent(`${API_BASE}/agents/${agentId}/skills/add-team`, {
      method: 'POST',
      body: JSON.stringify({ skill_id: skillId, team_id: teamId }),
    }),

  // Remove a skill from an agent
  removeSkill: (agentId: string, skillId: string) =>
    fetchAgent(`${API_BASE}/agents/${agentId}/skills/${skillId}`, {
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
