// CloudSourceView - View for managing cloud data sources (Responsive Grid Layout)
import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import {
  ArrowLeft,
  Cloud,
  RefreshCw,
  Users,
  Sparkles,
  Book,
  MoreVertical,
  Trash2,
  Wifi,
  WifiOff,
  Plus,
  Server,
  Globe,
  Settings,
  ExternalLink,
} from 'lucide-react';
import { Button } from '../ui/button';
import type { DataSource } from './sources/types';
import { sourceManager } from './sources/sourceManager';
import { AddSourceDialog } from './dashboard/AddSourceDialog';
import { ServerSettingsDialog } from './dashboard/ServerSettingsDialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';

type ResourceType = 'skill' | 'recipe' | 'extension';

interface CloudSourceViewProps {
  source: DataSource | null;
  onBack: () => void;
  onNavigateResources?: (type: ResourceType) => void;
  onNavigateTeams?: () => void;
}

const CloudSourceView: React.FC<CloudSourceViewProps> = ({
  onBack,
  onNavigateTeams,
}) => {
  const { t } = useTranslation('team');
  const [cloudSources, setCloudSources] = useState<DataSource[]>([]);
  const [refreshingId, setRefreshingId] = useState<string | null>(null);
  const [showAddDialog, setShowAddDialog] = useState(false);
  const [showSettingsDialog, setShowSettingsDialog] = useState(false);
  const [settingsSource, setSettingsSource] = useState<DataSource | null>(null);

  // Load cloud sources
  useEffect(() => {
    const sources = sourceManager.getAllSources().filter(s => s.type === 'cloud');
    setCloudSources(sources);
  }, []);

  const handleRefreshAll = async () => {
    for (const src of cloudSources) {
      setRefreshingId(src.id);
      await sourceManager.checkHealth(src.id);
    }
    setCloudSources(sourceManager.getAllSources().filter(s => s.type === 'cloud'));
    setRefreshingId(null);
  };

  const handleRefreshSource = async (sourceId: string) => {
    setRefreshingId(sourceId);
    await sourceManager.checkHealth(sourceId);
    setCloudSources(sourceManager.getAllSources().filter(s => s.type === 'cloud'));
    setRefreshingId(null);
  };

  const handleRemoveSource = async (sourceId: string) => {
    sourceManager.unregisterSource(sourceId);
    setCloudSources(sourceManager.getAllSources().filter(s => s.type === 'cloud'));
  };

  const handleAddSuccess = () => {
    setCloudSources(sourceManager.getAllSources().filter(s => s.type === 'cloud'));
  };

  const handleNavigateToTeams = (src: DataSource) => {
    sourceManager.setActiveSource(src.id);
    onNavigateTeams?.();
  };

  const handleOpenSettings = (src: DataSource) => {
    setSettingsSource(src);
    setShowSettingsDialog(true);
  };

  const handleSettingsSave = () => {
    setCloudSources(sourceManager.getAllSources().filter(s => s.type === 'cloud'));
  };

  // Calculate summary stats
  const totalTeams = cloudSources.reduce((sum, s) => sum + (s.teamsCount || 0), 0);
  const onlineCount = cloudSources.filter(s => s.status === 'online').length;

  return (
    <div className="flex flex-col h-full bg-background-default">
      {/* Header */}
      <div className="shrink-0 px-6 py-4 border-b border-border-subtle bg-background-card">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-4">
            <button
              onClick={onBack}
              className="p-2 -ml-2 rounded-lg hover:bg-background-muted text-text-muted hover:text-text-default transition-colors"
            >
              <ArrowLeft size={20} />
            </button>
            <div className="flex items-center gap-3">
              <div className="p-2.5 rounded-xl bg-gradient-to-br from-blue-500 to-blue-600 shadow-lg shadow-blue-500/20">
                <Cloud size={22} className="text-white" />
              </div>
              <div>
                <h1 className="text-xl font-semibold text-text-default">
                  {t('cloud.title', '云服务器')}
                </h1>
                <p className="text-sm text-text-muted">
                  {t('cloud.subtitle', '管理你的云服务器')}
                </p>
              </div>
            </div>
          </div>
          <div className="flex items-center gap-3">
            {cloudSources.length > 0 && (
              <Button
                variant="outline"
                size="sm"
                onClick={handleRefreshAll}
                disabled={refreshingId !== null}
                className="h-9"
              >
                <RefreshCw size={14} className={`mr-2 ${refreshingId ? 'animate-spin' : ''}`} />
                {t('cloud.refresh', '刷新全部')}
              </Button>
            )}
            <Button
              size="sm"
              onClick={() => setShowAddDialog(true)}
              className="h-9 bg-blue-600 hover:bg-blue-700"
            >
              <Plus size={14} className="mr-2" />
              {t('cloud.addServer', '添加服务器')}
            </Button>
          </div>
        </div>
      </div>

      {/* Main Content */}
      <div className="flex-1 overflow-y-auto">
        {cloudSources.length === 0 ? (
          <EmptyState onAdd={() => setShowAddDialog(true)} />
        ) : (
          <div className="p-6">
            {/* Summary Stats Bar */}
            <div className="mb-6 p-4 rounded-xl bg-gradient-to-r from-blue-50 to-indigo-50 dark:from-blue-900/20 dark:to-indigo-900/20 border border-blue-100 dark:border-blue-800/30">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-8">
                  <div className="flex items-center gap-3">
                    <div className="p-2 rounded-lg bg-blue-100 dark:bg-blue-900/40">
                      <Server size={18} className="text-blue-600 dark:text-blue-400" />
                    </div>
                    <div>
                      <p className="text-2xl font-bold text-text-default">{cloudSources.length}</p>
                      <p className="text-xs text-text-muted">{t('cloud.servers', '服务器')}</p>
                    </div>
                  </div>
                  <div className="w-px h-10 bg-blue-200 dark:bg-blue-800" />
                  <div className="flex items-center gap-3">
                    <div className="p-2 rounded-lg bg-green-100 dark:bg-green-900/40">
                      <Wifi size={18} className="text-green-600 dark:text-green-400" />
                    </div>
                    <div>
                      <p className="text-2xl font-bold text-text-default">{onlineCount}</p>
                      <p className="text-xs text-text-muted">{t('server.online', '在线')}</p>
                    </div>
                  </div>
                  <div className="w-px h-10 bg-blue-200 dark:bg-blue-800" />
                  <div className="flex items-center gap-3">
                    <div className="p-2 rounded-lg bg-purple-100 dark:bg-purple-900/40">
                      <Users size={18} className="text-purple-600 dark:text-purple-400" />
                    </div>
                    <div>
                      <p className="text-2xl font-bold text-text-default">{totalTeams}</p>
                      <p className="text-xs text-text-muted">{t('cloud.teams', '团队')}</p>
                    </div>
                  </div>
                </div>
              </div>
            </div>

            {/* Server Grid */}
            <div className="grid grid-cols-1 lg:grid-cols-2 xl:grid-cols-3 gap-5">
              {cloudSources.map((src) => (
                <ServerCard
                  key={src.id}
                  source={src}
                  isRefreshing={refreshingId === src.id}
                  onRefresh={() => handleRefreshSource(src.id)}
                  onRemove={() => handleRemoveSource(src.id)}
                  onNavigateTeams={() => handleNavigateToTeams(src)}
                  onOpenSettings={() => handleOpenSettings(src)}
                />
              ))}

              {/* Add Server Card */}
              <AddServerCard onClick={() => setShowAddDialog(true)} />
            </div>
          </div>
        )}
      </div>

      {/* Add Source Dialog */}
      <AddSourceDialog
        isOpen={showAddDialog}
        onClose={() => setShowAddDialog(false)}
        onSuccess={handleAddSuccess}
        defaultType="cloud"
      />

      {/* Server Settings Dialog */}
      <ServerSettingsDialog
        isOpen={showSettingsDialog}
        source={settingsSource}
        onClose={() => {
          setShowSettingsDialog(false);
          setSettingsSource(null);
        }}
        onSave={handleSettingsSave}
      />
    </div>
  );
};

// Empty State Component
const EmptyState: React.FC<{ onAdd: () => void }> = ({ onAdd }) => {
  const { t } = useTranslation('team');

  return (
    <div className="flex flex-col items-center justify-center h-full p-8">
      <div className="max-w-md text-center">
        <div className="mb-6 p-6 rounded-full bg-gradient-to-br from-blue-100 to-indigo-100 dark:from-blue-900/30 dark:to-indigo-900/30 inline-block">
          <Cloud size={48} className="text-blue-500" />
        </div>
        <h2 className="text-2xl font-semibold text-text-default mb-3">
          {t('cloud.noServers', '暂无连接的服务器')}
        </h2>
        <p className="text-text-muted mb-8 leading-relaxed">
          {t('server.noServersDescription', '添加云服务器与团队跨地域协作')}
        </p>

        {/* Feature highlights */}
        <div className="grid grid-cols-3 gap-4 mb-8">
          <div className="p-4 rounded-xl bg-background-card border border-border-subtle">
            <Globe size={24} className="text-blue-500 mx-auto mb-2" />
            <p className="text-xs text-text-muted">{t('mode.cloud.feature1', '跨地域团队')}</p>
          </div>
          <div className="p-4 rounded-xl bg-background-card border border-border-subtle">
            <Users size={24} className="text-purple-500 mx-auto mb-2" />
            <p className="text-xs text-text-muted">{t('mode.cloud.feature2', '正式的团队管理')}</p>
          </div>
          <div className="p-4 rounded-xl bg-background-card border border-border-subtle">
            <Sparkles size={24} className="text-amber-500 mx-auto mb-2" />
            <p className="text-xs text-text-muted">{t('mode.cloud.feature3', '资源集中管理')}</p>
          </div>
        </div>

        <Button
          size="lg"
          onClick={onAdd}
          className="bg-blue-600 hover:bg-blue-700 px-8"
        >
          <Plus size={18} className="mr-2" />
          {t('cloud.addFirst', '添加第一个服务器')}
        </Button>
      </div>
    </div>
  );
};

// Server Card Component
interface ServerCardProps {
  source: DataSource;
  isRefreshing: boolean;
  onRefresh: () => void;
  onRemove: () => void;
  onNavigateTeams: () => void;
  onOpenSettings: () => void;
}

const ServerCard: React.FC<ServerCardProps> = ({
  source,
  isRefreshing,
  onRefresh,
  onRemove,
  onNavigateTeams,
  onOpenSettings,
}) => {
  const { t } = useTranslation('team');
  const isOnline = source.status === 'online';
  const isConnecting = source.status === 'connecting';

  return (
    <div
      className={`
        group relative rounded-xl border bg-background-card overflow-hidden
        transition-all duration-200 hover:shadow-lg
        ${isOnline
          ? 'border-border-subtle hover:border-blue-300 dark:hover:border-blue-700'
          : 'border-border-subtle opacity-75 hover:opacity-100'
        }
      `}
    >
      {/* Status indicator line */}
      <div className={`absolute top-0 left-0 right-0 h-1 ${
        isOnline ? 'bg-gradient-to-r from-green-400 to-green-500' :
        isConnecting ? 'bg-gradient-to-r from-yellow-400 to-yellow-500' :
        'bg-gradient-to-r from-gray-300 to-gray-400 dark:from-gray-600 dark:to-gray-700'
      }`} />

      {/* Card Header */}
      <div className="p-5">
        <div className="flex items-start justify-between mb-4">
          <div className="flex items-center gap-3">
            <div className={`
              p-2.5 rounded-xl transition-colors
              ${isOnline
                ? 'bg-blue-100 dark:bg-blue-900/40'
                : 'bg-gray-100 dark:bg-gray-800'
              }
            `}>
              <Cloud size={22} className={isOnline ? 'text-blue-600 dark:text-blue-400' : 'text-gray-400'} />
            </div>
            <div className="min-w-0 flex-1">
              <h3 className="font-semibold text-text-default truncate">{source.name}</h3>
              <p className="text-xs text-text-muted truncate">{source.connection.url}</p>
            </div>
          </div>

          {/* Status Badge & Menu */}
          <div className="flex items-center gap-2">
            <StatusBadge status={source.status} />
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button className="p-1.5 rounded-lg hover:bg-background-muted text-text-muted opacity-0 group-hover:opacity-100 transition-opacity">
                  <MoreVertical size={16} />
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={onRefresh} disabled={isRefreshing}>
                  <RefreshCw size={14} className={isRefreshing ? 'animate-spin' : ''} />
                  {t('server.refresh', '刷新')}
                </DropdownMenuItem>
                <DropdownMenuItem onClick={onOpenSettings}>
                  <Settings size={14} />
                  {t('server.settings', '设置')}
                </DropdownMenuItem>
                <DropdownMenuItem onClick={onRemove} className="text-red-600 dark:text-red-400">
                  <Trash2 size={14} />
                  {t('server.remove', '移除')}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>

        {/* Stats Grid */}
        <div className="grid grid-cols-3 gap-3 mb-4">
          <StatItem
            icon={<Users size={16} />}
            value={source.teamsCount || 0}
            label={t('cloud.teams', '团队')}
            color="blue"
          />
          <StatItem
            icon={<Sparkles size={16} />}
            value="-"
            label={t('cloud.skills', '技能')}
            color="purple"
          />
          <StatItem
            icon={<Book size={16} />}
            value="-"
            label={t('cloud.recipes', '预设任务')}
            color="amber"
          />
        </div>

        {/* Action Button */}
        <Button
          variant="default"
          size="sm"
          onClick={onNavigateTeams}
          disabled={!isOnline}
          className="w-full bg-blue-600 hover:bg-blue-700 disabled:bg-gray-300 dark:disabled:bg-gray-700"
        >
          <ExternalLink size={14} className="mr-2" />
          {t('cloud.enterTeams', '进入团队')}
        </Button>
      </div>
    </div>
  );
};

// Status Badge Component
const StatusBadge: React.FC<{ status: DataSource['status'] }> = ({ status }) => {
  const { t } = useTranslation('team');

  switch (status) {
    case 'online':
      return (
        <span className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-400">
          <span className="w-1.5 h-1.5 rounded-full bg-green-500 animate-pulse" />
          {t('server.online', '在线')}
        </span>
      );
    case 'connecting':
      return (
        <span className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium bg-yellow-100 text-yellow-700 dark:bg-yellow-900/40 dark:text-yellow-400">
          <RefreshCw size={10} className="animate-spin" />
          {t('server.connecting', '连接中...')}
        </span>
      );
    default:
      return (
        <span className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-400">
          <WifiOff size={10} />
          {t('server.offline', '离线')}
        </span>
      );
  }
};

// Stat Item Component
interface StatItemProps {
  icon: React.ReactNode;
  value: number | string;
  label: string;
  color: 'blue' | 'purple' | 'amber' | 'green';
}

const StatItem: React.FC<StatItemProps> = ({ icon, value, label, color }) => {
  const colorClasses = {
    blue: 'text-blue-500',
    purple: 'text-purple-500',
    amber: 'text-amber-500',
    green: 'text-green-500',
  };

  return (
    <div className="p-3 rounded-lg bg-background-muted/50 text-center">
      <div className={`${colorClasses[color]} flex justify-center mb-1`}>{icon}</div>
      <p className="text-lg font-semibold text-text-default">{value}</p>
      <p className="text-xs text-text-muted truncate">{label}</p>
    </div>
  );
};

// Add Server Card Component
const AddServerCard: React.FC<{ onClick: () => void }> = ({ onClick }) => {
  const { t } = useTranslation('team');

  return (
    <button
      onClick={onClick}
      className="
        flex flex-col items-center justify-center
        min-h-[220px] rounded-xl border-2 border-dashed border-border-subtle
        bg-background-muted/30 hover:bg-background-muted/50
        hover:border-blue-300 dark:hover:border-blue-700
        transition-all duration-200 group
      "
    >
      <div className="p-3 rounded-full bg-blue-100 dark:bg-blue-900/30 mb-3 group-hover:scale-110 transition-transform">
        <Plus size={24} className="text-blue-500" />
      </div>
      <p className="text-sm font-medium text-text-muted group-hover:text-blue-600 dark:group-hover:text-blue-400">
        {t('cloud.addServer', '添加服务器')}
      </p>
    </button>
  );
};

export default CloudSourceView;
