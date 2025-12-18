import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AlertTriangle } from 'lucide-react';
import { Button } from './button';
import { toastError } from '../../toasts';
import { diagnostics } from '../../api';

interface DiagnosticsModalProps {
  isOpen: boolean;
  onClose: () => void;
  sessionId: string;
}

export const DiagnosticsModal: React.FC<DiagnosticsModalProps> = ({
  isOpen,
  onClose,
  sessionId,
}) => {
  const { t } = useTranslation('settings');
  const { t: tCommon } = useTranslation('common');
  const [isDownloading, setIsDownloading] = useState(false);

  const handleDownload = async () => {
    setIsDownloading(true);

    try {
      const response = await diagnostics({
        path: { session_id: sessionId },
        throwOnError: true,
      });

      const blob = new Blob([response.data], { type: 'application/zip' });
      const url = window.URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `diagnostics_${sessionId}.zip`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      window.URL.revokeObjectURL(url);

      onClose();
    } catch {
      toastError({
        title: t('diagnostics.error'),
        msg: t('diagnostics.downloadFailed'),
      });
    } finally {
      setIsDownloading(false);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
      <div className="bg-background-default border border-borderStandard rounded-lg p-6 max-w-md mx-4">
        <div className="flex items-start gap-3 mb-4">
          <AlertTriangle className="text-orange-500 flex-shrink-0 mt-1" size={20} />
          <div>
            <h3 className="text-lg font-semibold text-textStandard mb-2">{t('diagnostics.title')}</h3>
            <p className="text-sm text-textSubtle mb-3">
              {t('diagnostics.description')}
            </p>
            <ul className="text-sm text-textSubtle list-disc list-inside space-y-1 mb-3">
              <li>{t('diagnostics.systemInfo')}</li>
              <li>{t('diagnostics.sessionMessages')}</li>
              <li>{t('diagnostics.logFiles')}</li>
              <li>{t('diagnostics.configSettings')}</li>
            </ul>
            <p className="text-sm text-textSubtle">
              {t('diagnostics.warning')}
            </p>
          </div>
        </div>
        <div className="flex gap-2 justify-end">
          <Button onClick={onClose} variant="outline" size="sm" disabled={isDownloading}>
            {tCommon('cancel')}
          </Button>
          <Button
            onClick={handleDownload}
            variant="outline"
            size="sm"
            disabled={isDownloading}
            className="bg-slate-600 text-white hover:bg-slate-700"
          >
            {isDownloading ? t('diagnostics.downloading') : tCommon('download')}
          </Button>
        </div>
      </div>
    </div>
  );
};
