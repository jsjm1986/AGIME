import React, { memo, useMemo, useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ProviderCard } from './subcomponents/ProviderCard';
import CardContainer from './subcomponents/CardContainer';
import ProviderConfigurationModal from './modal/ProviderConfiguationModal';
import {
  DeclarativeProviderConfig,
  ProviderDetails,
  UpdateCustomProviderRequest,
  removeCustomProvider,
  readConfig,
} from '../../../api';
import { Plus } from 'lucide-react';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '../../ui/dialog';
import CustomProviderForm from './modal/subcomponents/forms/CustomProviderForm';
import { SwitchModelModal } from '../models/subcomponents/SwitchModelModal';
import type { View } from '../../../utils/navigationUtils';
import { toastSuccess, toastError } from '../../../toasts';

const GridLayout = memo(function GridLayout({ children }: { children: React.ReactNode }) {
  return (
    <div
      className="grid gap-4 [&_*]:z-20 p-1"
      style={{
        gridTemplateColumns: 'repeat(auto-fill, minmax(200px, 200px))',
        justifyContent: 'center',
      }}
    >
      {children}
    </div>
  );
});

const CustomProviderCard = memo(function CustomProviderCard({
  onClick,
  addLabel,
  customProviderLabel,
}: {
  onClick: () => void;
  addLabel: string;
  customProviderLabel: string;
}) {
  return (
    <CardContainer
      testId="add-custom-provider-card"
      onClick={onClick}
      header={null}
      body={
        <div className="flex flex-col items-center justify-center min-h-[200px]">
          <Plus className="w-8 h-8 text-gray-400 mb-2" />
          <div className="text-sm text-gray-600 dark:text-gray-400 text-center">
            <div>{addLabel}</div>
            <div>{customProviderLabel}</div>
          </div>
        </div>
      }
      grayedOut={false}
      borderStyle="dashed"
    />
  );
});

function ProviderCards({
  providers,
  isOnboarding,
  refreshProviders,
  setView,
  onModelSelected,
}: {
  providers: ProviderDetails[];
  isOnboarding: boolean;
  refreshProviders?: () => void;
  setView?: (view: View) => void;
  onModelSelected?: () => void;
}) {
  const { t } = useTranslation('settings');
  const [configuringProvider, setConfiguringProvider] = useState<ProviderDetails | null>(null);
  const [showCustomProviderModal, setShowCustomProviderModal] = useState(false);
  const [showSwitchModelModal, setShowSwitchModelModal] = useState(false);
  const [switchModelProvider, setSwitchModelProvider] = useState<string | null>(null);
  const [editingProvider, setEditingProvider] = useState<{
    id: string;
    config: DeclarativeProviderConfig;
    isEditable: boolean;
  } | null>(null);
  const [isActiveProvider, setIsActiveProvider] = useState(false);

  const handleProviderLaunchWithModelSelection = useCallback((provider: ProviderDetails) => {
    setSwitchModelProvider(provider.name);
    setShowSwitchModelModal(true);
  }, []);

  const openModal = useCallback(
    (provider: ProviderDetails) => setConfiguringProvider(provider),
    []
  );

  const configureProviderViaModal = useCallback(
    async (provider: ProviderDetails) => {
      if (provider.provider_type === 'Custom' || provider.provider_type === 'Declarative') {
        const { getCustomProvider } = await import('../../../api');

        // Parallel API calls for better performance
        const [result, currentProviderResult] = await Promise.all([
          getCustomProvider({ path: { id: provider.name }, throwOnError: true }),
          readConfig({ body: { key: 'AGIME_PROVIDER', is_secret: false } }).catch(() => ({ data: null })),
        ]);

        // Check if this is the active provider
        setIsActiveProvider(currentProviderResult.data === provider.name);

        if (result.data) {
          setEditingProvider({
            id: provider.name,
            config: result.data.config,
            isEditable: result.data.is_editable,
          });
          setShowCustomProviderModal(true);
        }
      } else {
        openModal(provider);
      }
    },
    [openModal]
  );

  const handleUpdateCustomProvider = useCallback(
    async (data: UpdateCustomProviderRequest) => {
      if (!editingProvider) return;

      const { updateCustomProvider } = await import('../../../api');
      await updateCustomProvider({
        path: { id: editingProvider.id },
        body: data,
        throwOnError: true,
      });
      const providerId = editingProvider.id;
      setShowCustomProviderModal(false);
      setEditingProvider(null);
      if (refreshProviders) {
        refreshProviders();
      }
      setSwitchModelProvider(providerId);
      setShowSwitchModelModal(true);
    },
    [editingProvider, refreshProviders]
  );

  const handleCloseModal = useCallback(() => {
    setShowCustomProviderModal(false);
    setEditingProvider(null);
    setIsActiveProvider(false);
  }, []);

  const handleDeleteCustomProvider = useCallback(async () => {
    if (!editingProvider) return;

    try {
      // Re-verify active provider status before deletion to prevent race condition
      const currentProviderResult = await readConfig({ body: { key: 'AGIME_PROVIDER', is_secret: false } });
      if (currentProviderResult.data === editingProvider.id) {
        setIsActiveProvider(true);
        toastError({
          title: t('provider.error'),
          msg: t('provider.cannotDeleteActive', { provider: editingProvider.config.display_name }),
        });
        return;
      }

      await removeCustomProvider({
        path: { id: editingProvider.id },
      });

      toastSuccess({
        title: t('provider.deleteProvider'),
        msg: `${editingProvider.config.display_name}`,
      });

      setShowCustomProviderModal(false);
      setEditingProvider(null);
      setIsActiveProvider(false);
      if (refreshProviders) {
        refreshProviders();
      }
    } catch (error) {
      console.error('Failed to delete custom provider:', error);
      toastError({
        title: t('provider.error'),
        msg: error instanceof Error ? error.message : String(error),
      });
    }
  }, [editingProvider, refreshProviders, t]);

  const onCloseProviderConfig = useCallback(() => {
    setConfiguringProvider(null);
    if (refreshProviders) {
      refreshProviders();
    }
  }, [refreshProviders]);

  const onProviderConfigured = useCallback(
    (provider: ProviderDetails) => {
      setConfiguringProvider(null);
      if (refreshProviders) {
        refreshProviders();
      }
      setSwitchModelProvider(provider.name);
      setShowSwitchModelModal(true);
    },
    [refreshProviders]
  );

  const onCloseSwitchModelModal = useCallback(() => {
    setShowSwitchModelModal(false);
  }, []);

  const handleSetView = useCallback(
    (view: View) => {
      setShowSwitchModelModal(false);
      if (setView) {
        setView(view);
      }
    },
    [setView]
  );

  const handleCreateCustomProvider = useCallback(
    async (data: UpdateCustomProviderRequest) => {
      const { createCustomProvider } = await import('../../../api');
      await createCustomProvider({ body: data, throwOnError: true });
      setShowCustomProviderModal(false);
      if (refreshProviders) {
        refreshProviders();
      }
      setShowSwitchModelModal(true);
    },
    [refreshProviders]
  );

  const providerCards = useMemo(() => {
    // providers needs to be an array
    const providersArray = Array.isArray(providers) ? providers : [];
    // Sort providers alphabetically by name
    const sortedProviders = [...providersArray].sort((a, b) => a.name.localeCompare(b.name));
    const cards = sortedProviders.map((provider) => (
      <ProviderCard
        key={provider.name}
        provider={provider}
        onConfigure={() => configureProviderViaModal(provider)}
        onLaunch={() => handleProviderLaunchWithModelSelection(provider)}
        isOnboarding={isOnboarding}
      />
    ));

    cards.push(
      <CustomProviderCard
        key="add-custom"
        onClick={() => setShowCustomProviderModal(true)}
        addLabel={t('customProvider.add')}
        customProviderLabel={t('customProvider.customProvider')}
      />
    );

    return cards;
  }, [providers, isOnboarding, configureProviderViaModal, handleProviderLaunchWithModelSelection, t]);

  const initialData = editingProvider && {
    engine: editingProvider.config.engine.toLowerCase(),
    display_name: editingProvider.config.display_name,
    api_url: editingProvider.config.base_url,
    api_key: '',
    models: editingProvider.config.models.map((m) => m.name),
    supports_streaming: editingProvider.config.supports_streaming ?? true,
  };

  const editable = editingProvider ? editingProvider.isEditable : true;
  const title = editingProvider
    ? editable
      ? t('customProvider.editProvider')
      : t('customProvider.configureProvider')
    : t('customProvider.addProvider');
  return (
    <>
      {providerCards}
      <Dialog open={showCustomProviderModal} onOpenChange={handleCloseModal}>
        <DialogContent className="sm:max-w-[600px]">
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
          </DialogHeader>
          <CustomProviderForm
            initialData={initialData}
            isEditable={editable}
            onSubmit={editingProvider ? handleUpdateCustomProvider : handleCreateCustomProvider}
            onCancel={handleCloseModal}
            onDelete={editingProvider ? handleDeleteCustomProvider : undefined}
            isActiveProvider={isActiveProvider}
          />
        </DialogContent>
      </Dialog>{' '}
      {configuringProvider && (
        <ProviderConfigurationModal
          provider={configuringProvider}
          onClose={onCloseProviderConfig}
          onConfigured={onProviderConfigured}
        />
      )}
      {showSwitchModelModal && (
        <SwitchModelModal
          sessionId={null}
          onClose={onCloseSwitchModelModal}
          setView={handleSetView}
          onModelSelected={onModelSelected}
          initialProvider={switchModelProvider}
          titleOverride={t('models.chooseModel')}
        />
      )}
    </>
  );
}

export default function ProviderGrid({
  providers,
  isOnboarding,
  refreshProviders,
  setView,
  onModelSelected,
}: {
  providers: ProviderDetails[];
  isOnboarding: boolean;
  refreshProviders?: () => void;
  setView?: (view: View) => void;
  onModelSelected?: () => void;
}) {
  return (
    <GridLayout>
      <ProviderCards
        providers={providers}
        isOnboarding={isOnboarding}
        refreshProviders={refreshProviders}
        setView={setView}
        onModelSelected={onModelSelected}
      />
    </GridLayout>
  );
}
