import { useState, useMemo, useCallback, useRef } from 'react';
import type { Session } from '../../../api/types.gen';
import {
  SessionFilterState,
  defaultFilterState,
  extractWorkingDirs,
  extractAllTags,
  hasActiveFilters,
  DateRangeType,
  SortByType,
  SortOrderType,
  getStartOfToday,
  getStartOfWeek,
  getStartOfMonth,
} from '../types';

interface UseSessionFiltersOptions {
  sessions: Session[];
  initialState?: Partial<SessionFilterState>;
  /** When true, skip client-side filtering (server handles it) */
  serverSideFiltering?: boolean;
  /** Total count from server (used when server-side filtering is enabled) */
  serverTotalCount?: number;
}

interface UseSessionFiltersResult {
  // Filter state
  filters: SessionFilterState;

  // Filtered and sorted sessions (or just sessions if server-side)
  filteredSessions: Session[];

  // Available options extracted from sessions
  availableWorkingDirs: string[];
  availableTags: string[];

  // Filter actions
  setDateRange: (range: DateRangeType, customStart?: Date, customEnd?: Date, customDates?: Date[]) => void;
  setWorkingDir: (dir: string | null) => void;
  setSortBy: (sortBy: SortByType) => void;
  setSortOrder: (order: SortOrderType) => void;
  toggleSortOrder: () => void;
  setShowFavoritesOnly: (show: boolean) => void;
  setSelectedTags: (tags: string[]) => void;
  toggleTag: (tag: string) => void;
  clearFilters: () => void;

  // State helpers
  hasActiveFilters: boolean;
  totalCount: number;
  filteredCount: number;

  // Convert filters to API parameters
  getApiParams: () => {
    workingDir?: string;
    dateFrom?: string;
    dateTo?: string;
    dates?: string;
    timezoneOffset?: number;
    sortBy?: string;
    sortOrder?: string;
    favoritesOnly?: boolean;
    tags?: string;
  };
}

/**
 * Convert date range type to actual date values
 */
function getDateRangeValues(
  dateRange: DateRangeType,
  customStart?: Date,
  customEnd?: Date,
  customDates?: Date[]
): { dateFrom?: Date; dateTo?: Date } {
  switch (dateRange) {
    case 'today':
      return { dateFrom: getStartOfToday() };
    case 'week':
      return { dateFrom: getStartOfWeek() };
    case 'month':
      return { dateFrom: getStartOfMonth() };
    case 'custom_range':
      return { dateFrom: customStart, dateTo: customEnd };
    case 'custom_dates':
      // For multiple non-contiguous dates, use min/max to create a range
      // This is a lossy conversion but works with the backend API
      if (customDates && customDates.length > 0) {
        const timestamps = customDates.map(d => d.getTime());
        const minDate = new Date(Math.min(...timestamps));
        const maxDate = new Date(Math.max(...timestamps));
        return { dateFrom: minDate, dateTo: maxDate };
      }
      return {};
    case 'all':
    default:
      return {};
  }
}

/**
 * Hook for managing session list filtering and sorting
 */
export function useSessionFilters({
  sessions,
  initialState,
  serverSideFiltering = false,
  serverTotalCount,
}: UseSessionFiltersOptions): UseSessionFiltersResult {
  const [filters, setFilters] = useState<SessionFilterState>({
    ...defaultFilterState,
    ...initialState,
  });

  // Track all working dirs seen across loads (for server-side filtering)
  const allWorkingDirsRef = useRef<Set<string>>(new Set());
  const allTagsRef = useRef<Set<string>>(new Set());

  // Extract available working directories from all sessions
  const availableWorkingDirs = useMemo(() => {
    const currentDirs = extractWorkingDirs(sessions);
    // Accumulate dirs for server-side filtering mode
    if (serverSideFiltering) {
      currentDirs.forEach(dir => allWorkingDirsRef.current.add(dir));
      return Array.from(allWorkingDirsRef.current).sort();
    }
    return currentDirs;
  }, [sessions, serverSideFiltering]);

  // Extract available tags from all sessions
  const availableTags = useMemo(() => {
    const currentTags = extractAllTags(sessions);
    // Accumulate tags for server-side filtering mode
    if (serverSideFiltering) {
      currentTags.forEach(tag => allTagsRef.current.add(tag));
      return Array.from(allTagsRef.current).sort();
    }
    return currentTags;
  }, [sessions, serverSideFiltering]);

  // For server-side filtering, just return sessions as-is (already filtered by server)
  // For client-side filtering, sessions are already the full list
  const filteredSessions = useMemo(() => {
    // When using server-side filtering, sessions are already filtered
    if (serverSideFiltering) {
      return sessions;
    }
    // Client-side: sessions passed in are the full list, no filtering needed here
    // (original filtering logic removed - caller should filter if needed)
    return sessions;
  }, [sessions, serverSideFiltering]);

  // Action handlers
  const setDateRange = useCallback(
    (range: DateRangeType, customStart?: Date, customEnd?: Date, customDates?: Date[]) => {
      setFilters((prev) => ({
        ...prev,
        dateRange: range,
        customDateStart: customStart,
        customDateEnd: customEnd,
        customDates: customDates,
      }));
    },
    []
  );

  const setWorkingDir = useCallback((dir: string | null) => {
    setFilters((prev) => ({ ...prev, workingDir: dir }));
  }, []);

  const setSortBy = useCallback((sortBy: SortByType) => {
    setFilters((prev) => ({ ...prev, sortBy }));
  }, []);

  const setSortOrder = useCallback((order: SortOrderType) => {
    setFilters((prev) => ({ ...prev, sortOrder: order }));
  }, []);

  const toggleSortOrder = useCallback(() => {
    setFilters((prev) => ({
      ...prev,
      sortOrder: prev.sortOrder === 'desc' ? 'asc' : 'desc',
    }));
  }, []);

  const setShowFavoritesOnly = useCallback((show: boolean) => {
    setFilters((prev) => ({ ...prev, showFavoritesOnly: show }));
  }, []);

  const setSelectedTags = useCallback((tags: string[]) => {
    setFilters((prev) => ({ ...prev, selectedTags: tags }));
  }, []);

  const toggleTag = useCallback((tag: string) => {
    setFilters((prev) => {
      const isSelected = prev.selectedTags.includes(tag);
      return {
        ...prev,
        selectedTags: isSelected
          ? prev.selectedTags.filter((t) => t !== tag)
          : [...prev.selectedTags, tag],
      };
    });
  }, []);

  const clearFilters = useCallback(() => {
    setFilters(defaultFilterState);
  }, []);

  // Convert current filter state to API query parameters
  const getApiParams = useCallback(() => {
    const params: {
      workingDir?: string;
      dateFrom?: string;
      dateTo?: string;
      dates?: string;
      timezoneOffset?: number;
      sortBy?: string;
      sortOrder?: string;
      favoritesOnly?: boolean;
      tags?: string;
    } = {};

    if (filters.workingDir) {
      params.workingDir = filters.workingDir;
    }

    // Handle date filtering - custom_dates sends discrete dates, others use range
    if (filters.dateRange === 'custom_dates' && filters.customDates && filters.customDates.length > 0) {
      // Send discrete dates as local date strings (YYYY-MM-DD) to avoid timezone issues
      params.dates = filters.customDates
        .map(d => {
          const year = d.getFullYear();
          const month = String(d.getMonth() + 1).padStart(2, '0');
          const day = String(d.getDate()).padStart(2, '0');
          return `${year}-${month}-${day}`;
        })
        .join(',');
      // Send timezone offset (in minutes) for proper UTC to local conversion
      // getTimezoneOffset() returns minutes behind UTC (e.g., UTC+8 returns -480)
      params.timezoneOffset = new Date().getTimezoneOffset();
    } else {
      // Use date range for other cases
      const { dateFrom, dateTo } = getDateRangeValues(
        filters.dateRange,
        filters.customDateStart,
        filters.customDateEnd,
        filters.customDates
      );
      if (dateFrom) {
        params.dateFrom = dateFrom.toISOString();
      }
      if (dateTo) {
        // Set to end of day
        const endOfDay = new Date(dateTo);
        endOfDay.setHours(23, 59, 59, 999);
        params.dateTo = endOfDay.toISOString();
      }
    }

    if (filters.sortBy !== 'updated_at') {
      params.sortBy = filters.sortBy;
    }
    if (filters.sortOrder !== 'desc') {
      params.sortOrder = filters.sortOrder;
    }
    if (filters.showFavoritesOnly) {
      params.favoritesOnly = true;
    }
    if (filters.selectedTags.length > 0) {
      params.tags = filters.selectedTags.join(',');
    }

    return params;
  }, [filters]);

  return {
    filters,
    filteredSessions,
    availableWorkingDirs,
    availableTags,
    setDateRange,
    setWorkingDir,
    setSortBy,
    setSortOrder,
    toggleSortOrder,
    setShowFavoritesOnly,
    setSelectedTags,
    toggleTag,
    clearFilters,
    hasActiveFilters: hasActiveFilters(filters),
    totalCount: serverSideFiltering && serverTotalCount !== undefined ? serverTotalCount : sessions.length,
    filteredCount: filteredSessions.length,
    getApiParams,
  };
}
