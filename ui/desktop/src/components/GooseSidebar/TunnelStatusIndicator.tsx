import { useState, useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Radio, Loader2, Copy, Check, ExternalLink, ChevronDown, ChevronUp, Monitor, Smartphone, Download, AlertCircle } from 'lucide-react';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogDescription } from '../ui/dialog';
import { Button } from '../ui/button';
import { QRCodeSVG } from 'qrcode.react';
import { isElectron } from '../../platform';

interface TunnelInfo {
  state: 'idle' | 'starting' | 'running' | 'error' | 'disabled' | 'checking' | 'not_installed';
  url: string;
  hostname: string;
  secret: string;
  error?: string;
}

export default function TunnelStatusIndicator() {
  const { t } = useTranslation('settings');
  const navigate = useNavigate();
  const [tunnelInfo, setTunnelInfo] = useState<TunnelInfo>({
    state: 'checking',
    url: '',
    hostname: '',
    secret: '',
  });
  const [isCloudflaredInstalled, setIsCloudflaredInstalled] = useState<boolean | null>(null);
  const [showModal, setShowModal] = useState(false);
  const [showInstallModal, setShowInstallModal] = useState(false);
  const [activeTab, setActiveTab] = useState<'web' | 'mobile'>('web');
  const [showDetails, setShowDetails] = useState(false);
  const [copiedUrl, setCopiedUrl] = useState(false);
  const [copiedSecret, setCopiedSecret] = useState(false);
  const [copiedWebUrl, setCopiedWebUrl] = useState(false);
  const [isStarting, setIsStarting] = useState(false);

  useEffect(() => {
    if (!isElectron) {
      setTunnelInfo({ state: 'disabled', url: '', hostname: '', secret: '' });
      return;
    }

    const checkStatus = async () => {
      try {
        // Check if cloudflared is installed
        const installed = await window.electron.cloudflaredCheckInstalled();
        setIsCloudflaredInstalled(installed);

        // Get current tunnel status
        const status = await window.electron.cloudflaredStatus();

        if (!installed) {
          setTunnelInfo({ state: 'not_installed', url: '', hostname: '', secret: '' });
        } else {
          setTunnelInfo(status as TunnelInfo);
        }
      } catch {
        setTunnelInfo({ state: 'disabled', url: '', hostname: '', secret: '' });
      }
    };

    checkStatus();

    // Poll for status updates every 5 seconds
    const interval = setInterval(checkStatus, 5000);
    return () => clearInterval(interval);
  }, []);

  // Don't render if tunnel feature is disabled
  if (tunnelInfo.state === 'disabled') {
    return null;
  }

  const getStatusColor = () => {
    switch (tunnelInfo.state) {
      case 'running':
        return 'text-green-500';
      case 'starting':
        return 'text-yellow-500';
      case 'error':
        return 'text-red-500';
      case 'not_installed':
      case 'idle':
      case 'checking':
      default:
        return 'text-text-muted/50';
    }
  };

  const getStatusText = () => {
    switch (tunnelInfo.state) {
      case 'running':
        return t('tunnel.status.running', '隧道运行中');
      case 'starting':
        return t('tunnel.status.starting', '正在启动...');
      case 'error':
        return t('tunnel.status.error', '隧道错误');
      case 'not_installed':
        return t('tunnel.status.notInstalled', '未安装');
      case 'idle':
        return t('tunnel.status.idle', '隧道未启动');
      case 'checking':
        return t('tunnel.status.checking', '检查中...');
      default:
        return t('tunnel.status.unknown', '未知状态');
    }
  };

  const getClickHint = () => {
    switch (tunnelInfo.state) {
      case 'running':
        return t('tunnel.clickToShowDetails', '点击查看连接信息');
      case 'idle':
        return t('tunnel.clickToStart', '点击启动隧道');
      case 'not_installed':
        return t('tunnel.clickToInstall', '点击了解安装');
      case 'error':
        return t('tunnel.clickToFix', '点击查看详情');
      case 'starting':
      case 'checking':
        return t('tunnel.pleaseWait', '请稍候...');
      default:
        return '';
    }
  };

  const handleClick = async () => {
    switch (tunnelInfo.state) {
      case 'running':
        // Show connection modal
        setShowModal(true);
        break;

      case 'idle':
        // Directly start tunnel
        await handleStartTunnel();
        break;

      case 'not_installed':
        // Show install explanation modal
        setShowInstallModal(true);
        break;

      case 'error':
        // Navigate to settings for troubleshooting
        navigate('/settings', { state: { section: 'tunnel' } });
        break;

      case 'starting':
      case 'checking':
        // Do nothing, wait
        break;
    }
  };

  const handleStartTunnel = async () => {
    setIsStarting(true);
    setTunnelInfo(prev => ({ ...prev, state: 'starting' }));

    try {
      const result = await window.electron.cloudflaredStart();
      if (result.success && result.data) {
        setTunnelInfo(result.data as TunnelInfo);
        // Auto show connection modal after successful start
        setShowModal(true);
      } else {
        setTunnelInfo(prev => ({
          ...prev,
          state: 'error',
          error: result.error || 'Failed to start tunnel'
        }));
      }
    } catch (err) {
      setTunnelInfo(prev => ({
        ...prev,
        state: 'error',
        error: String(err)
      }));
    } finally {
      setIsStarting(false);
    }
  };

  const handleStopTunnel = async () => {
    try {
      const result = await window.electron.cloudflaredStop();
      if (result.success) {
        setTunnelInfo({ state: 'idle', url: '', hostname: '', secret: '' });
        setShowModal(false);
      }
    } catch (err) {
      console.error('Failed to stop tunnel:', err);
    }
  };

  const handleGoToInstall = () => {
    setShowInstallModal(false);
    navigate('/settings', { state: { section: 'tunnel' } });
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

  const getWebAccessUrl = () => {
    if (tunnelInfo.state !== 'running' || !tunnelInfo.url || !tunnelInfo.secret) return '';
    return `${tunnelInfo.url}/web/#secret=${tunnelInfo.secret}`;
  };

  const getMobileQRCodeData = () => {
    return getWebAccessUrl();
  };

  const isLoading = tunnelInfo.state === 'starting' || tunnelInfo.state === 'checking' || isStarting;

  return (
    <>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            onClick={handleClick}
            disabled={isLoading}
            className={`p-1.5 rounded-md transition-colors hover:bg-black/5 dark:hover:bg-white/5 ${getStatusColor()} ${isLoading ? 'cursor-wait' : ''}`}
            aria-label={getStatusText()}
          >
            {isLoading ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              <Radio className="w-4 h-4" />
            )}
          </button>
        </TooltipTrigger>
        <TooltipContent side="top">
          <p className="text-xs font-medium">{getStatusText()}</p>
          <p className="text-[10px] text-text-muted mt-0.5">{getClickHint()}</p>
        </TooltipContent>
      </Tooltip>

      {/* Install Explanation Modal */}
      <Dialog open={showInstallModal} onOpenChange={setShowInstallModal}>
        <DialogContent className="sm:max-w-[420px]">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Download className="w-5 h-5 text-teal-500" />
              {t('tunnel.installModal.title', '需要安装 Cloudflared')}
            </DialogTitle>
            <DialogDescription>
              {t('tunnel.installModal.description', '远程访问功能需要 Cloudflared 组件支持')}
            </DialogDescription>
          </DialogHeader>

          <div className="py-4 space-y-4">
            <div className="p-4 bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-lg">
              <div className="flex gap-3">
                <AlertCircle className="w-5 h-5 text-blue-500 flex-shrink-0 mt-0.5" />
                <div className="text-sm text-blue-800 dark:text-blue-200">
                  <p className="font-medium mb-1">{t('tunnel.installModal.whatIs', '什么是 Cloudflared？')}</p>
                  <p className="text-xs opacity-80">
                    {t('tunnel.installModal.explanation', 'Cloudflared 是 Cloudflare 提供的安全隧道工具，可以让您从任何设备远程访问 AGIME，无需复杂的网络配置。')}
                  </p>
                </div>
              </div>
            </div>

            <div className="space-y-2 text-sm text-text-muted">
              <p>✓ {t('tunnel.installModal.benefit1', '一键安装，自动配置')}</p>
              <p>✓ {t('tunnel.installModal.benefit2', '安全加密连接')}</p>
              <p>✓ {t('tunnel.installModal.benefit3', '支持手机扫码访问')}</p>
            </div>
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={() => setShowInstallModal(false)}>
              {t('tunnel.buttons.later', '稍后再说')}
            </Button>
            <Button onClick={handleGoToInstall}>
              <Download className="w-4 h-4 mr-2" />
              {t('tunnel.buttons.goToInstall', '前往安装')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Connection Modal */}
      <Dialog open={showModal} onOpenChange={setShowModal}>
        <DialogContent className="sm:max-w-[550px]">
          <DialogHeader>
            <DialogTitle>{t('tunnel.connectionModal.title', '远程访问连接')}</DialogTitle>
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
                  {t('tunnel.tabs.webAccess', '网页访问')}
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
                  {t('tunnel.tabs.mobileApp', '移动端')}
                </button>
              </div>

              {/* Web Access Tab */}
              {activeTab === 'web' && (
                <div className="space-y-4">
                  <div className="text-center text-sm text-text-muted">
                    {t('tunnel.connectionModal.webHint', '复制以下链接，可在任何浏览器中访问 AGIME。链接已包含认证信息。')}
                  </div>

                  <div className="p-4 bg-background-muted rounded-lg space-y-3">
                    <div>
                      <h3 className="text-xs font-medium mb-2">{t('tunnel.connectionModal.webAccessUrl', 'Web 访问链接（已包含认证信息）')}</h3>
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
                        variant="outline"
                        onClick={() => window.open(getWebAccessUrl(), '_blank')}
                      >
                        <ExternalLink className="h-4 w-4 mr-2" />
                        {t('tunnel.openInBrowser', '在浏览器中打开')}
                      </Button>
                    </div>
                  </div>

                  {/* Advanced options */}
                  <div className="border-t pt-4">
                    <button
                      onClick={() => setShowDetails(!showDetails)}
                      className="flex items-center justify-between w-full text-sm font-medium hover:opacity-70 transition-opacity"
                    >
                      <span>{t('tunnel.connectionModal.connectionDetails', '高级：连接详情')}</span>
                      {showDetails ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
                    </button>

                    {showDetails && (
                      <div className="mt-3 space-y-3">
                        <div>
                          <h3 className="text-xs font-medium mb-1 text-text-muted">{t('tunnel.connectionModal.tunnelUrl', '隧道 URL')}</h3>
                          <div className="flex items-center gap-2">
                            <code className="flex-1 p-2 bg-background-muted rounded-lg text-xs break-all overflow-hidden">
                              {tunnelInfo.url}
                            </code>
                            <Button
                              size="sm"
                              variant="ghost"
                              onClick={() => copyToClipboard(tunnelInfo.url, 'url')}
                            >
                              {copiedUrl ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                            </Button>
                          </div>
                        </div>

                        <div>
                          <h3 className="text-xs font-medium mb-1 text-text-muted">{t('tunnel.connectionModal.secretKey', '密钥')}</h3>
                          <div className="flex items-center gap-2">
                            <code className="flex-1 p-2 bg-background-muted rounded-lg text-xs break-all overflow-hidden">
                              {tunnelInfo.secret}
                            </code>
                            <Button
                              size="sm"
                              variant="ghost"
                              onClick={() => copyToClipboard(tunnelInfo.secret, 'secret')}
                            >
                              {copiedSecret ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
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
                    {t('tunnel.connectionModal.scanHint', '使用手机扫描二维码，在移动浏览器中访问 AGIME')}
                  </div>
                </div>
              )}
            </div>
          )}

          <DialogFooter>
            <Button variant="outline" onClick={() => setShowModal(false)}>
              {t('tunnel.buttons.close', '关闭')}
            </Button>
            <Button variant="destructive" onClick={handleStopTunnel}>
              {t('tunnel.buttons.stopTunnel', '停止隧道')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
