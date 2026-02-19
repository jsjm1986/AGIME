// Cloud Data Source Adapter
// Handles communication with remote cloud team servers

import type {
  DataSource,
  DataSourceAdapter,
  HealthStatus,
  ListResourcesParams,
  PaginatedResponse,
} from '../types';
import type {
  Team,
  TeamSummary,
  SharedSkill,
  SharedRecipe,
  SharedExtension,
} from '../../types';
import { authAdapter } from '../../auth/authAdapter';

const API_BASE = '/api/team';

export class CloudAdapter implements DataSourceAdapter {
  private source: DataSource;

  constructor(source: DataSource) {
    if (source.type !== 'cloud') {
      throw new Error('CloudAdapter requires a cloud data source');
    }
    this.source = source;
  }

  getSource(): DataSource {
    return this.source;
  }

  private getBaseUrl(): string {
    return this.source.connection.url.replace(/\/+$/, '');
  }

  private async fetchApi<T>(path: string, options?: RequestInit): Promise<T> {
    const authHeaders = await authAdapter.getAuthHeaders(this.source);
    const url = `${this.getBaseUrl()}${API_BASE}${path}`;

    const response = await fetch(url, {
      ...options,
      headers: {
        'Content-Type': 'application/json',
        ...authHeaders,
        ...options?.headers,
      },
    });

    if (!response.ok) {
      const error = await response.json().catch(() => ({ message: response.statusText }));
      throw new Error(error.message || `API Error: ${response.status}`);
    }

    return response.json();
  }

  async isAvailable(): Promise<boolean> {
    try {
      const response = await fetch(`${this.getBaseUrl()}/health`, {
        method: 'GET',
        signal: AbortSignal.timeout(5000),
      });
      const available = response.ok;
      this.source.status = available ? 'online' : 'offline';
      return available;
    } catch {
      this.source.status = 'offline';
      return false;
    }
  }

  async checkHealth(): Promise<HealthStatus> {
    const startTime = Date.now();
    try {
      const response = await fetch(`${this.getBaseUrl()}/health`, {
        method: 'GET',
        signal: AbortSignal.timeout(5000),
      });

      const latencyMs = Date.now() - startTime;

      if (response.ok) {
        const data = await response.json();
        const databaseHealthy =
          data.database_connected === true ||
          data.database === 'ok' ||
          data.database === 'mongodb' ||
          data.database === 'sqlite';
        this.source.status = 'online';
        return {
          sourceId: this.source.id,
          healthy: true,
          latencyMs,
          version: data.version,
          database: databaseHealthy ? 'ok' : 'error',
          checkedAt: new Date().toISOString(),
        };
      }

      this.source.status = 'error';
      return {
        sourceId: this.source.id,
        healthy: false,
        latencyMs,
        error: `HTTP ${response.status}`,
        checkedAt: new Date().toISOString(),
      };
    } catch (error) {
      this.source.status = 'error';
      return {
        sourceId: this.source.id,
        healthy: false,
        error: error instanceof Error ? error.message : 'Health check failed',
        checkedAt: new Date().toISOString(),
      };
    }
  }

  async listTeams(params?: ListResourcesParams): Promise<PaginatedResponse<Team>> {
    const query = new URLSearchParams();
    query.set('page', String(params?.page || 1));
    query.set('limit', String(params?.limit || 20));
    if (params?.search) query.set('search', params.search);

    const response = await this.fetchApi<{
      teams: Team[];
      total: number;
      page: number;
      limit: number;
    }>(`/teams?${query}`);

    this.source.teamsCount = response.total;
    this.source.lastSyncedAt = new Date().toISOString();

    return {
      items: response.teams,
      total: response.total,
      page: response.page,
      limit: response.limit,
    };
  }

  async getTeam(teamId: string): Promise<TeamSummary | null> {
    try {
      return await this.fetchApi<TeamSummary>(`/teams/${teamId}`);
    } catch {
      return null;
    }
  }

  async listSkills(params?: ListResourcesParams): Promise<PaginatedResponse<SharedSkill>> {
    const query = new URLSearchParams();
    query.set('page', String(params?.page || 1));
    query.set('limit', String(params?.limit || 20));
    if (params?.teamId) query.set('teamId', params.teamId);
    if (params?.search) query.set('search', params.search);
    if (params?.tags?.length) query.set('tags', params.tags.join(','));

    const response = await this.fetchApi<{
      skills: SharedSkill[];
      total: number;
      page: number;
      limit: number;
    }>(`/skills?${query}`);

    return {
      items: response.skills,
      total: response.total,
      page: response.page,
      limit: response.limit,
    };
  }

  async getSkill(skillId: string): Promise<SharedSkill | null> {
    try {
      return await this.fetchApi<SharedSkill>(`/skills/${skillId}`);
    } catch {
      return null;
    }
  }

  async listRecipes(params?: ListResourcesParams): Promise<PaginatedResponse<SharedRecipe>> {
    const query = new URLSearchParams();
    query.set('page', String(params?.page || 1));
    query.set('limit', String(params?.limit || 20));
    if (params?.teamId) query.set('teamId', params.teamId);
    if (params?.search) query.set('search', params.search);
    if (params?.tags?.length) query.set('tags', params.tags.join(','));

    const response = await this.fetchApi<{
      recipes: SharedRecipe[];
      total: number;
      page: number;
      limit: number;
    }>(`/recipes?${query}`);

    return {
      items: response.recipes,
      total: response.total,
      page: response.page,
      limit: response.limit,
    };
  }

  async getRecipe(recipeId: string): Promise<SharedRecipe | null> {
    try {
      return await this.fetchApi<SharedRecipe>(`/recipes/${recipeId}`);
    } catch {
      return null;
    }
  }

  async listExtensions(params?: ListResourcesParams): Promise<PaginatedResponse<SharedExtension>> {
    const query = new URLSearchParams();
    query.set('page', String(params?.page || 1));
    query.set('limit', String(params?.limit || 20));
    if (params?.teamId) query.set('teamId', params.teamId);
    if (params?.search) query.set('search', params.search);
    if (params?.tags?.length) query.set('tags', params.tags.join(','));

    const response = await this.fetchApi<{
      extensions: SharedExtension[];
      total: number;
      page: number;
      limit: number;
    }>(`/extensions?${query}`);

    return {
      items: response.extensions,
      total: response.total,
      page: response.page,
      limit: response.limit,
    };
  }

  async getExtension(extensionId: string): Promise<SharedExtension | null> {
    try {
      return await this.fetchApi<SharedExtension>(`/extensions/${extensionId}`);
    } catch {
      return null;
    }
  }
}

/** Create a cloud adapter from source configuration */
export function createCloudAdapter(source: DataSource): CloudAdapter {
  return new CloudAdapter(source);
}
