// Recent teams store - tracks team access across cloud and LAN connections
// Uses localStorage for persistence

import { Team } from './types';

// Storage key
const STORAGE_KEY = 'AGIME_TEAM_RECENT_TEAMS';

// Maximum number of recent teams to keep
const MAX_RECENT_TEAMS = 10;

/**
 * Source type for the team
 */
export type TeamSourceType = 'cloud' | 'lan';

/**
 * Recent team record
 */
export interface RecentTeam {
    /** Team ID */
    teamId: string;
    /** Team name */
    teamName: string;
    /** Source type (cloud server or LAN connection) */
    sourceType: TeamSourceType;
    /** Source ID (server ID or connection ID) */
    sourceId: string;
    /** Source name (server name or device name) */
    sourceName: string;
    /** Last accessed timestamp (ISO string) */
    lastAccessed: string;
    /** Team description (optional) */
    description?: string;
    /** Member count (optional) */
    memberCount?: number;
}

/**
 * Get all recent teams
 */
export function getRecentTeams(): RecentTeam[] {
    try {
        const data = localStorage.getItem(STORAGE_KEY);
        if (data) {
            const teams = JSON.parse(data) as RecentTeam[];
            // Sort by lastAccessed descending
            return teams.sort(
                (a, b) => new Date(b.lastAccessed).getTime() - new Date(a.lastAccessed).getTime()
            );
        }
    } catch {
        // Ignore errors
    }
    return [];
}

/**
 * Save recent teams to localStorage
 */
function saveRecentTeams(teams: RecentTeam[]): void {
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(teams));
    } catch (e) {
        console.error('Failed to save recent teams:', e);
    }
}

/**
 * Add or update a team in recent history
 */
export function addRecentTeam(
    team: Team,
    sourceType: TeamSourceType,
    sourceId: string,
    sourceName: string
): void {
    const teams = getRecentTeams();

    // Find existing entry
    const existingIndex = teams.findIndex(
        (t) => t.teamId === team.id && t.sourceType === sourceType && t.sourceId === sourceId
    );

    const recentTeam: RecentTeam = {
        teamId: team.id,
        teamName: team.name,
        sourceType,
        sourceId,
        sourceName,
        lastAccessed: new Date().toISOString(),
        description: team.description ?? undefined,
        // memberCount is not available on Team type, left undefined
    };

    if (existingIndex !== -1) {
        // Update existing entry
        teams.splice(existingIndex, 1);
    }

    // Add to beginning
    teams.unshift(recentTeam);

    // Limit to max size
    const trimmed = teams.slice(0, MAX_RECENT_TEAMS);

    saveRecentTeams(trimmed);
}

/**
 * Remove a team from recent history
 */
export function removeRecentTeam(teamId: string, sourceType: TeamSourceType, sourceId: string): void {
    const teams = getRecentTeams();
    const filtered = teams.filter(
        (t) => !(t.teamId === teamId && t.sourceType === sourceType && t.sourceId === sourceId)
    );
    saveRecentTeams(filtered);
}

/**
 * Clear all recent teams
 */
export function clearRecentTeams(): void {
    try {
        localStorage.removeItem(STORAGE_KEY);
    } catch {
        // Ignore
    }
}

/**
 * Get recent teams by source type
 */
export function getRecentTeamsBySource(sourceType: TeamSourceType): RecentTeam[] {
    return getRecentTeams().filter((t) => t.sourceType === sourceType);
}

/**
 * Get recent teams by source ID
 */
export function getRecentTeamsBySourceId(sourceId: string): RecentTeam[] {
    return getRecentTeams().filter((t) => t.sourceId === sourceId);
}

/**
 * Format relative time for display
 */
export function formatRelativeTime(isoString: string, t: (key: string, fallback: string) => string): string {
    const date = new Date(isoString);
    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffMins = Math.floor(diffMs / 60000);
    const diffHours = Math.floor(diffMs / 3600000);
    const diffDays = Math.floor(diffMs / 86400000);

    if (diffMins < 1) {
        return t('dashboard.justNow', 'Just now');
    } else if (diffMins < 60) {
        return t('dashboard.minutesAgo', '{{count}} min ago').replace('{{count}}', String(diffMins));
    } else if (diffHours < 24) {
        return t('dashboard.hoursAgo', '{{count}} hour ago').replace('{{count}}', String(diffHours));
    } else if (diffDays < 7) {
        return t('dashboard.daysAgo', '{{count}} days ago').replace('{{count}}', String(diffDays));
    } else {
        return date.toLocaleDateString();
    }
}
