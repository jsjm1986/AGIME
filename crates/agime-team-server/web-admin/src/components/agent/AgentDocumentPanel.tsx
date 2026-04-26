import { useEffect, useMemo, useState, lazy, Suspense } from 'react';
import { useTranslation } from 'react-i18next';
import { ExternalLink, FileText, GitBranch, History, MessageSquareText } from 'lucide-react';
import { Badge } from '../ui/badge';
import { Button } from '../ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { StatusBadge, DOC_STATUS_MAP } from '../ui/status-badge';
import { VersionTimeline } from '../documents/VersionTimeline';
import { documentApi, formatFileSize, type DocumentSummary } from '../../api/documents';
import type { PortalDocumentAccessMode } from '../../api/avatarPortal';
import { formatDateTime } from '../../utils/format';

const DocumentPreview = lazy(() =>
  import('../documents/DocumentPreview').then((module) => ({ default: module.DocumentPreview })),
);

function DocumentPreviewLoading() {
  return (
    <div className="flex h-full items-center justify-center rounded-xl border border-dashed border-border/60 bg-muted/10 text-sm text-muted-foreground">
      Loading document preview...
    </div>
  );
}

function formatAccessMode(
  mode: PortalDocumentAccessMode | undefined,
  t: (key: string, fallback: string) => string,
): string {
  switch (mode) {
    case 'read_only':
      return t('ecosystem.documentAccessModeReadOnly', 'Read only');
    case 'co_edit_draft':
      return t('ecosystem.documentAccessModeCoEditDraft', 'Collaborative draft');
    case 'controlled_write':
      return t('ecosystem.documentAccessModeControlledWrite', 'Controlled write');
    default:
      return t('common.notSet', 'Not set');
  }
}

function formatFamilyHint(
  mode: PortalDocumentAccessMode | undefined,
  t: (key: string, fallback: string) => string,
): string {
  switch (mode) {
    case 'read_only':
      return t(
        'agent.manage.agentDocumentsFamilyHintReadOnly',
        'This mode focuses on viewing and Q&A, showing only the original document and its related AI versions.'
      );
    case 'co_edit_draft':
      return t(
        'agent.manage.agentDocumentsFamilyHintDraft',
        'Conversational edits first land in the AI workspace draft so you can continue iterating.'
      );
    case 'controlled_write':
      return t(
        'agent.manage.agentDocumentsFamilyHintControlledWrite',
        'Conversational edits can keep using the AI version or write back directly into the target document.'
      );
    default:
      return t(
        'agent.manage.agentDocumentsFamilyHintDefault',
        'Related AI versions are grouped by original document here so you can confirm the current working context.'
      );
  }
}

function getDocDisplayName(doc: DocumentSummary): string {
  return doc.display_name || doc.name;
}

function getDocTimestamp(doc: DocumentSummary): string {
  return doc.updated_at || doc.created_at;
}

function sortRelatedDocs(a: DocumentSummary, b: DocumentSummary): number {
  if (a.status === 'draft' && b.status !== 'draft') {
    return -1;
  }
  if (a.status !== 'draft' && b.status === 'draft') {
    return 1;
  }
  return getDocTimestamp(b).localeCompare(getDocTimestamp(a));
}

function totalVersionCount(value: number | undefined): string {
  if (!value || value < 1) {
    return 'v1';
  }
  return `v${value}`;
}

interface AgentDocumentPanelProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  teamId: string;
  portalName?: string | null;
  serviceAgentName?: string | null;
  documentAccessMode?: PortalDocumentAccessMode;
  documentIds: string[];
  documents: DocumentSummary[];
  canManage: boolean;
  onStartChat: (targetDoc: DocumentSummary, sourceDoc: DocumentSummary | null) => void;
  onOpenDocumentsChannel: () => void;
}

export function AgentDocumentPanel({
  open,
  onOpenChange,
  teamId,
  portalName,
  serviceAgentName,
  documentAccessMode,
  documentIds,
  documents,
  canManage,
  onStartChat,
  onOpenDocumentsChannel,
}: AgentDocumentPanelProps) {
  const { t } = useTranslation();
  const [selectedSourceDocId, setSelectedSourceDocId] = useState<string | null>(null);
  const [selectedDocId, setSelectedDocId] = useState<string | null>(null);
  const [relatedAiDocuments, setRelatedAiDocuments] = useState<DocumentSummary[]>([]);
  const [loadingRelatedAi, setLoadingRelatedAi] = useState(false);
  const [versionTotalsById, setVersionTotalsById] = useState<Record<string, number>>({});
  const [versionTarget, setVersionTarget] = useState<DocumentSummary | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    if (documents.length === 0) {
      setSelectedSourceDocId(null);
      setSelectedDocId(null);
      return;
    }
    if (!selectedSourceDocId || !documents.some(doc => doc.id === selectedSourceDocId)) {
      setSelectedSourceDocId(documents[0].id);
      setSelectedDocId(documents[0].id);
    }
  }, [documents, open, selectedSourceDocId]);

  useEffect(() => {
    if (!open) {
      setRelatedAiDocuments([]);
      return;
    }
    if (documentIds.length === 0) {
      setRelatedAiDocuments([]);
      return;
    }
    let cancelled = false;
    setLoadingRelatedAi(true);
    documentApi.getRelatedAiDocuments(teamId, documentIds, 1, 500)
      .then(result => {
        if (!cancelled) {
          setRelatedAiDocuments(result.items || []);
        }
      })
      .catch(error => {
        if (!cancelled) {
          console.error('Failed to load related AI documents:', error);
          setRelatedAiDocuments([]);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoadingRelatedAi(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [documentIds, open, teamId]);

  const selectedSourceDoc = useMemo(
    () => documents.find(doc => doc.id === selectedSourceDocId) || null,
    [documents, selectedSourceDocId],
  );

  const familyDocs = useMemo(() => {
    if (!selectedSourceDoc) {
      return [];
    }
    return relatedAiDocuments
      .filter(doc => doc.source_document_ids.includes(selectedSourceDoc.id))
      .sort(sortRelatedDocs);
  }, [relatedAiDocuments, selectedSourceDoc]);

  const recommendedDraftId = useMemo(
    () => familyDocs.find(doc => doc.status === 'draft')?.id || familyDocs[0]?.id || null,
    [familyDocs],
  );

  const previewCandidates = useMemo(
    () => (selectedSourceDoc ? [selectedSourceDoc, ...familyDocs] : []),
    [familyDocs, selectedSourceDoc],
  );

  useEffect(() => {
    if (!open) {
      return;
    }
    if (!selectedSourceDoc) {
      setSelectedDocId(null);
      return;
    }
    const previewIds = new Set(previewCandidates.map(doc => doc.id));
    if (!selectedDocId || !previewIds.has(selectedDocId)) {
      setSelectedDocId(selectedSourceDoc.id);
    }
  }, [open, previewCandidates, selectedDocId, selectedSourceDoc]);

  const selectedDoc = useMemo(
    () => previewCandidates.find(doc => doc.id === selectedDocId) || selectedSourceDoc || null,
    [previewCandidates, selectedDocId, selectedSourceDoc],
  );

  const familyVersionKey = useMemo(
    () => [selectedSourceDoc?.id || '', ...familyDocs.map(doc => doc.id)].join('|'),
    [familyDocs, selectedSourceDoc?.id],
  );

  useEffect(() => {
    if (!open || !selectedSourceDoc) {
      return;
    }
    const targets = [selectedSourceDoc, ...familyDocs].filter(
      doc => versionTotalsById[doc.id] === undefined,
    );
    if (targets.length === 0) {
      return;
    }
    let cancelled = false;
    Promise.all(
      targets.map(async doc => {
        try {
          const response = await documentApi.listVersions(teamId, doc.id, 1, 1);
          return [doc.id, Math.max(1, (response.total || 0) + 1)] as const;
        } catch (error) {
          console.error(`Failed to load versions for ${doc.id}:`, error);
          return [doc.id, 1] as const;
        }
      }),
    ).then(entries => {
      if (cancelled) {
        return;
      }
      setVersionTotalsById(prev => {
        const next = { ...prev };
        for (const [docId, total] of entries) {
          next[docId] = total;
        }
        return next;
      });
    });
    return () => {
      cancelled = true;
    };
  }, [familyVersionKey, familyDocs, open, selectedSourceDoc, teamId, versionTotalsById]);

  const missingCount = Math.max(0, documentIds.length - documents.length);
  const selectedDocIsDerived = !!selectedSourceDoc && !!selectedDoc && selectedDoc.id !== selectedSourceDoc.id;

  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent className="w-[min(1360px,97vw)] max-w-[97vw] overflow-hidden p-0 sm:h-[88vh]">
          <div className="flex h-full min-h-0 flex-col">
            <DialogHeader className="border-b border-border/70 px-6 py-4">
              <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
              <div className="min-w-0 space-y-2">
                  <DialogTitle className="truncate text-left">
                    {t('agent.manage.agentDocumentsTitle', '服务 Agent 文档')}
                  </DialogTitle>
                  <DialogDescription className="text-left">
                    {t(
                      'agent.manage.agentDocumentsFamilyDescription',
                      'Review the service agent document family grouped by original bound document: source doc, related AI versions, and version timeline.'
                    )}
                  </DialogDescription>
                  <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                    {portalName ? (
                      <Badge variant="secondary" className="text-[11px]">
                        {portalName}
                      </Badge>
                    ) : null}
                    {serviceAgentName ? (
                      <Badge variant="outline" className="text-[11px]">
                        {serviceAgentName}
                      </Badge>
                    ) : null}
                    <Badge variant="outline" className="text-[11px]">
                      {t('agent.manage.avatarBoundDocumentsLabel', '绑定文档')} {documentIds.length}
                    </Badge>
                    <Badge variant="outline" className="text-[11px]">
                      {formatAccessMode(documentAccessMode, (key, fallback) => t(key, fallback))}
                    </Badge>
                    {missingCount > 0 ? (
                      <span>
                        {t('agent.manage.agentDocumentsMissingHint', '有 {{count}} 份绑定文档当前不可用', {
                          count: missingCount,
                        })}
                      </span>
                    ) : null}
                  </div>
              </div>

                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    type="button"
                    size="sm"
                    onClick={() => {
                      if (!selectedDoc) {
                        return;
                      }
                      onStartChat(selectedDoc, selectedSourceDoc);
                      onOpenChange(false);
                    }}
                    disabled={!selectedDoc}
                  >
                    <MessageSquareText className="mr-1.5 h-3.5 w-3.5" />
                    {t('agent.manage.agentDocumentsStartChat', '基于当前文档对话')}
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      onOpenDocumentsChannel();
                      onOpenChange(false);
                    }}
                  >
                    <ExternalLink className="mr-1.5 h-3.5 w-3.5" />
                    {t('agent.manage.openDocumentsChannel', '打开文档频道')}
                  </Button>
                </div>
              </div>
            </DialogHeader>

            <div className="flex min-h-0 flex-1 flex-col xl:flex-row">
              <div className="flex max-h-[220px] min-h-[200px] flex-col border-b border-border/70 bg-muted/10 xl:max-h-none xl:min-h-0 xl:w-[min(24vw,300px)] xl:min-w-[260px] xl:border-b-0 xl:border-r">
                <div className="border-b border-border/60 px-4 py-3">
                  <div className="text-sm font-medium text-foreground">
                    {t('agent.manage.agentDocumentsSourceTitle', '绑定原始文档')}
                  </div>
                  <div className="mt-1 text-xs text-muted-foreground">
                    {documents.length > 0
                      ? t('agent.manage.agentDocumentsSourceHint', '先选择一份原文，再查看它派生出来的 AI 版本。')
                      : t('agent.manage.noBoundDocuments', '未绑定文档')}
                  </div>
                </div>

                <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
                  {documents.length === 0 ? (
                    <div className="flex h-full flex-col items-center justify-center gap-2 px-4 text-center text-sm text-muted-foreground">
                      <FileText className="h-8 w-8 text-muted-foreground/70" />
                      <p>{t('agent.manage.noBoundDocuments', '未绑定文档')}</p>
                    </div>
                  ) : (
                    <div className="space-y-2">
                      {documents.map(doc => {
                        const selected = doc.id === selectedSourceDoc?.id;
                        const familyCount = relatedAiDocuments.filter(item => item.source_document_ids.includes(doc.id)).length;
                        return (
                          <button
                            key={doc.id}
                            type="button"
                            onClick={() => {
                              setSelectedSourceDocId(doc.id);
                              setSelectedDocId(doc.id);
                            }}
                            className={[
                              'w-full rounded-xl border px-3 py-3 text-left transition-colors',
                              selected
                                ? 'border-primary/40 bg-primary/10'
                                : 'border-border/60 bg-background hover:border-primary/30 hover:bg-background/80',
                            ].join(' ')}
                          >
                            <div className="flex items-start gap-3">
                              <div className="rounded-lg bg-muted px-2 py-1 text-xs text-muted-foreground">
                                <FileText className="h-3.5 w-3.5" />
                              </div>
                              <div className="min-w-0 flex-1">
                                <div className="truncate text-sm font-medium text-foreground">
                                  {getDocDisplayName(doc)}
                                </div>
                                <div className="mt-1 flex flex-wrap gap-2 text-xs text-muted-foreground">
                                  <span>{formatFileSize(doc.file_size)}</span>
                                  <span>{doc.mime_type || '-'}</span>
                                  <span>{totalVersionCount(versionTotalsById[doc.id])}</span>
                                </div>
                                <div className="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
                                  <Badge variant="outline" className="text-[10px]">
                                    {t('agent.manage.agentDocumentsOriginalBadge', '原始文档')}
                                  </Badge>
                                  <span>
                                    {t('agent.manage.agentDocumentsFamilyCount', '相关 AI 版本 {{count}} 个', {
                                      count: familyCount,
                                    })}
                                  </span>
                                </div>
                                <div className="mt-1 truncate text-xs text-muted-foreground">
                                  {formatDateTime(getDocTimestamp(doc))}
                                </div>
                              </div>
                            </div>
                          </button>
                        );
                      })}
                    </div>
                  )}
                </div>
              </div>

              <div className="flex min-h-[280px] flex-col border-b border-border/70 bg-background xl:min-h-0 xl:w-[min(28vw,360px)] xl:min-w-[300px] xl:border-b-0 xl:border-r">
                <div className="border-b border-border/60 px-4 py-3">
                  <div className="flex items-center gap-2">
                    <GitBranch className="h-4 w-4 text-primary" />
                    <div className="text-sm font-medium text-foreground">
                      {t('agent.manage.agentDocumentsFamilyTitle', '文档家族')}
                    </div>
                  </div>
                  <div className="mt-1 text-xs leading-5 text-muted-foreground">
                    {formatFamilyHint(documentAccessMode, (key, fallback) => t(key, fallback))}
                  </div>
                </div>

                <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3">
                  {!selectedSourceDoc ? (
                    <div className="flex h-full items-center justify-center px-4 text-center text-sm text-muted-foreground">
                      {t('agent.manage.agentDocumentsFamilyEmpty', '先从左侧选择一份原始文档。')}
                    </div>
                  ) : (
                    <div className="space-y-4">
                      <div className="rounded-xl border border-border/60 bg-muted/10 p-3">
                        <div className="flex flex-wrap items-center gap-2">
                          <Badge variant="secondary" className="text-[11px]">
                            {t('agent.manage.agentDocumentsOriginalBadge', '原始文档')}
                          </Badge>
                          <Badge variant="outline" className="text-[11px]">
                            {totalVersionCount(versionTotalsById[selectedSourceDoc.id])}
                          </Badge>
                          <span className="text-xs text-muted-foreground">
                            {formatDateTime(getDocTimestamp(selectedSourceDoc))}
                          </span>
                        </div>
                        <div className="mt-2 text-sm font-medium text-foreground">
                          {getDocDisplayName(selectedSourceDoc)}
                        </div>
                        <div className="mt-1 flex flex-wrap gap-2 text-xs text-muted-foreground">
                          <span>{selectedSourceDoc.mime_type || '-'}</span>
                          <span>{formatFileSize(selectedSourceDoc.file_size)}</span>
                        </div>
                      </div>

                      <div className="space-y-2">
                        <div className="flex items-center justify-between">
                          <div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                            {t('agent.manage.agentDocumentsDerivedTitle', '相关 AI 文档')}
                          </div>
                          <Badge variant="outline" className="text-[11px]">
                            {familyDocs.length}
                          </Badge>
                        </div>

                        <button
                          type="button"
                          onClick={() => setSelectedDocId(selectedSourceDoc.id)}
                          className={[
                            'w-full rounded-xl border px-3 py-3 text-left transition-colors',
                            selectedDoc?.id === selectedSourceDoc.id
                              ? 'border-primary/40 bg-primary/10'
                              : 'border-border/60 bg-background hover:border-primary/30 hover:bg-background/80',
                          ].join(' ')}
                        >
                          <div className="flex items-start gap-3">
                            <div className="rounded-lg bg-muted px-2 py-1 text-xs text-muted-foreground">
                              <FileText className="h-3.5 w-3.5" />
                            </div>
                            <div className="min-w-0 flex-1">
                              <div className="truncate text-sm font-medium text-foreground">
                                {getDocDisplayName(selectedSourceDoc)}
                              </div>
                              <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                                <span>{t('agent.manage.agentDocumentsOriginalBadge', '原始文档')}</span>
                                <span>{totalVersionCount(versionTotalsById[selectedSourceDoc.id])}</span>
                              </div>
                            </div>
                          </div>
                        </button>

                        {loadingRelatedAi ? (
                          <div className="rounded-xl border border-dashed border-border/70 bg-muted/10 px-4 py-5 text-sm text-muted-foreground">
                            {t('common.loading', '加载中')}
                          </div>
                        ) : familyDocs.length === 0 ? (
                          <div className="rounded-xl border border-dashed border-border/70 bg-muted/10 px-4 py-5 text-sm text-muted-foreground">
                            {t(
                              'agent.manage.agentDocumentsNoDerived',
                            'This original document does not have any related AI document versions yet.'
                            )}
                          </div>
                        ) : (
                          <div className="space-y-2">
                            {familyDocs.map(doc => {
                              const selected = doc.id === selectedDoc?.id;
                              const isRecommended = recommendedDraftId === doc.id;
                              return (
                                <button
                                  key={doc.id}
                                  type="button"
                                  onClick={() => setSelectedDocId(doc.id)}
                                  className={[
                                    'w-full rounded-xl border px-3 py-3 text-left transition-colors',
                                    selected
                                      ? 'border-primary/40 bg-primary/10'
                                      : 'border-border/60 bg-background hover:border-primary/30 hover:bg-background/80',
                                  ].join(' ')}
                                >
                                  <div className="flex items-start gap-3">
                                    <div className="rounded-lg bg-primary/10 px-2 py-1 text-primary">
                                      <GitBranch className="h-3.5 w-3.5" />
                                    </div>
                                    <div className="min-w-0 flex-1">
                                      <div className="flex flex-wrap items-center gap-1.5">
                                        <div className="truncate text-sm font-medium text-foreground">
                                          {getDocDisplayName(doc)}
                                        </div>
                                        <StatusBadge status={DOC_STATUS_MAP[doc.status] || 'neutral'}>
                                          {t(`documents.status.${doc.status}`)}
                                        </StatusBadge>
                                        <Badge variant="outline" className="text-[10px]">
                                          {totalVersionCount(versionTotalsById[doc.id])}
                                        </Badge>
                                        {isRecommended ? (
                                          <Badge variant="secondary" className="text-[10px]">
                                            {t('agent.manage.agentDocumentsRecommendedDraft', '推荐继续')}
                                          </Badge>
                                        ) : null}
                                      </div>
                                      <div className="mt-1 flex flex-wrap gap-2 text-xs text-muted-foreground">
                                        <span>{doc.mime_type || '-'}</span>
                                        <span>{formatFileSize(doc.file_size)}</span>
                                        <span>{formatDateTime(getDocTimestamp(doc))}</span>
                                      </div>
                                      {doc.lineage_description ? (
                                        <p className="mt-1 line-clamp-2 text-xs leading-5 text-muted-foreground">
                                          {doc.lineage_description}
                                        </p>
                                      ) : null}
                                    </div>
                                  </div>
                                </button>
                              );
                            })}
                          </div>
                        )}
                      </div>

                      <div className="rounded-xl border border-border/60 bg-muted/10 px-3 py-3">
                        <div className="flex items-center gap-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                          <History className="h-3.5 w-3.5" />
                          {t('agent.manage.agentDocumentsVersionHintTitle', '版本提示')}
                        </div>
                        <p className="mt-2 text-xs leading-5 text-muted-foreground">
                          {t(
                            'agent.manage.agentDocumentsVersionHint',
                            'Original and AI documents each keep their own version timeline. By default the current working draft is updated and historical changes are stored in version history.'
                          )}
                        </p>
                      </div>
                    </div>
                  )}
                </div>
              </div>

              <div className="min-h-[320px] flex-1 overflow-hidden bg-background">
                {selectedDoc ? (
                  <Suspense fallback={<DocumentPreviewLoading />}>
                    <DocumentPreview
                      teamId={teamId}
                      document={selectedDoc}
                      onClose={() => onOpenChange(false)}
                      onVersions={() => setVersionTarget(selectedDoc)}
                    />
                  </Suspense>
                ) : (
                  <div className="flex h-full items-center justify-center px-6 text-center text-sm text-muted-foreground">
                    {documents.length > 0
                      ? t('agent.manage.agentDocumentsPreviewEmpty', '从左侧选择一份文档开始预览。')
                      : t('agent.manage.agentDocumentsPreviewNone', '当前没有可预览的绑定文档。')}
                  </div>
                )}
                {selectedDoc && (
                  <div className="border-t border-border/60 bg-muted/10 px-4 py-3 text-xs text-muted-foreground">
                    {selectedDocIsDerived
                      ? t(
                          'agent.manage.agentDocumentsCurrentPreviewDerived',
                          'You are previewing an AI-version document, which can be used for further conversational iteration.'
                        )
                      : t(
                          'agent.manage.agentDocumentsCurrentPreviewOriginal',
                          'You are previewing the original bound document, which serves as the baseline for future conversational edits.'
                        )}
                  </div>
                )}
              </div>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={!!versionTarget} onOpenChange={(nextOpen) => {
        if (!nextOpen) {
          setVersionTarget(null);
        }
      }}>
        <DialogContent className="w-[min(980px,94vw)] max-w-[94vw] overflow-hidden p-0 sm:h-[80vh]">
          <div className="flex h-full min-h-0 flex-col">
            <DialogHeader className="border-b border-border/70 px-6 py-4">
              <DialogTitle className="text-left">
                {t('agent.manage.agentDocumentsVersionTimelineTitle', '版本时间线')}
              </DialogTitle>
              <DialogDescription className="text-left">
                {versionTarget
                  ? t(
                      'agent.manage.agentDocumentsVersionTimelineDescription',
                      'Review historical snapshots for {{name}} and confirm the changes captured from each conversational edit.',
                      { name: getDocDisplayName(versionTarget) },
                    )
                  : ''}
              </DialogDescription>
            </DialogHeader>
            <div className="min-h-0 flex-1 overflow-hidden">
              {versionTarget ? (
                <VersionTimeline
                  teamId={teamId}
                  docId={versionTarget.id}
                  canManage={canManage}
                />
              ) : null}
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}

