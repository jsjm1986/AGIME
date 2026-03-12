import { useState, useEffect, useRef, useCallback, useMemo, lazy, Suspense } from 'react';
import { useTranslation } from 'react-i18next';
import { Loader2, FolderOpen, CheckSquare, X, Download } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { Card, CardHeader, CardTitle, CardContent } from '../ui/card';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Skeleton } from '../ui/skeleton';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui/select';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '../ui/dialog';
import {
  folderApi,
  documentApi,
  formatFileSize,
} from '../../api/documents';
import type {
  DocumentBindingPortalRef,
  DocumentBindingUsageSummary,
  FolderTreeNode,
  DocumentSummary,
  LockInfo,
  VersionSummary,
} from '../../api/documents';
import { DocumentEditor } from '../documents/DocumentEditor';
import { VersionTimeline } from '../documents/VersionTimeline';
import { VersionDiff } from '../documents/VersionDiff';
import { AiWorkbench } from '../documents/AiWorkbench';
import { DocumentLineage } from '../documents/DocumentLineage';
import { SupportedFormatsGuide } from '../documents/SupportedFormatsGuide';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { useIsMobile } from '../../hooks/useMediaQuery';
import { useToast } from '../../contexts/ToastContext';
import { formatDate, formatDateTime } from '../../utils/format';

const MAX_FILE_SIZE = 50 * 1024 * 1024; // 50MB
const DocumentPreview = lazy(() =>
  import('../documents/DocumentPreview').then((module) => ({ default: module.DocumentPreview })),
);

function DocumentPreviewLoading() {
  return (
    <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
      正在加载文档预览...
    </div>
  );
}

function getFileIcon(mimeType: string): string {
  if (mimeType.startsWith('image/')) return '🖼️';
  if (mimeType.startsWith('video/')) return '🎬';
  if (mimeType.startsWith('audio/')) return '🎵';
  if (mimeType === 'application/pdf') return '📄';
  if (mimeType.includes('word')) return '📝';
  if (mimeType.includes('sheet') || mimeType.includes('excel')) return '📊';
  if (mimeType.includes('zip') || mimeType.includes('rar')) return '📦';
  if (mimeType.startsWith('text/')) return '📃';
  return '📁';
}

type BindingTone = 'read' | 'draft' | 'write';

function getBindingToneClasses(tone: BindingTone): string {
  if (tone === 'write') {
    return 'border-red-200 bg-red-50 text-red-700';
  }
  if (tone === 'draft') {
    return 'border-violet-200 bg-violet-50 text-violet-700';
  }
  return 'border-sky-200 bg-sky-50 text-sky-700';
}

function getBindingLabel(bindings: DocumentBindingPortalRef[], tone: BindingTone): string {
  const toneLabel = tone === 'write' ? '允许直写' : tone === 'draft' ? '草稿协作' : '读取中';
  if (bindings.length === 0) {
    return '';
  }
  if (bindings.length === 1) {
    return `${bindings[0].portalName} · ${toneLabel}`;
  }
  return `${bindings.length} 个分身${toneLabel}`;
}

function getBindingModeLabel(mode: DocumentBindingPortalRef['documentAccessMode']): string {
  if (mode === 'controlled_write') return '允许直写';
  if (mode === 'co_edit_draft') return '草稿协作';
  return '只读';
}

function getPortalStatusLabel(status: DocumentBindingPortalRef['portalStatus']): string {
  if (status === 'published') return '已发布';
  if (status === 'archived') return '已归档';
  return '草稿中';
}

function buildBindingTitle(bindings: DocumentBindingPortalRef[], prefix: string): string {
  return bindings.map((binding) => `${prefix}：${binding.portalName}`).join('\n');
}

type ViewMode = 'folders' | 'aiWorkbench' | 'lineage' | 'trash';
type RightPanelMode = 'preview' | 'edit' | 'versions' | 'diff' | null;
type BindingFilterMode = 'all' | 'bound' | 'read' | 'draft' | 'write' | 'unbound';

interface PaginationState {
  page: number;
  total: number;
  totalPages: number;
}

interface FolderDialogState {
  open: boolean;
  name: string;
  desc: string;
}

interface RenameFolderState {
  open: boolean;
  target: FolderTreeNode | null;
  name: string;
}

interface EditMetaState {
  open: boolean;
  doc: DocumentSummary | null;
  displayName: string;
  description: string;
  tags: string[];
  tagInput: string;
  saving: boolean;
}

interface RightPanelState {
  doc: DocumentSummary | null;
  mode: RightPanelMode;
  editContent: string;
  editLock: LockInfo | null;
  diffVersions: [VersionSummary, VersionSummary] | null;
}

type UploadEntry = { name: string; progress: number; done: boolean; error?: string };

const INITIAL_FOLDER_DIALOG: FolderDialogState = { open: false, name: '', desc: '' };
const INITIAL_RENAME: RenameFolderState = { open: false, target: null, name: '' };
const INITIAL_EDIT_META: EditMetaState = { open: false, doc: null, displayName: '', description: '', tags: [], tagInput: '', saving: false };
const INITIAL_PANEL: RightPanelState = { doc: null, mode: null, editContent: '', editLock: null, diffVersions: null };

interface DocumentsTabProps {
  teamId: string;
  canManage: boolean;
}

export function DocumentsTab({ teamId, canManage }: DocumentsTabProps) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const { addToast } = useToast();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const navigate = useNavigate();

  // Core data
  const [folders, setFolders] = useState<FolderTreeNode[]>([]);
  const [documents, setDocuments] = useState<DocumentSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [currentFolderPath, setCurrentFolderPath] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [debouncedSearch, setDebouncedSearch] = useState('');
  const [mimeFilter, setMimeFilter] = useState('');
  const [tagFilter, setTagFilter] = useState('');
  const [bindingFilter, setBindingFilter] = useState<BindingFilterMode>('all');
  const [teamTags, setTeamTags] = useState<{ tag: string; count: number }[]>([]);
  const [sortBy, setSortBy] = useState<'date' | 'name' | 'size'>('date');
  const [pagination, setPagination] = useState<PaginationState>({ page: 1, total: 0, totalPages: 0 });
  const limit = 50;

  // UI toggles
  const [showFolderTree, setShowFolderTree] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [selectionMode, setSelectionMode] = useState(false);
  const [viewMode, setViewMode] = useState<ViewMode>('folders');
  const [lineageDocId, setLineageDocId] = useState<string | null>(null);
  const [archivedDocuments, setArchivedDocuments] = useState<DocumentSummary[]>([]);
  const [archivedLoading, setArchivedLoading] = useState(false);
  const [archivedPage, setArchivedPage] = useState(1);
  const [archivedTotalPages, setArchivedTotalPages] = useState(0);
  const [bindingUsageByDocId, setBindingUsageByDocId] = useState<Record<string, DocumentBindingUsageSummary>>({});
  const [bindingUsageLoading, setBindingUsageLoading] = useState(false);

  // Grouped dialog/panel states
  const [folderDialog, setFolderDialog] = useState<FolderDialogState>(INITIAL_FOLDER_DIALOG);
  const [renameFolder, setRenameFolder] = useState<RenameFolderState>(INITIAL_RENAME);
  const [editMeta, setEditMeta] = useState<EditMetaState>(INITIAL_EDIT_META);
  const [panel, setPanel] = useState<RightPanelState>(INITIAL_PANEL);
  const [uploadProgress, setUploadProgress] = useState<Map<string, UploadEntry>>(new Map());

  // Confirm dialog targets
  const [deleteDocTarget, setDeleteDocTarget] = useState<string | null>(null);
  const [showBatchDeleteConfirm, setShowBatchDeleteConfirm] = useState(false);
  const [deleteFolderTarget, setDeleteFolderTarget] = useState<FolderTreeNode | null>(null);

  const loadData = useCallback(async () => {
    setLoading(true);
    try {
      const [foldersRes, docsRes] = await Promise.all([
        folderApi.getFolderTree(teamId),
        debouncedSearch
          ? documentApi.searchDocuments(teamId, debouncedSearch, pagination.page, limit, mimeFilter || undefined, currentFolderPath || undefined, tagFilter || undefined)
          : documentApi.listDocuments(teamId, pagination.page, limit, currentFolderPath || undefined, mimeFilter || undefined, tagFilter || undefined),
      ]);
      setFolders(foldersRes);
      setDocuments(docsRes.items);
      setPagination(prev => ({ ...prev, total: docsRes.total, totalPages: docsRes.total_pages }));
    } catch (error) {
      console.error('Failed to load documents:', error);
    } finally {
      setLoading(false);
    }
  }, [teamId, currentFolderPath, debouncedSearch, pagination.page, mimeFilter, tagFilter]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // Debounce search input — only fire API after 300ms idle
  useEffect(() => {
    const timer = setTimeout(() => setDebouncedSearch(searchQuery), 300);
    return () => clearTimeout(timer);
  }, [searchQuery]);

  useEffect(() => {
    setPagination(prev => ({ ...prev, page: 1 }));
  }, [debouncedSearch, currentFolderPath, mimeFilter, tagFilter]);

  // Load team tags for filter dropdown and autocomplete
  const loadTeamTags = useCallback(async () => {
    try {
      const tags = await documentApi.listTags(teamId);
      setTeamTags(tags);
    } catch (error) {
      console.error('Failed to load tags:', error);
    }
  }, [teamId]);

  useEffect(() => {
    loadTeamTags();
  }, [loadTeamTags]);

  const loadArchivedData = useCallback(async () => {
    setArchivedLoading(true);
    try {
      const res = await documentApi.listArchived(teamId, archivedPage, limit);
      setArchivedDocuments(res.items);
      setArchivedTotalPages(res.total_pages);
    } catch (error) {
      console.error('Failed to load archived documents:', error);
    } finally {
      setArchivedLoading(false);
    }
  }, [teamId, archivedPage, limit]);

  useEffect(() => {
    if (viewMode === 'trash') {
      loadArchivedData();
    }
  }, [viewMode, loadArchivedData]);

  const loadBindingUsage = useCallback(async (docIds: string[]) => {
    if (docIds.length === 0) {
      setBindingUsageByDocId({});
      return;
    }
    setBindingUsageByDocId({});
    setBindingUsageLoading(true);
    try {
      const rows = await documentApi.getBindingUsage(teamId, docIds);
      setBindingUsageByDocId(
        rows.reduce<Record<string, DocumentBindingUsageSummary>>((acc, row) => {
          acc[row.docId] = row;
          return acc;
        }, {}),
      );
    } catch (error) {
      console.error('Failed to load document binding usage:', error);
    } finally {
      setBindingUsageLoading(false);
    }
  }, [teamId]);

  useEffect(() => {
    if (viewMode !== 'folders') {
      return;
    }
    void loadBindingUsage(documents.map((doc) => doc.id));
  }, [documents, loadBindingUsage, viewMode]);

  const handleCreateFolder = async () => {
    if (!folderDialog.name.trim()) return;
    try {
      await folderApi.createFolder(teamId, {
        name: folderDialog.name.trim(),
        parentPath: currentFolderPath || '/',
        description: folderDialog.desc.trim() || undefined,
      });
      setFolderDialog(INITIAL_FOLDER_DIALOG);
      loadData();
    } catch (error) {
      console.error('Failed to create folder:', error);
    }
  };

  const handleUploadClick = () => {
    fileInputRef.current?.click();
  };

  const uploadFileWithProgress = useCallback((file: File, folderPath?: string): Promise<void> => {
    return new Promise((resolve, reject) => {
      const key = `${file.name}-${Date.now()}`;
      setUploadProgress(prev => new Map(prev).set(key, { name: file.name, progress: 0, done: false }));

      const formData = new FormData();
      formData.append('file', file);
      if (folderPath) formData.append('folder_path', folderPath);

      const xhr = new XMLHttpRequest();
      xhr.open('POST', `/api/team/teams/${teamId}/documents`);
      xhr.withCredentials = true;

      xhr.upload.onprogress = (e) => {
        if (e.lengthComputable) {
          const progress = Math.round((e.loaded / e.total) * 100);
          setUploadProgress(prev => new Map(prev).set(key, { name: file.name, progress, done: false }));
        }
      };

      xhr.onload = () => {
        if (xhr.status >= 200 && xhr.status < 300) {
          setUploadProgress(prev => new Map(prev).set(key, { name: file.name, progress: 100, done: true }));
          setTimeout(() => setUploadProgress(prev => { const m = new Map(prev); m.delete(key); return m; }), 2000);
          resolve();
        } else {
          const errMsg = 'Upload failed';
          setUploadProgress(prev => new Map(prev).set(key, { name: file.name, progress: 0, done: true, error: errMsg }));
          reject(new Error(errMsg));
        }
      };

      xhr.onerror = () => {
        setUploadProgress(prev => new Map(prev).set(key, { name: file.name, progress: 0, done: true, error: 'Network error' }));
        reject(new Error('Network error'));
      };

      xhr.send(formData);
    });
  }, [teamId]);

  const processFiles = useCallback(async (files: File[]) => {
    const validFiles: File[] = [];
    for (const file of files) {
      if (file.size > MAX_FILE_SIZE) {
        addToast('warning', t('documents.fileTooLarge', { name: file.name }));
        continue;
      }
      validFiles.push(file);
    }
    if (validFiles.length === 0) return;

    setUploading(true);
    try {
      for (const file of validFiles) {
        await uploadFileWithProgress(file, currentFolderPath || undefined);
      }
      loadData();
    } catch (error) {
      console.error('Failed to upload:', error);
    } finally {
      setUploading(false);
    }
  }, [currentFolderPath, loadData, t, uploadFileWithProgress]);

  const handleFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files;
    if (!files || files.length === 0) return;
    await processFiles(Array.from(files));
    if (fileInputRef.current) {
      fileInputRef.current.value = '';
    }
  };

  const handleDeleteDocument = (docId: string) => {
    setDeleteDocTarget(docId);
  };

  const handleRestoreDocument = async (docId: string) => {
    try {
      await documentApi.restoreDocument(teamId, docId);
      loadArchivedData();
      if (viewMode === 'folders') {
        loadData();
      }
    } catch (error) {
      console.error('Failed to restore document:', error);
    }
  };

  const confirmDeleteDocument = async () => {
    if (!deleteDocTarget) return;
    setDocuments(prev => prev.filter(d => d.id !== deleteDocTarget));
    setSelectedIds(prev => { const next = new Set(prev); next.delete(deleteDocTarget); return next; });
    try {
      await documentApi.deleteDocument(teamId, deleteDocTarget);
      loadData();
    } catch (error) {
      console.error('Failed to delete:', error);
      loadData();
    } finally {
      setDeleteDocTarget(null);
    }
  };

  // Drag & drop handlers
  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(true);
  };

  const handleDragLeave = (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
  };

  const handleDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
    const files = Array.from(e.dataTransfer.files);
    if (files.length > 0) {
      await processFiles(files);
    }
  };

  // Batch operations
  const toggleSelect = (docId: string) => {
    setSelectedIds(prev => {
      const next = new Set(prev);
      if (next.has(docId)) next.delete(docId);
      else next.add(docId);
      return next;
    });
  };

  const toggleSelectAll = () => {
    setSelectedIds(prev => {
      const next = new Set(prev);
      if (allVisibleSelected) {
        visibleDocs.forEach((doc) => next.delete(doc.id));
      } else {
        visibleDocs.forEach((doc) => next.add(doc.id));
      }
      return next;
    });
  };

  const exitSelectionMode = () => {
    setSelectionMode(false);
    setSelectedIds(new Set());
  };

  const handleBatchDownload = () => {
    if (selectedIds.size === 0) return;
    for (const id of selectedIds) {
      window.open(documentApi.getDownloadUrl(teamId, id), '_blank');
    }
  };

  const handleBatchDelete = () => {
    if (selectedIds.size === 0) return;
    setShowBatchDeleteConfirm(true);
  };

  const confirmBatchDelete = async () => {
    try {
      const results = await Promise.allSettled(
        Array.from(selectedIds).map(id => documentApi.deleteDocument(teamId, id))
      );
      const failures = results.filter(r => r.status === 'rejected');
      if (failures.length > 0) {
        console.error(`Failed to delete ${failures.length} documents`);
      }
    } finally {
      setSelectedIds(new Set());
      setShowBatchDeleteConfirm(false);
      loadData();
    }
  };

  // Folder management
  const handleDeleteFolder = (node: FolderTreeNode) => {
    setDeleteFolderTarget(node);
  };

  const confirmDeleteFolder = async () => {
    if (!deleteFolderTarget) return;
    try {
      await folderApi.deleteFolder(teamId, deleteFolderTarget.id);
      if (currentFolderPath === deleteFolderTarget.fullPath) setCurrentFolderPath(null);
      loadData();
    } catch (error) {
      console.error('Failed to delete folder:', error);
    } finally {
      setDeleteFolderTarget(null);
    }
  };

  const openRenameFolder = (node: FolderTreeNode) => {
    setRenameFolder({ open: true, target: node, name: node.name });
  };

  const handleRenameFolder = async () => {
    if (!renameFolder.target || !renameFolder.name.trim()) return;
    try {
      await folderApi.updateFolder(teamId, renameFolder.target.id, { name: renameFolder.name.trim() });
      setRenameFolder(INITIAL_RENAME);
      loadData();
    } catch (error) {
      console.error('Failed to rename folder:', error);
    }
  };

  const openEditMeta = (doc: DocumentSummary) => {
    setEditMeta({
      open: true, doc, saving: false,
      displayName: doc.display_name || doc.name,
      description: doc.description || '',
      tags: doc.tags || [],
      tagInput: '',
    });
  };

  const handleSaveMeta = async () => {
    if (!editMeta.doc) return;
    setEditMeta(prev => ({ ...prev, saving: true }));
    try {
      await documentApi.updateDocument(teamId, editMeta.doc.id, {
        display_name: editMeta.displayName.trim() || undefined,
        description: editMeta.description.trim() || undefined,
        tags: editMeta.tags,
      });
      setEditMeta(INITIAL_EDIT_META);
      loadData();
      loadTeamTags();
    } catch (error) {
      console.error('Failed to update metadata:', error);
    } finally {
      setEditMeta(prev => ({ ...prev, saving: false }));
    }
  };

  const addTagToEdit = (tag: string) => {
    const trimmed = tag.trim();
    if (!trimmed) return;
    setEditMeta(prev => {
      if (prev.tags.includes(trimmed)) return { ...prev, tagInput: '' };
      return { ...prev, tags: [...prev.tags, trimmed], tagInput: '' };
    });
  };

  const removeTagFromEdit = (tag: string) => {
    setEditMeta(prev => ({ ...prev, tags: prev.tags.filter(t => t !== tag) }));
  };

  const tagSuggestions = useMemo(() => {
    if (!editMeta.tagInput) return [];
    const input = editMeta.tagInput.toLowerCase();
    return teamTags
      .map(t => t.tag)
      .filter(t => t.toLowerCase().includes(input) && !editMeta.tags.includes(t))
      .slice(0, 5);
  }, [editMeta.tagInput, editMeta.tags, teamTags]);

  const handleDownload = (docId: string) => {
    window.open(documentApi.getDownloadUrl(teamId, docId), '_blank');
  };

  const handleDocClick = (doc: DocumentSummary) => {
    setPanel({ ...INITIAL_PANEL, doc, mode: 'preview' });
  };

  const handleClosePanel = () => {
    setPanel(INITIAL_PANEL);
  };

  const handleEdit = async () => {
    if (!panel.doc) return;
    try {
      const lock = await documentApi.acquireLock(teamId, panel.doc.id);
      const res = await documentApi.getTextContent(teamId, panel.doc.id);
      setPanel(prev => ({ ...prev, editLock: lock, editContent: res.text, mode: 'edit' }));
    } catch (err) {
      console.error('Failed to start editing:', err);
    }
  };

  const handleEditSave = () => {
    setPanel(prev => ({ ...prev, mode: 'preview', editLock: null }));
    loadData();
  };

  const handleEditClose = () => {
    setPanel(prev => ({ ...prev, mode: 'preview', editLock: null }));
  };

  const handleVersions = () => {
    setPanel(prev => ({ ...prev, mode: 'versions', diffVersions: null }));
  };

  const handleCompare = (v1: VersionSummary, v2: VersionSummary) => {
    setPanel(prev => ({ ...prev, diffVersions: [v1, v2], mode: 'diff' }));
  };

  const handleRollback = () => {
    loadData();
    setPanel(prev => ({ ...prev, mode: 'preview' }));
  };

  const renderFolderTree = (nodes: FolderTreeNode[], level = 0) => {
    const sorted = [...nodes].sort((a, b) => (b.is_system ? 1 : 0) - (a.is_system ? 1 : 0));
    return sorted.map((node) => (
      <div key={node.id}>
        <div
          className={`group flex items-center gap-2 px-2 py-1.5 rounded cursor-pointer hover:bg-muted ${
            currentFolderPath === node.fullPath ? 'bg-muted' : ''
          }`}
          style={{ paddingLeft: `${level * 16 + 8}px` }}
          onClick={() => setCurrentFolderPath(node.fullPath)}
        >
          <span>{node.is_system ? '🌐' : '📁'}</span>
          <span className="flex-1 truncate text-sm">{node.name}</span>
          {canManage && !node.is_system && (
            <span className="hidden group-hover:flex items-center gap-0.5" onClick={(e) => e.stopPropagation()}>
              <button className="p-0.5 rounded hover:bg-muted-foreground/20 text-xs" title={t('documents.renameFolder')} onClick={() => openRenameFolder(node)}>✏️</button>
              <button className="p-0.5 rounded hover:bg-destructive/20 text-xs" title={t('common.delete')} onClick={() => handleDeleteFolder(node)}>🗑️</button>
            </span>
          )}
        </div>
        {node.children.length > 0 && renderFolderTree(node.children, level + 1)}
      </div>
    ));
  };

  const sortedDocs = useMemo(() => {
    const sorted = [...documents];
    switch (sortBy) {
      case 'name': sorted.sort((a, b) => (a.display_name || a.name).localeCompare(b.display_name || b.name)); break;
      case 'size': sorted.sort((a, b) => b.file_size - a.file_size); break;
      default: sorted.sort((a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime());
    }
    return sorted;
  }, [documents, sortBy]);

  const visibleDocs = useMemo(() => {
    if (bindingFilter === 'all') {
      return sortedDocs;
    }

    return sortedDocs.filter((doc) => {
      const usage = bindingUsageByDocId[doc.id];
      const isReady = Boolean(usage);
      if (!isReady && bindingUsageLoading) {
        return false;
      }

      const readCount = usage?.readBindings.length ?? 0;
      const draftCount = usage?.draftBindings.length ?? 0;
      const writeCount = usage?.writeBindings.length ?? 0;
      const boundCount = readCount + draftCount + writeCount;

      switch (bindingFilter) {
        case 'bound':
          return boundCount > 0;
        case 'read':
          return readCount > 0;
        case 'draft':
          return draftCount > 0;
        case 'write':
          return writeCount > 0;
        case 'unbound':
          return boundCount === 0 && !bindingUsageLoading;
        default:
          return true;
      }
    });
  }, [bindingFilter, bindingUsageByDocId, bindingUsageLoading, sortedDocs]);

  const visibleDocIds = useMemo(() => new Set(visibleDocs.map((doc) => doc.id)), [visibleDocs]);
  const visibleSelectedCount = useMemo(
    () => Array.from(selectedIds).filter((id) => visibleDocIds.has(id)).length,
    [selectedIds, visibleDocIds],
  );
  const allVisibleSelected = visibleDocs.length > 0 && visibleSelectedCount === visibleDocs.length;

  useEffect(() => {
    setSelectedIds((prev) => {
      if (prev.size === 0) {
        return prev;
      }
      const next = new Set(Array.from(prev).filter((id) => visibleDocIds.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [visibleDocIds]);

  const renderBindingChips = (usage: DocumentBindingUsageSummary | undefined) => {
    if (!usage) {
      return null;
    }

    const chipDefs: Array<{ tone: BindingTone; bindings: DocumentBindingPortalRef[] }> = [
      { tone: 'write' as const, bindings: usage.writeBindings },
      { tone: 'draft' as const, bindings: usage.draftBindings },
      { tone: 'read' as const, bindings: usage.readBindings },
    ].filter((item) => item.bindings.length > 0);

    if (chipDefs.length === 0) {
      return null;
    }

    return (
      <div className="mt-1.5 flex flex-wrap gap-1.5">
        {chipDefs.map(({ tone, bindings }) => (
          <span
            key={tone}
            className={`inline-flex items-center rounded-full border px-2 py-0.5 text-[11px] font-medium ${getBindingToneClasses(tone)}`}
            title={buildBindingTitle(bindings, tone === 'write' ? '允许直写' : tone === 'draft' ? '草稿协作' : '读取中')}
          >
            {getBindingLabel(bindings, tone)}
          </span>
        ))}
      </div>
    );
  };

  const handleOpenAvatarBinding = (binding: DocumentBindingPortalRef) => {
    if (!binding.managerAgentId) {
      return;
    }
    navigate(`/teams/${teamId}/agent/avatar-managers/${binding.managerAgentId}`);
  };

  const renderBindingUsageDetail = (doc: DocumentSummary) => {
    const usage = bindingUsageByDocId[doc.id];
    const groups: Array<{ title: string; tone: BindingTone; bindings: DocumentBindingPortalRef[] }> = [
      { title: '允许直写', tone: 'write' as const, bindings: usage?.writeBindings ?? [] },
      { title: '草稿协作', tone: 'draft' as const, bindings: usage?.draftBindings ?? [] },
      { title: '读取中', tone: 'read' as const, bindings: usage?.readBindings ?? [] },
    ].filter((group) => group.bindings.length > 0);

    return (
      <div className="border-t bg-muted/10 px-4 py-3">
        <div className="flex items-start justify-between gap-3">
          <div>
            <div className="text-sm font-semibold">分身占用情况</div>
            <div className="text-xs text-muted-foreground">
              当前文档被哪些分身读取、草稿协作或允许直写
            </div>
          </div>
          {bindingUsageLoading && (
            <div className="text-xs text-muted-foreground">加载中...</div>
          )}
        </div>
        {groups.length === 0 ? (
          <div className="mt-3 rounded-lg border border-dashed px-3 py-2 text-xs text-muted-foreground">
            当前文档未被任何服务 Agent 绑定。
          </div>
        ) : (
          <div className="mt-3 space-y-3">
            {groups.map((group) => (
              <div key={group.title} className="space-y-2">
                <div className={`inline-flex items-center rounded-full border px-2 py-0.5 text-[11px] font-medium ${getBindingToneClasses(group.tone)}`}>
                  {group.title}
                </div>
                <div className="space-y-2">
                  {group.bindings.map((binding) => (
                    <div key={`${binding.portalId}-${group.title}`} className="rounded-lg border bg-background/80 px-3 py-2">
                      <div className="flex items-start justify-between gap-3">
                        <div className="min-w-0">
                          <div className="truncate text-sm font-medium">{binding.portalName}</div>
                          <div className="mt-1 flex flex-wrap gap-1.5 text-[11px] text-muted-foreground">
                            <span>{getBindingModeLabel(binding.documentAccessMode)}</span>
                            <span>·</span>
                            <span>{getPortalStatusLabel(binding.portalStatus)}</span>
                            <span>·</span>
                            <span>{binding.publicAccessEnabled ? '公开访问中' : '仅预览'}</span>
                          </div>
                          {binding.serviceAgentName && (
                            <div className="mt-1 text-xs text-muted-foreground">
                              服务 Agent：{binding.serviceAgentName}
                            </div>
                          )}
                        </div>
                        {binding.managerAgentId && binding.portalDomain === 'avatar' && (
                          <Button
                            size="sm"
                            variant="outline"
                            className="h-7 px-2 text-xs"
                            onClick={() => handleOpenAvatarBinding(binding)}
                          >
                            查看分身
                          </Button>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    );
  };

  // Build breadcrumb path from currentFolderPath
  const breadcrumbs = useMemo(() => {
    if (!currentFolderPath) return [];
    const parts = currentFolderPath.split('/').filter(Boolean);
    return parts.map((name, i) => ({
      name,
      path: '/' + parts.slice(0, i + 1).join('/'),
    }));
  }, [currentFolderPath]);

  if (loading) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  const hasRightPanel = panel.doc && panel.mode;

  return (
    <div className="flex flex-col h-[calc(100vh-40px)]">
      {/* View Mode Tabs */}
      <div className="flex items-center gap-1 mb-3 border-b pb-2">
        {(['folders', 'aiWorkbench', 'lineage', 'trash'] as ViewMode[]).map(mode => (
          <button
            key={mode}
            onClick={() => {
              setViewMode(mode);
              setLineageDocId(null);
              if (mode === 'trash') {
                setArchivedPage(1);
              }
            }}
            className={`px-3 py-1.5 text-sm rounded-md transition-colors ${
              viewMode === mode
                ? 'bg-primary text-primary-foreground'
                : 'hover:bg-muted text-muted-foreground'
            }`}
          >
            {t(`documents.viewMode.${mode}`)}
          </button>
        ))}
      </div>

      {/* AI Workbench View */}
      {viewMode === 'aiWorkbench' && (
        <div className="flex-1 min-h-0 px-4">
          <AiWorkbench teamId={teamId} canManage={canManage} />
        </div>
      )}

      {/* Lineage View */}
      {viewMode === 'lineage' && (
        <div className="flex-1 overflow-auto p-4">
          {lineageDocId ? (
            <DocumentLineage
              teamId={teamId}
              docId={lineageDocId}
              onNavigate={(id) => setLineageDocId(id)}
            />
          ) : (
            <div className="text-center py-8 text-muted-foreground text-sm">
              {t('documents.noAiDocuments')}
            </div>
          )}
        </div>
      )}

      {/* Trash View */} 
      {viewMode === 'trash' && (
        <div className="flex-1 overflow-auto p-4">
          <Card>
            <CardHeader className="py-3">
              <CardTitle className="text-sm">{t('documents.archivedDocuments')}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2">
              {archivedLoading ? (
                <div className="text-center py-8 text-muted-foreground text-sm">{t('common.loading')}</div>
              ) : archivedDocuments.length === 0 ? (
                <div className="text-center py-8 text-muted-foreground text-sm">{t('documents.archivedEmpty')}</div>
              ) : (
                archivedDocuments.map((doc) => (
                  <div key={doc.id} className="flex items-center gap-3 p-3 border rounded-lg">
                    <span className="text-2xl">{getFileIcon(doc.mime_type)}</span>
                    <div className="flex-1 min-w-0">
                      <p className="font-medium truncate">{doc.display_name || doc.name}</p>
                      <p className="text-xs text-muted-foreground">
                        {formatFileSize(doc.file_size)} · {formatDateTime(doc.updated_at || doc.created_at)}
                      </p>
                    </div>
                    {canManage && (
                      <Button size="sm" variant="outline" onClick={() => handleRestoreDocument(doc.id)}>
                        {t('documents.restore')}
                      </Button>
                    )}
                  </div>
                ))
              )}

              {archivedTotalPages > 1 && (
                <div className="flex items-center justify-center gap-2 pt-2">
                  <Button size="sm" variant="outline" disabled={archivedPage <= 1} onClick={() => setArchivedPage(p => p - 1)}>
                    {t('pagination.previous')}
                  </Button>
                  <span className="text-sm">{archivedPage} / {archivedTotalPages}</span>
                  <Button size="sm" variant="outline" disabled={archivedPage >= archivedTotalPages} onClick={() => setArchivedPage(p => p + 1)}>
                    {t('pagination.next')}
                  </Button>
                </div>
              )}
            </CardContent>
          </Card>
        </div>
      )}

      {/* Folders View (original) */}
      {viewMode === 'folders' && (
      <div className={`flex gap-4 flex-1 min-h-0 ${isMobile ? 'flex-col' : ''}`}>
      {/* Folder Tree - hidden on mobile unless toggled */}
      {isMobile && (
        <Button size="sm" variant="outline" className="self-start" onClick={() => setShowFolderTree(!showFolderTree)}>
          <FolderOpen className="w-4 h-4 mr-1.5" />
          {t('documents.folders')}
        </Button>
      )}
      {(!isMobile || showFolderTree) && (
      <Card className={isMobile ? 'w-full' : 'w-48 flex-shrink-0'}>
        <CardHeader className="py-3">
          <CardTitle className="text-sm flex items-center justify-between">
            {t('documents.folders')}
            {canManage && (
              <Button size="sm" variant="ghost" onClick={() => setFolderDialog(prev => ({ ...prev, open: true }))}>
                +
              </Button>
            )}
          </CardTitle>
        </CardHeader>
        <CardContent className="py-0 overflow-auto flex-1">
          <div
            className={`flex items-center gap-2 px-2 py-1.5 rounded cursor-pointer hover:bg-muted ${
              currentFolderPath === null ? 'bg-muted' : ''
            }`}
            onClick={() => setCurrentFolderPath(null)}
          >
            <span>🏠</span>
            <span>{t('documents.allFiles')}</span>
          </div>
          {renderFolderTree(folders)}
        </CardContent>
      </Card>
      )}

      {/* Document List */}
      <Card
        className={`flex-1 min-w-0 ${isDragging ? 'ring-2 ring-primary ring-dashed' : ''}`}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
      >
        <CardHeader className="py-3">
          <div className="flex items-center justify-between">
            <CardTitle className="text-sm">{t('documents.files')}</CardTitle>
            <div className="flex items-center gap-2 flex-wrap">
              <Input
                placeholder={t('documents.search')}
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className={`${isMobile ? 'flex-1 min-w-0' : 'w-36'} h-8`}
              />
              <Select value={bindingFilter} onValueChange={(value) => setBindingFilter(value as BindingFilterMode)}>
                <SelectTrigger className={`${isMobile ? 'w-[8.5rem]' : 'w-32'} h-8`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">全部占用</SelectItem>
                  <SelectItem value="bound">已绑定</SelectItem>
                  <SelectItem value="read">被读取</SelectItem>
                  <SelectItem value="draft">草稿协作</SelectItem>
                  <SelectItem value="write">允许直写</SelectItem>
                  <SelectItem value="unbound">未绑定</SelectItem>
                </SelectContent>
              </Select>
              {!isMobile && (
                <>
                  <Select value={mimeFilter || '__all__'} onValueChange={v => setMimeFilter(v === '__all__' ? '' : v)}>
                    <SelectTrigger className="w-32 h-8">
                      <SelectValue placeholder={t('documents.filterAll')} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="__all__">{t('documents.filterAll')}</SelectItem>
                      <SelectItem value="text/">{t('documents.filterDocuments')}</SelectItem>
                      <SelectItem value="image/">{t('documents.filterImages')}</SelectItem>
                      <SelectItem value="application/">{t('documents.filterCode')}</SelectItem>
                      <SelectItem value="video/,audio/">{t('documents.filterMedia')}</SelectItem>
                    </SelectContent>
                  </Select>
                  {teamTags.length > 0 && (
                    <Select value={tagFilter || '__all__'} onValueChange={v => setTagFilter(v === '__all__' ? '' : v)}>
                      <SelectTrigger className="w-32 h-8">
                        <SelectValue placeholder={t('documents.allTags')} />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="__all__">{t('documents.allTags')}</SelectItem>
                        {teamTags.map(({ tag, count }) => (
                          <SelectItem key={tag} value={tag}>{tag} ({count})</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  )}
                  <Select value={sortBy} onValueChange={v => setSortBy(v as 'date' | 'name' | 'size')}>
                    <SelectTrigger className="w-28 h-8">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="date">{t('documents.sortDate')}</SelectItem>
                      <SelectItem value="name">{t('documents.sortName')}</SelectItem>
                      <SelectItem value="size">{t('documents.sortSize')}</SelectItem>
                    </SelectContent>
                  </Select>
                  <SupportedFormatsGuide />
                  {canManage && !selectionMode && (
                    <Button size="sm" variant="outline" onClick={() => setSelectionMode(true)}>
                      <CheckSquare className="w-4 h-4 mr-1" />
                      {t('documents.selectMode')}
                    </Button>
                  )}
                  {canManage && selectionMode && (
                    <Button size="sm" variant="ghost" onClick={exitSelectionMode}>
                      <X className="w-4 h-4 mr-1" />
                      {t('documents.exitSelectMode')}
                    </Button>
                  )}
                </>
              )}
              {canManage && (
                <Button size="sm" onClick={handleUploadClick} disabled={uploading}>
                  {uploading ? t('documents.uploading') : t('documents.upload')}
                </Button>
              )}
              <input
                ref={fileInputRef}
                type="file"
                multiple
                className="hidden"
                onChange={handleFileChange}
              />
            </div>
          </div>
          {/* Batch toolbar */}
          {canManage && selectionMode && (
            <div className="flex items-center gap-2 mt-2 p-2 bg-muted rounded">
              <span className="text-sm">{t('documents.selectedCount', { count: visibleSelectedCount })}</span>
              <Button size="sm" variant="outline" onClick={handleBatchDownload} disabled={visibleSelectedCount === 0}>
                <Download className="w-3.5 h-3.5 mr-1" />
                {t('documents.batchDownload')}
              </Button>
              <Button size="sm" variant="destructive" onClick={handleBatchDelete} disabled={visibleSelectedCount === 0}>
                {t('documents.batchDelete')}
              </Button>
            </div>
          )}
        </CardHeader>
        <CardContent className="overflow-auto flex-1">
          {/* Breadcrumb */}
          {breadcrumbs.length > 0 && (
            <div className="flex items-center gap-1 text-xs text-muted-foreground mb-2 px-1">
              <button className="hover:text-foreground" onClick={() => setCurrentFolderPath(null)}>
                {t('documents.allFiles')}
              </button>
              {breadcrumbs.map((bc) => (
                <span key={bc.path} className="flex items-center gap-1">
                  <span>/</span>
                  <button className="hover:text-foreground" onClick={() => setCurrentFolderPath(bc.path)}>
                    {bc.name}
                  </button>
                </span>
              ))}
            </div>
          )}
          {bindingFilter !== 'all' && (
            <div className="mb-2 px-1 text-xs text-muted-foreground">
              当前页筛选结果：{visibleDocs.length} / {sortedDocs.length}
            </div>
          )}
          {isDragging && (
            <div className="flex items-center justify-center py-12 border-2 border-dashed border-primary rounded-lg mb-4">
              <span className="text-muted-foreground">{t('documents.dragDropHint')}</span>
            </div>
          )}
          {visibleDocs.length === 0 ? (
            <div className="text-center py-8 text-muted-foreground">
              {bindingFilter === 'all'
                ? t('documents.empty')
                : bindingUsageLoading
                  ? '正在按分身占用状态筛选...'
                  : '当前筛选条件下没有文档'}
            </div>
          ) : (
            <div className="space-y-2">
              {canManage && selectionMode && visibleDocs.length > 0 && (
                <div className="flex items-center gap-2 px-3 py-1">
                  <input
                    type="checkbox"
                    checked={allVisibleSelected}
                    onChange={toggleSelectAll}
                    aria-label="Select all"
                    className="h-4 w-4"
                  />
                </div>
              )}
              {visibleDocs.map((doc) => (
                <div
                  key={doc.id}
                  className={`flex items-center gap-3 p-3 border rounded-lg cursor-pointer hover:bg-muted/50 ${
                    panel.doc?.id === doc.id ? 'bg-muted/50 border-primary/50' : ''
                  }`}
                  onClick={() => handleDocClick(doc)}
                >
                  {canManage && selectionMode && (
                    <input
                      type="checkbox"
                      checked={selectedIds.has(doc.id)}
                      onChange={(e) => { e.stopPropagation(); toggleSelect(doc.id); }}
                      onClick={(e) => e.stopPropagation()}
                      aria-label={`Select ${doc.display_name || doc.name}`}
                      className="h-4 w-4 flex-shrink-0"
                    />
                  )}
                  {doc.mime_type.startsWith('image/') ? (
                    <img
                      src={documentApi.getDownloadUrl(teamId, doc.id)}
                      alt=""
                      className="w-8 h-8 rounded object-cover flex-shrink-0"
                    />
                  ) : (
                    <span className="text-2xl">{getFileIcon(doc.mime_type)}</span>
                  )}
                  <div className="flex-1 min-w-0">
                    <p className="font-medium truncate">
                      {doc.display_name || doc.name}
                      {doc.is_public && <span className="ml-1 text-xs text-blue-500" title="Public">🌐</span>}
                    </p>
                    <p className="text-xs text-muted-foreground">
                      {formatFileSize(doc.file_size)} · {formatDate(doc.created_at)}
                    </p>
                    {renderBindingChips(bindingUsageByDocId[doc.id])}
                    {doc.tags && doc.tags.length > 0 && (
                      <div className="flex flex-wrap gap-1 mt-0.5">
                        {doc.tags.slice(0, 3).map(tag => (
                          <span
                            key={tag}
                            className="text-micro px-1.5 py-px rounded-full bg-primary/10 text-primary cursor-pointer hover:bg-primary/20"
                            onClick={(e) => { e.stopPropagation(); setTagFilter(tag); }}
                          >
                            {tag}
                          </span>
                        ))}
                        {doc.tags.length > 3 && (
                          <span className="text-micro text-muted-foreground">+{doc.tags.length - 3}</span>
                        )}
                      </div>
                    )}
                  </div>
                  {!isMobile && (
                    <div className="flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
                      {canManage && (
                        <Button size="sm" variant="ghost" onClick={() => openEditMeta(doc)} title={t('documents.editInfo')}>
                          ✏️
                        </Button>
                      )}
                      <Button size="sm" variant="ghost" onClick={() => handleDownload(doc.id)}>
                        {t('documents.download')}
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => { setViewMode('lineage'); setLineageDocId(doc.id); }}>
                        {t('documents.lineage')}
                      </Button>
                      {canManage && (
                        <Button
                          size="sm"
                          variant="ghost"
                          className="text-destructive"
                          onClick={() => handleDeleteDocument(doc.id)}
                        >
                          {t('common.delete')}
                        </Button>
                      )}
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </CardContent>
        {pagination.totalPages > 1 && (
          <div className={`flex items-center justify-between ${isMobile ? 'px-3' : 'px-6'} py-3 border-t`}>
            {!isMobile && (
              <span className="text-sm text-muted-foreground">
                {bindingFilter === 'all'
                  ? `${t('common.total')}: ${pagination.total}`
                  : `当前页可见: ${visibleDocs.length} / ${sortedDocs.length}`}
              </span>
            )}
            <div className="flex items-center gap-2">
              <Button size="sm" variant="outline" disabled={pagination.page <= 1} onClick={() => setPagination(p => ({ ...p, page: p.page - 1 }))}>
                {t('pagination.previous')}
              </Button>
              <span className="text-sm">{pagination.page} / {pagination.totalPages}</span>
              <Button size="sm" variant="outline" disabled={pagination.page >= pagination.totalPages} onClick={() => setPagination(p => ({ ...p, page: p.page + 1 }))}>
                {t('pagination.next')}
              </Button>
            </div>
          </div>
        )}
      </Card>

      {/* Right Panel: Preview / Edit / Versions / Diff */}
      {hasRightPanel && panel.doc && (
        <Card className={isMobile ? 'fixed inset-0 z-50' : 'w-[45%] min-w-[320px] relative'}>
          {panel.mode === 'preview' && (
            <div className="flex h-full flex-col">
              <div className="min-h-0 flex-1">
                <Suspense fallback={<DocumentPreviewLoading />}>
                  <DocumentPreview
                    teamId={teamId}
                    document={panel.doc}
                    onClose={handleClosePanel}
                    onEdit={handleEdit}
                    onVersions={handleVersions}
                  />
                </Suspense>
              </div>
              {renderBindingUsageDetail(panel.doc)}
            </div>
          )}
          {panel.mode === 'edit' && panel.editLock && (
            <DocumentEditor
              teamId={teamId}
              document={panel.doc}
              initialContent={panel.editContent}
              lock={panel.editLock}
              onSave={handleEditSave}
              onClose={handleEditClose}
            />
          )}
          {panel.mode === 'versions' && (
            <VersionTimeline
              teamId={teamId}
              docId={panel.doc.id}
              canManage={canManage}
              onCompare={handleCompare}
              onRollback={handleRollback}
            />
          )}
          {panel.mode === 'diff' && panel.diffVersions && (
            <VersionDiff
              teamId={teamId}
              docId={panel.doc.id}
              version1={panel.diffVersions[0]}
              version2={panel.diffVersions[1]}
              onClose={() => setPanel(prev => ({ ...prev, mode: 'versions' }))}
            />
          )}
        </Card>
      )}

      </div>
      )}

      {/* Create Folder Dialog */}
      <Dialog open={folderDialog.open} onOpenChange={(open) => { if (!open) setFolderDialog(INITIAL_FOLDER_DIALOG); }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('documents.createFolder')}</DialogTitle>
          </DialogHeader>
          <div className="space-y-4 py-4">
            <div>
              <label className="text-sm font-medium">{t('documents.folderName')}</label>
              <Input
                value={folderDialog.name}
                onChange={(e) => setFolderDialog(prev => ({ ...prev, name: e.target.value }))}
                placeholder={t('documents.folderNamePlaceholder')}
              />
            </div>
            <div>
              <label className="text-sm font-medium">{t('documents.folderDescription')}</label>
              <Input
                value={folderDialog.desc}
                onChange={(e) => setFolderDialog(prev => ({ ...prev, desc: e.target.value }))}
                placeholder={t('documents.folderDescPlaceholder')}
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setFolderDialog(INITIAL_FOLDER_DIALOG)}>
              {t('common.cancel')}
            </Button>
            <Button onClick={handleCreateFolder} disabled={!folderDialog.name.trim()}>
              {t('common.create')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Rename Folder Dialog */}
      <Dialog open={renameFolder.open} onOpenChange={(open) => { if (!open) setRenameFolder(INITIAL_RENAME); }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('documents.renameFolder')}</DialogTitle>
          </DialogHeader>
          <div className="py-4">
            <label className="text-sm font-medium">{t('documents.folderName')}</label>
            <Input
              value={renameFolder.name}
              onChange={(e) => setRenameFolder(prev => ({ ...prev, name: e.target.value }))}
              placeholder={t('documents.folderNamePlaceholder')}
            />
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRenameFolder(INITIAL_RENAME)}>
              {t('common.cancel')}
            </Button>
            <Button onClick={handleRenameFolder} disabled={!renameFolder.name.trim()}>
              {t('common.save')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Edit Document Metadata Dialog */}
      <Dialog open={editMeta.open} onOpenChange={(open) => { if (!open) setEditMeta(INITIAL_EDIT_META); }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('documents.editInfo')}</DialogTitle>
          </DialogHeader>
          <div className="space-y-4 py-4">
            <div>
              <label className="text-sm font-medium">{t('documents.displayName')}</label>
              <Input
                value={editMeta.displayName}
                onChange={(e) => setEditMeta(prev => ({ ...prev, displayName: e.target.value }))}
              />
            </div>
            <div>
              <label className="text-sm font-medium">{t('documents.description')}</label>
              <Input
                value={editMeta.description}
                onChange={(e) => setEditMeta(prev => ({ ...prev, description: e.target.value }))}
              />
            </div>
            <div>
              <label className="text-sm font-medium">{t('documents.tags')}</label>
              <div className="flex flex-wrap gap-1 mb-1.5">
                {editMeta.tags.map(tag => (
                  <span key={tag} className="inline-flex items-center gap-0.5 text-xs px-2 py-0.5 rounded-full bg-primary/10 text-primary">
                    {tag}
                    <button type="button" className="hover:text-destructive" onClick={() => removeTagFromEdit(tag)}>&times;</button>
                  </span>
                ))}
              </div>
              <div className="relative">
                <Input
                  value={editMeta.tagInput}
                  onChange={(e) => setEditMeta(prev => ({ ...prev, tagInput: e.target.value }))}
                  onKeyDown={(e) => {
                    if ((e.key === 'Enter' || e.key === ',') && editMeta.tagInput.trim()) {
                      e.preventDefault();
                      addTagToEdit(editMeta.tagInput);
                    }
                    if (e.key === 'Backspace' && !editMeta.tagInput && editMeta.tags.length > 0) {
                      removeTagFromEdit(editMeta.tags[editMeta.tags.length - 1]);
                    }
                  }}
                  onBlur={() => { if (editMeta.tagInput.trim()) addTagToEdit(editMeta.tagInput); }}
                  placeholder={t('documents.addTag')}
                />
                {tagSuggestions.length > 0 && (
                  <div className="absolute z-10 w-full mt-1 bg-popover border rounded-md shadow-md">
                    {tagSuggestions.map(tag => (
                      <button
                        key={tag}
                        type="button"
                        className="w-full text-left px-3 py-1.5 text-sm hover:bg-muted"
                        onMouseDown={(e) => { e.preventDefault(); addTagToEdit(tag); }}
                      >
                        {tag}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditMeta(INITIAL_EDIT_META)}>
              {t('common.cancel')}
            </Button>
            <Button onClick={handleSaveMeta} disabled={editMeta.saving}>
              {editMeta.saving && <Loader2 className="w-4 h-4 animate-spin mr-1.5" />}
              {t('common.save')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Upload Progress */}
      {uploadProgress.size > 0 && (
        <div className="fixed bottom-4 right-4 w-80 bg-background border rounded-lg shadow-lg p-3 space-y-2 z-50">
          <p className="text-sm font-medium">{t('documents.uploadProgress')}</p>
          {Array.from(uploadProgress.entries()).map(([key, item]) => (
            <div key={key} className="space-y-1">
              <div className="flex items-center justify-between text-xs">
                <span className="truncate flex-1">{item.name}</span>
                <span>{item.error ? '❌' : item.done ? '✅' : `${item.progress}%`}</span>
              </div>
              <div className="h-1.5 bg-muted rounded-full overflow-hidden">
                <div
                  className={`h-full rounded-full transition-all ${item.error ? 'bg-destructive' : 'bg-primary'}`}
                  style={{ width: `${item.progress}%` }}
                />
              </div>
            </div>
          ))}
        </div>
      )}
      <ConfirmDialog
        open={!!deleteDocTarget}
        onOpenChange={(open) => { if (!open) setDeleteDocTarget(null); }}
        title={t('documents.deleteConfirm')}
        variant="destructive"
        onConfirm={confirmDeleteDocument}
      />
      <ConfirmDialog
        open={showBatchDeleteConfirm}
        onOpenChange={setShowBatchDeleteConfirm}
        title={t('documents.batchDeleteConfirm', { count: selectedIds.size })}
        variant="destructive"
        onConfirm={confirmBatchDelete}
      />
      <ConfirmDialog
        open={!!deleteFolderTarget}
        onOpenChange={(open) => { if (!open) setDeleteFolderTarget(null); }}
        title={t('documents.deleteFolderConfirm')}
        variant="destructive"
        onConfirm={confirmDeleteFolder}
      />
    </div>
  );
}
