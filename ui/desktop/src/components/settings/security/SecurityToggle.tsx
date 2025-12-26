import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Switch } from '../../ui/switch';
import { useConfig } from '../../ConfigContext';
import { SettingsCard } from '../common';
import { Shield } from 'lucide-react';

interface SecurityConfig {
  SECURITY_PROMPT_ENABLED?: boolean;
  SECURITY_PROMPT_THRESHOLD?: number;
}

interface SecurityToggleProps {
  asInline?: boolean;
}

export const SecurityToggle = ({ asInline = false }: SecurityToggleProps) => {
  const { t } = useTranslation('settings');
  const { config, upsert } = useConfig();

  const {
    SECURITY_PROMPT_ENABLED: enabled = false,
    SECURITY_PROMPT_THRESHOLD: configThreshold = 0.7,
  } = (config as SecurityConfig) ?? {};

  const [thresholdInput, setThresholdInput] = useState(configThreshold.toString());

  useEffect(() => {
    setThresholdInput(configThreshold.toString());
  }, [configThreshold]);

  const handleToggle = async (enabled: boolean) => {
    await upsert('SECURITY_PROMPT_ENABLED', enabled, false);
  };

  const handleThresholdChange = async (threshold: number) => {
    const validThreshold = Math.max(0, Math.min(1, threshold));
    await upsert('SECURITY_PROMPT_THRESHOLD', validThreshold, false);
  };

  const content = (
    <>
      {/* 开关控制 */}
      <div className="flex items-center justify-between py-2 px-2 hover:bg-background-muted rounded-lg transition-colors">
        <div className="flex-1">
          <h4 className="text-sm font-medium text-text-default leading-5">
            {t('chat.promptInjection.title')}
          </h4>
          <p className="text-xs text-text-muted mt-0.5 leading-4 max-w-md">
            {t('chat.promptInjection.thresholdDescription')}
          </p>
        </div>
        <Switch checked={enabled} onCheckedChange={handleToggle} variant="mono" />
      </div>

      {/* 展开内容 */}
      <div
        className={`overflow-hidden transition-all duration-300 ease-in-out ${
          enabled ? 'max-h-96 opacity-100' : 'max-h-0 opacity-0'
        }`}
      >
        <div className="space-y-3 px-2 pt-2">
          <div className={enabled ? '' : 'opacity-50'}>
            <label className="text-xs text-text-muted mb-1 block">
              {t('chat.promptInjection.threshold')}
            </label>
            <input
              type="number"
              min={0.01}
              max={1.0}
              step={0.01}
              value={thresholdInput}
              onChange={(e) => {
                setThresholdInput(e.target.value);
              }}
              onBlur={(e) => {
                const value = parseFloat(e.target.value);
                if (isNaN(value) || value < 0.01 || value > 1.0) {
                  // Revert to previous valid value
                  setThresholdInput(configThreshold.toString());
                } else {
                  handleThresholdChange(value);
                }
              }}
              disabled={!enabled}
              className={`w-24 px-3 py-1.5 text-sm border rounded-lg transition-colors ${
                enabled
                  ? 'border-border-default bg-background-default text-text-default focus:border-block-teal focus:outline-none'
                  : 'border-border-muted bg-background-muted text-text-muted cursor-not-allowed'
              }`}
              placeholder="0.70"
            />
          </div>
        </div>
      </div>
    </>
  );

  if (asInline) {
    return content;
  }

  return (
    <SettingsCard
      icon={<Shield className="h-5 w-5" />}
      title={t('chat.promptInjection.title')}
      description={t('chat.promptInjection.description')}
    >
      {content}
    </SettingsCard>
  );
};
