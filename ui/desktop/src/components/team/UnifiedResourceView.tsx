// Unified Resource View Component
// Displays resources from all data sources with filtering and caching

import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Download, Eye, RefreshCw, CheckCircle, Cloud, AlertTriangle } from 'lucide-react';
import type {
  DataSource,
  SourcedResource,
  ResourceFilters,
  AggregatedQueryResult,
} from './sources/types';
import type { SharedSkill, SharedRecipe, SharedExtension } from './types';
import { useSourceManager } from './sources/sourceManager';
import { SourceFilter } from './SourceFilter';
import { Button } from '../ui/button';
import { useDebounce } from './hooks';

// Resource type for the view
type ResourceType = 'skill' | 'recipe' | 'extension';
type AnyResource = SharedSkill | SharedRecipe | SharedExtension;

interface UnifiedResourceViewProps {
  resourceType: ResourceType;
  teamId?: string;
  onResourceSelect?: (resource: SourcedResource<AnyResource>) => void;
  onInstall?: (resource: SourcedResource<AnyResource>) => void;
  onViewDetails?: (resource: SourcedResource<AnyResource>) => void;
  className?: string;
}

export const UnifiedResourceView: React.FC<UnifiedResourceViewProps> = ({
  resourceType,
  teamId,
  onResourceSelect,
  onInstall,
  onViewDetails,
  className = '',
}) => {
  const { t } = useTranslation('team');
  const sourceManager = useSourceManager();

  const [sources, setSources] = useState<DataSource[]>([]);
  const [resources, setResources] = useState<SourcedResource<AnyResource>[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [errors, setErrors] = useState<Array<{ sourceId: string; sourceName: string; error: string }>>([]);

  const [filters, setFilters] = useState<ResourceFilters>({
    sources: 'all',
    search: '',
    tags: [],
    teamId,
  });

  // Debounce search input (300ms delay)
  const debouncedSearch = useDebounce(filters.search, 300);

  // Initialize sources
  useEffect(() => {
    const init = async () => {
      await sourceManager.initialize();
      setSources(sourceManager.getAllSources());
    };
    init();
  }, []);

  // Fetch resources (uses cache by default)
  const fetchResources = useCallback(async (forceRefresh = false) => {
    if (forceRefresh) {
      setRefreshing(true);
    } else {
      setLoading(true);
    }
    setErrors([]);

    // Use debounced search for actual query
    const queryFilters = { ...filters, search: debouncedSearch };

    try {
      let result: AggregatedQueryResult<AnyResource>;

      switch (resourceType) {
        case 'skill':
          result = await sourceManager.aggregateSkills(queryFilters, undefined, { forceRefresh });
          break;
        case 'recipe':
          result = await sourceManager.aggregateRecipes(queryFilters, undefined, { forceRefresh });
          break;
        case 'extension':
          result = await sourceManager.aggregateExtensions(queryFilters, undefined, { forceRefresh });
          break;
      }

      setResources(result.resources);
      setErrors(result.errors);
    } catch (error) {
      console.error('Failed to fetch resources:', error);
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, [resourceType, filters.sources, filters.tags, filters.teamId, debouncedSearch, sourceManager]);

  // Handle manual refresh
  const handleRefresh = useCallback(() => {
    fetchResources(true);
  }, [fetchResources]);

  useEffect(() => {
    fetchResources();
  }, [fetchResources]);

  // Filter by search
  const filteredResources = resources.filter(item => {
    if (!filters.search) return true;
    const searchLower = filters.search.toLowerCase();
    const resource = item.resource as AnyResource;
    return (
      resource.name.toLowerCase().includes(searchLower) ||
      resource.description?.toLowerCase().includes(searchLower)
    );
  });

  const handleSourceSelectionChange = (selection: string[] | 'all') => {
    setFilters(prev => ({ ...prev, sources: selection }));
  };

  const handleSearchChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setFilters(prev => ({ ...prev, search: e.target.value }));
  };

  return (
    <div className={`flex flex-col gap-4 ${className}`}>
      {/* Filters */}
      <div className="flex flex-col gap-3">
        {/* Source filter */}
        <SourceFilter
          sources={sources}
          selectedSources={filters.sources}
          onSelectionChange={handleSourceSelectionChange}
        />

        {/* Search and Refresh */}
        <div className="flex gap-2">
          <div className="relative flex-1">
            <input
              type="text"
              value={filters.search || ''}
              onChange={handleSearchChange}
              placeholder={t('resources.search', 'Search resources...')}
              className="w-full px-4 py-2 pl-10 border rounded-lg
                bg-white dark:bg-gray-800
                border-gray-300 dark:border-gray-600
                text-gray-900 dark:text-gray-100
                placeholder-gray-500 dark:placeholder-gray-400
                focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            />
            <svg
              className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
            >
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
                d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
            </svg>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={handleRefresh}
            disabled={refreshing}
            title={t('resources.refresh', 'Refresh')}
          >
            <RefreshCw className={`h-4 w-4 ${refreshing ? 'animate-spin' : ''}`} />
          </Button>
        </div>
      </div>

      {/* Errors */}
      {errors.length > 0 && (
        <div className="p-3 bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-800 rounded-lg">
          <p className="text-sm text-yellow-700 dark:text-yellow-300">
            {t('resources.someSourcesFailed', 'Some sources failed to load:')}
          </p>
          <ul className="mt-1 text-sm text-yellow-600 dark:text-yellow-400">
            {errors.map((err, i) => (
              <li key={i}>{err.sourceName}: {err.error}</li>
            ))}
          </ul>
        </div>
      )}

      {/* Loading */}
      {loading && (
        <div className="flex items-center justify-center py-8">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500" />
        </div>
      )}

      {/* Resource list */}
      {!loading && (
        <ResourceList
          resources={filteredResources}
          resourceType={resourceType}
          onSelect={onResourceSelect}
          onInstall={onInstall}
          onViewDetails={onViewDetails}
        />
      )}
    </div>
  );
};

// ============================================================
// Resource List Component
// ============================================================

interface ResourceListProps {
  resources: SourcedResource<AnyResource>[];
  resourceType: ResourceType;
  onSelect?: (resource: SourcedResource<AnyResource>) => void;
  onInstall?: (resource: SourcedResource<AnyResource>) => void;
  onViewDetails?: (resource: SourcedResource<AnyResource>) => void;
}

const ResourceList: React.FC<ResourceListProps> = ({
  resources,
  resourceType,
  onSelect,
  onInstall,
  onViewDetails,
}) => {
  const { t } = useTranslation('team');

  if (resources.length === 0) {
    return (
      <div className="text-center py-8 text-gray-500 dark:text-gray-400">
        {t('resources.noResults', 'No resources found')}
      </div>
    );
  }

  return (
    <div className="grid gap-3">
      {resources.map((item, index) => (
        <ResourceCard
          key={`${item.source.id}-${item.resource.id}-${index}`}
          sourcedResource={item}
          resourceType={resourceType}
          onClick={() => onSelect?.(item)}
          onInstall={() => onInstall?.(item)}
          onViewDetails={() => onViewDetails?.(item)}
        />
      ))}
    </div>
  );
};

// ============================================================
// Resource Card Component
// ============================================================

interface ResourceCardProps {
  sourcedResource: SourcedResource<AnyResource>;
  resourceType: ResourceType;
  onClick?: () => void;
  onInstall?: () => void;
  onViewDetails?: () => void;
}

const ResourceCard: React.FC<ResourceCardProps> = ({
  sourcedResource,
  resourceType,
  onClick,
  onInstall,
  onViewDetails,
}) => {
  const { t } = useTranslation('team');
  const { source, resource, syncStatus } = sourcedResource;

  const getSourceBadgeColor = (type: DataSource['type']) => {
    switch (type) {
      case 'local':
        return 'bg-gray-100 text-gray-700 dark:bg-gray-700 dark:text-gray-300';
      case 'cloud':
        return 'bg-blue-100 text-blue-700 dark:bg-blue-900 dark:text-blue-300';
      case 'lan':
        return 'bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300';
    }
  };

  const getSyncStatusIcon = () => {
    switch (syncStatus) {
      case 'synced':
        return <CheckCircle size={14} className="text-green-500" />;
      case 'remote-only':
        return <Cloud size={14} className="text-blue-500" />;
      case 'conflict':
        return <AlertTriangle size={14} className="text-yellow-500" />;
      default:
        return null;
    }
  };

  const getResourceTypeLabel = () => {
    switch (resourceType) {
      case 'skill':
        return t('resources.skill', 'Skill');
      case 'recipe':
        return t('resources.recipe', 'Recipe');
      case 'extension':
        return t('resources.extension', 'Extension');
    }
  };

  const canInstall = syncStatus === 'remote-only' || source.type !== 'local';

  const handleInstallClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    onInstall?.();
  };

  const handleViewDetailsClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    onViewDetails?.();
  };

  return (
    <div
      onClick={onClick}
      className="p-4 border rounded-lg cursor-pointer transition-colors
        bg-background-default
        border-border-subtle
        hover:border-teal-300 dark:hover:border-teal-600
        hover:bg-background-muted"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <h3 className="font-medium text-text-default truncate">
              {resource.name}
            </h3>
            {getSyncStatusIcon()}
          </div>
          {resource.description && (
            <p className="mt-1 text-sm text-text-muted line-clamp-2">
              {resource.description}
            </p>
          )}
          <div className="mt-2 flex items-center gap-2 flex-wrap">
            <span className={`px-2 py-0.5 text-xs rounded-full ${getSourceBadgeColor(source.type)}`}>
              {source.name}
            </span>
            <span className="text-xs text-text-muted">
              {getResourceTypeLabel()}
            </span>
            <span className="text-xs text-text-muted">
              v{resource.version}
            </span>
          </div>
        </div>

        {/* Action buttons */}
        <div className="flex items-center gap-2 flex-shrink-0">
          {onViewDetails && (
            <Button
              variant="ghost"
              size="sm"
              onClick={handleViewDetailsClick}
              className="h-8 px-2"
            >
              <Eye size={14} className="mr-1" />
              {t('resources.details', 'Details')}
            </Button>
          )}
          {canInstall && onInstall && (
            <Button
              variant="default"
              size="sm"
              onClick={handleInstallClick}
              className="h-8 px-3"
            >
              <Download size={14} className="mr-1" />
              {t('resources.install', 'Install')}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
};

export default UnifiedResourceView;
