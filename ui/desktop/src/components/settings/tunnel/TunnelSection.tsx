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
  QrCode,
  Globe,
} from 'lucide-react';
import { errorMessage } from '../../../utils/conversionUtils';
import { startTunnel, stopTunnel, getTunnelStatus } from '../../../api/sdk.gen';
import type { TunnelInfo } from '../../../api/types.gen';
import { SettingsCard, SettingsItem } from '../common';

const IOS_APP_STORE_URL = 'https://apps.apple.com/us/app/goose-ai/id6752889295';

export default function TunnelSection() {
  const { t } = useTranslation('settings');
  const [tunnelInfo, setTunnelInfo] = useState<TunnelInfo>({
    state: 'idle',
    url: '',
    hostname: '',
    secret: '',
  });
  const [showQRModal, setShowQRModal] = useState(false);
  const [showAppStoreQRModal, setShowAppStoreQRModal] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copiedUrl, setCopiedUrl] = useState(false);
  const [copiedSecret, setCopiedSecret] = useState(false);
  const [showDetails, setShowDetails] = useState(false);

  useEffect(() => {
    const loadTunnelInfo = async () => {
      try {
        const { data } = await getTunnelStatus();
        if (data) {
          setTunnelInfo(data);
        }
      } catch (err) {
        const errorMsg = errorMessage(err, 'Failed to load tunnel status');
        setError(errorMsg);
        setTunnelInfo({ state: 'error', url: '', hostname: '', secret: '' });
      }
    };

    loadTunnelInfo();
  }, []);

  const handleToggleTunnel = async () => {
    if (tunnelInfo.state === 'running') {
      try {
        await stopTunnel();
        setTunnelInfo({ state: 'idle', url: '', hostname: '', secret: '' });
        setShowQRModal(false);
      } catch (err) {
        setError(errorMessage(err, 'Failed to stop tunnel'));
        try {
          const { data } = await getTunnelStatus();
          if (data) {
            setTunnelInfo(data);
          }
        } catch (statusErr) {
          console.error('Failed to fetch tunnel status after stop error:', statusErr);
        }
      }
    } else {
      setError(null);
      setTunnelInfo({ state: 'starting', url: '', hostname: '', secret: '' });

      try {
        const { data } = await startTunnel();
        if (data) {
          setTunnelInfo(data);
          setShowQRModal(true);
        }
      } catch (err) {
        const errorMsg = errorMessage(err, 'Failed to start tunnel');
        setError(errorMsg);
        setTunnelInfo({ state: 'error', url: '', hostname: '', secret: '' });
      }
    }
  };

  const copyToClipboard = async (text: string, type: 'url' | 'secret') => {
    try {
      await navigator.clipboard.writeText(text);
      if (type === 'url') {
        setCopiedUrl(true);
        setTimeout(() => setCopiedUrl(false), 2000);
      } else {
        setCopiedSecret(true);
        setTimeout(() => setCopiedSecret(false), 2000);
      }
    } catch (err) {
      console.error('Failed to copy to clipboard:', err);
    }
  };

  const getQRCodeData = () => {
    if (tunnelInfo.state !== 'running') return '';

    const configJson = JSON.stringify({
      url: tunnelInfo.url,
      secret: tunnelInfo.secret,
    });
    const urlEncodedConfig = encodeURIComponent(configJson);
    return `goosechat://configure?data=${urlEncodedConfig}`;
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
        {/* 预览功能提示 */}
        <div className="flex items-start gap-2 p-3 bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-lg">
          <Info className="h-4 w-4 text-blue-600 dark:text-blue-400 flex-shrink-0 mt-0.5" />
          <div className="text-xs text-blue-800 dark:text-blue-200">
            <strong>{t('tunnel.previewFeature')}</strong>{' '}
            <a
              href={IOS_APP_STORE_URL}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1 underline hover:no-underline"
            >
              {t('tunnel.getIosApp')}
              <ExternalLink className="h-3 w-3" />
            </a>
            {' ' + t('tunnel.or') + ' '}
            <button
              onClick={() => setShowAppStoreQRModal(true)}
              className="inline-flex items-center gap-1 underline hover:no-underline"
            >
              {t('tunnel.scanQrCode')}
              <QrCode className="h-3 w-3" />
            </button>
          </div>
        </div>

        {error && (
          <div className="p-3 bg-red-100 dark:bg-red-900/20 border border-red-300 dark:border-red-800 rounded-lg text-sm text-red-800 dark:text-red-200">
            {error}
          </div>
        )}

        {/* 隧道状态 */}
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
                  <Button onClick={() => setShowQRModal(true)} variant="default" size="sm">
                    {t('tunnel.buttons.showQrCode')}
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

        {tunnelInfo.state === 'running' && (
          <div className="p-3 bg-green-100 dark:bg-green-900/20 border border-green-300 dark:border-green-800 rounded-lg">
            <p className="text-xs text-green-800 dark:text-green-200">
              <strong>{t('tunnel.url')}</strong> {tunnelInfo.url}
            </p>
          </div>
        )}
      </SettingsCard>

      <Dialog open={showQRModal} onOpenChange={setShowQRModal}>
        <DialogContent className="sm:max-w-[500px]">
          <DialogHeader>
            <DialogTitle>{t('tunnel.connectionModal.title')}</DialogTitle>
          </DialogHeader>

          {tunnelInfo.state === 'running' && (
            <div className="py-4 space-y-4">
              <div className="flex justify-center">
                <div className="p-4 bg-white rounded-lg">
                  <QRCodeSVG value={getQRCodeData()} size={200} />
                </div>
              </div>

              <div className="text-center text-sm text-text-muted">
                {t('tunnel.connectionModal.scanHint')}
              </div>

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

          <DialogFooter>
            <Button variant="outline" onClick={() => setShowQRModal(false)}>
              {t('tunnel.buttons.close')}
            </Button>
            <Button variant="destructive" onClick={handleToggleTunnel}>
              {t('tunnel.buttons.stopTunnel')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={showAppStoreQRModal} onOpenChange={setShowAppStoreQRModal}>
        <DialogContent className="sm:max-w-[400px]">
          <DialogHeader>
            <DialogTitle>{t('tunnel.appStoreModal.title')}</DialogTitle>
          </DialogHeader>

          <div className="py-4 space-y-4">
            <div className="flex justify-center">
              <div className="p-4 bg-white rounded-lg">
                <QRCodeSVG value={IOS_APP_STORE_URL} size={200} />
              </div>
            </div>

            <div className="text-center text-sm text-text-muted">
              {t('tunnel.appStoreModal.scanHint')}
            </div>

            <div className="text-center">
              <a
                href={IOS_APP_STORE_URL}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-2 text-sm text-blue-600 dark:text-blue-400 hover:underline"
              >
                <ExternalLink className="h-4 w-4" />
                {t('tunnel.appStoreModal.openInAppStore')}
              </a>
            </div>
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={() => setShowAppStoreQRModal(false)}>
              {t('tunnel.buttons.close')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
