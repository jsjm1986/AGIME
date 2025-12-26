import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Switch } from '../../ui/switch';
import { useConfig } from '../../ConfigContext';
import { TELEMETRY_UI_ENABLED } from '../../../updates';
import TelemetryOptOutModal from '../../TelemetryOptOutModal';
import { toastService } from '../../../toasts';
import { buildAgimeKey, buildGooseKey } from '../../../utils/envCompat';
import { SettingsCard, SettingsToggleItem } from '../common';
import { Eye } from 'lucide-react';

const TELEMETRY_CONFIG_KEY = buildAgimeKey('TELEMETRY_ENABLED');

interface TelemetrySettingsProps {
  isWelcome: boolean;
}

export default function TelemetrySettings({ isWelcome = false }: TelemetrySettingsProps) {
  const { t } = useTranslation('settings');
  const { read, upsert } = useConfig();
  const [telemetryEnabled, setTelemetryEnabled] = useState(true);
  const [isLoading, setIsLoading] = useState(true);
  const [showModal, setShowModal] = useState(false);

  const loadTelemetryStatus = useCallback(async () => {
    try {
      // Try AGIME_TELEMETRY_ENABLED first, fallback to GOOSE_TELEMETRY_ENABLED
      let value = await read(TELEMETRY_CONFIG_KEY, false);
      if (value === null) {
        value = await read(buildGooseKey('TELEMETRY_ENABLED'), false);
      }
      setTelemetryEnabled(value === null ? true : Boolean(value));
    } catch (error) {
      console.error('Failed to load telemetry status:', error);
      toastService.error({
        title: t('telemetry.configError'),
        msg: t('telemetry.loadFailed'),
        traceback: error instanceof Error ? error.stack || '' : '',
      });
    } finally {
      setIsLoading(false);
    }
  }, [read, t]);

  useEffect(() => {
    loadTelemetryStatus();
  }, [loadTelemetryStatus]);

  const handleTelemetryToggle = async (checked: boolean) => {
    try {
      await upsert(TELEMETRY_CONFIG_KEY, checked, false);
      setTelemetryEnabled(checked);
    } catch (error) {
      console.error('Failed to update telemetry status:', error);
      toastService.error({
        title: t('telemetry.configError'),
        msg: t('telemetry.updateFailed'),
        traceback: error instanceof Error ? error.stack || '' : '',
      });
    }
  };

  const handleModalClose = () => {
    setShowModal(false);
    loadTelemetryStatus();
  };

  if (!TELEMETRY_UI_ENABLED) {
    return null;
  }

  const title = t('telemetry.title');
  const description = t('telemetry.description');
  const toggleLabel = t('telemetry.anonymousUsage');
  const toggleDescription = (
    <>
      {t('telemetry.anonymousUsageDescription')}{' '}
      <button
        onClick={() => setShowModal(true)}
        className="text-blue-600 dark:text-blue-400 hover:underline"
      >
        {t('telemetry.learnMore')}
      </button>
    </>
  );

  const modal = <TelemetryOptOutModal controlled isOpen={showModal} onClose={handleModalClose} />;

  if (isWelcome) {
    return (
      <>
        <div className="w-full p-4 sm:p-6 bg-transparent border border-background-hover rounded-xl">
          <h3 className="font-semibold text-base text-text-default mb-1">{title}</h3>
          <p className="text-text-muted text-sm mb-4">{description}</p>
          <div className="flex items-center justify-between py-2">
            <div className="flex-1">
              <h4 className="text-sm font-medium text-text-default">{toggleLabel}</h4>
              <p className="text-sm text-text-muted mt-0.5 max-w-md">
                {t('telemetry.anonymousUsageDescription')}{' '}
                <button
                  onClick={() => setShowModal(true)}
                  className="text-blue-600 dark:text-blue-400 hover:underline"
                >
                  {t('telemetry.learnMore')}
                </button>
              </p>
            </div>
            <Switch
              checked={telemetryEnabled}
              onCheckedChange={handleTelemetryToggle}
              disabled={isLoading}
              variant="mono"
            />
          </div>
        </div>
        {modal}
      </>
    );
  }

  return (
    <>
      <SettingsCard
        icon={<Eye className="h-5 w-5" />}
        title={title}
        description={description}
      >
        <SettingsToggleItem
          title={toggleLabel}
          description={toggleDescription}
          checked={telemetryEnabled}
          onCheckedChange={handleTelemetryToggle}
          disabled={isLoading}
        />
      </SettingsCard>
      {modal}
    </>
  );
}
