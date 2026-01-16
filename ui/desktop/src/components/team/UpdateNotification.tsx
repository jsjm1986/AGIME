import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Bell, Download } from 'lucide-react';
import { Button } from '../ui/button';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  DialogDescription,
} from '../ui/dialog';

interface UpdateInfo {
  resourceId: string;
  resourceType: 'skill' | 'recipe' | 'extension';
  resourceName: string;
  currentVersion: string;
  latestVersion: string;
  hasUpdate: boolean;
}

interface UpdateNotificationProps {
  /** Auto-check for updates on mount */
  autoCheck?: boolean;
  /** Check interval in milliseconds (default: 5 minutes) */
  checkInterval?: number;
  /** Callback when updates are installed */
  onUpdatesInstalled?: () => void;
}

export function UpdateNotification({
  autoCheck = true,
  checkInterval = 5 * 60 * 1000,
  onUpdatesInstalled,
}: UpdateNotificationProps) {
  const { t } = useTranslation('team');
  const [updates, setUpdates] = useState<UpdateInfo[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [showDialog, setShowDialog] = useState(false);
  const [installingIds, setInstallingIds] = useState<Set<string>>(new Set());

  // Check for updates
  const checkUpdates = async () => {
    setIsLoading(true);
    try {
      // First get installed resources
      const installedResponse = await fetch('/api/team/resources/installed');
      if (!installedResponse.ok) {
        throw new Error('Failed to fetch installed resources');
      }
      const installedData = await installedResponse.json();
      const resourceIds = installedData.resources.map(
        (r: { resourceId: string }) => r.resourceId
      );

      if (resourceIds.length === 0) {
        setUpdates([]);
        return;
      }

      // Check for updates
      const response = await fetch('/api/team/resources/check-updates', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ resourceIds }),
      });

      if (response.ok) {
        const data = await response.json();
        setUpdates(data.updates.filter((u: UpdateInfo) => u.hasUpdate));
      }
    } catch (err) {
      console.error('Failed to check updates:', err);
    } finally {
      setIsLoading(false);
    }
  };

  // Install a single update
  const installUpdate = async (update: UpdateInfo) => {
    setInstallingIds((prev) => new Set(prev).add(update.resourceId));
    try {
      const endpoint = `/api/team/${update.resourceType}s/${update.resourceId}/install`;
      const response = await fetch(endpoint, { method: 'POST' });

      if (response.ok) {
        // Remove from updates list
        setUpdates((prev) =>
          prev.filter((u) => u.resourceId !== update.resourceId)
        );
        onUpdatesInstalled?.();
      }
    } catch (err) {
      console.error('Failed to install update:', err);
    } finally {
      setInstallingIds((prev) => {
        const next = new Set(prev);
        next.delete(update.resourceId);
        return next;
      });
    }
  };

  // Install all updates
  const installAllUpdates = async () => {
    const resources = updates.map((u) => ({
      resourceType: u.resourceType,
      id: u.resourceId,
    }));

    try {
      const response = await fetch('/api/team/resources/batch-install', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ resources }),
      });

      if (response.ok) {
        setUpdates([]);
        setShowDialog(false);
        onUpdatesInstalled?.();
      }
    } catch (err) {
      console.error('Failed to install updates:', err);
    }
  };

  // Auto-check on mount and interval
  useEffect(() => {
    if (!autoCheck) {
      return;
    }
    checkUpdates();
    const interval = setInterval(checkUpdates, checkInterval);
    return () => clearInterval(interval);
  }, [autoCheck, checkInterval]);

  const updateCount = updates.length;

  if (updateCount === 0 && !isLoading) {
    return null;
  }

  const getResourceTypeLabel = (type: string) => {
    switch (type) {
      case 'skill':
        return t('resources.skill', 'Skill');
      case 'recipe':
        return t('resources.recipe', 'Recipe');
      case 'extension':
        return t('resources.extension', 'Extension');
      default:
        return type;
    }
  };

  return (
    <>
      {/* Notification Badge */}
      {updateCount > 0 && (
        <button
          onClick={() => setShowDialog(true)}
          className="relative p-2 rounded-lg hover:bg-background-muted transition-colors"
        >
          <Bell className="h-5 w-5 text-blue-500" />
          <span className="absolute -top-1 -right-1 bg-red-500 text-white text-xs rounded-full h-5 w-5 flex items-center justify-center font-medium">
            {updateCount}
          </span>
        </button>
      )}

      {/* Updates Dialog */}
      <Dialog open={showDialog} onOpenChange={setShowDialog}>
        <DialogContent className="sm:max-w-[500px]">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Download className="h-5 w-5 text-blue-500" />
              {t('updates.title', 'Updates Available')}
            </DialogTitle>
            <DialogDescription>
              {t('updates.description', '{{count}} resource(s) have updates available', {
                count: updateCount,
              })}
            </DialogDescription>
          </DialogHeader>

          <div className="py-4">
            <div className="space-y-2 max-h-[300px] overflow-y-auto">
              {updates.map((update) => (
                <div
                  key={update.resourceId}
                  className="flex items-center justify-between p-3 bg-background-muted rounded-lg"
                >
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-xs px-2 py-0.5 bg-blue-100 dark:bg-blue-900 text-blue-700 dark:text-blue-300 rounded">
                        {getResourceTypeLabel(update.resourceType)}
                      </span>
                      <span className="font-medium truncate">
                        {update.resourceName}
                      </span>
                    </div>
                    <div className="text-xs text-text-muted mt-1">
                      {update.currentVersion} → {update.latestVersion}
                    </div>
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => installUpdate(update)}
                    disabled={installingIds.has(update.resourceId)}
                  >
                    {installingIds.has(update.resourceId) ? (
                      <span className="flex items-center gap-1">
                        <span className="h-3 w-3 border-2 border-current border-t-transparent rounded-full animate-spin" />
                        {t('updates.installing', 'Installing...')}
                      </span>
                    ) : (
                      t('updates.update', 'Update')
                    )}
                  </Button>
                </div>
              ))}
            </div>
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={() => setShowDialog(false)}>
              {t('common.close', 'Close')}
            </Button>
            <Button onClick={installAllUpdates} disabled={updateCount === 0}>
              <Download className="h-4 w-4 mr-2" />
              {t('updates.updateAll', 'Update All')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

export default UpdateNotification;
