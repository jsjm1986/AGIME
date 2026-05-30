import { useEffect, useState, useCallback, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { View } from '../../../utils/navigationUtils';
import ModelSettingsButtons from './subcomponents/ModelSettingsButtons';
import { useConfig } from '../../ConfigContext';
import {
  UNKNOWN_PROVIDER_MSG,
  UNKNOWN_PROVIDER_TITLE,
  useModelAndProvider,
} from '../../ModelAndProviderContext';
import { useChatContext } from '../../../contexts/ChatContext';
import { updateAgentProvider } from '../../../api';
import { toastError, toastSuccess } from '../../../toasts';

import { SettingsCard } from '../common';
import { Switch } from '../../ui/switch';
import ResetProviderSection from '../reset_provider/ResetProviderSection';
import { Cpu, Image, RefreshCw, Zap } from 'lucide-react';
import { QuickSetupModal, type QuickSetupConfig } from '../quick-setup';

interface ModelsSectionProps {
  setView: (view: View) => void;
}

function interpretMultimodalValue(value: unknown): boolean {
  // Unset (config.yaml has no AGIME_MULTIMODAL key) means enabled by default,
  // mirroring the Rust default in ModelConfig::parse_supports_multimodal.
  if (value === null || value === undefined) {
    return true;
  }
  if (typeof value === 'boolean') {
    return value;
  }
  if (typeof value === 'number') {
    return value !== 0;
  }
  if (typeof value === 'string') {
    return !['0', 'false', 'no', 'off'].includes(value.trim().toLowerCase());
  }
  return true;
}

export default function ModelsSection({ setView }: ModelsSectionProps) {
  const { t } = useTranslation('settings');
  const [provider, setProvider] = useState<string | null>(null);
  const [displayModelName, setDisplayModelName] = useState<string>('');
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [showQuickSetup, setShowQuickSetup] = useState<boolean>(false);
  const [multimodalEnabled, setMultimodalEnabled] = useState<boolean>(true);
  const [multimodalSaving, setMultimodalSaving] = useState<boolean>(false);
  const { read, upsert, getProviders } = useConfig();
  const chatContext = useChatContext();
  const {
    getCurrentModelDisplayName,
    getCurrentProviderDisplayName,
    currentModel,
    currentProvider,
    refreshCurrentModelAndProvider,
  } = useModelAndProvider();

  const loadModelData = useCallback(async () => {
    try {
      setIsLoading(true);

      // Get display name (alias if available, otherwise model name)
      const modelDisplayName = await getCurrentModelDisplayName();
      setDisplayModelName(modelDisplayName);

      try {
        const multimodalValue = await read('AGIME_MULTIMODAL', false);
        setMultimodalEnabled(interpretMultimodalValue(multimodalValue));
      } catch (error) {
        console.error('Error reading AGIME_MULTIMODAL:', error);
        setMultimodalEnabled(true);
      }

      // Get provider display name (subtext if available from predefined models, otherwise provider metadata)
      const providerDisplayName = await getCurrentProviderDisplayName();
      if (providerDisplayName) {
        setProvider(providerDisplayName);
      } else {
        // Fallback to original provider lookup
        const agimeProvider = (await read('AGIME_PROVIDER', false)) as string;
        const providers = await getProviders(true);
        const providerDetailsList = providers.filter((provider) => provider.name === agimeProvider);

        if (providerDetailsList.length != 1) {
          toastError({
            title: UNKNOWN_PROVIDER_TITLE,
            msg: UNKNOWN_PROVIDER_MSG,
          });
          setProvider(agimeProvider);
        } else {
          const fallbackProviderDisplayName = providerDetailsList[0].metadata.display_name;
          setProvider(fallbackProviderDisplayName);
        }
      }
    } catch (error) {
      console.error('Error loading model data:', error);
    } finally {
      setIsLoading(false);
    }
  }, [read, getProviders, getCurrentModelDisplayName, getCurrentProviderDisplayName]);

  useEffect(() => {
    loadModelData();
  }, [loadModelData]);

  // Update display when model or provider changes - but only if they actually changed
  const prevModelRef = useRef<string | null>(null);
  const prevProviderRef = useRef<string | null>(null);

  useEffect(() => {
    if (
      currentModel &&
      currentProvider &&
      (currentModel !== prevModelRef.current || currentProvider !== prevProviderRef.current)
    ) {
      prevModelRef.current = currentModel;
      prevProviderRef.current = currentProvider;
      loadModelData();
    }
  }, [currentModel, currentProvider, loadModelData]);

  const handleQuickSetupComplete = useCallback(async (config: QuickSetupConfig) => {
    // Refresh the global model/provider context to sync across the app (including chat window)
    await refreshCurrentModelAndProvider();
    // Reload local model data to reflect the new configuration
    loadModelData();
    toastSuccess({
      title: t('quickSetup.toast.configSuccess'),
      msg: t('quickSetup.toast.switchedTo', { provider: config.provider.displayName, model: config.modelName }),
    });
  }, [loadModelData, refreshCurrentModelAndProvider, t]);

  const handleMultimodalToggle = useCallback(
    async (checked: boolean) => {
      const previous = multimodalEnabled;
      setMultimodalEnabled(checked);
      setMultimodalSaving(true);
      try {
        await upsert('AGIME_MULTIMODAL', checked, false);

        // Rebuild the live session's provider so the change takes effect on the
        // current chat without restarting. Falls back to next-chat if there is
        // no active session yet (e.g. opened from a fresh Hub).
        const sessionId = chatContext?.activeSessionId;
        if (sessionId && currentProvider && currentModel) {
          await updateAgentProvider({
            body: {
              session_id: sessionId,
              provider: currentProvider,
              model: currentModel,
            },
            throwOnError: true,
          });
          toastSuccess({
            title: t('models.multimodal.title'),
            msg: t('models.multimodal.appliedToSession'),
          });
        } else {
          toastSuccess({
            title: t('models.multimodal.title'),
            msg: t('models.multimodal.savedForNextChat'),
          });
        }
      } catch (error) {
        setMultimodalEnabled(previous);
        toastError({
          title: t('models.multimodal.title'),
          msg: t('models.multimodal.updateFailed'),
          traceback: error instanceof Error ? error.message : String(error),
        });
      } finally {
        setMultimodalSaving(false);
      }
    },
    [multimodalEnabled, upsert, chatContext, currentProvider, currentModel, t]
  );

  return (
    <div className="space-y-6">
      {/* Quick Setup Card */}
      <SettingsCard
        icon={<Zap className="h-5 w-5" />}
        title={t('quickSetup.card.title')}
        description={t('quickSetup.card.description')}
      >
        <button
          onClick={() => setShowQuickSetup(true)}
          className="px-4 py-2.5 bg-gradient-to-r from-block-teal to-block-teal/80 hover:shadow-lg hover:shadow-block-teal/25 text-white rounded-xl text-sm font-semibold transition-all duration-200 flex items-center gap-2"
        >
          <Zap className="w-4 h-4" />
          {t('quickSetup.card.startButton')}
        </button>
      </SettingsCard>

      {/* Current Model Card */}
      <SettingsCard
        icon={<Cpu className="h-5 w-5" />}
        title={isLoading ? '' : displayModelName}
        description={isLoading ? '' : (provider ?? undefined)}
      >
        <ModelSettingsButtons setView={setView} />
      </SettingsCard>

      {/* Multimodal (image) input toggle */}
      <SettingsCard
        icon={<Image className="h-5 w-5" />}
        title={t('models.multimodal.title')}
        description={t('models.multimodal.description')}
        headerClassName="pb-4"
        contentClassName="hidden"
        headerAction={
          <Switch
            checked={multimodalEnabled}
            disabled={multimodalSaving}
            onCheckedChange={handleMultimodalToggle}
            aria-label={t('models.multimodal.title')}
          />
        }
      >
        {null}
      </SettingsCard>

      {/* Reset Provider Card */}
      <SettingsCard
        icon={<RefreshCw className="h-5 w-5" />}
        title={t('models.resetProvider.title')}
        description={t('models.resetProvider.description')}
      >
        <ResetProviderSection setView={setView} />
      </SettingsCard>

      {/* Quick Setup Modal */}
      <QuickSetupModal
        open={showQuickSetup}
        onOpenChange={setShowQuickSetup}
        onComplete={handleQuickSetupComplete}
      />
    </div>
  );
}
