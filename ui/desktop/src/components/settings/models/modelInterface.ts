import { ProviderDetails } from '../../../api';

export default interface Model {
  id?: number; // Make `id` optional to allow user-defined models
  name: string;
  provider: string;
  lastUsed?: string;
  alias?: string; // optional model display name
  subtext?: string; // goes below model name if not the provider
}

export function createModelStruct(
  modelName: string,
  provider: string,
  id?: number, // Make `id` optional to allow user-defined models
  lastUsed?: string,
  alias?: string, // optional model display name
  subtext?: string
): Model {
  // use the metadata to create a Model
  return {
    name: modelName,
    provider: provider,
    alias: alias,
    id: id,
    lastUsed: lastUsed,
    subtext: subtext,
  };
}

export async function getProviderMetadata(
  providerName: string,
  getProvidersFunc: (b: boolean) => Promise<ProviderDetails[]>
) {
  const providers = await getProvidersFunc(false);
  const matches = providers.find((providerMatch) => providerMatch.name === providerName);
  if (!matches) {
    throw Error(`No match for provider: ${providerName}`);
  }
  return matches.metadata;
}

export interface ProviderModelsResult {
  provider: ProviderDetails;
  models: string[] | null;
  error: string | null;
}

// Timeout for fetching models from a single provider (5 seconds)
const FETCH_MODELS_TIMEOUT_MS = 5000;

/**
 * Wraps a promise with a timeout
 */
function withTimeout<T>(promise: Promise<T>, timeoutMs: number, errorMessage: string): Promise<T> {
  return Promise.race([
    promise,
    new Promise<T>((_, reject) =>
      setTimeout(() => reject(new Error(errorMessage)), timeoutMs)
    )
  ]);
}

/**
 * Fetches recommended models for all active providers in parallel.
 * Falls back to known_models if fetching fails or returns no models.
 * Each provider has a 5-second timeout to prevent hanging.
 */
export async function fetchModelsForProviders(
  activeProviders: ProviderDetails[],
  getProviderModelsFunc: (providerName: string) => Promise<string[]>
): Promise<ProviderModelsResult[]> {
  const modelPromises = activeProviders.map(async (p) => {
    const providerName = p.name;
    try {
      // Add timeout to prevent hanging on slow providers
      let models = await withTimeout(
        getProviderModelsFunc(providerName),
        FETCH_MODELS_TIMEOUT_MS,
        `Timeout fetching models for ${providerName}`
      );
      if ((!models || models.length === 0) && p.metadata.known_models?.length) {
        console.log(`[ModelFetch] ${providerName}: No models from API, using known_models fallback`);
        models = p.metadata.known_models.map((m) => m.name);
      }
      return { provider: p, models, error: null };
    } catch (e: unknown) {
      const errorMessage = `Failed to fetch models for ${providerName}${e instanceof Error ? `: ${e.message}` : ''}`;
      console.warn(`[ModelFetch] ${errorMessage}`);
      // Fallback to known_models on error
      if (p.metadata.known_models?.length) {
        console.log(`[ModelFetch] ${providerName}: Using known_models fallback after error`);
        return {
          provider: p,
          models: p.metadata.known_models.map((m) => m.name),
          error: null,
        };
      }
      return {
        provider: p,
        models: null,
        error: errorMessage,
      };
    }
  });

  return await Promise.all(modelPromises);
}
