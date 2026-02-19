import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import JSZip from 'jszip';
import { documentApi } from '../../../api/documents';

interface PptPreviewProps {
  teamId: string;
  docId: string;
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

export function PptPreview({ teamId, docId }: PptPreviewProps) {
  const { t } = useTranslation();
  const [slides, setSlides] = useState<SlideContent[]>([]);
  const [current, setCurrent] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    const url = documentApi.getContentUrl(teamId, docId);
    fetch(url, { credentials: 'include' })
      .then((res) => {
        if (!res.ok) throw new Error('Failed to fetch presentation');
        return res.arrayBuffer();
      })
      .then((buffer) => JSZip.loadAsync(buffer))
      .then(async (zip) => {
        if (cancelled) return;

        // Find slide files: ppt/slides/slide1.xml, slide2.xml, ...
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

        if (!cancelled) {
          setSlides(parsed);
          setCurrent(0);
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
  }, [teamId, docId]);

  const goTo = useCallback((idx: number) => {
    setCurrent(Math.max(0, Math.min(slides.length - 1, idx)));
  }, [slides.length]);

  if (loading) {
    return <div className="p-4 text-muted-foreground">{t('common.loading')}</div>;
  }

  if (error) {
    return <div className="p-4 text-destructive">{error}</div>;
  }

  if (slides.length === 0) {
    return <div className="p-4 text-muted-foreground">{t('documents.noPreview')}</div>;
  }

  const slide = slides[current];

  return (
    <div className="flex flex-col h-full">
      {/* Navigation bar */}
      <div className="flex items-center gap-2 px-3 py-1.5 border-b bg-muted/30">
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
              className={`flex-shrink-0 w-6 h-6 text-[10px] rounded border ${
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
      <div className="flex-1 overflow-auto flex items-center justify-center p-6">
        <div className="w-full max-w-2xl aspect-[16/9] bg-background border rounded-lg shadow-sm flex flex-col justify-center p-8 gap-3">
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
