// DashboardView - Main dashboard view component
import React, { useState, useEffect } from 'react';
import type { DataSource } from '../sources/types';
import { sourceManager } from '../sources/sourceManager';
import { SourceList } from './SourceList';
import { StatsOverview } from './StatsOverview';
import { AddSourceDialog } from './AddSourceDialog';

interface DashboardViewProps {
  onSelectSource?: (source: DataSource) => void;
}

export const DashboardView: React.FC<DashboardViewProps> = ({
  onSelectSource,
}) => {
  const [sources, setSources] = useState<DataSource[]>([]);
  const [showAddDialog, setShowAddDialog] = useState(false);
  const [loading, setLoading] = useState(true);

  // Load sources on mount
  useEffect(() => {
    const loadSources = async () => {
      await sourceManager.initialize();
      setSources(sourceManager.getAllSources());
      setLoading(false);
    };
    loadSources();
  }, []);

  const handleRefreshSource = async (sourceId: string) => {
    await sourceManager.checkHealth(sourceId);
    setSources(sourceManager.getAllSources());
  };

  const handleRemoveSource = (sourceId: string) => {
    if (confirm('Remove this data source?')) {
      sourceManager.unregisterSource(sourceId);
      setSources(sourceManager.getAllSources());
    }
  };

  const handleAddSuccess = () => {
    setSources(sourceManager.getAllSources());
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-gray-500">Loading...</div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Stats Overview */}
      <StatsOverview sources={sources} />

      {/* Source List */}
      <div className="bg-white rounded-lg p-4">
        <h2 className="text-lg font-semibold mb-4">Data Sources</h2>
        <SourceList
          sources={sources}
          onSelectSource={onSelectSource}
          onRemoveSource={handleRemoveSource}
          onRefreshSource={handleRefreshSource}
          onAddSource={() => setShowAddDialog(true)}
        />
      </div>

      {/* Add Source Dialog */}
      <AddSourceDialog
        isOpen={showAddDialog}
        onClose={() => setShowAddDialog(false)}
        onSuccess={handleAddSuccess}
      />
    </div>
  );
};
