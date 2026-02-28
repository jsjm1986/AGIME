import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { documentApi } from '../../../api/documents';

interface TextPreviewProps {
  teamId?: string;
  docId?: string;
  contentUrl?: string;
  mimeType: string;
}

function getLanguageFromMime(mime: string): string {
  const map: Record<string, string> = {
    'application/json': 'json',
    'application/xml': 'xml',
    'application/x-yaml': 'yaml',
    'application/javascript': 'javascript',
    'application/typescript': 'typescript',
    'application/x-sh': 'bash',
    'application/x-shellscript': 'bash',
    'application/sql': 'sql',
    'application/x-sql': 'sql',
    'application/toml': 'ini',
    'application/x-toml': 'ini',
    'application/graphql': 'graphql',
    'application/x-protobuf': 'protobuf',
    'text/html': 'html',
    'text/css': 'css',
    'text/javascript': 'javascript',
    'text/x-python': 'python',
    'text/x-rust': 'rust',
    'text/x-go': 'go',
    'text/x-java': 'java',
    'text/x-typescript': 'typescript',
    'text/x-c': 'c',
    'text/x-c++': 'cpp',
    'text/x-csharp': 'csharp',
    'text/x-kotlin': 'kotlin',
    'text/x-swift': 'swift',
    'text/x-scala': 'scala',
    'text/x-ruby': 'ruby',
    'text/x-perl': 'perl',
    'text/x-php': 'php',
    'text/x-lua': 'lua',
    'text/x-r': 'r',
    'text/x-shellscript': 'bash',
    'text/x-sh': 'bash',
    'text/x-sql': 'sql',
    'text/x-toml': 'ini',
    'text/x-ini': 'ini',
    'text/x-dockerfile': 'dockerfile',
    'text/x-makefile': 'makefile',
    'text/x-diff': 'diff',
    'text/csv': 'csv',
    'text/markdown': 'markdown',
  };
  return map[mime] || 'plaintext';
}

export function TextPreview({ teamId, docId, contentUrl, mimeType }: TextPreviewProps) {
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

  const lang = getLanguageFromMime(mimeType);

  return (
    <div className="h-full overflow-auto">
      <pre className="p-4 text-sm font-mono whitespace-pre-wrap break-words">
        <code data-language={lang}>{text}</code>
      </pre>
    </div>
  );
}
