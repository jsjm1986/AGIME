import { useState, useEffect, useMemo, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Input } from '../../ui/input';
import { Button } from '../../ui/button';
import { useConfig } from '../../ConfigContext';
import { cn } from '../../../utils';
import { Save, RotateCcw, FileText, Settings } from 'lucide-react';
import { toastSuccess, toastError } from '../../../toasts';
import { providerPrefixes } from '../../../utils/configUtils';
import type { ConfigData, ConfigValue } from '../../../types/config';
import { SettingsCard } from '../common';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '../../ui/dialog';

export default function ConfigSettings() {
  const { t } = useTranslation('settings');
  const { config, upsert } = useConfig();
  const typedConfig = config as ConfigData;
  const [configValues, setConfigValues] = useState<ConfigData>({});
  const [modifiedKeys, setModifiedKeys] = useState<Set<string>>(new Set());
  const [saving, setSaving] = useState<string | null>(null);
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [originalKeyOrder, setOriginalKeyOrder] = useState<string[]>([]);

  // Helper function to get translated config label
  const getConfigLabel = useCallback((key: string): string => {
    const translatedLabel = t(`config.labels.${key}`, { defaultValue: '' });
    if (translatedLabel) {
      return translatedLabel;
    }
    // Fallback: convert key to title case
    return key
      .split('_')
      .map((word) => word.charAt(0) + word.slice(1).toLowerCase())
      .join(' ');
  }, [t]);

  useEffect(() => {
    setConfigValues(typedConfig);
    setModifiedKeys(new Set());

    // Capture the original key order only on first load or when new keys are added
    const currentKeys = Object.keys(typedConfig);
    setOriginalKeyOrder((prevOrder) => {
      if (prevOrder.length === 0) {
        // First load - capture the initial order
        return currentKeys;
      } else if (currentKeys.length > prevOrder.length) {
        // New keys have been added - add them to the end while preserving existing order
        const newKeys = currentKeys.filter((key) => !prevOrder.includes(key));
        return [...prevOrder, ...newKeys];
      }
      // Don't reorder when keys are just updated/saved - preserve the original order
      return prevOrder;
    });
  }, [typedConfig]);

  const handleChange = (key: string, value: string) => {
    setConfigValues((prev: ConfigData) => ({
      ...prev,
      [key]: value,
    }));

    setModifiedKeys((prev) => {
      const newSet = new Set(prev);
      if (value !== String(typedConfig[key] || '')) {
        newSet.add(key);
      } else {
        newSet.delete(key);
      }
      return newSet;
    });
  };

  const handleSave = async (key: string) => {
    setSaving(key);
    try {
      await upsert(key, configValues[key], false);
      toastSuccess({
        title: t('config.toasts.updated'),
        msg: t('config.toasts.savedSuccess', { key: getConfigLabel(key) }),
      });

      // Remove this key from modified keys since it's now saved
      setModifiedKeys((prev) => {
        const newSet = new Set(prev);
        newSet.delete(key);
        return newSet;
      });
    } catch (error) {
      console.error('Failed to save config:', error);
      toastError({
        title: t('config.toasts.saveFailed'),
        msg: t('config.toasts.saveFailedMsg', { key: getConfigLabel(key) }),
        traceback: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setSaving(null);
    }
  };

  const handleReset = () => {
    setConfigValues(typedConfig);
    setModifiedKeys(new Set());
    toastSuccess({
      title: t('config.toasts.reset'),
      msg: t('config.toasts.resetMsg'),
    });
  };

  const handleModalClose = (open: boolean) => {
    if (!open && modifiedKeys.size > 0) {
      // Reset any unsaved changes when closing the modal
      setConfigValues(typedConfig);
      setModifiedKeys(new Set());
    }
    setIsModalOpen(open);
  };

  // Use AGIME_PROVIDER (preferred) with fallback to GOOSE_PROVIDER (legacy)
  const currentProvider = typedConfig.AGIME_PROVIDER || typedConfig.GOOSE_PROVIDER || '';

  const configEntries: [string, ConfigValue][] = useMemo(() => {
    const currentProviderPrefixes = providerPrefixes[currentProvider] || [];
    const allProviderPrefixes = Object.values(providerPrefixes).flat();

    // Legacy GOOSE_* keys that should be hidden (migrated to AGIME_* equivalents)
    const legacyGooseKeys = [
      'GOOSE_PROVIDER',
      'GOOSE_MODEL',
      'GOOSE_MODE',
      'GOOSE_TEMPERATURE',
      'GOOSE_THINKING_ENABLED',
      'GOOSE_THINKING_BUDGET',
      'GOOSE_LEAD_PROVIDER',
      'GOOSE_LEAD_MODEL',
      'GOOSE_LEAD_TURNS',
      'GOOSE_LEAD_FALLBACK_TURNS',
      'GOOSE_PLANNER_PROVIDER',
      'GOOSE_PLANNER_MODEL',
      'GOOSE_TOOLSHIM',
      'GOOSE_TOOLSHIM_OLLAMA_MODEL',
      'GOOSE_CLI_MIN_PRIORITY',
      'GOOSE_ALLOWLIST',
      'GOOSE_RECIPE_GITHUB_REPO',
      'GOOSE_ENABLE_ROUTER',
      'GOOSE_TELEMETRY_ENABLED',
      'GOOSE_MAX_TURNS',
      'GOOSE_CUSTOM_SYSTEM_PROMPT',
      'GOOSE_CUSTOM_PROMPT_ENABLED',
    ];

    return originalKeyOrder
      .filter((key) => {
        // Skip secrets
        if (key === 'extensions' || key.includes('_KEY') || key.includes('_TOKEN')) {
          return false;
        }

        // Skip legacy GOOSE_* keys - they have AGIME_* equivalents
        if (legacyGooseKeys.includes(key)) {
          return false;
        }

        // Only show provider-specific entries for the current provider
        const providerSpecific = allProviderPrefixes.some((prefix: string) =>
          key.startsWith(prefix)
        );
        if (providerSpecific) {
          return currentProviderPrefixes.some((prefix: string) => key.startsWith(prefix));
        }

        return true;
      })
      .map((key) => [key, configValues[key]]);
  }, [originalKeyOrder, configValues, currentProvider]);

  return (
    <SettingsCard
      icon={<FileText className="h-5 w-5" />}
      title={t('config.title')}
      description={
        currentProvider
          ? t('config.descriptionWithProvider', { provider: currentProvider })
          : t('config.description')
      }
    >
      <Dialog open={isModalOpen} onOpenChange={handleModalClose}>
        <DialogTrigger asChild>
          <Button className="flex items-center gap-2" variant="secondary" size="sm">
            <Settings className="h-4 w-4" />
            {t('config.editConfiguration')}
          </Button>
        </DialogTrigger>
        <DialogContent className="max-w-4xl max-h-[80vh]">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <FileText className="text-iconStandard" size={20} />
              {t('config.configurationEditor')}
            </DialogTitle>
            <DialogDescription>
              {currentProvider
                ? t('config.descriptionWithProvider', { provider: currentProvider })
                : t('config.description')}
            </DialogDescription>
          </DialogHeader>

          <div className="flex-1 max-h-[60vh] overflow-auto pr-4">
            <div className="space-y-4">
              {configEntries.length === 0 ? (
                <p className="text-textSubtle">{t('config.noSettings')}</p>
              ) : (
                configEntries.map(([key, _value]) => (
                  <div key={key} className="grid grid-cols-[200px_1fr_auto] gap-3 items-center">
                    <label className="text-sm font-medium text-textStandard" title={key}>
                      {getConfigLabel(key)}
                    </label>
                    <Input
                      value={String(configValues[key] || '')}
                      onChange={(e) => handleChange(key, e.target.value)}
                      className={cn(
                        'text-textStandard border-borderSubtle hover:border-borderStandard transition-colors',
                        modifiedKeys.has(key) && 'border-blue-500 focus:ring-blue-500/20'
                      )}
                      placeholder={t('config.enterValue', { key: getConfigLabel(key), defaultValue: `Enter ${getConfigLabel(key)}` })}
                    />
                    <Button
                      onClick={() => handleSave(key)}
                      disabled={!modifiedKeys.has(key) || saving === key}
                      variant="ghost"
                      size="sm"
                      className="min-w-[60px]"
                    >
                      {saving === key ? (
                        <span className="text-xs">{t('config.saving')}</span>
                      ) : (
                        <Save className="h-4 w-4" />
                      )}
                    </Button>
                  </div>
                ))
              )}
            </div>
          </div>

          <DialogFooter className="gap-2">
            {modifiedKeys.size > 0 && (
              <Button onClick={handleReset} variant="outline">
                <RotateCcw className="h-4 w-4 mr-2" />
                {t('config.resetChanges')}
              </Button>
            )}
            <Button onClick={() => setIsModalOpen(false)} variant="default">
              {t('config.done')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsCard>
  );
}
