import { useState, useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import DOMPurify from 'dompurify';
import { Light as SyntaxHighlighter } from 'react-syntax-highlighter';
import xml from 'react-syntax-highlighter/dist/esm/languages/hljs/xml';
import { vs2015 } from 'react-syntax-highlighter/dist/esm/styles/hljs';
import { documentApi } from '../../../api/documents';

SyntaxHighlighter.registerLanguage('xml', xml);

interface SvgPreviewProps {
  teamId: string;
  docId: string;
}

export function SvgPreview({ teamId, docId }: SvgPreviewProps) {
  const { t } = useTranslation();
  const [svg, setSvg] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<'rendered' | 'source'>('rendered');
  const [scale, setScale] = useState(1);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    documentApi.getTextContent(teamId, docId).then((res) => {
      if (!cancelled) {
        setSvg(res.text);
        setLoading(false);
      }
    }).catch((err) => {
      if (!cancelled) {
        setError(err.message);
        setLoading(false);
      }
    });

    return () => { cancelled = true; };
  }, [teamId, docId]);

  const sanitized = useMemo(() => DOMPurify.sanitize(svg, {
    USE_PROFILES: { svg: true, svgFilters: true },
    ADD_TAGS: ['use'],
  }), [svg]);

  if (loading) {
    return <div className="p-4 text-muted-foreground">{t('common.loading')}</div>;
  }

  if (error) {
    return <div className="p-4 text-destructive">{error}</div>;
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-1 px-3 py-1.5 border-b bg-muted/30">
        <button
          className={`px-2 py-1 text-xs rounded ${view === 'rendered' ? 'bg-primary text-primary-foreground' : 'hover:bg-muted'}`}
          onClick={() => setView('rendered')}
        >
          {t('documents.previewPanel.svg.render')}
        </button>
        <button
          className={`px-2 py-1 text-xs rounded ${view === 'source' ? 'bg-primary text-primary-foreground' : 'hover:bg-muted'}`}
          onClick={() => setView('source')}
        >
          {t('documents.previewPanel.svg.source')}
        </button>
        {view === 'rendered' && (
          <>
            <span className="mx-2 h-4 w-px bg-border" />
            <button
              className="px-2 py-1 text-sm border rounded hover:bg-muted"
              onClick={() => setScale((s) => Math.max(0.25, s - 0.25))}
            >
              -
            </button>
            <span className="text-xs text-muted-foreground w-10 text-center">{Math.round(scale * 100)}%</span>
            <button
              className="px-2 py-1 text-sm border rounded hover:bg-muted"
              onClick={() => setScale((s) => Math.min(4, s + 0.25))}
            >
              +
            </button>
            <button
              className="px-2 py-1 text-xs border rounded hover:bg-muted"
              onClick={() => setScale(1)}
            >
              {t('common.reset')}
            </button>
          </>
        )}
      </div>
      <div className="flex-1 overflow-auto">
        {view === 'rendered' ? (
          <div className="flex items-center justify-center min-h-full p-4">
            <div
              style={{ transform: `scale(${scale})`, transformOrigin: 'center' }}
              className="transition-transform"
              dangerouslySetInnerHTML={{ __html: sanitized }}
            />
          </div>
        ) : (
          <SyntaxHighlighter
            language="xml"
            style={vs2015}
            customStyle={{ margin: 0, padding: '1rem', fontSize: '0.8125rem', height: '100%' }}
            wrapLongLines
          >
            {svg}
          </SyntaxHighlighter>
        )}
      </div>
    </div>
  );
}
