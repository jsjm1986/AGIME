import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../ui/button';
import { FolderKey } from 'lucide-react';
import { GoosehintsModal } from './GoosehintsModal';

export const GoosehintsSection = () => {
  const { t } = useTranslation('settings');
  const [isModalOpen, setIsModalOpen] = useState(false);
  const directory = window.appConfig?.get('GOOSE_WORKING_DIR') as string;

  return (
    <>
      <div className="flex items-center justify-between px-2 py-2">
        <div className="flex-1">
          <h3 className="text-text-default">{t('goosehints.sectionTitle')}</h3>
          <p className="text-xs text-text-muted mt-[2px]">
            {t('goosehints.sectionDescription')}
          </p>
        </div>
        <Button
          onClick={() => setIsModalOpen(true)}
          variant="outline"
          size="sm"
          className="flex items-center gap-2"
        >
          <FolderKey size={16} />
          {t('goosehints.configure')}
        </Button>
      </div>
      {isModalOpen && (
        <GoosehintsModal directory={directory} setIsGoosehintsModalOpen={setIsModalOpen} />
      )}
    </>
  );
};
