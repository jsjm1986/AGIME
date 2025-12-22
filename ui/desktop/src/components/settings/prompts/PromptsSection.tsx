import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Settings2, Loader2 } from 'lucide-react';
import { Button } from '../../ui/button';
import { Switch } from '../../ui/switch';
import { SystemPromptModal } from './SystemPromptModal';
import { getSystemPrompt, setSystemPrompt as apiSetSystemPrompt } from '../../../lib/api/prompts';

export const PromptsSection: React.FC = () => {
  const { t } = useTranslation('settings');
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [customPromptEnabled, setCustomPromptEnabled] = useState(false);
  const [isLoading, setIsLoading] = useState(true);

  const fetchPromptStatus = useCallback(async () => {
    try {
      const response = await getSystemPrompt();
      if (response.data) {
        setCustomPromptEnabled(response.data.is_enabled);
      }
    } catch (error) {
      console.error('Failed to fetch prompt status:', error);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchPromptStatus();
  }, [fetchPromptStatus]);

  const handleToggle = async (enabled: boolean) => {
    try {
      // Get current prompt content first
      const response = await getSystemPrompt();

      // Safety check: only proceed if we successfully got the current content
      // to prevent accidentally overwriting with empty content
      if (!response.data) {
        console.error('Failed to get current prompt content - aborting toggle to prevent data loss');
        return;
      }

      const currentContent = response.data.content;

      // Update with new enabled state
      await apiSetSystemPrompt({
        body: {
          content: currentContent,
          enabled: enabled,
        },
      });
      setCustomPromptEnabled(enabled);
    } catch (error) {
      console.error('Failed to toggle custom prompt:', error);
      // Revert UI state on error by re-fetching
      fetchPromptStatus();
    }
  };

  const handleModalClose = () => {
    setIsModalOpen(false);
    // Refresh status after modal closes
    fetchPromptStatus();
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-between px-2 py-2">
        <div className="flex-1">
          <h3 className="text-sm text-text-default">{t('prompts.sectionTitle')}</h3>
          <p className="text-xs text-text-muted mt-[2px]">
            {t('prompts.sectionDescription')}
          </p>
        </div>
        <Loader2 className="h-5 w-5 animate-spin text-text-muted" />
      </div>
    );
  }

  return (
    <>
      <div className="flex items-center justify-between px-2 py-2">
        <div className="flex-1">
          <h3 className="text-sm text-text-default">{t('prompts.sectionTitle')}</h3>
          <p className="text-xs text-text-muted mt-[2px]">
            {t('prompts.sectionDescription')}
          </p>
        </div>
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2">
            <span className="text-xs text-text-muted">
              {customPromptEnabled ? t('prompts.enableCustom') : t('prompts.disableCustom')}
            </span>
            <Switch
              checked={customPromptEnabled}
              onCheckedChange={handleToggle}
            />
          </div>
          <Button
            onClick={() => setIsModalOpen(true)}
            variant="outline"
            size="sm"
            className="flex items-center gap-2"
          >
            <Settings2 size={16} />
            {t('prompts.configure')}
          </Button>
        </div>
      </div>
      {isModalOpen && (
        <SystemPromptModal
          isOpen={isModalOpen}
          onClose={handleModalClose}
        />
      )}
    </>
  );
};
