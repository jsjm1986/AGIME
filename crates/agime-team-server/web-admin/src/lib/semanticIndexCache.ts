import {
  type SemanticIndexResponse,
  semanticIndexApi,
} from '../api/semanticIndex';

const semanticIndexCache = new Map<string, SemanticIndexResponse>();
const semanticIndexInFlight = new Map<string, Promise<SemanticIndexResponse>>();

export async function loadSemanticIndex(
  teamId: string,
  options?: { force?: boolean },
): Promise<SemanticIndexResponse> {
  const cacheKey = teamId.trim();
  if (!cacheKey) {
    throw new Error('teamId is required');
  }

  if (!options?.force) {
    const cached = semanticIndexCache.get(cacheKey);
    if (cached) {
      return cached;
    }

    const pending = semanticIndexInFlight.get(cacheKey);
    if (pending) {
      return pending;
    }
  }

  const request = semanticIndexApi
    .get(cacheKey)
    .then((payload) => {
      semanticIndexCache.set(cacheKey, payload);
      return payload;
    })
    .finally(() => {
      semanticIndexInFlight.delete(cacheKey);
    });

  semanticIndexInFlight.set(cacheKey, request);
  return request;
}

export function getCachedSemanticIndex(
  teamId: string,
): SemanticIndexResponse | null {
  return semanticIndexCache.get(teamId.trim()) ?? null;
}

export function invalidateSemanticIndex(teamId?: string): void {
  if (teamId) {
    const cacheKey = teamId.trim();
    semanticIndexCache.delete(cacheKey);
    semanticIndexInFlight.delete(cacheKey);
    return;
  }
  semanticIndexCache.clear();
  semanticIndexInFlight.clear();
}
