import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { documentApi } from '../../../api/documents';

interface JsonPreviewProps {
  teamId: string;
  docId: string;
}

const MAX_STRING_DISPLAY = 500;

export function JsonPreview({ teamId, docId }: JsonPreviewProps) {
  const { t } = useTranslation();
  const [raw, setRaw] = useState('');
  const [parsed, setParsed] = useState<unknown>(null);
  const [parseError, setParseError] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<'tree' | 'raw'>('tree');

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    documentApi.getTextContent(teamId, docId).then((res) => {
      if (cancelled) return;
      setRaw(res.text);
      try {
        setParsed(JSON.parse(res.text));
        setParseError(false);
      } catch {
        setParseError(true);
      }
      setLoading(false);
    }).catch((err) => {
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

  // If JSON is invalid, fall back to raw text
  if (parseError) {
    return (
      <div className="h-full overflow-auto">
        <pre className="p-4 text-sm font-mono whitespace-pre-wrap break-words">{raw}</pre>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-1 px-3 py-1.5 border-b bg-muted/30">
        <button
          className={`px-2 py-1 text-xs rounded ${view === 'tree' ? 'bg-primary text-primary-foreground' : 'hover:bg-muted'}`}
          onClick={() => setView('tree')}
        >
          {t('documents.previewPanel.json.tree')}
        </button>
        <button
          className={`px-2 py-1 text-xs rounded ${view === 'raw' ? 'bg-primary text-primary-foreground' : 'hover:bg-muted'}`}
          onClick={() => setView('raw')}
        >
          {t('documents.previewPanel.json.raw')}
        </button>
      </div>
      <div className="flex-1 overflow-auto">
        {view === 'tree' ? (
          <div className="p-3 text-sm font-mono">
            <JsonNode value={parsed} defaultExpanded itemsLabel={t('documents.previewPanel.json.items')} />
          </div>
        ) : (
          <pre className="p-4 text-sm font-mono whitespace-pre-wrap break-words">
            {JSON.stringify(parsed, null, 2)}
          </pre>
        )}
      </div>
    </div>
  );
}

// --- Tree node component ---

interface JsonNodeProps {
  label?: string;
  value: unknown;
  defaultExpanded?: boolean;
  itemsLabel: string;
}

function JsonNode({ label, value, defaultExpanded = false, itemsLabel }: JsonNodeProps) {
  const [expanded, setExpanded] = useState(defaultExpanded);

  const toggle = useCallback(() => setExpanded((e) => !e), []);

  if (value === null) {
    return <Line label={label} valueClass="text-orange-500" value="null" />;
  }

  if (typeof value === 'boolean') {
    return <Line label={label} valueClass="text-blue-500" value={String(value)} />;
  }

  if (typeof value === 'number') {
    return <Line label={label} valueClass="text-green-600 dark:text-green-400" value={String(value)} />;
  }

  if (typeof value === 'string') {
    const display = value.length > MAX_STRING_DISPLAY
      ? `"${value.slice(0, MAX_STRING_DISPLAY)}…" (${value.length})`
      : `"${value}"`;
    return <Line label={label} valueClass="text-amber-700 dark:text-amber-400" value={display} />;
  }

  if (Array.isArray(value)) {
    if (value.length === 0) {
      return <Line label={label} valueClass="text-muted-foreground" value="[]" />;
    }
    return (
      <Collapsible
        label={label}
        bracket={['[', ']']}
        count={value.length}
        expanded={expanded}
        onToggle={toggle}
        itemsLabel={itemsLabel}
      >
        {value.map((item, i) => (
          <JsonNode key={i} label={String(i)} value={item} itemsLabel={itemsLabel} />
        ))}
      </Collapsible>
    );
  }

  if (typeof value === 'object') {
    const entries = Object.entries(value as Record<string, unknown>);
    if (entries.length === 0) {
      return <Line label={label} valueClass="text-muted-foreground" value="{}" />;
    }
    return (
      <Collapsible
        label={label}
        bracket={['{', '}']}
        count={entries.length}
        expanded={expanded}
        onToggle={toggle}
        itemsLabel={itemsLabel}
      >
        {entries.map(([k, v]) => (
          <JsonNode key={k} label={k} value={v} itemsLabel={itemsLabel} />
        ))}
      </Collapsible>
    );
  }

  return <Line label={label} valueClass="" value={String(value)} />;
}

function Line({ label, value, valueClass }: { label?: string; value: string; valueClass: string }) {
  return (
    <div className="leading-6 truncate">
      {label !== undefined && <span className="text-purple-600 dark:text-purple-400">{label}: </span>}
      <span className={valueClass}>{value}</span>
    </div>
  );
}

function Collapsible({
  label,
  bracket,
  count,
  expanded,
  onToggle,
  children,
  itemsLabel,
}: {
  label?: string;
  bracket: [string, string];
  count: number;
  expanded: boolean;
  onToggle: () => void;
  children: React.ReactNode;
  itemsLabel: string;
}) {
  return (
    <div>
      <div className="leading-6 cursor-pointer select-none hover:bg-muted/50 rounded -mx-1 px-1" onClick={onToggle}>
        <span className="inline-block w-4 text-center text-muted-foreground">{expanded ? '▾' : '▸'}</span>
        {label !== undefined && <span className="text-purple-600 dark:text-purple-400">{label}: </span>}
        {expanded ? (
          <span className="text-muted-foreground">{bracket[0]}</span>
        ) : (
          <span className="text-muted-foreground">{bracket[0]} {count} {itemsLabel} {bracket[1]}</span>
        )}
      </div>
      {expanded && (
        <>
          <div className="ml-4 border-l border-border/50 pl-2">{children}</div>
          <div className="leading-6 text-muted-foreground">{bracket[1]}</div>
        </>
      )}
    </div>
  );
}
