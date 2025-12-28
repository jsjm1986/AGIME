import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../ui/button';
import { Loader2, Download, CheckCircle, AlertCircle } from 'lucide-react';
import { isElectron } from '../../../platform';

type UpdateStatus =
  | 'idle'
  | 'checking'
  | 'downloading'
  | 'installing'
  | 'success'
  | 'error'
  | 'ready';

interface UpdateInfo {
  currentVersion: string;
  latestVersion?: string;
  isUpdateAvailable?: boolean;
  error?: string;
}

interface UpdateEventData {
  version?: string;
  percent?: number;
}

export default function UpdateSection() {
  // UpdateSection is only available in Electron since it manages desktop app updates
  if (!isElectron) {
    return null;
  }

  return <UpdateSectionContent />;
}

function UpdateSectionContent() {
  const { t } = useTranslation('settings');
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus>('idle');
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo>({
    currentVersion: '',
  });
  const [progress, setProgress] = useState<number>(0);
  const [isUsingGitHubFallback, setIsUsingGitHubFallback] = useState<boolean>(false);
  const progressTimeoutRef = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastProgressRef = React.useRef<number>(0); // Track last progress to prevent backward jumps

  useEffect(() => {
    // Get current version on mount
    const currentVersion = window.electron.getVersion();
    setUpdateInfo((prev) => ({ ...prev, currentVersion }));

    // Check if there's already an update state from the auto-check
    window.electron.getUpdateState().then((state) => {
      if (state) {
        console.log('Found existing update state:', state);
        setUpdateInfo((prev) => ({
          ...prev,
          isUpdateAvailable: state.updateAvailable,
          latestVersion: state.latestVersion,
        }));
      }
    });

    // Check if using GitHub fallback
    window.electron.isUsingGitHubFallback().then((isGitHub) => {
      setIsUsingGitHubFallback(isGitHub);
    });

    // Listen for updater events
    window.electron.onUpdaterEvent((event) => {
      console.log('Updater event:', event);

      switch (event.event) {
        case 'checking-for-update':
          setUpdateStatus('checking');
          break;

        case 'update-available':
          setUpdateStatus('idle');
          setUpdateInfo((prev) => ({
            ...prev,
            latestVersion: (event.data as UpdateEventData)?.version,
            isUpdateAvailable: true,
          }));
          // Check if GitHub fallback is being used
          window.electron.isUsingGitHubFallback().then((isGitHub) => {
            setIsUsingGitHubFallback(isGitHub);
          });
          break;

        case 'update-not-available':
          setUpdateStatus('idle');
          setUpdateInfo((prev) => ({
            ...prev,
            isUpdateAvailable: false,
          }));
          break;

        case 'download-progress': {
          setUpdateStatus('downloading');

          // Get the new progress value (ensure it's a valid number)
          const rawPercent = (event.data as UpdateEventData)?.percent;
          const newProgress = typeof rawPercent === 'number' ? Math.round(rawPercent) : 0;

          // Only update if progress increased (prevents backward jumps from out-of-order events)
          if (newProgress > lastProgressRef.current) {
            lastProgressRef.current = newProgress;

            // Cancel any pending update
            if (progressTimeoutRef.current) {
              clearTimeout(progressTimeoutRef.current);
            }

            // Use a small delay to batch rapid updates
            progressTimeoutRef.current = setTimeout(() => {
              setProgress(newProgress);
            }, 50); // 50ms delay for smoother batching
          }
          break;
        }

        case 'update-downloaded':
          setUpdateStatus('ready');
          setProgress(100);
          break;

        case 'error':
          setUpdateStatus('error');
          setUpdateInfo((prev) => ({
            ...prev,
            error: String(event.data || 'An error occurred'),
          }));
          setTimeout(() => setUpdateStatus('idle'), 5000);
          break;
      }
    });

    // Cleanup timeout on unmount
    return () => {
      if (progressTimeoutRef.current) {
        clearTimeout(progressTimeoutRef.current);
      }
    };
  }, []);

  const checkForUpdates = async () => {
    setUpdateStatus('checking');
    setProgress(0);
    lastProgressRef.current = 0; // Reset progress tracking for new download

    try {
      const result = await window.electron.checkForUpdates();

      if (result.error) {
        throw new Error(result.error);
      }

      // If we successfully checked and no update is available, show success
      if (!result.error && updateInfo.isUpdateAvailable === false) {
        setUpdateStatus('success');
        setTimeout(() => setUpdateStatus('idle'), 3000);
      }
      // The actual status will be handled by the updater events
    } catch (error) {
      console.error('Error checking for updates:', error);
      setUpdateInfo((prev) => ({
        ...prev,
        error: error instanceof Error ? error.message : 'Failed to check for updates',
      }));
      setUpdateStatus('error');
      setTimeout(() => setUpdateStatus('idle'), 5000);
    }
  };

  const installUpdate = () => {
    window.electron.installUpdate();
  };

  const getStatusMessage = () => {
    switch (updateStatus) {
      case 'checking':
        return t('update.status.checking');
      case 'downloading':
        return t('update.status.downloading', { progress: Math.round(progress) });
      case 'ready':
        return t('update.status.ready');
      case 'success':
        return updateInfo.isUpdateAvailable === false
          ? t('update.status.latestVersion')
          : t('update.status.updateAvailable');
      case 'error':
        return updateInfo.error || t('update.status.error');
      default:
        if (updateInfo.isUpdateAvailable) {
          return t('update.status.versionAvailable', { version: updateInfo.latestVersion });
        }
        return '';
    }
  };

  const getStatusIcon = () => {
    switch (updateStatus) {
      case 'checking':
      case 'downloading':
        return <Loader2 className="w-4 h-4 animate-spin" />;
      case 'success':
        return <CheckCircle className="w-4 h-4 text-green-500" />;
      case 'error':
        return <AlertCircle className="w-4 h-4 text-red-500" />;
      case 'ready':
        return <CheckCircle className="w-4 h-4 text-blue-500" />;
      default:
        return updateInfo.isUpdateAvailable ? <Download className="w-4 h-4" /> : null;
    }
  };

  return (
    <div>
      <div className="text-sm text-text-muted mb-4 flex items-center gap-2">
        <div className="flex flex-col">
          <div className="text-text-default text-2xl font-mono">
            {updateInfo.currentVersion || t('update.loading')}
          </div>
          <div className="text-xs text-text-muted">{t('update.currentVersion')}</div>
        </div>
        {updateInfo.latestVersion && updateInfo.isUpdateAvailable && (
          <span className="text-textSubtle"> {t('update.versionAvailable', { version: updateInfo.latestVersion })}</span>
        )}
        {updateInfo.currentVersion && updateInfo.isUpdateAvailable === false && (
          <span className="text-text-default"> {t('update.upToDate')}</span>
        )}
      </div>

      <div className="flex gap-2">
        <div className="flex items-center gap-2">
          <Button
            onClick={checkForUpdates}
            disabled={updateStatus !== 'idle' && updateStatus !== 'error'}
            variant="secondary"
            size="sm"
          >
            {t('update.checkForUpdates')}
          </Button>

          {updateStatus === 'ready' && (
            <Button onClick={installUpdate} variant="default" size="sm">
              {t('update.installAndRestart')}
            </Button>
          )}
        </div>

        {getStatusMessage() && (
          <div className="flex items-center gap-2 text-xs text-text-muted">
            {getStatusIcon()}
            <span>{getStatusMessage()}</span>
          </div>
        )}

        {updateStatus === 'downloading' && (
          <div className="w-full mt-2">
            <div className="flex justify-between text-xs text-text-muted mb-1">
              <span>{t('update.downloading')}</span>
              <span>{progress}%</span>
            </div>
            <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-2 overflow-hidden">
              <div
                className="bg-blue-500 h-2 rounded-full transition-[width] duration-150 ease-out"
                style={{ width: `${Math.max(progress, 0)}%`, minWidth: progress > 0 ? '8px' : '0' }}
              />
            </div>
          </div>
        )}

        {/* Update information */}
        {updateInfo.isUpdateAvailable && updateStatus === 'idle' && (
          <div className="text-xs text-text-muted mt-4 space-y-1">
            <p>{t('update.info.autoDownload')}</p>
            {isUsingGitHubFallback ? (
              <p className="text-xs text-amber-600">
                {t('update.info.manualInstall')}
              </p>
            ) : (
              <p className="text-xs text-green-600">
                {t('update.info.autoInstall')}
              </p>
            )}
          </div>
        )}

        {updateStatus === 'ready' && (
          <div className="text-xs text-text-muted mt-4 space-y-1">
            {isUsingGitHubFallback ? (
              <>
                <p className="text-xs text-green-600">
                  {t('update.ready.githubReady')}
                </p>
                <p className="text-xs text-text-muted">
                  {t('update.ready.githubManual')}
                </p>
              </>
            ) : (
              <>
                <p className="text-xs text-green-600">
                  {t('update.ready.autoReady')}
                </p>
                <p className="text-xs text-text-muted">
                  {t('update.ready.installNow')}
                </p>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
