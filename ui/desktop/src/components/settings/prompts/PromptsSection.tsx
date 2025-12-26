import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Settings2, Loader2 } from 'lucide-react';
import { Button } from '../../ui/button';
import { SystemPromptModal } from './SystemPromptModal';
import { getSystemPrompt } from '../../../lib/api/prompts';

export const PromptsSection: React.FC = () => {
  const { t } = useTranslation('settings');
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [isCustomPrompt, setIsCustomPrompt] = useState(false);
  const [isLoading, setIsLoading] = useState(true);

  const fetchPromptStatus = useCallback(async () => {
    try {
      const response = await getSystemPrompt();
      if (response.data) {
        setIsCustomPrompt(response.data.is_custom);
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

  const handleModalClose = () => {
    setIsModalOpen(false);
    // Refresh status after modal closes
    fetchPromptStatus();
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-between px-2 py-2 rounded-lg">
        <div className="flex-1">
          <h3 className="text-sm font-medium text-text-default leading-5">{t('prompts.sectionTitle')}</h3>
          <p className="text-xs text-text-muted mt-0.5 leading-4">
            {t('prompts.sectionDescription')}
          </p>
        </div>
        <Loader2 className="h-5 w-5 animate-spin text-text-muted" />
      </div>
    );
  }

  return (
    <>
      <div className="flex items-center justify-between px-2 py-2 hover:bg-background-muted rounded-lg transition-colors">
        <div className="flex-1">
          <h3 className="text-sm font-medium text-text-default leading-5">{t('prompts.sectionTitle')}</h3>
          <p className="text-xs text-text-muted mt-0.5 leading-4">
            {t('prompts.sectionDescription')}
          </p>
        </div>
        <div className="flex items-center gap-3">
          <span className="text-xs text-text-muted">
            {isCustomPrompt ? t('prompts.usingCustom') : t('prompts.usingDefault')}
          </span>
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
