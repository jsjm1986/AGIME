/**
 * Model Capabilities API service
 * Provides functions to interact with the backend capabilities registry
 */
/* eslint-disable no-undef */

import { getConfigCompat } from '../utils/envCompat';

export interface CapabilitiesResponse {
  model_name: string;
  matched_pattern: string | null;
  provider: string | null;
  thinking_supported: boolean;
  thinking_enabled: boolean;
  thinking_type: 'none' | 'api' | 'tag';
  thinking_budget: number | null;
  reasoning_supported: boolean;
  reasoning_effort: string | null;
  temperature_supported: boolean;
  tools_supported: boolean;
  system_role: string;
  context_length: number | null;
}

export interface CapableModelsResponse {
  thinking_models: string[];
  reasoning_models: string[];
}

export interface ThinkingConfigResponse {
  enabled: boolean;
  budget: number | null;
}

export interface SetThinkingConfigRequest {
  enabled: boolean;
  budget?: number;
}

/** Custom error class for API errors */
export class CapabilitiesAPIError extends Error {
  constructor(
    message: string,
    public readonly status?: number,
    public readonly statusText?: string
  ) {
    super(message);
    this.name = 'CapabilitiesAPIError';
  }
}

const getBaseUrl = (): string => {
  // Get from window.appConfig (set by preload.ts from main process or web platform shim)
  const apiHost = getConfigCompat('API_HOST') as string | undefined;
  if (apiHost) {
    return apiHost;
  }

  // Fallback: On web platform, use current origin (tunnel URL)
  if (typeof window !== 'undefined' && !window.electron?.isElectron) {
    return window.location.origin;
  }

  // Default fallback for Electron
  return 'http://localhost:38457';
};

/** Default timeout for API requests (10 seconds) */
const DEFAULT_TIMEOUT = 10000;

/**
 * Get headers with authentication
 */
async function getAuthHeaders(): Promise<Record<string, string>> {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  };
  try {
    const secretKey = await window.electron.getSecretKey();
    if (secretKey) {
      headers['X-Secret-Key'] = secretKey;
    }
  } catch (e) {
    console.warn('Failed to get secret key:', e);
  }
  return headers;
}

/**
 * Helper function to make fetch requests with timeout and error handling
 */
async function fetchWithTimeout<T>(
  url: string,
  options: RequestInit = {},
  timeout: number = DEFAULT_TIMEOUT
): Promise<T> {
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), timeout);

  // Get auth headers
  const authHeaders = await getAuthHeaders();
  const mergedHeaders = {
    ...authHeaders,
    ...(options.headers || {}),
  };

  try {
    const response = await fetch(url, {
      ...options,
      headers: mergedHeaders,
      signal: controller.signal,
    });

    if (!response.ok) {
      throw new CapabilitiesAPIError(
        `Request failed: ${response.statusText}`,
        response.status,
        response.statusText
      );
    }

    return await response.json();
  } catch (error) {
    if (error instanceof CapabilitiesAPIError) {
      throw error;
    }
    if (error instanceof Error) {
      if (error.name === 'AbortError') {
        throw new CapabilitiesAPIError('Request timed out');
      }
      throw new CapabilitiesAPIError(
        `Network error: ${error.message}`
      );
    }
    throw new CapabilitiesAPIError('Unknown error occurred');
  } finally {
    clearTimeout(timeoutId);
  }
}

/**
 * Get capabilities for a specific model
 */
export async function getModelCapabilities(modelName: string): Promise<CapabilitiesResponse> {
  return fetchWithTimeout<CapabilitiesResponse>(
    `${getBaseUrl()}/capabilities/${encodeURIComponent(modelName)}`
  );
}

/**
 * Get list of models with specific capabilities
 */
export async function getCapableModels(): Promise<CapableModelsResponse> {
  return fetchWithTimeout<CapableModelsResponse>(
    `${getBaseUrl()}/capabilities/models`
  );
}

/**
 * Get current thinking configuration
 */
export async function getThinkingConfig(): Promise<ThinkingConfigResponse> {
  return fetchWithTimeout<ThinkingConfigResponse>(
    `${getBaseUrl()}/config/thinking`
  );
}

/**
 * Set thinking configuration
 */
export async function setThinkingConfig(config: SetThinkingConfigRequest): Promise<string> {
  return fetchWithTimeout<string>(
    `${getBaseUrl()}/config/thinking`,
    {
      method: 'POST',
      body: JSON.stringify(config),
    }
  );
}
