import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { documentApi, formatFileSize } from '../../api/documents';
import type { DocumentSummary, SourceDocumentSnapshot } from '../../api/documents';

/** Convert a SourceDocumentSnapshot into a minimal DocumentSummary placeholder. */
function snapshotToSummary(snap: SourceDocumentSnapshot): DocumentSummary {
  return {
    id: snap.id,
    name: snap.name,
    display_name: null,
    description: null,
    mime_type: snap.mime_type,
    file_size: 0,
    folder_path: '',
    tags: [],
    uploaded_by: '',
    origin: snap.origin,
    status: 'archived',
    category: snap.category,
    source_document_ids: [],
    source_session_id: null,
    source_mission_id: null,
    created_by_agent_id: null,
    supersedes_id: null,
    lineage_description: null,
    created_at: '',
  };
}

interface DocumentLineageProps {
  teamId: string;
  docId: string;
  sourceSnapshots?: SourceDocumentSnapshot[];
  onNavigate?: (docId: string) => void;
}

function originBadge(origin: string, t: (k: string) => string) {
  return origin === 'agent'
    ? <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-purple-100 text-purple-800 dark:bg-purple-900 dark:text-purple-200">{t('documents.origin.agent')}</span>
    : <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200">{t('documents.origin.human')}</span>;
}

function statusBadge(status: string, t: (k: string) => string) {
  const colors: Record<string, string> = {
    active: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200',
    draft: 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200',
    accepted: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200',
    archived: 'bg-gray-100 text-gray-800 dark:bg-gray-900 dark:text-gray-200',
    superseded: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200',
  };
  return (
    <span className={`text-[10px] px-1.5 py-0.5 rounded-full ${colors[status] || 'bg-muted'}`}>
      {t(`documents.status.${status}`)}
    </span>
  );
}

export function DocumentLineage({ teamId, docId, sourceSnapshots, onNavigate }: DocumentLineageProps) {
  const { t } = useTranslation();
  const [lineage, setLineage] = useState<DocumentSummary[]>([]);
  const [derived, setDerived] = useState<DocumentSummary[]>([]);
  const [currentDocSnapshots, setCurrentDocSnapshots] = useState<SourceDocumentSnapshot[]>(sourceSnapshots || []);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);

    const fetches: [Promise<DocumentSummary[]>, Promise<DocumentSummary[]>] = [
      documentApi.getLineage(teamId, docId),
      documentApi.listDerived(teamId, docId).then(r => r.items),
    ];

    Promise.all(fetches).then(([lin, der]) => {
      if (cancelled) return;
      setLineage(lin);
      setDerived(der);

      // If sourceSnapshots not provided externally, try to find the current doc
      // in the lineage result to extract its source_snapshots
      if (!sourceSnapshots) {
        const currentInLineage = lin.find(d => d.id === docId);
        if (currentInLineage?.source_snapshots?.length) {
          setCurrentDocSnapshots(currentInLineage.source_snapshots);
        }
      }
    }).catch(e => {
      console.error('Failed to load lineage:', e);
    }).finally(() => {
      if (!cancelled) setLoading(false);
    });
    return () => { cancelled = true; };
  }, [teamId, docId, sourceSnapshots]);

  if (loading) {
    return <div className="text-center py-8 text-muted-foreground text-sm">{t('common.loading')}</div>;
  }

  // Build snapshot fallback nodes for source_document_ids not found in lineage
  const lineageIds = new Set(lineage.map(d => d.id));
  const snapshotNodes = currentDocSnapshots
    .filter(snap => !lineageIds.has(snap.id))
    .map(snapshotToSummary);

  const allSources = [...snapshotNodes, ...lineage];
  const hasLineage = allSources.length > 0 || derived.length > 0;

  if (!hasLineage) {
    return (
      <div className="text-center py-8 text-muted-foreground text-sm">
        {t('documents.noAiDocuments')}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {/* Source chain (ancestors) */}
      {allSources.length > 0 && (
        <div className="space-y-1">
          <h4 className="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-2">
            {t('documents.lineageSourceDocuments')}
          </h4>
          {allSources.map((doc, i) => (
            <LineageNode
              key={doc.id}
              doc={doc}
              t={t}
              isCurrent={false}
              isLast={i === allSources.length - 1}
              isSnapshot={snapshotNodes.some(s => s.id === doc.id)}
              onClick={doc.status !== 'archived' ? () => onNavigate?.(doc.id) : undefined}
            />
          ))}
        </div>
      )}

      {/* Current document marker */}
      <div className="flex items-center gap-2 px-3 py-2 bg-primary/10 border border-primary/30 rounded-lg">
        <div className="h-2 w-2 rounded-full bg-primary" />
        <span className="text-sm font-medium">{t('documents.currentVersion')}</span>
      </div>

      {/* Derived documents */}
      {derived.length > 0 && (
        <div className="space-y-1">
          <h4 className="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-2">
            {t('documents.lineageDerivedDocuments')}
          </h4>
          {derived.map((doc, i) => (
            <LineageNode
              key={doc.id}
              doc={doc}
              t={t}
              isCurrent={false}
              isLast={i === derived.length - 1}
              onClick={() => onNavigate?.(doc.id)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function dotStyle(isCurrent: boolean, isArchived: boolean): string {
  if (isCurrent) return 'bg-primary border-primary';
  if (isArchived) return 'bg-muted border-muted-foreground/30';
  return 'bg-background border-muted-foreground/40';
}

function LineageNode({
  doc, t, isCurrent, isLast, isSnapshot, onClick,
}: {
  doc: DocumentSummary;
  t: (k: string) => string;
  isCurrent: boolean;
  isLast: boolean;
  isSnapshot?: boolean;
  onClick?: () => void;
}) {
  const isArchived = doc.status === 'archived';
  return (
    <div className="relative pl-6">
      {/* Vertical connector line */}
      {!isLast && (
        <div className="absolute left-[9px] top-6 bottom-0 w-px bg-border" />
      )}
      {/* Node dot */}
      <div className={`absolute left-1 top-2.5 h-2.5 w-2.5 rounded-full border-2 ${dotStyle(isCurrent, isArchived)}`} />

      <div
        className={`flex items-center gap-2 p-2 rounded ${isArchived ? 'opacity-60' : 'hover:bg-muted/50 cursor-pointer'}`}
        onClick={onClick}
      >
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5">
            <span className={`text-sm truncate ${isArchived ? 'line-through text-muted-foreground' : ''}`}>
              {doc.display_name || doc.name}
            </span>
            {originBadge(doc.origin, t)}
            {isArchived ? (
              <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200">
                {t('documents.deletedSource')}
              </span>
            ) : statusBadge(doc.status, t)}
            {isSnapshot && (
              <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-400">
                {t('documents.sourceSnapshot')}
              </span>
            )}
          </div>
          {!isSnapshot && (
            <p className="text-xs text-muted-foreground">
              {formatFileSize(doc.file_size)} · {new Date(doc.created_at).toLocaleString()}
            </p>
          )}
          {doc.lineage_description && (
            <p className="text-xs text-muted-foreground/80 mt-0.5 italic">
              {doc.lineage_description}
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
