import { useState, useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import DOMPurify from 'dompurify';
import { Light as SyntaxHighlighter } from 'react-syntax-highlighter';
import xml from 'react-syntax-highlighter/dist/esm/languages/hljs/xml';
import { vs2015 } from 'react-syntax-highlighter/dist/esm/styles/hljs';
import { documentApi } from '../../../api/documents';

SyntaxHighlighter.registerLanguage('xml', xml);

interface HtmlPreviewProps {
  teamId: string;
  docId: string;
}

export function HtmlPreview({ teamId, docId }: HtmlPreviewProps) {
  const { t } = useTranslation();
  const [html, setHtml] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<'rendered' | 'source'>('rendered');

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    documentApi.getTextContent(teamId, docId).then((res) => {
      if (!cancelled) {
        setHtml(res.text);
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

  const sanitized = useMemo(() => DOMPurify.sanitize(html, {
    WHOLE_DOCUMENT: true,
    ADD_TAGS: ['style'],
    FORBID_TAGS: ['script'],
  }), [html]);

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
          {t('documents.previewPanel.html.rendered')}
        </button>
        <button
          className={`px-2 py-1 text-xs rounded ${view === 'source' ? 'bg-primary text-primary-foreground' : 'hover:bg-muted'}`}
          onClick={() => setView('source')}
        >
          {t('documents.previewPanel.html.source')}
        </button>
      </div>
      <div className="flex-1 overflow-auto">
        {view === 'rendered' ? (
          <iframe
            sandbox=""
            srcDoc={sanitized}
            className="w-full h-full border-0"
            title="HTML Preview"
          />
        ) : (
          <SyntaxHighlighter
            language="xml"
            style={vs2015}
            customStyle={{ margin: 0, padding: '1rem', fontSize: '0.8125rem', height: '100%' }}
            wrapLongLines
          >
            {html}
          </SyntaxHighlighter>
        )}
      </div>
    </div>
  );
}
