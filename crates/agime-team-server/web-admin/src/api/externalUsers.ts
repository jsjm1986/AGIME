import { fetchApi } from './client';
import type { DocumentSummary } from './documents';

const API_BASE = '/api/team';

function request<T>(path: string, options?: RequestInit): Promise<T> {
  return fetchApi<T>(`${API_BASE}${path}`, options);
}

export type ExternalUserStatus = 'active' | 'disabled';

export interface ExternalUserSummary {
  id: string;
  teamId: string;
  username: string;
  displayName: string | null;
  phone: string | null;
  status: ExternalUserStatus;
  linkedVisitorCount: number;
  uploadCount: number;
  sessionCount: number;
  eventCount: number;
  createdAt: string;
  lastLoginAt: string | null;
  lastSeenAt: string | null;
}

export interface ExternalUserPortalSessionSummary {
  sessionId: string;
  portalId: string | null;
  portalSlug: string | null;
  title: string | null;
  lastMessageAt: string | null;
  updatedAt: string;
  messageCount: number;
  isProcessing: boolean;
}

export interface ExternalUserDetail {
  user: ExternalUserSummary;
  linkedVisitorIds: string[];
  recentUploads: DocumentSummary[];
  recentSessions: ExternalUserPortalSessionSummary[];
}

export interface ExternalUserEventResponse {
  id: string;
  eventId: string;
  teamId: string;
  externalUserId: string | null;
  username: string | null;
  visitorId: string | null;
  portalId: string | null;
  portalSlug: string | null;
  sessionId: string | null;
  eventType: string;
  result: string;
  ipAddress: string | null;
  userAgent: string | null;
  targetDocumentId: string | null;
  metadata: unknown;
  createdAt: string;
}

export interface PaginatedResponse<T> {
  items: T[];
  total: number;
  page: number;
  limit: number;
  totalPages: number;
}

export interface ListExternalUsersParams {
  page?: number;
  limit?: number;
  search?: string;
  status?: ExternalUserStatus | 'all';
}

export interface ListExternalUserEventsParams {
  externalUserId?: string;
  eventType?: string;
  page?: number;
  limit?: number;
}

export const externalUsersApi = {
  async listUsers(teamId: string, params: ListExternalUsersParams = {}): Promise<PaginatedResponse<ExternalUserSummary>> {
    const query = new URLSearchParams();
    query.set('page', String(params.page ?? 1));
    query.set('limit', String(params.limit ?? 20));
    if (params.search?.trim()) {
      query.set('search', params.search.trim());
    }
    if (params.status && params.status !== 'all') {
      query.set('status', params.status);
    }
    return request<PaginatedResponse<ExternalUserSummary>>(`/teams/${teamId}/external-users?${query.toString()}`);
  },

  async getUserDetail(teamId: string, externalUserId: string): Promise<ExternalUserDetail> {
    return request<ExternalUserDetail>(`/teams/${teamId}/external-users/${externalUserId}`);
  },

  async listEvents(teamId: string, params: ListExternalUserEventsParams = {}): Promise<PaginatedResponse<ExternalUserEventResponse>> {
    const query = new URLSearchParams();
    query.set('page', String(params.page ?? 1));
    query.set('limit', String(params.limit ?? 20));
    if (params.externalUserId?.trim()) {
      query.set('external_user_id', params.externalUserId.trim());
    }
    if (params.eventType?.trim()) {
      query.set('event_type', params.eventType.trim());
    }
    return request<PaginatedResponse<ExternalUserEventResponse>>(`/teams/${teamId}/external-users/events?${query.toString()}`);
  },

  async disable(teamId: string, externalUserId: string): Promise<void> {
    await request(`/teams/${teamId}/external-users/${externalUserId}/disable`, { method: 'POST' });
  },

  async enable(teamId: string, externalUserId: string): Promise<void> {
    await request(`/teams/${teamId}/external-users/${externalUserId}/enable`, { method: 'POST' });
  },

  async resetPassword(teamId: string, externalUserId: string, newPassword: string): Promise<void> {
    await request(`/teams/${teamId}/external-users/${externalUserId}/reset-password`, {
      method: 'POST',
      body: JSON.stringify({ new_password: newPassword }),
    });
  },
};
