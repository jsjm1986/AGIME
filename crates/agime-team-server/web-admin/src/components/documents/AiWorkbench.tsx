import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui/select';
import { documentApi, formatFileSize } from '../../api/documents';
import type { DocumentSummary, DocumentStatusType } from '../../api/documents';
import { ConfirmDialog } from '../ui/confirm-dialog';

interface AiWorkbenchProps {
  teamId: string;
}

function statusColor(status: DocumentStatusType): string {
  switch (status) {
    case 'draft': return 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200';
    case 'accepted': return 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200';
    case 'archived': return 'bg-gray-100 text-gray-800 dark:bg-gray-900 dark:text-gray-200';
    case 'superseded': return 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200';
    default: return 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200';
  }
}

function categoryIcon(category: string): string {
  switch (category) {
    case 'translation': return '🌐';
    case 'summary': return '📋';
    case 'review': return '🔍';
    case 'code': return '💻';
    case 'report': return '📊';
    default: return '📄';
  }
}

interface GroupedDocs {
  key: string;
  label: string;
  docs: DocumentSummary[];
}

export function AiWorkbench({ teamId }: AiWorkbenchProps) {
  const { t } = useTranslation();
  const [documents, setDocuments] = useState<DocumentSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [statusFilter, setStatusFilter] = useState<string>('');
  const [groupBySource, setGroupBySource] = useState(true);
  const [page, setPage] = useState(1);
  const [totalPages, setTotalPages] = useState(0);
  const [retryTarget, setRetryTarget] = useState<string | null>(null);

  const loadData = useCallback(async () => {
    setLoading(true);
    try {
      const res = await documentApi.listAiWorkbench(teamId, undefined, undefined, page, 50);
      let items = res.items;
      if (statusFilter) {
        items = items.filter(d => d.status === statusFilter);
      }
      setDocuments(items);
      setTotalPages(res.total_pages);
    } catch (e) {
      console.error('Failed to load AI workbench:', e);
    } finally {
      setLoading(false);
    }
  }, [teamId, page, statusFilter]);

  useEffect(() => { loadData(); }, [loadData]);

  const handleUpdateStatus = async (docId: string, status: DocumentStatusType) => {
    try {
      await documentApi.updateStatus(teamId, docId, status);
      loadData();
    } catch (e) {
      console.error('Failed to update status:', e);
    }
  };

  const handleRetry = async (doc: DocumentSummary) => {
    setRetryTarget(doc.id);
  };

  const confirmRetry = async () => {
    if (!retryTarget) return;
    try {
      await documentApi.updateStatus(teamId, retryTarget, 'superseded');
      loadData();
    } catch (e) {
      console.error('Failed to retry:', e);
    } finally {
      setRetryTarget(null);
    }
  };

  // Group documents by source
  const grouped: GroupedDocs[] = groupBySource
    ? groupDocsBySource(documents, t)
    : [{ key: 'all', label: t('documents.allFiles'), docs: documents }];

  if (loading) {
    return <div className="text-center py-8 text-muted-foreground text-sm">{t('common.loading')}</div>;
  }

  if (documents.length === 0) {
    return (
      <div className="text-center py-12 text-muted-foreground">
        <p>{t('documents.noAiDocuments')}</p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {/* Toolbar */}
      <div className="flex items-center gap-2">
        <Select value={statusFilter || '__all__'} onValueChange={v => { setStatusFilter(v === '__all__' ? '' : v); setPage(1); }}>
          <SelectTrigger className="w-36 h-8">
            <SelectValue placeholder={t('documents.filterByStatus')} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__all__">{t('documents.filterAll')}</SelectItem>
            <SelectItem value="draft">{t('documents.status.draft')}</SelectItem>
            <SelectItem value="accepted">{t('documents.status.accepted')}</SelectItem>
            <SelectItem value="archived">{t('documents.status.archived')}</SelectItem>
            <SelectItem value="superseded">{t('documents.status.superseded')}</SelectItem>
          </SelectContent>
        </Select>
        <Button
          size="sm"
          variant={groupBySource ? 'default' : 'outline'}
          onClick={() => setGroupBySource(!groupBySource)}
        >
          {t('documents.groupBySource')}
        </Button>
      </div>

      {/* Document groups */}
      {grouped.map(group => (
        <div key={group.key} className="space-y-2">
          {groupBySource && (
            <h3 className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
              {group.label}
            </h3>
          )}
          {group.docs.map(doc => (
            <DocCard
              key={doc.id}
              doc={doc}
              t={t}
              onAccept={() => handleUpdateStatus(doc.id, 'accepted')}
              onArchive={() => handleUpdateStatus(doc.id, 'archived')}
              onRetry={() => handleRetry(doc)}
              onDownload={() => window.open(documentApi.getDownloadUrl(teamId, doc.id), '_blank')}
            />
          ))}
        </div>
      ))}

      {/* Pagination */}
      {totalPages > 1 && (
        <div className="flex items-center justify-center gap-2">
          <Button size="sm" variant="outline" disabled={page <= 1} onClick={() => setPage(p => p - 1)}>
            {t('pagination.previous')}
          </Button>
          <span className="text-xs">{page}/{totalPages}</span>
          <Button size="sm" variant="outline" disabled={page >= totalPages} onClick={() => setPage(p => p + 1)}>
            {t('pagination.next')}
          </Button>
        </div>
      )}
      <ConfirmDialog
        open={!!retryTarget}
        onOpenChange={(open) => { if (!open) setRetryTarget(null); }}
        title={t('documents.retryGenerate')}
        description={t('documents.retryConfirm')}
        variant="destructive"
        onConfirm={confirmRetry}
      />
    </div>
  );
}

function DocCard({
  doc, t, onAccept, onArchive, onRetry, onDownload,
}: {
  doc: DocumentSummary;
  t: (key: string, opts?: Record<string, unknown>) => string;
  onAccept: () => void;
  onArchive: () => void;
  onRetry: () => void;
  onDownload: () => void;
}) {
  return (
    <div className="flex items-center gap-3 p-3 border rounded-lg hover:bg-muted/30">
      <span className="text-xl">{categoryIcon(doc.category || 'general')}</span>
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <p className="text-sm font-medium truncate">{doc.display_name || doc.name}</p>
          <span className={`text-[10px] px-1.5 py-0.5 rounded-full ${statusColor(doc.status)}`}>
            {t(`documents.status.${doc.status}`)}
          </span>
        </div>
        <p className="text-xs text-muted-foreground">
          {formatFileSize(doc.file_size)} · {new Date(doc.created_at).toLocaleString()}
        </p>
      </div>
      <div className="flex items-center gap-1 shrink-0">
        {doc.status === 'draft' && (
          <Button size="sm" variant="outline" onClick={onAccept}>
            {t('documents.accept')}
          </Button>
        )}
        {(doc.status === 'draft' || doc.status === 'accepted') && (
          <Button size="sm" variant="outline" onClick={onArchive}>
            {t('documents.archive')}
          </Button>
        )}
        {doc.status !== 'superseded' && (
          <Button size="sm" variant="ghost" onClick={onRetry}>
            {t('documents.retryGenerate')}
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={onDownload}>
          {t('documents.download')}
        </Button>
      </div>
    </div>
  );
}

function groupDocsBySource(docs: DocumentSummary[], t: (key: string, opts?: Record<string, unknown>) => string): GroupedDocs[] {
  const groups = new Map<string, DocumentSummary[]>();
  for (const doc of docs) {
    const key = doc.source_session_id || doc.source_mission_id || 'unknown';
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key)!.push(doc);
  }
  // Sort each group by created_at desc
  const result: GroupedDocs[] = [];
  for (const [key, items] of groups) {
    items.sort((a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime());
    const first = items[0];
    const label = first.source_session_id
      ? t('documents.sourceSession', { id: first.source_session_id.slice(0, 8) })
      : first.source_mission_id
        ? t('documents.sourceMission', { id: first.source_mission_id.slice(0, 8) })
        : t('documents.sourceOther');
    result.push({ key, label, docs: items });
  }
  return result;
}
