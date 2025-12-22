/**
 * Web platform implementation
 * Provides browser-compatible alternatives for Electron APIs
 */

import type {
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
import type { Recipe } from '../recipe';

// Storage keys
const OLD_STORAGE_KEY_SETTINGS = 'goose_settings';
const OLD_STORAGE_KEY_RECIPE_HASHES = 'goose_recipe_hashes';
const STORAGE_KEY_SETTINGS = 'agime_settings';
const STORAGE_KEY_RECIPE_HASHES = 'agime_recipe_hashes';

// Migrate from old keys to new keys (one-time migration)
if (typeof window !== 'undefined' && typeof localStorage !== 'undefined') {
  // Migrate settings
  if (!localStorage.getItem(STORAGE_KEY_SETTINGS) && localStorage.getItem(OLD_STORAGE_KEY_SETTINGS)) {
    localStorage.setItem(STORAGE_KEY_SETTINGS, localStorage.getItem(OLD_STORAGE_KEY_SETTINGS) as string);
    localStorage.removeItem(OLD_STORAGE_KEY_SETTINGS);
  }
  // Migrate recipe hashes
  if (!localStorage.getItem(STORAGE_KEY_RECIPE_HASHES) && localStorage.getItem(OLD_STORAGE_KEY_RECIPE_HASHES)) {
    localStorage.setItem(STORAGE_KEY_RECIPE_HASHES, localStorage.getItem(OLD_STORAGE_KEY_RECIPE_HASHES) as string);
    localStorage.removeItem(OLD_STORAGE_KEY_RECIPE_HASHES);
  }
}

// In-memory secret storage (more secure than sessionStorage - not accessible via XSS)
let memorySecret: string | null = null;

/**
 * Generate a secure SHA-256 hash for a recipe
 */
async function hashRecipe(recipe: Recipe): Promise<string> {
  const data = JSON.stringify({ title: recipe.title, description: recipe.description });
  const encoder = new TextEncoder();
  const dataBuffer = encoder.encode(data);

  // Use Web Crypto API for secure hashing
  if (crypto?.subtle) {
    const hashBuffer = await crypto.subtle.digest('SHA-256', dataBuffer);
    const hashArray = Array.from(new Uint8Array(hashBuffer));
    return hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
  }

  // Fallback for environments without Web Crypto (should be rare)
  return btoa(data);
}

/**
 * Web platform capabilities - limited compared to Electron
 */
const webCapabilities: PlatformCapabilities = {
  fileSystem: false, // No direct filesystem access
  notifications: 'Notification' in window, // Browser notifications
  fileDialogs: false, // No native file dialogs
  autoUpdate: false, // No auto-update (handled by server)
  powerControl: false, // No wakelock
  systemTray: false, // No system tray
  multiWindow: true, // Can open new tabs
  ipcEvents: false, // No IPC, use custom events
};

/**
 * Custom event system for web (replaces IPC)
 */
const eventListeners = new Map<string, Set<IpcEventCallback>>();

/**
 * Web platform implementation
 * Provides browser-compatible alternatives with graceful degradation
 */
export const webPlatform: PlatformAPI = {
  // Platform identification
  platform: 'web' as PlatformType,
  isWeb: true,
  isElectron: false,
  capabilities: webCapabilities,

  // Lifecycle
  reactReady(): void {
    // No-op on web - there's no main process to notify
    console.log('[Web] App ready');
  },
  reloadApp(): void {
    window.location.reload();
  },
  closeWindow(): void {
    window.close();
  },

  // Configuration
  getConfig(): Record<string, unknown> {
    // Return empty config on web
    return {};
  },
  getVersion(): string {
    // Version could be injected at build time
    return import.meta.env.VITE_APP_VERSION || 'web';
  },

  // API connection
  async getSecretKey(): Promise<string> {
    // Get from memory (set during authentication flow)
    if (!memorySecret) {
      throw new Error('No authentication secret found. Please access via tunnel URL with secret.');
    }
    return memorySecret;
  },
  async getGoosedHostPort(): Promise<string | null> {
    // On web, the API is at the same origin
    return window.location.origin;
  },

  // Settings (using localStorage)
  async getSettings(): Promise<unknown | null> {
    try {
      const stored = localStorage.getItem(STORAGE_KEY_SETTINGS);
      return stored ? JSON.parse(stored) : null;
    } catch (error) {
      console.warn('[Web] Failed to get settings:', error);
      return null;
    }
  },
  async saveSettings(settings: unknown): Promise<boolean> {
    try {
      localStorage.setItem(STORAGE_KEY_SETTINGS, JSON.stringify(settings));
      return true;
    } catch (error) {
      console.warn('[Web] Failed to save settings:', error);
      return false;
    }
  },

  // Window management
  hideWindow(): void {
    // Can't hide browser window, minimize behavior
    console.warn('[Web] hideWindow not available');
  },
  createChatWindow(
    query?: string,
    _dir?: string,
    _version?: string,
    resumeSessionId?: string,
    viewType?: string,
    recipeId?: string
  ): void {
    // Open new tab with parameters
    const params = new URLSearchParams();
    if (query) params.set('q', query);
    if (resumeSessionId) params.set('resumeSessionId', resumeSessionId);
    if (viewType) params.set('view', viewType);
    if (recipeId) params.set('recipeId', recipeId);

    const url = `${window.location.pathname}?${params.toString()}`;
    window.open(url, '_blank');
  },

  // Dialogs
  async directoryChooser(_replace?: boolean): Promise<OpenDialogReturnValue> {
    // Directory selection not available on web
    console.warn('[Web] directoryChooser not available');
    return { canceled: true, filePaths: [] };
  },
  async selectFileOrDirectory(_defaultPath?: string): Promise<string | null> {
    // File selection not available on web
    console.warn('[Web] selectFileOrDirectory not available');
    return null;
  },
  async showMessageBox(options: MessageBoxOptions): Promise<MessageBoxResponse> {
    // Use browser confirm/alert as fallback
    const result = window.confirm(`${options.title || ''}\n\n${options.message}\n${options.detail || ''}`);
    return {
      response: result ? 0 : 1,
      checkboxChecked: false,
    };
  },

  // Notifications
  showNotification(data: NotificationData): void {
    if (!webCapabilities.notifications) {
      console.log('[Web] Notification:', data.title, data.body);
      return;
    }

    if (Notification.permission === 'granted') {
      new Notification(data.title, { body: data.body });
    } else if (Notification.permission !== 'denied') {
      Notification.requestPermission().then((permission) => {
        if (permission === 'granted') {
          new Notification(data.title, { body: data.body });
        }
      });
    }
  },
  async openNotificationsSettings(): Promise<boolean> {
    // Can't open system settings from browser
    console.warn('[Web] openNotificationsSettings not available');
    return false;
  },

  // Logging
  logInfo(txt: string): void {
    console.log('[Web]', txt);
  },

  // External URLs
  openInChrome(url: string): void {
    window.open(url, '_blank');
  },
  async openExternal(url: string): Promise<void> {
    window.open(url, '_blank', 'noopener,noreferrer');
  },
  async openDirectoryInExplorer(_directoryPath: string): Promise<boolean> {
    console.warn('[Web] openDirectoryInExplorer not available');
    return false;
  },
  async fetchMetadata(url: string): Promise<string> {
    try {
      const response = await fetch(url);
      return await response.text();
    } catch (error) {
      console.error('[Web] fetchMetadata error:', error);
      return '';
    }
  },

  // File system (not available on web)
  async readFile(_filePath: string): Promise<FileResponse> {
    return {
      file: '',
      filePath: '',
      error: 'File system access not available on web',
      found: false,
    };
  },
  async writeFile(_filePath: string, _content: string): Promise<boolean> {
    console.warn('[Web] writeFile not available');
    return false;
  },
  async ensureDirectory(_dirPath: string): Promise<boolean> {
    console.warn('[Web] ensureDirectory not available');
    return false;
  },
  async listFiles(_dirPath: string, _extension?: string): Promise<string[]> {
    console.warn('[Web] listFiles not available');
    return [];
  },
  getPathForFile(_file: File): string {
    // Return file name since we can't get the full path
    return _file.name;
  },
  async getAllowedExtensions(): Promise<string[]> {
    return [];
  },
  async getBinaryPath(_binaryName: string): Promise<string> {
    return '';
  },

  // Image handling
  async saveDataUrlToTemp(dataUrl: string, uniqueId: string): Promise<SaveDataUrlResponse> {
    // Store in memory/blob URL instead of temp file
    try {
      const blob = await fetch(dataUrl).then((r) => r.blob());
      const blobUrl = URL.createObjectURL(blob);
      return {
        id: uniqueId,
        filePath: blobUrl,
      };
    } catch (error) {
      return {
        id: uniqueId,
        error: String(error),
      };
    }
  },
  deleteTempFile(filePath: string): void {
    // Revoke blob URL if it's a blob
    if (filePath.startsWith('blob:')) {
      URL.revokeObjectURL(filePath);
    }
  },
  async getTempImage(filePath: string): Promise<string | null> {
    // If it's a blob URL, return it directly
    if (filePath.startsWith('blob:')) {
      return filePath;
    }
    return null;
  },

  // System tray / dock (not available on web)
  async setMenuBarIcon(_show: boolean): Promise<boolean> {
    return false;
  },
  async getMenuBarIconState(): Promise<boolean> {
    return false;
  },
  async setDockIcon(_show: boolean): Promise<boolean> {
    return false;
  },
  async getDockIconState(): Promise<boolean> {
    return false;
  },

  // Power management (not available on web)
  async setWakelock(_enable: boolean): Promise<boolean> {
    // Could use Wake Lock API in the future
    return false;
  },
  async getWakelockState(): Promise<boolean> {
    return false;
  },

  // Ollama
  async checkForOllama(): Promise<boolean> {
    // Can't check for local Ollama from web
    return false;
  },

  // Updates (not applicable on web)
  async checkForUpdates(): Promise<UpdateInfo> {
    return { updateInfo: null, error: 'Updates not available on web' };
  },
  async downloadUpdate(): Promise<{ success: boolean; error: string | null }> {
    return { success: false, error: 'Updates not available on web' };
  },
  installUpdate(): void {
    console.warn('[Web] installUpdate not available');
  },
  restartApp(): void {
    window.location.reload();
  },
  onUpdaterEvent(_callback: (event: UpdaterEvent) => void): void {
    // No-op on web
  },
  async getUpdateState(): Promise<UpdateState | null> {
    return null;
  },
  async isUsingGitHubFallback(): Promise<boolean> {
    return false;
  },

  // Chat state
  setChatBusy(_busy: boolean): void {
    // Could update document title or favicon
    // For now, no-op
  },

  // Recipe management (using localStorage with secure hashing)
  async hasAcceptedRecipeBefore(recipe: Recipe): Promise<boolean> {
    try {
      const hashes = JSON.parse(localStorage.getItem(STORAGE_KEY_RECIPE_HASHES) || '[]');
      const hash = await hashRecipe(recipe);
      return hashes.includes(hash);
    } catch {
      return false;
    }
  },
  async recordRecipeHash(recipe: Recipe): Promise<boolean> {
    try {
      const hashes = JSON.parse(localStorage.getItem(STORAGE_KEY_RECIPE_HASHES) || '[]');
      const hash = await hashRecipe(recipe);
      if (!hashes.includes(hash)) {
        hashes.push(hash);
        localStorage.setItem(STORAGE_KEY_RECIPE_HASHES, JSON.stringify(hashes));
      }
      return true;
    } catch {
      return false;
    }
  },

  // IPC Events (using custom event system)
  on(channel: string, callback: IpcEventCallback): void {
    if (!eventListeners.has(channel)) {
      eventListeners.set(channel, new Set());
    }
    eventListeners.get(channel)!.add(callback);
  },
  off(channel: string, callback: IpcEventCallback): void {
    eventListeners.get(channel)?.delete(callback);
  },
  emit(channel: string, ...args: unknown[]): void {
    const listeners = eventListeners.get(channel);
    if (listeners) {
      listeners.forEach((callback) => callback(null, ...args));
    }
  },
  onMouseBackButtonClicked(callback: () => void): void {
    window.addEventListener('popstate', callback);
  },
  offMouseBackButtonClicked(callback: () => void): void {
    window.removeEventListener('popstate', callback);
  },

  // Theme
  broadcastThemeChange(themeData: {
    mode: string;
    useSystemTheme: boolean;
    theme: string;
  }): void {
    // Apply theme directly since there's no main process
    if (themeData.useSystemTheme) {
      localStorage.setItem('use_system_theme', 'true');
    } else {
      localStorage.setItem('use_system_theme', 'false');
      localStorage.setItem('theme', themeData.theme);
    }

    const isDark = themeData.useSystemTheme
      ? window.matchMedia('(prefers-color-scheme: dark)').matches
      : themeData.mode === 'dark';

    if (isDark) {
      document.documentElement.classList.add('dark');
      document.documentElement.classList.remove('light');
    } else {
      document.documentElement.classList.remove('dark');
      document.documentElement.classList.add('light');
    }

    // Emit event for any listeners
    this.emit('theme-changed', themeData);
  },
};

/**
 * Web app config implementation
 * Uses URL parameters and environment variables
 */
export const webAppConfig: AppConfigAPI = {
  get(key: string): unknown {
    // Check URL parameters first
    const urlParams = new URLSearchParams(window.location.search);
    const urlValue = urlParams.get(key);
    if (urlValue !== null) return urlValue;

    // Check environment variables (injected at build time)
    const envKey = `VITE_${key}` as keyof ImportMetaEnv;
    return import.meta.env[envKey];
  },
  getAll(): Record<string, unknown> {
    const config: Record<string, unknown> = {};

    // Add URL parameters
    const urlParams = new URLSearchParams(window.location.search);
    urlParams.forEach((value, key) => {
      config[key] = value;
    });

    return config;
  },
};

/**
 * Helper to set the tunnel secret (called during authentication)
 * Uses memory storage for better security (not accessible via XSS)
 */
export function setTunnelSecret(secret: string): void {
  memorySecret = secret;
}

/**
 * Helper to clear the tunnel secret (called on logout)
 */
export function clearTunnelSecret(): void {
  memorySecret = null;
  // Also clear any persisted data
  try {
    localStorage.removeItem(STORAGE_KEY_SETTINGS);
    localStorage.removeItem(STORAGE_KEY_RECIPE_HASHES);
  } catch {
    // Ignore storage errors (e.g., in private mode)
  }
}

/**
 * Check if user is authenticated
 */
export function isAuthenticated(): boolean {
  return !!memorySecret;
}

/**
 * Clear all event listeners (for cleanup/testing)
 * Helps prevent memory leaks when unmounting
 */
export function clearAllEventListeners(): void {
  eventListeners.clear();
}
