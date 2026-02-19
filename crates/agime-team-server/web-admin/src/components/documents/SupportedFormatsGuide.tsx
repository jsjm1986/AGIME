import { useState, useRef, useEffect } from 'react';
import { useTranslation } from 'react-i18next';

const FORMAT_CATEGORIES = [
  { key: 'documents', exts: ['.txt', '.md', '.html', '.htm', '.pdf', '.docx', '.doc', '.pptx', '.ppt'] },
  { key: 'spreadsheets', exts: ['.xlsx', '.xls', '.csv'] },
  { key: 'code', exts: ['.js', '.ts', '.py', '.java', '.go', '.rs', '.json', '.yaml', '.xml', '.css', '.sql', '.sh', '.toml', '.ini', '.lua', '.rb', '.php', '.kt', '.swift', '.proto'] },
  { key: 'images', exts: ['.png', '.jpg', '.gif', '.webp', '.svg', '.bmp'] },
  { key: 'media', exts: ['.mp3', '.wav', '.ogg', '.mp4', '.webm'] },
  { key: 'fonts', exts: ['.ttf', '.otf', '.woff', '.woff2'] },
] as const;

export function SupportedFormatsGuide() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpen(!open)}
        className="w-7 h-7 flex items-center justify-center rounded-full border text-xs text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
        title={t('documents.previewPanel.supportedFormats')}
      >
        ?
      </button>
      {open && (
        <div className="absolute right-0 top-full mt-1 z-50 w-80 bg-popover border rounded-lg shadow-lg p-3">
          <p className="text-sm font-medium mb-2">{t('documents.previewPanel.supportedFormats')}</p>
          <div className="space-y-2">
            {FORMAT_CATEGORIES.map(({ key, exts }) => (
              <div key={key}>
                <span className="text-xs font-medium text-muted-foreground">
                  {t(`documents.previewPanel.formatCategories.${key}`)}
                </span>
                <div className="flex flex-wrap gap-1 mt-0.5">
                  {exts.map((ext) => (
                    <span
                      key={ext}
                      className="px-1.5 py-0.5 text-xs bg-muted rounded"
                    >
                      {ext}
                    </span>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
