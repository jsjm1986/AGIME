import {
  Dialog,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogPortal,
  DialogOverlay,
} from './dialog';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import { useTranslation } from 'react-i18next';
import { Button } from './button';
import MarkdownContent from '../MarkdownContent';
import { cn } from '../../utils';

interface RecipeWarningModalProps {
  isOpen: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  recipeDetails: {
    title?: string;
    description?: string;
    instructions?: string;
  };
  hasSecurityWarnings?: boolean;
}

export function RecipeWarningModal({
  isOpen,
  onConfirm,
  onCancel,
  recipeDetails,
  hasSecurityWarnings = false,
}: RecipeWarningModalProps) {
  const { t } = useTranslation('recipes');
  const { t: tCommon } = useTranslation('common');
  return (
    <Dialog open={isOpen}>
      <DialogPortal>
        <DialogOverlay />
        <DialogPrimitive.Content
          className={cn(
            'bg-background-default data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 fixed top-[50%] left-[50%] z-50 grid w-full max-w-[calc(100%-2rem)] translate-x-[-50%] translate-y-[-50%] gap-4 rounded-lg border p-6 shadow-lg duration-200 sm:max-w-[80vw] max-h-[80vh] flex flex-col p-0'
          )}
          onPointerDownOutside={(e) => e.preventDefault()}
          onEscapeKeyDown={(e) => e.preventDefault()}
        >
          <DialogHeader className="flex-shrink-0 p-6 pb-0">
            <DialogTitle>
              {hasSecurityWarnings ? t('warning.securityTitle') : t('warning.newRecipeTitle')}
            </DialogTitle>
            <DialogDescription>
              {!hasSecurityWarnings && t('warning.newRecipeNote') + ' '}
              {t('warning.trustSource')}
            </DialogDescription>
          </DialogHeader>

          {hasSecurityWarnings && (
            <div className="px-6">
              <div className="bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-800 rounded-lg p-4">
                <div className="flex items-start">
                  <div className="ml-3">
                    <div className="mt-2 text-sm text-yellow-700 dark:text-yellow-300">
                      <p>
                        {t('warning.hiddenCharacters')}
                      </p>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          )}

          <div className="flex-1 overflow-y-auto p-6 pt-4">
            <div className="bg-background-muted p-4 rounded-lg">
              <h3 className="font-medium mb-3 text-sm text-text-standard">{t('warning.recipePreview')}</h3>
              <div className="space-y-4">
                {recipeDetails.title && (
                  <p className="text-text-standard">
                    <strong>{t('warning.title')}</strong> {recipeDetails.title}
                  </p>
                )}
                {recipeDetails.description && (
                  <p className="text-text-standard">
                    <strong>{t('warning.description')}</strong> {recipeDetails.description}
                  </p>
                )}
                {recipeDetails.instructions && (
                  <div>
                    <h4 className="font-medium text-text-standard mb-1">{t('warning.instructions')}</h4>
                    <MarkdownContent content={recipeDetails.instructions} className="text-sm" />
                  </div>
                )}
              </div>
            </div>
          </div>

          <DialogFooter className="flex-shrink-0 p-6 pt-0">
            <Button variant="outline" onClick={onCancel}>
              {tCommon('cancel')}
            </Button>
            <Button onClick={onConfirm}>{t('warning.trustAndExecute')}</Button>
          </DialogFooter>
        </DialogPrimitive.Content>
      </DialogPortal>
    </Dialog>
  );
}
