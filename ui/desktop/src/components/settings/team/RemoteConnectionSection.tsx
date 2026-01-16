import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Switch } from '../../ui/switch';
import { Input } from '../../ui/input';
import { Button } from '../../ui/button';
import { SettingsCard } from '../common';
import { AlertCircle, Globe, Check, Loader2, Wifi, Cloud } from 'lucide-react';
import { isElectron } from '../../../platform';
import {
  checkServiceHealth,
  setConnectionMode,
  clearRemoteConnection,
  TeamConnectionMode,
} from '../../team/api';

// localStorage keys (must match api.ts)
const STORAGE_KEYS = {
  CONNECTION_MODE: 'AGIME_TEAM_CONNECTION_MODE',
  LAN_SERVER_URL: 'AGIME_TEAM_LAN_SERVER_URL',
  LAN_SECRET_KEY: 'AGIME_TEAM_LAN_SECRET_KEY',
  CLOUD_SERVER_URL: 'AGIME_TEAM_SERVER_URL',
  CLOUD_API_KEY: 'AGIME_TEAM_API_KEY',
};

interface RemoteConnectionConfig {
  enabled: boolean;
  mode: 'lan' | 'cloud';
  lanUrl: string;
  lanSecretKey: string;
  cloudUrl: string;
  cloudApiKey: string;
}

const DEFAULT_CONFIG: RemoteConnectionConfig = {
  enabled: false,
  mode: 'lan',
  lanUrl: '',
  lanSecretKey: '',
  cloudUrl: '',
  cloudApiKey: '',
};

function loadConfigFromStorage(): RemoteConnectionConfig {
  try {
    const mode = localStorage.getItem(STORAGE_KEYS.CONNECTION_MODE) as TeamConnectionMode;
    const lanUrl = localStorage.getItem(STORAGE_KEYS.LAN_SERVER_URL) || '';
    const lanSecretKey = localStorage.getItem(STORAGE_KEYS.LAN_SECRET_KEY) || '';
    const cloudUrl = localStorage.getItem(STORAGE_KEYS.CLOUD_SERVER_URL) || '';
    const cloudApiKey = localStorage.getItem(STORAGE_KEYS.CLOUD_API_KEY) || '';

    return {
      enabled: mode === 'lan' || mode === 'cloud',
      mode: mode === 'cloud' ? 'cloud' : 'lan',
      lanUrl,
      lanSecretKey,
      cloudUrl,
      cloudApiKey,
    };
  } catch {
    return DEFAULT_CONFIG;
  }
}

function saveConfigToStorage(config: RemoteConnectionConfig): void {
  try {
    if (config.enabled) {
      setConnectionMode(config.mode);
      if (config.mode === 'lan') {
        localStorage.setItem(STORAGE_KEYS.LAN_SERVER_URL, config.lanUrl);
        localStorage.setItem(STORAGE_KEYS.LAN_SECRET_KEY, config.lanSecretKey);
      } else {
        localStorage.setItem(STORAGE_KEYS.CLOUD_SERVER_URL, config.cloudUrl);
        localStorage.setItem(STORAGE_KEYS.CLOUD_API_KEY, config.cloudApiKey);
      }
    } else {
      clearRemoteConnection();
    }
  } catch (error) {
    console.error('Failed to save remote connection config:', error);
  }
}

export default function RemoteConnectionSection() {
  if (!isElectron) {
    return null;
  }

  return <RemoteConnectionSectionContent />;
}

function RemoteConnectionSectionContent() {
  const { t } = useTranslation('settings');
  const [config, setConfig] = useState<RemoteConnectionConfig>(DEFAULT_CONFIG);
  const [urlError, setUrlError] = useState<string | null>(null);
  const [testStatus, setTestStatus] = useState<'idle' | 'testing' | 'success' | 'error'>('idle');
  const [testMessage, setTestMessage] = useState<string>('');

  useEffect(() => {
    setConfig(loadConfigFromStorage());
  }, []);

  const validateUrl = (value: string): boolean => {
    if (!value) {
      setUrlError(null);
      return true;
    }
    try {
      const parsed = new URL(value);
      if (!['http:', 'https:'].includes(parsed.protocol)) {
        setUrlError(t('team.remote.urlProtocolError', 'URL must use http or https protocol'));
        return false;
      }
      setUrlError(null);
      return true;
    } catch {
      setUrlError(t('team.remote.invalidUrlError', 'Invalid URL format'));
      return false;
    }
  };

  const updateConfig = (updates: Partial<RemoteConnectionConfig>) => {
    const newConfig = { ...config, ...updates };
    setConfig(newConfig);
    saveConfigToStorage(newConfig);
    setTestStatus('idle');
  };

  const handleEnabledChange = (enabled: boolean) => {
    updateConfig({ enabled });
  };

  const handleModeChange = (mode: 'lan' | 'cloud') => {
    updateConfig({ mode });
  };

  const handleUrlChange = (value: string, field: 'lanUrl' | 'cloudUrl') => {
    setConfig(prev => ({ ...prev, [field]: value }));
    validateUrl(value);
    setTestStatus('idle');
  };

  const handleUrlBlur = (field: 'lanUrl' | 'cloudUrl') => {
    const url = field === 'lanUrl' ? config.lanUrl : config.cloudUrl;
    if (validateUrl(url)) {
      saveConfigToStorage(config);
    }
  };

  const handleKeyChange = (value: string, field: 'lanSecretKey' | 'cloudApiKey') => {
    setConfig(prev => ({ ...prev, [field]: value }));
    setTestStatus('idle');
  };

  const handleKeyBlur = () => {
    saveConfigToStorage(config);
  };

  const testConnection = async () => {
    const url = config.mode === 'lan' ? config.lanUrl : config.cloudUrl;
    if (!url) {
      setTestStatus('error');
      setTestMessage(t('team.remote.noUrlError', 'Please enter a server URL'));
      return;
    }

    setTestStatus('testing');
    setTestMessage('');

    try {
      // Save current config to ensure test uses correct settings
      saveConfigToStorage(config);

      const health = await checkServiceHealth();

      if (health.online) {
        setTestStatus('success');
        setTestMessage(
          health.latency
            ? t('team.remote.connectedWithLatency', { latency: health.latency, defaultValue: `Connected (${health.latency}ms)` })
            : t('team.remote.connected', 'Connected successfully')
        );
      } else {
        setTestStatus('error');
        setTestMessage(health.error || t('team.remote.connectionFailed', 'Connection failed'));
      }
    } catch (error) {
      setTestStatus('error');
      setTestMessage(error instanceof Error ? error.message : t('team.remote.connectionFailed', 'Connection failed'));
    }
  };

  return (
    <SettingsCard
      icon={<Globe className="h-5 w-5" />}
      title={t('team.remote.title', 'Remote Connection')}
      description={t('team.remote.description', 'Connect to a remote team server for collaboration')}
    >
      {/* Toggle for remote connection */}
      <div className="flex items-center justify-between py-2 px-2 hover:bg-background-muted rounded-lg transition-colors">
        <div className="flex-1">
          <h4 className="text-sm font-medium text-text-default leading-5">
            {t('team.remote.enableRemote', 'Enable Remote Connection')}
          </h4>
          <p className="text-xs text-text-muted mt-0.5 leading-4 max-w-md">
            {t('team.remote.enableRemoteDescription', 'Connect to another AGIME instance or cloud team server')}
          </p>
        </div>
        <div className="flex-shrink-0">
          <Switch
            checked={config.enabled}
            onCheckedChange={handleEnabledChange}
            variant="mono"
          />
        </div>
      </div>

      {/* Connection configuration (only visible if enabled) */}
      {config.enabled && (
        <div className="space-y-4 px-2 mt-2">
          {/* Mode selector */}
          <div className="space-y-2">
            <label className="text-sm font-medium text-text-default">
              {t('team.remote.connectionMode', 'Connection Mode')}
            </label>
            <div className="flex gap-2">
              <Button
                variant={config.mode === 'lan' ? 'default' : 'outline'}
                size="sm"
                onClick={() => handleModeChange('lan')}
                className="flex-1"
              >
                <Wifi className="h-4 w-4 mr-2" />
                {t('team.remote.lanMode', 'LAN')}
              </Button>
              <Button
                variant={config.mode === 'cloud' ? 'default' : 'outline'}
                size="sm"
                onClick={() => handleModeChange('cloud')}
                className="flex-1"
              >
                <Cloud className="h-4 w-4 mr-2" />
                {t('team.remote.cloudMode', 'Cloud')}
              </Button>
            </div>
          </div>

          {/* LAN mode configuration */}
          {config.mode === 'lan' && (
            <div className="space-y-3 p-3 bg-background-muted rounded-lg">
              <div className="flex items-center gap-2 text-sm font-medium text-text-default">
                <Wifi className="h-4 w-4" />
                {t('team.remote.lanModeTitle', 'LAN Mode')}
              </div>
              <p className="text-xs text-text-muted">
                {t('team.remote.lanModeDescription', 'Connect to another AGIME instance on your local network')}
              </p>

              <div className="space-y-2">
                <label htmlFor="lan-server-url" className="text-sm font-medium text-text-default">
                  {t('team.remote.serverUrl', 'Server URL')}
                </label>
                <Input
                  id="lan-server-url"
                  type="url"
                  placeholder="http://192.168.1.100:3000"
                  value={config.lanUrl}
                  onChange={(e) => handleUrlChange(e.target.value, 'lanUrl')}
                  onBlur={() => handleUrlBlur('lanUrl')}
                  className={urlError && config.mode === 'lan' ? 'border-red-500' : ''}
                />
              </div>

              <div className="space-y-2">
                <label htmlFor="lan-secret-key" className="text-sm font-medium text-text-default">
                  {t('team.remote.secretKey', 'Secret Key')}
                </label>
                <Input
                  id="lan-secret-key"
                  type="password"
                  placeholder={t('team.remote.secretKeyPlaceholder', 'Enter the secret key from the host')}
                  value={config.lanSecretKey}
                  onChange={(e) => handleKeyChange(e.target.value, 'lanSecretKey')}
                  onBlur={handleKeyBlur}
                />
                <p className="text-xs text-text-muted">
                  {t('team.remote.secretKeyHint', 'Get this from the LAN Sharing section on the host computer')}
                </p>
              </div>
            </div>
          )}

          {/* Cloud mode configuration */}
          {config.mode === 'cloud' && (
            <div className="space-y-3 p-3 bg-background-muted rounded-lg">
              <div className="flex items-center gap-2 text-sm font-medium text-text-default">
                <Cloud className="h-4 w-4" />
                {t('team.remote.cloudModeTitle', 'Cloud Mode')}
              </div>
              <p className="text-xs text-text-muted">
                {t('team.remote.cloudModeDescription', 'Connect to a cloud-hosted team server')}
              </p>

              <div className="space-y-2">
                <label htmlFor="cloud-server-url" className="text-sm font-medium text-text-default">
                  {t('team.remote.serverUrl', 'Server URL')}
                </label>
                <Input
                  id="cloud-server-url"
                  type="url"
                  placeholder="https://team.agime.io"
                  value={config.cloudUrl}
                  onChange={(e) => handleUrlChange(e.target.value, 'cloudUrl')}
                  onBlur={() => handleUrlBlur('cloudUrl')}
                  className={urlError && config.mode === 'cloud' ? 'border-red-500' : ''}
                />
              </div>

              <div className="space-y-2">
                <label htmlFor="cloud-api-key" className="text-sm font-medium text-text-default">
                  {t('team.remote.apiKey', 'API Key')}
                </label>
                <Input
                  id="cloud-api-key"
                  type="password"
                  placeholder={t('team.remote.apiKeyPlaceholder', 'agime_xxx_...')}
                  value={config.cloudApiKey}
                  onChange={(e) => handleKeyChange(e.target.value, 'cloudApiKey')}
                  onBlur={handleKeyBlur}
                />
                <p className="text-xs text-text-muted">
                  {t('team.remote.apiKeyHint', 'Get your API key by registering on the team server')}
                </p>
              </div>
            </div>
          )}

          {/* URL error message */}
          {urlError && (
            <p className="text-xs text-red-500 flex items-center gap-1 px-1">
              <AlertCircle size={12} />
              {urlError}
            </p>
          )}

          {/* Test connection button */}
          <div className="flex items-center gap-3">
            <Button
              variant="outline"
              size="sm"
              onClick={testConnection}
              disabled={testStatus === 'testing'}
            >
              {testStatus === 'testing' ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  {t('team.remote.testing', 'Testing...')}
                </>
              ) : (
                t('team.remote.testConnection', 'Test Connection')
              )}
            </Button>

            {testStatus === 'success' && (
              <span className="text-sm text-green-600 dark:text-green-400 flex items-center gap-1">
                <Check size={14} />
                {testMessage}
              </span>
            )}

            {testStatus === 'error' && (
              <span className="text-sm text-red-600 dark:text-red-400 flex items-center gap-1">
                <AlertCircle size={14} />
                {testMessage}
              </span>
            )}
          </div>

          {/* Info banner */}
          <div className="bg-blue-50 dark:bg-blue-950 border border-blue-200 dark:border-blue-800 rounded-lg p-3">
            <p className="text-xs text-blue-800 dark:text-blue-200">
              {config.mode === 'lan'
                ? t('team.remote.lanInfo', 'LAN mode connects to another AGIME instance on your network. Both devices must be on the same network.')
                : t('team.remote.cloudInfo', 'Cloud mode connects to a dedicated team server. Your data will be stored on the remote server.')}
            </p>
          </div>
        </div>
      )}
    </SettingsCard>
  );
}
