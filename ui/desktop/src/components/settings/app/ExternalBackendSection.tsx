import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Switch } from '../../ui/switch';
import { Input } from '../../ui/input';
import { SettingsCard } from '../common';
import { AlertCircle, Server } from 'lucide-react';
import { isElectron } from '../../../platform';

interface ExternalAgimedConfig {
  enabled: boolean;
  url: string;
  secret: string;
}

interface Settings {
  externalAgimed?: Partial<ExternalAgimedConfig>;
}

const DEFAULT_CONFIG: ExternalAgimedConfig = {
  enabled: false,
  url: '',
  secret: '',
};

function parseConfig(partial: Partial<ExternalAgimedConfig> | undefined): ExternalAgimedConfig {
  return {
    enabled: partial?.enabled ?? DEFAULT_CONFIG.enabled,
    url: partial?.url ?? DEFAULT_CONFIG.url,
    secret: partial?.secret ?? DEFAULT_CONFIG.secret,
  };
}

export default function ExternalBackendSection() {
  // ExternalBackendSection is only available in Electron since it manages local settings
  if (!isElectron) {
    return null;
  }

  return <ExternalBackendSectionContent />;
}

function ExternalBackendSectionContent() {
  const { t } = useTranslation('settings');
  const [config, setConfig] = useState<ExternalAgimedConfig>(DEFAULT_CONFIG);
  const [isSaving, setIsSaving] = useState(false);
  const [urlError, setUrlError] = useState<string | null>(null);

  useEffect(() => {
    const loadSettings = async () => {
      const settings = (await window.electron.getSettings()) as Settings | null;
      setConfig(parseConfig(settings?.externalAgimed));
    };
    loadSettings();
  }, []);

  const validateUrl = (value: string): boolean => {
    if (!value) {
      setUrlError(null);
      return true;
    }
    try {
      const parsed = new URL(value);
      if (!['http:', 'https:'].includes(parsed.protocol)) {
        setUrlError(t('session.agimeServer.urlProtocolError'));
        return false;
      }
      setUrlError(null);
      return true;
    } catch {
      setUrlError(t('session.agimeServer.invalidUrlError'));
      return false;
    }
  };

  const saveConfig = async (newConfig: ExternalAgimedConfig): Promise<void> => {
    setIsSaving(true);
    try {
      const currentSettings = ((await window.electron.getSettings()) as Settings) || {};
      await window.electron.saveSettings({
        ...currentSettings,
        externalAgimed: newConfig,
      });
    } catch (error) {
      console.error('Failed to save external backend settings:', error);
    } finally {
      setIsSaving(false);
    }
  };

  const updateField = <K extends keyof ExternalAgimedConfig>(
    field: K,
    value: ExternalAgimedConfig[K]
  ) => {
    const newConfig = { ...config, [field]: value };
    setConfig(newConfig);
    return newConfig;
  };

  const handleUrlChange = (value: string) => {
    updateField('url', value);
    validateUrl(value);
  };

  const handleUrlBlur = async () => {
    if (validateUrl(config.url)) {
      await saveConfig(config);
    }
  };

  return (
    <SettingsCard
      icon={<Server className="h-5 w-5" />}
      title={t('session.agimeServer.title')}
      description={t('session.agimeServer.description')}
    >
      {/* Toggle for external server */}
      <div className="flex items-center justify-between py-2 px-2 hover:bg-background-muted rounded-lg transition-colors">
        <div className="flex-1">
          <h4 className="text-sm font-medium text-text-default leading-5">
            {t('session.agimeServer.useExternal')}
          </h4>
          <p className="text-xs text-text-muted mt-0.5 leading-4 max-w-md">
            {t('session.agimeServer.useExternalDescription')}
          </p>
        </div>
        <div className="flex-shrink-0">
          <Switch
            checked={config.enabled}
            onCheckedChange={(checked) => saveConfig(updateField('enabled', checked))}
            disabled={isSaving}
            variant="mono"
          />
        </div>
      </div>

      {/* Server configuration (only visible if enabled) */}
      {config.enabled && (
        <div className="space-y-3 px-2">
          <div className="space-y-2">
            <label htmlFor="external-url" className="text-sm font-medium text-text-default">
              {t('session.agimeServer.serverUrl')}
            </label>
            <Input
              id="external-url"
              type="url"
              placeholder="http://127.0.0.1:3000"
              value={config.url}
              onChange={(e) => handleUrlChange(e.target.value)}
              onBlur={handleUrlBlur}
              disabled={isSaving}
              className={urlError ? 'border-red-500' : ''}
            />
            {urlError && (
              <p className="text-xs text-red-500 flex items-center gap-1">
                <AlertCircle size={12} />
                {urlError}
              </p>
            )}
          </div>

          <div className="space-y-2">
            <label htmlFor="external-secret" className="text-sm font-medium text-text-default">
              {t('session.agimeServer.secretKey')}
            </label>
            <Input
              id="external-secret"
              type="password"
              placeholder={t('session.agimeServer.secretKeyPlaceholder')}
              value={config.secret}
              onChange={(e) => updateField('secret', e.target.value)}
              onBlur={() => saveConfig(config)}
              disabled={isSaving}
            />
            <p className="text-xs text-text-muted">
              {t('session.agimeServer.secretKeyDescription')}
            </p>
          </div>

          <div className="bg-amber-50 dark:bg-amber-950 border border-amber-200 dark:border-amber-800 rounded-lg p-3">
            <p className="text-xs text-amber-800 dark:text-amber-200">
              <strong>Note:</strong> {t('session.agimeServer.restartNote')}
            </p>
          </div>
        </div>
      )}
    </SettingsCard>
  );
}
