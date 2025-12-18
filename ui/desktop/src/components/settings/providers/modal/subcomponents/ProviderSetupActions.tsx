import { SyntheticEvent } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../../../ui/button';
import { Trash2, AlertTriangle } from 'lucide-react';
import { ConfigKey } from '../../../../../api';

interface ProviderSetupActionsProps {
  onCancel: () => void;
  onSubmit: (e: SyntheticEvent) => void;
  onDelete?: () => void;
  showDeleteConfirmation?: boolean;
  onConfirmDelete?: () => void;
  onCancelDelete?: () => void;
  canDelete?: boolean;
  providerName?: string;
  requiredParameters?: ConfigKey[];
  isActiveProvider?: boolean; // Made optional with default false
}

/**
 * Renders the action buttons at the bottom of the provider modal.
 * Includes submit, cancel, and delete functionality with confirmation.
 */
export default function ProviderSetupActions({
  onCancel,
  onSubmit,
  onDelete,
  showDeleteConfirmation,
  onConfirmDelete,
  onCancelDelete,
  canDelete,
  providerName,
  requiredParameters,
  isActiveProvider = false, // Default value provided
}: ProviderSetupActionsProps) {
  const { t } = useTranslation('settings');
  const { t: tCommon } = useTranslation('common');

  // If we're showing delete confirmation, render the delete confirmation buttons
  if (showDeleteConfirmation) {
    // Check if this is the active provider
    if (isActiveProvider) {
      return (
        <div className="w-full">
          <div className="w-full px-6 py-4 bg-yellow-600/20 border-t border-yellow-500/30">
            <p className="text-yellow-500 text-sm mb-2 flex items-start">
              <AlertTriangle className="h-4 w-4 mr-2 mt-0.5 flex-shrink-0" />
              <span>
                {t('provider.cannotDeleteActive', { provider: providerName })}
              </span>
            </p>
          </div>
          <Button
            variant="ghost"
            onClick={onCancelDelete}
            className="w-full h-[60px] rounded-none hover:bg-bgSubtle text-textSubtle hover:text-textStandard text-md font-regular"
          >
            {t('provider.ok')}
          </Button>
        </div>
      );
    }

    // Normal delete confirmation
    return (
      <div className="w-full">
        <div className="w-full px-6 py-4 bg-red-900/20 border-t border-red-500/30">
          <p className="text-red-400 text-sm mb-2">
            {t('provider.deleteConfirmation', { provider: providerName })}
          </p>
        </div>
        <Button
          onClick={onConfirmDelete}
          className="w-full h-[60px] rounded-none border-b border-borderSubtle bg-transparent hover:bg-red-900/20 text-red-500 font-medium text-md"
        >
          <Trash2 className="h-4 w-4 mr-2" /> {t('provider.confirmDelete')}
        </Button>
        <Button
          variant="ghost"
          onClick={onCancelDelete}
          className="w-full h-[60px] rounded-none hover:bg-bgSubtle text-textSubtle hover:text-textStandard text-md font-regular"
        >
          {tCommon('cancel')}
        </Button>
      </div>
    );
  }

  // Regular buttons (with delete if applicable)
  return (
    <div className="w-full">
      {canDelete && onDelete && (
        <Button
          type="button"
          onClick={onDelete}
          className="w-full h-[60px] rounded-none border-t border-borderSubtle bg-transparent hover:bg-bgSubtle text-red-500 font-medium text-md"
        >
          <Trash2 className="h-4 w-4 mr-2" /> {t('provider.deleteProvider')}
        </Button>
      )}
      {requiredParameters && requiredParameters.length > 0 ? (
        <>
          <Button
            type="submit"
            variant="ghost"
            onClick={onSubmit}
            className="w-full h-[60px] rounded-none border-t border-borderSubtle text-md hover:bg-bgSubtle text-textProminent font-medium"
          >
            {t('provider.submit')}
          </Button>
          <Button
            type="button"
            variant="ghost"
            onClick={onCancel}
            className="w-full h-[60px] rounded-none border-t border-borderSubtle hover:text-textStandard text-textSubtle hover:bg-bgSubtle text-md font-regular"
          >
            {tCommon('cancel')}
          </Button>
        </>
      ) : (
        <>
          <Button
            type="submit"
            variant="ghost"
            onClick={onSubmit}
            className="w-full h-[60px] rounded-none border-t border-borderSubtle text-md hover:bg-bgSubtle text-textProminent font-medium"
          >
            {t('provider.enableProvider')}
          </Button>
          <Button
            type="button"
            variant="ghost"
            onClick={onCancel}
            className="w-full h-[60px] rounded-none border-t border-borderSubtle hover:text-textStandard text-textSubtle hover:bg-bgSubtle text-md font-regular"
          >
            {tCommon('cancel')}
          </Button>
        </>
      )}
    </div>
  );
}
