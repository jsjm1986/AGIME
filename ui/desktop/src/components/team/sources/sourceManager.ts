// Source Manager
// Central manager for all data sources with aggregation capabilities

import type {
  DataSource,
  DataSourceType,
  DataSourceStatus,
  DataSourceAdapter,
  HealthStatus,
  SourcedResource,
  AggregatedQueryResult,
  ResourceFilters,
  ListResourcesParams,
} from './types';
import type {
  Team,
  SharedSkill,
  SharedRecipe,
  SharedExtension,
} from '../types';
import { localAdapter, createCloudAdapter, createLANAdapter } from './adapters';
import { removeCredential, storeCredential } from '../auth/authAdapter';
import { resourceCache } from './resourceCache';

// Old storage keys for migration
const OLD_STORAGE_KEYS = {
  CLOUD_SERVERS: 'AGIME_TEAM_CLOUD_SERVERS',
  LAN_CONNECTIONS: 'AGIME_TEAM_LAN_CONNECTIONS',
  CONNECTION_MODE: 'AGIME_TEAM_CONNECTION_MODE',
  CLOUD_SERVER_URL: 'AGIME_TEAM_SERVER_URL',
  CLOUD_API_KEY: 'AGIME_TEAM_API_KEY',
};

// Storage keys
const STORAGE_KEYS = {
  SOURCES: 'AGIME_TEAM_DATA_SOURCES',
  ACTIVE_SOURCE: 'AGIME_TEAM_ACTIVE_SOURCE',
  MIGRATION_STATUS: 'AGIME_TEAM_STORAGE_MIGRATED',
};

// Event types for source changes
export type SourceChangeEvent = 'active-changed' | 'source-added' | 'source-removed' | 'status-changed';
export type SourceChangeListener = (event: SourceChangeEvent, sourceId: string) => void;

// ============================================================
// Source Manager Class
// ============================================================

export class SourceManager {
  private sources: Map<string, DataSource> = new Map();
  private adapters: Map<string, DataSourceAdapter> = new Map();
  private healthCache: Map<string, HealthStatus> = new Map();
  private initialized = false;
  private activeSourceId: string = 'local';
  private listeners: Set<SourceChangeListener> = new Set();

  constructor() {
    // Local source is always available
    this.registerLocalSource();
  }

  private registerLocalSource(): void {
    const localSource = localAdapter.getSource();
    this.sources.set(localSource.id, localSource);
    this.adapters.set(localSource.id, localAdapter);
  }

  // ============================================================
  // Initialization
  // ============================================================

  async initialize(): Promise<void> {
    if (this.initialized) return;

    // Migrate from old storage format first
    this.migrateFromOldStorage();

    // Load saved sources from storage
    this.loadFromStorage();

    // Load active source from storage
    this.loadActiveSource();

    // Check local source health
    await this.checkHealth('local');

    // Check health of all remote sources in background
    this.checkAllHealth().catch(console.error);

    this.initialized = true;
  }

  private loadActiveSource(): void {
    try {
      const activeId = localStorage.getItem(STORAGE_KEYS.ACTIVE_SOURCE);
      if (activeId && this.sources.has(activeId)) {
        this.activeSourceId = activeId;
      } else {
        this.activeSourceId = 'local';
      }
    } catch {
      this.activeSourceId = 'local';
    }
  }

  private loadFromStorage(): void {
    try {
      const stored = localStorage.getItem(STORAGE_KEYS.SOURCES);
      if (!stored) return;

      const sources: DataSource[] = JSON.parse(stored);
      for (const source of sources) {
        if (source.id === 'local') continue; // Skip local, already registered
        this.registerSource(source);
      }
    } catch (error) {
      console.error('Failed to load sources from storage:', error);
    }
  }

  private saveToStorage(): void {
    try {
      const sources = Array.from(this.sources.values());
      localStorage.setItem(STORAGE_KEYS.SOURCES, JSON.stringify(sources));
    } catch (error) {
      console.error('Failed to save sources to storage:', error);
    }
  }

  // ============================================================
  // Source Registration
  // ============================================================

  registerSource(source: DataSource): void {
    this.sources.set(source.id, source);

    // Create appropriate adapter
    let adapter: DataSourceAdapter;
    switch (source.type) {
      case 'cloud':
        adapter = createCloudAdapter(source);
        break;
      case 'lan':
        adapter = createLANAdapter(source);
        break;
      default:
        return; // Local is already registered
    }

    this.adapters.set(source.id, adapter);
    this.saveToStorage();
    this.notifyListeners('source-added', source.id);
  }

  unregisterSource(sourceId: string): boolean {
    if (sourceId === 'local') return false; // Cannot remove local

    const source = this.sources.get(sourceId);
    if (!source) return false;

    // If removing the active source, reset to local
    if (this.activeSourceId === sourceId) {
      this.resetToLocal();
    }

    // Remove credential
    removeCredential(source.connection.credentialRef);

    this.sources.delete(sourceId);
    this.adapters.delete(sourceId);
    this.healthCache.delete(sourceId);
    this.saveToStorage();
    this.notifyListeners('source-removed', sourceId);

    return true;
  }

  /**
   * Update an existing data source
   */
  updateSource(sourceId: string, updates: Partial<Omit<DataSource, 'id' | 'type' | 'createdAt'>>): DataSource | null {
    if (sourceId === 'local') return null; // Cannot update local

    const source = this.sources.get(sourceId);
    if (!source) return null;

    // Apply updates
    const updatedSource: DataSource = {
      ...source,
      ...updates,
      connection: updates.connection
        ? { ...source.connection, ...updates.connection }
        : source.connection,
      capabilities: updates.capabilities
        ? { ...source.capabilities, ...updates.capabilities }
        : source.capabilities,
      userInfo: updates.userInfo
        ? { ...source.userInfo, ...updates.userInfo }
        : source.userInfo,
    };

    this.sources.set(sourceId, updatedSource);
    this.saveToStorage();

    return updatedSource;
  }

  /**
   * Update data source status
   */
  updateSourceStatus(sourceId: string, status: DataSourceStatus, error?: string): void {
    const source = this.sources.get(sourceId);
    if (!source || sourceId === 'local') return;

    source.status = status;
    source.lastError = error;
    if (status === 'online') {
      source.lastSyncedAt = new Date().toISOString();
    }

    this.sources.set(sourceId, source);
    this.saveToStorage();
  }

  // ============================================================
  // Source Access
  // ============================================================

  getSource(sourceId: string): DataSource | undefined {
    return this.sources.get(sourceId);
  }

  getAllSources(): DataSource[] {
    return Array.from(this.sources.values());
  }

  getActiveSources(): DataSource[] {
    return Array.from(this.sources.values()).filter(
      s => s.status === 'online' || s.status === 'connecting'
    );
  }

  getSourcesByType(type: DataSourceType): DataSource[] {
    return Array.from(this.sources.values()).filter(s => s.type === type);
  }

  getAdapter(sourceId: string): DataSourceAdapter | undefined {
    return this.adapters.get(sourceId);
  }

  // ============================================================
  // Active Source Management
  // ============================================================

  /**
   * Get the currently active data source ID
   */
  getActiveSourceId(): string {
    return this.activeSourceId;
  }

  /**
   * Get the currently active data source
   */
  getActiveSource(): DataSource | undefined {
    return this.sources.get(this.activeSourceId);
  }

  /**
   * Get the adapter for the currently active data source
   */
  getActiveAdapter(): DataSourceAdapter | undefined {
    return this.adapters.get(this.activeSourceId);
  }

  /**
   * Set the active data source
   */
  setActiveSource(sourceId: string): boolean {
    if (!this.sources.has(sourceId)) {
      return false;
    }

    const previousId = this.activeSourceId;
    this.activeSourceId = sourceId;

    try {
      localStorage.setItem(STORAGE_KEYS.ACTIVE_SOURCE, sourceId);
    } catch {
      // Ignore storage errors
    }

    if (previousId !== sourceId) {
      this.notifyListeners('active-changed', sourceId);
    }

    return true;
  }

  /**
   * Reset to local source
   */
  resetToLocal(): void {
    this.setActiveSource('local');
  }

  /**
   * Check if the active source is remote (cloud or LAN)
   */
  isActiveSourceRemote(): boolean {
    const source = this.getActiveSource();
    return source?.type === 'cloud' || source?.type === 'lan';
  }

  // ============================================================
  // Event Listeners
  // ============================================================

  /**
   * Add a listener for source changes
   */
  addListener(listener: SourceChangeListener): void {
    this.listeners.add(listener);
  }

  /**
   * Remove a listener
   */
  removeListener(listener: SourceChangeListener): void {
    this.listeners.delete(listener);
  }

  private notifyListeners(event: SourceChangeEvent, sourceId: string): void {
    this.listeners.forEach(listener => {
      try {
        listener(event, sourceId);
      } catch (error) {
        console.error('Source change listener error:', error);
      }
    });
  }

  // ============================================================
  // Health Check
  // ============================================================

  async checkHealth(sourceId: string): Promise<HealthStatus> {
    const adapter = this.adapters.get(sourceId);
    if (!adapter) {
      return {
        sourceId,
        healthy: false,
        error: 'Source not found',
        checkedAt: new Date().toISOString(),
      };
    }

    const health = await adapter.checkHealth();
    this.healthCache.set(sourceId, health);

    // Update source status
    const source = this.sources.get(sourceId);
    if (source) {
      source.status = health.healthy ? 'online' : 'error';
      source.lastError = health.error;
      this.saveToStorage();
    }

    return health;
  }

  async checkAllHealth(): Promise<Map<string, HealthStatus>> {
    const results = new Map<string, HealthStatus>();
    const promises = Array.from(this.sources.keys()).map(async sourceId => {
      const health = await this.checkHealth(sourceId);
      results.set(sourceId, health);
    });

    await Promise.all(promises);
    return results;
  }

  getCachedHealth(sourceId: string): HealthStatus | undefined {
    return this.healthCache.get(sourceId);
  }

  // ============================================================
  // Aggregated Queries
  // ============================================================

  async aggregateTeams(
    filters: ResourceFilters,
    params?: ListResourcesParams,
    options?: { forceRefresh?: boolean }
  ): Promise<AggregatedQueryResult<Team>> {
    const fetchFn = () => this._fetchTeams(filters, params);
    return resourceCache.getOrFetch('teams', filters, fetchFn, params, options);
  }

  private async _fetchTeams(
    filters: ResourceFilters,
    params?: ListResourcesParams
  ): Promise<AggregatedQueryResult<Team>> {
    const sourceIds = filters.sources === 'all'
      ? Array.from(this.sources.keys())
      : filters.sources;

    const results: SourcedResource<Team>[] = [];
    const countBySource: Record<string, number> = {};
    const errors: Array<{ sourceId: string; sourceName: string; error: string }> = [];

    await Promise.all(
      sourceIds.map(async sourceId => {
        const adapter = this.adapters.get(sourceId);
        const source = this.sources.get(sourceId);
        if (!adapter || !source) return;

        try {
          const response = await adapter.listTeams({
            ...params,
            search: filters.search,
          });

          countBySource[sourceId] = response.total;

          for (const team of response.items) {
            results.push({
              source,
              resource: team,
              syncStatus: source.type === 'local' ? 'synced' : 'remote-only',
            });
          }
        } catch (error) {
          errors.push({
            sourceId,
            sourceName: source.name,
            error: error instanceof Error ? error.message : 'Query failed',
          });
        }
      })
    );

    return {
      resources: results,
      totalCount: results.length,
      countBySource,
      errors,
    };
  }

  async aggregateSkills(
    filters: ResourceFilters,
    params?: ListResourcesParams,
    options?: { forceRefresh?: boolean }
  ): Promise<AggregatedQueryResult<SharedSkill>> {
    // Try cache first
    const fetchFn = () => this._fetchSkills(filters, params);
    return resourceCache.getOrFetch('skills', filters, fetchFn, params, options);
  }

  private async _fetchSkills(
    filters: ResourceFilters,
    params?: ListResourcesParams
  ): Promise<AggregatedQueryResult<SharedSkill>> {
    const sourceIds = filters.sources === 'all'
      ? Array.from(this.sources.keys())
      : filters.sources;

    const results: SourcedResource<SharedSkill>[] = [];
    const countBySource: Record<string, number> = {};
    const errors: Array<{ sourceId: string; sourceName: string; error: string }> = [];

    await Promise.all(
      sourceIds.map(async sourceId => {
        const adapter = this.adapters.get(sourceId);
        const source = this.sources.get(sourceId);
        if (!adapter || !source) return;

        try {
          const response = await adapter.listSkills({
            ...params,
            teamId: filters.teamId,
            search: filters.search,
            tags: filters.tags,
          });

          countBySource[sourceId] = response.total;

          for (const skill of response.items) {
            results.push({
              source,
              resource: skill,
              syncStatus: source.type === 'local' ? 'synced' : 'remote-only',
            });
          }
        } catch (error) {
          errors.push({
            sourceId,
            sourceName: source.name,
            error: error instanceof Error ? error.message : 'Query failed',
          });
        }
      })
    );

    return {
      resources: results,
      totalCount: results.length,
      countBySource,
      errors,
    };
  }

  async aggregateRecipes(
    filters: ResourceFilters,
    params?: ListResourcesParams,
    options?: { forceRefresh?: boolean }
  ): Promise<AggregatedQueryResult<SharedRecipe>> {
    const fetchFn = () => this._fetchRecipes(filters, params);
    return resourceCache.getOrFetch('recipes', filters, fetchFn, params, options);
  }

  private async _fetchRecipes(
    filters: ResourceFilters,
    params?: ListResourcesParams
  ): Promise<AggregatedQueryResult<SharedRecipe>> {
    const sourceIds = filters.sources === 'all'
      ? Array.from(this.sources.keys())
      : filters.sources;

    const results: SourcedResource<SharedRecipe>[] = [];
    const countBySource: Record<string, number> = {};
    const errors: Array<{ sourceId: string; sourceName: string; error: string }> = [];

    await Promise.all(
      sourceIds.map(async sourceId => {
        const adapter = this.adapters.get(sourceId);
        const source = this.sources.get(sourceId);
        if (!adapter || !source) return;

        try {
          const response = await adapter.listRecipes({
            ...params,
            teamId: filters.teamId,
            search: filters.search,
            tags: filters.tags,
          });

          countBySource[sourceId] = response.total;

          for (const recipe of response.items) {
            results.push({
              source,
              resource: recipe,
              syncStatus: source.type === 'local' ? 'synced' : 'remote-only',
            });
          }
        } catch (error) {
          errors.push({
            sourceId,
            sourceName: source.name,
            error: error instanceof Error ? error.message : 'Query failed',
          });
        }
      })
    );

    return {
      resources: results,
      totalCount: results.length,
      countBySource,
      errors,
    };
  }

  async aggregateExtensions(
    filters: ResourceFilters,
    params?: ListResourcesParams,
    options?: { forceRefresh?: boolean }
  ): Promise<AggregatedQueryResult<SharedExtension>> {
    const fetchFn = () => this._fetchExtensions(filters, params);
    return resourceCache.getOrFetch('extensions', filters, fetchFn, params, options);
  }

  private async _fetchExtensions(
    filters: ResourceFilters,
    params?: ListResourcesParams
  ): Promise<AggregatedQueryResult<SharedExtension>> {
    const sourceIds = filters.sources === 'all'
      ? Array.from(this.sources.keys())
      : filters.sources;

    const results: SourcedResource<SharedExtension>[] = [];
    const countBySource: Record<string, number> = {};
    const errors: Array<{ sourceId: string; sourceName: string; error: string }> = [];

    await Promise.all(
      sourceIds.map(async sourceId => {
        const adapter = this.adapters.get(sourceId);
        const source = this.sources.get(sourceId);
        if (!adapter || !source) return;

        try {
          const response = await adapter.listExtensions({
            ...params,
            teamId: filters.teamId,
            search: filters.search,
            tags: filters.tags,
          });

          countBySource[sourceId] = response.total;

          for (const extension of response.items) {
            results.push({
              source,
              resource: extension,
              syncStatus: source.type === 'local' ? 'synced' : 'remote-only',
            });
          }
        } catch (error) {
          errors.push({
            sourceId,
            sourceName: source.name,
            error: error instanceof Error ? error.message : 'Query failed',
          });
        }
      })
    );

    return {
      resources: results,
      totalCount: results.length,
      countBySource,
      errors,
    };
  }

  // ============================================================
  // Migration from Old Storage
  // ============================================================

  /**
   * Migrate from old storage format (serverStore/lanStore) to unified format
   * Call this on app startup
   */
  migrateFromOldStorage(): void {
    // Check if already migrated
    if (localStorage.getItem(STORAGE_KEYS.MIGRATION_STATUS)) {
      return;
    }

    try {
      let migrated = false;

      // Migrate cloud servers
      migrated = this.migrateCloudServers() || migrated;

      // Migrate LAN connections
      migrated = this.migrateLANConnections() || migrated;

      if (migrated) {
        // Mark migration complete
        localStorage.setItem(STORAGE_KEYS.MIGRATION_STATUS, 'v2');
        console.log('Migration to unified storage completed');
      }
    } catch (error) {
      console.error('Migration failed:', error);
    }
  }

  private migrateCloudServers(): boolean {
    const cloudServersJson = localStorage.getItem(OLD_STORAGE_KEYS.CLOUD_SERVERS);
    if (!cloudServersJson) return false;

    try {
      const servers = JSON.parse(cloudServersJson);
      if (!Array.isArray(servers) || servers.length === 0) return false;

      for (const server of servers) {
        const sourceId = `cloud-${server.id}`;

        // Skip if already exists
        if (this.sources.has(sourceId)) continue;

        const dataSource: DataSource = {
          id: sourceId,
          type: 'cloud',
          name: server.name || 'Cloud Server',
          status: server.status === 'online' ? 'online' : 'offline',
          connection: {
            url: server.url,
            authType: 'api-key',
            credentialRef: sourceId,
          },
          capabilities: {
            canCreate: true,
            canSync: true,
            supportsOffline: false,
            canManageTeams: true,
            canInviteMembers: true,
          },
          teamsCount: server.teamsCount,
          lastSyncedAt: server.lastSyncedAt,
          lastError: server.lastError,
          userInfo: server.userEmail ? {
            email: server.userEmail,
            displayName: server.displayName,
            userId: server.userId,
          } : undefined,
          createdAt: server.createdAt || new Date().toISOString(),
        };

        // Store credential
        if (server.apiKey) {
          storeCredential(sourceId, server.apiKey);
        }

        this.registerSource(dataSource);
      }

      console.log(`Migrated ${servers.length} cloud servers`);
      return true;
    } catch (error) {
      console.error('Failed to migrate cloud servers:', error);
      return false;
    }
  }

  private migrateLANConnections(): boolean {
    const connectionsJson = localStorage.getItem(OLD_STORAGE_KEYS.LAN_CONNECTIONS);
    if (!connectionsJson) return false;

    try {
      const connections = JSON.parse(connectionsJson);
      if (!Array.isArray(connections) || connections.length === 0) return false;

      for (const conn of connections) {
        const sourceId = `lan-${conn.id}`;

        // Skip if already exists
        if (this.sources.has(sourceId)) continue;

        const url = `http://${conn.host}:${conn.port}`;

        const dataSource: DataSource = {
          id: sourceId,
          type: 'lan',
          name: conn.name || `LAN Device (${conn.host})`,
          status: conn.status === 'connected' ? 'online' : 'offline',
          connection: {
            url,
            authType: 'secret-key',
            credentialRef: sourceId,
          },
          capabilities: {
            canCreate: false,
            canSync: true,
            supportsOffline: false,
            canManageTeams: false,
            canInviteMembers: false,
          },
          teamsCount: conn.teamsCount,
          lastSyncedAt: conn.lastOnline,
          lastError: conn.lastError,
          createdAt: conn.createdAt || new Date().toISOString(),
        };

        // Store credential
        if (conn.secretKey) {
          storeCredential(sourceId, conn.secretKey);
        }

        this.registerSource(dataSource);
      }

      console.log(`Migrated ${connections.length} LAN connections`);
      return true;
    } catch (error) {
      console.error('Failed to migrate LAN connections:', error);
      return false;
    }
  }

  // ============================================================
  // Cache Management
  // ============================================================

  /**
   * Invalidate cache for a specific resource type
   */
  invalidateCache(resourceType: 'teams' | 'skills' | 'recipes' | 'extensions'): void {
    resourceCache.invalidate(resourceType);
  }

  /**
   * Invalidate all caches
   */
  invalidateAllCaches(): void {
    resourceCache.invalidateAll();
  }

  /**
   * Invalidate cache for a specific team
   */
  invalidateTeamCache(teamId: string): void {
    resourceCache.invalidateTeam(teamId);
  }
}

// Singleton instance
export const sourceManager = new SourceManager();

// React hook for using source manager
export function useSourceManager(): SourceManager {
  return sourceManager;
}