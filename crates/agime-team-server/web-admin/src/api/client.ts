const API_BASE = '/api';

/**
 * Shared fetch helper for all API modules.
 * Handles credentials, JSON headers, 204 No Content, and error extraction.
 */
export async function fetchApi<T>(url: string, options?: RequestInit): Promise<T> {
  const res = await fetch(url, {
    ...options,
    credentials: 'include',
    headers: {
      'Content-Type': 'application/json',
      ...options?.headers,
    },
  });
  if (!res.ok) {
    const body = await res.text().catch(() => '');
    let message = `${res.status}: ${body || res.statusText}`;
    try {
      const parsed = JSON.parse(body);
      if (parsed.error) message = parsed.error;
    } catch { /* use raw text */ }
    throw new Error(message);
  }
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  if (!text) return undefined as T;
  return JSON.parse(text);
}

import type {
  TeamsResponse,
  TeamResponse,
  TeamSummaryResponse,
  MembersResponse,
  TeamMember,
  SkillsResponse,
  SharedSkill,
  SkillFile,
  RecipesResponse,
  SharedRecipe,
  ExtensionsResponse,
  SharedExtension,
  InvitesResponse,
  CreateInviteResponse,
  ValidateInviteResponse,
  AcceptInviteResponse,
  TeamRole,
  SmartLogParams,
  SmartLogsResponse,
  TeamSettingsResponse,
  UpdateTeamSettingsRequest,
} from './types';

export interface User {
  id: string;
  email: string;
  display_name: string;
  role: string;
}

export interface ApiKey {
  id: string;
  key_prefix: string;
  name: string | null;
  last_used_at: string | null;
  expires_at: string | null;
  created_at: string;
}

export interface RegisterResponse {
  user: User;
  api_key: string;
  message: string;
}

export interface SessionResponse {
  user: User;
}

export interface ApiKeysResponse {
  keys: ApiKey[];
}

export interface CreateApiKeyResponse {
  key: {
    id: string;
    api_key: string;
    name: string | null;
    expires_at: string | null;
  };
  message: string;
}

class ApiClient {
  private request<T>(path: string, options?: RequestInit): Promise<T> {
    return fetchApi<T>(`${API_BASE}${path}`, options);
  }

  async register(email: string, displayName: string, password?: string): Promise<RegisterResponse> {
    return this.request('/auth/register', {
      method: 'POST',
      body: JSON.stringify({ email, display_name: displayName, password }),
    });
  }

  async login(apiKey: string): Promise<SessionResponse> {
    return this.request('/auth/login', {
      method: 'POST',
      body: JSON.stringify({ api_key: apiKey }),
    });
  }

  async loginWithPassword(email: string, password: string): Promise<SessionResponse> {
    return this.request('/auth/login/password', {
      method: 'POST',
      body: JSON.stringify({ email, password }),
    });
  }

  async logout(): Promise<void> {
    await this.request('/auth/logout', { method: 'POST' });
  }

  async getSession(): Promise<SessionResponse> {
    return this.request('/auth/session');
  }

  async changePassword(currentPassword: string | null, newPassword: string): Promise<void> {
    await this.request('/auth/change-password', {
      method: 'POST',
      body: JSON.stringify({ current_password: currentPassword, new_password: newPassword }),
    });
  }

  async getRegistrations(): Promise<{ requests: Array<{ request_id: string; email: string; display_name: string; status: string; created_at: string }> }> {
    return this.request('/auth/admin/registrations');
  }

  async approveRegistration(id: string): Promise<RegisterResponse> {
    return this.request(`/auth/admin/registrations/${id}/approve`, { method: 'POST' });
  }

  async rejectRegistration(id: string, reason?: string): Promise<void> {
    await this.request(`/auth/admin/registrations/${id}/reject`, {
      method: 'POST',
      body: JSON.stringify({ reason }),
    });
  }

  async getApiKeys(): Promise<ApiKeysResponse> {
    return this.request('/auth/keys');
  }

  async createApiKey(name?: string, expiresInDays?: number): Promise<CreateApiKeyResponse> {
    return this.request('/auth/keys', {
      method: 'POST',
      body: JSON.stringify({ name, expires_in_days: expiresInDays }),
    });
  }

  async revokeApiKey(keyId: string): Promise<void> {
    await this.request(`/auth/keys/${keyId}`, { method: 'DELETE' });
  }

  // Team methods
  async getTeams(): Promise<TeamsResponse> {
    return this.request('/team/teams');
  }

  async createTeam(name: string, description?: string, repoUrl?: string): Promise<TeamResponse> {
    return this.request('/team/teams', {
      method: 'POST',
      body: JSON.stringify({ name, description, repositoryUrl: repoUrl }),
    });
  }

  async getTeam(teamId: string): Promise<TeamResponse> {
    const response: TeamSummaryResponse = await this.request(`/team/teams/${teamId}`);
    // 将嵌套结构转换为扁平结构
    return {
      team: {
        ...response.team,
        membersCount: response.membersCount,
        skillsCount: response.skillsCount,
        recipesCount: response.recipesCount,
        extensionsCount: response.extensionsCount,
        currentUserId: response.currentUserId,
        currentUserRole: response.currentUserRole,
      }
    };
  }

  async updateTeam(teamId: string, data: { name?: string; description?: string; repositoryUrl?: string }): Promise<TeamResponse> {
    return this.request(`/team/teams/${teamId}`, {
      method: 'PUT',
      body: JSON.stringify(data),
    });
  }

  async deleteTeam(teamId: string): Promise<void> {
    await this.request(`/team/teams/${teamId}`, { method: 'DELETE' });
  }

  // Member methods
  async getMembers(teamId: string): Promise<MembersResponse> {
    return this.request(`/team/teams/${teamId}/members`);
  }

  async addMember(teamId: string, email: string, role: TeamRole): Promise<{ member: TeamMember }> {
    return this.request(`/team/teams/${teamId}/members`, {
      method: 'POST',
      body: JSON.stringify({ email, role }),
    });
  }

  async updateMember(memberId: string, role: TeamRole): Promise<{ member: TeamMember }> {
    return this.request(`/team/members/${memberId}`, {
      method: 'PUT',
      body: JSON.stringify({ role }),
    });
  }

  async removeMember(memberId: string): Promise<void> {
    await this.request(`/team/members/${memberId}`, { method: 'DELETE' });
  }

  // Skill methods
  async getSkills(teamId: string, params?: { page?: number; limit?: number; search?: string; sort?: string }): Promise<SkillsResponse> {
    const query = new URLSearchParams({ teamId });
    if (params?.page) query.set('page', String(params.page));
    if (params?.limit) query.set('limit', String(params.limit));
    if (params?.search) query.set('search', params.search);
    if (params?.sort) query.set('sort', params.sort);
    return this.request(`/team/skills?${query}`);
  }

  async getSkill(skillId: string): Promise<SharedSkill> {
    return this.request(`/team/skills/${skillId}`);
  }

  async updateSkill(skillId: string, data: { name?: string; description?: string; content?: string }): Promise<{ skill: SharedSkill }> {
    return this.request(`/team/skills/${skillId}`, {
      method: 'PUT',
      body: JSON.stringify(data),
    });
  }

  async deleteSkill(skillId: string): Promise<void> {
    await this.request(`/team/skills/${skillId}`, { method: 'DELETE' });
  }

  // Recipe methods
  async getRecipes(teamId: string, params?: { page?: number; limit?: number; search?: string; sort?: string }): Promise<RecipesResponse> {
    const query = new URLSearchParams({ teamId });
    if (params?.page) query.set('page', String(params.page));
    if (params?.limit) query.set('limit', String(params.limit));
    if (params?.search) query.set('search', params.search);
    if (params?.sort) query.set('sort', params.sort);
    return this.request(`/team/recipes?${query}`);
  }

  async getRecipe(recipeId: string): Promise<SharedRecipe> {
    return this.request(`/team/recipes/${recipeId}`);
  }

  async updateRecipe(recipeId: string, data: { name?: string; description?: string; content?: string }): Promise<{ recipe: SharedRecipe }> {
    const payload: {
      description?: string;
      contentYaml?: string;
    } = {
      description: data.description,
      contentYaml: data.content,
    };

    return this.request(`/team/recipes/${recipeId}`, {
      method: 'PUT',
      body: JSON.stringify(payload),
    });
  }

  async deleteRecipe(recipeId: string): Promise<void> {
    await this.request(`/team/recipes/${recipeId}`, { method: 'DELETE' });
  }

  // Extension methods
  async getExtensions(teamId: string, params?: { page?: number; limit?: number; search?: string; sort?: string }): Promise<ExtensionsResponse> {
    const query = new URLSearchParams({ teamId });
    if (params?.page) query.set('page', String(params.page));
    if (params?.limit) query.set('limit', String(params.limit));
    if (params?.search) query.set('search', params.search);
    if (params?.sort) query.set('sort', params.sort);
    return this.request(`/team/extensions?${query}`);
  }

  async getExtension(extensionId: string): Promise<SharedExtension> {
    return this.request(`/team/extensions/${extensionId}`);
  }

  async updateExtension(extensionId: string, data: { name?: string; description?: string; config?: string }): Promise<{ extension: SharedExtension }> {
    let parsedConfig: Record<string, unknown> | undefined;
    if (typeof data.config === 'string' && data.config.trim().length > 0) {
      try {
        parsedConfig = JSON.parse(data.config) as Record<string, unknown>;
      } catch {
        throw new Error('Invalid extension config JSON');
      }
    }

    const payload: {
      name?: string;
      description?: string;
      config?: Record<string, unknown>;
    } = {
      name: data.name,
      description: data.description,
      config: parsedConfig,
    };

    return this.request(`/team/extensions/${extensionId}`, {
      method: 'PUT',
      body: JSON.stringify(payload),
    });
  }

  async deleteExtension(extensionId: string): Promise<void> {
    await this.request(`/team/extensions/${extensionId}`, { method: 'DELETE' });
  }

  // Skill create/install methods
  async createSkill(data: {
    teamId: string;
    name: string;
    content?: string;
    description?: string;
    tags?: string[];
    visibility?: string;
    skillMd?: string;
    files?: SkillFile[];
  }): Promise<{ skill: SharedSkill }> {
    return this.request('/team/skills', {
      method: 'POST',
      body: JSON.stringify(data),
    });
  }

  async installSkill(skillId: string): Promise<{ success: boolean; localPath?: string; error?: string }> {
    return this.request(`/team/skills/${skillId}/install`, { method: 'POST' });
  }

  async uninstallSkill(skillId: string): Promise<void> {
    await this.request(`/team/skills/${skillId}/uninstall`, { method: 'DELETE' });
  }

  // Recipe create/install methods
  async createRecipe(data: {
    teamId: string;
    name: string;
    contentYaml: string;
    description?: string;
    category?: string;
    tags?: string[];
    visibility?: string;
  }): Promise<{ recipe: SharedRecipe }> {
    return this.request('/team/recipes', {
      method: 'POST',
      body: JSON.stringify(data),
    });
  }

  async installRecipe(recipeId: string): Promise<{ success: boolean; localPath?: string; error?: string }> {
    return this.request(`/team/recipes/${recipeId}/install`, { method: 'POST' });
  }

  async uninstallRecipe(recipeId: string): Promise<void> {
    await this.request(`/team/recipes/${recipeId}/uninstall`, { method: 'DELETE' });
  }

  // Extension create/install methods
  async createExtension(data: {
    teamId: string;
    name: string;
    extensionType: string;
    config: Record<string, unknown>;
    description?: string;
    tags?: string[];
    visibility?: string;
  }): Promise<{ extension: SharedExtension }> {
    return this.request('/team/extensions', {
      method: 'POST',
      body: JSON.stringify(data),
    });
  }

  async installExtension(extensionId: string): Promise<{ success: boolean; localPath?: string; error?: string }> {
    return this.request(`/team/extensions/${extensionId}/install`, { method: 'POST' });
  }

  async uninstallExtension(extensionId: string): Promise<void> {
    await this.request(`/team/extensions/${extensionId}/uninstall`, { method: 'DELETE' });
  }

  async reviewExtension(extensionId: string, approved: boolean, notes?: string): Promise<{ extension: SharedExtension }> {
    return this.request(`/team/extensions/${extensionId}/review`, {
      method: 'POST',
      body: JSON.stringify({ approved, notes }),
    });
  }

  // Invite methods
  async getInvites(teamId: string): Promise<InvitesResponse> {
    return this.request(`/team/teams/${teamId}/invites`);
  }

  async createInvite(teamId: string, role: TeamRole, expiresInDays?: number, maxUses?: number): Promise<CreateInviteResponse> {
    return this.request(`/team/teams/${teamId}/invites`, {
      method: 'POST',
      body: JSON.stringify({ role, expires_in_days: expiresInDays, max_uses: maxUses }),
    });
  }

  async revokeInvite(teamId: string, code: string): Promise<void> {
    await this.request(`/team/teams/${teamId}/invites/${code}`, { method: 'DELETE' });
  }

  // Public invite methods (no auth required for validation)
  async validateInvite(code: string): Promise<ValidateInviteResponse> {
    return this.request(`/team/invites/${code}`);
  }

  async acceptInvite(code: string, displayName?: string): Promise<AcceptInviteResponse> {
    return this.request(`/team/invites/${code}/accept`, {
      method: 'POST',
      body: JSON.stringify({ displayName }),
    });
  }

  // AI Describe methods
  async describeExtension(teamId: string, extId: string, lang: string): Promise<{ description: string; lang: string; generated_at: string }> {
    return this.request(`/teams/${teamId}/extensions/${extId}/ai-describe`, {
      method: 'POST',
      body: JSON.stringify({ lang }),
    });
  }

  async describeSkill(teamId: string, skillId: string, lang: string): Promise<{ description: string; lang: string; generated_at: string }> {
    return this.request(`/teams/${teamId}/skills/${skillId}/ai-describe`, {
      method: 'POST',
      body: JSON.stringify({ lang }),
    });
  }

  async describeBuiltinExtension(teamId: string, req: { id: string; name: string; description: string; is_platform: boolean; lang: string }): Promise<{ description: string; lang: string; generated_at: string }> {
    return this.request(`/teams/${teamId}/builtin-extensions/ai-describe`, {
      method: 'POST',
      body: JSON.stringify(req),
    });
  }

  async describeBuiltinExtensionsBatch(teamId: string, lang: string): Promise<{ generated: number; total: number }> {
    return this.request(`/teams/${teamId}/builtin-extensions/ai-describe-batch`, {
      method: 'POST',
      body: JSON.stringify({ lang }),
    });
  }

  async describeSkillsBatch(teamId: string, lang: string): Promise<{ generated: number }> {
    return this.request(`/teams/${teamId}/skills/ai-describe-batch`, {
      method: 'POST',
      body: JSON.stringify({ lang }),
    });
  }

  async describeExtensionsBatch(teamId: string, lang: string): Promise<{ generated: number }> {
    return this.request(`/teams/${teamId}/extensions/ai-describe-batch`, {
      method: 'POST',
      body: JSON.stringify({ lang }),
    });
  }

  async describeBuiltinSkill(teamId: string, req: { id: string; name: string; description: string; lang: string }): Promise<{ description: string; lang: string; generated_at: string }> {
    return this.request(`/teams/${teamId}/builtin-skills/ai-describe`, {
      method: 'POST',
      body: JSON.stringify(req),
    });
  }

  async describeBuiltinSkillsBatch(teamId: string, lang: string): Promise<{ generated: number; total: number }> {
    return this.request(`/teams/${teamId}/builtin-skills/ai-describe-batch`, {
      method: 'POST',
      body: JSON.stringify({ lang }),
    });
  }

  async backfillSkillMd(teamId: string): Promise<{ updated: number; message: string }> {
    return this.request('/team/skills/backfill-md', {
      method: 'POST',
      body: JSON.stringify({ teamId }),
    });
  }

  async getAiInsights(teamId: string, lang?: string): Promise<{ insights: Array<{ id: string; type: string; name: string; ai_description: string; ai_description_lang: string; ai_described_at: string }>; total: number }> {
    const query = new URLSearchParams();
    if (lang) query.set('lang', lang);
    return this.request(`/teams/${teamId}/ai-insights?${query}`);
  }

  // Smart Log methods
  async getSmartLogs(teamId: string, params?: SmartLogParams): Promise<SmartLogsResponse> {
    const query = new URLSearchParams();
    if (params?.resourceType) query.set('resourceType', params.resourceType);
    if (params?.action) query.set('action', params.action);
    if (params?.source) query.set('source', params.source);
    if (params?.userId) query.set('userId', params.userId);
    if (params?.page) query.set('page', String(params.page));
    if (params?.limit) query.set('limit', String(params.limit));
    return this.request(`/team/teams/${teamId}/smart-logs?${query}`);
  }

  // Team Settings methods
  async getTeamSettings(teamId: string): Promise<TeamSettingsResponse> {
    return this.request(`/team/teams/${teamId}/settings`);
  }

  async updateTeamSettings(teamId: string, data: UpdateTeamSettingsRequest): Promise<TeamSettingsResponse> {
    return this.request(`/team/teams/${teamId}/settings`, {
      method: 'PUT',
      body: JSON.stringify(data),
    });
  }
}

export const apiClient = new ApiClient();
