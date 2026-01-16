// LAN connection state management using localStorage
// Manages peer-to-peer connections with other AGIME instances

import { LANConnection, LANConnectionStatus, AddLANConnectionRequest, LANShareSettings } from './types';

// Storage keys
const STORAGE_KEYS = {
    CONNECTIONS: 'AGIME_TEAM_LAN_CONNECTIONS',
    SHARE_SETTINGS: 'AGIME_TEAM_LAN_SHARE_SETTINGS',
};

// Default port for AGIME
const DEFAULT_PORT = 7778;

// Generate a unique ID
function generateId(): string {
    return `lan_${Date.now()}_${Math.random().toString(36).substring(2, 9)}`;
}

// ============================================================
// LAN Connection CRUD Operations
// ============================================================

/**
 * Get all saved LAN connections
 */
export function getConnections(): LANConnection[] {
    try {
        const data = localStorage.getItem(STORAGE_KEYS.CONNECTIONS);
        return data ? JSON.parse(data) : [];
    } catch {
        return [];
    }
}

/**
 * Save connections to localStorage
 */
function saveConnections(connections: LANConnection[]): void {
    try {
        localStorage.setItem(STORAGE_KEYS.CONNECTIONS, JSON.stringify(connections));
    } catch (e) {
        console.error('Failed to save LAN connections:', e);
    }
}

/**
 * Add a new LAN connection
 */
export function addConnection(request: AddLANConnectionRequest): LANConnection {
    const connections = getConnections();

    // Check for duplicate host:port
    const key = `${request.host}:${request.port}`;
    if (connections.some(c => `${c.host}:${c.port}` === key)) {
        throw new Error('Connection to this address already exists');
    }

    const newConnection: LANConnection = {
        id: generateId(),
        name: request.name,
        host: request.host,
        port: request.port,
        secretKey: request.secretKey,
        myNickname: request.myNickname,
        status: 'disconnected',
        createdAt: new Date().toISOString(),
    };

    connections.push(newConnection);
    saveConnections(connections);

    return newConnection;
}

/**
 * Update a LAN connection
 */
export function updateConnection(
    connectionId: string,
    updates: Partial<Omit<LANConnection, 'id' | 'createdAt'>>
): LANConnection | null {
    const connections = getConnections();
    const index = connections.findIndex(c => c.id === connectionId);

    if (index === -1) {
        return null;
    }

    connections[index] = { ...connections[index], ...updates };
    saveConnections(connections);

    return connections[index];
}

/**
 * Update connection status
 */
export function updateConnectionStatus(
    connectionId: string,
    status: LANConnectionStatus,
    error?: string
): void {
    updateConnection(connectionId, {
        status,
        lastError: error,
        lastOnline: status === 'connected' ? new Date().toISOString() : undefined,
    });
}

/**
 * Remove a LAN connection
 */
export function removeConnection(connectionId: string): boolean {
    const connections = getConnections();
    const filtered = connections.filter(c => c.id !== connectionId);

    if (filtered.length === connections.length) {
        return false; // Not found
    }

    saveConnections(filtered);
    return true;
}

/**
 * Get a connection by ID
 */
export function getConnectionById(connectionId: string): LANConnection | null {
    const connections = getConnections();
    return connections.find(c => c.id === connectionId) ?? null;
}

/**
 * Get a connection by host:port
 */
export function getConnectionByAddress(host: string, port: number): LANConnection | null {
    const connections = getConnections();
    return connections.find(c => c.host === host && c.port === port) ?? null;
}

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

// ============================================================
// Connection Testing
// ============================================================

/**
 * Test connection to a LAN device
 */
export async function testLANConnection(
    host: string,
    port: number,
    secretKey: string
): Promise<{
    success: boolean;
    teamsCount?: number;
    version?: string;
    error?: string;
}> {
    try {
        const url = `http://${host}:${port}/api/team/teams?page=1&limit=1`;

        const response = await fetch(url, {
            method: 'GET',
            headers: {
                'Accept': 'application/json',
                'X-Secret-Key': secretKey,
            },
            signal: AbortSignal.timeout(5000), // 5 second timeout
        });

        if (!response.ok) {
            if (response.status === 401 || response.status === 403) {
                return { success: false, error: 'Invalid Secret Key' };
            }
            return { success: false, error: `HTTP ${response.status}` };
        }

        const data = await response.json();

        return {
            success: true,
            teamsCount: data.total || 0,
        };
    } catch (e) {
        return {
            success: false,
            error: e instanceof Error ? e.message : 'Connection failed',
        };
    }
}

/**
 * Check all connections' status
 */
export async function checkAllConnections(): Promise<void> {
    const connections = getConnections();

    for (const conn of connections) {
        updateConnectionStatus(conn.id, 'connecting');

        const result = await testLANConnection(conn.host, conn.port, conn.secretKey);

        if (result.success) {
            updateConnection(conn.id, {
                status: 'connected',
                teamsCount: result.teamsCount,
                lastOnline: new Date().toISOString(),
                lastError: undefined,
            });
        } else {
            updateConnectionStatus(conn.id, 'error', result.error);
        }
    }
}
