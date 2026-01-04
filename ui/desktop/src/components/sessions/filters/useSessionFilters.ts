import { useState, useMemo, useCallback } from 'react';
import type { Session } from '../../../api/types.gen';
import {
  SessionFilterState,
  defaultFilterState,
  matchesDateRange,
  compareSessions,
  extractWorkingDirs,
  extractAllTags,
  extractSessionMetadata,
  hasActiveFilters,
  DateRangeType,
  SortByType,
  SortOrderType,
} from '../types';

interface UseSessionFiltersOptions {
  sessions: Session[];
  initialState?: Partial<SessionFilterState>;
}

interface UseSessionFiltersResult {
  // Filter state
  filters: SessionFilterState;

  // Filtered and sorted sessions
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
}

/**
 * Hook for managing session list filtering and sorting
 */
export function useSessionFilters({
  sessions,
  initialState,
}: UseSessionFiltersOptions): UseSessionFiltersResult {
  const [filters, setFilters] = useState<SessionFilterState>({
    ...defaultFilterState,
    ...initialState,
  });

  // Extract available working directories from all sessions
  const availableWorkingDirs = useMemo(() => {
    return extractWorkingDirs(sessions);
  }, [sessions]);

  // Extract available tags from all sessions
  const availableTags = useMemo(() => {
    return extractAllTags(sessions);
  }, [sessions]);

  // Apply filters and sorting
  const filteredSessions = useMemo(() => {
    let result = sessions;

    // Apply date range filter
    if (filters.dateRange !== 'all') {
      result = result.filter((session) =>
        matchesDateRange(
          session,
          filters.dateRange,
          filters.customDateStart,
          filters.customDateEnd,
          filters.customDates
        )
      );
    }

    // Apply working directory filter
    if (filters.workingDir) {
      result = result.filter((session) => session.working_dir === filters.workingDir);
    }

    // Apply favorites filter
    if (filters.showFavoritesOnly) {
      result = result.filter((session) => {
        const metadata = extractSessionMetadata(session);
        return metadata.isFavorite;
      });
    }

    // Apply tags filter
    if (filters.selectedTags.length > 0) {
      result = result.filter((session) => {
        const metadata = extractSessionMetadata(session);
        return filters.selectedTags.some((tag) => metadata.tags.includes(tag));
      });
    }

    // Apply sorting
    result = [...result].sort((a, b) =>
      compareSessions(a, b, filters.sortBy, filters.sortOrder)
    );

    return result;
  }, [sessions, filters]);

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
    totalCount: sessions.length,
    filteredCount: filteredSessions.length,
  };
}
