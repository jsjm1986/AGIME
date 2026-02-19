// User Groups API client

import { fetchApi } from './client';

const API_BASE = '/api/team';

export interface UserGroupSummary {
  id: string;
  name: string;
  description?: string;
  memberCount: number;
  color?: string;
  isSystem: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface UserGroupDetail {
  id: string;
  name: string;
  description?: string;
  members: string[];
  color?: string;
  isSystem: boolean;
  createdBy: string;
  createdAt: string;
  updatedAt: string;
}

export interface PaginatedGroups {
  items: UserGroupSummary[];
  total: number;
  page: number;
  limit: number;
  total_pages: number;
}

export const userGroupApi = {
  list: (teamId: string, page = 1, limit = 20) =>
    fetchApi<PaginatedGroups>(
      `${API_BASE}/teams/${teamId}/groups?page=${page}&limit=${limit}`
    ),

  get: (teamId: string, groupId: string) =>
    fetchApi<UserGroupDetail>(
      `${API_BASE}/teams/${teamId}/groups/${groupId}`
    ),

  create: (teamId: string, req: {
    name: string;
    description?: string;
    members?: string[];
    color?: string;
  }) =>
    fetchApi<UserGroupDetail>(
      `${API_BASE}/teams/${teamId}/groups`,
      { method: 'POST', body: JSON.stringify(req) }
    ),

  update: (teamId: string, groupId: string, req: {
    name?: string;
    description?: string;
    color?: string;
  }) =>
    fetchApi<UserGroupDetail>(
      `${API_BASE}/teams/${teamId}/groups/${groupId}`,
      { method: 'PUT', body: JSON.stringify(req) }
    ),

  delete: (teamId: string, groupId: string) =>
    fetchApi<void>(
      `${API_BASE}/teams/${teamId}/groups/${groupId}`,
      { method: 'DELETE' }
    ),

  updateMembers: (teamId: string, groupId: string, req: {
    add?: string[];
    remove?: string[];
  }) =>
    fetchApi<UserGroupDetail>(
      `${API_BASE}/teams/${teamId}/groups/${groupId}/members`,
      { method: 'PUT', body: JSON.stringify(req) }
    ),
};
