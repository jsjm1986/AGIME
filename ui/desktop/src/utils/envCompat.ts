/**
 * Environment Variable Compatibility Layer for AGIME/Goose Migration
 *
 * This module provides dual-prefix support for configuration during
 * the brand migration from "Goose" to "AGIME". Configuration can be accessed
 * using either AGIME_* prefix (preferred) or GOOSE_* prefix (legacy).
 *
 * ## Priority Order
 * 1. AGIME_ prefix (new, preferred)
 * 2. GOOSE_ prefix (legacy, fallback)
 *
 * ## Usage Examples
 * ```typescript
 * import { getConfigCompat, getEnvCompat, configCompatExists } from '@/utils/envCompat';
 *
 * // Get from window.appConfig with dual-prefix support
 * const provider = getConfigCompat('PROVIDER'); // Checks AGIME_PROVIDER, then GOOSE_PROVIDER
 *
 * // Get from process.env with dual-prefix support
 * const apiKey = getEnvCompat('API_KEY'); // Checks AGIME_API_KEY, then GOOSE_API_KEY
 *
 * // Get with typed default
 * const mode = getConfigCompatTyped<string>('MODE', 'auto');
 * ```
 */

/** The new AGIME prefix for configuration (preferred) */
export const AGIME_PREFIX = 'AGIME_';

/** The legacy GOOSE prefix for configuration (fallback) */
export const GOOSE_PREFIX = 'GOOSE_';

/**
 * Get a configuration value from window.appConfig with dual-prefix support.
 *
 * Tries AGIME_ prefix first, then falls back to GOOSE_ prefix.
 *
 * @param key - The key without prefix (e.g., "PROVIDER" not "GOOSE_PROVIDER")
 * @returns The configuration value, or undefined if not found
 *
 * @example
 * // With GOOSE_MODE=auto in config:
 * getConfigCompat('MODE') // returns 'auto'
 *
 * // With both AGIME_MODE=manual and GOOSE_MODE=auto:
 * getConfigCompat('MODE') // returns 'manual' (AGIME takes priority)
 */
export function getConfigCompat(key: string): unknown {
  // Check if window.appConfig is available (Electron environment)
  if (typeof window !== 'undefined' && window.appConfig) {
    // Tier 1: Try AGIME_ prefix (preferred)
    const agimeKey = `${AGIME_PREFIX}${key}`;
    const agimeValue = window.appConfig.get?.(agimeKey);
    if (agimeValue !== undefined && agimeValue !== null) {
      return agimeValue;
    }

    // Tier 2: Try GOOSE_ prefix (legacy fallback)
    const gooseKey = `${GOOSE_PREFIX}${key}`;
    const gooseValue = window.appConfig.get?.(gooseKey);
    if (gooseValue !== undefined && gooseValue !== null) {
      return gooseValue;
    }
  }

  return undefined;
}

/**
 * Get a configuration value with type safety and optional default.
 *
 * @param key - The key without prefix
 * @param defaultValue - Optional default value if not found
 * @returns The typed configuration value, or default/undefined if not found
 */
export function getConfigCompatTyped<T>(key: string, defaultValue?: T): T | undefined {
  const value = getConfigCompat(key);
  if (value !== undefined) {
    return value as T;
  }
  return defaultValue;
}

/**
 * Get an environment variable from process.env with dual-prefix support.
 *
 * @param key - The key without prefix
 * @returns The environment variable value, or undefined if not found
 */
export function getEnvCompat(key: string): string | undefined {
  // Check if process.env is available
  if (typeof process !== 'undefined' && process.env) {
    // Tier 1: Try AGIME_ prefix (preferred)
    const agimeKey = `${AGIME_PREFIX}${key}`;
    const agimeValue = process.env[agimeKey];
    if (agimeValue !== undefined && agimeValue !== '') {
      return agimeValue;
    }

    // Tier 2: Try GOOSE_ prefix (legacy fallback)
    const gooseKey = `${GOOSE_PREFIX}${key}`;
    const gooseValue = process.env[gooseKey];
    if (gooseValue !== undefined && gooseValue !== '') {
      return gooseValue;
    }
  }

  return undefined;
}

/**
 * Get an environment variable with a default value.
 *
 * @param key - The key without prefix
 * @param defaultValue - Default value if not found
 * @returns The environment variable value, or the default
 */
export function getEnvCompatOr(key: string, defaultValue: string): string {
  return getEnvCompat(key) ?? defaultValue;
}

/**
 * Get a configuration value from either appConfig or process.env with dual-prefix support.
 *
 * Priority:
 * 1. window.appConfig with AGIME_ prefix
 * 2. window.appConfig with GOOSE_ prefix
 * 3. process.env with AGIME_ prefix
 * 4. process.env with GOOSE_ prefix
 *
 * @param key - The key without prefix
 * @returns The configuration value as string, or undefined if not found
 */
export function getAnyConfigCompat(key: string): string | undefined {
  // First try appConfig
  const appConfigValue = getConfigCompat(key);
  if (appConfigValue !== undefined) {
    return String(appConfigValue);
  }

  // Then try process.env
  return getEnvCompat(key);
}

/**
 * Check if a configuration exists under either prefix.
 *
 * @param key - The key without prefix
 * @returns true if the configuration exists under AGIME_ or GOOSE_ prefix
 */
export function configCompatExists(key: string): boolean {
  if (typeof window !== 'undefined' && window.appConfig) {
    const agimeKey = `${AGIME_PREFIX}${key}`;
    const gooseKey = `${GOOSE_PREFIX}${key}`;

    const agimeValue = window.appConfig.get?.(agimeKey);
    const gooseValue = window.appConfig.get?.(gooseKey);

    if (agimeValue !== undefined && agimeValue !== null) return true;
    if (gooseValue !== undefined && gooseValue !== null) return true;
  }

  return false;
}

/**
 * Check if an environment variable exists under either prefix.
 *
 * @param key - The key without prefix
 * @returns true if the env var exists under AGIME_ or GOOSE_ prefix
 */
export function envCompatExists(key: string): boolean {
  if (typeof process !== 'undefined' && process.env) {
    const agimeKey = `${AGIME_PREFIX}${key}`;
    const gooseKey = `${GOOSE_PREFIX}${key}`;

    if (process.env[agimeKey] !== undefined && process.env[agimeKey] !== '') return true;
    if (process.env[gooseKey] !== undefined && process.env[gooseKey] !== '') return true;
  }

  return false;
}

/**
 * Get the actual key name that was found (for debugging/logging).
 *
 * @param key - The key without prefix
 * @returns Object with the actual prefixed key name, value, and source
 */
export function getConfigCompatWithSource(
  key: string
): { fullKey: string; value: unknown; source: 'appConfig' | 'env' } | undefined {
  // Check appConfig first
  if (typeof window !== 'undefined' && window.appConfig) {
    const agimeKey = `${AGIME_PREFIX}${key}`;
    const agimeValue = window.appConfig.get?.(agimeKey);
    if (agimeValue !== undefined && agimeValue !== null) {
      return { fullKey: agimeKey, value: agimeValue, source: 'appConfig' };
    }

    const gooseKey = `${GOOSE_PREFIX}${key}`;
    const gooseValue = window.appConfig.get?.(gooseKey);
    if (gooseValue !== undefined && gooseValue !== null) {
      return { fullKey: gooseKey, value: gooseValue, source: 'appConfig' };
    }
  }

  // Check process.env
  if (typeof process !== 'undefined' && process.env) {
    const agimeKey = `${AGIME_PREFIX}${key}`;
    const agimeValue = process.env[agimeKey];
    if (agimeValue !== undefined && agimeValue !== '') {
      return { fullKey: agimeKey, value: agimeValue, source: 'env' };
    }

    const gooseKey = `${GOOSE_PREFIX}${key}`;
    const gooseValue = process.env[gooseKey];
    if (gooseValue !== undefined && gooseValue !== '') {
      return { fullKey: gooseKey, value: gooseValue, source: 'env' };
    }
  }

  return undefined;
}

/**
 * Convert a legacy GOOSE_ prefixed key to the new AGIME_ prefix.
 *
 * @param key - A key that may have GOOSE_ prefix
 * @returns The key with AGIME_ prefix if it had GOOSE_ prefix, otherwise unchanged
 */
export function migrateKeyToAgime(key: string): string {
  if (key.startsWith(GOOSE_PREFIX)) {
    return `${AGIME_PREFIX}${key.slice(GOOSE_PREFIX.length)}`;
  }
  return key;
}

/**
 * Check if a full key (with prefix) is using the legacy GOOSE_ prefix.
 */
export function isLegacyKey(key: string): boolean {
  return key.startsWith(GOOSE_PREFIX);
}

/**
 * Check if a full key (with prefix) is using the new AGIME_ prefix.
 */
export function isAgimeKey(key: string): boolean {
  return key.startsWith(AGIME_PREFIX);
}

/**
 * Get the base key without any prefix.
 *
 * @param key - A key that may have AGIME_ or GOOSE_ prefix
 * @returns The key without prefix
 */
export function stripPrefix(key: string): string {
  if (key.startsWith(AGIME_PREFIX)) {
    return key.slice(AGIME_PREFIX.length);
  }
  if (key.startsWith(GOOSE_PREFIX)) {
    return key.slice(GOOSE_PREFIX.length);
  }
  return key;
}

/**
 * Build a full key with the preferred AGIME_ prefix.
 *
 * @param baseKey - The key without prefix
 * @returns The key with AGIME_ prefix
 */
export function buildAgimeKey(baseKey: string): string {
  const stripped = stripPrefix(baseKey);
  return `${AGIME_PREFIX}${stripped}`;
}

/**
 * Build a full key with the legacy GOOSE_ prefix.
 *
 * @param baseKey - The key without prefix
 * @returns The key with GOOSE_ prefix
 */
export function buildGooseKey(baseKey: string): string {
  const stripped = stripPrefix(baseKey);
  return `${GOOSE_PREFIX}${stripped}`;
}

// Re-export for backward compatibility with existing code that directly uses GOOSE_ keys
// These constants can be used to maintain compatibility while transitioning
export const LEGACY_CONFIG_KEYS = {
  PROVIDER: 'GOOSE_PROVIDER',
  MODEL: 'GOOSE_MODEL',
  MODE: 'GOOSE_MODE',
  WORKING_DIR: 'GOOSE_WORKING_DIR',
  API_HOST: 'GOOSE_API_HOST',
  DEFAULT_PROVIDER: 'GOOSE_DEFAULT_PROVIDER',
  DEFAULT_MODEL: 'GOOSE_DEFAULT_MODEL',
  PREDEFINED_MODELS: 'GOOSE_PREDEFINED_MODELS',
  BASE_URL_SHARE: 'GOOSE_BASE_URL_SHARE',
  VERSION: 'GOOSE_VERSION',
  EXTERNAL_BACKEND: 'GOOSE_EXTERNAL_BACKEND',
  ALLOWLIST_WARNING: 'GOOSE_ALLOWLIST_WARNING',
  PATH_ROOT: 'GOOSE_PATH_ROOT',
  ALLOWLIST: 'GOOSE_ALLOWLIST',
  MAX_TURNS: 'GOOSE_MAX_TURNS',
  ENABLE_ROUTER: 'GOOSE_ENABLE_ROUTER',
  LEAD_MODEL: 'GOOSE_LEAD_MODEL',
  LEAD_PROVIDER: 'GOOSE_LEAD_PROVIDER',
  LEAD_TURNS: 'GOOSE_LEAD_TURNS',
  LEAD_FAILURE_THRESHOLD: 'GOOSE_LEAD_FAILURE_THRESHOLD',
  LEAD_FALLBACK_TURNS: 'GOOSE_LEAD_FALLBACK_TURNS',
  SERVER_MEMORY: 'GOOSE_SERVER__MEMORY',
  SERVER_COMPUTER_CONTROLLER: 'GOOSE_SERVER__COMPUTER_CONTROLLER',
} as const;

// New AGIME_ equivalents
export const AGIME_CONFIG_KEYS = {
  PROVIDER: 'AGIME_PROVIDER',
  MODEL: 'AGIME_MODEL',
  MODE: 'AGIME_MODE',
  WORKING_DIR: 'AGIME_WORKING_DIR',
  API_HOST: 'AGIME_API_HOST',
  DEFAULT_PROVIDER: 'AGIME_DEFAULT_PROVIDER',
  DEFAULT_MODEL: 'AGIME_DEFAULT_MODEL',
  PREDEFINED_MODELS: 'AGIME_PREDEFINED_MODELS',
  BASE_URL_SHARE: 'AGIME_BASE_URL_SHARE',
  VERSION: 'AGIME_VERSION',
  EXTERNAL_BACKEND: 'AGIME_EXTERNAL_BACKEND',
  ALLOWLIST_WARNING: 'AGIME_ALLOWLIST_WARNING',
  PATH_ROOT: 'AGIME_PATH_ROOT',
  ALLOWLIST: 'AGIME_ALLOWLIST',
  MAX_TURNS: 'AGIME_MAX_TURNS',
  ENABLE_ROUTER: 'AGIME_ENABLE_ROUTER',
  LEAD_MODEL: 'AGIME_LEAD_MODEL',
  LEAD_PROVIDER: 'AGIME_LEAD_PROVIDER',
  LEAD_TURNS: 'AGIME_LEAD_TURNS',
  LEAD_FAILURE_THRESHOLD: 'AGIME_LEAD_FAILURE_THRESHOLD',
  LEAD_FALLBACK_TURNS: 'AGIME_LEAD_FALLBACK_TURNS',
  SERVER_MEMORY: 'AGIME_SERVER__MEMORY',
  SERVER_COMPUTER_CONTROLLER: 'AGIME_SERVER__COMPUTER_CONTROLLER',
} as const;
