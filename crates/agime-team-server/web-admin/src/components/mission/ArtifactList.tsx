import { useState, useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { missionApi, MissionArtifact } from '../../api/mission';
import { documentApi, folderApi, type DocumentSummary, type FolderTreeNode } from '../../api/documents';
import { ArtifactPreview } from './ArtifactPreview';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '../ui/dialog';
import { Input } from '../ui/input';
import { Button } from '../ui/button';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui/select';

interface ArtifactListProps {
  missionId: string;
  teamId: string;
}

export function ArtifactList({ missionId, teamId }: ArtifactListProps) {
  const { t } = useTranslation();
  const [artifacts, setArtifacts] = useState<MissionArtifact[]>([]);
  const [missionDocs, setMissionDocs] = useState<DocumentSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [selectedArtifactId, setSelectedArtifactId] = useState<string | null>(null);
  const [archivingArtifactId, setArchivingArtifactId] = useState<string | null>(null);
  const [acceptingDocId, setAcceptingDocId] = useState<string | null>(null);
  const [archiveDialogOpen, setArchiveDialogOpen] = useState(false);
  const [archiveTarget, setArchiveTarget] = useState<MissionArtifact | null>(null);
  const [archiveName, setArchiveName] = useState('');
  const [archiveFolderPath, setArchiveFolderPath] = useState('/');
  const [acceptDialogOpen, setAcceptDialogOpen] = useState(false);
  const [acceptTargetDoc, setAcceptTargetDoc] = useState<DocumentSummary | null>(null);
  const [acceptName, setAcceptName] = useState('');
  const [acceptFolderPath, setAcceptFolderPath] = useState('/');
  const [folderTree, setFolderTree] = useState<FolderTreeNode[]>([]);
  const [foldersLoading, setFoldersLoading] = useState(false);

  const artifactRole = (artifact: MissionArtifact) =>
    artifact.delivery_role || 'core_deliverable';

  const artifactRoleLabel = (artifact: MissionArtifact) =>
    artifactRole(artifact) === 'core_deliverable'
      ? t('mission.coreDeliverables', 'Core deliverables')
      : t('mission.supportingArtifacts', 'Supporting materials');

  const artifactRoleHint = (artifact: MissionArtifact) => {
    if (artifactRole(artifact) === 'core_deliverable') {
      return artifact.is_required_output
        ? t('mission.requiredOutputHint', 'Requested output in the final deliverable package.')
        : t('mission.reusableOutputHint', 'Reusable output created during execution.');
    }
    return t(
      'mission.supportingArtifactHint',
      'Process material, notes, evidence, or recovery output kept for traceability.',
    );
  };

  const formatSize = (size: number) => {
    if (size < 1024) return `${size} B`;
    if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
    return `${(size / (1024 * 1024)).toFixed(1)} MB`;
  };

  const copyText = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // no-op
    }
  };

  const downloadContent = (name: string, content: string) => {
    const blob = new Blob([content], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = name || 'artifact.txt';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  const loadData = async () => {
    const [items, docs] = await Promise.all([
      missionApi.listArtifacts(missionId),
      documentApi.listAiWorkbench(teamId, undefined, missionId, 1, 100),
    ]);
    setArtifacts(items || []);
    setMissionDocs(docs.items || []);
  };

  const archiveArtifact = async (artifact: MissionArtifact, payload?: { name?: string; folder_path?: string }) => {
    setArchivingArtifactId(artifact.artifact_id);
    try {
      const res = await missionApi.archiveArtifactToDocument(artifact.artifact_id, payload || {});
      setArtifacts(prev => prev.map(item => (
        item.artifact_id === artifact.artifact_id ? res.artifact : item
      )));
      await loadData();
    } catch (error) {
      console.error('Failed to archive artifact to document:', error);
    } finally {
      setArchivingArtifactId(null);
    }
  };

  const loadFolders = async () => {
    setFoldersLoading(true);
    try {
      const tree = await folderApi.getFolderTree(teamId);
      setFolderTree(tree || []);
    } catch (error) {
      console.error('Failed to load folder tree for artifact archive:', error);
      setFolderTree([]);
    } finally {
      setFoldersLoading(false);
    }
  };

  const openArchiveDialog = async (artifact: MissionArtifact) => {
    setArchiveTarget(artifact);
    setArchiveName(artifact.name || '');
    setArchiveFolderPath('/');
    setArchiveDialogOpen(true);
    await loadFolders();
  };

  const closeArchiveDialog = () => {
    setArchiveDialogOpen(false);
    setArchiveTarget(null);
    setArchiveName('');
    setArchiveFolderPath('/');
  };

  const flattenFolders = (nodes: FolderTreeNode[], level = 0): Array<{ path: string; label: string }> => {
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
  };

  const confirmArchiveDialog = async () => {
    if (!archiveTarget) return;
    await archiveArtifact(archiveTarget, {
      name: archiveName.trim() || undefined,
      folder_path: archiveFolderPath && archiveFolderPath !== '/' ? archiveFolderPath : undefined,
    });
    closeArchiveDialog();
  };

  const openAcceptDialog = async (doc: DocumentSummary) => {
    setAcceptTargetDoc(doc);
    setAcceptName(doc.display_name || doc.name);
    setAcceptFolderPath(doc.folder_path || '/');
    setAcceptDialogOpen(true);
    await loadFolders();
  };

  const closeAcceptDialog = () => {
    setAcceptDialogOpen(false);
    setAcceptTargetDoc(null);
    setAcceptName('');
    setAcceptFolderPath('/');
  };

  const openAcceptDialogById = async (docId: string, fallbackName?: string) => {
    if (!docId) return;
    const existing = missionDocs.find((doc) => doc.id === docId);
    if (existing) {
      await openAcceptDialog(existing);
      return;
    }

    try {
      const refreshed = await documentApi.listAiWorkbench(teamId, undefined, missionId, 1, 200);
      const refreshedItems = refreshed.items || [];
      setMissionDocs(refreshedItems);
      const target = refreshedItems.find((doc) => doc.id === docId);
      if (target) {
        await openAcceptDialog(target);
        return;
      }
    } catch (error) {
      console.error('Failed to refresh mission documents before accept:', error);
    }

    // Fallback: keep flow usable even when list query misses the doc.
    const fallbackDoc: DocumentSummary = {
      id: docId,
      name: fallbackName || 'document',
      display_name: fallbackName || null,
      description: null,
      mime_type: 'application/octet-stream',
      file_size: 0,
      folder_path: '/',
      tags: [],
      uploaded_by: '',
      origin: 'agent',
      status: 'draft',
      category: 'other',
      source_document_ids: [],
      source_session_id: null,
      source_mission_id: missionId,
      created_by_agent_id: null,
      supersedes_id: null,
      lineage_description: null,
      created_at: new Date().toISOString(),
    };
    await openAcceptDialog(fallbackDoc);
  };

  const confirmAcceptDialog = async () => {
    if (!acceptTargetDoc) return;

    setAcceptingDocId(acceptTargetDoc.id);
    try {
      const updates: { display_name?: string; folder_path?: string } = {};
      const nextDisplayName = acceptName.trim();
      const currentDisplayName = acceptTargetDoc.display_name || acceptTargetDoc.name;
      if (nextDisplayName && nextDisplayName !== currentDisplayName) {
        updates.display_name = nextDisplayName;
      }

      const nextFolder = acceptFolderPath || '/';
      const currentFolder = acceptTargetDoc.folder_path || '/';
      if (nextFolder !== currentFolder) {
        updates.folder_path = nextFolder;
      }

      if (Object.keys(updates).length > 0) {
        await documentApi.updateDocument(teamId, acceptTargetDoc.id, updates);
      }

      await documentApi.updateStatus(teamId, acceptTargetDoc.id, 'accepted');
      closeAcceptDialog();
      await loadData();
    } catch (error) {
      console.error('Failed to accept mission document:', error);
    } finally {
      setAcceptingDocId(null);
    }
  };

  const docStatusLabel = (status?: string) => {
    switch (status?.toLowerCase()) {
      case 'active': return t('documents.status.active');
      case 'accepted': return t('documents.status.accepted');
      case 'archived': return t('documents.status.archived');
      case 'superseded': return t('documents.status.superseded');
      default: return t('documents.status.draft');
    }
  };

  const isDraftStatus = (status?: string) => (status || '').toLowerCase() === 'draft';

  const openDocumentsSection = () => {
    const base = window.location.pathname.startsWith('/admin') ? '/admin' : '';
    window.open(`${base}/teams/${teamId}?section=documents`, '_blank');
  };

  const coreArtifacts = useMemo(
    () => artifacts.filter((artifact) => artifactRole(artifact) === 'core_deliverable'),
    [artifacts],
  );
  const supportingArtifacts = useMemo(
    () => artifacts.filter((artifact) => artifactRole(artifact) === 'supporting_artifact'),
    [artifacts],
  );
  const orderedArtifacts = useMemo(
    () => [...coreArtifacts, ...supportingArtifacts],
    [coreArtifacts, supportingArtifacts],
  );

  useEffect(() => {
    let cancelled = false;
    loadData().then(() => {
      if (!cancelled) setLoading(false);
    }).catch(() => {
      if (!cancelled) setLoading(false);
    });
    return () => { cancelled = true; };
  }, [missionId, teamId]);

  useEffect(() => {
    if (orderedArtifacts.length === 0) {
      if (selectedArtifactId !== null) {
        setSelectedArtifactId(null);
      }
      return;
    }
    if (!selectedArtifactId || !orderedArtifacts.some((artifact) => artifact.artifact_id === selectedArtifactId)) {
      setSelectedArtifactId(orderedArtifacts[0].artifact_id);
    }
  }, [orderedArtifacts, selectedArtifactId]);

  if (loading) {
    return <p className="text-sm text-muted-foreground p-3">{t('common.loading', 'Loading...')}</p>;
  }

  if (artifacts.length === 0) {
    return (
      <p className="text-sm text-muted-foreground p-3 text-center">
        {t('mission.noArtifacts', 'No artifacts')}
      </p>
    );
  }

  const selectedArtifact =
    orderedArtifacts.find((artifact) => artifact.artifact_id === selectedArtifactId) ?? orderedArtifacts[0];

  const renderArtifactSection = (
    sectionArtifacts: MissionArtifact[],
    tone: 'core' | 'supporting',
  ) => {
    if (sectionArtifacts.length === 0) return null;
    const sectionTitle =
      tone === 'core'
        ? t('mission.coreDeliverables', 'Core deliverables')
        : t('mission.supportingArtifacts', 'Supporting materials');
    const sectionHint =
      tone === 'core'
        ? t(
            'mission.coreDeliverablesHint',
            'These are the assets most likely to matter to the user after the task ends.',
          )
        : t(
            'mission.supportingArtifactsHint',
            'These files preserve process context and can help with audit, debugging, or downstream reuse.',
          );

    return (
      <section key={tone} className="space-y-2 px-3 pt-3">
        <div className="flex items-end justify-between gap-3 px-1">
          <div>
            <p className="text-[11px] uppercase tracking-[0.16em] text-muted-foreground/48">
              {sectionTitle}
            </p>
            <p className="mt-1 text-[11px] leading-5 text-muted-foreground/68">
              {sectionHint}
            </p>
          </div>
          <div className="rounded-full bg-background/78 px-2.5 py-1 text-[11px] font-medium text-foreground ring-1 ring-border/18">
            {sectionArtifacts.length}
          </div>
        </div>
        <div className="overflow-hidden rounded-[20px] bg-background/52 ring-1 ring-border/12">
          {sectionArtifacts.map((artifact, index) => {
            const isSelected = artifact.artifact_id === selectedArtifact?.artifact_id;
            return (
              <button
                key={artifact.artifact_id}
                onClick={() => setSelectedArtifactId(artifact.artifact_id)}
                className={`relative w-full px-4 py-3 text-left transition-colors ${
                  index > 0 ? 'border-t border-border/10' : ''
                } ${isSelected ? 'bg-background/92' : 'bg-transparent hover:bg-background/66'}`}
              >
                <span
                  className={`absolute bottom-3 left-0 top-3 w-[3px] rounded-r-full ${
                    isSelected
                      ? tone === 'core'
                        ? 'bg-foreground/46'
                        : 'bg-muted-foreground/44'
                      : 'bg-transparent'
                  }`}
                  aria-hidden="true"
                />
                <div className="flex items-start justify-between gap-3 pl-1">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] uppercase tracking-[0.14em] text-muted-foreground/52">
                      <span className="font-mono">{artifact.artifact_type}</span>
                      <span>Step {artifact.step_index + 1}</span>
                      {artifact.is_required_output && (
                        <span className="rounded-full bg-foreground/7 px-2 py-0.5 text-[9px] font-semibold text-foreground/78 ring-1 ring-foreground/10">
                          {t('mission.requiredOutput', 'Requested')}
                        </span>
                      )}
                    </div>
                    <p className="mt-2 truncate text-sm font-semibold text-foreground">{artifact.name}</p>
                    {artifact.file_path && (
                      <p className="mt-1 truncate text-xs leading-5 text-muted-foreground/70">{artifact.file_path}</p>
                    )}
                  </div>
                  <div className="shrink-0 pt-0.5 text-right text-[11px] text-muted-foreground/66">
                    {formatSize(artifact.size)}
                  </div>
                </div>
              </button>
            );
          })}
        </div>
      </section>
    );
  };

  return (
    <div className="h-full overflow-hidden bg-transparent p-4">
      <div className="grid h-full overflow-hidden rounded-[28px] bg-[linear-gradient(180deg,rgba(252,249,243,0.94),rgba(255,255,255,0.98))] ring-1 ring-border/24 lg:grid-cols-[292px_minmax(0,1fr)]">
        <aside className="min-h-0 border-b border-border/18 bg-[linear-gradient(180deg,rgba(247,242,233,0.76),rgba(255,255,255,0.28))] lg:flex lg:flex-col lg:border-b-0 lg:border-r lg:border-r-border/18">
          <div className="border-b border-border/16 px-4 py-4">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <p className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground/56">
                  {t('mission.assetInventory', 'Asset inventory')}
                </p>
                <h3 className="mt-2 text-sm font-semibold text-foreground">
                  {t('mission.artifacts', 'Artifacts')}
                </h3>
                <p className="mt-1 text-xs leading-5 text-muted-foreground/72">
                  {t('mission.artifactRoleSplitHint', {
                    core: coreArtifacts.length,
                    supporting: supportingArtifacts.length,
                  })}
                </p>
              </div>
              <div className="rounded-2xl bg-background/72 px-3 py-2 text-right ring-1 ring-border/18">
                <p className="text-[10px] uppercase tracking-[0.16em] text-muted-foreground/48">
                  {t('mission.coreDeliverables', 'Core deliverables')}
                </p>
                <p className="mt-1 text-sm font-semibold text-foreground">{coreArtifacts.length}</p>
                <p className="mt-1 text-[11px] text-muted-foreground/64">
                  {t('mission.supportingArtifactsCount', { count: supportingArtifacts.length })}
                </p>
              </div>
            </div>
          </div>
          <div className="min-h-0 overflow-y-auto pb-3">
            {renderArtifactSection(coreArtifacts, 'core')}
            {renderArtifactSection(supportingArtifacts, 'supporting')}
          </div>

          {missionDocs.length > 0 && (
            <div className="border-t border-border/16 bg-background/54 px-4 py-3">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <p className="text-[11px] uppercase tracking-[0.16em] text-muted-foreground/54">
                    {t('mission.documentHandoff', 'Document handoff')}
                  </p>
                  <p className="mt-1 text-xs leading-5 text-muted-foreground/70">
                    {t('mission.documentHandoffHint', 'Archived assets can be accepted in AI Workbench and reused downstream.')}
                  </p>
                </div>
                <button
                  onClick={openDocumentsSection}
                  className="rounded-full bg-background px-3 py-1.5 text-xs transition-colors ring-1 ring-border/22 hover:bg-accent"
                >
                  {t('mission.openDocuments', 'Open Documents')}
                </button>
              </div>
              <div className="mt-3 space-y-2">
                {missionDocs.slice(0, 2).map(doc => (
                  <div key={doc.id} className="flex items-center justify-between gap-2 rounded-[16px] bg-background/76 px-3 py-2 text-xs ring-1 ring-border/16">
                    <div className="min-w-0">
                      <p className="truncate font-medium text-foreground">{doc.display_name || doc.name}</p>
                      <p className="mt-1 text-muted-foreground/68">{docStatusLabel(doc.status)}</p>
                    </div>
                    {isDraftStatus(doc.status) ? (
                      <button
                        onClick={() => openAcceptDialog(doc)}
                        disabled={acceptingDocId === doc.id}
                        className="rounded-full bg-background px-2.5 py-1 text-[11px] ring-1 ring-border/20 transition-colors hover:bg-accent disabled:opacity-50"
                      >
                        {acceptingDocId === doc.id
                          ? t('common.processing', 'Processing...')
                          : t('documents.accept', 'Accept')}
                      </button>
                    ) : (
                      <button
                        onClick={() => window.open(documentApi.getDownloadUrl(teamId, doc.id), '_blank')}
                        className="rounded-full bg-background px-2.5 py-1 text-[11px] ring-1 ring-border/20 transition-colors hover:bg-accent"
                      >
                        {t('documents.download', 'Download')}
                      </button>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}
        </aside>

        <section className="min-h-0 overflow-hidden bg-background/96 lg:flex lg:flex-col">
          {selectedArtifact ? (
            <>
              <div className="border-b border-border/16 px-6 py-5">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] uppercase tracking-[0.14em] text-muted-foreground/54">
                      <span className={`rounded-full px-2 py-0.5 text-[9px] font-semibold ring-1 ${
                        artifactRole(selectedArtifact) === 'core_deliverable'
                          ? 'bg-foreground/7 text-foreground/82 ring-foreground/12'
                          : 'bg-muted/55 text-muted-foreground ring-border/18'
                      }`}>
                        {artifactRoleLabel(selectedArtifact)}
                      </span>
                      <span className="font-mono text-[10px] uppercase tracking-[0.16em] text-muted-foreground/62">
                        {selectedArtifact.artifact_type}
                      </span>
                      <span className="text-[10px] uppercase tracking-[0.16em] text-muted-foreground/46">
                        Step {selectedArtifact.step_index + 1}
                      </span>
                      {selectedArtifact.is_required_output && (
                        <span className="rounded-full bg-foreground/7 px-2 py-0.5 text-[9px] font-semibold text-foreground/78 ring-1 ring-foreground/10">
                          {t('mission.requiredOutput', 'Requested')}
                        </span>
                      )}
                    </div>
                    <h3 className="mt-2 text-base font-semibold text-foreground">{selectedArtifact.name}</h3>
                    <p className="mt-2 text-xs leading-5 text-muted-foreground/72">
                      {artifactRoleHint(selectedArtifact)}
                    </p>
                    {selectedArtifact.file_path && (
                      <p className="mt-2 break-all text-xs leading-5 text-muted-foreground/76">
                        <span className="font-medium text-foreground/80">{t('mission.filePath', 'Path')}:</span>{' '}
                        {selectedArtifact.file_path}
                      </p>
                    )}
                  </div>
                  <div className="shrink-0 rounded-2xl bg-muted/10 px-3 py-2 text-right ring-1 ring-border/16">
                    <p className="text-[11px] uppercase tracking-[0.16em] text-muted-foreground/52">
                      {t('mission.previewPanel', 'Preview')}
                    </p>
                    <p className="mt-1 text-base font-semibold text-foreground">{formatSize(selectedArtifact.size)}</p>
                  </div>
                </div>

                <div className="mt-4 flex flex-wrap gap-2">
                  <button
                    onClick={() => window.open(missionApi.getArtifactDownloadUrl(selectedArtifact.artifact_id), '_blank')}
                    className="rounded-full bg-foreground px-3 py-1.5 text-xs font-medium text-background transition-opacity hover:opacity-88"
                  >
                    {t('documents.download', 'Download')}
                  </button>
                  {selectedArtifact.content && (
                    <>
                      <button
                        onClick={() => copyText(selectedArtifact.content || '')}
                        className="rounded-full px-3 py-1.5 text-xs text-muted-foreground transition-colors hover:bg-muted/24 hover:text-foreground"
                      >
                        {t('mission.copyContent', 'Copy content')}
                      </button>
                      <button
                        onClick={() => downloadContent(selectedArtifact.name, selectedArtifact.content || '')}
                        className="rounded-full px-3 py-1.5 text-xs text-muted-foreground transition-colors hover:bg-muted/24 hover:text-foreground"
                      >
                        {t('mission.downloadText', 'Download text')}
                      </button>
                    </>
                  )}
                  {selectedArtifact.file_path && (
                    <button
                      onClick={() => copyText(selectedArtifact.file_path || '')}
                      className="rounded-full px-3 py-1.5 text-xs text-muted-foreground transition-colors hover:bg-muted/24 hover:text-foreground"
                    >
                      {t('mission.copyPath', 'Copy path')}
                    </button>
                  )}
                  {!selectedArtifact.archived_document_id && (
                    <button
                      onClick={() => openArchiveDialog(selectedArtifact)}
                      disabled={archivingArtifactId === selectedArtifact.artifact_id}
                      className="rounded-full bg-background px-3 py-1.5 text-xs transition-colors ring-1 ring-border/35 hover:bg-accent disabled:opacity-50"
                    >
                      {archivingArtifactId === selectedArtifact.artifact_id
                        ? t('common.processing', 'Processing...')
                        : t('mission.archiveToDocuments', 'Archive to Documents')}
                    </button>
                  )}
                  {selectedArtifact.archived_document_id && (
                    <>
                      <span className="rounded-full bg-muted px-3 py-1.5 text-xs text-muted-foreground">
                        {t('mission.archivedDocumentStatus', 'Document status')}: {docStatusLabel(selectedArtifact.archived_document_status)}
                      </span>
                      {isDraftStatus(selectedArtifact.archived_document_status) && (
                        <button
                          onClick={() => openAcceptDialogById(selectedArtifact.archived_document_id || '', selectedArtifact.name)}
                          disabled={!selectedArtifact.archived_document_id || acceptingDocId === selectedArtifact.archived_document_id}
                          className="rounded-full bg-background px-3 py-1.5 text-xs transition-colors ring-1 ring-border/35 hover:bg-accent disabled:opacity-50"
                        >
                          {acceptingDocId === selectedArtifact.archived_document_id
                            ? t('common.processing', 'Processing...')
                            : t('documents.accept', 'Accept')}
                        </button>
                      )}
                      <button
                        onClick={() => window.open(documentApi.getDownloadUrl(teamId, selectedArtifact.archived_document_id || ''), '_blank')}
                        disabled={!selectedArtifact.archived_document_id}
                        className="rounded-full bg-background px-3 py-1.5 text-xs transition-colors ring-1 ring-border/35 hover:bg-accent"
                      >
                        {t('mission.viewArchivedDocument', 'View document')}
                      </button>
                    </>
                  )}
                </div>
              </div>

              <div className="min-h-0 flex-1 px-6 py-5">
                <ArtifactPreview
                  artifact={selectedArtifact}
                  downloadUrl={missionApi.getArtifactDownloadUrl(selectedArtifact.artifact_id)}
                />
              </div>
            </>
          ) : (
            <div className="flex h-full items-center justify-center px-6 text-sm text-muted-foreground">
              {t('mission.noArtifacts', 'No artifacts')}
            </div>
          )}
        </section>
      </div>

      <Dialog open={archiveDialogOpen} onOpenChange={(open) => { if (!open) closeArchiveDialog(); }}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t('mission.archiveToDocuments', 'Archive to Documents')}</DialogTitle>
          </DialogHeader>

          <div className="space-y-3">
            <div>
              <label className="text-xs text-muted-foreground block mb-1">
                {t('documents.displayName', 'Display name')}
              </label>
              <Input
                value={archiveName}
                onChange={(e) => setArchiveName(e.target.value)}
                placeholder={t('documents.displayName', 'Display name')}
              />
            </div>
            <div>
              <label className="text-xs text-muted-foreground block mb-1">
                {t('documents.folders', 'Folders')}
              </label>
              <Select value={archiveFolderPath} onValueChange={setArchiveFolderPath}>
                <SelectTrigger>
                  <SelectValue placeholder={t('documents.allFiles', 'All files')} />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="/">{t('documents.allFiles', 'All files')}</SelectItem>
                  {flattenFolders(folderTree).map((folder) => (
                    <SelectItem key={folder.path} value={folder.path}>
                      {folder.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {foldersLoading && (
                <p className="text-xs text-muted-foreground mt-1">
                  {t('common.loading', 'Loading...')}
                </p>
              )}
            </div>
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={closeArchiveDialog}>
              {t('common.cancel', 'Cancel')}
            </Button>
            <Button
              onClick={confirmArchiveDialog}
              disabled={!archiveTarget || !!(archivingArtifactId && archivingArtifactId !== archiveTarget?.artifact_id)}
            >
              {archiveTarget && archivingArtifactId === archiveTarget.artifact_id
                ? t('common.processing', 'Processing...')
                : t('mission.archiveToDocuments', 'Archive to Documents')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={acceptDialogOpen} onOpenChange={(open) => { if (!open) closeAcceptDialog(); }}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t('documents.accept', 'Accept')}</DialogTitle>
          </DialogHeader>

          <div className="space-y-3">
            <div>
              <label className="text-xs text-muted-foreground block mb-1">
                {t('documents.displayName', 'Display name')}
              </label>
              <Input
                value={acceptName}
                onChange={(e) => setAcceptName(e.target.value)}
                placeholder={t('documents.displayName', 'Display name')}
              />
            </div>
            <div>
              <label className="text-xs text-muted-foreground block mb-1">
                {t('documents.folders', 'Folders')}
              </label>
              <Select value={acceptFolderPath} onValueChange={setAcceptFolderPath}>
                <SelectTrigger>
                  <SelectValue placeholder={t('documents.allFiles', 'All files')} />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="/">{t('documents.allFiles', 'All files')}</SelectItem>
                  {flattenFolders(folderTree).map((folder) => (
                    <SelectItem key={folder.path} value={folder.path}>
                      {folder.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {foldersLoading && (
                <p className="text-xs text-muted-foreground mt-1">
                  {t('common.loading', 'Loading...')}
                </p>
              )}
            </div>
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={closeAcceptDialog}>
              {t('common.cancel', 'Cancel')}
            </Button>
            <Button
              onClick={confirmAcceptDialog}
              disabled={!acceptTargetDoc || (acceptingDocId !== null && acceptingDocId !== acceptTargetDoc.id)}
            >
              {acceptTargetDoc && acceptingDocId === acceptTargetDoc.id
                ? t('common.loading', 'Loading...')
                : t('documents.accept', 'Accept')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
