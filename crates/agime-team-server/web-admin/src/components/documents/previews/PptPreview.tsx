import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import JSZip from 'jszip';
import { documentApi } from '../../../api/documents';

interface PptPreviewProps {
  teamId?: string;
  docId?: string;
  contentUrl?: string;
  fileName?: string;
  mimeType?: string;
}

interface SlideContent {
  index: number;
  texts: string[];
}

/** Extract text runs from a PPTX slide XML string */
function extractTextsFromSlideXml(xml: string): string[] {
  const parser = new DOMParser();
  const doc = parser.parseFromString(xml, 'application/xml');
  const paragraphs: string[] = [];

  // <a:p> elements contain text paragraphs
  const pNodes = doc.getElementsByTagName('a:p');
  for (let i = 0; i < pNodes.length; i++) {
    const runs = pNodes[i].getElementsByTagName('a:t');
    let line = '';
    for (let j = 0; j < runs.length; j++) {
      line += runs[j].textContent || '';
    }
    if (line.trim()) {
      paragraphs.push(line);
    }
  }
  return paragraphs;
}

export function PptPreview({ teamId, docId, contentUrl, fileName, mimeType }: PptPreviewProps) {
  const { t } = useTranslation();
  const [slides, setSlides] = useState<SlideContent[]>([]);
  const [current, setCurrent] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const isLegacyPpt = /\.ppt$/i.test(fileName || '') || mimeType === 'application/vnd.ms-powerpoint';

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    const url = contentUrl || (teamId && docId ? documentApi.getContentUrl(teamId, docId) : '');
    if (!url) {
      setError('Invalid document source');
      setLoading(false);
      return () => { cancelled = true; };
    }
    if (isLegacyPpt) {
      setError('旧版 .ppt 格式暂不支持可靠预览。建议先转换为 .pptx，再获得逐页预览。');
      setLoading(false);
      return () => { cancelled = true; };
    }
    fetch(url, { credentials: 'include' })
      .then((res) => {
        if (!res.ok) throw new Error('Failed to fetch presentation');
        return res.arrayBuffer();
      })
      .then(async (buffer) => {
        const tryZipPreview = async () => {
          const zip = await JSZip.loadAsync(buffer);
          const slideFiles = Object.keys(zip.files)
            .filter((name) => /^ppt\/slides\/slide\d+\.xml$/.test(name))
            .sort((a, b) => {
              const na = parseInt(a.match(/slide(\d+)/)?.[1] || '0');
              const nb = parseInt(b.match(/slide(\d+)/)?.[1] || '0');
              return na - nb;
            });

          const parsed: SlideContent[] = [];
          for (let i = 0; i < slideFiles.length; i++) {
            const xml = await zip.files[slideFiles[i]].async('text');
            parsed.push({ index: i + 1, texts: extractTextsFromSlideXml(xml) });
          }
          return parsed.filter((slide) => slide.texts.length > 0);
        };

        let parsed: SlideContent[] = [];
        try {
          parsed = await tryZipPreview();
        } catch {
          parsed = [];
        }

        if (!cancelled) {
          if (parsed.length === 0) {
            setError('当前演示文稿没有可读取的页面内容。');
          } else {
            setSlides(parsed);
            setCurrent(0);
          }
          setLoading(false);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setError(err.message);
          setLoading(false);
        }
      });

    return () => { cancelled = true; };
  }, [teamId, docId, contentUrl, isLegacyPpt]);

  const goTo = useCallback((idx: number) => {
    setCurrent(Math.max(0, Math.min(slides.length - 1, idx)));
  }, [slides.length]);

  if (loading) {
    return <div className="document-preview-scroll p-4 text-muted-foreground">{t('common.loading')}</div>;
  }

  if (error) {
    return <div className="document-preview-scroll p-4 text-destructive">{error}</div>;
  }

  if (slides.length === 0) {
    return <div className="document-preview-scroll p-4 text-muted-foreground">{t('documents.noPreview')}</div>;
  }

  const slide = slides[current];

  return (
    <div className="flex flex-col h-full">
      {/* Navigation bar */}
      <div className="document-preview-subtoolbar flex items-center gap-2 px-3 py-1.5 border-b">
        <button
          className="px-2 py-1 text-xs border rounded hover:bg-muted disabled:opacity-40"
          disabled={current === 0}
          onClick={() => goTo(current - 1)}
        >
          ←
        </button>
        <span className="text-xs text-muted-foreground">
          {slide.index} / {slides.length}
        </span>
        <button
          className="px-2 py-1 text-xs border rounded hover:bg-muted disabled:opacity-40"
          disabled={current === slides.length - 1}
          onClick={() => goTo(current + 1)}
        >
          →
        </button>
        {/* Thumbnail strip */}
        <span className="mx-2 h-4 w-px bg-border" />
        <div className="flex gap-1 overflow-x-auto flex-1">
          {slides.map((s, i) => (
            <button
              key={i}
              onClick={() => setCurrent(i)}
              className={`flex-shrink-0 w-6 h-6 text-micro rounded border ${
                i === current
                  ? 'bg-primary text-primary-foreground border-primary'
                  : 'hover:bg-muted border-border'
              }`}
            >
              {s.index}
            </button>
          ))}
        </div>
      </div>

      {/* Slide content */}
      <div className="document-preview-scroll flex-1 flex items-center justify-center p-4 sm:p-6">
        <div className="document-preview-paper flex w-full max-w-2xl aspect-[16/9] flex-col justify-center gap-3 p-5 sm:p-8">
          {slide.texts.length > 0 ? (
            slide.texts.map((text, i) => (
              <p
                key={i}
                className={
                  i === 0
                    ? 'text-xl font-semibold text-foreground'
                    : 'text-sm text-muted-foreground'
                }
              >
                {text}
              </p>
            ))
          ) : (
            <p className="text-sm text-muted-foreground text-center italic">
              ({t('documents.noPreview')})
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
