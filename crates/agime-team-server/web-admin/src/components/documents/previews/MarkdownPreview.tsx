import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { documentApi } from '../../../api/documents';

interface MarkdownPreviewProps {
  teamId?: string;
  docId?: string;
  contentUrl?: string;
}

export function MarkdownPreview({ teamId, docId, contentUrl }: MarkdownPreviewProps) {
  const { t } = useTranslation();
  const [text, setText] = useState<string>('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    if (contentUrl) {
      fetch(contentUrl, { credentials: 'include' }).then((res) => {
        if (!res.ok) throw new Error('Failed to fetch document');
        return res.text();
      }).then((value) => {
        if (!cancelled) {
          setText(value);
          setLoading(false);
        }
      }).catch((err) => {
        if (!cancelled) {
          setError(err.message);
          setLoading(false);
        }
      });
    } else if (teamId && docId) {
      documentApi.getTextContent(teamId, docId).then((res) => {
        if (!cancelled) {
          setText(res.text);
          setLoading(false);
        }
      }).catch((err) => {
        if (!cancelled) {
          setError(err.message);
          setLoading(false);
        }
      });
    } else {
      setError('Invalid document source');
      setLoading(false);
    }

    return () => { cancelled = true; };
  }, [teamId, docId, contentUrl]);

  if (loading) {
    return <div className="p-4 text-muted-foreground">{t('common.loading')}</div>;
  }

  if (error) {
    return <div className="p-4 text-destructive">{error}</div>;
  }

  return (
    <div className="h-full overflow-auto p-6">
      <div className="prose prose-sm dark:prose-invert max-w-none">
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{text}</ReactMarkdown>
      </div>
    </div>
  );
}
