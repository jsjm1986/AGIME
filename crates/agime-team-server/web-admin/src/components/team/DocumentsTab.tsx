import { useState, useEffect, useRef, useCallback, useMemo, lazy, Suspense } from 'react';
import { useTranslation } from 'react-i18next';
import { Loader2, FolderOpen, CheckSquare, X, Download, MessageSquareText, SlidersHorizontal, LayoutGrid } from 'lucide-react';
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
import { DocumentFolderNavigator } from '../documents/DocumentFolderNavigator';
import { SupportedFormatsGuide } from '../documents/SupportedFormatsGuide';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { useIsMobile } from '../../hooks/useMediaQuery';
import { useToast } from '../../contexts/ToastContext';
import { formatDate, formatDateTime } from '../../utils/format';
import { useMobileInteractionMode } from '../../contexts/MobileInteractionModeContext';
import { ContextSummaryBar } from '../mobile/ContextSummaryBar';
import { BottomSheetPanel } from '../mobile/BottomSheetPanel';
import { ManagementRail } from '../mobile/ManagementRail';
import { MobileWorkspaceShell } from '../mobile/MobileWorkspaceShell';

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
    return 'border-[hsl(var(--status-error-text))/0.16] bg-[hsl(var(--status-error-bg))] text-[hsl(var(--status-error-text))]';
  }
  if (tone === 'draft') {
    return 'border-[hsl(var(--status-warning-text))/0.16] bg-[hsl(var(--status-warning-bg))] text-[hsl(var(--status-warning-text))]';
  }
  return 'border-[hsl(var(--status-info-text))/0.16] bg-[hsl(var(--status-info-bg))] text-[hsl(var(--status-info-text))]';
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
  parentPath: string | null;
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

const INITIAL_FOLDER_DIALOG: FolderDialogState = { open: false, name: '', desc: '', parentPath: null };
const INITIAL_RENAME: RenameFolderState = { open: false, target: null, name: '' };
const INITIAL_EDIT_META: EditMetaState = { open: false, doc: null, displayName: '', description: '', tags: [], tagInput: '', saving: false };
const INITIAL_PANEL: RightPanelState = { doc: null, mode: null, editContent: '', editLock: null, diffVersions: null };

function readStoredBoolean(key: string, fallback: boolean): boolean {
  if (typeof window === 'undefined') {
    return fallback;
  }
  const value = window.localStorage.getItem(key);
  if (value === null) {
    return fallback;
  }
  return value === '1';
}

function readStoredFolderPath(key: string): string | null {
  if (typeof window === 'undefined') {
    return null;
  }
  const value = window.localStorage.getItem(key);
  return value && value.trim().length > 0 ? value : null;
}

function folderPathExists(nodes: FolderTreeNode[], targetPath: string | null): boolean {
  if (!targetPath || targetPath === '/') {
    return true;
  }

  return nodes.some((node) => {
    if (node.fullPath === targetPath) {
      return true;
    }
    if (node.children.length === 0) {
      return false;
    }
    return folderPathExists(node.children, targetPath);
  });
}

function findFolderNode(nodes: FolderTreeNode[], targetPath: string | null): FolderTreeNode | null {
  if (!targetPath || targetPath === '/') {
    return null;
  }

  for (const node of nodes) {
    if (node.fullPath === targetPath) {
      return node;
    }
    if (node.children.length > 0) {
      const nested = findFolderNode(node.children, targetPath);
      if (nested) {
        return nested;
      }
    }
  }

  return null;
}

interface DocumentsTabProps {
  teamId: string;
  canManage: boolean;
}

export function DocumentsTab({ teamId, canManage }: DocumentsTabProps) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const { isConversationMode, isMobileWorkspace } = useMobileInteractionMode();
  const { addToast } = useToast();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const navigate = useNavigate();
  const pageSize = isMobile ? 12 : 20;
  const isConversationTaskMode = isConversationMode && isMobileWorkspace;
  const folderTreeVisibleStorageKey = `agime.documents.${teamId}.folderTreeVisible`;
  const folderTreeExpandedStorageKey = `agime.documents.${teamId}.folderTreeExpanded`;
  const recentFolderStorageKey = `agime.documents.${teamId}.recentFolder`;

  // Core data
  const [folders, setFolders] = useState<FolderTreeNode[]>([]);
  const [documents, setDocuments] = useState<DocumentSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [currentFolderPath, setCurrentFolderPath] = useState<string | null>(() => readStoredFolderPath(recentFolderStorageKey));
  const [searchQuery, setSearchQuery] = useState('');
  const [debouncedSearch, setDebouncedSearch] = useState('');
  const [mimeFilter, setMimeFilter] = useState('');
  const [tagFilter, setTagFilter] = useState('');
  const [bindingFilter, setBindingFilter] = useState<BindingFilterMode>('all');
  const [teamTags, setTeamTags] = useState<{ tag: string; count: number }[]>([]);
  const [sortBy, setSortBy] = useState<'date' | 'name' | 'size'>('date');
  const [pagination, setPagination] = useState<PaginationState>({ page: 1, total: 0, totalPages: 0 });

  // UI toggles
  const [showFolderTree, setShowFolderTree] = useState<boolean>(() => readStoredBoolean(folderTreeVisibleStorageKey, true));
  const [uploading, setUploading] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [selectionMode, setSelectionMode] = useState(false);
  const [viewMode, setViewMode] = useState<ViewMode>('folders');
  const [mobileFolderSheetOpen, setMobileFolderSheetOpen] = useState(false);
  const [mobileLibrarySheetOpen, setMobileLibrarySheetOpen] = useState(false);
  const [mobileViewSheetOpen, setMobileViewSheetOpen] = useState(false);
  const [mobileFilterSheetOpen, setMobileFilterSheetOpen] = useState(false);
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
          ? documentApi.searchDocuments(teamId, debouncedSearch, pagination.page, pageSize, mimeFilter || undefined, currentFolderPath || undefined, tagFilter || undefined)
          : documentApi.listDocuments(teamId, pagination.page, pageSize, currentFolderPath || undefined, mimeFilter || undefined, tagFilter || undefined),
      ]);
      setFolders(foldersRes);
      setDocuments(docsRes.items);
      setPagination(prev => ({ ...prev, total: docsRes.total, totalPages: docsRes.total_pages }));
    } catch (error) {
      console.error('Failed to load documents:', error);
    } finally {
      setLoading(false);
    }
  }, [teamId, currentFolderPath, debouncedSearch, pagination.page, pageSize, mimeFilter, tagFilter]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  useEffect(() => {
    setShowFolderTree(readStoredBoolean(folderTreeVisibleStorageKey, true));
    setCurrentFolderPath(readStoredFolderPath(recentFolderStorageKey));
  }, [folderTreeVisibleStorageKey, recentFolderStorageKey]);

  useEffect(() => {
    if (typeof window === 'undefined') {
      return;
    }
    window.localStorage.setItem(folderTreeVisibleStorageKey, showFolderTree ? '1' : '0');
  }, [folderTreeVisibleStorageKey, showFolderTree]);

  useEffect(() => {
    if (typeof window === 'undefined') {
      return;
    }
    if (!currentFolderPath) {
      window.localStorage.removeItem(recentFolderStorageKey);
      return;
    }
    window.localStorage.setItem(recentFolderStorageKey, currentFolderPath);
  }, [currentFolderPath, recentFolderStorageKey]);

  // Debounce search input — only fire API after 300ms idle
  useEffect(() => {
    const timer = setTimeout(() => setDebouncedSearch(searchQuery), 300);
    return () => clearTimeout(timer);
  }, [searchQuery]);

  useEffect(() => {
    setPagination(prev => ({ ...prev, page: 1 }));
  }, [debouncedSearch, currentFolderPath, mimeFilter, tagFilter]);

  useEffect(() => {
    if (loading) {
      return;
    }
    if (!folderPathExists(folders, currentFolderPath)) {
      setCurrentFolderPath(null);
    }
  }, [folders, currentFolderPath, loading]);

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
      const res = await documentApi.listArchived(teamId, archivedPage, pageSize);
      setArchivedDocuments(res.items);
      setArchivedTotalPages(res.total_pages);
    } catch (error) {
      console.error('Failed to load archived documents:', error);
    } finally {
      setArchivedLoading(false);
    }
  }, [teamId, archivedPage, pageSize]);

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
        parentPath: folderDialog.parentPath || currentFolderPath || '/',
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

  const openCreateFolder = useCallback((parentPath?: string | null) => {
    setFolderDialog({
      open: true,
      name: '',
      desc: '',
      parentPath: parentPath ?? currentFolderPath ?? '/',
    });
  }, [currentFolderPath]);

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

  const currentFolderNode = useMemo(
    () => findFolderNode(folders, currentFolderPath),
    [folders, currentFolderPath],
  );

  const visibleChildFolders = useMemo(() => {
    if (!currentFolderPath) {
      return folders;
    }
    return currentFolderNode?.children ?? [];
  }, [currentFolderNode, currentFolderPath, folders]);

  const handleFolderSelect = useCallback((path: string | null) => {
    setCurrentFolderPath(path);
    setPagination((prev) => (prev.page === 1 ? prev : { ...prev, page: 1 }));
    if (viewMode !== 'folders') {
      setViewMode('folders');
      setLineageDocId(null);
    }
    if (isMobile) {
      setMobileFolderSheetOpen(false);
      setMobileLibrarySheetOpen(false);
      setMobileViewSheetOpen(false);
      setMobileFilterSheetOpen(false);
    }
  }, [isMobile, viewMode]);

  const renderChildFolderButtons = useCallback((compact = false) => {
    if (visibleChildFolders.length === 0) {
      return null;
    }

    return (
      <div className="space-y-3">
        <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
          {t('documents.childFolders', '子目录')}
        </div>
        <div className="space-y-2">
          {visibleChildFolders.map((folder) => (
            <button
              key={folder.id}
              type="button"
              className={`flex w-full items-center gap-3 rounded-[16px] border border-border/60 bg-background/88 ${compact ? 'px-3 py-2.5' : 'px-3.5 py-3'} text-left transition-colors hover:border-primary/25 hover:bg-primary/5`}
              onClick={() => handleFolderSelect(folder.fullPath)}
            >
              <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-[hsl(var(--ui-surface-panel-muted))/0.42] text-muted-foreground">
                <FolderOpen className="h-4 w-4" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[12px] font-medium text-foreground">{folder.name}</span>
                <span className="block truncate text-[10px] text-muted-foreground">{folder.fullPath}</span>
              </span>
            </button>
          ))}
        </div>
      </div>
    );
  }, [handleFolderSelect, t, visibleChildFolders]);

  if (loading) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  const hasRightPanel = panel.doc && panel.mode;
  const showDocumentPagination = viewMode === 'folders' && pagination.totalPages > 1;

  const renderFolderNavigator = (variant: 'desktop' | 'mobile') => (
    <DocumentFolderNavigator
      nodes={folders}
      currentPath={currentFolderPath}
      onSelectPath={handleFolderSelect}
      canManage={canManage}
      onCreateFolder={openCreateFolder}
      onRenameFolder={openRenameFolder}
      onDeleteFolder={handleDeleteFolder}
      storageKey={folderTreeExpandedStorageKey}
      variant={variant}
      embedded={variant === 'desktop'}
      className="min-h-0"
    />
  );

  const openMobileFolderPanel = () => {
    setMobileLibrarySheetOpen(false);
    setMobileViewSheetOpen(false);
    setMobileFilterSheetOpen(false);
    setMobileFolderSheetOpen(true);
  };

  const openMobileLibraryPanel = () => {
    setMobileFolderSheetOpen(false);
    setMobileViewSheetOpen(false);
    setMobileFilterSheetOpen(false);
    setMobileLibrarySheetOpen(true);
  };

  const openMobileViewPanel = () => {
    setMobileFolderSheetOpen(false);
    setMobileLibrarySheetOpen(false);
    setMobileFilterSheetOpen(false);
    setMobileViewSheetOpen(true);
  };

  const openMobileFilterPanel = () => {
    setMobileFolderSheetOpen(false);
    setMobileLibrarySheetOpen(false);
    setMobileViewSheetOpen(false);
    setMobileFilterSheetOpen(true);
  };

  const classicMobileFolderSummary = isMobile ? (
    <div className="mt-2 flex items-center gap-2 rounded-[18px] border border-border/60 bg-[hsl(var(--ui-surface-panel-muted))/0.24] px-3 py-2.5">
      <Button
        size="sm"
        variant="outline"
        className="h-8 shrink-0 rounded-[12px] text-[11px]"
        onClick={openMobileFolderPanel}
      >
        <FolderOpen className="mr-1.5 h-3.5 w-3.5" />
        {t('documents.openFolderNavigator', '文件夹')}
      </Button>
      <div className="min-w-0 flex-1">
        <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
          {t('documents.currentFolder', '当前目录')}
        </div>
        <div className="truncate text-[12px] font-semibold text-foreground">{currentFolderPath || '/'}</div>
      </div>
    </div>
  ) : null;
  const isDesktopFoldersLayout = viewMode === 'folders' && !isMobile;

  const renderDocumentPagination = (compact = false) => (
    <div className={`flex items-center ${compact ? 'gap-2' : 'justify-between gap-3'} rounded-md border bg-muted/20 px-3 py-2`}>
      {!compact && (
        <div className="min-w-0">
          <p className="text-xs font-medium text-foreground">
            {t('documents.pageIndicator', '第 {{page}} / {{totalPages}} 页', {
              page: pagination.page,
              totalPages: pagination.totalPages,
            })}
          </p>
          <p className="mt-0.5 text-[11px] text-muted-foreground">
            {t('documents.pageSummary', '共 {{total}} 条，每页 {{count}} 条', {
              total: pagination.total,
              count: pageSize,
            })}
          </p>
        </div>
      )}
      <div className="ml-auto flex items-center gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={pagination.page <= 1}
          onClick={() => setPagination((p) => ({ ...p, page: p.page - 1 }))}
        >
          {t('pagination.previous')}
        </Button>
        <span className="min-w-[4.5rem] text-center text-sm text-muted-foreground">
          {pagination.page} / {pagination.totalPages}
        </span>
        <Button
          size="sm"
          variant="outline"
          disabled={pagination.page >= pagination.totalPages}
          onClick={() => setPagination((p) => ({ ...p, page: p.page + 1 }))}
        >
          {t('pagination.next')}
        </Button>
      </div>
    </div>
  );

  const handleSelectViewMode = (mode: ViewMode) => {
    setViewMode(mode);
    setLineageDocId(null);
    if (mode === 'trash') {
      setArchivedPage(1);
    }
    setMobileViewSheetOpen(false);
  };

  const viewModeSwitcher = (
    <div className="flex flex-wrap items-center gap-2">
      {(['folders', 'aiWorkbench', 'lineage', 'trash'] as ViewMode[]).map((mode) => (
        <button
          key={mode}
          onClick={() => handleSelectViewMode(mode)}
          className={`rounded-full px-3 py-1.5 text-sm transition-colors ${
            viewMode === mode
              ? 'bg-primary text-primary-foreground'
              : 'bg-[hsl(var(--ui-surface-panel-muted))/0.52] text-muted-foreground hover:bg-muted'
          }`}
        >
          {t(`documents.viewMode.${mode}`)}
        </button>
      ))}
    </div>
  );

  const mobileFilterPanel = (
    <div className="space-y-4">
      <Input
        placeholder={t('documents.search')}
        value={searchQuery}
        onChange={(e) => setSearchQuery(e.target.value)}
        className="h-10"
      />
      <Select value={bindingFilter} onValueChange={(value) => setBindingFilter(value as BindingFilterMode)}>
        <SelectTrigger className="h-10">
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
      <Select value={mimeFilter || '__all__'} onValueChange={v => setMimeFilter(v === '__all__' ? '' : v)}>
        <SelectTrigger className="h-10">
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
          <SelectTrigger className="h-10">
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
        <SelectTrigger className="h-10">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="date">{t('documents.sortDate')}</SelectItem>
          <SelectItem value="name">{t('documents.sortName')}</SelectItem>
          <SelectItem value="size">{t('documents.sortSize')}</SelectItem>
        </SelectContent>
      </Select>
      <div className="flex items-center justify-between rounded-[18px] border border-border/70 bg-[hsl(var(--ui-surface-panel-muted))/0.38] px-3 py-2.5">
        <div className="min-w-0">
          <div className="text-sm font-medium">{t('documents.supportedFormats', '支持格式')}</div>
          <div className="text-xs text-muted-foreground">
            {t('documents.supportedFormatsHint', '查看当前文档工作台允许上传与预览的类型。')}
          </div>
        </div>
        <SupportedFormatsGuide />
      </div>
      <div className="flex gap-2">
        <Button variant="outline" className="flex-1" onClick={() => setMobileFilterSheetOpen(false)}>
          {t('common.confirm', '确认')}
        </Button>
        <Button
          variant="ghost"
          className="flex-1"
          onClick={() => {
            setMimeFilter('');
            setTagFilter('');
            setBindingFilter('all');
            setSortBy('date');
          }}
        >
          {t('common.reset', '重置')}
        </Button>
      </div>
    </div>
  );

  const mobileLibraryContent = (
    <>
      {viewMode === 'aiWorkbench' ? (
        <div className="min-h-[360px]">
          <AiWorkbench teamId={teamId} canManage={canManage} />
        </div>
      ) : viewMode === 'lineage' ? (
        lineageDocId ? (
          <div className="min-h-[360px]">
            <DocumentLineage
              teamId={teamId}
              docId={lineageDocId}
              onNavigate={(id) => setLineageDocId(id)}
            />
          </div>
        ) : (
          <div className="space-y-4">
            <div className="rounded-[16px] border border-border/60 bg-background/86 px-3.5 py-3">
              <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
                {t('documents.viewMode.lineage')}
              </div>
              <p className="mt-1 text-[11px] leading-4.5 text-muted-foreground">
                {t('documents.mobileLineageHint', '先选一份文档，再查看它的来源、引用链路和下游结果。')}
              </p>
            </div>
            <div className="space-y-2">
              {visibleDocs.length > 0 ? visibleDocs.slice(0, 12).map((doc) => (
                <button
                  key={doc.id}
                  type="button"
                  className="w-full rounded-[16px] border border-border/65 bg-background/88 px-3.5 py-3 text-left transition-colors hover:bg-accent/20"
                  onClick={() => setLineageDocId(doc.id)}
                >
                  <div className="flex items-start gap-3">
                    <span className="text-base leading-none">{getFileIcon(doc.mime_type)}</span>
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-[12px] font-semibold text-foreground">
                        {doc.display_name || doc.name}
                      </div>
                      <div className="mt-1 text-[11px] leading-4.5 text-muted-foreground">
                        {doc.folder_path || '/'}
                      </div>
                    </div>
                  </div>
                </button>
              )) : (
                <div className="rounded-[16px] border border-dashed border-border/70 px-3.5 py-4 text-[11px] text-muted-foreground">
                  {t('documents.noAiDocuments')}
                </div>
              )}
            </div>
          </div>
        )
      ) : viewMode === 'trash' ? (
        <div className="space-y-3">
          {archivedLoading ? (
            <div className="rounded-[16px] border border-border/60 bg-background/86 px-3.5 py-5 text-center text-[11px] text-muted-foreground">
              {t('common.loading')}
            </div>
          ) : archivedDocuments.length === 0 ? (
            <div className="rounded-[16px] border border-dashed border-border/70 px-3.5 py-4 text-[11px] text-muted-foreground">
              {t('documents.noArchivedDocuments', '暂无归档文档')}
            </div>
          ) : (
            archivedDocuments.map((doc) => (
              <div key={doc.id} className="rounded-[16px] border border-border/65 bg-background/88 px-3.5 py-3">
                <div className="flex items-start gap-3">
                  <span className="text-base leading-none">{getFileIcon(doc.mime_type)}</span>
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[12px] font-semibold text-foreground">
                      {doc.display_name || doc.name}
                    </div>
                    <div className="mt-1 text-[11px] leading-4.5 text-muted-foreground">
                      {formatFileSize(doc.file_size)} · {formatDateTime(doc.updated_at || doc.created_at)}
                    </div>
                  </div>
                  {canManage && (
                    <Button size="sm" variant="outline" className="h-8 rounded-[12px] text-[11px]" onClick={() => handleRestoreDocument(doc.id)}>
                      {t('documents.restore')}
                    </Button>
                  )}
                </div>
              </div>
            ))
          )}
          {archivedTotalPages > 1 && (
            <div className="flex items-center justify-center gap-2 pt-1">
              <Button size="sm" variant="outline" disabled={archivedPage <= 1} onClick={() => setArchivedPage(p => p - 1)}>
                {t('pagination.previous')}
              </Button>
              <span className="text-[11px] text-muted-foreground">{archivedPage} / {archivedTotalPages}</span>
              <Button size="sm" variant="outline" disabled={archivedPage >= archivedTotalPages} onClick={() => setArchivedPage(p => p + 1)}>
                {t('pagination.next')}
              </Button>
            </div>
          )}
        </div>
      ) : (
        <div className="space-y-4">
          <div className="space-y-3 rounded-[18px] border border-border/65 bg-background/86 px-3.5 py-3.5">
            <div className="flex items-center justify-between gap-3">
              <div>
                <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
                  {t('documents.currentFolder', '当前目录')}
                </div>
                <div className="mt-1 text-[12px] font-semibold text-foreground">
                  {currentFolderPath || '/'}
                </div>
              </div>
              <Button
                size="sm"
                variant="outline"
                className="h-8 rounded-[12px] px-3 text-[11px]"
                onClick={() => {
                  setMobileLibrarySheetOpen(false);
                  openMobileFilterPanel();
                }}
              >
                <SlidersHorizontal className="mr-1.5 h-3.5 w-3.5" />
                {t('documents.quickFilters', '筛选与排序')}
              </Button>
            </div>
            <p className="text-[11px] leading-4.5 text-muted-foreground">
              {t('documents.mobileFolderNavigatorHint', '使用折叠文件夹树快速切换目录，文档列表会跟随当前路径更新。')}
            </p>
          </div>
          {renderFolderNavigator('mobile')}
          <div className="rounded-[18px] border border-border/65 bg-background/86 px-3.5 py-3.5">
            <div className="flex items-center justify-between gap-3">
              <div>
                <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
                  {t('documents.files', '文件')}
                </div>
                <div className="mt-1 text-[12px] font-semibold text-foreground">
                  {t('documents.folderResultCount', '当前目录 {{count}} 份文档', { count: visibleDocs.length })}
                </div>
              </div>
            </div>
          </div>
          <div className="space-y-2">
            {visibleDocs.length > 0 ? visibleDocs.map((doc) => (
              <button
                key={doc.id}
                type="button"
                className={`w-full rounded-[16px] border px-3.5 py-3 text-left transition-colors ${panel.doc?.id === doc.id ? 'border-primary/35 bg-primary/6' : 'border-border/65 bg-background/88 hover:bg-accent/20'}`}
                onClick={() => {
                  handleDocClick(doc);
                  setMobileLibrarySheetOpen(false);
                }}
              >
                <div className="flex items-start gap-3">
                  <span className="text-base leading-none">{getFileIcon(doc.mime_type)}</span>
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[12px] font-semibold text-foreground">
                      {doc.display_name || doc.name}
                    </div>
                    <div className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] leading-4.5 text-muted-foreground">
                      <span>{doc.folder_path || '/'}</span>
                      <span>·</span>
                      <span>{formatFileSize(doc.file_size)}</span>
                      <span>·</span>
                      <span>{t(`documents.status.${doc.status}`, doc.status)}</span>
                    </div>
                  </div>
                </div>
              </button>
            )) : (
              <div className="space-y-3 rounded-[16px] border border-dashed border-border/70 bg-background/72 px-3.5 py-4">
                <div className="text-[11px] text-muted-foreground">
                  {visibleChildFolders.length > 0
                    ? t('documents.emptyCurrentFolderWithChildren', '当前目录没有直属文件，请继续进入下一级目录。')
                    : t('documents.empty', '当前条件下没有文档')}
                </div>
                {renderChildFolderButtons(true)}
              </div>
            )}
          </div>
          {showDocumentPagination && renderDocumentPagination(true)}
        </div>
      )}
    </>
  );

  const documentsContent = (
    <div className={`flex flex-col ${isConversationTaskMode ? 'min-h-0' : 'h-[calc(100vh-40px)]'}`}>
      {!isConversationTaskMode && (
        <div className="mb-3 flex items-center gap-1 border-b pb-2">
          {viewModeSwitcher}
        </div>
      )}

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
      <div className={`flex flex-1 min-h-0 ${isMobile ? 'flex-col gap-4' : 'overflow-hidden rounded-[28px] border border-border/65 bg-[linear-gradient(180deg,hsl(var(--ui-surface-panel))/0.99_0%,hsl(var(--background))/0.985_100%)] shadow-[0_22px_56px_-42px_hsl(var(--foreground)/0.25)]'}`}>
      {!isMobile && showFolderTree ? (
        <div className="w-[280px] flex-shrink-0 min-h-0 border-r border-border/55 bg-[hsl(var(--ui-surface-panel-muted))/0.22] px-3 py-3">
          {renderFolderNavigator('desktop')}
        </div>
      ) : null}

      {/* Document List */}
      <Card
        className={`flex min-h-0 flex-1 min-w-0 flex-col ${isDesktopFoldersLayout ? 'rounded-none border-0 bg-transparent shadow-none' : ''} ${isDragging ? 'ring-2 ring-primary ring-dashed' : ''}`}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
      >
        <CardHeader className={`shrink-0 py-3 ${isDesktopFoldersLayout ? 'space-y-3 border-b border-border/55 px-5' : ''}`}>
          <div className={`flex ${isDesktopFoldersLayout ? 'items-start justify-between gap-3' : 'items-center justify-between'}`}>
            <div className="flex min-w-0 items-center gap-2">
              <CardTitle className="text-sm">{t('documents.files')}</CardTitle>
              {!isMobile ? (
                <Button
                  size="sm"
                  variant="outline"
                  className="h-8"
                  onClick={() => setShowFolderTree((prev) => !prev)}
                >
                  <FolderOpen className="mr-1.5 h-3.5 w-3.5" />
                  {showFolderTree
                    ? t('documents.hideFolderNavigator', '收起文件夹')
                    : t('documents.showFolderNavigator', '显示文件夹')}
                </Button>
              ) : null}
            </div>
            <div className={`flex items-center gap-2 flex-wrap ${isDesktopFoldersLayout ? 'justify-end' : ''}`}>
              <Input
                placeholder={t('documents.search')}
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className={`${isMobile ? 'flex-1 min-w-0' : isDesktopFoldersLayout ? 'w-52' : 'w-36'} h-8`}
              />
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
          <div className={`flex items-center gap-2 flex-wrap ${isDesktopFoldersLayout ? 'rounded-[16px] bg-[hsl(var(--ui-surface-panel-muted))/0.18] px-3 py-2.5' : ''}`}>
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
            </div>
          {classicMobileFolderSummary}
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
          {showDocumentPagination && (
            <div className="mt-2">
              {renderDocumentPagination()}
            </div>
          )}
        </CardHeader>
        <CardContent className={`min-h-0 flex-1 overflow-auto ${isDesktopFoldersLayout ? 'px-5 py-4' : ''}`}>
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
            <div className="mx-auto w-full max-w-xl rounded-[20px] border border-dashed border-border/70 bg-background/72 px-5 py-5">
              <div className="text-center text-sm text-muted-foreground">
                {bindingFilter === 'all'
                  ? (visibleChildFolders.length > 0
                    ? t('documents.emptyCurrentFolderWithChildren', '当前目录没有直属文件，请继续进入下一级目录。')
                    : t('documents.empty'))
                  : bindingUsageLoading
                    ? '正在按分身占用状态筛选...'
                    : '当前筛选条件下没有文档'}
              </div>
              {bindingFilter === 'all' && visibleChildFolders.length > 0 ? (
                <div className="mt-4 border-t border-border/50 pt-4">
                  {renderChildFolderButtons()}
                </div>
              ) : null}
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
                      {doc.is_public && <span className="ml-1 text-xs text-status-info-text" title="Public">🌐</span>}
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
        {showDocumentPagination && (
          <div className={`${isMobile ? 'px-3' : 'px-6'} shrink-0 border-t py-3`}>
            {renderDocumentPagination(true)}
          </div>
        )}
      </Card>

      {/* Right Panel: Preview / Edit / Versions / Diff */}
      {hasRightPanel && panel.doc && (
        <Card className={isMobile ? 'fixed inset-0 z-50 overflow-hidden rounded-none border-0' : 'relative w-[min(45%,420px)] min-w-[300px] rounded-none border-0 border-l border-border/55 bg-background/96 shadow-none'}>
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
                    embedded={!isMobile}
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

      <Dialog open={mobileFolderSheetOpen && !isConversationTaskMode} onOpenChange={setMobileFolderSheetOpen}>
        <DialogContent className="left-0 top-0 h-[100dvh] max-h-[100dvh] w-screen max-w-none translate-x-0 translate-y-0 gap-0 overflow-x-hidden rounded-none border-0 px-0 pb-0 pt-0 sm:max-h-[100dvh] sm:max-w-none">
          <div className="flex h-full min-h-0 flex-col overflow-x-hidden bg-background">
            <div className="border-b border-border/60 px-4 py-4">
              <div className="flex items-start gap-3 pr-10">
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  className="h-8 shrink-0 rounded-full px-2.5 text-[11px] text-muted-foreground"
                  onClick={() => setMobileFolderSheetOpen(false)}
                >
                  {t('documents.backToDocuments', '返回文档')}
                </Button>
                <div>
                  <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
                    {t('documents.folderNavigator', '文件夹导航')}
                  </div>
                  <div className="mt-1 text-[14px] font-semibold text-foreground">
                    {t('documents.folderNavigatorMobileTitle', '在移动端切换目录')}
                  </div>
                  <div className="mt-1 text-[11px] leading-4.5 text-muted-foreground">
                    {t('documents.folderNavigatorMobileHint', '展开目录树选择当前位置，关闭后会保留当前目录和文档列表状态。')}
                  </div>
                </div>
              </div>
            </div>
            <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto px-4 py-4">
              {renderFolderNavigator('mobile')}
            </div>
          </div>
        </DialogContent>
      </Dialog>

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
            <div className="rounded-[14px] border border-border/60 bg-[hsl(var(--ui-surface-panel-muted))/0.24] px-3 py-2 text-[11px] text-muted-foreground">
              {t('documents.folderCreateTarget', '创建到目录：{{path}}', { path: folderDialog.parentPath || currentFolderPath || '/' })}
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

  if (isConversationTaskMode) {
    const focusDocument = panel.doc;

    return (
      <>
        <MobileWorkspaceShell
          summary={(
            <ContextSummaryBar
              eyebrow={t('teamNav.documents', '文档')}
              title={t('documents.title', '文档工作台')}
              description={t(
                'documents.mobileConversationDescription',
                '先围绕当前材料推进工作，列表、筛选和视图切换都退到辅助面板。',
              )}
              metrics={[
                { label: t('documents.summaryView', '当前视图'), value: t(`documents.viewMode.${viewMode}`) },
                { label: t('documents.summaryFolder', '当前位置'), value: currentFolderPath || '/' },
                { label: t('documents.summaryTotal', '当前文档'), value: pagination.total },
                { label: t('documents.summarySelected', '选中项'), value: selectedIds.size },
              ]}
            />
          )}
          quickActions={(
            <div className="grid grid-cols-2 gap-1.5">
                <Button variant="outline" className="h-9 justify-start rounded-[14px] px-3 text-[11px]" onClick={openMobileFolderPanel}>
                <FolderOpen className="mr-2 h-4 w-4" />
                {t('documents.openFolderNavigator', '文件夹导航')}
              </Button>
              <Button variant="outline" className="h-9 justify-start rounded-[14px] px-3 text-[11px]" onClick={openMobileLibraryPanel}>
                <FolderOpen className="mr-2 h-4 w-4" />
                {t('documents.openLibrary', '打开文档面板')}
              </Button>
              <Button variant="outline" className="h-9 justify-start rounded-[14px] px-3 text-[11px]" onClick={openMobileViewPanel}>
                <LayoutGrid className="mr-2 h-4 w-4" />
                {t('documents.quickViews', '切换视图')}
              </Button>
              <Button variant="outline" className="h-9 justify-start rounded-[14px] px-3 text-[11px]" onClick={openMobileFilterPanel}>
                <SlidersHorizontal className="mr-2 h-4 w-4" />
                {t('documents.quickFilters', '筛选与排序')}
              </Button>
              <Button
                variant="outline"
                className="h-9 justify-start rounded-[14px] px-3 text-[11px]"
                onClick={() => navigate(`/teams/${teamId}?section=chat`)}
              >
                <MessageSquareText className="mr-2 h-4 w-4" />
                {t('documents.quickChat', '进入对话协助')}
              </Button>
            </div>
          )}
          stage={(
            <div className="flex h-full min-h-[360px] flex-col gap-3 p-3">
              <div className="rounded-[18px] border border-border/65 bg-background/88 px-3.5 py-3">
                <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">
                  {focusDocument ? t('documents.currentDocument', '当前文档') : t('documents.contextRail', '文档与产物上下文')}
                </div>
                <div className="mt-1 text-[13px] font-semibold text-foreground">
                  {focusDocument
                    ? (focusDocument.display_name || focusDocument.name)
                    : t('documents.mobileConversationHeadline', '先选材料，再继续对话或进入 AI 工作区。')}
                </div>
                <div className="mt-1 text-[11px] leading-4.5 text-muted-foreground">
                  {focusDocument
                    ? `${focusDocument.folder_path || '/'} · ${formatFileSize(focusDocument.file_size || 0)}`
                    : t('documents.mobileConversationHint', '文档列表、视图切换和筛选都退到文档面板，首屏优先保留当前材料线索。')}
                </div>
                <div className="mt-3 flex flex-wrap gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-8 rounded-[12px] px-3 text-[11px]"
                    onClick={openMobileFolderPanel}
                  >
                    <FolderOpen className="mr-1.5 h-3.5 w-3.5" />
                    {t('documents.openFolderNavigator', '文件夹导航')}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-8 rounded-[12px] px-3 text-[11px]"
                    onClick={() => focusDocument ? handleDocClick(focusDocument) : openMobileLibraryPanel()}
                  >
                    <CheckSquare className="mr-1.5 h-3.5 w-3.5" />
                    {focusDocument ? t('documents.openCurrent', '打开当前文档') : t('documents.openLibrary', '打开文档面板')}
                  </Button>
                </div>
              </div>
            </div>
          )}
          rail={(
            <ManagementRail
              title={focusDocument ? t('documents.currentContext', '当前文档上下文') : t('documents.contextRail', '文档与产物上下文')}
              description={t(
                'documents.mobileConversationRail',
                '文档页在对话模式里只承担材料与结果物角色：浏览、预览、版本和 AI 工作台都作为当前任务上下文出现。',
              )}
            >
              {focusDocument ? (
                <div className="space-y-2 rounded-[16px] border border-border/60 bg-background/82 px-3 py-3 text-[11px]">
                  <div className="flex items-start justify-between gap-3">
                    <span className="text-muted-foreground">{t('documents.currentMode', '当前模式')}</span>
                    <span className="text-right font-semibold text-foreground">
                      {panel.mode ? t(`documents.panelMode.${panel.mode}`, panel.mode) : t(`documents.viewMode.${viewMode}`)}
                    </span>
                  </div>
                  <div className="flex items-start justify-between gap-3">
                    <span className="text-muted-foreground">{t('documents.status', '状态')}</span>
                    <span className="text-right font-semibold text-foreground">
                      {t(`documents.status.${focusDocument.status}`, focusDocument.status)}
                    </span>
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    className="mt-1 h-8 w-full justify-center rounded-[12px] text-[11px]"
                    onClick={openMobileLibraryPanel}
                  >
                    {t('documents.openLibrary', '打开文档面板')}
                  </Button>
                </div>
              ) : (
                <div className="space-y-2 rounded-[16px] border border-border/60 bg-background/82 px-3 py-3 text-[11px]">
                  <div className="flex items-start justify-between gap-3">
                    <span className="text-muted-foreground">{t('documents.currentMode', '当前模式')}</span>
                    <span className="text-right font-semibold text-foreground">{t(`documents.viewMode.${viewMode}`)}</span>
                  </div>
                  <div className="flex items-start justify-between gap-3">
                    <span className="text-muted-foreground">{t('documents.currentFolder', '当前目录')}</span>
                    <span className="text-right font-semibold text-foreground">{currentFolderPath || '/'}</span>
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    className="mt-1 h-8 w-full justify-center rounded-[12px] text-[11px]"
                    onClick={openMobileFolderPanel}
                  >
                    {t('documents.openFolderNavigator', '文件夹导航')}
                  </Button>
                </div>
              )}
            </ManagementRail>
          )}
          panel={(
            <>
              <BottomSheetPanel
                open={mobileFolderSheetOpen}
                onOpenChange={setMobileFolderSheetOpen}
                title={t('documents.folderNavigator', '文件夹导航')}
                description={t('documents.mobileFolderNavigatorHint', '使用折叠文件夹树快速切换目录，文档列表会跟随当前路径更新。')}
                fullHeight
                onBack={() => setMobileFolderSheetOpen(false)}
                backLabel={t('documents.backToWorkspace', '返回工作台')}
              >
                {renderFolderNavigator('mobile')}
              </BottomSheetPanel>
              <BottomSheetPanel
                open={mobileLibrarySheetOpen}
                onOpenChange={setMobileLibrarySheetOpen}
                title={t('documents.openLibrary', '打开文档面板')}
                description={t('documents.mobileLibraryHint', '浏览文档、切换视图、查看 AI 工作区与引用材料。')}
                fullHeight
                onBack={() => setMobileLibrarySheetOpen(false)}
                backLabel={t('documents.backToWorkspace', '返回工作台')}
              >
                <div className="space-y-4">
                  <div className="flex flex-wrap items-center gap-2">
                    {viewModeSwitcher}
                  </div>
                  {mobileLibraryContent}
                </div>
              </BottomSheetPanel>
              <BottomSheetPanel
                open={mobileViewSheetOpen}
                onOpenChange={setMobileViewSheetOpen}
                title={t('documents.quickViews', '切换视图')}
                description={t('documents.viewModeHint', '在移动端快速切换文档、AI 工作台、血缘和归档视图。')}
                onBack={() => setMobileViewSheetOpen(false)}
                backLabel={t('documents.backToWorkspace', '返回工作台')}
              >
                {viewModeSwitcher}
              </BottomSheetPanel>
              <BottomSheetPanel
                open={mobileFilterSheetOpen}
                onOpenChange={setMobileFilterSheetOpen}
                title={t('documents.quickFilters', '筛选与排序')}
                description={t('documents.filterHint', '调整搜索、占用情况、标签与排序方式。')}
                onBack={() => setMobileFilterSheetOpen(false)}
                backLabel={t('documents.backToWorkspace', '返回工作台')}
              >
                {mobileFilterPanel}
              </BottomSheetPanel>
            </>
          )}
        />
      </>
    );
  }

  return documentsContent;
}
