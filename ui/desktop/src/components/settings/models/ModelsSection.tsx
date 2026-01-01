import { useEffect, useState, useCallback, useRef } from 'react';
import { View } from '../../../utils/navigationUtils';
import ModelSettingsButtons from './subcomponents/ModelSettingsButtons';
import { useConfig } from '../../ConfigContext';
import {
  UNKNOWN_PROVIDER_MSG,
  UNKNOWN_PROVIDER_TITLE,
  useModelAndProvider,
} from '../../ModelAndProviderContext';
import { toastError, toastSuccess } from '../../../toasts';

import { SettingsCard } from '../common';
import ResetProviderSection from '../reset_provider/ResetProviderSection';
import { Cpu, RefreshCw, Zap } from 'lucide-react';
import { QuickSetupModal, type QuickSetupConfig } from '../quick-setup';

interface ModelsSectionProps {
  setView: (view: View) => void;
}

export default function ModelsSection({ setView }: ModelsSectionProps) {
  const [provider, setProvider] = useState<string | null>(null);
  const [displayModelName, setDisplayModelName] = useState<string>('');
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [showQuickSetup, setShowQuickSetup] = useState<boolean>(false);
  const { read, getProviders } = useConfig();
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

      // Get provider display name (subtext if available from predefined models, otherwise provider metadata)
      const providerDisplayName = await getCurrentProviderDisplayName();
      if (providerDisplayName) {
        setProvider(providerDisplayName);
      } else {
        // Fallback to original provider lookup
        const gooseProvider = (await read('GOOSE_PROVIDER', false)) as string;
        const providers = await getProviders(true);
        const providerDetailsList = providers.filter((provider) => provider.name === gooseProvider);

        if (providerDetailsList.length != 1) {
          toastError({
            title: UNKNOWN_PROVIDER_TITLE,
            msg: UNKNOWN_PROVIDER_MSG,
          });
          setProvider(gooseProvider);
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
      title: '配置成功',
      msg: `已切换到 ${config.provider.displayName} - ${config.modelName}`,
    });
  }, [loadModelData, refreshCurrentModelAndProvider]);

  return (
    <div className="space-y-6">
      {/* Quick Setup Card */}
      <SettingsCard
        icon={<Zap className="h-5 w-5" />}
        title="快速配置"
        description="新手向导 - 快速配置模型提供商和凭证"
      >
        <button
          onClick={() => setShowQuickSetup(true)}
          className="px-4 py-2.5 bg-gradient-to-r from-block-teal to-block-teal/80 hover:shadow-lg hover:shadow-block-teal/25 text-white rounded-xl text-sm font-semibold transition-all duration-200 flex items-center gap-2"
        >
          <Zap className="w-4 h-4" />
          开始配置
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

      {/* Reset Provider Card */}
      <SettingsCard
        icon={<RefreshCw className="h-5 w-5" />}
        title="Reset Provider and Model"
        description="Clear your selected model and provider settings to start fresh"
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
