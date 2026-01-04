import { View, ViewOptions } from '../../utils/navigationUtils';
import { useTranslation } from 'react-i18next';
import { useChatContext } from '../../contexts/ChatContext';
import ExtensionsSection from '../settings/extensions/ExtensionsSection';
import { ExtensionConfig } from '../../api';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { Button } from '../ui/button';
import { Plus } from 'lucide-react';
import { GPSIcon } from '../ui/icons';
import { useState, useEffect, useRef, useCallback } from 'react';
import kebabCase from 'lodash/kebabCase';
import ExtensionModal from '../settings/extensions/modal/ExtensionModal';
import {
  getDefaultFormData,
  ExtensionFormData,
  createExtensionConfig,
} from '../settings/extensions/utils';
import { activateExtension } from '../settings/extensions';
import { useConfig } from '../ConfigContext';
import { SearchView } from '../conversation/SearchView';
import { createSession } from '../../sessions';
import { toastError } from '../../toasts';
import { chatStreamManager } from '../../services/ChatStreamManager';

export type ExtensionsViewOptions = {
  deepLinkConfig?: ExtensionConfig;
  showEnvVars?: boolean;
};

export default function ExtensionsView({
  viewOptions,
}: {
  onClose: () => void;
  setView: (view: View, viewOptions?: ViewOptions) => void;
  viewOptions: ExtensionsViewOptions;
}) {
  const { t } = useTranslation('extensions');
  const [isAddModalOpen, setIsAddModalOpen] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);
  const [searchTerm, setSearchTerm] = useState('');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const { addExtension } = useConfig();
  const chatContext = useChatContext();
  const initialSessionId = chatContext?.chat.sessionId || '';

  // Track active sessionId (may be updated by lazy creation)
  const [activeSessionId, setActiveSessionId] = useState(initialSessionId);

  // Refs for cleanup and preventing stale closures
  const isMountedRef = useRef(true);

  // Sync with context sessionId when it changes
  useEffect(() => {
    if (initialSessionId) {
      setActiveSessionId(initialSessionId);
    }
  }, [initialSessionId]);

  // Cleanup on unmount
  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  // Only trigger refresh when deep link config changes AND we don't need to show env vars
  useEffect(() => {
    if (viewOptions.deepLinkConfig && !viewOptions.showEnvVars) {
      setRefreshKey((prevKey) => prevKey + 1);
    }
  }, [viewOptions.deepLinkConfig, viewOptions.showEnvVars]);

  const scrollToExtension = (extensionName: string) => {
    setTimeout(() => {
      const element = document.getElementById(`extension-${kebabCase(extensionName)}`);
      if (element) {
        element.scrollIntoView({
          behavior: 'smooth',
          block: 'center',
        });
        // Add a subtle highlight effect
        element.style.boxShadow = '0 0 0 2px rgba(59, 130, 246, 0.5)';
        setTimeout(() => {
          element.style.boxShadow = '';
        }, 2000);
      }
    }, 200);
  };

  // Scroll to extension whenever extensionId is provided (after refresh)
  useEffect(() => {
    if (viewOptions.deepLinkConfig?.name && refreshKey > 0) {
      scrollToExtension(viewOptions.deepLinkConfig?.name);
    }
  }, [viewOptions.deepLinkConfig?.name, refreshKey]);

  const handleModalClose = () => {
    setIsAddModalOpen(false);
  };

  // Shared function to ensure session exists (lazy creation if needed)
  const ensureSession = useCallback(async (): Promise<string> => {
    if (activeSessionId) {
      return activeSessionId;
    }

    // Lazy session creation
    const newSession = await createSession();
    const newSessionId = newSession.id;

    // Initialize session for extensions
    await chatStreamManager.initializeSession(newSessionId);

    // Update local state for ExtensionsSection
    if (isMountedRef.current) {
      setActiveSessionId(newSessionId);
    }

    // Notify parent components to update URL/state
    window.dispatchEvent(new CustomEvent('lazy-session-created', {
      detail: { sessionId: newSessionId }
    }));

    return newSessionId;
  }, [activeSessionId]);

  const handleAddExtension = useCallback(async (formData: ExtensionFormData) => {
    // Prevent duplicate submissions
    if (isSubmitting) {
      return;
    }

    // Close the modal immediately
    handleModalClose();
    setIsSubmitting(true);

    let currentSessionId: string;

    try {
      currentSessionId = await ensureSession();
    } catch (error) {
      console.error('Failed to create session for extension:', error);
      if (isMountedRef.current) {
        setIsSubmitting(false);
        toastError({
          title: t('errors.sessionCreationFailed'),
          msg: String(error)
        });
      }
      return;
    }

    const extensionConfig = createExtensionConfig(formData);

    try {
      await activateExtension({
        addToConfig: addExtension,
        extensionConfig: extensionConfig,
        sessionId: currentSessionId,
      });
      // Trigger a refresh of the extensions list
      if (isMountedRef.current) {
        setRefreshKey((prevKey) => prevKey + 1);
      }
    } catch (error) {
      console.error('Failed to activate extension:', error);
      if (isMountedRef.current) {
        toastError({
          title: t('errors.activationFailed'),
          msg: String(error)
        });
        setRefreshKey((prevKey) => prevKey + 1);
      }
    } finally {
      if (isMountedRef.current) {
        setIsSubmitting(false);
      }
    }
  }, [ensureSession, isSubmitting, addExtension, t]);

  return (
    <MainPanelLayout>
      <div
        className="flex flex-col min-w-0 flex-1 overflow-y-auto relative"
        data-search-scroll-area
      >
        <div className="bg-background-default px-8 pb-4 pt-16">
          <div className="flex flex-col page-transition">
            <div className="flex justify-between items-center mb-1">
              <h1 className="text-4xl font-light">{t('title')}</h1>
            </div>
            <p className="text-sm text-text-muted mb-6">
              {t('description')}
            </p>

            {/* Action Buttons */}
            <div className="flex gap-4 mb-8">
              <Button
                className="flex items-center gap-2 justify-center"
                variant="default"
                onClick={() => setIsAddModalOpen(true)}
                disabled={isSubmitting}
              >
                <Plus className="h-4 w-4" />
                {t('addCustom')}
              </Button>
              <Button
                className="flex items-center gap-2 justify-center"
                variant="secondary"
                onClick={() =>
                  window.open('https://github.com/jsjm1986/AGIME', '_blank')
                }
              >
                <GPSIcon size={12} />
                {t('browseExtensions')}
              </Button>
            </div>
          </div>
        </div>

        <div className="px-8 pb-16">
          <SearchView onSearch={(term) => setSearchTerm(term)} placeholder={t('searchPlaceholder')}>
            <ExtensionsSection
              key={refreshKey}
              sessionId={activeSessionId}
              deepLinkConfig={viewOptions.deepLinkConfig}
              showEnvVars={viewOptions.showEnvVars}
              hideButtons={true}
              searchTerm={searchTerm}
              ensureSession={ensureSession}
              onModalClose={(extensionName: string) => {
                scrollToExtension(extensionName);
              }}
            />
          </SearchView>
        </div>

        {/* Bottom padding space - same as in hub.tsx */}
        <div className="block h-8" />
      </div>

      {/* Modal for adding a new extension */}
      {isAddModalOpen && (
        <ExtensionModal
          title={t('addCustomTitle')}
          initialData={getDefaultFormData()}
          onClose={handleModalClose}
          onSubmit={handleAddExtension}
          submitLabel={t('addCustom')}
          modalType={'add'}
        />
      )}
    </MainPanelLayout>
  );
}
