// Portal API client - matches MongoDB backend

import { fetchApi } from './client';

const API_BASE = '/api/team';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type PortalStatus = 'draft' | 'published' | 'archived';
export type PortalOutputForm = 'website' | 'widget' | 'agent_only';
export type PortalDocumentAccessMode = 'read_only' | 'co_edit_draft' | 'controlled_write';
export type PortalDomain = 'ecosystem' | 'avatar';
export type PortalPublicExposure = 'public_page' | 'preview_only';

export interface PortalEffectivePublicConfig {
  exposure: PortalPublicExposure;
  publicAccessEnabled: boolean;
  chatEnabled: boolean;
  showChatWidget: boolean;
  effectiveDocumentAccessMode: PortalDocumentAccessMode;
  effectiveAllowedExtensions: string[];
  effectiveAllowedSkillIds: string[];
  extensionsInherited: boolean;
  skillsInherited: boolean;
}

export interface PortalSummary {
  id: string;
  slug: string;
  name: string;
  description: string | null;
  status: PortalStatus;
  outputForm: PortalOutputForm;
  agentEnabled: boolean;
  codingAgentId?: string | null;
  serviceAgentId?: string | null;
  /** Legacy single-agent field */
  agentId?: string | null;
  allowedExtensions?: string[] | null;
  allowedSkillIds?: string[] | null;
  documentAccessMode: PortalDocumentAccessMode;
  domain?: PortalDomain | null;
  tags: string[];
  projectPath?: string | null;
  createdBy: string;
  publishedAt: string | null;
  createdAt: string;
  updatedAt: string;
  /** Public URL for portal access */
  publicUrl: string | null;
  /** Optional testing URL (typically IP:port) */
  testPublicUrl?: string | null;
  /** Authenticated preview URL for admin */
  previewUrl: string;
  effectivePublicConfig?: PortalEffectivePublicConfig | null;
}

export interface PortalDetail {
  id: string;
  teamId: string;
  slug: string;
  name: string;
  description: string | null;
  status: PortalStatus;
  outputForm: PortalOutputForm;
  agentEnabled: boolean;
  codingAgentId: string | null;
  serviceAgentId: string | null;
  /** Legacy single-agent field */
  agentId: string | null;
  agentSystemPrompt: string | null;
  agentWelcomeMessage: string | null;
  boundDocumentIds: string[];
  allowedExtensions?: string[];
  allowedSkillIds?: string[];
  documentAccessMode: PortalDocumentAccessMode;
  domain?: PortalDomain | null;
  tags: string[];
  settings: Record<string, unknown>;
  projectPath?: string;
  createdBy: string;
  publishedAt: string | null;
  createdAt: string;
  updatedAt: string;
  /** Public URL for portal access */
  publicUrl: string | null;
  /** Optional testing URL (typically IP:port) */
  testPublicUrl?: string | null;
  /** Authenticated preview URL for admin */
  previewUrl: string;
  effectivePublicConfig?: PortalEffectivePublicConfig | null;
}

export interface PortalInteraction {
  id: string;
  portal_id: string;
  team_id: string;
  visitor_id: string;
  interaction_type: string;
  page_path: string | null;
  data: unknown;
  created_at: string;
}

export interface PaginatedResponse<T> {
  items: T[];
  total: number;
  page: number;
  limit: number;
  totalPages: number;
}

export interface PortalListResponse extends PaginatedResponse<PortalSummary> {
  portalBaseUrl: string | null;
  portalTestBaseUrl: string | null;
}

export interface CreatePortalRequest {
  name: string;
  slug?: string;
  description?: string;
  outputForm?: PortalOutputForm;
  agentEnabled?: boolean;
  codingAgentId?: string;
  serviceAgentId?: string;
  /** Legacy single-agent field */
  agentId?: string;
  agentSystemPrompt?: string;
  agentWelcomeMessage?: string;
  boundDocumentIds?: string[];
  allowedExtensions?: string[];
  allowedSkillIds?: string[];
  documentAccessMode?: PortalDocumentAccessMode;
  tags?: string[];
  settings?: Record<string, unknown>;
}

export interface UpdatePortalRequest {
  name?: string;
  slug?: string;
  description?: string;
  outputForm?: PortalOutputForm;
  agentEnabled?: boolean;
  codingAgentId?: string | null;
  serviceAgentId?: string | null;
  /** Legacy single-agent field */
  agentId?: string | null;
  agentSystemPrompt?: string | null;
  agentWelcomeMessage?: string | null;
  boundDocumentIds?: string[];
  allowedExtensions?: string[];
  allowedSkillIds?: string[];
  documentAccessMode?: PortalDocumentAccessMode;
  tags?: string[];
  settings?: Record<string, unknown>;
}

export interface PortalStats {
  totalInteractions: number;
  pageViews: number;
  chatMessages: number;
  formSubmits: number;
  uniqueVisitors: number;
}

export interface PortalFileEntry {
  name: string;
  path: string;
  isDir: boolean;
  size?: number | null;
  modifiedAt?: string | null;
}

export interface PortalFileListResponse {
  path: string;
  parentPath?: string | null;
  entries: PortalFileEntry[];
}

export interface PortalFileContentResponse {
  path: string;
  name: string;
  contentType: string;
  size: number;
  modifiedAt?: string | null;
  isText: boolean;
  truncated: boolean;
  content?: string | null;
}

export interface AvatarGovernanceCounts {
  pendingCapabilityRequests: number;
  pendingGapProposals: number;
  pendingOptimizationTickets: number;
  pendingRuntimeLogs: number;
}

export interface AvatarInstanceProjection {
  portalId: string;
  teamId: string;
  slug: string;
  name: string;
  status: string;
  avatarType: string;
  managerAgentId?: string | null;
  serviceAgentId?: string | null;
  documentAccessMode: PortalDocumentAccessMode | string;
  governanceCounts: AvatarGovernanceCounts;
  portalUpdatedAt: string;
  projectedAt: string;
}

interface AvatarInstanceProjectionWire {
  portalId?: string;
  teamId?: string;
  slug?: string;
  name?: string;
  status?: string;
  avatarType?: string;
  managerAgentId?: string | null;
  serviceAgentId?: string | null;
  documentAccessMode?: PortalDocumentAccessMode | string;
  governanceCounts?: Partial<AvatarGovernanceCounts>;
  portalUpdatedAt?: string;
  projectedAt?: string;
  portal_id?: string;
  team_id?: string;
  avatar_type?: string;
  manager_agent_id?: string | null;
  service_agent_id?: string | null;
  document_access_mode?: PortalDocumentAccessMode | string;
  governance_counts?: Partial<AvatarGovernanceCounts>;
  portal_updated_at?: string;
  projected_at?: string;
}

export interface AvatarGovernanceStatePayload {
  portal_id: string;
  team_id: string;
  state: Record<string, unknown>;
  config: Record<string, unknown>;
  updated_at: string;
}

export interface AvatarGovernanceEventPayload {
  event_id: string;
  portal_id: string;
  team_id: string;
  event_type: 'created' | 'updated' | 'removed' | 'config_updated' | string;
  entity_type: 'capability' | 'proposal' | 'ticket' | 'runtime' | 'config' | string;
  entity_id?: string | null;
  title: string;
  status?: string | null;
  detail?: string | null;
  actor_id?: string | null;
  actor_name?: string | null;
  meta: Record<string, unknown>;
  created_at: string;
}

export interface AvatarGovernanceQueueItemPayload {
  id: string;
  kind: 'capability' | 'proposal' | 'ticket' | string;
  title: string;
  detail: string;
  status: string;
  ts: string;
  meta: string[];
  source_id: string;
}

export interface AvatarWorkbenchSummaryPayload {
  portal_id: string;
  team_id: string;
  avatar_name: string;
  avatar_type: string;
  avatar_status: string;
  manager_agent_id?: string | null;
  service_agent_id?: string | null;
  document_access_mode: string;
  work_object_count: number;
  pending_decision_count: number;
  running_mission_count: number;
  needs_attention_count: number;
  last_activity_at: string;
}

export interface AvatarWorkbenchReportItemPayload {
  id: string;
  ts: string;
  kind: string;
  title: string;
  summary: string;
  status: string;
  source: string;
  recommendation?: string | null;
  action_kind?: string | null;
  action_target_id?: string | null;
  work_objects: string[];
  outputs: string[];
  needs_decision: boolean;
}

export interface AvatarWorkbenchDecisionItemPayload {
  id: string;
  ts: string;
  kind: string;
  title: string;
  detail: string;
  status: string;
  risk: string;
  source: string;
  recommendation?: string | null;
  work_objects: string[];
  action_kind?: string | null;
  action_target_id?: string | null;
}

export interface AvatarWorkbenchSnapshotPayload {
  portal_id: string;
  team_id: string;
  summary: AvatarWorkbenchSummaryPayload;
  reports: AvatarWorkbenchReportItemPayload[];
  decisions: AvatarWorkbenchDecisionItemPayload[];
}

// ---------------------------------------------------------------------------
// API functions
// ---------------------------------------------------------------------------

function request<T>(path: string, options?: RequestInit): Promise<T> {
  return fetchApi<T>(`${API_BASE}${path}`, options);
}

function normalizeAvatarGovernanceCounts(
  counts?: Partial<AvatarGovernanceCounts> | null,
): AvatarGovernanceCounts {
  const raw = (counts || {}) as Partial<AvatarGovernanceCounts> & {
    pending_capability_requests?: number;
    pending_gap_proposals?: number;
    pending_optimization_tickets?: number;
    pending_runtime_logs?: number;
  };
  return {
    pendingCapabilityRequests:
      raw.pendingCapabilityRequests ?? raw.pending_capability_requests ?? 0,
    pendingGapProposals: raw.pendingGapProposals ?? raw.pending_gap_proposals ?? 0,
    pendingOptimizationTickets:
      raw.pendingOptimizationTickets ?? raw.pending_optimization_tickets ?? 0,
    pendingRuntimeLogs: raw.pendingRuntimeLogs ?? raw.pending_runtime_logs ?? 0,
  };
}

function normalizeAvatarInstanceProjection(
  item: AvatarInstanceProjectionWire,
): AvatarInstanceProjection {
  return {
    portalId: item.portalId ?? item.portal_id ?? '',
    teamId: item.teamId ?? item.team_id ?? '',
    slug: item.slug ?? '',
    name: item.name ?? '',
    status: item.status ?? '',
    avatarType: item.avatarType ?? item.avatar_type ?? '',
    managerAgentId: item.managerAgentId ?? item.manager_agent_id ?? null,
    serviceAgentId: item.serviceAgentId ?? item.service_agent_id ?? null,
    documentAccessMode:
      item.documentAccessMode ?? item.document_access_mode ?? 'read_only',
    governanceCounts: normalizeAvatarGovernanceCounts(
      item.governanceCounts ?? item.governance_counts,
    ),
    portalUpdatedAt: item.portalUpdatedAt ?? item.portal_updated_at ?? '',
    projectedAt: item.projectedAt ?? item.projected_at ?? '',
  };
}

export const portalApi = {
  async list(
    teamId: string,
    page = 1,
    limit = 20,
    domain?: 'ecosystem' | 'avatar',
  ): Promise<PortalListResponse> {
    const domainQ = domain ? `&domain=${encodeURIComponent(domain)}` : '';
    return request(`/teams/${teamId}/portals?page=${page}&limit=${limit}${domainQ}`);
  },

  async listAvatarInstances(teamId: string): Promise<AvatarInstanceProjection[]> {
    const raw = await request<AvatarInstanceProjectionWire[]>(
      `/agent/avatar-instances?team_id=${encodeURIComponent(teamId)}`,
    );
    return (raw || []).map(normalizeAvatarInstanceProjection);
  },

  async getAvatarGovernance(teamId: string, portalId: string): Promise<AvatarGovernanceStatePayload> {
    return request(`/agent/avatar-governance/${encodeURIComponent(portalId)}?team_id=${encodeURIComponent(teamId)}`);
  },

  async updateAvatarGovernance(
    teamId: string,
    portalId: string,
    req: { state?: Record<string, unknown>; config?: Record<string, unknown> },
  ): Promise<AvatarGovernanceStatePayload> {
    return request(`/agent/avatar-governance/${encodeURIComponent(portalId)}?team_id=${encodeURIComponent(teamId)}`, {
      method: 'PUT',
      body: JSON.stringify(req),
    });
  },

  async listAvatarGovernanceEvents(
    teamId: string,
    portalId: string,
    limit = 120,
  ): Promise<AvatarGovernanceEventPayload[]> {
    return request(
      `/agent/avatar-governance/${encodeURIComponent(portalId)}/events?team_id=${encodeURIComponent(teamId)}&limit=${limit}`,
    );
  },

  async listTeamAvatarGovernanceEvents(
    teamId: string,
    limit = 300,
    portalId?: string,
  ): Promise<AvatarGovernanceEventPayload[]> {
    const params = new URLSearchParams({
      team_id: teamId,
      limit: String(limit),
    });
    if (portalId) params.set('portal_id', portalId);
    return request(`/agent/avatar-governance/events?${params.toString()}`);
  },

  async listAvatarGovernanceQueue(
    teamId: string,
    portalId: string,
  ): Promise<AvatarGovernanceQueueItemPayload[]> {
    return request(
      `/agent/avatar-governance/${encodeURIComponent(portalId)}/queue?team_id=${encodeURIComponent(teamId)}`,
    );
  },

  async getAvatarWorkbenchSnapshot(
    teamId: string,
    portalId: string,
  ): Promise<AvatarWorkbenchSnapshotPayload> {
    return request(
      `/agent/avatar-governance/${encodeURIComponent(portalId)}/workbench?team_id=${encodeURIComponent(teamId)}`,
    );
  },

  async get(teamId: string, portalId: string): Promise<PortalDetail> {
    return request(`/teams/${teamId}/portals/${portalId}`);
  },

  async create(teamId: string, req: CreatePortalRequest): Promise<PortalDetail> {
    return request(`/teams/${teamId}/portals`, {
      method: 'POST',
      body: JSON.stringify(req),
    });
  },

  async update(teamId: string, portalId: string, req: UpdatePortalRequest): Promise<PortalDetail> {
    return request(`/teams/${teamId}/portals/${portalId}`, {
      method: 'PUT',
      body: JSON.stringify(req),
    });
  },

  async delete(teamId: string, portalId: string): Promise<void> {
    await request(`/teams/${teamId}/portals/${portalId}`, { method: 'DELETE' });
  },

  async checkSlug(teamId: string, slug: string): Promise<{ available: boolean; slug: string }> {
    return request(`/teams/${teamId}/portals/check-slug?slug=${encodeURIComponent(slug)}`);
  },

  async publish(teamId: string, portalId: string): Promise<PortalDetail> {
    return request(`/teams/${teamId}/portals/${portalId}/publish`, { method: 'POST' });
  },

  async unpublish(teamId: string, portalId: string): Promise<PortalDetail> {
    return request(`/teams/${teamId}/portals/${portalId}/unpublish`, { method: 'POST' });
  },

  // Interactions & Stats
  async listInteractions(teamId: string, portalId: string, page = 1, limit = 20): Promise<PaginatedResponse<PortalInteraction>> {
    return request(`/teams/${teamId}/portals/${portalId}/interactions?page=${page}&limit=${limit}`);
  },

  async getStats(teamId: string, portalId: string): Promise<PortalStats> {
    return request(`/teams/${teamId}/portals/${portalId}/stats`);
  },

  async listFiles(teamId: string, portalId: string, path = ''): Promise<PortalFileListResponse> {
    const q = path ? `?path=${encodeURIComponent(path)}` : '';
    return request(`/teams/${teamId}/portals/${portalId}/files${q}`);
  },

  async getFile(teamId: string, portalId: string, path: string): Promise<PortalFileContentResponse> {
    return request(`/teams/${teamId}/portals/${portalId}/file?path=${encodeURIComponent(path)}`);
  },
};
