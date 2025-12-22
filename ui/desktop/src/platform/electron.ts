/**
 * Electron platform implementation
 * Wraps the existing window.electron API
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

/**
 * Electron platform capabilities - full native access
 */
const electronCapabilities: PlatformCapabilities = {
  fileSystem: true,
  notifications: true,
  fileDialogs: true,
  autoUpdate: true,
  powerControl: true,
  systemTray: true,
  multiWindow: true,
  ipcEvents: true,
};

/**
 * Electron platform implementation
 * Delegates all calls to window.electron
 */
export const electronPlatform: PlatformAPI = {
  // Platform identification
  get platform(): PlatformType {
    return window.electron.platform as PlatformType;
  },
  isWeb: false,
  isElectron: true,
  capabilities: electronCapabilities,

  // Lifecycle
  reactReady(): void {
    window.electron.reactReady();
  },
  reloadApp(): void {
    window.electron.reloadApp();
  },
  closeWindow(): void {
    window.electron.closeWindow();
  },

  // Configuration
  getConfig(): Record<string, unknown> {
    return window.electron.getConfig();
  },
  getVersion(): string {
    return window.electron.getVersion();
  },

  // API connection
  getSecretKey(): Promise<string> {
    return window.electron.getSecretKey();
  },
  getGoosedHostPort(): Promise<string | null> {
    return window.electron.getGoosedHostPort();
  },

  // Settings
  getSettings(): Promise<unknown | null> {
    return window.electron.getSettings();
  },
  saveSettings(settings: unknown): Promise<boolean> {
    return window.electron.saveSettings(settings);
  },

  // Window management
  hideWindow(): void {
    window.electron.hideWindow();
  },
  createChatWindow(
    query?: string,
    dir?: string,
    version?: string,
    resumeSessionId?: string,
    viewType?: string,
    recipeId?: string
  ): void {
    window.electron.createChatWindow(query, dir, version, resumeSessionId, viewType, recipeId);
  },

  // Dialogs
  directoryChooser(replace?: boolean): Promise<OpenDialogReturnValue> {
    return window.electron.directoryChooser(replace);
  },
  selectFileOrDirectory(defaultPath?: string): Promise<string | null> {
    return window.electron.selectFileOrDirectory(defaultPath);
  },
  showMessageBox(options: MessageBoxOptions): Promise<MessageBoxResponse> {
    return window.electron.showMessageBox(options);
  },

  // Notifications
  showNotification(data: NotificationData): void {
    window.electron.showNotification(data);
  },
  openNotificationsSettings(): Promise<boolean> {
    return window.electron.openNotificationsSettings();
  },

  // Logging
  logInfo(txt: string): void {
    window.electron.logInfo(txt);
  },

  // External URLs
  openInChrome(url: string): void {
    window.electron.openInChrome(url);
  },
  openExternal(url: string): Promise<void> {
    return window.electron.openExternal(url);
  },
  openDirectoryInExplorer(directoryPath: string): Promise<boolean> {
    return window.electron.openDirectoryInExplorer(directoryPath);
  },
  fetchMetadata(url: string): Promise<string> {
    return window.electron.fetchMetadata(url);
  },

  // File system
  readFile(filePath: string): Promise<FileResponse> {
    return window.electron.readFile(filePath);
  },
  writeFile(filePath: string, content: string): Promise<boolean> {
    return window.electron.writeFile(filePath, content);
  },
  ensureDirectory(dirPath: string): Promise<boolean> {
    return window.electron.ensureDirectory(dirPath);
  },
  listFiles(dirPath: string, extension?: string): Promise<string[]> {
    return window.electron.listFiles(dirPath, extension);
  },
  getPathForFile(file: File): string {
    return window.electron.getPathForFile(file);
  },
  getAllowedExtensions(): Promise<string[]> {
    return window.electron.getAllowedExtensions();
  },
  getBinaryPath(binaryName: string): Promise<string> {
    return window.electron.getBinaryPath(binaryName);
  },

  // Image handling
  saveDataUrlToTemp(dataUrl: string, uniqueId: string): Promise<SaveDataUrlResponse> {
    return window.electron.saveDataUrlToTemp(dataUrl, uniqueId);
  },
  deleteTempFile(filePath: string): void {
    window.electron.deleteTempFile(filePath);
  },
  getTempImage(filePath: string): Promise<string | null> {
    return window.electron.getTempImage(filePath);
  },

  // System tray / dock
  setMenuBarIcon(show: boolean): Promise<boolean> {
    return window.electron.setMenuBarIcon(show);
  },
  getMenuBarIconState(): Promise<boolean> {
    return window.electron.getMenuBarIconState();
  },
  setDockIcon(show: boolean): Promise<boolean> {
    return window.electron.setDockIcon(show);
  },
  getDockIconState(): Promise<boolean> {
    return window.electron.getDockIconState();
  },

  // Power management
  setWakelock(enable: boolean): Promise<boolean> {
    return window.electron.setWakelock(enable);
  },
  getWakelockState(): Promise<boolean> {
    return window.electron.getWakelockState();
  },

  // Ollama
  checkForOllama(): Promise<boolean> {
    return window.electron.checkForOllama();
  },

  // Updates
  checkForUpdates(): Promise<UpdateInfo> {
    return window.electron.checkForUpdates();
  },
  downloadUpdate(): Promise<{ success: boolean; error: string | null }> {
    return window.electron.downloadUpdate();
  },
  installUpdate(): void {
    window.electron.installUpdate();
  },
  restartApp(): void {
    window.electron.restartApp();
  },
  onUpdaterEvent(callback: (event: UpdaterEvent) => void): void {
    window.electron.onUpdaterEvent(callback);
  },
  getUpdateState(): Promise<UpdateState | null> {
    return window.electron.getUpdateState();
  },
  isUsingGitHubFallback(): Promise<boolean> {
    return window.electron.isUsingGitHubFallback();
  },

  // Chat state
  setChatBusy(busy: boolean): void {
    window.electron.setChatBusy(busy);
  },

  // Recipe management
  hasAcceptedRecipeBefore(recipe: Recipe): Promise<boolean> {
    return window.electron.hasAcceptedRecipeBefore(recipe);
  },
  recordRecipeHash(recipe: Recipe): Promise<boolean> {
    return window.electron.recordRecipeHash(recipe);
  },

  // IPC Events
  on(channel: string, callback: IpcEventCallback): void {
    window.electron.on(channel, callback);
  },
  off(channel: string, callback: IpcEventCallback): void {
    window.electron.off(channel, callback);
  },
  emit(channel: string, ...args: unknown[]): void {
    window.electron.emit(channel, ...args);
  },
  onMouseBackButtonClicked(callback: () => void): void {
    window.electron.onMouseBackButtonClicked(callback);
  },
  offMouseBackButtonClicked(callback: () => void): void {
    window.electron.offMouseBackButtonClicked(callback);
  },

  // Theme
  broadcastThemeChange(themeData: {
    mode: string;
    useSystemTheme: boolean;
    theme: string;
  }): void {
    window.electron.broadcastThemeChange(themeData);
  },
};

/**
 * Electron app config implementation
 */
export const electronAppConfig: AppConfigAPI = {
  get(key: string): unknown {
    return window.appConfig?.get(key);
  },
  getAll(): Record<string, unknown> {
    return window.appConfig?.getAll() ?? {};
  },
};
