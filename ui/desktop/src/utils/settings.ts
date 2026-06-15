import { app } from 'electron';
import fs from 'fs';
import path from 'path';

export interface EnvToggles {
  AGIME_SERVER__MEMORY: boolean;
  AGIME_SERVER__COMPUTER_CONTROLLER: boolean;
}

export interface ExternalAgimedConfig {
  enabled: boolean;
  url: string;
  secret: string;
}

export interface Settings {
  envToggles: EnvToggles;
  showMenuBarIcon: boolean;
  showDockIcon: boolean;
  enableWakelock: boolean;
  externalAgimed?: ExternalAgimedConfig;
}

const SETTINGS_FILE = path.join(app.getPath('userData'), 'settings.json');

const defaultSettings: Settings = {
  envToggles: {
    AGIME_SERVER__MEMORY: false,
    AGIME_SERVER__COMPUTER_CONTROLLER: false,
  },
  showMenuBarIcon: true,
  showDockIcon: true,
  enableWakelock: false,
};

// Settings management
export function loadSettings(): Settings {
  try {
    if (fs.existsSync(SETTINGS_FILE)) {
      const data = fs.readFileSync(SETTINGS_FILE, 'utf8');
      try {
        const parsed = JSON.parse(data) as Partial<Settings>;
        // Merge with defaults so a partial-but-valid file (e.g. missing
        // envToggles) can't leave required fields undefined and crash callers
        // like updateEnvironmentVariables().
        return {
          ...defaultSettings,
          ...parsed,
          envToggles: {
            ...defaultSettings.envToggles,
            ...(parsed.envToggles ?? {}),
          },
        };
      } catch (parseError) {
        console.error('Error parsing settings.json, resetting to defaults:', parseError);
        // Backup corrupted file for debugging
        try {
          const backupPath = SETTINGS_FILE + '.corrupt.' + Date.now();
          fs.writeFileSync(backupPath, data);
          console.warn(`Corrupted settings backed up to: ${backupPath}`);
        } catch {
          // Backup failed, continue
        }
      }
    }
  } catch (error) {
    console.error('Error loading settings:', error);
  }
  return defaultSettings;
}

export function saveSettings(settings: Settings): void {
  try {
    fs.writeFileSync(SETTINGS_FILE, JSON.stringify(settings, null, 2));
  } catch (error) {
    console.error('Error saving settings:', error);
  }
}

export function updateEnvironmentVariables(envToggles: EnvToggles): void {
  if (envToggles.AGIME_SERVER__MEMORY) {
    process.env.AGIME_SERVER__MEMORY = 'true';
  } else {
    delete process.env.AGIME_SERVER__MEMORY;
  }

  if (envToggles.AGIME_SERVER__COMPUTER_CONTROLLER) {
    process.env.AGIME_SERVER__COMPUTER_CONTROLLER = 'true';
  } else {
    delete process.env.AGIME_SERVER__COMPUTER_CONTROLLER;
  }
}
