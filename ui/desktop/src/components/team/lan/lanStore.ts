// LAN Share Settings management using localStorage
// Note: LAN connection management has been moved to SourceManager
// This file only handles local sharing settings (whether this device shares to others)

import { LANShareSettings } from './types';

// Storage keys
const STORAGE_KEYS = {
    SHARE_SETTINGS: 'AGIME_TEAM_LAN_SHARE_SETTINGS',
};

// Default port for AGIME
const DEFAULT_PORT = 7778;

// ============================================================
// LAN Share Settings
// ============================================================

/**
 * Get LAN share settings
 */
export function getShareSettings(): LANShareSettings {
    try {
        const data = localStorage.getItem(STORAGE_KEYS.SHARE_SETTINGS);
        if (data) {
            return JSON.parse(data);
        }
    } catch {
        // Ignore
    }

    // Default settings
    return {
        enabled: false,
        port: DEFAULT_PORT,
        displayName: 'My AGIME',
    };
}

/**
 * Save LAN share settings
 */
export function saveShareSettings(settings: Partial<LANShareSettings>): LANShareSettings {
    const current = getShareSettings();
    const updated = { ...current, ...settings };

    try {
        localStorage.setItem(STORAGE_KEYS.SHARE_SETTINGS, JSON.stringify(updated));
    } catch (e) {
        console.error('Failed to save LAN share settings:', e);
    }

    return updated;
}

/**
 * Enable LAN sharing
 */
export function enableSharing(secretKey: string): LANShareSettings {
    return saveShareSettings({
        enabled: true,
        secretKey,
    });
}

/**
 * Disable LAN sharing
 */
export function disableSharing(): LANShareSettings {
    return saveShareSettings({
        enabled: false,
        secretKey: undefined,
    });
}
