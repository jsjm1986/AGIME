import { useState, useEffect } from 'react';
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
  const [expanded, setExpanded] = useState<string | null>(null);
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

  useEffect(() => {
    let cancelled = false;
    loadData().then(() => {
      if (!cancelled) setLoading(false);
    }).catch(() => {
      if (!cancelled) setLoading(false);
    });
    return () => { cancelled = true; };
  }, [missionId, teamId]);

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

  return (
    <div className="space-y-2 p-3">
      {missionDocs.length > 0 && (
        <div className="border rounded-md p-2 bg-muted/20">
          <div className="flex items-center justify-between gap-2 mb-1">
            <div className="text-xs text-muted-foreground">
              {t('mission.relatedDocuments', 'Related documents in AI Workbench')}
            </div>
            <button
              onClick={openDocumentsSection}
              className="text-xs px-2 py-0.5 rounded border border-border hover:bg-accent transition-colors"
            >
              {t('mission.openDocuments', 'Open Documents')}
            </button>
          </div>
          <div className="text-caption text-muted-foreground mb-2">
            {t(
              'mission.relatedDocumentsHint',
              'Accept here or in AI Workbench. Accepted documents will appear in Files.',
            )}
          </div>
          <div className="space-y-1">
            {missionDocs.slice(0, 5).map(doc => (
              <div key={doc.id} className="flex items-center justify-between gap-2 text-xs">
                <span className="truncate">
                  {doc.display_name || doc.name}
                </span>
                <div className="flex items-center gap-2 shrink-0">
                  <span className="px-1.5 py-0.5 rounded bg-muted">
                    {docStatusLabel(doc.status)}
                  </span>
                  {isDraftStatus(doc.status) && (
                    <button
                      onClick={() => openAcceptDialog(doc)}
                      disabled={acceptingDocId === doc.id}
                      className="px-2 py-0.5 rounded border border-border hover:bg-accent transition-colors disabled:opacity-50"
                    >
                      {acceptingDocId === doc.id
                        ? t('common.processing', 'Processing...')
                        : t('documents.accept', 'Accept')}
                    </button>
                  )}
                  <button
                    onClick={() => window.open(documentApi.getDownloadUrl(teamId, doc.id), '_blank')}
                    className="px-2 py-0.5 rounded border border-border hover:bg-accent transition-colors"
                  >
                    {t('documents.download', 'Download')}
                  </button>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
      {artifacts.map(a => (
        <div key={a.artifact_id} className="border rounded-md">
          <button
            onClick={() => setExpanded(
              expanded === a.artifact_id ? null : a.artifact_id
            )}
            className="w-full flex items-center justify-between p-2 text-sm hover:bg-accent rounded-md"
          >
            <div className="flex items-center gap-2">
              <span className="text-xs px-1.5 py-0.5 rounded bg-muted font-mono">
                {a.artifact_type}
              </span>
              <span className="font-medium truncate">{a.name}</span>
            </div>
            <div className="text-xs text-muted-foreground flex items-center gap-2">
              <span>{formatSize(a.size)}</span>
              <span>·</span>
              <span>Step {a.step_index + 1}</span>
            </div>
          </button>

          {expanded === a.artifact_id && (
            <div className="border-t p-2">
              {a.file_path && (
                <div className="text-xs text-muted-foreground mb-2 break-all">
                  <span className="font-medium mr-1">{t('mission.filePath', 'Path')}:</span>
                  {a.file_path}
                </div>
              )}

              <div className="flex items-center gap-2 mb-2">
                <button
                  onClick={() => window.open(missionApi.getArtifactDownloadUrl(a.artifact_id), '_blank')}
                  className="text-xs px-2 py-1 rounded border border-border hover:bg-accent transition-colors"
                >
                  {t('documents.download', 'Download')}
                </button>
                {a.content && (
                  <>
                    <button
                      onClick={() => copyText(a.content || '')}
                      className="text-xs px-2 py-1 rounded border border-border hover:bg-accent transition-colors"
                    >
                      {t('mission.copyContent', 'Copy content')}
                    </button>
                    <button
                      onClick={() => downloadContent(a.name, a.content || '')}
                      className="text-xs px-2 py-1 rounded border border-border hover:bg-accent transition-colors"
                    >
                      {t('mission.downloadText', 'Download text')}
                    </button>
                  </>
                )}
                {a.file_path && (
                  <button
                    onClick={() => copyText(a.file_path || '')}
                    className="text-xs px-2 py-1 rounded border border-border hover:bg-accent transition-colors"
                  >
                    {t('mission.copyPath', 'Copy path')}
                  </button>
                )}
                {!a.archived_document_id && (
                  <button
                    onClick={() => openArchiveDialog(a)}
                    disabled={archivingArtifactId === a.artifact_id}
                    className="text-xs px-2 py-1 rounded border border-border hover:bg-accent transition-colors disabled:opacity-50"
                  >
                    {archivingArtifactId === a.artifact_id
                      ? t('common.processing', 'Processing...')
                      : t('mission.archiveToDocuments', 'Archive to Documents')}
                  </button>
                )}
                {a.archived_document_id && (
                  <>
                    <span className="text-xs px-2 py-1 rounded bg-muted">
                      {t('mission.archivedDocumentStatus', 'Document status')}: {docStatusLabel(a.archived_document_status)}
                    </span>
                    {isDraftStatus(a.archived_document_status) && (
                      <button
                        onClick={() => openAcceptDialogById(a.archived_document_id || '', a.name)}
                        disabled={!a.archived_document_id || acceptingDocId === a.archived_document_id}
                        className="text-xs px-2 py-1 rounded border border-border hover:bg-accent transition-colors disabled:opacity-50"
                      >
                        {acceptingDocId === a.archived_document_id
                          ? t('common.processing', 'Processing...')
                          : t('documents.accept', 'Accept')}
                      </button>
                    )}
                    <button
                      onClick={() => window.open(documentApi.getDownloadUrl(teamId, a.archived_document_id || ''), '_blank')}
                      disabled={!a.archived_document_id}
                      className="text-xs px-2 py-1 rounded border border-border hover:bg-accent transition-colors"
                    >
                      {t('mission.viewArchivedDocument', 'View document')}
                    </button>
                  </>
                )}
              </div>

              <ArtifactPreview
                artifact={a}
                downloadUrl={missionApi.getArtifactDownloadUrl(a.artifact_id)}
              />
            </div>
          )}
        </div>
      ))}

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
