/**
 * Provider presets for Quick Setup Wizard
 * Data loaded from providers.json for easy maintenance
 */

import providersData from './providers.json';

export type ProviderCategory = 'international' | 'china' | 'proxy';

export interface ModelPreset {
  name: string;
  description?: string;
  contextLimit?: number;
  isDefault?: boolean;
  isRecommended?: boolean;
}

export interface ProviderPreset {
  id: string;
  name: string;
  displayName: string;
  description: string;
  category: ProviderCategory;
  icon: string;

  // Connection config
  engine: 'openai' | 'anthropic' | 'ollama';
  baseUrl: string;
  apiKeyEnv: string;
  apiKeyHelpUrl?: string;

  // Model config
  canListModels: boolean;
  recommendedModels: ModelPreset[];
}

// Type for the JSON structure
interface ProviderJsonData {
  id: string;
  name: string;
  displayName: string;
  description: string;
  category: string;
  icon: string;
  engine: string;
  baseUrl: string;
  apiKeyEnv: string;
  apiKeyHelpUrl?: string;
  canListModels: boolean;
  models: Array<{
    name: string;
    description?: string;
    contextLimit?: number;
    isDefault?: boolean;
    isRecommended?: boolean;
  }>;
}

// Transform JSON data to typed ProviderPreset
function transformProvider(data: ProviderJsonData): ProviderPreset {
  return {
    id: data.id,
    name: data.name,
    displayName: data.displayName,
    description: data.description,
    category: data.category as ProviderCategory,
    icon: data.icon,
    engine: data.engine as 'openai' | 'anthropic' | 'ollama',
    baseUrl: data.baseUrl,
    apiKeyEnv: data.apiKeyEnv,
    apiKeyHelpUrl: data.apiKeyHelpUrl,
    canListModels: data.canListModels,
    recommendedModels: data.models,
  };
}

// Load and categorize providers from JSON
const allProvidersRaw = (providersData.providers as ProviderJsonData[]).map(transformProvider);

export const internationalProviders: ProviderPreset[] = allProvidersRaw.filter(
  p => p.category === 'international'
);

export const chinaProviders: ProviderPreset[] = allProvidersRaw.filter(
  p => p.category === 'china'
);

export const proxyProviders: ProviderPreset[] = allProvidersRaw.filter(
  p => p.category === 'proxy'
);

// All providers grouped
export const allProviders = {
  international: internationalProviders,
  china: chinaProviders,
  proxy: proxyProviders,
};

// Helper to find provider by id
export function getProviderById(id: string): ProviderPreset | undefined {
  return allProvidersRaw.find(p => p.id === id);
}

// Helper to get all providers as flat array
export function getAllProviders(): ProviderPreset[] {
  return allProvidersRaw;
}

// Helper to get provider icon
export function getProviderIcon(providerId: string): string {
  const provider = getProviderById(providerId);
  return provider?.icon || '🤖';
}

// Model base types for capability inheritance
export type ModelBaseType = 'gpt-4o' | 'claude' | 'deepseek' | 'qwen' | 'glm' | 'gemini' | 'llama' | 'other';

export interface ModelBaseTypeInfo {
  id: ModelBaseType;
  name: string;
  description: string;
  patterns: string[];
}

// Load model base types from JSON
export const modelBaseTypes: ModelBaseTypeInfo[] = (providersData.modelBaseTypes as Array<{
  id: string;
  name: string;
  description: string;
  patterns: string[];
}>).map(t => ({
  id: t.id as ModelBaseType,
  name: t.name,
  description: t.description,
  patterns: t.patterns,
}));

// Helper to match model name to base type
export function matchModelBaseType(modelName: string): ModelBaseType {
  const lowerName = modelName.toLowerCase();

  for (const baseType of modelBaseTypes) {
    if (baseType.id === 'other') continue;

    for (const pattern of baseType.patterns) {
      // Convert glob pattern to regex
      const regexPattern = pattern
        .replace(/\*/g, '.*')
        .replace(/\?/g, '.');

      if (new RegExp(`^${regexPattern}$`, 'i').test(modelName)) {
        return baseType.id;
      }
    }
  }

  // Check by simple substring match as fallback
  if (lowerName.includes('gpt') || lowerName.includes('openai')) return 'gpt-4o';
  if (lowerName.includes('claude') || lowerName.includes('anthropic')) return 'claude';
  if (lowerName.includes('deepseek')) return 'deepseek';
  if (lowerName.includes('qwen') || lowerName.includes('qwq')) return 'qwen';
  if (lowerName.includes('glm')) return 'glm';
  if (lowerName.includes('gemini')) return 'gemini';
  if (lowerName.includes('llama')) return 'llama';

  return 'other';
}

// Category info from JSON
export const categoryInfo = providersData.categories as Record<string, {
  title: string;
  icon: string;
}>;
