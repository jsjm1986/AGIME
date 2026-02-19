// Portal API client - matches MongoDB backend

import { fetchApi } from './client';

const API_BASE = '/api/team';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type PortalStatus = 'draft' | 'published' | 'archived';
export type PortalOutputForm = 'website' | 'widget' | 'agent_only';

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
  tags: string[];
  createdBy: string;
  publishedAt: string | null;
  createdAt: string;
  updatedAt: string;
  /** Public URL only when portal is published */
  publicUrl: string | null;
  /** Optional testing URL (typically IP:port) for published portal */
  testPublicUrl?: string | null;
  /** Authenticated preview URL, available for draft/published */
  previewUrl: string;
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
  tags: string[];
  settings: Record<string, unknown>;
  projectPath?: string;
  createdBy: string;
  publishedAt: string | null;
  createdAt: string;
  updatedAt: string;
  /** Public URL only when portal is published */
  publicUrl: string | null;
  /** Optional testing URL (typically IP:port) for published portal */
  testPublicUrl?: string | null;
  /** Authenticated preview URL, available for draft/published */
  previewUrl: string;
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
  tags?: string[];
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
  tags?: string[];
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

// ---------------------------------------------------------------------------
// API functions
// ---------------------------------------------------------------------------

function request<T>(path: string, options?: RequestInit): Promise<T> {
  return fetchApi<T>(`${API_BASE}${path}`, options);
}

export const portalApi = {
  async list(teamId: string, page = 1, limit = 20): Promise<PaginatedResponse<PortalSummary>> {
    return request(`/teams/${teamId}/portals?page=${page}&limit=${limit}`);
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
