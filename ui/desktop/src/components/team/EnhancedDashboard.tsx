// Enhanced Dashboard Component
// Unified view of all data sources with quick actions

import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import type { DataSource, HealthStatus } from './sources/types';
import { useSourceManager } from './sources/sourceManager';

interface EnhancedDashboardProps {
  onNavigateToSource?: (sourceId: string) => void;
  onAddCloudServer?: () => void;
  onAddLANConnection?: () => void;
  className?: string;
}

export const EnhancedDashboard: React.FC<EnhancedDashboardProps> = ({
  onNavigateToSource,
  onAddCloudServer,
  onAddLANConnection,
  className = '',
}) => {
  const { t } = useTranslation();
  const sourceManager = useSourceManager();

  const [sources, setSources] = useState<DataSource[]>([]);
  const [healthMap, setHealthMap] = useState<Map<string, HealthStatus>>(new Map());
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const init = async () => {
      setLoading(true);
      await sourceManager.initialize();
      setSources(sourceManager.getAllSources());
      const health = await sourceManager.checkAllHealth();
      setHealthMap(health);
      setSources(sourceManager.getAllSources());
      setLoading(false);
    };
    init();
  }, []);

  const refreshHealth = async () => {
    const health = await sourceManager.checkAllHealth();
    setHealthMap(health);
    setSources(sourceManager.getAllSources());
  };

  const localSource = sources.find(s => s.type === 'local');
  const cloudSources = sources.filter(s => s.type === 'cloud');
  const lanSources = sources.filter(s => s.type === 'lan');

  const onlineCount = sources.filter(s => s.status === 'online').length;
  const totalTeams = sources.reduce((sum, s) => sum + (s.teamsCount || 0), 0);

  return (
    <div className={`space-y-6 ${className}`}>
      {/* Header */}
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-semibold text-gray-900 dark:text-gray-100">
          {t('team.dashboard.title', 'Team Dashboard')}
        </h2>
        <button
          onClick={refreshHealth}
          className="p-2 text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200"
          title={t('team.dashboard.refresh', 'Refresh')}
        >
          <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
              d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
          </svg>
        </button>
      </div>

      {/* Stats Overview */}
      <StatsOverview
        onlineCount={onlineCount}
        totalSources={sources.length}
        totalTeams={totalTeams}
      />

      {/* Loading */}
      {loading && (
        <div className="flex items-center justify-center py-8">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500" />
        </div>
      )}

      {!loading && (
        <>
          {/* Local Source */}
          {localSource && (
            <SourceSection
              title={t('team.dashboard.local', 'Local')}
              sources={[localSource]}
              healthMap={healthMap}
              onNavigate={onNavigateToSource}
            />
          )}

          {/* Cloud Sources */}
          <SourceSection
            title={t('team.dashboard.cloudServers', 'Cloud Servers')}
            sources={cloudSources}
            healthMap={healthMap}
            onNavigate={onNavigateToSource}
            onAdd={onAddCloudServer}
            addLabel={t('team.dashboard.addCloud', 'Add Server')}
            emptyMessage={t('team.dashboard.noCloudServers', 'No cloud servers connected')}
          />

          {/* LAN Sources */}
          <SourceSection
            title={t('team.dashboard.lanConnections', 'LAN Connections')}
            sources={lanSources}
            healthMap={healthMap}
            onNavigate={onNavigateToSource}
            onAdd={onAddLANConnection}
            addLabel={t('team.dashboard.addLAN', 'Add Connection')}
            emptyMessage={t('team.dashboard.noLANConnections', 'No LAN connections')}
          />
        </>
      )}
    </div>
  );
};

// ============================================================
// Stats Overview Component
// ============================================================

interface StatsOverviewProps {
  onlineCount: number;
  totalSources: number;
  totalTeams: number;
}

const StatsOverview: React.FC<StatsOverviewProps> = ({
  onlineCount,
  totalSources,
  totalTeams,
}) => {
  const { t } = useTranslation();

  return (
    <div className="grid grid-cols-3 gap-4">
      <div className="p-4 bg-green-50 dark:bg-green-900/20 rounded-lg">
        <div className="text-2xl font-bold text-green-600 dark:text-green-400">
          {onlineCount}/{totalSources}
        </div>
        <div className="text-sm text-green-700 dark:text-green-300">
          {t('team.dashboard.sourcesOnline', 'Sources Online')}
        </div>
      </div>
      <div className="p-4 bg-blue-50 dark:bg-blue-900/20 rounded-lg">
        <div className="text-2xl font-bold text-blue-600 dark:text-blue-400">
          {totalTeams}
        </div>
        <div className="text-sm text-blue-700 dark:text-blue-300">
          {t('team.dashboard.totalTeams', 'Total Teams')}
        </div>
      </div>
      <div className="p-4 bg-purple-50 dark:bg-purple-900/20 rounded-lg">
        <div className="text-2xl font-bold text-purple-600 dark:text-purple-400">
          {totalSources}
        </div>
        <div className="text-sm text-purple-700 dark:text-purple-300">
          {t('team.dashboard.dataSources', 'Data Sources')}
        </div>
      </div>
    </div>
  );
};

// ============================================================
// Source Section Component
// ============================================================

interface SourceSectionProps {
  title: string;
  sources: DataSource[];
  healthMap: Map<string, HealthStatus>;
  onNavigate?: (sourceId: string) => void;
  onAdd?: () => void;
  addLabel?: string;
  emptyMessage?: string;
}

const SourceSection: React.FC<SourceSectionProps> = ({
  title,
  sources,
  healthMap,
  onNavigate,
  onAdd,
  addLabel,
  emptyMessage,
}) => {
  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-lg font-medium text-gray-800 dark:text-gray-200">
          {title}
        </h3>
        {onAdd && (
          <button
            onClick={onAdd}
            className="flex items-center gap-1 px-3 py-1.5 text-sm
              text-blue-600 hover:text-blue-700
              dark:text-blue-400 dark:hover:text-blue-300"
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
            </svg>
            {addLabel}
          </button>
        )}
      </div>

      {sources.length === 0 ? (
        <div className="p-4 text-center text-gray-500 dark:text-gray-400 bg-gray-50 dark:bg-gray-800 rounded-lg">
          {emptyMessage}
        </div>
      ) : (
        <div className="grid gap-3">
          {sources.map(source => (
            <SourceCard
              key={source.id}
              source={source}
              health={healthMap.get(source.id)}
              onClick={() => onNavigate?.(source.id)}
            />
          ))}
        </div>
      )}
    </div>
  );
};

// ============================================================
// Source Card Component
// ============================================================

interface SourceCardProps {
  source: DataSource;
  health?: HealthStatus;
  onClick?: () => void;
}

const SourceCard: React.FC<SourceCardProps> = ({
  source,
  health,
  onClick,
}) => {
  const { t } = useTranslation();

  const getStatusColor = () => {
    switch (source.status) {
      case 'online':
        return 'bg-green-500';
      case 'connecting':
        return 'bg-yellow-500';
      case 'offline':
      case 'error':
        return 'bg-red-500';
    }
  };

  const getStatusText = () => {
    switch (source.status) {
      case 'online':
        return t('team.status.online', 'Online');
      case 'connecting':
        return t('team.status.connecting', 'Connecting');
      case 'offline':
        return t('team.status.offline', 'Offline');
      case 'error':
        return t('team.status.error', 'Error');
    }
  };

  return (
    <div
      onClick={onClick}
      className="p-4 border rounded-lg cursor-pointer transition-colors
        bg-white dark:bg-gray-800
        border-gray-200 dark:border-gray-700
        hover:border-blue-300 dark:hover:border-blue-600"
    >
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <span className={`w-3 h-3 rounded-full ${getStatusColor()}`} />
          <div>
            <h4 className="font-medium text-gray-900 dark:text-gray-100">
              {source.name}
            </h4>
            <p className="text-sm text-gray-500 dark:text-gray-400">
              {source.connection.url || t('team.source.local', 'Local Server')}
            </p>
          </div>
        </div>
        <div className="text-right">
          <div className="text-sm text-gray-600 dark:text-gray-300">
            {getStatusText()}
          </div>
          {source.teamsCount !== undefined && (
            <div className="text-xs text-gray-400">
              {source.teamsCount} {t('team.teams', 'teams')}
            </div>
          )}
          {health?.latencyMs && (
            <div className="text-xs text-gray-400">
              {health.latencyMs}ms
            </div>
          )}
        </div>
      </div>
      {source.lastError && (
        <div className="mt-2 text-sm text-red-500 dark:text-red-400">
          {source.lastError}
        </div>
      )}
    </div>
  );
};

export default EnhancedDashboard;
