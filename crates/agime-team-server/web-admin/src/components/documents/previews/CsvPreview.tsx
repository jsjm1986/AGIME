import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import * as XLSX from 'xlsx';
import { documentApi } from '../../../api/documents';

interface CsvPreviewProps {
  teamId?: string;
  docId?: string;
  contentUrl?: string;
}

const MAX_ROWS = 1000;

export function CsvPreview({ teamId, docId, contentUrl }: CsvPreviewProps) {
  const { t } = useTranslation();
  const [rows, setRows] = useState<string[][]>([]);
  const [totalRows, setTotalRows] = useState(0);
  const [rawText, setRawText] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<'table' | 'raw'>('table');

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    const consume = (value: string) => {
      if (cancelled) return;
      setRawText(value);
      try {
        const wb = XLSX.read(value, { type: 'string' });
        const sheetName = wb.SheetNames[0];
        if (!sheetName) {
          setRows([]);
          setTotalRows(0);
          setLoading(false);
          return;
        }
        const ws = wb.Sheets[sheetName];
        const data: string[][] = XLSX.utils.sheet_to_json(ws, { header: 1, defval: '' });
        setTotalRows(data.length);
        setRows(data.slice(0, MAX_ROWS));
      } catch {
        setRows([]);
        setTotalRows(0);
      }
      setLoading(false);
    };

    if (contentUrl) {
      fetch(contentUrl, { credentials: 'include' }).then((res) => {
        if (!res.ok) throw new Error('Failed to fetch document');
        return res.text();
      }).then(consume).catch((err) => {
        if (!cancelled) {
          setError(err.message);
          setLoading(false);
        }
      });
    } else if (teamId && docId) {
      documentApi.getTextContent(teamId, docId).then((res) => consume(res.text)).catch((err) => {
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
    return <div className="document-preview-scroll p-4 text-muted-foreground">{t('common.loading')}</div>;
  }

  if (error) {
    return <div className="document-preview-scroll p-4 text-destructive">{error}</div>;
  }

  return (
    <div className="flex flex-col h-full">
      <div className="document-preview-subtoolbar flex items-center gap-1 px-3 py-1.5 border-b">
        <button
          className={`px-2 py-1 text-xs rounded ${view === 'table' ? 'bg-primary text-primary-foreground' : 'hover:bg-muted'}`}
          onClick={() => setView('table')}
        >
          {t('documents.previewPanel.csv.table')}
        </button>
        <button
          className={`px-2 py-1 text-xs rounded ${view === 'raw' ? 'bg-primary text-primary-foreground' : 'hover:bg-muted'}`}
          onClick={() => setView('raw')}
        >
          {t('documents.previewPanel.csv.raw')}
        </button>
        {totalRows > MAX_ROWS && view === 'table' && (
          <span className="ml-2 text-xs text-muted-foreground">
            {t('documents.previewPanel.csv.truncated', { count: MAX_ROWS })}
          </span>
        )}
      </div>
      <div className="document-preview-surface flex-1 overflow-auto">
        {view === 'table' ? (
          <table className="w-full text-sm border-collapse bg-[hsl(var(--ui-surface-panel-strong))]">
            <tbody>
              {rows.map((row, ri) => (
                <tr key={ri} className={ri === 0 ? 'bg-muted/50 font-medium sticky top-0' : 'hover:bg-muted/30'}>
                  <td className="px-2 py-1 border text-xs text-muted-foreground text-right w-10 bg-muted/20">
                    {ri + 1}
                  </td>
                  {row.map((cell, ci) => (
                    <td key={ci} className="px-2 py-1 border whitespace-nowrap max-w-[200px] truncate">
                      {String(cell)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <pre className="document-preview-code p-4 text-sm font-mono whitespace-pre-wrap break-words">
            {rawText}
          </pre>
        )}
      </div>
    </div>
  );
}
