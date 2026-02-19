import { useState, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { documentApi } from '../../../api/documents';

interface FontPreviewProps {
  teamId: string;
  docId: string;
}

const SAMPLE_SIZES = [16, 24, 36, 48, 72];
const SAMPLE_EN = 'The quick brown fox jumps over the lazy dog';
const SAMPLE_ZH = '天地玄黄宇宙洪荒日月盈昃辰宿列张';

export function FontPreview({ teamId, docId }: FontPreviewProps) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [fontFamily, setFontFamily] = useState('');
  const [customText, setCustomText] = useState('');
  const blobUrlRef = useRef<string | null>(null);
  const styleRef = useRef<HTMLStyleElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    const url = documentApi.getContentUrl(teamId, docId);
    fetch(url, { credentials: 'include' })
      .then((res) => {
        if (!res.ok) throw new Error('Failed to fetch font');
        return res.blob();
      })
      .then((blob) => {
        if (cancelled) return;
        const blobUrl = URL.createObjectURL(blob);
        blobUrlRef.current = blobUrl;

        const family = `preview-font-${docId.slice(0, 8)}`;
        const style = document.createElement('style');
        style.textContent = `@font-face { font-family: '${family}'; src: url('${blobUrl}'); }`;
        document.head.appendChild(style);
        styleRef.current = style;

        setFontFamily(family);
        setLoading(false);
      })
      .catch((err) => {
        if (!cancelled) {
          setError(err.message);
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
      if (blobUrlRef.current) URL.revokeObjectURL(blobUrlRef.current);
      if (styleRef.current) styleRef.current.remove();
    };
  }, [teamId, docId]);

  if (loading) {
    return <div className="p-4 text-muted-foreground">{t('common.loading')}</div>;
  }

  if (error) {
    return <div className="p-4 text-destructive">{error}</div>;
  }

  return (
    <div className="h-full overflow-auto p-6 space-y-6">
      <div>
        <label className="text-sm text-muted-foreground block mb-2">
          {t('documents.previewPanel.font.sampleText')}
        </label>
        <input
          type="text"
          value={customText}
          onChange={(e) => setCustomText(e.target.value)}
          placeholder={t('documents.previewPanel.font.customPlaceholder')}
          className="w-full px-3 py-2 border rounded-md text-sm bg-background"
        />
      </div>

      <div className="space-y-4">
        {SAMPLE_SIZES.map((size) => (
          <div key={size} className="border-b pb-4">
            <span className="text-xs text-muted-foreground">{size}px</span>
            <p style={{ fontFamily: `'${fontFamily}'`, fontSize: `${size}px`, lineHeight: 1.4 }}>
              {customText || SAMPLE_EN}
            </p>
            {!customText && (
              <p style={{ fontFamily: `'${fontFamily}'`, fontSize: `${size}px`, lineHeight: 1.4 }} className="mt-1">
                {SAMPLE_ZH}
              </p>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
