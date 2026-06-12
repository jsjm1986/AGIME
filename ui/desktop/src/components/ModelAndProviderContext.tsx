import React, { createContext, useContext, useState, useEffect, useMemo, useCallback } from 'react';
import { toastError, toastSuccess } from '../toasts';
import Model, { getProviderMetadata } from './settings/models/modelInterface';
import { ProviderMetadata, setConfigProvider, updateAgentProvider } from '../api';
import { useConfig } from './ConfigContext';
import {
  getModelDisplayName,
  getProviderDisplayName,
} from './settings/models/predefinedModelsUtils';
import { getConfigCompat } from '../utils/envCompat';

// titles
export const UNKNOWN_PROVIDER_TITLE = 'Provider name lookup';

// errors
export const UNKNOWN_PROVIDER_MSG = 'Unknown provider in config -- please inspect your config.yaml';

// success
const CHANGE_MODEL_TOAST_TITLE = 'Model changed';
const SWITCH_MODEL_SUCCESS_MSG = 'Successfully switched models';

// config.yaml key holding the per-model multimodal map: { "<model_name>": bool }.
// The backend (ModelConfig::parse_supports_multimodal) reads this same key,
// keyed by model name, to decide whether image content is sent to the model.
export const MODEL_MULTIMODAL_KEY = 'AGIME_MODEL_MULTIMODAL';

interface ModelAndProviderContextType {
  currentModel: string | null;
  currentProvider: string | null;
  changeModel: (sessionId: string | null, model: Model) => Promise<void>;
  getCurrentModelAndProvider: () => Promise<{ model: string; provider: string }>;
  getFallbackModelAndProvider: () => Promise<{ model: string; provider: string }>;
  getCurrentModelAndProviderForDisplay: () => Promise<{ model: string; provider: string }>;
  getCurrentModelDisplayName: () => Promise<string>;
  getCurrentProviderDisplayName: () => Promise<string>; // Gets provider display name from subtext
  refreshCurrentModelAndProvider: () => Promise<void>;
}

interface ModelAndProviderProviderProps {
  children: React.ReactNode;
}

const ModelAndProviderContext = createContext<ModelAndProviderContextType | undefined>(undefined);

export const ModelAndProviderProvider: React.FC<ModelAndProviderProviderProps> = ({ children }) => {
  const [currentModel, setCurrentModel] = useState<string | null>(null);
  const [currentProvider, setCurrentProvider] = useState<string | null>(null);
  const { read, upsert, getProviders } = useConfig();

  const changeModel = useCallback(
    async (sessionId: string | null, model: Model) => {
      const modelName = model.name;
      const providerName = model.provider;
      let phase = 'agent';

      try {
        // Persist this model's multimodal capability into the per-model map
        // (keyed by model name) so the backend honors it when building the
        // ModelConfig for this model. Read-modify-write to preserve other
        // models' entries. A failure here must not block the model switch.
        if (typeof model.supportsMultimodal === 'boolean') {
          try {
            const existing = (await read(MODEL_MULTIMODAL_KEY, false)) as Record<
              string,
              unknown
            > | null;
            const map =
              existing && typeof existing === 'object' && !Array.isArray(existing)
                ? { ...existing }
                : {};
            map[modelName] = model.supportsMultimodal;
            await upsert(MODEL_MULTIMODAL_KEY, map, false);
          } catch (mmError) {
            console.error('Failed to persist per-model multimodal flag', mmError);
          }
        }

        if (sessionId) {
          await updateAgentProvider({
            body: {
              session_id: sessionId,
              provider: providerName,
              model: modelName,
            },
          });
        }

        phase = 'config';
        await setConfigProvider({
          body: {
            provider: providerName,
            model: modelName,
          },
          throwOnError: true,
        });

        setCurrentProvider(providerName);
        setCurrentModel(modelName);

        toastSuccess({
          title: CHANGE_MODEL_TOAST_TITLE,
          msg: `${SWITCH_MODEL_SUCCESS_MSG} -- using ${model.alias ?? modelName} from ${model.subtext ?? providerName}`,
        });
      } catch (error) {
        console.error(`Failed to change model at ${phase} step -- ${modelName} ${providerName}`);
        toastError({
          title: `${providerName}/${modelName} failed`,
          msg: `${error}`,
          traceback: error instanceof Error ? error.message : String(error),
        });
      }
    },
    [read, upsert]
  );

  const getFallbackModelAndProvider = useCallback(async () => {
    const provider = getConfigCompat('DEFAULT_PROVIDER') as string;
    const model = getConfigCompat('DEFAULT_MODEL') as string;
    if (provider && model) {
      try {
        await setConfigProvider({
          body: {
            provider: provider,
            model: model,
          },
          throwOnError: true,
        });
      } catch (error) {
        console.error('[getFallbackModelAndProvider] Failed to write to config', error);
      }
    }
    return { model: model, provider: provider };
  }, []);

  const getCurrentModelAndProvider = useCallback(async () => {
    let model: string;
    let provider: string;

    // read from config
    try {
      model = (await read('AGIME_MODEL', false)) as string;
      provider = (await read('AGIME_PROVIDER', false)) as string;
    } catch {
      console.error(`Failed to read AGIME_MODEL or AGIME_PROVIDER from config`);
      throw new Error('Failed to read AGIME_MODEL or AGIME_PROVIDER from config');
    }
    if (!model || !provider) {
      console.log('[getCurrentModelAndProvider] Checking app environment as fallback');
      return getFallbackModelAndProvider();
    }
    return { model: model, provider: provider };
  }, [read, getFallbackModelAndProvider]);

  const getCurrentModelAndProviderForDisplay = useCallback(async () => {
    const modelProvider = await getCurrentModelAndProvider();
    const gooseModel = modelProvider.model;
    const gooseProvider = modelProvider.provider;

    // lookup display name
    let metadata: ProviderMetadata;

    try {
      metadata = await getProviderMetadata(String(gooseProvider), getProviders);
    } catch {
      return { model: gooseModel, provider: gooseProvider };
    }
    const providerDisplayName = metadata.display_name;

    return { model: gooseModel, provider: providerDisplayName };
  }, [getCurrentModelAndProvider, getProviders]);

  const getCurrentModelDisplayName = useCallback(async () => {
    try {
      const currentModelName = (await read('AGIME_MODEL', false)) as string;
      return getModelDisplayName(currentModelName);
    } catch {
      return 'Select Model';
    }
  }, [read]);

  const getCurrentProviderDisplayName = useCallback(async () => {
    try {
      const currentModelName = (await read('AGIME_MODEL', false)) as string;
      const providerDisplayName = getProviderDisplayName(currentModelName);
      if (providerDisplayName) {
        return providerDisplayName;
      }
      // Fall back to regular provider display name lookup
      const { provider } = await getCurrentModelAndProviderForDisplay();
      return provider;
    } catch {
      return '';
    }
  }, [read, getCurrentModelAndProviderForDisplay]);

  const refreshCurrentModelAndProvider = useCallback(async () => {
    try {
      const { model, provider } = await getCurrentModelAndProvider();
      setCurrentModel(model);
      setCurrentProvider(provider);
    } catch (_error) {
      console.error('Failed to refresh current model and provider:', _error);
    }
  }, [getCurrentModelAndProvider]);

  // Load initial model and provider on mount
  useEffect(() => {
    refreshCurrentModelAndProvider();
  }, [refreshCurrentModelAndProvider]);

  const contextValue = useMemo(
    () => ({
      currentModel,
      currentProvider,
      changeModel,
      getCurrentModelAndProvider,
      getFallbackModelAndProvider,
      getCurrentModelAndProviderForDisplay,
      getCurrentModelDisplayName,
      getCurrentProviderDisplayName,
      refreshCurrentModelAndProvider,
    }),
    [
      currentModel,
      currentProvider,
      changeModel,
      getCurrentModelAndProvider,
      getFallbackModelAndProvider,
      getCurrentModelAndProviderForDisplay,
      getCurrentModelDisplayName,
      getCurrentProviderDisplayName,
      refreshCurrentModelAndProvider,
    ]
  );

  return (
    <ModelAndProviderContext.Provider value={contextValue}>
      {children}
    </ModelAndProviderContext.Provider>
  );
};

export const useModelAndProvider = () => {
  const context = useContext(ModelAndProviderContext);
  if (context === undefined) {
    throw new Error('useModelAndProvider must be used within a ModelAndProviderProvider');
  }
  return context;
};
