import { useEffect, useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useConfig } from '../../ConfigContext';
import { getApiUrl } from '../../../config';

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

    setError(null); // Clear any previous errors
    setIsLoading(true);

    try {
      // First update the configuration
      try {
        await upsert('GOOSE_ENABLE_ROUTER', enableRouter.toString(), false);
      } catch (error) {
        console.error('Error updating configuration:', error);
        setError(`${t('toolSelection.errors.updateConfig')}: ${error}`);
        setIsLoading(false);
        return;
      }

      // Then update the backend
      try {
        const response = await fetch(getApiUrl('/agent/update_router_tool_selector'), {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
            'X-Secret-Key': await window.electron.getSecretKey(),
          },
          body: JSON.stringify({
            session_id: '', // TODO(jack) add the session id, or remove from this request payload
          }),
        });

        if (!response.ok) {
          const errorData = await response
            .json()
            .catch(() => ({ error: t('toolSelection.errors.unknownBackend') }));
          throw new Error(errorData.error || t('toolSelection.errors.unknownBackend'));
        }

        // Parse the success response
        const data = await response
          .json()
          .catch(() => ({ message: t('toolSelection.errors.successMessage') }));
        if (data.error) {
          throw new Error(data.error);
        }
      } catch (error) {
        console.error('Error updating backend:', error);
        setError(`${t('toolSelection.errors.updateBackend')}: ${error}`);
        setIsLoading(false);
        return;
      }

      // If both succeeded, update the UI
      setRouterEnabled(enableRouter);
    } catch (error) {
      console.error('Error updating tool selection strategy:', error);
      setError(`${t('toolSelection.errors.updateStrategy')}: ${error}`);
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
  }, [read]);

  useEffect(() => {
    fetchCurrentStrategy();
  }, [fetchCurrentStrategy]);

  return (
    <div className="space-y-1">
      {all_tool_selection_strategies.map((strategy) => (
        <div className="group hover:cursor-pointer text-sm" key={strategy.key.toString()}>
          <div
            className={`flex items-center justify-between text-text-default py-2 px-2 ${routerEnabled === strategy.key ? 'bg-background-muted' : 'bg-background-default hover:bg-background-muted'} rounded-lg transition-all`}
            onClick={() => handleStrategyChange(strategy.key)}
          >
            <div className="flex">
              <div>
                <h3 className="text-sm text-text-default">{strategy.label}</h3>
                <p className="text-xs text-text-muted mt-[2px]">{strategy.description}</p>
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
