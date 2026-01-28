// SourceList - List of data sources
import React from 'react';
import type { DataSource } from '../sources/types';
import { SourceCard } from './SourceCard';

interface SourceListProps {
  sources: DataSource[];
  onSelectSource?: (source: DataSource) => void;
  onRemoveSource?: (sourceId: string) => void;
  onRefreshSource?: (sourceId: string) => void;
  onAddSource?: () => void;
}

export const SourceList: React.FC<SourceListProps> = ({
  sources,
  onSelectSource,
  onRemoveSource,
  onRefreshSource,
  onAddSource,
}) => {
  // Group sources by type
  const localSources = sources.filter(s => s.type === 'local');
  const cloudSources = sources.filter(s => s.type === 'cloud');
  const lanSources = sources.filter(s => s.type === 'lan');

  return (
    <div className="space-y-6">
      {/* Local Sources */}
      {localSources.length > 0 && (
        <div>
          <h3 className="text-sm font-medium text-gray-500 mb-2">Local</h3>
          <div className="grid gap-3">
            {localSources.map(source => (
              <SourceCard
                key={source.id}
                source={source}
                onSelect={onSelectSource}
              />
            ))}
          </div>
        </div>
      )}

      {/* Cloud Sources */}
      <div>
        <div className="flex items-center justify-between mb-2">
          <h3 className="text-sm font-medium text-gray-500">Cloud Servers</h3>
          <button
            className="text-xs text-blue-600 hover:text-blue-800"
            onClick={onAddSource}
          >
            + Add Server
          </button>
        </div>
        {cloudSources.length > 0 ? (
          <div className="grid gap-3">
            {cloudSources.map(source => (
              <SourceCard
                key={source.id}
                source={source}
                onSelect={onSelectSource}
                onRemove={onRemoveSource}
                onRefresh={onRefreshSource}
              />
            ))}
          </div>
        ) : (
          <div className="text-sm text-gray-400 p-4 border border-dashed rounded-lg text-center">
            No cloud servers connected
          </div>
        )}
      </div>

      {/* LAN Sources */}
      <div>
        <div className="flex items-center justify-between mb-2">
          <h3 className="text-sm font-medium text-gray-500">LAN Devices</h3>
          <button
            className="text-xs text-blue-600 hover:text-blue-800"
            onClick={onAddSource}
          >
            + Connect Device
          </button>
        </div>
        {lanSources.length > 0 ? (
          <div className="grid gap-3">
            {lanSources.map(source => (
              <SourceCard
                key={source.id}
                source={source}
                onSelect={onSelectSource}
                onRemove={onRemoveSource}
                onRefresh={onRefreshSource}
              />
            ))}
          </div>
        ) : (
          <div className="text-sm text-gray-400 p-4 border border-dashed rounded-lg text-center">
            No LAN devices connected
          </div>
        )}
      </div>
    </div>
  );
};
