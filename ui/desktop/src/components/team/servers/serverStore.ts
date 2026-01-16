// Server state management using localStorage
// Manages multiple cloud server connections

import { CloudServer, CreateCloudServerRequest, ServerConnectionStatus } from './types';

// Storage keys
const STORAGE_KEYS = {
    SERVERS: 'AGIME_TEAM_CLOUD_SERVERS',
    ACTIVE_SERVER: 'AGIME_TEAM_ACTIVE_SERVER',
};

// Generate a unique ID
function generateId(): string {
    return `server_${Date.now()}_${Math.random().toString(36).substring(2, 9)}`;
}

// ============================================================
// Server CRUD Operations
// ============================================================

/**
 * Get all saved cloud servers
 */
export function getServers(): CloudServer[] {
    try {
        const data = localStorage.getItem(STORAGE_KEYS.SERVERS);
        return data ? JSON.parse(data) : [];
    } catch {
        return [];
    }
}

/**
 * Save servers to localStorage
 */
function saveServers(servers: CloudServer[]): void {
    try {
        localStorage.setItem(STORAGE_KEYS.SERVERS, JSON.stringify(servers));
    } catch (e) {
        console.error('Failed to save servers:', e);
    }
}

/**
 * Add a new cloud server
 */
export function addServer(request: CreateCloudServerRequest): CloudServer {
    const servers = getServers();

    // Check for duplicate URL
    const normalizedUrl = request.url.replace(/\/+$/, ''); // Remove trailing slash
    if (servers.some(s => s.url.replace(/\/+$/, '') === normalizedUrl)) {
        throw new Error('Server with this URL already exists');
    }

    const newServer: CloudServer = {
        id: generateId(),
        name: request.name,
        url: normalizedUrl,
        apiKey: request.apiKey,
        teamsCount: 0,
        status: 'connecting',
        createdAt: new Date().toISOString(),
    };

    servers.push(newServer);
    saveServers(servers);

    return newServer;
}

/**
 * Update a cloud server
 */
export function updateServer(
    serverId: string,
    updates: Partial<Omit<CloudServer, 'id' | 'createdAt'>>
): CloudServer | null {
    const servers = getServers();
    const index = servers.findIndex(s => s.id === serverId);

    if (index === -1) {
        return null;
    }

    servers[index] = { ...servers[index], ...updates };
    saveServers(servers);

    return servers[index];
}

/**
 * Update server connection status
 */
export function updateServerStatus(
    serverId: string,
    status: ServerConnectionStatus,
    error?: string
): void {
    updateServer(serverId, {
        status,
        lastError: error,
        lastSyncedAt: status === 'online' ? new Date().toISOString() : undefined,
    });
}

/**
 * Remove a cloud server
 */
export function removeServer(serverId: string): boolean {
    const servers = getServers();
    const filtered = servers.filter(s => s.id !== serverId);

    if (filtered.length === servers.length) {
        return false; // Not found
    }

    saveServers(filtered);

    // If this was the active server, clear active server
    if (getActiveServerId() === serverId) {
        clearActiveServer();
    }

    return true;
}

/**
 * Get a server by ID
 */
export function getServerById(serverId: string): CloudServer | null {
    const servers = getServers();
    return servers.find(s => s.id === serverId) ?? null;
}

/**
 * Get a server by URL
 */
export function getServerByUrl(url: string): CloudServer | null {
    const normalizedUrl = url.replace(/\/+$/, '');
    const servers = getServers();
    return servers.find(s => s.url.replace(/\/+$/, '') === normalizedUrl) ?? null;
}

// ============================================================
// Active Server Management
// ============================================================

/**
 * Get the active server ID
 */
export function getActiveServerId(): string | null {
    try {
        return localStorage.getItem(STORAGE_KEYS.ACTIVE_SERVER);
    } catch {
        return null;
    }
}

/**
 * Get the active server
 */
export function getActiveServer(): CloudServer | null {
    const activeId = getActiveServerId();
    if (!activeId) return null;
    return getServerById(activeId);
}

/**
 * Set the active server
 */
export function setActiveServer(serverId: string): CloudServer | null {
    const server = getServerById(serverId);
    if (!server) return null;

    try {
        localStorage.setItem(STORAGE_KEYS.ACTIVE_SERVER, serverId);
    } catch (e) {
        console.error('Failed to set active server:', e);
    }

    return server;
}

/**
 * Clear the active server
 */
export function clearActiveServer(): void {
    try {
        localStorage.removeItem(STORAGE_KEYS.ACTIVE_SERVER);
    } catch {
        // Ignore
    }
}

// ============================================================
// Server Connection Testing
// ============================================================

/**
 * Test connection to a server
 * Returns the server info if successful
 */
export async function testServerConnection(
    url: string,
    apiKey: string
): Promise<{
    success: boolean;
    userEmail?: string;
    displayName?: string;
    userId?: string;
    error?: string;
}> {
    try {
        const normalizedUrl = url.replace(/\/+$/, '');

        // First, check health endpoint
        const healthResponse = await fetch(`${normalizedUrl}/health`, {
            method: 'GET',
            headers: {
                'Accept': 'application/json',
            },
        });

        if (!healthResponse.ok) {
            return { success: false, error: 'Server health check failed' };
        }

        // Then, validate API key by getting user info
        const userResponse = await fetch(`${normalizedUrl}/api/auth/me`, {
            method: 'GET',
            headers: {
                'Accept': 'application/json',
                'X-API-Key': apiKey,
            },
        });

        if (!userResponse.ok) {
            if (userResponse.status === 401) {
                return { success: false, error: 'Invalid API Key' };
            }
            return { success: false, error: `Auth failed: ${userResponse.statusText}` };
        }

        const userData = await userResponse.json();

        return {
            success: true,
            userEmail: userData.email,
            displayName: userData.displayName || userData.display_name,
            userId: userData.id,
        };
    } catch (e) {
        return {
            success: false,
            error: e instanceof Error ? e.message : 'Connection failed',
        };
    }
}

// ============================================================
// Migration from old single-server storage
// ============================================================

/**
 * Migrate from old localStorage keys to new multi-server format
 * Call this on app startup
 */
export function migrateFromOldStorage(): void {
    const OLD_KEYS = {
        CONNECTION_MODE: 'AGIME_TEAM_CONNECTION_MODE',
        CLOUD_SERVER_URL: 'AGIME_TEAM_SERVER_URL',
        CLOUD_API_KEY: 'AGIME_TEAM_API_KEY',
    };

    try {
        const mode = localStorage.getItem(OLD_KEYS.CONNECTION_MODE);
        const url = localStorage.getItem(OLD_KEYS.CLOUD_SERVER_URL);
        const apiKey = localStorage.getItem(OLD_KEYS.CLOUD_API_KEY);

        // Only migrate if there's old cloud config and no new servers
        if (mode === 'cloud' && url && apiKey && getServers().length === 0) {
            const server = addServer({
                name: 'Cloud Server (Migrated)',
                url,
                apiKey,
            });

            setActiveServer(server.id);

            // Clean up old keys
            localStorage.removeItem(OLD_KEYS.CONNECTION_MODE);
            localStorage.removeItem(OLD_KEYS.CLOUD_SERVER_URL);
            localStorage.removeItem(OLD_KEYS.CLOUD_API_KEY);

            console.log('Migrated old cloud server config to new format');
        }
    } catch (e) {
        console.error('Migration failed:', e);
    }
}
