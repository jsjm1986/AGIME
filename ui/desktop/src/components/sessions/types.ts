import type { Session, ExtensionData } from '../../api/types.gen';

// ============================================================================
// Filter State Types
// ============================================================================

/**
 * Date range filter options
 */
export type DateRangeType = 'all' | 'today' | 'week' | 'month' | 'custom_range' | 'custom_dates';

/**
 * Sort field options
 */
export type SortByType = 'updated_at' | 'created_at' | 'message_count' | 'total_tokens';

/**
 * Sort order
 */
export type SortOrderType = 'desc' | 'asc';

/**
 * Complete filter state for session list
 */
export interface SessionFilterState {
  // Date range filter
  dateRange: DateRangeType;
  customDateStart?: Date;
  customDateEnd?: Date;
  customDates?: Date[];  // For selecting multiple non-contiguous dates

  // Working directory filter
  workingDir: string | null;

  // Sort options
  sortBy: SortByType;
  sortOrder: SortOrderType;

  // Favorites and tags (Phase 2)
  showFavoritesOnly: boolean;
  selectedTags: string[];
}

/**
 * Default filter state
 */
export const defaultFilterState: SessionFilterState = {
  dateRange: 'all',
  workingDir: null,
  sortBy: 'updated_at',
  sortOrder: 'desc',
  showFavoritesOnly: false,
  selectedTags: [],
};

// ============================================================================
// Session Metadata Types (for extension_data)
// ============================================================================

/**
 * Keys used in extension_data for session metadata
 */
export const EXTENSION_DATA_KEYS = {
  FAVORITES: 'favorites.v0',
  TAGS: 'tags.v0',
} as const;

/**
 * Metadata extracted from session's extension_data
 */
export interface SessionMetadata {
  isFavorite: boolean;
  tags: string[];
}

/**
 * Extract metadata from session's extension_data
 */
export function extractSessionMetadata(session: Session): SessionMetadata {
  const extData: ExtensionData = session.extension_data || {};
  return {
    isFavorite: extData[EXTENSION_DATA_KEYS.FAVORITES] === true,
    tags: Array.isArray(extData[EXTENSION_DATA_KEYS.TAGS])
      ? (extData[EXTENSION_DATA_KEYS.TAGS] as string[])
      : [],
  };
}

// ============================================================================
// Filter Helper Functions
// ============================================================================

/**
 * Get the start of today in local time
 */
export function getStartOfToday(): Date {
  const now = new Date();
  now.setHours(0, 0, 0, 0);
  return now;
}

/**
 * Get the start of this week (Monday) in local time
 */
export function getStartOfWeek(): Date {
  const now = new Date();
  const day = now.getDay();
  // Adjust for Monday as start of week (0 = Sunday, 1 = Monday, ...)
  const diff = day === 0 ? 6 : day - 1;
  now.setDate(now.getDate() - diff);
  now.setHours(0, 0, 0, 0);
  return now;
}

/**
 * Get the start of this month in local time
 */
export function getStartOfMonth(): Date {
  const now = new Date();
  now.setDate(1);
  now.setHours(0, 0, 0, 0);
  return now;
}

/**
 * Check if a session matches the date range filter
 */
export function matchesDateRange(
  session: Session,
  dateRange: DateRangeType,
  customStart?: Date,
  customEnd?: Date,
  customDates?: Date[]
): boolean {
  if (dateRange === 'all') return true;

  const sessionDate = new Date(session.updated_at);

  switch (dateRange) {
    case 'today':
      return sessionDate >= getStartOfToday();
    case 'week':
      return sessionDate >= getStartOfWeek();
    case 'month':
      return sessionDate >= getStartOfMonth();
    case 'custom_range':
      if (customStart && sessionDate < customStart) return false;
      if (customEnd) {
        const endOfDay = new Date(customEnd);
        endOfDay.setHours(23, 59, 59, 999);
        if (sessionDate > endOfDay) return false;
      }
      return true;
    case 'custom_dates':
      if (!customDates || customDates.length === 0) return true;
      const sessionDateStr = sessionDate.toDateString();
      return customDates.some((d) => d.toDateString() === sessionDateStr);
    default:
      return true;
  }
}

/**
 * Compare sessions for sorting
 */
export function compareSessions(
  a: Session,
  b: Session,
  sortBy: SortByType,
  sortOrder: SortOrderType
): number {
  let comparison = 0;

  switch (sortBy) {
    case 'updated_at':
      comparison = new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime();
      break;
    case 'created_at':
      comparison = new Date(b.created_at).getTime() - new Date(a.created_at).getTime();
      break;
    case 'message_count':
      comparison = (b.message_count || 0) - (a.message_count || 0);
      break;
    case 'total_tokens':
      comparison = (b.total_tokens || 0) - (a.total_tokens || 0);
      break;
  }

  return sortOrder === 'asc' ? -comparison : comparison;
}

/**
 * Extract unique working directories from sessions
 */
export function extractWorkingDirs(sessions: Session[]): string[] {
  const dirs = new Set<string>();
  for (const session of sessions) {
    if (session.working_dir) {
      dirs.add(session.working_dir);
    }
  }
  return Array.from(dirs).sort();
}

/**
 * Extract all unique tags from sessions
 */
export function extractAllTags(sessions: Session[]): string[] {
  const tags = new Set<string>();
  for (const session of sessions) {
    const metadata = extractSessionMetadata(session);
    for (const tag of metadata.tags) {
      tags.add(tag);
    }
  }
  return Array.from(tags).sort();
}

/**
 * Check if any filters are active (not at default values)
 */
export function hasActiveFilters(filters: SessionFilterState): boolean {
  return (
    filters.dateRange !== 'all' ||
    filters.workingDir !== null ||
    filters.sortBy !== 'updated_at' ||
    filters.sortOrder !== 'desc' ||
    filters.showFavoritesOnly ||
    filters.selectedTags.length > 0
  );
}
