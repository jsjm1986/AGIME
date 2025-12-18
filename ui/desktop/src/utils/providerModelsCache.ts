/**
 * Provider Models Cache Module
 *
 * Caches model lists fetched from providers to improve performance
 * when opening the model selection dialog. Uses in-memory caching
 * with TTL and request deduplication.
 */

interface CachedModels {
  data: string[];
  timestamp: number;
  inFlight?: Promise<string[]>;
}

// Cache TTL: 30 minutes
const CACHE_TTL_MS = 30 * 60 * 1000;

const providerModelsCache = new Map<string, CachedModels>();

/**
 * Get cached models for a provider if cache is valid
 */
export function getCachedModels(providerName: string): string[] | null {
  const cached = providerModelsCache.get(providerName);
  if (!cached) return null;

  const now = Date.now();
  if (now - cached.timestamp > CACHE_TTL_MS) {
    return null; // Cache expired
  }

  return cached.data;
}

/**
 * Store models in cache for a provider
 */
export function setCachedModels(providerName: string, models: string[]): void {
  const existing = providerModelsCache.get(providerName);
  providerModelsCache.set(providerName, {
    data: models,
    timestamp: Date.now(),
    inFlight: existing?.inFlight,
  });
}

/**
 * Get in-flight request promise for a provider (for request deduplication)
 */
export function getInFlightRequest(providerName: string): Promise<string[]> | null {
  const cached = providerModelsCache.get(providerName);
  return cached?.inFlight || null;
}

/**
 * Set in-flight request promise for a provider
 */
export function setInFlightRequest(providerName: string, promise: Promise<string[]>): void {
  const cached = providerModelsCache.get(providerName);
  if (cached) {
    cached.inFlight = promise;
  } else {
    providerModelsCache.set(providerName, {
      data: [],
      timestamp: 0,
      inFlight: promise,
    });
  }
}

/**
 * Clear in-flight request for a provider (call after request completes)
 */
export function clearInFlightRequest(providerName: string): void {
  const cached = providerModelsCache.get(providerName);
  if (cached) {
    cached.inFlight = undefined;
  }
}

/**
 * Clear cache for a specific provider or all providers
 */
export function clearProviderModelsCache(providerName?: string): void {
  if (providerName) {
    providerModelsCache.delete(providerName);
  } else {
    providerModelsCache.clear();
  }
}

/**
 * Check if cache is valid for a provider
 */
export function isCacheValid(providerName: string): boolean {
  const cached = providerModelsCache.get(providerName);
  if (!cached) return false;
  return Date.now() - cached.timestamp <= CACHE_TTL_MS;
}
