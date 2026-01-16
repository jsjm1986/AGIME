import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../ui/button';
import { SettingsCard } from '../common';
import { Wifi, Copy, Check, QrCode, RefreshCw } from 'lucide-react';
import { isElectron, platform } from '../../../platform';
import { QRCodeSVG } from 'qrcode.react';

interface NetworkInfo {
  addresses: Array<{ name: string; address: string; family: string }>;
  port: number;
  secretKey: string;
}

export default function LanSharingSection() {
  // LanSharingSection is only available in Electron
  if (!isElectron) {
    return null;
  }

  return <LanSharingSectionContent />;
}

function LanSharingSectionContent() {
  const { t } = useTranslation('settings');
  const [networkInfo, setNetworkInfo] = useState<NetworkInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [copied, setCopied] = useState<string | null>(null);
  const [qrCodeData, setQrCodeData] = useState<string | null>(null);
  const [showQrCode, setShowQrCode] = useState(false);

  const loadNetworkInfo = async () => {
    setLoading(true);
    try {
      const info = await platform.getNetworkInfo();
      setNetworkInfo(info);
    } catch (error) {
      console.error('Failed to get network info:', error);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadNetworkInfo();
  }, []);

  const copyToClipboard = async (text: string, key: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(key);
      setTimeout(() => setCopied(null), 2000);
    } catch (error) {
      console.error('Failed to copy:', error);
    }
  };

  const getConnectionUrl = (address: string) => {
    if (!networkInfo) return '';
    return `http://${address}:${networkInfo.port}`;
  };

  const handleShowQrCode = (address: string) => {
    const url = getConnectionUrl(address);
    const info = JSON.stringify({
      url,
      key: networkInfo?.secretKey,
    });
    setQrCodeData(info);
    setShowQrCode(true);
  };

  if (loading) {
    return (
      <SettingsCard
        icon={<Wifi className="h-5 w-5" />}
        title={t('team.lanSharing.title', 'LAN Sharing')}
        description={t('team.lanSharing.description', 'Share AGIME with other devices on your local network')}
      >
        <div className="flex items-center justify-center py-4">
          <RefreshCw className="h-5 w-5 animate-spin text-text-muted" />
          <span className="ml-2 text-sm text-text-muted">
            {t('team.lanSharing.loading', 'Loading network info...')}
          </span>
        </div>
      </SettingsCard>
    );
  }

  if (!networkInfo || networkInfo.addresses.length === 0) {
    return (
      <SettingsCard
        icon={<Wifi className="h-5 w-5" />}
        title={t('team.lanSharing.title', 'LAN Sharing')}
        description={t('team.lanSharing.description', 'Share AGIME with other devices on your local network')}
      >
        <div className="text-sm text-text-muted py-2">
          {t('team.lanSharing.noNetwork', 'No network interfaces found. Make sure you are connected to a network.')}
        </div>
        <Button variant="outline" size="sm" onClick={loadNetworkInfo}>
          <RefreshCw className="h-4 w-4 mr-2" />
          {t('team.lanSharing.refresh', 'Refresh')}
        </Button>
      </SettingsCard>
    );
  }

  return (
    <SettingsCard
      icon={<Wifi className="h-5 w-5" />}
      title={t('team.lanSharing.title', 'LAN Sharing')}
      description={t('team.lanSharing.description', 'Share AGIME with other devices on your local network')}
    >
      <div className="space-y-4">
        {/* Info banner */}
        <div className="bg-blue-50 dark:bg-blue-950 border border-blue-200 dark:border-blue-800 rounded-lg p-3">
          <p className="text-xs text-blue-800 dark:text-blue-200">
            {t('team.lanSharing.info', 'Other devices on the same network can connect to this computer to collaborate. Share the connection URL and secret key with team members.')}
          </p>
        </div>

        {/* Network addresses */}
        <div className="space-y-3">
          <h4 className="text-sm font-medium text-text-default">
            {t('team.lanSharing.availableAddresses', 'Available Addresses')}
          </h4>

          {networkInfo.addresses.map((addr, index) => (
            <div key={index} className="bg-background-muted rounded-lg p-3 space-y-2">
              <div className="flex items-center justify-between">
                <div>
                  <span className="text-xs text-text-muted">{addr.name}</span>
                  <div className="font-mono text-sm text-text-default">
                    {getConnectionUrl(addr.address)}
                  </div>
                </div>
                <div className="flex gap-2">
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => copyToClipboard(getConnectionUrl(addr.address), `url-${index}`)}
                    title={t('team.lanSharing.copyUrl', 'Copy URL')}
                  >
                    {copied === `url-${index}` ? (
                      <Check className="h-4 w-4 text-green-500" />
                    ) : (
                      <Copy className="h-4 w-4" />
                    )}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => handleShowQrCode(addr.address)}
                    title={t('team.lanSharing.showQr', 'Show QR Code')}
                  >
                    <QrCode className="h-4 w-4" />
                  </Button>
                </div>
              </div>
            </div>
          ))}
        </div>

        {/* Secret Key */}
        <div className="space-y-2">
          <h4 className="text-sm font-medium text-text-default">
            {t('team.lanSharing.secretKey', 'Secret Key')}
          </h4>
          <div className="flex items-center gap-2">
            <code className="flex-1 bg-background-muted rounded px-3 py-2 font-mono text-xs text-text-default overflow-x-auto">
              {networkInfo.secretKey}
            </code>
            <Button
              variant="outline"
              size="sm"
              onClick={() => copyToClipboard(networkInfo.secretKey, 'secret')}
            >
              {copied === 'secret' ? (
                <>
                  <Check className="h-4 w-4 mr-1 text-green-500" />
                  {t('team.lanSharing.copied', 'Copied!')}
                </>
              ) : (
                <>
                  <Copy className="h-4 w-4 mr-1" />
                  {t('team.lanSharing.copy', 'Copy')}
                </>
              )}
            </Button>
          </div>
          <p className="text-xs text-text-muted">
            {t('team.lanSharing.secretKeyHint', 'Share this key securely with team members. They will need it to connect.')}
          </p>
        </div>

        {/* Refresh button */}
        <div className="flex justify-end">
          <Button variant="outline" size="sm" onClick={loadNetworkInfo}>
            <RefreshCw className="h-4 w-4 mr-2" />
            {t('team.lanSharing.refresh', 'Refresh')}
          </Button>
        </div>

        {/* QR Code Modal */}
        {showQrCode && qrCodeData && (
          <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50" onClick={() => setShowQrCode(false)}>
            <div className="bg-background-default rounded-lg p-6 shadow-xl" onClick={e => e.stopPropagation()}>
              <h3 className="text-lg font-medium mb-4 text-center">
                {t('team.lanSharing.scanQr', 'Scan to Connect')}
              </h3>
              <div className="bg-white p-4 rounded-lg flex items-center justify-center">
                <QRCodeSVG value={qrCodeData} size={200} />
              </div>
              <p className="text-xs text-text-muted mt-4 text-center max-w-xs">
                {t('team.lanSharing.qrHint', 'Scan this QR code with another device to get the connection details.')}
              </p>
              <div className="flex justify-center mt-4">
                <Button variant="outline" onClick={() => setShowQrCode(false)}>
                  {t('team.lanSharing.close', 'Close')}
                </Button>
              </div>
            </div>
          </div>
        )}
      </div>
    </SettingsCard>
  );
}
