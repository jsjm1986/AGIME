import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import * as XLSX from 'xlsx';
import { documentApi } from '../../../api/documents';

interface ExcelPreviewProps {
  teamId: string;
  docId: string;
}

export function ExcelPreview({ teamId, docId }: ExcelPreviewProps) {
  const { t } = useTranslation();
  const [sheets, setSheets] = useState<{ name: string; data: string[][] }[]>([]);
  const [activeSheet, setActiveSheet] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    const url = documentApi.getContentUrl(teamId, docId);
    fetch(url, { credentials: 'include' })
      .then((res) => {
        if (!res.ok) throw new Error('Failed to fetch document');
        return res.arrayBuffer();
      })
      .then((buffer) => {
        const wb = XLSX.read(buffer, { type: 'array' });
        const parsed = wb.SheetNames.map((name) => {
          const ws = wb.Sheets[name];
          const data: string[][] = XLSX.utils.sheet_to_json(ws, {
            header: 1,
            defval: '',
          });
          return { name, data };
        });
        if (!cancelled) {
          setSheets(parsed);
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

  if (loading) {
    return <div className="p-4 text-muted-foreground">{t('common.loading')}</div>;
  }

  if (error) {
    return <div className="p-4 text-destructive">{error}</div>;
  }

  const current = sheets[activeSheet];
  if (!current) {
    return <div className="p-4 text-muted-foreground">{t('documents.emptySpreadsheet')}</div>;
  }

  return (
    <div className="flex flex-col h-full">
      {/* Sheet tabs */}
      {sheets.length > 1 && (
        <div className="flex border-b bg-muted/30 px-2 gap-1 overflow-x-auto">
          {sheets.map((s, i) => (
            <button
              key={s.name}
              className={`px-3 py-1.5 text-xs whitespace-nowrap border-b-2 ${
                i === activeSheet
                  ? 'border-primary text-primary font-medium'
                  : 'border-transparent text-muted-foreground hover:text-foreground'
              }`}
              onClick={() => setActiveSheet(i)}
            >
              {s.name}
            </button>
          ))}
        </div>
      )}

      {/* Table */}
      <div className="flex-1 overflow-auto">
        <table className="w-full text-sm border-collapse">
          <tbody>
            {current.data.map((row, ri) => (
              <tr key={ri} className={ri === 0 ? 'bg-muted/50 font-medium' : 'hover:bg-muted/30'}>
                {/* Row number */}
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
      </div>
    </div>
  );
}
