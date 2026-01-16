import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { RefreshCw } from 'lucide-react';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { Button } from '../ui/button';
import { getSyncStatus, triggerSync as triggerSyncApi, SyncStatus } from './api';

interface SyncStatusIndicatorProps {
  teamId: string;
  onSyncComplete?: () => void;
  compact?: boolean; // 紧凑模式，只显示状态点
}

export function SyncStatusIndicator({ teamId, onSyncComplete, compact = false }: SyncStatusIndicatorProps) {
  const { t } = useTranslation('team');
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [isSyncing, setIsSyncing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Fetch sync status
  const fetchStatus = useCallback(async () => {
    try {
      const data = await getSyncStatus(teamId);
      setStatus(data);
      setError(null);
    } catch (err) {
      console.error('Failed to fetch sync status:', err);
    }
  }, [teamId]);

  // Trigger sync
  const triggerSync = async () => {
    setIsSyncing(true);
    setError(null);

    try {
      await triggerSyncApi(teamId);
      // Refresh status after sync
      await fetchStatus();
      onSyncComplete?.();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('sync.syncFailed', 'Sync failed'));
    } finally {
      setIsSyncing(false);
    }
  };

  // Poll status periodically
  useEffect(() => {
    fetchStatus();
    const interval = setInterval(fetchStatus, 30000); // Every 30 seconds
    return () => clearInterval(interval);
  }, [fetchStatus]);

  const getStatusColor = () => {
    if (isSyncing || status?.state === 'syncing') {
      return 'bg-blue-500 animate-pulse';
    }
    if (error || status?.state === 'error') {
      return 'bg-red-500';
    }
    if (status?.lastSyncAt) {
      return 'bg-green-500';
    }
    return 'bg-gray-400';
  };

  const getStatusText = () => {
    if (isSyncing || status?.state === 'syncing') {
      return t('sync.syncing', '同步中...');
    }
    if (error) {
      return error;
    }
    if (status?.state === 'error') {
      return status.errorMessage || t('sync.error', '同步错误');
    }
    if (status?.lastSyncAt) {
      const date = new Date(status.lastSyncAt);
      const now = new Date();
      const diffMs = now.getTime() - date.getTime();
      const diffMins = Math.floor(diffMs / 60000);

      if (diffMins < 1) return t('sync.justNow', '刚刚同步');
      if (diffMins < 60) return t('sync.minsAgo', '{{mins}}分钟前同步', { mins: diffMins });

      const diffHours = Math.floor(diffMins / 60);
      if (diffHours < 24) return t('sync.hoursAgo', '{{hours}}小时前同步', { hours: diffHours });

      return t('sync.lastSync', '上次同步: {{time}}', { time: date.toLocaleDateString() });
    }
    return t('sync.neverSynced', '从未同步');
  };

  if (compact) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            onClick={triggerSync}
            disabled={isSyncing || status?.state === 'syncing'}
            className="flex items-center gap-1.5 px-2 py-1 rounded hover:bg-background-muted transition-colors"
          >
            <div className={`w-2 h-2 rounded-full ${getStatusColor()}`} />
            {isSyncing && <RefreshCw className="h-3 w-3 animate-spin text-text-muted" />}
          </button>
        </TooltipTrigger>
        <TooltipContent side="bottom">
          <p className="text-xs">{getStatusText()}</p>
          <p className="text-xs text-text-muted mt-1">{t('sync.clickToSync', '点击同步')}</p>
        </TooltipContent>
      </Tooltip>
    );
  }

  return (
    <div className="flex items-center gap-1.5">
      <Tooltip>
        <TooltipTrigger asChild>
          <div className="flex items-center gap-1.5 text-sm text-text-muted">
            <div className={`w-2 h-2 rounded-full ${getStatusColor()}`} />
          </div>
        </TooltipTrigger>
        <TooltipContent side="bottom">
          <p className="text-xs">{getStatusText()}</p>
        </TooltipContent>
      </Tooltip>

      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            onClick={triggerSync}
            disabled={isSyncing || status?.state === 'syncing'}
            className="h-7 w-7 p-0"
          >
            <RefreshCw
              className={`h-4 w-4 ${isSyncing ? 'animate-spin' : ''}`}
            />
          </Button>
        </TooltipTrigger>
        <TooltipContent side="bottom">
          <p className="text-xs">{t('sync.syncNow', '立即同步')}</p>
        </TooltipContent>
      </Tooltip>
    </div>
  );
}

export default SyncStatusIndicator;
