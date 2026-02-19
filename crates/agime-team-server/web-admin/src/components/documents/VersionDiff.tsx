import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { diffLines as computeDiffLines } from 'diff';
import { documentApi } from '../../api/documents';
import type { VersionSummary } from '../../api/documents';

interface VersionDiffProps {
  teamId: string;
  docId: string;
  version1: VersionSummary;
  version2: VersionSummary;
  onClose: () => void;
}

interface DiffLine {
  type: 'added' | 'removed' | 'unchanged';
  content: string;
  oldLineNum?: number;
  newLineNum?: number;
}

export function VersionDiff({
  teamId,
  docId,
  version1,
  version2,
  onClose,
}: VersionDiffProps) {
  const { t } = useTranslation();
  const [lines, setLines] = useState<DiffLine[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);

    Promise.all([
      documentApi.getVersionContent(teamId, docId, version1.id),
      documentApi.getVersionContent(teamId, docId, version2.id),
    ]).then(([res1, res2]) => {
      if (cancelled) return;
      const changes = computeDiffLines(res1.text, res2.text);
      const result: DiffLine[] = [];
      let oldNum = 1;
      let newNum = 1;

      for (const change of changes) {
        const changeLines = change.value.replace(/\n$/, '').split('\n');
        for (const line of changeLines) {
          if (change.added) {
            result.push({ type: 'added', content: line, newLineNum: newNum++ });
          } else if (change.removed) {
            result.push({ type: 'removed', content: line, oldLineNum: oldNum++ });
          } else {
            result.push({ type: 'unchanged', content: line, oldLineNum: oldNum++, newLineNum: newNum++ });
          }
        }
      }

      setLines(result);
      setLoading(false);
    }).catch((err) => {
      if (!cancelled) {
        console.error('Failed to load version diff:', err);
        setLoading(false);
      }
    });

    return () => { cancelled = true; };
  }, [teamId, docId, version1.id, version2.id]);

  if (loading) {
    return <div className="p-4 text-muted-foreground">{t('common.loading')}</div>;
  }

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2 border-b bg-muted/30">
        <span className="text-sm">
          {t('documents.versionNumber', { number: version1.version_number })}
          {' → '}
          {t('documents.versionNumber', { number: version2.version_number })}
        </span>
        <button
          className="text-sm text-muted-foreground hover:text-foreground"
          onClick={onClose}
        >
          {t('documents.closePreview')}
        </button>
      </div>

      {/* Diff content */}
      <div className="flex-1 overflow-auto">
        <pre className="text-sm font-mono">
          {lines.map((line, i) => (
            <div
              key={i}
              className={`flex ${
                line.type === 'added'
                  ? 'bg-green-50 dark:bg-green-950'
                  : line.type === 'removed'
                  ? 'bg-red-50 dark:bg-red-950'
                  : ''
              }`}
            >
              <span className="w-12 text-right pr-2 text-muted-foreground select-none border-r text-xs leading-6">
                {line.oldLineNum ?? ''}
              </span>
              <span className="w-12 text-right pr-2 text-muted-foreground select-none border-r text-xs leading-6">
                {line.newLineNum ?? ''}
              </span>
              <span className="w-6 text-center select-none text-xs leading-6">
                {line.type === 'added' ? '+' : line.type === 'removed' ? '-' : ' '}
              </span>
              <span className="flex-1 px-2 leading-6 whitespace-pre-wrap break-all">
                {line.content}
              </span>
            </div>
          ))}
        </pre>
      </div>
    </div>
  );
}

