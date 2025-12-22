import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../ui/button';
import { FolderKey } from 'lucide-react';
import { AgimehintsModal } from './AgimehintsModal';
import { hasCapability } from '../../../platform';

export const AgimehintsSection = () => {
  const { t } = useTranslation('settings');
  const [isModalOpen, setIsModalOpen] = useState(false);
  const directory = window.appConfig?.get('GOOSE_WORKING_DIR') as string;
  const canAccessFileSystem = hasCapability('fileSystem');

  // Hide section if file system is not available (e.g., on web platform)
  if (!canAccessFileSystem) {
    return (
      <div className="flex items-center justify-between px-2 py-2 opacity-50">
        <div className="flex-1">
          <h3 className="text-sm text-text-default">{t('agimehints.sectionTitle')}</h3>
          <p className="text-xs text-text-muted mt-[2px]">
            {t('agimehints.notAvailableOnWeb', 'File system access not available on web platform')}
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          className="flex items-center gap-2"
          disabled
        >
          <FolderKey size={16} />
          {t('agimehints.configure')}
        </Button>
      </div>
    );
  }

  return (
    <>
      <div className="flex items-center justify-between px-2 py-2">
        <div className="flex-1">
          <h3 className="text-sm text-text-default">{t('agimehints.sectionTitle')}</h3>
          <p className="text-xs text-text-muted mt-[2px]">
            {t('agimehints.sectionDescription')}
          </p>
        </div>
        <Button
          onClick={() => setIsModalOpen(true)}
          variant="outline"
          size="sm"
          className="flex items-center gap-2"
        >
          <FolderKey size={16} />
          {t('agimehints.configure')}
        </Button>
      </div>
      {isModalOpen && (
        <AgimehintsModal directory={directory} setIsAgimehintsModalOpen={setIsModalOpen} />
      )}
    </>
  );
};

// Backward compatibility
export { AgimehintsSection as GoosehintsSection };
