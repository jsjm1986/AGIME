// CloudSourceView - View for managing cloud data sources
import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import {
  ArrowLeft,
  Cloud,
  RefreshCw,
  Users,
  Sparkles,
  Book,
  Puzzle,
  Settings,
  Wifi,
  WifiOff,
} from 'lucide-react';
import { Button } from '../ui/button';
import type { DataSource } from './sources/types';
import { sourceManager } from './sources/sourceManager';
import { CloudServerCard } from './servers';
import { AddSourceDialog } from './dashboard/AddSourceDialog';

type ResourceType = 'skill' | 'recipe' | 'extension';

interface CloudSourceViewProps {
  source: DataSource | null;
  onBack: () => void;
  onNavigateResources?: (type: ResourceType) => void;
}

const CloudSourceView: React.FC<CloudSourceViewProps> = ({
  source,
  onBack,
  onNavigateResources,
}) => {
  const { t } = useTranslation('team');
  const [cloudSources, setCloudSources] = useState<DataSource[]>([]);
  const [activeSource, setActiveSource] = useState<DataSource | null>(source);
  const [refreshing, setRefreshing] = useState(false);
  const [showAddDialog, setShowAddDialog] = useState(false);

  // Load cloud sources
  useEffect(() => {
    const sources = sourceManager.getAllSources().filter(s => s.type === 'cloud');
    setCloudSources(sources);
    if (!activeSource && sources.length > 0) {
      setActiveSource(sources[0]);
    }
  }, []);

  const handleRefresh = async () => {
    if (!activeSource) return;
    setRefreshing(true);
    await sourceManager.checkHealth(activeSource.id);
    setCloudSources(sourceManager.getAllSources().filter(s => s.type === 'cloud'));
    setRefreshing(false);
  };

  const handleRemoveSource = async (sourceId: string) => {
    sourceManager.unregisterSource(sourceId);
    const sources = sourceManager.getAllSources().filter(s => s.type === 'cloud');
    setCloudSources(sources);
    if (activeSource?.id === sourceId) {
      setActiveSource(sources[0] || null);
    }
  };

  const handleAddSuccess = () => {
    const sources = sourceManager.getAllSources().filter(s => s.type === 'cloud');
    setCloudSources(sources);
    if (sources.length === 1) {
      setActiveSource(sources[0]);
    }
  };

  const getStatusIcon = () => {
    if (!activeSource) return <WifiOff size={16} className="text-gray-400" />;
    switch (activeSource.status) {
      case 'online':
        return <Wifi size={16} className="text-green-500" />;
      case 'connecting':
        return <RefreshCw size={16} className="text-yellow-500 animate-spin" />;
      default:
        return <WifiOff size={16} className="text-red-500" />;
    }
  };

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="p-4 border-b border-border-subtle">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <button
              onClick={onBack}
              className="p-2 rounded-lg hover:bg-background-muted text-text-muted hover:text-text-default"
            >
              <ArrowLeft size={20} />
            </button>
            <div className="p-2 rounded-lg bg-blue-100 dark:bg-blue-900/30">
              <Cloud size={24} className="text-blue-600 dark:text-blue-400" />
            </div>
            <div>
              <h1 className="text-xl font-semibold text-text-default">
                {t('cloud.title', 'Cloud Servers')}
              </h1>
              <p className="text-sm text-text-muted">
                {t('cloud.subtitle', 'Manage your cloud team servers')}
              </p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={handleRefresh}
              disabled={refreshing || !activeSource}
            >
              <RefreshCw size={14} className={`mr-1 ${refreshing ? 'animate-spin' : ''}`} />
              {t('cloud.refresh', 'Refresh')}
            </Button>
            <Button
              size="sm"
              onClick={() => setShowAddDialog(true)}
            >
              {t('cloud.addServer', 'Add Server')}
            </Button>
          </div>
        </div>
      </div>

      <div className="flex-1 flex overflow-hidden">
        {/* Sidebar - Server list */}
        <div className="w-72 border-r border-border-subtle p-4 overflow-y-auto">
          <h2 className="text-sm font-medium text-text-muted mb-3">
            {t('cloud.servers', 'Servers')} ({cloudSources.length})
          </h2>

          {cloudSources.length === 0 ? (
            <div className="text-center py-8">
              <Cloud size={32} className="mx-auto mb-2 text-text-muted opacity-50" />
              <p className="text-sm text-text-muted">
                {t('cloud.noServers', 'No cloud servers connected')}
              </p>
              <Button
                variant="outline"
                size="sm"
                className="mt-3"
                onClick={() => setShowAddDialog(true)}
              >
                {t('cloud.addFirst', 'Add your first server')}
              </Button>
            </div>
          ) : (
            <div className="space-y-2">
              {cloudSources.map((src) => (
                <CloudServerCard
                  key={src.id}
                  source={src}
                  isActive={activeSource?.id === src.id}
                  onSelect={() => setActiveSource(src)}
                  onRemove={() => handleRemoveSource(src.id)}
                  onRefresh={() => sourceManager.checkHealth(src.id)}
                />
              ))}
            </div>
          )}
        </div>

        {/* Main content */}
        <div className="flex-1 p-6 overflow-y-auto">
          {activeSource ? (
            <div className="space-y-6">
              {/* Source info */}
              <div className="p-4 rounded-lg border border-border-subtle bg-background-muted/50">
                <div className="flex items-center justify-between mb-4">
                  <div className="flex items-center gap-3">
                    {getStatusIcon()}
                    <div>
                      <h3 className="font-medium text-text-default">{activeSource.name}</h3>
                      <p className="text-xs text-text-muted">{activeSource.connection.url}</p>
                    </div>
                  </div>
                  {activeSource.userInfo?.email && (
                    <div className="text-right">
                      <p className="text-sm text-text-default">
                        {activeSource.userInfo.displayName || activeSource.userInfo.email}
                      </p>
                      <p className="text-xs text-text-muted">{activeSource.userInfo.email}</p>
                    </div>
                  )}
                </div>

                {/* Stats */}
                <div className="grid grid-cols-3 gap-4">
                  <div className="text-center p-3 rounded-lg bg-background-default">
                    <Users size={20} className="mx-auto mb-1 text-blue-500" />
                    <p className="text-lg font-semibold">{activeSource.teamsCount || 0}</p>
                    <p className="text-xs text-text-muted">{t('cloud.teams', 'Teams')}</p>
                  </div>
                  <div className="text-center p-3 rounded-lg bg-background-default">
                    <Sparkles size={20} className="mx-auto mb-1 text-purple-500" />
                    <p className="text-lg font-semibold">-</p>
                    <p className="text-xs text-text-muted">{t('cloud.skills', 'Skills')}</p>
                  </div>
                  <div className="text-center p-3 rounded-lg bg-background-default">
                    <Book size={20} className="mx-auto mb-1 text-amber-500" />
                    <p className="text-lg font-semibold">-</p>
                    <p className="text-xs text-text-muted">{t('cloud.recipes', 'Recipes')}</p>
                  </div>
                </div>
              </div>

              {/* Quick actions */}
              {onNavigateResources && (
                <div>
                  <h3 className="text-sm font-medium text-text-muted mb-3">
                    {t('cloud.browseResources', 'Browse Resources')}
                  </h3>
                  <div className="grid grid-cols-3 gap-3">
                    <button
                      onClick={() => onNavigateResources('skill')}
                      className="p-4 rounded-lg border border-border-subtle hover:border-purple-500 hover:bg-purple-50 dark:hover:bg-purple-900/10 transition-all text-left"
                    >
                      <Sparkles size={20} className="text-purple-500 mb-2" />
                      <p className="font-medium text-text-default">{t('cloud.skills', 'Skills')}</p>
                      <p className="text-xs text-text-muted">{t('cloud.browseSkills', 'Browse skills')}</p>
                    </button>
                    <button
                      onClick={() => onNavigateResources('recipe')}
                      className="p-4 rounded-lg border border-border-subtle hover:border-amber-500 hover:bg-amber-50 dark:hover:bg-amber-900/10 transition-all text-left"
                    >
                      <Book size={20} className="text-amber-500 mb-2" />
                      <p className="font-medium text-text-default">{t('cloud.recipes', 'Recipes')}</p>
                      <p className="text-xs text-text-muted">{t('cloud.browseRecipes', 'Browse recipes')}</p>
                    </button>
                    <button
                      onClick={() => onNavigateResources('extension')}
                      className="p-4 rounded-lg border border-border-subtle hover:border-blue-500 hover:bg-blue-50 dark:hover:bg-blue-900/10 transition-all text-left"
                    >
                      <Puzzle size={20} className="text-blue-500 mb-2" />
                      <p className="font-medium text-text-default">{t('cloud.extensions', 'Extensions')}</p>
                      <p className="text-xs text-text-muted">{t('cloud.browseExtensions', 'Browse extensions')}</p>
                    </button>
                  </div>
                </div>
              )}

              {/* Server settings */}
              <div>
                <h3 className="text-sm font-medium text-text-muted mb-3">
                  {t('cloud.serverSettings', 'Server Settings')}
                </h3>
                <div className="p-4 rounded-lg border border-border-subtle">
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-3">
                      <Settings size={20} className="text-text-muted" />
                      <div>
                        <p className="font-medium text-text-default">
                          {t('cloud.manageConnection', 'Manage Connection')}
                        </p>
                        <p className="text-xs text-text-muted">
                          {t('cloud.manageConnectionDesc', 'Update credentials or remove server')}
                        </p>
                      </div>
                    </div>
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => handleRemoveSource(activeSource.id)}
                    >
                      {t('cloud.remove', 'Remove')}
                    </Button>
                  </div>
                </div>
              </div>
            </div>
          ) : (
            <div className="flex flex-col items-center justify-center h-full text-center">
              <Cloud size={48} className="text-text-muted opacity-50 mb-4" />
              <h3 className="text-lg font-medium text-text-default mb-2">
                {t('cloud.noServerSelected', 'No Server Selected')}
              </h3>
              <p className="text-text-muted mb-4">
                {t('cloud.selectOrAdd', 'Select a server from the list or add a new one')}
              </p>
              <Button onClick={() => setShowAddDialog(true)}>
                {t('cloud.addServer', 'Add Server')}
              </Button>
            </div>
          )}
        </div>
      </div>

      {/* Add Source Dialog */}
      <AddSourceDialog
        isOpen={showAddDialog}
        onClose={() => setShowAddDialog(false)}
        onSuccess={handleAddSuccess}
        defaultType="cloud"
      />
    </div>
  );
};

export default CloudSourceView;
