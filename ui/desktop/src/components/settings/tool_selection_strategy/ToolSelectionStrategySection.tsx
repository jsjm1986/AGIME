import { useEffect, useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useConfig } from '../../ConfigContext';

export const ToolSelectionStrategySection = () => {
  const { t } = useTranslation('settings');
  const [routerEnabled, setRouterEnabled] = useState(false);
  const [_error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const { read, upsert } = useConfig();

  const all_tool_selection_strategies = [
    {
      key: false,
      label: t('toolSelection.disabled'),
      description: t('toolSelection.disabledDescription'),
    },
    {
      key: true,
      label: t('toolSelection.enabled'),
      description: t('toolSelection.enabledDescription'),
    },
  ];

  const handleStrategyChange = async (enableRouter: boolean) => {
    if (isLoading) return; // Prevent multiple simultaneous requests
    if (routerEnabled === enableRouter) return; // No change needed

    setError(null); // Clear any previous errors
    setIsLoading(true);
    setRouterEnabled(enableRouter); // Optimistic update - immediately update UI

    try {
      // Save configuration - this will apply on next session start
      await upsert('GOOSE_ENABLE_ROUTER', enableRouter.toString(), false);
      // Note: Backend update removed - sending empty session_id was causing
      // a new Agent to be created which triggered MCP extension loading.
      // The config change will take effect on the next session.
    } catch (error) {
      // Rollback if config save failed
      console.error('Error saving configuration:', error);
      setError(`${t('toolSelection.errors.updateConfig')}: ${error}`);
      setRouterEnabled(!enableRouter); // Rollback on config save error
    } finally {
      setIsLoading(false);
    }
  };

  const fetchCurrentStrategy = useCallback(async () => {
    try {
      const strategy = (await read('GOOSE_ENABLE_ROUTER', false)) as string;
      if (strategy) {
        setRouterEnabled(strategy === 'true');
      }
    } catch (error) {
      console.error('Error fetching current router setting:', error);
      setError(`${t('toolSelection.errors.fetchSetting')}: ${error}`);
    }
  }, [read, t]);

  useEffect(() => {
    fetchCurrentStrategy();
  }, [fetchCurrentStrategy]);

  return (
    <div className="space-y-2">
      {all_tool_selection_strategies.map((strategy) => (
        <div className="group hover:cursor-pointer text-sm" key={strategy.key.toString()}>
          <div
            className={`flex items-center justify-between text-text-default py-2 px-3 rounded-lg transition-all duration-200 ${
              routerEnabled === strategy.key
                ? 'bg-gray-100 dark:bg-background-muted shadow-[0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-none'
                : 'hover:bg-background-muted'
            }`}
            onClick={() => handleStrategyChange(strategy.key)}
          >
            <div className="flex">
              <div>
                <h3 className="text-sm font-medium text-text-default leading-5">{strategy.label}</h3>
                <p className="text-xs text-text-muted mt-0.5 leading-4">{strategy.description}</p>
              </div>
            </div>

            <div className="relative flex items-center gap-2">
              <input
                type="radio"
                name="tool-selection-strategy"
                value={strategy.key.toString()}
                checked={routerEnabled === strategy.key}
                onChange={() => handleStrategyChange(strategy.key)}
                disabled={isLoading}
                className="peer sr-only"
              />
              <div
                className="h-4 w-4 rounded-full border border-border-default
                      peer-checked:border-[6px] peer-checked:border-black dark:peer-checked:border-white
                      peer-checked:bg-white dark:peer-checked:bg-black
                      transition-all duration-200 ease-in-out group-hover:border-border-default"
              ></div>
            </div>
          </div>
        </div>
      ))}
    </div>
  );
};
