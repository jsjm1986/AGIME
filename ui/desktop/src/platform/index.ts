/**
 * Platform abstraction layer - main entry point
 *
 * This module provides a unified API that works on both Electron desktop and Web browser.
 * It automatically detects the current platform and exports the appropriate implementation.
 *
 * Usage:
 * ```typescript
 * import { platform, appConfig, isWeb, isElectron } from './platform';
 *
 * // Use platform API
 * const secret = await platform.getSecretKey();
 * const settings = await platform.getSettings();
 *
 * // Check platform
 * if (isWeb) {
 *   // Web-specific logic
 * }
 * ```
 */

import type { PlatformAPI, AppConfigAPI, PlatformCapabilities } from './types';
import { electronPlatform, electronAppConfig } from './electron';
import { webPlatform, webAppConfig, setTunnelSecret, clearTunnelSecret, isAuthenticated, clearAllEventListeners } from './web';

// Re-export types
export type {
  PlatformAPI,
  PlatformCapabilities,
  PlatformType,
  NotificationData,
  MessageBoxOptions,
  MessageBoxResponse,
  OpenDialogReturnValue,
  FileResponse,
  SaveDataUrlResponse,
  UpdateInfo,
  UpdateState,
  UpdaterEvent,
  IpcEventCallback,
  AppConfigAPI,
} from './types';

// Re-export web helpers
export { setTunnelSecret, clearTunnelSecret, isAuthenticated, clearAllEventListeners };

/**
 * Detect if running in Electron environment
 */
export const isElectron: boolean = typeof window !== 'undefined' && typeof window.electron !== 'undefined';

/**
 * Detect if running in Web browser environment
 */
export const isWeb: boolean = !isElectron;

/**
 * Detect if running on macOS
 */
export const isMac: boolean = typeof navigator !== 'undefined' && /Mac|iPhone|iPad|iPod/i.test(navigator.platform);

/**
 * Platform API instance
 * Automatically uses Electron or Web implementation based on environment
 */
export const platform: PlatformAPI = isElectron ? electronPlatform : webPlatform;

/**
 * App config API instance
 * Automatically uses Electron or Web implementation based on environment
 */
export const appConfig: AppConfigAPI = isElectron ? electronAppConfig : webAppConfig;

/**
 * Platform capabilities
 * Quick access to check what features are available
 */
export const capabilities: PlatformCapabilities = platform.capabilities;

/**
 * Check if a specific capability is available
 */
export function hasCapability(capability: keyof PlatformCapabilities): boolean {
  return platform.capabilities[capability];
}

// Install window.electron shim on web for compatibility with existing code
// This allows components that use window.electron directly to work on web
if (isWeb && typeof window !== 'undefined') {
  // Create a compatibility shim
  (window as unknown as { electron: PlatformAPI }).electron = webPlatform;
  // Also install appConfig
  (window as unknown as { appConfig: AppConfigAPI }).appConfig = webAppConfig;
}

// Default export for convenience
export default platform;
