import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import mammoth from 'mammoth';
import DOMPurify from 'dompurify';
import { documentApi } from '../../../api/documents';

interface WordPreviewProps {
  teamId?: string;
  docId?: string;
  contentUrl?: string;
}

export function WordPreview({ teamId, docId, contentUrl }: WordPreviewProps) {
  const { t } = useTranslation();
  const [html, setHtml] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

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
    fetch(url, { credentials: 'include' })
      .then((res) => {
        if (!res.ok) throw new Error('Failed to fetch document');
        return res.arrayBuffer();
      })
      .then((buffer) => mammoth.convertToHtml({ arrayBuffer: buffer }))
      .then((result) => {
        if (!cancelled) {
          setHtml(DOMPurify.sanitize(result.value));
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
  }, [teamId, docId, contentUrl]);

  if (loading) {
    return <div className="document-preview-scroll p-4 text-muted-foreground">{t('common.loading')}</div>;
  }

  if (error) {
    return <div className="document-preview-scroll p-4 text-destructive">{error}</div>;
  }

  return (
    <div className="document-preview-scroll p-4 sm:p-6">
      <div className="document-preview-paper mx-auto max-w-4xl p-4 sm:p-6">
        <div
          className="document-preview-prose prose prose-sm dark:prose-invert max-w-none"
          dangerouslySetInnerHTML={{ __html: html }}
        />
      </div>
    </div>
  );
}
