import { memo, useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Calendar,
  FolderOpen,
  ArrowUpDown,
  X,
  Star,
  ChevronDown,
  ArrowUp,
  ArrowDown,
  Check,
} from 'lucide-react';
import { Button } from '../../ui/button';
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuLabel,
  DropdownMenuCheckboxItem,
} from '../../ui/dropdown-menu';
import { cn } from '../../../utils';
import type {
  SessionFilterState,
  DateRangeType,
  SortByType,
} from '../types';
import { DatePickerPopover } from './DatePickerPopover';

interface SessionFilterBarProps {
  filters: SessionFilterState;
  availableWorkingDirs: string[];
  availableTags: string[];
  hasActiveFilters: boolean;
  totalCount: number;
  filteredCount: number;
  onSetDateRange: (range: DateRangeType, customStart?: Date, customEnd?: Date, customDates?: Date[]) => void;
  onSetWorkingDir: (dir: string | null) => void;
  onSetSortBy: (sortBy: SortByType) => void;
  onToggleSortOrder: () => void;
  onSetShowFavoritesOnly: (show: boolean) => void;
  onToggleTag: (tag: string) => void;
  onClearFilters: () => void;
}

export const SessionFilterBar = memo(function SessionFilterBar({
  filters,
  availableWorkingDirs,
  availableTags,
  hasActiveFilters,
  totalCount,
  filteredCount,
  onSetDateRange,
  onSetWorkingDir,
  onSetSortBy,
  onToggleSortOrder,
  onSetShowFavoritesOnly,
  onToggleTag,
  onClearFilters,
}: SessionFilterBarProps) {
  const { t } = useTranslation('sessions');
  const [showDatePicker, setShowDatePicker] = useState(false);

  // Get display text for date range
  const getDateRangeLabel = useCallback(
    (range: DateRangeType) => {
      switch (range) {
        case 'all':
          return t('filters.dateRange.all');
        case 'today':
          return t('filters.dateRange.today');
        case 'week':
          return t('filters.dateRange.week');
        case 'month':
          return t('filters.dateRange.month');
        case 'custom_range':
        case 'custom_dates':
          if (filters.customDates && filters.customDates.length > 0) {
            return t('filters.dateRange.customDates', { count: filters.customDates.length });
          }
          return t('filters.dateRange.custom');
        default:
          return t('filters.dateRange.all');
      }
    },
    [t, filters.customDates]
  );

  // Get display text for sort option
  const getSortByLabel = useCallback(
    (sortBy: SortByType) => {
      switch (sortBy) {
        case 'updated_at':
          return t('filters.sortBy.updated_at');
        case 'created_at':
          return t('filters.sortBy.created_at');
        case 'message_count':
          return t('filters.sortBy.message_count');
        case 'total_tokens':
          return t('filters.sortBy.total_tokens');
        default:
          return t('filters.sortBy.updated_at');
      }
    },
    [t]
  );

  // Get shortened working dir for display
  const getShortWorkingDir = useCallback((dir: string) => {
    const parts = dir.split(/[/\\]/);
    if (parts.length <= 2) return dir;
    return '...' + parts.slice(-2).join('/');
  }, []);

  return (
    <div className="flex flex-wrap items-center gap-2 px-1 py-2">
      {/* Favorites toggle */}
      <Button
        variant={filters.showFavoritesOnly ? 'default' : 'ghost'}
        size="sm"
        onClick={() => onSetShowFavoritesOnly(!filters.showFavoritesOnly)}
        className={cn(
          'gap-1.5',
          filters.showFavoritesOnly && 'bg-yellow-500/20 text-yellow-600 hover:bg-yellow-500/30'
        )}
      >
        <Star
          className={cn(
            'w-4 h-4',
            filters.showFavoritesOnly && 'fill-yellow-500 text-yellow-500'
          )}
        />
        {t('filters.favorites')}
      </Button>

      {/* Tags dropdown */}
      {availableTags.length > 0 && (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant={filters.selectedTags.length > 0 ? 'secondary' : 'ghost'}
              size="sm"
              className="gap-1.5"
            >
              <span className="text-xs">
                {filters.selectedTags.length > 0
                  ? `${filters.selectedTags.length} ${t('filters.tagsSelected')}`
                  : t('filters.tags')}
              </span>
              <ChevronDown className="w-3.5 h-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="min-w-[160px]">
            <DropdownMenuLabel>{t('filters.tags')}</DropdownMenuLabel>
            <DropdownMenuSeparator />
            {availableTags.map((tag) => (
              <DropdownMenuCheckboxItem
                key={tag}
                checked={filters.selectedTags.includes(tag)}
                onCheckedChange={() => onToggleTag(tag)}
              >
                {tag}
              </DropdownMenuCheckboxItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      )}

      {/* Date range dropdown */}
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant={filters.dateRange !== 'all' ? 'secondary' : 'ghost'}
            size="sm"
            className="gap-1.5"
          >
            <Calendar className="w-4 h-4" />
            {getDateRangeLabel(filters.dateRange)}
            <ChevronDown className="w-3.5 h-3.5" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="min-w-[140px]">
          {(['all', 'today', 'week', 'month'] as DateRangeType[]).map((range) => (
            <DropdownMenuItem
              key={range}
              onClick={() => onSetDateRange(range)}
              className="flex items-center justify-between"
            >
              {getDateRangeLabel(range)}
              {filters.dateRange === range && <Check className="w-4 h-4 ml-2" />}
            </DropdownMenuItem>
          ))}
          <DropdownMenuSeparator />
          <DropdownMenuItem
            onClick={() => setShowDatePicker(true)}
            className="flex items-center justify-between"
          >
            {t('filters.dateRange.custom')}
            {(filters.dateRange === 'custom_dates' || filters.dateRange === 'custom_range') && (
              <Check className="w-4 h-4 ml-2" />
            )}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      {/* Date picker popover */}
      {showDatePicker && (
        <DatePickerPopover
          selectedDates={filters.customDates || []}
          onSelect={(dates) => {
            onSetDateRange('custom_dates', undefined, undefined, dates);
          }}
          onClose={() => setShowDatePicker(false)}
        />
      )}

      {/* Working directory dropdown */}
      {availableWorkingDirs.length > 1 && (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant={filters.workingDir ? 'secondary' : 'ghost'}
              size="sm"
              className="gap-1.5 max-w-[200px]"
            >
              <FolderOpen className="w-4 h-4 flex-shrink-0" />
              <span className="truncate">
                {filters.workingDir
                  ? getShortWorkingDir(filters.workingDir)
                  : t('filters.workingDir.all')}
              </span>
              <ChevronDown className="w-3.5 h-3.5 flex-shrink-0" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="max-w-[300px] max-h-[300px] overflow-y-auto">
            <DropdownMenuItem
              onClick={() => onSetWorkingDir(null)}
              className="flex items-center justify-between"
            >
              {t('filters.workingDir.all')}
              {filters.workingDir === null && <Check className="w-4 h-4 ml-2" />}
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            {availableWorkingDirs.map((dir) => (
              <DropdownMenuItem
                key={dir}
                onClick={() => onSetWorkingDir(dir)}
                className="flex items-center justify-between"
              >
                <span className="truncate" title={dir}>
                  {getShortWorkingDir(dir)}
                </span>
                {filters.workingDir === dir && (
                  <Check className="w-4 h-4 ml-2 flex-shrink-0" />
                )}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      )}

      {/* Sort dropdown */}
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="sm" className="gap-1.5">
            <ArrowUpDown className="w-4 h-4" />
            {getSortByLabel(filters.sortBy)}
            <ChevronDown className="w-3.5 h-3.5" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="min-w-[140px]">
          {(['updated_at', 'created_at', 'message_count', 'total_tokens'] as SortByType[]).map(
            (sortBy) => (
              <DropdownMenuItem
                key={sortBy}
                onClick={() => onSetSortBy(sortBy)}
                className="flex items-center justify-between"
              >
                {getSortByLabel(sortBy)}
                {filters.sortBy === sortBy && <Check className="w-4 h-4 ml-2" />}
              </DropdownMenuItem>
            )
          )}
        </DropdownMenuContent>
      </DropdownMenu>

      {/* Sort order toggle */}
      <Button
        variant="ghost"
        size="sm"
        shape="round"
        onClick={onToggleSortOrder}
        title={filters.sortOrder === 'desc' ? 'Descending' : 'Ascending'}
      >
        {filters.sortOrder === 'desc' ? (
          <ArrowDown className="w-4 h-4" />
        ) : (
          <ArrowUp className="w-4 h-4" />
        )}
      </Button>

      {/* Clear filters button */}
      {hasActiveFilters && (
        <Button
          variant="ghost"
          size="sm"
          onClick={onClearFilters}
          className="gap-1.5 text-text-muted hover:text-text-default"
        >
          <X className="w-4 h-4" />
          {t('filters.clear')}
        </Button>
      )}

      {/* Result count */}
      {hasActiveFilters && filteredCount !== totalCount && (
        <span className="text-xs text-text-muted ml-auto">
          {filteredCount} / {totalCount}
        </span>
      )}
    </div>
  );
});

export default SessionFilterBar;
