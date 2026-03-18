import { useState, useEffect, useCallback, lazy, Suspense } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import { useIsMobile } from '../../hooks/useMediaQuery';
import { Input } from '../ui/input';
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui/select';
import { documentApi, folderApi, formatFileSize } from '../../api/documents';
import type { DocumentSummary, DocumentStatusType, FolderTreeNode } from '../../api/documents';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { Card } from '../ui/card';
import { StatusBadge, DOC_STATUS_MAP } from '../ui/status-badge';
import { EmptyState } from '../ui/empty-state';
import { LoadingState } from '../ui/loading-state';
import { formatDateTime } from '../../utils/format';

const DocumentPreview = lazy(() =>
  import('./DocumentPreview').then((module) => ({ default: module.DocumentPreview })),
);

function DocumentPreviewLoading() {
  return (
    <div className="flex h-full min-h-[320px] items-center justify-center text-sm text-muted-foreground">
      正在加载文档预览...
    </div>
  );
}

interface AiWorkbenchProps {
  teamId: string;
  canManage?: boolean;
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

export function AiWorkbench({ teamId, canManage = false }: AiWorkbenchProps) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const [documents, setDocuments] = useState<DocumentSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [statusFilter, setStatusFilter] = useState<string>('');
  const [groupBySource, setGroupBySource] = useState(true);
  const [page, setPage] = useState(1);
  const [totalPages, setTotalPages] = useState(0);
  const [retryTarget, setRetryTarget] = useState<string | null>(null);
  const [acceptingDocId, setAcceptingDocId] = useState<string | null>(null);
  const [acceptDialogOpen, setAcceptDialogOpen] = useState(false);
  const [acceptTarget, setAcceptTarget] = useState<DocumentSummary | null>(null);
  const [acceptName, setAcceptName] = useState('');
  const [acceptFolderPath, setAcceptFolderPath] = useState('/');
  const [folderTree, setFolderTree] = useState<FolderTreeNode[]>([]);
  const [foldersLoading, setFoldersLoading] = useState(false);
  const [previewDoc, setPreviewDoc] = useState<DocumentSummary | null>(null);

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

  const loadFolders = useCallback(async () => {
    setFoldersLoading(true);
    try {
      const tree = await folderApi.getFolderTree(teamId);
      setFolderTree(tree || []);
    } catch (error) {
      console.error('Failed to load folder tree:', error);
      setFolderTree([]);
    } finally {
      setFoldersLoading(false);
    }
  }, [teamId]);

  const openAcceptDialog = async (doc: DocumentSummary) => {
    setAcceptTarget(doc);
    setAcceptName(doc.display_name || doc.name);
    setAcceptFolderPath(doc.folder_path || '/');
    setAcceptDialogOpen(true);
    await loadFolders();
  };

  const closeAcceptDialog = () => {
    setAcceptDialogOpen(false);
    setAcceptTarget(null);
    setAcceptName('');
    setAcceptFolderPath('/');
  };

  const confirmAccept = async () => {
    if (!acceptTarget) return;

    setAcceptingDocId(acceptTarget.id);
    try {
      const updates: {
        display_name?: string;
        folder_path?: string;
      } = {};

      const nextDisplayName = acceptName.trim();
      const currentDisplayName = acceptTarget.display_name || acceptTarget.name;
      if (nextDisplayName && nextDisplayName !== currentDisplayName) {
        updates.display_name = nextDisplayName;
      }

      const nextFolderPath = acceptFolderPath || '/';
      const currentFolderPath = acceptTarget.folder_path || '/';
      if (nextFolderPath !== currentFolderPath) {
        updates.folder_path = nextFolderPath;
      }

      if (Object.keys(updates).length > 0) {
        await documentApi.updateDocument(teamId, acceptTarget.id, updates);
      }

      await documentApi.updateStatus(teamId, acceptTarget.id, 'accepted');
      closeAcceptDialog();
      await loadData();
    } catch (error) {
      console.error('Failed to accept document:', error);
    } finally {
      setAcceptingDocId(null);
    }
  };

  const handleRetry = (doc: DocumentSummary) => {
    setRetryTarget(doc.id);
  };

  const confirmRetry = async () => {
    if (!retryTarget) return;
    try {
      await documentApi.retryAnalysis(teamId, retryTarget);
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

  const hasPreview = previewDoc !== null;

  return (
    <div className={`flex h-full ${hasPreview && !isMobile ? 'gap-3' : 'flex-col'}`}>
      {/* Left: list area */}
      <div className={`flex flex-col ${hasPreview && !isMobile ? 'flex-1 min-w-0' : 'h-full'}`}>
      {/* Toolbar — always visible, sticky */}
      <div className="flex items-center gap-2 shrink-0 pb-3 sticky top-0 bg-background z-10">
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
        {!loading && (
          <span className="ml-auto text-caption tabular-nums text-muted-foreground/75">
            {documents.length} {t('documents.files').toLowerCase()}
          </span>
        )}
      </div>

      {/* Content area */}
      <div className="flex-1 overflow-auto space-y-4">
        {loading ? (
          <LoadingState variant="text" message={t('common.loading')} />
        ) : documents.length === 0 ? (
          <EmptyState message={t('documents.noAiDocuments')} className="py-12" />
        ) : (
          <>
            {grouped.map(group => (
              <div key={group.key} className="space-y-2">
                {groupBySource && (
                  <h3 className="text-xs font-medium text-muted-foreground uppercase tracking-wide flex items-center gap-2">
                    {group.label}
                    <span className="rounded px-1.5 py-0.5 text-micro tabular-nums text-muted-foreground/70 bg-muted/60">
                      {group.docs.length}
                    </span>
                  </h3>
                )}
                {group.docs.map(doc => (
                  <DocCard
                    key={doc.id}
                    doc={doc}
                    t={t}
                    isMobile={isMobile}
                    canManage={canManage}
                    accepting={acceptingDocId === doc.id}
                    selected={previewDoc?.id === doc.id}
                    onClick={() => setPreviewDoc(doc)}
                    onAccept={() => openAcceptDialog(doc)}
                    onArchive={() => handleUpdateStatus(doc.id, 'archived')}
                    onRetry={() => handleRetry(doc)}
                    onDownload={() => window.open(documentApi.getDownloadUrl(teamId, doc.id), '_blank')}
                  />
                ))}
              </div>
            ))}

            {/* Pagination */}
            {totalPages > 1 && (
              <div className="flex items-center justify-center gap-2 py-2">
                <Button size="sm" variant="outline" disabled={page <= 1} onClick={() => setPage(p => p - 1)}>
                  {t('pagination.previous')}
                </Button>
                <span className="text-xs">{page}/{totalPages}</span>
                <Button size="sm" variant="outline" disabled={page >= totalPages} onClick={() => setPage(p => p + 1)}>
                  {t('pagination.next')}
                </Button>
              </div>
            )}
          </>
        )}
      </div>
      </div>{/* end left list area */}

      {/* Right: Preview panel */}
      {hasPreview && previewDoc && (
        <Card className={isMobile ? 'fixed inset-0 z-50 overflow-hidden rounded-none border-0' : 'relative w-full lg:w-[min(45%,420px)] lg:min-w-[300px]'}>
          <Suspense fallback={<DocumentPreviewLoading />}>
            <DocumentPreview
              teamId={teamId}
              document={previewDoc}
              onClose={() => setPreviewDoc(null)}
            />
          </Suspense>
        </Card>
      )}

      <ConfirmDialog
        open={!!retryTarget}
        onOpenChange={(open) => { if (!open) setRetryTarget(null); }}
        title={t('documents.retryGenerate')}
        description={t('documents.retryConfirm')}
        variant="destructive"
        onConfirm={confirmRetry}
      />

      <Dialog open={acceptDialogOpen} onOpenChange={(open) => { if (!open) closeAcceptDialog(); }}>
        <DialogContent className="max-w-[92vw] sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t('documents.accept')}</DialogTitle>
          </DialogHeader>

          <div className="space-y-3">
            <div>
              <label className="text-xs text-muted-foreground block mb-1">
                {t('documents.displayName')}
              </label>
              <Input
                value={acceptName}
                onChange={(e) => setAcceptName(e.target.value)}
                placeholder={t('documents.displayName')}
              />
            </div>

            <div>
              <label className="text-xs text-muted-foreground block mb-1">
                {t('documents.folders')}
              </label>
              <Select value={acceptFolderPath} onValueChange={setAcceptFolderPath}>
                <SelectTrigger>
                  <SelectValue placeholder={t('documents.allFiles')} />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="/">{t('documents.allFiles')}</SelectItem>
                  {flattenFolders(folderTree).map((folder) => (
                    <SelectItem key={folder.path} value={folder.path}>
                      {folder.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {foldersLoading && (
                <p className="text-xs text-muted-foreground mt-1">
                  {t('common.loading')}
                </p>
              )}
            </div>
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={closeAcceptDialog}>
              {t('common.cancel')}
            </Button>
            <Button
              onClick={confirmAccept}
              disabled={!acceptTarget || (acceptingDocId !== null && acceptingDocId !== acceptTarget.id)}
            >
              {acceptTarget && acceptingDocId === acceptTarget.id
                ? t('common.loading')
                : t('documents.accept')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function DocCard({
  doc, t, isMobile, canManage, accepting, selected, onClick, onAccept, onArchive, onRetry, onDownload,
}: {
  doc: DocumentSummary;
  t: (key: string, opts?: Record<string, unknown>) => string;
  isMobile: boolean;
  canManage: boolean;
  accepting: boolean;
  selected?: boolean;
  onClick?: () => void;
  onAccept: () => void;
  onArchive: () => void;
  onRetry: () => void;
  onDownload: () => void;
}) {
  return (
    <div
      className={`p-3 border rounded-lg hover:bg-muted/30 cursor-pointer transition-colors ${selected ? 'border-primary/50 bg-muted/40' : ''} ${isMobile ? 'space-y-2' : 'flex items-center gap-3'}`}
      onClick={onClick}
    >
      <div className={`flex items-start gap-2 ${isMobile ? '' : 'flex-1 min-w-0'}`}>
        <span className="text-xl shrink-0">{categoryIcon(doc.category || 'general')}</span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5 flex-wrap">
            <p className="text-sm font-medium truncate max-w-full">{doc.display_name || doc.name}</p>
            <StatusBadge status={DOC_STATUS_MAP[doc.status]} className="shrink-0">
              {t(`documents.status.${doc.status}`)}
            </StatusBadge>
          </div>
          <p className="text-xs text-muted-foreground">
            {formatFileSize(doc.file_size)} · {formatDateTime(doc.created_at)}
          </p>
        </div>
      </div>
      <div className={`flex items-center gap-1 ${isMobile ? 'pl-7' : 'shrink-0'}`} onClick={(e) => e.stopPropagation()}>
        {canManage && doc.status === 'draft' && (
          <Button size="sm" variant="outline" onClick={onAccept} disabled={accepting}>
            {accepting
              ? t('common.loading')
              : t('documents.accept')}
          </Button>
        )}
        {canManage && (doc.status === 'draft' || doc.status === 'accepted') && (
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

function flattenFolders(nodes: FolderTreeNode[], level = 0): Array<{ path: string; label: string }> {
  const items: Array<{ path: string; label: string }> = [];
  for (const node of nodes) {
    items.push({
      path: node.fullPath,
      label: `${'  '.repeat(level)}${node.name}`,
    });
    if (node.children?.length) {
      items.push(...flattenFolders(node.children, level + 1));
    }
  }
  return items;
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
    let label: string;
    if (first.source_session_id) {
      label = t('documents.sourceSession', { id: first.source_session_id.slice(0, 8) });
    } else if (first.source_mission_id) {
      label = t('documents.sourceMission', { id: first.source_mission_id.slice(0, 8) });
    } else {
      label = t('documents.sourceOther');
    }
    result.push({ key, label, docs: items });
  }
  return result;
}
