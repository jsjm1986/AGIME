import React, { useState, useEffect, useCallback, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { AlertTriangle, RotateCcw, Loader2, Info } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../ui/dialog';
import { Button } from '../../ui/button';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../../ui/tabs';
import { Alert, AlertDescription } from '../../ui/alert';
import { PromptEditor } from './PromptEditor';
import { PromptTemplateSelector } from './PromptTemplateSelector';
import {
  getSystemPrompt,
  setSystemPrompt,
  resetSystemPrompt,
  getDefaultPrompts,
  PromptTemplate,
} from '../../../lib/api/prompts';

interface SystemPromptModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export const SystemPromptModal: React.FC<SystemPromptModalProps> = ({
  isOpen,
  onClose,
}) => {
  const { t } = useTranslation('settings');
  const { t: tCommon } = useTranslation('common');
  const [content, setContent] = useState('');
  const [originalContent, setOriginalContent] = useState('');
  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [isCustom, setIsCustom] = useState(false);
  const [templates, setTemplates] = useState<PromptTemplate[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const autoCloseTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Cleanup timeout on unmount or when modal closes
  useEffect(() => {
    return () => {
      if (autoCloseTimeoutRef.current) {
        clearTimeout(autoCloseTimeoutRef.current);
      }
    };
  }, []);

  const fetchCurrentPrompt = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      // Use Promise.allSettled to handle partial failures gracefully
      // Templates are optional - don't fail the whole operation if templates fail to load
      const [promptResult, templatesResult] = await Promise.allSettled([
        getSystemPrompt(),
        getDefaultPrompts(),
      ]);

      // Handle prompt response (required)
      if (promptResult.status === 'fulfilled' && promptResult.value.data) {
        setContent(promptResult.value.data.content);
        setOriginalContent(promptResult.value.data.content);
        setIsCustom(promptResult.value.data.is_custom);
      } else if (promptResult.status === 'rejected') {
        console.error('Failed to fetch prompt:', promptResult.reason);
        setError(t('prompts.fetchError'));
        return;
      }

      // Handle templates response (optional - don't fail if templates unavailable)
      if (templatesResult.status === 'fulfilled' && templatesResult.value.data) {
        setTemplates(templatesResult.value.data.prompts);
      } else if (templatesResult.status === 'rejected') {
        console.warn('Failed to fetch templates:', templatesResult.reason);
        // Don't set error - templates are optional, user can still edit prompts
        setTemplates([]);
      }
    } catch (err) {
      console.error('Failed to fetch prompt:', err);
      setError(t('prompts.fetchError'));
    } finally {
      setIsLoading(false);
    }
  }, [t]);

  useEffect(() => {
    if (isOpen) {
      fetchCurrentPrompt();
    }
  }, [isOpen, fetchCurrentPrompt]);

  const handleSave = async () => {
    setIsSaving(true);
    setError(null);
    setSuccessMessage(null);

    // Clear any existing auto-close timeout
    if (autoCloseTimeoutRef.current) {
      clearTimeout(autoCloseTimeoutRef.current);
    }

    try {
      await setSystemPrompt({
        body: {
          content,
          enabled: true,
        },
      });
      setSuccessMessage(t('prompts.savedSuccess'));
      setOriginalContent(content);
      setIsCustom(true);

      // Auto-close after success with cancellable timeout
      autoCloseTimeoutRef.current = setTimeout(() => {
        onClose();
      }, 1500);
    } catch (err) {
      console.error('Failed to save prompt:', err);
      setError(t('prompts.saveError'));
    } finally {
      setIsSaving(false);
    }
  };

  const handleReset = async () => {
    setIsSaving(true);
    setError(null);
    setSuccessMessage(null);

    try {
      await resetSystemPrompt();
      setSuccessMessage(t('prompts.resetSuccess'));

      // Fetch the default prompt to update the editor
      const response = await getSystemPrompt();
      if (response.data) {
        setContent(response.data.content);
        setOriginalContent(response.data.content);
        setIsCustom(false);
      }
    } catch (err) {
      console.error('Failed to reset prompt:', err);
      setError(t('prompts.resetError'));
    } finally {
      setIsSaving(false);
    }
  };

  const handleTemplateSelect = (template: PromptTemplate) => {
    setContent(template.content);
  };

  const hasChanges = content !== originalContent;
  const trimmedContent = content.trim();
  const isContentValid = trimmedContent.length > 0 && trimmedContent.length <= 50000;

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="w-[85vw] max-w-4xl max-h-[90vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>{t('prompts.editTitle')}</DialogTitle>
          <DialogDescription>
            {t('prompts.editDescription')}
          </DialogDescription>
        </DialogHeader>

        <Alert variant="default" className="bg-yellow-500/10 border-yellow-500/30">
          <AlertTriangle className="h-4 w-4 text-yellow-500" />
          <AlertDescription className="text-yellow-200">
            {t('prompts.warning')}
          </AlertDescription>
        </Alert>

        {error && (
          <Alert variant="destructive">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        {successMessage && (
          <Alert className="bg-green-500/10 border-green-500/30">
            <AlertDescription className="text-green-400">{successMessage}</AlertDescription>
          </Alert>
        )}

        {successMessage && (
          <Alert className="bg-blue-500/10 border-blue-500/30">
            <Info className="h-4 w-4 text-blue-400" />
            <AlertDescription className="text-blue-300">
              {t('prompts.newSessionHint')}
            </AlertDescription>
          </Alert>
        )}

        {isLoading ? (
          <div className="flex-1 flex items-center justify-center py-12">
            <Loader2 className="h-6 w-6 animate-spin text-text-muted" />
          </div>
        ) : (
          <Tabs defaultValue="edit" className="flex-1 flex flex-col min-h-0">
            <TabsList className="grid w-full grid-cols-2">
              <TabsTrigger value="edit">{t('prompts.tabEdit')}</TabsTrigger>
              <TabsTrigger value="templates">{t('prompts.tabTemplates')}</TabsTrigger>
            </TabsList>

            <TabsContent value="edit" className="flex-1 min-h-0 mt-4">
              <PromptEditor
                value={content}
                onChange={setContent}
                placeholder={t('prompts.placeholder')}
              />
              <div className="flex justify-between items-center mt-2 text-xs text-text-muted">
                <span>
                  {isCustom ? t('prompts.usingCustom') : t('prompts.usingDefault')}
                </span>
                <span className={!isContentValid && hasChanges ? 'text-red-400' : ''}>
                  {trimmedContent.length} / 50000 {t('prompts.characters')}
                </span>
              </div>
            </TabsContent>

            <TabsContent value="templates" className="flex-1 min-h-0 mt-4 overflow-auto">
              <PromptTemplateSelector
                templates={templates}
                onSelect={handleTemplateSelect}
              />
            </TabsContent>
          </Tabs>
        )}

        <DialogFooter className="gap-2 sm:gap-0">
          <Button
            variant="outline"
            onClick={handleReset}
            disabled={isSaving || !isCustom}
            className="gap-2"
          >
            <RotateCcw size={16} />
            {t('prompts.resetToDefault')}
          </Button>
          <div className="flex-1" />
          <Button variant="outline" onClick={onClose} disabled={isSaving}>
            {tCommon('cancel')}
          </Button>
          <Button
            onClick={handleSave}
            disabled={isSaving || !hasChanges || !isContentValid}
          >
            {isSaving ? tCommon('saving') : tCommon('save')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
