import { useState } from 'react';
import { useTranslation } from 'react-i18next';

interface ImagePreviewProps {
  contentUrl: string;
  fileName: string;
}

export function ImagePreview({ contentUrl, fileName }: ImagePreviewProps) {
  const { t } = useTranslation();
  const [scale, setScale] = useState(1);

  return (
    <div className="flex flex-col items-center h-full overflow-auto p-4">
      <div className="flex items-center gap-2 mb-3">
        <button
          className="px-2 py-1 text-sm border rounded hover:bg-muted"
          onClick={() => setScale((s) => Math.max(0.25, s - 0.25))}
        >
          -
        </button>
        <span className="text-sm text-muted-foreground">{Math.round(scale * 100)}%</span>
        <button
          className="px-2 py-1 text-sm border rounded hover:bg-muted"
          onClick={() => setScale((s) => Math.min(4, s + 0.25))}
        >
          +
        </button>
        <button
          className="px-2 py-1 text-sm border rounded hover:bg-muted"
          onClick={() => setScale(1)}
        >
          {t('common.reset')}
        </button>
      </div>
      <div className="flex-1 flex items-center justify-center overflow-auto">
        <img
          src={contentUrl}
          alt={fileName}
          style={{ transform: `scale(${scale})`, transformOrigin: 'center' }}
          className="max-w-full transition-transform"
        />
      </div>
    </div>
  );
}
