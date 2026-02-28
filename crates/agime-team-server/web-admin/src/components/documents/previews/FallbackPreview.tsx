import { useTranslation } from 'react-i18next';
import { formatFileSize } from '../../../api/documents';
import { Button } from '../../ui/button';

export interface PreviewDocumentMeta {
  name: string;
  mime_type: string;
  file_size: number;
}

interface FallbackPreviewProps {
  document: PreviewDocumentMeta;
  onDownload?: () => void;
}

export function FallbackPreview({ document, onDownload }: FallbackPreviewProps) {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col items-center justify-center h-full gap-4 text-muted-foreground">
      <div className="text-6xl">📄</div>
      <div className="text-center">
        <p className="font-medium text-foreground">{document.name}</p>
        <p className="text-sm mt-1">{document.mime_type}</p>
        <p className="text-sm">{formatFileSize(document.file_size)}</p>
      </div>
      <p className="text-sm">{t('documents.noPreview')}</p>
      {onDownload && <Button onClick={onDownload}>{t('documents.download')}</Button>}
    </div>
  );
}
