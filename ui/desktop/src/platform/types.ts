/**
 * Platform abstraction layer types
 * Allows the same React codebase to run on both Electron desktop and Web browser
 */

import type { Recipe } from '../recipe';

// Shared types (mirrored from preload.ts for web compatibility)
export interface NotificationData {
  title: string;
  body: string;
}

export interface MessageBoxOptions {
  type?: 'none' | 'info' | 'error' | 'question' | 'warning';
  buttons?: string[];
  defaultId?: number;
  title?: string;
  message: string;
  detail?: string;
}

export interface MessageBoxResponse {
  response: number;
  checkboxChecked?: boolean;
}

export interface FileResponse {
  file: string;
  filePath: string;
  error: string | null;
  found: boolean;
}

export interface SaveDataUrlResponse {
  id: string;
  filePath?: string;
  error?: string;
}

export interface OpenDialogReturnValue {
  canceled: boolean;
  filePaths: string[];
}

export interface UpdaterEvent {
  event: string;
  data?: unknown;
}

export interface UpdateInfo {
  updateInfo: unknown;
  error: string | null;
}

export interface UpdateState {
  updateAvailable: boolean;
  latestVersion?: string;
}

export type PlatformType = 'darwin' | 'win32' | 'linux' | 'web';

/**
 * Platform capabilities - what features are available
 */
export interface PlatformCapabilities {
  /** Can access local file system */
  fileSystem: boolean;
  /** Can show native notifications */
  notifications: boolean;
  /** Can open file/directory dialogs */
  fileDialogs: boolean;
  /** Can auto-update the application */
  autoUpdate: boolean;
  /** Can control system power (wakelock) */
  powerControl: boolean;
  /** Can control dock/menubar icons */
  systemTray: boolean;
  /** Can open multiple windows */
  multiWindow: boolean;
  /** Has IPC event system */
  ipcEvents: boolean;
}

/**
 * IPC Event callback type
 */
export type IpcEventCallback = (event: unknown, ...args: unknown[]) => void;

/**
 * Platform API interface
 * All methods that interact with the platform should go through this interface
 */
export interface PlatformAPI {
  // Platform identification
  readonly platform: PlatformType;
  readonly isWeb: boolean;
  readonly isElectron: boolean;
  readonly capabilities: PlatformCapabilities;

  // Lifecycle
  reactReady(): void;
  reloadApp(): void;
  closeWindow(): void;

  // Configuration
  getConfig(): Record<string, unknown>;
  getVersion(): string;

  // API connection
  getSecretKey(): Promise<string>;
  getAgimedHostPort(): Promise<string | null>;
  /** @deprecated Use getAgimedHostPort instead */
  getGoosedHostPort(): Promise<string | null>;

  // Settings (persisted across sessions)
  getSettings(): Promise<unknown | null>;
  saveSettings(settings: unknown): Promise<boolean>;

  // Window management
  hideWindow(): void;
  createChatWindow(
    query?: string,
    dir?: string,
    version?: string,
    resumeSessionId?: string,
    viewType?: string,
    recipeId?: string
  ): void;

  // Dialogs
  directoryChooser(replace?: boolean): Promise<OpenDialogReturnValue>;
  selectFileOrDirectory(defaultPath?: string): Promise<string | null>;
  showMessageBox(options: MessageBoxOptions): Promise<MessageBoxResponse>;

  // Notifications
  showNotification(data: NotificationData): void;
  openNotificationsSettings(): Promise<boolean>;

  // Logging
  logInfo(txt: string): void;

  // External URLs
  openInChrome(url: string): void;
  openExternal(url: string): Promise<void>;
  openDirectoryInExplorer(directoryPath: string): Promise<boolean>;
  fetchMetadata(url: string): Promise<string>;

  // File system
  readFile(filePath: string): Promise<FileResponse>;
  writeFile(filePath: string, content: string): Promise<boolean>;
  ensureDirectory(dirPath: string): Promise<boolean>;
  listFiles(dirPath: string, extension?: string): Promise<string[]>;
  getPathForFile(file: File): string;
  getAllowedExtensions(): Promise<string[]>;
  getBinaryPath(binaryName: string): Promise<string>;

  // Image handling
  saveDataUrlToTemp(dataUrl: string, uniqueId: string): Promise<SaveDataUrlResponse>;
  deleteTempFile(filePath: string): void;
  getTempImage(filePath: string): Promise<string | null>;

  // System tray / dock
  setMenuBarIcon(show: boolean): Promise<boolean>;
  getMenuBarIconState(): Promise<boolean>;
  setDockIcon(show: boolean): Promise<boolean>;
  getDockIconState(): Promise<boolean>;

  // Power management
  setWakelock(enable: boolean): Promise<boolean>;
  getWakelockState(): Promise<boolean>;

  // Ollama
  checkForOllama(): Promise<boolean>;

  // Updates
  checkForUpdates(): Promise<UpdateInfo>;
  downloadUpdate(): Promise<{ success: boolean; error: string | null }>;
  installUpdate(): void;
  restartApp(): void;
  onUpdaterEvent(callback: (event: UpdaterEvent) => void): void;
  getUpdateState(): Promise<UpdateState | null>;
  isUsingGitHubFallback(): Promise<boolean>;

  // Chat state
  setChatBusy(busy: boolean): void;

  // Recipe management
  hasAcceptedRecipeBefore(recipe: Recipe): Promise<boolean>;
  recordRecipeHash(recipe: Recipe): Promise<boolean>;

  // IPC Events
  on(channel: string, callback: IpcEventCallback): void;
  off(channel: string, callback: IpcEventCallback): void;
  emit(channel: string, ...args: unknown[]): void;
  onMouseBackButtonClicked(callback: () => void): void;
  offMouseBackButtonClicked(callback: () => void): void;

  // Theme
  broadcastThemeChange(themeData: {
    mode: string;
    useSystemTheme: boolean;
    theme: string;
  }): void;
}

/**
 * App config API interface
 */
export interface AppConfigAPI {
  get(key: string): unknown;
  getAll(): Record<string, unknown>;
}
