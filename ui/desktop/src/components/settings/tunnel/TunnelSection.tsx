import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../ui/button';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from '../../ui/dialog';
import { QRCodeSVG } from 'qrcode.react';
import {
  Loader2,
  Copy,
  Check,
  ChevronDown,
  ChevronUp,
  Info,
  ExternalLink,
  Globe,
  Monitor,
  Smartphone,
  Download,
} from 'lucide-react';
import { SettingsCard, SettingsItem } from '../common';
import { isElectron } from '../../../platform';

interface CloudflaredTunnelInfo {
  state: 'idle' | 'starting' | 'running' | 'error' | 'disabled';
  url: string;
  hostname: string;
  secret: string;
  error?: string;
}

export default function TunnelSection() {
  // TunnelSection is only available in Electron since it manages local cloudflared
  if (!isElectron) {
    return null;
  }

  return <TunnelSectionContent />;
}

function TunnelSectionContent() {
  const { t } = useTranslation('settings');
  const [tunnelInfo, setTunnelInfo] = useState<CloudflaredTunnelInfo>({
    state: 'idle',
    url: '',
    hostname: '',
    secret: '',
  });
  const [showConnectionModal, setShowConnectionModal] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copiedUrl, setCopiedUrl] = useState(false);
  const [copiedSecret, setCopiedSecret] = useState(false);
  const [copiedWebUrl, setCopiedWebUrl] = useState(false);
  const [showDetails, setShowDetails] = useState(false);
  const [activeTab, setActiveTab] = useState<'web' | 'mobile'>('web');

  // Cloudflared specific state
  const [isCloudflaredInstalled, setIsCloudflaredInstalled] = useState<boolean | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<number>(0);
  const [isDownloading, setIsDownloading] = useState(false);

  useEffect(() => {
    const loadInitialState = async () => {
      try {
        // Check if cloudflared is installed
        const installed = await window.electron.cloudflaredCheckInstalled();
        setIsCloudflaredInstalled(installed);

        // Get current tunnel status
        const status = await window.electron.cloudflaredStatus();
        setTunnelInfo(status as CloudflaredTunnelInfo);
      } catch (err) {
        console.error('Failed to load initial tunnel state:', err);
        setError(t('tunnel.errors.checkFailed') || 'Failed to check cloudflared status');
      }
    };

    loadInitialState();

    // Listen for download progress and store cleanup function
    const cleanupProgressListener = window.electron.onCloudflaredDownloadProgress((percent: number) => {
      setDownloadProgress(percent);
    });

    // Cleanup on unmount
    return () => {
      cleanupProgressListener();
    };
  }, [t]);

  const handleDownloadCloudflared = async () => {
    setIsDownloading(true);
    setDownloadProgress(0);
    setError(null);

    try {
      const result = await window.electron.cloudflaredDownload();
      if (result.success) {
        setIsCloudflaredInstalled(true);
      } else {
        setError(result.error || t('tunnel.errors.downloadFailed') || 'Failed to download cloudflared');
      }
    } catch (err) {
      setError(t('tunnel.errors.downloadFailed') || `Download failed: ${err}`);
    } finally {
      setIsDownloading(false);
    }
  };

  const handleToggleTunnel = async () => {
    if (tunnelInfo.state === 'running') {
      try {
        const result = await window.electron.cloudflaredStop();
        if (result.success) {
          setTunnelInfo({ state: 'idle', url: '', hostname: '', secret: '' });
          setShowConnectionModal(false);
        } else {
          setError(result.error || t('tunnel.errors.stopFailed') || 'Failed to stop tunnel');
        }
      } catch (err) {
        setError(t('tunnel.errors.stopFailed') || `Failed to stop tunnel: ${err}`);
      }
    } else {
      setError(null);
      setTunnelInfo({ state: 'starting', url: '', hostname: '', secret: '' });

      try {
        const result = await window.electron.cloudflaredStart();
        if (result.success && result.data) {
          setTunnelInfo(result.data as CloudflaredTunnelInfo);
          setShowConnectionModal(true);
        } else {
          setError(result.error || t('tunnel.errors.startFailed') || 'Failed to start tunnel');
          setTunnelInfo({ state: 'error', url: '', hostname: '', secret: '' });
        }
      } catch (err) {
        setError(t('tunnel.errors.startFailed') || `Failed to start tunnel: ${err}`);
        setTunnelInfo({ state: 'error', url: '', hostname: '', secret: '' });
      }
    }
  };

  const copyToClipboard = async (text: string, type: 'url' | 'secret' | 'webUrl') => {
    try {
      await navigator.clipboard.writeText(text);
      if (type === 'url') {
        setCopiedUrl(true);
        setTimeout(() => setCopiedUrl(false), 2000);
      } else if (type === 'secret') {
        setCopiedSecret(true);
        setTimeout(() => setCopiedSecret(false), 2000);
      } else if (type === 'webUrl') {
        setCopiedWebUrl(true);
        setTimeout(() => setCopiedWebUrl(false), 2000);
      }
    } catch (err) {
      console.error('Failed to copy to clipboard:', err);
    }
  };

  // Generate Web access URL with authentication
  const getWebAccessUrl = () => {
    if (tunnelInfo.state !== 'running' || !tunnelInfo.url || !tunnelInfo.secret) return '';
    return `${tunnelInfo.url}/web/#secret=${tunnelInfo.secret}`;
  };

  // Generate QR code data for mobile web access
  // Uses the same Web URL as desktop, optimized for mobile browsers
  const getMobileQRCodeData = () => {
    if (tunnelInfo.state !== 'running' || !tunnelInfo.url || !tunnelInfo.secret) return '';
    // Use the same URL as web access - the web UI is already mobile-optimized
    return `${tunnelInfo.url}/web/#secret=${tunnelInfo.secret}`;
  };

  // Render download section if cloudflared is not installed
  const renderDownloadSection = () => {
    if (isCloudflaredInstalled === null) {
      return (
        <div className="flex items-center justify-center p-4">
          <Loader2 className="h-5 w-5 animate-spin mr-2" />
          <span className="text-sm text-text-muted">{t('tunnel.checkingCloudflared') || 'Checking cloudflared...'}</span>
        </div>
      );
    }

    if (!isCloudflaredInstalled) {
      return (
        <div className="space-y-4">
          <div className="flex items-start gap-2 p-3 bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-800 rounded-lg">
            <Info className="h-4 w-4 text-yellow-600 dark:text-yellow-400 flex-shrink-0 mt-0.5" />
            <div className="text-xs text-yellow-800 dark:text-yellow-200">
              <strong>{t('tunnel.cloudflaredRequired') || 'Cloudflared Required'}</strong>{' '}
              {t('tunnel.cloudflaredRequiredHint') || 'Cloudflared is required to create public tunnel URLs. Click the button below to download and install it automatically.'}
            </div>
          </div>

          {isDownloading ? (
            <div className="space-y-2">
              <div className="flex items-center gap-2">
                <Loader2 className="h-4 w-4 animate-spin" />
                <span className="text-sm">{t('tunnel.downloadingCloudflared') || 'Downloading cloudflared...'}</span>
              </div>
              <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-2">
                <div
                  className="bg-blue-600 h-2 rounded-full transition-all duration-300"
                  style={{ width: `${downloadProgress}%` }}
                />
              </div>
              <div className="text-xs text-text-muted text-right">{downloadProgress}%</div>
            </div>
          ) : (
            <Button onClick={handleDownloadCloudflared} variant="default" className="w-full">
              <Download className="h-4 w-4 mr-2" />
              {t('tunnel.downloadCloudflared') || 'Download Cloudflared'}
            </Button>
          )}
        </div>
      );
    }

    return null;
  };

  if (tunnelInfo.state === 'disabled') {
    return null;
  }

  return (
    <>
      <SettingsCard
        icon={<Globe className="h-5 w-5" />}
        title={t('tunnel.title')}
        description={t('tunnel.description')}
      >
        {/* Preview feature notice */}
        <div className="flex items-start gap-2 p-3 bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-lg">
          <Info className="h-4 w-4 text-blue-600 dark:text-blue-400 flex-shrink-0 mt-0.5" />
          <div className="text-xs text-blue-800 dark:text-blue-200">
            <strong>{t('tunnel.previewFeature')}</strong>{' '}
            {t('tunnel.webAccessHint')}
          </div>
        </div>

        {error && (
          <div className="p-3 bg-red-100 dark:bg-red-900/20 border border-red-300 dark:border-red-800 rounded-lg text-sm text-red-800 dark:text-red-200">
            {error}
          </div>
        )}

        {/* Show download section if cloudflared not installed */}
        {renderDownloadSection()}

        {/* Tunnel status - only show if cloudflared is installed */}
        {isCloudflaredInstalled && (
          <>
            <SettingsItem
              title={t('tunnel.tunnelStatus')}
              description={t(`tunnel.status.${tunnelInfo.state}`)}
              control={
                <div className="flex items-center gap-2">
                  {tunnelInfo.state === 'starting' ? (
                    <Button disabled variant="secondary" size="sm">
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                      {t('tunnel.buttons.starting')}
                    </Button>
                  ) : tunnelInfo.state === 'running' ? (
                    <>
                      <Button onClick={() => setShowConnectionModal(true)} variant="default" size="sm">
                        {t('tunnel.buttons.showConnection')}
                      </Button>
                      <Button onClick={handleToggleTunnel} variant="destructive" size="sm">
                        {t('tunnel.buttons.stopTunnel')}
                      </Button>
                    </>
                  ) : (
                    <Button onClick={handleToggleTunnel} variant="default" size="sm">
                      {tunnelInfo.state === 'error' ? t('tunnel.buttons.retry') : t('tunnel.buttons.startTunnel')}
                    </Button>
                  )}
                </div>
              }
            />

            {/* Show Web access link when tunnel is running */}
            {tunnelInfo.state === 'running' && (
              <div className="p-3 bg-green-100 dark:bg-green-900/20 border border-green-300 dark:border-green-800 rounded-lg space-y-3">
                <div className="flex items-center justify-between">
                  <div className="flex-1 min-w-0">
                    <p className="text-xs font-medium text-green-800 dark:text-green-200 mb-1">
                      {t('tunnel.webAccessUrl')}
                    </p>
                    <code className="text-xs text-green-700 dark:text-green-300 break-all block">
                      {getWebAccessUrl()}
                    </code>
                  </div>
                  <div className="flex gap-2 ml-3 flex-shrink-0">
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => copyToClipboard(getWebAccessUrl(), 'webUrl')}
                      className="text-green-700 border-green-300 hover:bg-green-50 dark:text-green-300 dark:border-green-700 dark:hover:bg-green-900/30"
                    >
                      {copiedWebUrl ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                      <span className="ml-1">{copiedWebUrl ? t('tunnel.copied') : t('tunnel.copyLink')}</span>
                    </Button>
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => window.open(getWebAccessUrl(), '_blank')}
                      className="text-green-700 border-green-300 hover:bg-green-50 dark:text-green-300 dark:border-green-700 dark:hover:bg-green-900/30"
                    >
                      <ExternalLink className="h-4 w-4" />
                      <span className="ml-1">{t('tunnel.openInBrowser')}</span>
                    </Button>
                  </div>
                </div>
              </div>
            )}
          </>
        )}
      </SettingsCard>

      {/* Connection details modal */}
      <Dialog open={showConnectionModal} onOpenChange={setShowConnectionModal}>
        <DialogContent className="sm:max-w-[550px]">
          <DialogHeader>
            <DialogTitle>{t('tunnel.connectionModal.title')}</DialogTitle>
          </DialogHeader>

          {tunnelInfo.state === 'running' && (
            <div className="py-4 space-y-4">
              {/* Tab switcher */}
              <div className="flex border-b border-border">
                <button
                  onClick={() => setActiveTab('web')}
                  className={`flex items-center gap-2 px-4 py-2 text-sm font-medium border-b-2 transition-colors ${
                    activeTab === 'web'
                      ? 'border-primary text-primary'
                      : 'border-transparent text-text-muted hover:text-foreground'
                  }`}
                >
                  <Monitor className="h-4 w-4" />
                  {t('tunnel.tabs.webAccess')}
                </button>
                <button
                  onClick={() => setActiveTab('mobile')}
                  className={`flex items-center gap-2 px-4 py-2 text-sm font-medium border-b-2 transition-colors ${
                    activeTab === 'mobile'
                      ? 'border-primary text-primary'
                      : 'border-transparent text-text-muted hover:text-foreground'
                  }`}
                >
                  <Smartphone className="h-4 w-4" />
                  {t('tunnel.tabs.mobileApp')}
                </button>
              </div>

              {/* Web Access Tab */}
              {activeTab === 'web' && (
                <div className="space-y-4">
                  <div className="text-center text-sm text-text-muted">
                    {t('tunnel.connectionModal.webHint')}
                  </div>

                  {/* Web access link */}
                  <div className="p-4 bg-background-muted rounded-lg space-y-3">
                    <div>
                      <h3 className="text-xs font-medium mb-2">{t('tunnel.connectionModal.webAccessUrl')}</h3>
                      <div className="flex items-center gap-2">
                        <code className="flex-1 p-2 bg-background rounded-lg text-xs break-all overflow-hidden border">
                          {getWebAccessUrl()}
                        </code>
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => copyToClipboard(getWebAccessUrl(), 'webUrl')}
                        >
                          {copiedWebUrl ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                        </Button>
                      </div>
                    </div>

                    <div className="flex gap-2">
                      <Button
                        className="flex-1"
                        onClick={() => window.open(getWebAccessUrl(), '_blank')}
                      >
                        <ExternalLink className="h-4 w-4 mr-2" />
                        {t('tunnel.openInBrowser')}
                      </Button>
                    </div>
                  </div>

                  {/* Advanced options: show details */}
                  <div className="border-t pt-4">
                    <button
                      onClick={() => setShowDetails(!showDetails)}
                      className="flex items-center justify-between w-full text-sm font-medium hover:opacity-70 transition-opacity"
                    >
                      <span>{t('tunnel.connectionModal.connectionDetails')}</span>
                      {showDetails ? (
                        <ChevronUp className="h-4 w-4" />
                      ) : (
                        <ChevronDown className="h-4 w-4" />
                      )}
                    </button>

                    {showDetails && (
                      <div className="mt-3 space-y-3">
                        <div>
                          <h3 className="text-xs font-medium mb-1 text-text-muted">{t('tunnel.connectionModal.tunnelUrl')}</h3>
                          <div className="flex items-center gap-2">
                            <code className="flex-1 p-2 bg-background-muted rounded-lg text-xs break-all overflow-hidden">
                              {tunnelInfo.url}
                            </code>
                            <Button
                              size="sm"
                              variant="ghost"
                              className="flex-shrink-0"
                              onClick={() => tunnelInfo.url && copyToClipboard(tunnelInfo.url, 'url')}
                            >
                              {copiedUrl ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                            </Button>
                          </div>
                        </div>

                        <div>
                          <h3 className="text-xs font-medium mb-1 text-text-muted">{t('tunnel.connectionModal.secretKey')}</h3>
                          <div className="flex items-center gap-2">
                            <code className="flex-1 p-2 bg-background-muted rounded-lg text-xs break-all overflow-hidden">
                              {tunnelInfo.secret}
                            </code>
                            <Button
                              size="sm"
                              variant="ghost"
                              className="flex-shrink-0"
                              onClick={() =>
                                tunnelInfo.secret && copyToClipboard(tunnelInfo.secret, 'secret')
                              }
                            >
                              {copiedSecret ? (
                                <Check className="h-4 w-4" />
                              ) : (
                                <Copy className="h-4 w-4" />
                              )}
                            </Button>
                          </div>
                        </div>
                      </div>
                    )}
                  </div>
                </div>
              )}

              {/* Mobile Tab */}
              {activeTab === 'mobile' && (
                <div className="space-y-4">
                  <div className="flex justify-center">
                    <div className="p-4 bg-white rounded-lg">
                      <QRCodeSVG value={getMobileQRCodeData()} size={200} />
                    </div>
                  </div>

                  <div className="text-center text-sm text-text-muted">
                    {t('tunnel.connectionModal.scanHint')}
                  </div>

                  <div className="text-center text-xs text-text-muted">
                    {t('tunnel.connectionModal.mobileAppNote')}
                  </div>
                </div>
              )}
            </div>
          )}

          <DialogFooter>
            <Button variant="outline" onClick={() => setShowConnectionModal(false)}>
              {t('tunnel.buttons.close')}
            </Button>
            <Button variant="destructive" onClick={handleToggleTunnel}>
              {t('tunnel.buttons.stopTunnel')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
