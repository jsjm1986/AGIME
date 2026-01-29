// Resource Cache Manager
// Provides caching for API responses to reduce server load and improve UX

import type {
  ResourceFilters,
  ListResourcesParams,
} from './types';

// Cache entry with metadata
interface CacheEntry<T> {
  data: T;
  timestamp: number;
  expiresAt: number;
}

// Cache configuration
interface CacheConfig {
  // Default TTL in milliseconds (5 minutes)
  defaultTTL: number;
  // Maximum cache entries per resource type
  maxEntries: number;
  // Enable background refresh
  backgroundRefresh: boolean;
}

const DEFAULT_CONFIG: CacheConfig = {
  defaultTTL: 5 * 60 * 1000, // 5 minutes
  maxEntries: 50,
  backgroundRefresh: true,
};

// Generate cache key from filters and params
function generateCacheKey(
  resourceType: string,
  filters: ResourceFilters,
  params?: ListResourcesParams
): string {
  const filterKey = JSON.stringify({
    sources: filters.sources,
    search: filters.search || '',
    tags: filters.tags || [],
    teamId: filters.teamId || '',
  });
  const paramKey = params ? JSON.stringify(params) : '';
  return `${resourceType}:${filterKey}:${paramKey}`;
}

// Resource Cache Class
export class ResourceCache {
  private cache: Map<string, CacheEntry<unknown>> = new Map();
  private config: CacheConfig;
  private pendingRequests: Map<string, Promise<unknown>> = new Map();

  constructor(config: Partial<CacheConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  // Get cached data if valid
  get<T>(
    resourceType: string,
    filters: ResourceFilters,
    params?: ListResourcesParams
  ): T | null {
    const key = generateCacheKey(resourceType, filters, params);
    const entry = this.cache.get(key) as CacheEntry<T> | undefined;

    if (!entry) return null;

    // Check if expired
    if (Date.now() > entry.expiresAt) {
      this.cache.delete(key);
      return null;
    }

    return entry.data;
  }

  // Set cache data
  set<T>(
    resourceType: string,
    filters: ResourceFilters,
    data: T,
    params?: ListResourcesParams,
    ttl?: number
  ): void {
    const key = generateCacheKey(resourceType, filters, params);
    const now = Date.now();

    const entry: CacheEntry<T> = {
      data,
      timestamp: now,
      expiresAt: now + (ttl || this.config.defaultTTL),
    };

    this.cache.set(key, entry);

    // Enforce max entries limit
    this.enforceMaxEntries(resourceType);
  }

  // Get or fetch with caching
  async getOrFetch<T>(
    resourceType: string,
    filters: ResourceFilters,
    fetchFn: () => Promise<T>,
    params?: ListResourcesParams,
    options?: { forceRefresh?: boolean; ttl?: number }
  ): Promise<T> {
    const key = generateCacheKey(resourceType, filters, params);

    // Return cached data if not forcing refresh
    if (!options?.forceRefresh) {
      const cached = this.get<T>(resourceType, filters, params);
      if (cached !== null) {
        // Optionally trigger background refresh if data is stale
        if (this.config.backgroundRefresh && this.isStale(key)) {
          this.backgroundRefresh(key, fetchFn);
        }
        return cached;
      }
    }

    // Check for pending request to avoid duplicate calls
    const pending = this.pendingRequests.get(key);
    if (pending) {
      return pending as Promise<T>;
    }

    // Fetch and cache
    const fetchPromise = fetchFn()
      .then(data => {
        this.set(resourceType, filters, data, params, options?.ttl);
        this.pendingRequests.delete(key);
        return data;
      })
      .catch(error => {
        this.pendingRequests.delete(key);
        throw error;
      });

    this.pendingRequests.set(key, fetchPromise);
    return fetchPromise;
  }

  // Check if cache entry is stale (past 80% of TTL)
  private isStale(key: string): boolean {
    const entry = this.cache.get(key);
    if (!entry) return true;

    const age = Date.now() - entry.timestamp;
    const ttl = entry.expiresAt - entry.timestamp;
    return age > ttl * 0.8;
  }

  // Background refresh without blocking
  private backgroundRefresh<T>(key: string, fetchFn: () => Promise<T>): void {
    // Don't refresh if already pending
    if (this.pendingRequests.has(key)) return;

    fetchFn()
      .then(data => {
        const entry = this.cache.get(key);
        if (entry) {
          entry.data = data;
          entry.timestamp = Date.now();
          entry.expiresAt = Date.now() + this.config.defaultTTL;
        }
      })
      .catch(() => {
        // Silently fail background refresh
      });
  }

  // Invalidate cache for a resource type
  invalidate(resourceType: string): void {
    const prefix = `${resourceType}:`;
    for (const key of this.cache.keys()) {
      if (key.startsWith(prefix)) {
        this.cache.delete(key);
      }
    }
  }

  // Invalidate all cache
  invalidateAll(): void {
    this.cache.clear();
  }

  // Invalidate cache for a specific team
  invalidateTeam(teamId: string): void {
    for (const [key] of this.cache.entries()) {
      // Check if the cache key contains this teamId
      if (key.includes(`"teamId":"${teamId}"`)) {
        this.cache.delete(key);
      }
    }
  }

  // Enforce max entries per resource type
  private enforceMaxEntries(resourceType: string): void {
    const prefix = `${resourceType}:`;
    const entries: Array<[string, CacheEntry<unknown>]> = [];

    for (const [key, entry] of this.cache.entries()) {
      if (key.startsWith(prefix)) {
        entries.push([key, entry]);
      }
    }

    // Remove oldest entries if over limit
    if (entries.length > this.config.maxEntries) {
      entries.sort((a, b) => a[1].timestamp - b[1].timestamp);
      const toRemove = entries.length - this.config.maxEntries;
      for (let i = 0; i < toRemove; i++) {
        this.cache.delete(entries[i][0]);
      }
    }
  }

  // Get cache statistics
  getStats(): {
    totalEntries: number;
    byType: Record<string, number>;
    hitRate: number;
  } {
    const byType: Record<string, number> = {};

    for (const key of this.cache.keys()) {
      const type = key.split(':')[0];
      byType[type] = (byType[type] || 0) + 1;
    }

    return {
      totalEntries: this.cache.size,
      byType,
      hitRate: 0, // Would need tracking to calculate
    };
  }
}

// Singleton instance
export const resourceCache = new ResourceCache();

// Hook for using cache in components
export function useResourceCache(): ResourceCache {
  return resourceCache;
}
