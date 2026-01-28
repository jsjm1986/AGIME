// SourceCard - Unified data source card component
import React from 'react';
import type { DataSource } from '../sources/types';

interface SourceCardProps {
  source: DataSource;
  onSelect?: (source: DataSource) => void;
  onRemove?: (sourceId: string) => void;
  onRefresh?: (sourceId: string) => void;
}

export const SourceCard: React.FC<SourceCardProps> = ({
  source,
  onSelect,
  onRemove,
  onRefresh,
}) => {
  const getStatusColor = () => {
    switch (source.status) {
      case 'online':
        return 'bg-green-500';
      case 'connecting':
        return 'bg-yellow-500';
      case 'error':
        return 'bg-red-500';
      default:
        return 'bg-gray-500';
    }
  };

  const getTypeIcon = () => {
    switch (source.type) {
      case 'local':
        return '💻';
      case 'cloud':
        return '☁️';
      case 'lan':
        return '🔗';
      default:
        return '📦';
    }
  };

  const getTypeLabel = () => {
    switch (source.type) {
      case 'local':
        return 'Local';
      case 'cloud':
        return 'Cloud';
      case 'lan':
        return 'LAN';
      default:
        return 'Unknown';
    }
  };

  return (
    <div
      className="border rounded-lg p-4 hover:shadow-md transition-shadow cursor-pointer"
      onClick={() => onSelect?.(source)}
    >
      <div className="flex items-start justify-between">
        <div className="flex items-center gap-2">
          <span className="text-2xl">{getTypeIcon()}</span>
          <div>
            <h3 className="font-medium">{source.name}</h3>
            <span className="text-xs text-gray-500">{getTypeLabel()}</span>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <span className={`w-2 h-2 rounded-full ${getStatusColor()}`} />
          <span className="text-xs text-gray-500 capitalize">{source.status}</span>
        </div>
      </div>

      <div className="mt-3 text-sm text-gray-600">
        {source.teamsCount !== undefined && (
          <div>Teams: {source.teamsCount}</div>
        )}
        {source.lastSyncedAt && (
          <div className="text-xs text-gray-400">
            Last sync: {new Date(source.lastSyncedAt).toLocaleString()}
          </div>
        )}
        {source.lastError && (
          <div className="text-xs text-red-500 mt-1">{source.lastError}</div>
        )}
      </div>

      {source.type !== 'local' && (
        <div className="mt-3 flex gap-2">
          <button
            className="text-xs px-2 py-1 bg-gray-100 rounded hover:bg-gray-200"
            onClick={(e) => {
              e.stopPropagation();
              onRefresh?.(source.id);
            }}
          >
            Refresh
          </button>
          <button
            className="text-xs px-2 py-1 bg-red-50 text-red-600 rounded hover:bg-red-100"
            onClick={(e) => {
              e.stopPropagation();
              onRemove?.(source.id);
            }}
          >
            Remove
          </button>
        </div>
      )}
    </div>
  );
};
