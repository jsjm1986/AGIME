// Data Source Types for Unified Team Architecture
// Supports local, cloud, and LAN data sources

import type {
  Team,
  TeamSummary,
  SharedSkill,
  SharedRecipe,
  SharedExtension,
  InstalledResource,
} from '../types';

// ============================================================
// Data Source Core Types
// ============================================================

/** Data source type */
export type DataSourceType = 'local' | 'cloud' | 'lan';

/** Data source connection status */
export type DataSourceStatus = 'online' | 'offline' | 'connecting' | 'error';

/** Authentication type for data sources */
export type AuthType = 'secret-key' | 'api-key';

/** Data source connection configuration */
export interface DataSourceConnection {
  /** Server URL */
  url: string;
  /** Authentication type */
  authType: AuthType;
  /** Credential reference (actual value stored securely) */
  credentialRef: string;
}

/** Data source capabilities */
export interface DataSourceCapabilities {
  /** Can create new resources */
  canCreate: boolean;
  /** Supports synchronization */
  canSync: boolean;
  /** Supports offline access */
  supportsOffline: boolean;
  /** Can manage teams */
  canManageTeams: boolean;
  /** Can invite members */
  canInviteMembers: boolean;
}

/** Unified data source interface */
export interface DataSource {
  /** Unique identifier */
  id: string;
  /** Data source type */
  type: DataSourceType;
  /** Display name */
  name: string;
  /** Connection status */
  status: DataSourceStatus;
  /** Connection configuration */
  connection: DataSourceConnection;
  /** Source capabilities */
  capabilities: DataSourceCapabilities;
  /** Number of teams available */
  teamsCount?: number;
  /** Last successful sync time (ISO 8601) */
  lastSyncedAt?: string;
  /** Last error message */
  lastError?: string;
  /** User info on this source */
  userInfo?: {
    userId?: string;
    email?: string;
    displayName?: string;
  };
  /** Creation time (ISO 8601) */
  createdAt: string;
}

// ============================================================
// Sourced Resource Types
// ============================================================

/** Sync status for resources */
export type SyncStatus = 'synced' | 'local-only' | 'remote-only' | 'conflict' | 'pending';

/** Resource with source information */
export interface SourcedResource<T> {
  /** The data source this resource comes from */
  source: DataSource;
  /** The actual resource data */
  resource: T;
  /** Sync status of this resource */
  syncStatus?: SyncStatus;
  /** Whether this resource is cached locally */
  isCached?: boolean;
  /** Cache expiration time (ISO 8601) */
  cacheExpiresAt?: string;
}

/** Sourced team type */
export type SourcedTeam = SourcedResource<Team>;

/** Sourced team summary type */
export type SourcedTeamSummary = SourcedResource<TeamSummary>;

/** Sourced skill type */
export type SourcedSkill = SourcedResource<SharedSkill>;

/** Sourced recipe type */
export type SourcedRecipe = SourcedResource<SharedRecipe>;

/** Sourced extension type */
export type SourcedExtension = SourcedResource<SharedExtension>;

// ============================================================
// Aggregated Query Types
// ============================================================

/** Resource type for queries */
export type ResourceType = 'skill' | 'recipe' | 'extension';

/** Filter options for resource queries */
export interface ResourceFilters {
  /** Filter by specific sources (source IDs), 'all' for all sources */
  sources: string[] | 'all';
  /** Search query */
  search?: string;
  /** Filter by tags */
  tags?: string[];
  /** Filter by team ID */
  teamId?: string;
  /** Filter by resource type */
  resourceType?: ResourceType;
  /** Only show installed resources */
  installedOnly?: boolean;
  /** Only show resources with updates */
  updatesOnly?: boolean;
}

/** Aggregated query result */
export interface AggregatedQueryResult<T> {
  /** Resources from all sources */
  resources: SourcedResource<T>[];
  /** Total count across all sources */
  totalCount: number;
  /** Count per source */
  countBySource: Record<string, number>;
  /** Any errors that occurred during query */
  errors: Array<{
    sourceId: string;
    sourceName: string;
    error: string;
  }>;
}

// ============================================================
// Health Check Types
// ============================================================

/** Health status for a data source */
export interface HealthStatus {
  /** Source ID */
  sourceId: string;
  /** Whether the source is healthy */
  healthy: boolean;
  /** Response latency in milliseconds */
  latencyMs?: number;
  /** Server version (if available) */
  version?: string;
  /** Database status */
  database?: 'ok' | 'error';
  /** Error message if unhealthy */
  error?: string;
  /** Last check time (ISO 8601) */
  checkedAt: string;
}

// ============================================================
// Sync Types
// ============================================================

/** Conflict resolution strategy */
export type ConflictStrategy =
  | 'keep-remote'      // Remote wins (default)
  | 'keep-local'       // Local wins
  | 'keep-both'        // Create new version for both
  | 'auto-timestamp'   // Auto-select by timestamp
  | 'manual';          // Manual resolution required

/** Sync result for a single resource */
export interface SyncResourceResult {
  resourceId: string;
  resourceType: ResourceType;
  action: 'created' | 'updated' | 'deleted' | 'conflict' | 'skipped';
  conflictStrategy?: ConflictStrategy;
  error?: string;
}

/** Overall sync result */
export interface SyncResult {
  sourceId: string;
  success: boolean;
  syncedAt: string;
  results: SyncResourceResult[];
  errors: string[];
  stats: {
    created: number;
    updated: number;
    deleted: number;
    conflicts: number;
    skipped: number;
  };
}

// ============================================================
// Cache Types
// ============================================================

/** Cached resource entry */
export interface CachedResource {
  /** Cache entry ID */
  id: string;
  /** Source ID this resource came from */
  sourceId: string;
  /** Source type */
  sourceType: DataSourceType;
  /** Resource type */
  resourceType: ResourceType;
  /** Original resource ID */
  resourceId: string;
  /** Cached content as JSON */
  contentJson: string;
  /** When this was cached (ISO 8601) */
  cachedAt: string;
  /** When this cache expires (ISO 8601) */
  expiresAt?: string;
  /** Sync status */
  syncStatus: SyncStatus;
}

// ============================================================
// Data Source Adapter Interface
// ============================================================

/** Query parameters for listing resources */
export interface ListResourcesParams {
  teamId?: string;
  search?: string;
  tags?: string[];
  page?: number;
  limit?: number;
}

/** Paginated response */
export interface PaginatedResponse<T> {
  items: T[];
  total: number;
  page: number;
  limit: number;
}

/** Data source adapter interface - implemented by each adapter */
export interface DataSourceAdapter {
  /** Get the data source info */
  getSource(): DataSource;

  /** Check if the source is available */
  isAvailable(): Promise<boolean>;

  /** Check health status */
  checkHealth(): Promise<HealthStatus>;

  /** List teams */
  listTeams(params?: ListResourcesParams): Promise<PaginatedResponse<Team>>;

  /** Get team details */
  getTeam(teamId: string): Promise<TeamSummary | null>;

  /** List skills */
  listSkills(params?: ListResourcesParams): Promise<PaginatedResponse<SharedSkill>>;

  /** Get skill details */
  getSkill(skillId: string): Promise<SharedSkill | null>;

  /** List recipes */
  listRecipes(params?: ListResourcesParams): Promise<PaginatedResponse<SharedRecipe>>;

  /** Get recipe details */
  getRecipe(recipeId: string): Promise<SharedRecipe | null>;

  /** List extensions */
  listExtensions(params?: ListResourcesParams): Promise<PaginatedResponse<SharedExtension>>;

  /** Get extension details */
  getExtension(extensionId: string): Promise<SharedExtension | null>;

  /** List installed resources (local only) */
  listInstalled?(): Promise<InstalledResource[]>;
}

// ============================================================
// Storage Migration Types
// ============================================================

/** Migration status */
export interface MigrationStatus {
  /** Whether migration has been completed */
  migrated: boolean;
  /** Migration version */
  version: string;
  /** Migration timestamp */
  migratedAt?: string;
}

/** Storage keys for data sources */
export const DATA_SOURCE_STORAGE_KEYS = {
  /** All data sources */
  SOURCES: 'AGIME_TEAM_DATA_SOURCES',
  /** Active source ID */
  ACTIVE_SOURCE: 'AGIME_TEAM_ACTIVE_SOURCE',
  /** Migration status */
  MIGRATION_STATUS: 'AGIME_TEAM_STORAGE_MIGRATED',
} as const;
