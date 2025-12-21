import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Switch } from '../../ui/switch';
import { Brain } from 'lucide-react';
import {
  getThinkingConfig,
  setThinkingConfig,
  getCapableModels,
  type ThinkingConfigResponse,
  type CapableModelsResponse,
} from '../../../services/capabilities';

export const ThinkingModeToggle = () => {
  const { t } = useTranslation('settings');
  const [config, setConfig] = useState<ThinkingConfigResponse>({ enabled: false, budget: null });
  const [models, setModels] = useState<CapableModelsResponse>({ thinking_models: [], reasoning_models: [] });
  const [budgetInput, setBudgetInput] = useState('16000');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadConfig = useCallback(async () => {
    try {
      setLoading(true);
      const [configResponse, modelsResponse] = await Promise.all([
        getThinkingConfig(),
        getCapableModels(),
      ]);
      setConfig(configResponse);
      setModels(modelsResponse);
      setBudgetInput(configResponse.budget?.toString() || '16000');
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load thinking config');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadConfig();
  }, [loadConfig]);

  const handleToggle = async (enabled: boolean) => {
    try {
      await setThinkingConfig({
        enabled,
        budget: enabled ? parseInt(budgetInput) || 16000 : undefined,
      });
      setConfig((prev) => ({ ...prev, enabled }));
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to update thinking config');
    }
  };

  const handleBudgetChange = async (budget: number) => {
    if (budget < 1024) budget = 1024;
    if (budget > 100000) budget = 100000;

    try {
      await setThinkingConfig({
        enabled: config.enabled,
        budget,
      });
      setConfig((prev) => ({ ...prev, budget }));
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to update thinking budget');
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-4">
        <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-text-muted"></div>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between py-2 px-2 hover:bg-background-muted rounded-lg transition-all">
        <div className="flex items-start gap-3">
          <Brain className="h-5 w-5 text-text-muted mt-0.5" />
          <div>
            <h3 className="text-text-default">{t('chat.thinking.title', 'Extended Thinking')}</h3>
            <p className="text-xs text-text-muted max-w-md mt-[2px]">
              {t('chat.thinking.description', 'Enable extended thinking mode for supported models. Allows the model to think through complex problems step by step.')}
            </p>
          </div>
        </div>
        <div className="flex items-center">
          <Switch checked={config.enabled} onCheckedChange={handleToggle} variant="mono" />
        </div>
      </div>

      {error && (
        <div className="px-2 py-2 text-sm text-red-500 bg-red-50 dark:bg-red-900/20 rounded-lg">
          {error}
        </div>
      )}

      <div
        className={`overflow-hidden transition-all duration-300 ease-in-out ${
          config.enabled ? 'max-h-96 opacity-100' : 'max-h-0 opacity-0'
        }`}
      >
        <div className="space-y-4 px-2 pb-2">
          <div className={config.enabled ? '' : 'opacity-50'}>
            <label
              className={`text-sm font-medium ${config.enabled ? 'text-text-default' : 'text-text-muted'}`}
            >
              {t('chat.thinking.budget', 'Thinking Budget (tokens)')}
            </label>
            <p className="text-xs text-text-muted mb-2">
              {t('chat.thinking.budgetDescription', 'Maximum number of tokens for thinking. Higher values allow more thorough reasoning but increase costs. Minimum: 1024')}
            </p>
            <input
              type="number"
              min={1024}
              max={100000}
              step={1000}
              value={budgetInput}
              onChange={(e) => {
                setBudgetInput(e.target.value);
              }}
              onBlur={(e) => {
                const value = parseInt(e.target.value);
                if (isNaN(value) || value < 1024) {
                  setBudgetInput('1024');
                  handleBudgetChange(1024);
                } else if (value > 100000) {
                  setBudgetInput('100000');
                  handleBudgetChange(100000);
                } else {
                  handleBudgetChange(value);
                }
              }}
              disabled={!config.enabled}
              className={`w-32 px-2 py-1 text-sm border rounded ${
                config.enabled
                  ? 'border-border-default bg-background-default text-text-default'
                  : 'border-border-muted bg-background-muted text-text-muted cursor-not-allowed'
              }`}
              placeholder="16000"
            />
          </div>

          {models.thinking_models.length > 0 && (
            <div className="mt-4">
              <label className="text-sm font-medium text-text-default">
                {t('chat.thinking.supportedModels', 'Supported Models')}
              </label>
              <p className="text-xs text-text-muted mt-1">
                {models.thinking_models.join(', ')}
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};
