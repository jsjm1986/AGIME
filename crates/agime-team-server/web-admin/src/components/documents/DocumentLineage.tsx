import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { documentApi, formatFileSize } from '../../api/documents';
import type { DocumentSummary, SourceDocumentSnapshot } from '../../api/documents';
import { StatusBadge, DOC_STATUS_MAP } from '../ui/status-badge';
import { formatDateTime } from '../../utils/format';

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
    ? <StatusBadge status="info">{t('documents.origin.agent')}</StatusBadge>
    : <StatusBadge status="neutral">{t('documents.origin.human')}</StatusBadge>;
}

function statusBadge(status: string, t: (k: string) => string) {
  return (
    <StatusBadge status={DOC_STATUS_MAP[status] || 'neutral'}>
      {t(`documents.status.${status}`)}
    </StatusBadge>
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
              <StatusBadge status="error">
                {t('documents.deletedSource')}
              </StatusBadge>
            ) : statusBadge(doc.status, t)}
            {isSnapshot && (
              <StatusBadge status="neutral">
                {t('documents.sourceSnapshot')}
              </StatusBadge>
            )}
          </div>
          {!isSnapshot && (
            <p className="text-xs text-muted-foreground">
              {formatFileSize(doc.file_size)} · {formatDateTime(doc.created_at)}
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
