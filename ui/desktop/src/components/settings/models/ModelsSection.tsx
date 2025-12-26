import { useEffect, useState, useCallback, useRef } from 'react';
import { View } from '../../../utils/navigationUtils';
import ModelSettingsButtons from './subcomponents/ModelSettingsButtons';
import { useConfig } from '../../ConfigContext';
import {
  UNKNOWN_PROVIDER_MSG,
  UNKNOWN_PROVIDER_TITLE,
  useModelAndProvider,
} from '../../ModelAndProviderContext';
import { toastError } from '../../../toasts';

import { SettingsCard } from '../common';
import ResetProviderSection from '../reset_provider/ResetProviderSection';
import { Cpu, RefreshCw } from 'lucide-react';

interface ModelsSectionProps {
  setView: (view: View) => void;
}

export default function ModelsSection({ setView }: ModelsSectionProps) {
  const [provider, setProvider] = useState<string | null>(null);
  const [displayModelName, setDisplayModelName] = useState<string>('');
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const { read, getProviders } = useConfig();
  const {
    getCurrentModelDisplayName,
    getCurrentProviderDisplayName,
    currentModel,
    currentProvider,
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

  return (
    <div className="space-y-6">
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
    </div>
  );
}
