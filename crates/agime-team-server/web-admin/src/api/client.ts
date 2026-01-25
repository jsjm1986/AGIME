const API_BASE = '/api';

import type {
  TeamsResponse,
  TeamResponse,
  TeamSummaryResponse,
  MembersResponse,
  TeamMember,
  SkillsResponse,
  SharedSkill,
  RecipesResponse,
  SharedRecipe,
  ExtensionsResponse,
  SharedExtension,
  InvitesResponse,
  CreateInviteResponse,
  TeamRole,
} from './types';

export interface User {
  id: string;
  email: string;
  display_name: string;
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
  private async request<T>(path: string, options?: RequestInit): Promise<T> {
    const res = await fetch(`${API_BASE}${path}`, {
      ...options,
      credentials: 'include',
      headers: {
        'Content-Type': 'application/json',
        ...options?.headers,
      },
    });

    if (!res.ok) {
      const error = await res.json().catch(() => ({ error: 'Request failed' }));
      throw new Error(error.error || 'Request failed');
    }

    return res.json();
  }

  async register(email: string, displayName: string): Promise<RegisterResponse> {
    return this.request('/auth/register', {
      method: 'POST',
      body: JSON.stringify({ email, display_name: displayName }),
    });
  }

  async login(apiKey: string): Promise<SessionResponse> {
    return this.request('/auth/login', {
      method: 'POST',
      body: JSON.stringify({ api_key: apiKey }),
    });
  }

  async logout(): Promise<void> {
    await this.request('/auth/logout', { method: 'POST' });
  }

  async getSession(): Promise<SessionResponse> {
    return this.request('/auth/session');
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
  async getSkills(teamId: string): Promise<SkillsResponse> {
    return this.request(`/team/skills?teamId=${teamId}`);
  }

  async getSkill(skillId: string): Promise<{ skill: SharedSkill }> {
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
  async getRecipes(teamId: string): Promise<RecipesResponse> {
    return this.request(`/team/recipes?teamId=${teamId}`);
  }

  async getRecipe(recipeId: string): Promise<{ recipe: SharedRecipe }> {
    return this.request(`/team/recipes/${recipeId}`);
  }

  async updateRecipe(recipeId: string, data: { name?: string; description?: string; content?: string }): Promise<{ recipe: SharedRecipe }> {
    return this.request(`/team/recipes/${recipeId}`, {
      method: 'PUT',
      body: JSON.stringify(data),
    });
  }

  async deleteRecipe(recipeId: string): Promise<void> {
    await this.request(`/team/recipes/${recipeId}`, { method: 'DELETE' });
  }

  // Extension methods
  async getExtensions(teamId: string): Promise<ExtensionsResponse> {
    return this.request(`/team/extensions?teamId=${teamId}`);
  }

  async getExtension(extensionId: string): Promise<{ extension: SharedExtension }> {
    return this.request(`/team/extensions/${extensionId}`);
  }

  async updateExtension(extensionId: string, data: { name?: string; description?: string; config?: string }): Promise<{ extension: SharedExtension }> {
    return this.request(`/team/extensions/${extensionId}`, {
      method: 'PUT',
      body: JSON.stringify(data),
    });
  }

  async deleteExtension(extensionId: string): Promise<void> {
    await this.request(`/team/extensions/${extensionId}`, { method: 'DELETE' });
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
}

export const apiClient = new ApiClient();
