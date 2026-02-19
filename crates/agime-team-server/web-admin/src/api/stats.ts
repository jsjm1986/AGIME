// Stats & Recommendations API client

import { fetchApi } from './client';

const API_BASE = '/api/team';

function request<T>(path: string): Promise<T> {
  return fetchApi<T>(`${API_BASE}${path}`);
}

export interface ResourceStats {
  skills: number;
  recipes: number;
  extensions: number;
  documents: number;
  members: number;
}

export interface TrendingItem {
  id: string;
  name: string;
  resourceType: string;
  useCount: number;
}

export interface RecommendedItem {
  id: string;
  name: string;
  description: string | null;
  resourceType: string;
  useCount: number;
  reason: string;
}

export interface AuditLogItem {
  id: string;
  teamId: string;
  userId: string;
  userName: string | null;
  action: string;
  resourceType: string;
  resourceId: string | null;
  resourceName: string | null;
  details: string | null;
  createdAt: string;
}

export interface AuditLogResponse {
  items: AuditLogItem[];
  total: number;
  page: number;
  limit: number;
  total_pages: number;
}

export const statsApi = {
  async getStats(teamId: string): Promise<ResourceStats> {
    return request(`/teams/${teamId}/stats`);
  },

  async getTrending(teamId: string, limit?: number): Promise<TrendingItem[]> {
    const params = limit ? `?limit=${limit}` : '';
    return request(`/teams/${teamId}/trending${params}`);
  },

  async getPopular(teamId: string, limit?: number): Promise<RecommendedItem[]> {
    const params = limit ? `?limit=${limit}` : '';
    return request(`/teams/${teamId}/recommendations/popular${params}`);
  },

  async getNewest(teamId: string, limit?: number): Promise<RecommendedItem[]> {
    const params = limit ? `?limit=${limit}` : '';
    return request(`/teams/${teamId}/recommendations/newest${params}`);
  },
};

export const auditApi = {
  async getAuditLogs(
    teamId: string,
    params?: {
      action?: string;
      resourceType?: string;
      userId?: string;
      page?: number;
      limit?: number;
    }
  ): Promise<AuditLogResponse> {
    const searchParams = new URLSearchParams();
    if (params?.action) searchParams.set('action', params.action);
    if (params?.resourceType) searchParams.set('resource_type', params.resourceType);
    if (params?.userId) searchParams.set('user_id', params.userId);
    if (params?.page) searchParams.set('page', String(params.page));
    if (params?.limit) searchParams.set('limit', String(params.limit));
    const query = searchParams.toString() ? `?${searchParams}` : '';
    return request(`/teams/${teamId}/audit${query}`);
  },

  async getActivity(teamId: string, limit?: number): Promise<AuditLogItem[]> {
    const params = limit ? `?limit=${limit}` : '';
    return request(`/teams/${teamId}/activity${params}`);
  },
};
