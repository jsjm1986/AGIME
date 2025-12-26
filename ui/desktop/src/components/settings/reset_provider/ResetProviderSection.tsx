import { useTranslation } from 'react-i18next';
import { Button } from '../../ui/button';
import { RefreshCw } from 'lucide-react';
import { useConfig } from '../../ConfigContext';
import { View, ViewOptions } from '../../../utils/navigationUtils';

interface ResetProviderSectionProps {
  setView: (view: View, viewOptions?: ViewOptions) => void;
}

export default function ResetProviderSection(_props: ResetProviderSectionProps) {
  const { t } = useTranslation('settings');
  const { remove } = useConfig();

  const handleResetProvider = async () => {
    try {
      // Remove both legacy (GOOSE_) and new (AGIME_) keys for complete reset
      await remove('GOOSE_PROVIDER', false);
      await remove('GOOSE_MODEL', false);
      await remove('AGIME_PROVIDER', false);
      await remove('AGIME_MODEL', false);

      // Refresh the page to trigger the ProviderGuard check
      window.location.reload();
    } catch (error) {
      console.error('Failed to reset provider and model:', error);
    }
  };

  return (
    <div className="p-2">
      <Button
        onClick={handleResetProvider}
        variant="destructive"
        className="flex items-center justify-center gap-2"
      >
        <RefreshCw className="h-4 w-4" />
        {t('models.resetProvider.button')}
      </Button>
      <p className="text-xs text-textSubtle mt-2">
        {t('models.resetProvider.warning')}
      </p>
    </div>
  );
}
