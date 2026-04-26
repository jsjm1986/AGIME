import { useState, useEffect, useRef, useCallback, useMemo, lazy, Suspense } from 'react';
import { useTranslation } from 'react-i18next';
import { Loader2, FolderOpen, CheckSquare, X, Download, MessageSquareText, SlidersHorizontal, LayoutGrid, Search, Upload } from 'lucide-react';
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
import { DocumentFileCard, type DocumentFileCardAction } from '../documents/DocumentFileCard';
import { SupportedFormatsGuide } from '../documents/SupportedFormatsGuide';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { useIsMobile } from '../../hooks/useMediaQuery';
import { useToast } from '../../contexts/ToastContext';
import { formatDate } from '../../utils/format';
import { cn } from '../../utils';
import { useMobileInteractionMode } from '../../contexts/MobileInteractionModeContext';
import { ContextSummaryBar } from '../mobile/ContextSummaryBar';
import { BottomSheetPanel } from '../mobile/BottomSheetPanel';
import { ManagementRail } from '../mobile/ManagementRail';
import { MobileWorkspaceShell } from '../mobile/MobileWorkspaceShell';
import { apiClient } from '../../api/client';
import type { TeamMember } from '../../api/types';

const MAX_FILE_SIZE = 50 * 1024 * 1024; // 50MB
const USER_UPLOAD_ROOT_PATH = '/用户上传文档';
const DocumentPreview = lazy(() =>
  import('../documents/DocumentPreview').then((module) => ({ default: module.DocumentPreview })),
);

function DocumentPreviewLoading() {
  return (
    <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
      Loading document preview...
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

function getBindingLabel(
  bindings: DocumentBindingPortalRef[],
  tone: BindingTone,
  translate: (key: string, fallback: string) => string,
): string {
  const toneLabel =
    tone === 'write'
      ? translate('documents.bindingWrite', 'Controlled write')
      : tone === 'draft'
        ? translate('documents.bindingDraft', 'Draft collaboration')
        : translate('documents.bindingReading', 'Reading');
  if (bindings.length === 0) {
    return '';
  }
  if (bindings.length === 1) {
    return `${bindings[0].portalName} · ${toneLabel}`;
  }
  return `${bindings.length} ${translate('documents.bindingPortalCount', 'portal(s)')}${toneLabel}`;
}

function getBindingModeLabel(
  mode: DocumentBindingPortalRef['documentAccessMode'],
  translate: (key: string, fallback: string) => string,
): string {
  if (mode === 'controlled_write') return translate('documents.bindingWrite', 'Controlled write');
  if (mode === 'co_edit_draft') return translate('documents.bindingDraft', 'Draft collaboration');
  return translate('documents.bindingReadOnly', 'Read only');
}

function getPortalStatusLabel(
  status: DocumentBindingPortalRef['portalStatus'],
  translate: (key: string, fallback: string) => string,
): string {
  if (status === 'published') return translate('documents.portalStatusPublished', 'Published');
  if (status === 'archived') return translate('documents.portalStatusArchived', 'Archived');
  return translate('documents.portalStatusDraft', 'Draft');
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

function getParentFolderPath(path: string | null): string | null {
  if (!path || path === '/') {
    return null;
  }
  const parts = path.split('/').filter(Boolean);
  if (parts.length <= 1) {
    return null;
  }
  return `/${parts.slice(0, -1).join('/')}`;
}

function countFolderNodes(nodes: FolderTreeNode[]): number {
  return nodes.reduce((total, node) => total + 1 + countFolderNodes(node.children ?? []), 0);
}

interface DocumentsTabProps {
  teamId: string;
  canManage: boolean;
}

interface UserUploadFolderSummary {
  docCount: number;
  previewDocs: DocumentSummary[];
  uploaderLabels: string[];
  uploaderCount: number;
  primaryUploaderLabel: string;
  latestUpdatedAt: string | null;
  childFolderCount: number;
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
  const [loadingFolders, setLoadingFolders] = useState(true);
  const [loadingDocuments, setLoadingDocuments] = useState(true);
  const [currentFolderPath, setCurrentFolderPath] = useState<string | null>(() => readStoredFolderPath(recentFolderStorageKey));
  const [searchQuery, setSearchQuery] = useState('');
  const [debouncedSearch, setDebouncedSearch] = useState('');
  const [mimeFilter, setMimeFilter] = useState('');
  const [tagFilter, setTagFilter] = useState('');
  const [bindingFilter, setBindingFilter] = useState<BindingFilterMode>('all');
  const [teamTags, setTeamTags] = useState<{ tag: string; count: number }[]>([]);
  const [sortBy, setSortBy] = useState<'date' | 'name' | 'size'>('date');
  const [pagination, setPagination] = useState<PaginationState>({ page: 1, total: 0, totalPages: 0 });
  const [childFolderPage, setChildFolderPage] = useState(1);

  // UI toggles
  const [showFolderTree, setShowFolderTree] = useState<boolean>(() => readStoredBoolean(folderTreeVisibleStorageKey, true));
  const [uploading, setUploading] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [selectionMode, setSelectionMode] = useState(false);
  const [openFileActionId, setOpenFileActionId] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>('folders');
  const [mobileFolderSheetOpen, setMobileFolderSheetOpen] = useState(false);
  const [mobileLibrarySheetOpen, setMobileLibrarySheetOpen] = useState(false);
  const [mobileViewSheetOpen, setMobileViewSheetOpen] = useState(false);
  const [mobileFilterSheetOpen, setMobileFilterSheetOpen] = useState(false);
  const [mobileNestedReturnTarget, setMobileNestedReturnTarget] = useState<'workspace' | 'library'>('workspace');
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
  const [deletingFolder, setDeletingFolder] = useState(false);
  const [userUploadSummaries, setUserUploadSummaries] = useState<Record<string, UserUploadFolderSummary>>({});
  const [loadingUserUploadSummaries, setLoadingUserUploadSummaries] = useState(false);

  const loadFolders = useCallback(async (options?: { silent?: boolean }) => {
    if (!options?.silent) {
      setLoadingFolders(true);
    }
    try {
      const foldersRes = await folderApi.getFolderTree(teamId);
      setFolders(foldersRes);
    } catch (error) {
      console.error('Failed to load folders:', error);
    } finally {
      if (!options?.silent) {
        setLoadingFolders(false);
      }
    }
  }, [teamId]);

  const loadDocuments = useCallback(async (options?: { silent?: boolean }) => {
    if (!options?.silent) {
      setLoadingDocuments(true);
    }
    try {
      const docsRes = debouncedSearch
        ? await documentApi.searchDocuments(teamId, debouncedSearch, pagination.page, pageSize, mimeFilter || undefined, currentFolderPath || undefined, tagFilter || undefined)
        : await documentApi.listDocuments(teamId, pagination.page, pageSize, currentFolderPath || undefined, mimeFilter || undefined, tagFilter || undefined);
      setDocuments(docsRes.items);
      setPagination(prev => ({ ...prev, total: docsRes.total, totalPages: docsRes.total_pages }));
    } catch (error) {
      console.error('Failed to load documents:', error);
    } finally {
      if (!options?.silent) {
        setLoadingDocuments(false);
      }
    }
  }, [teamId, currentFolderPath, debouncedSearch, pagination.page, pageSize, mimeFilter, tagFilter]);

  useEffect(() => {
    void loadFolders();
  }, [loadFolders]);

  useEffect(() => {
    void loadDocuments();
  }, [loadDocuments]);

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
    if (loadingFolders) {
      return;
    }
    if (!folderPathExists(folders, currentFolderPath)) {
      setCurrentFolderPath(null);
    }
  }, [folders, currentFolderPath, loadingFolders]);

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
      void loadFolders();
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
      void loadDocuments();
    } catch (error) {
      console.error('Failed to upload:', error);
    } finally {
      setUploading(false);
    }
  }, [currentFolderPath, loadDocuments, t, uploadFileWithProgress]);

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
        void loadDocuments();
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
      void loadDocuments();
    } catch (error) {
      console.error('Failed to delete:', error);
      void loadDocuments();
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
      void loadDocuments();
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
    setDeletingFolder(true);
    try {
      await folderApi.deleteFolder(teamId, deleteFolderTarget.id);
      if (
        currentFolderPath === deleteFolderTarget.fullPath ||
        currentFolderPath?.startsWith(`${deleteFolderTarget.fullPath}/`)
      ) {
        setCurrentFolderPath(null);
      }
      addToast('success', `已删除文件夹“${deleteFolderTarget.name}”及其中的文档`);
      await Promise.all([loadFolders({ silent: true }), loadDocuments({ silent: true })]);
    } catch (error) {
      console.error('Failed to delete folder:', error);
      const message = error instanceof Error ? error.message : t('documents.deleteFolderFailed', '删除文件夹失败');
      if (message.includes('Cannot delete a system folder')) {
        addToast('warning', t('documents.systemFolderDeleteForbidden', '这个文件夹是系统托管目录，不能手动删除。'));
      } else {
        addToast('error', message);
      }
    } finally {
      setDeletingFolder(false);
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
      void loadFolders();
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
      void loadDocuments();
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
    setOpenFileActionId(null);
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
    void loadDocuments();
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
    void loadDocuments();
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
            title={buildBindingTitle(
              bindings,
              tone === 'write'
                ? t('documents.bindingWrite', '允许直写')
                : tone === 'draft'
                  ? t('documents.bindingDraft', '草稿协作')
                  : t('documents.bindingReading', '读取中'),
            )}
          >
            {getBindingLabel(bindings, tone, (key, fallback) => t(key, fallback))}
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
      { title: t('documents.bindingWrite', '允许直写'), tone: 'write' as const, bindings: usage?.writeBindings ?? [] },
      { title: t('documents.bindingDraft', '草稿协作'), tone: 'draft' as const, bindings: usage?.draftBindings ?? [] },
      { title: t('documents.bindingReading', '读取中'), tone: 'read' as const, bindings: usage?.readBindings ?? [] },
    ].filter((group) => group.bindings.length > 0);

    return (
      <div className="border-t bg-muted/10 px-4 py-3">
        <div className="flex items-start justify-between gap-3">
          <div>
            <div className="text-sm font-semibold">
              {t('documents.bindingUsageTitle', '分身占用情况')}
            </div>
            <div className="text-xs text-muted-foreground">
              {t(
                'documents.bindingUsageDescription',
                 'See which portals currently read this document, collaborate on it as a draft, or have controlled write access.',
              )}
            </div>
          </div>
          {bindingUsageLoading && (
            <div className="text-xs text-muted-foreground">{t('common.loading', '加载中...')}</div>
          )}
        </div>
        {groups.length === 0 ? (
          <div className="mt-3 rounded-lg border border-dashed px-3 py-2 text-xs text-muted-foreground">
            {t('documents.bindingUsageEmpty', '当前文档未被任何服务 Agent 绑定。')}
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
                            <span>
                              {getBindingModeLabel(binding.documentAccessMode, (key, fallback) =>
                                t(key, fallback),
                              )}
                            </span>
                            <span>·</span>
                            <span>
                              {getPortalStatusLabel(binding.portalStatus, (key, fallback) =>
                                t(key, fallback),
                              )}
                            </span>
                            <span>·</span>
                            <span>
                              {binding.publicAccessEnabled
                                ? t('documents.portalPublicAccess', '公开访问中')
                                : t('documents.portalPreviewOnly', '仅预览')}
                            </span>
                          </div>
                          {binding.serviceAgentName && (
                            <div className="mt-1 text-xs text-muted-foreground">
                              {t('documents.serviceAgentLabel', '服务 Agent：{{name}}', {
                                name: binding.serviceAgentName,
                              })}
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
                            {t('documents.openAvatar', '查看分身')}
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

  const closeMobileDocumentPanels = useCallback(() => {
    if (!isMobile) {
      return;
    }
    setMobileLibrarySheetOpen(false);
    setMobileFolderSheetOpen(false);
    setMobileViewSheetOpen(false);
    setMobileFilterSheetOpen(false);
  }, [isMobile]);

  const getDocumentMetaLabel = useCallback((doc: DocumentSummary) => {
    return `${formatFileSize(doc.file_size)} · ${formatDate(doc.updated_at || doc.created_at)}`;
  }, []);

  const renderDocumentCardFooter = useCallback((doc: DocumentSummary, compact = false) => {
    const usage = bindingUsageByDocId[doc.id];
    const tags = doc.tags?.slice(0, compact ? 2 : 3) ?? [];
    const hasUsage = Boolean(usage && (usage.readBindings.length || usage.draftBindings.length || usage.writeBindings.length));
    if (!hasUsage && tags.length === 0) {
      return null;
    }

    return (
      <div className="space-y-2">
        {renderBindingChips(usage)}
        {tags.length > 0 ? (
          <div className="flex flex-wrap gap-1.5">
            {tags.map((tag) => (
              <button
                key={tag}
                type="button"
                className="inline-flex items-center rounded-full border border-primary/15 bg-primary/6 px-2 py-0.5 text-[10px] font-medium text-primary transition-colors hover:bg-primary/12"
                onClick={(event) => {
                  event.stopPropagation();
                  setTagFilter(tag);
                }}
              >
                {tag}
              </button>
            ))}
            {doc.tags && doc.tags.length > tags.length ? (
              <span className="inline-flex items-center text-[10px] font-medium text-muted-foreground">
                +{doc.tags.length - tags.length}
              </span>
            ) : null}
          </div>
        ) : null}
      </div>
    );
  }, [bindingUsageByDocId, renderBindingChips]);

  const buildDocumentActions = useCallback((doc: DocumentSummary, options?: { archived?: boolean }) => {
    const wrapAction = (action: () => void) => () => {
      setOpenFileActionId(null);
      action();
    };

    if (options?.archived) {
      return canManage ? [{
        key: 'restore',
        label: t('documents.restore'),
        onSelect: wrapAction(() => handleRestoreDocument(doc.id)),
      }] satisfies DocumentFileCardAction[] : [];
    }

    const actions: DocumentFileCardAction[] = [
      {
        key: 'download',
        label: t('documents.download'),
        onSelect: wrapAction(() => handleDownload(doc.id)),
      },
      {
        key: 'lineage',
        label: t('documents.lineage'),
        onSelect: wrapAction(() => {
          closeMobileDocumentPanels();
          setViewMode('lineage');
          setLineageDocId(doc.id);
        }),
      },
    ];

    if (canManage) {
      actions.push({
        key: 'edit',
        label: t('documents.editInfo'),
        onSelect: wrapAction(() => {
          closeMobileDocumentPanels();
          openEditMeta(doc);
        }),
      });
      actions.push({
        key: 'delete',
        label: t('common.delete'),
        tone: 'danger',
        onSelect: wrapAction(() => handleDeleteDocument(doc.id)),
      });
    }

    return actions;
  }, [canManage, closeMobileDocumentPanels, t]);

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

  const userUploadRoot = useMemo(
    () => folders.find((node) => node.fullPath === USER_UPLOAD_ROOT_PATH) ?? null,
    [folders],
  );
  const mainFolderNodes = useMemo(
    () => folders.filter((node) => node.fullPath !== USER_UPLOAD_ROOT_PATH),
    [folders],
  );
  const isBrowsingUserUploads = Boolean(
    currentFolderPath && (currentFolderPath === USER_UPLOAD_ROOT_PATH || currentFolderPath.startsWith(`${USER_UPLOAD_ROOT_PATH}/`)),
  );
  const userUploadFolderCount = useMemo(
    () => countFolderNodes(userUploadRoot?.children ?? []),
    [userUploadRoot],
  );
  const userUploadBackPath = useMemo(() => {
    if (!isBrowsingUserUploads) {
      return null;
    }
    if (currentFolderPath && currentFolderPath !== USER_UPLOAD_ROOT_PATH) {
      return getParentFolderPath(currentFolderPath) || USER_UPLOAD_ROOT_PATH;
    }
    return null;
  }, [currentFolderPath, isBrowsingUserUploads]);
  const userUploadBackLabel = userUploadBackPath
    ? t('documents.back', '返回上一级')
    : t('documents.backToWorkspace', '返回团队资料');

  const visibleChildFolders = useMemo(() => {
    if (!currentFolderPath) {
      return folders;
    }
    return currentFolderNode?.children ?? [];
  }, [currentFolderNode, currentFolderPath, folders]);
  const childFolderPageSize = isMobile ? 8 : 10;
  const childFolderTotalPages = Math.max(1, Math.ceil(visibleChildFolders.length / childFolderPageSize));
  const pagedChildFolders = useMemo(() => {
    const start = (childFolderPage - 1) * childFolderPageSize;
    return visibleChildFolders.slice(start, start + childFolderPageSize);
  }, [childFolderPage, childFolderPageSize, visibleChildFolders]);

  const handleFolderSelect = useCallback((path: string | null) => {
    setOpenFileActionId(null);
    setCurrentFolderPath(path);
    setPagination((prev) => (prev.page === 1 ? prev : { ...prev, page: 1 }));
    setChildFolderPage(1);
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

  const openUserUploadLibrary = useCallback(() => {
    setOpenFileActionId(null);
    setCurrentFolderPath(USER_UPLOAD_ROOT_PATH);
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

  const handleUserUploadBack = useCallback(() => {
    if (userUploadBackPath) {
      handleFolderSelect(userUploadBackPath);
      return;
    }
    handleFolderSelect(null);
  }, [handleFolderSelect, userUploadBackPath]);

  useEffect(() => {
    let active = true;
    if (!isBrowsingUserUploads || visibleChildFolders.length === 0) {
      setUserUploadSummaries({});
      setLoadingUserUploadSummaries(false);
      return;
    }

    setLoadingUserUploadSummaries(true);
    (async () => {
      try {
        const membersResponse = await apiClient.getMembers(teamId).catch(() => null);
        const memberNameById = new Map(
          (membersResponse?.members || []).map((member: TeamMember) => [member.userId, member.displayName]),
        );
        const summaries = await Promise.all(
          visibleChildFolders.map(async (folder) => {
            const response = await documentApi.listDocuments(teamId, 1, 4, folder.fullPath);
            const uploaderLabels = Array.from(
              new Set(
                response.items
                  .map((doc) => memberNameById.get(doc.uploaded_by) || doc.uploaded_by.slice(0, 8))
                  .filter(Boolean),
              ),
            );
            const latestUpdatedAt = response.items.reduce<string | null>((latest, doc) => {
              const candidate = doc.updated_at || doc.created_at;
              if (!latest) return candidate;
              return candidate > latest ? candidate : latest;
            }, null);
            return [
              folder.fullPath,
              {
                docCount: response.total,
                previewDocs: response.items,
                uploaderLabels: uploaderLabels.slice(0, 3),
                uploaderCount: uploaderLabels.length,
                primaryUploaderLabel:
                  uploaderLabels[0] || t('documents.unknownUploader', '待识别用户'),
                latestUpdatedAt,
                childFolderCount: folder.children?.length ?? 0,
              } satisfies UserUploadFolderSummary,
            ] as const;
          }),
        );
        if (!active) return;
        setUserUploadSummaries(Object.fromEntries(summaries));
      } catch (error) {
        console.error('Failed to load user upload folder summaries:', error);
        if (!active) return;
        setUserUploadSummaries({});
      } finally {
        if (active) {
          setLoadingUserUploadSummaries(false);
        }
      }
    })();

    return () => {
      active = false;
    };
  }, [isBrowsingUserUploads, teamId, visibleChildFolders]);

  useEffect(() => {
    setChildFolderPage((prev) => (prev > childFolderTotalPages ? childFolderTotalPages : prev));
  }, [childFolderTotalPages]);

  const userUploadOverview = useMemo(() => {
    if (!isBrowsingUserUploads || visibleChildFolders.length === 0) {
      return {
        folderCount: 0,
        docCount: 0,
        uploaderCount: 0,
        latestUpdatedAt: null as string | null,
      };
    }

    const uploaderSet = new Set<string>();
    let docCount = 0;
    let latestUpdatedAt: string | null = null;

    visibleChildFolders.forEach((folder) => {
      const summary = userUploadSummaries[folder.fullPath];
      if (!summary) {
        return;
      }
      docCount += summary.docCount;
      summary.uploaderLabels.forEach((label) => uploaderSet.add(label));
      if (!latestUpdatedAt || (summary.latestUpdatedAt && summary.latestUpdatedAt > latestUpdatedAt)) {
        latestUpdatedAt = summary.latestUpdatedAt;
      }
    });

    return {
      folderCount: visibleChildFolders.length,
      docCount,
      uploaderCount: uploaderSet.size,
      latestUpdatedAt,
    };
  }, [isBrowsingUserUploads, userUploadSummaries, visibleChildFolders]);

  const renderChildFolderButtons = useCallback((compact = false) => {
    if (visibleChildFolders.length === 0) {
      return null;
    }

    if (isBrowsingUserUploads && !compact) {
      return (
        <div className="space-y-2">
          <div className="flex items-center justify-between gap-3 border-b border-[hsl(var(--ui-line-soft))] pb-2">
            <div className="text-[11px] font-medium text-foreground">
              {t('documents.uploadFolders', 'Upload folders')}
            </div>
            <div className="text-[10.5px] text-muted-foreground">
              {t(
                'documents.uploadFoldersHint',
                 'Browse by uploader, document count, and most recent update time',
              )}
            </div>
          </div>
          <div className="overflow-hidden rounded-[16px] border border-[hsl(var(--ui-line-soft))] bg-background">
            <div className="grid grid-cols-[minmax(0,1.45fr)_110px_128px_minmax(220px,1fr)] gap-3 border-b border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-4 py-2 text-[10px] font-medium uppercase tracking-[0.06em] text-muted-foreground">
              <span>{t('documents.folderAndUploader', 'Folder / uploader')}</span>
              <span>{t('documents.files', '文件')}</span>
              <span>{t('documents.lastUpdated', 'Last updated')}</span>
              <span>{t('documents.preview', '预览')}</span>
            </div>
            <div className="divide-y divide-[hsl(var(--ui-line-soft))]">
              {pagedChildFolders.map((folder) => {
                const summary = userUploadSummaries[folder.fullPath];
                return (
                  <button
                    key={folder.id}
                    type="button"
                    className="grid w-full grid-cols-[minmax(0,1.45fr)_110px_128px_minmax(220px,1fr)] items-start gap-3 px-4 py-3 text-left transition-colors hover:bg-[hsl(var(--ui-surface-selected))]"
                    onClick={() => handleFolderSelect(folder.fullPath)}
                    title={folder.fullPath}
                  >
                    <span className="flex min-w-0 items-start gap-3">
                      <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-[12px] bg-[hsl(var(--ui-surface-panel))] text-muted-foreground">
                        <FolderOpen className="h-4 w-4" />
                      </span>
                      <span className="min-w-0">
                        <span className="block truncate text-[12px] font-semibold text-foreground">
                          {summary?.primaryUploaderLabel || t('documents.unknownUploader', '待识别用户')}
                        </span>
                        <span className="mt-0.5 block truncate text-[10.5px] text-muted-foreground">
                          {folder.name}
                        </span>
                        {summary?.uploaderCount && summary.uploaderCount > 1 ? (
                          <span className="mt-1 block truncate text-[10.5px] text-muted-foreground">
                            {t('documents.otherUploaders', 'Other uploaders: {{value}}', {
                              value:
                                summary.uploaderLabels.slice(1).join('、') ||
                                t('documents.uploaderCount', '{{count}} people total', {
                                  count: summary.uploaderCount,
                                }),
                            })}
                          </span>
                        ) : null}
                      </span>
                    </span>
                    <span className="pt-1 text-[11px] text-foreground">
                      {t('documents.documentCount', '{{count}} 份', {
                        count: summary?.docCount ?? 0,
                      })}
                      {(summary?.childFolderCount ?? 0) > 0 ? (
                        <span className="ml-1 text-[10px] text-muted-foreground">
                          {t('documents.childFolderCount', '· {{count}} 子目录', {
                            count: summary?.childFolderCount,
                          })}
                        </span>
                      ) : null}
                    </span>
                    <span className="pt-1 text-[10.5px] text-muted-foreground">
                      {summary?.latestUpdatedAt
                        ? formatDate(summary.latestUpdatedAt)
                        : t('documents.noneShort', '暂无')}
                    </span>
                    <span className="pt-1 text-[10.5px] text-muted-foreground">
                      {summary?.previewDocs?.length
                        ? summary.previewDocs.map((doc) => doc.display_name || doc.name).join('、')
                        : loadingUserUploadSummaries
                          ? t('documents.loadingOverview', '正在读取文档概览…')
                          : t('documents.emptyCurrentFolder', 'No documents in the current folder')}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>
        </div>
      );
    }

    if (!compact) {
      return (
        <div className="space-y-2">
          <div className="flex items-center justify-between gap-3 border-b border-[hsl(var(--ui-line-soft))] pb-2">
            <div className="text-[11px] font-medium text-foreground">
              {t('documents.childFolders', 'Child folders')}
            </div>
            <div className="text-[10.5px] text-muted-foreground">
              {t('documents.childFolderSummary', '{{count}} folders in the current directory', {
                count: visibleChildFolders.length,
              })}
            </div>
          </div>
          <div className="overflow-hidden rounded-[16px] border border-[hsl(var(--ui-line-soft))] bg-background">
            <div className="grid grid-cols-[minmax(0,1fr)_140px] gap-3 border-b border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-4 py-2 text-[10px] font-medium uppercase tracking-[0.06em] text-muted-foreground">
              <span>{t('documents.folders', 'Folders')}</span>
              <span>{t('documents.childFolders', 'Child folders')}</span>
            </div>
            <div className="divide-y divide-[hsl(var(--ui-line-soft))]">
              {pagedChildFolders.map((folder) => (
                <button
                  key={folder.id}
                  type="button"
                  className="grid w-full grid-cols-[minmax(0,1fr)_140px] items-start gap-3 px-4 py-3 text-left transition-colors hover:bg-[hsl(var(--ui-surface-selected))]"
                  onClick={() => handleFolderSelect(folder.fullPath)}
                  title={folder.fullPath}
                >
                  <span className="flex min-w-0 items-start gap-3">
                    <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-[12px] bg-[hsl(var(--ui-surface-panel))] text-muted-foreground">
                      <FolderOpen className="h-4 w-4" />
                    </span>
                    <span className="min-w-0">
                      <span className="block truncate text-[12px] font-medium text-foreground">
                        {folder.name}
                      </span>
                      <span className="mt-0.5 block truncate text-[10.5px] text-muted-foreground">
                        {folder.fullPath}
                      </span>
                    </span>
                  </span>
                  <span className="pt-1 text-[10.5px] text-muted-foreground">
                    {t('documents.folderCountShort', '{{count}} 个', {
                      count: folder.children?.length ?? 0,
                    })}
                  </span>
                </button>
              ))}
            </div>
          </div>
        </div>
      );
    }

    return (
      <div className="space-y-2">
        <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground/82">
          {t('documents.childFolders', 'Child folders')}
        </div>
        <div className="space-y-1.5">
          {pagedChildFolders.map((folder) => (
            <button
              key={folder.id}
              type="button"
              className={`flex w-full items-start gap-3 rounded-[16px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] ${compact ? 'px-3 py-2.5' : 'px-3.5 py-3.5'} text-left transition-colors hover:bg-[hsl(var(--ui-surface-selected))]`}
              onClick={() => handleFolderSelect(folder.fullPath)}
              title={folder.fullPath}
            >
              <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[12px] bg-[hsl(var(--ui-surface-panel-strong))] text-muted-foreground">
                <FolderOpen className="h-4 w-4" />
              </span>
              <span className="min-w-0 flex-1">
                {isBrowsingUserUploads ? (
                  <span className="flex items-center justify-between gap-3">
                    <span className="min-w-0">
                      <span className="block truncate text-[12px] font-semibold text-foreground">
                        {userUploadSummaries[folder.fullPath]?.primaryUploaderLabel || t('documents.unknownUploader', '待识别用户')}
                      </span>
                      <span className="mt-0.5 block truncate text-[10.5px] text-muted-foreground">
                        {t('documents.folderLabel', '目录：{{name}}', { name: folder.name })}
                      </span>
                    </span>
                    <span className="flex shrink-0 items-center gap-1.5">
                      <span className="rounded-full border border-[hsl(var(--ui-line-soft))] bg-background px-2 py-0.5 text-[10px] text-muted-foreground">
                        {t('documents.documentCountLong', '{{count}} 份文档', {
                          count: userUploadSummaries[folder.fullPath]?.docCount ?? 0,
                        })}
                      </span>
                      {(userUploadSummaries[folder.fullPath]?.childFolderCount ?? 0) > 0 ? (
                        <span className="rounded-full border border-[hsl(var(--ui-line-soft))] bg-background px-2 py-0.5 text-[10px] text-muted-foreground">
                          {t('documents.childFolderCountPlain', '{{count}} 子目录', {
                            count: userUploadSummaries[folder.fullPath]?.childFolderCount,
                          })}
                        </span>
                      ) : null}
                    </span>
                  </span>
                ) : (
                  <span className="flex items-center gap-2">
                    <span className="truncate text-[12px] font-medium text-foreground">
                      {folder.name}
                    </span>
                  </span>
                )}
                {isBrowsingUserUploads ? (
                  <span className="mt-1 block space-y-1">
                    <span className="flex flex-wrap items-center gap-2 text-[10.5px] text-muted-foreground">
                      {userUploadSummaries[folder.fullPath]?.uploaderCount ? (
                        <span>
                          {t('documents.uploaderPrefix', 'Uploader: ')}
                          {userUploadSummaries[folder.fullPath].uploaderLabels.join('、')}
                          {userUploadSummaries[folder.fullPath].uploaderCount > userUploadSummaries[folder.fullPath].uploaderLabels.length
                            ? t('documents.andUploaderCount', ' and {{count}} more', {
                                count: userUploadSummaries[folder.fullPath].uploaderCount,
                              })
                            : ''}
                        </span>
                      ) : (
                        <span>{t('documents.unknownUploaderInline', 'Uploader: pending identification')}</span>
                      )}
                      {userUploadSummaries[folder.fullPath]?.latestUpdatedAt ? (
                        <span>{t('documents.lastUpdatedInline', 'Last updated: {{time}}', {
                          time: formatDate(userUploadSummaries[folder.fullPath].latestUpdatedAt || ''),
                        })}</span>
                      ) : null}
                    </span>
                    {userUploadSummaries[folder.fullPath]?.previewDocs?.length ? (
                      <span className="block text-[10.5px] text-muted-foreground">
                        {t('documents.documentsInline', '文档：{{names}}', {
                          names: userUploadSummaries[folder.fullPath].previewDocs.map((doc) => doc.display_name || doc.name).join('、'),
                        })}
                      </span>
                    ) : loadingUserUploadSummaries ? (
                      <span className="block text-[10.5px] text-muted-foreground">{t('documents.loadingOverview', '正在读取文档概览…')}</span>
                    ) : (
                      <span className="block text-[10.5px] text-muted-foreground">{t('documents.emptyCurrentFolder', 'No documents in the current folder')}</span>
                    )}
                  </span>
                ) : null}
              </span>
            </button>
          ))}
        </div>
      </div>
    );
  }, [handleFolderSelect, isBrowsingUserUploads, loadingUserUploadSummaries, pagedChildFolders, t, userUploadSummaries, visibleChildFolders.length]);

  const isInitialLoading = loadingFolders && loadingDocuments && folders.length === 0;

  if (isInitialLoading) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  const hasRightPanel = panel.doc && panel.mode;
  const showDocumentPagination = viewMode === 'folders' && pagination.totalPages > 1;

  const renderFolderNavigator = (
    variant: 'desktop' | 'mobile',
    options?: { embedded?: boolean; showHeader?: boolean },
  ) => (
    <DocumentFolderNavigator
      nodes={isBrowsingUserUploads ? (userUploadRoot?.children ?? []) : mainFolderNodes}
      currentPath={currentFolderPath}
      rootPath={isBrowsingUserUploads ? USER_UPLOAD_ROOT_PATH : null}
      rootLabel={isBrowsingUserUploads ? t('documents.userUploads', 'User uploads') : undefined}
      rootHint={isBrowsingUserUploads ? t('documents.userUploadsRootHint', 'Return to the root view of uploaded materials') : undefined}
      onSelectPath={handleFolderSelect}
      canManage={canManage}
      onCreateFolder={openCreateFolder}
      onRenameFolder={openRenameFolder}
      onDeleteFolder={handleDeleteFolder}
      storageKey={folderTreeExpandedStorageKey}
      variant={variant}
      embedded={options?.embedded ?? variant === 'desktop'}
      showHeader={options?.showHeader}
      className={cn('min-h-0', variant === 'desktop' && 'h-full')}
    />
  );

  const openMobileFolderPanel = () => {
    setMobileNestedReturnTarget('workspace');
    setMobileLibrarySheetOpen(false);
    setMobileViewSheetOpen(false);
    setMobileFilterSheetOpen(false);
    setMobileFolderSheetOpen(true);
  };

  const openMobileLibraryPanel = () => {
    setMobileNestedReturnTarget('workspace');
    setMobileFolderSheetOpen(false);
    setMobileViewSheetOpen(false);
    setMobileFilterSheetOpen(false);
    setMobileLibrarySheetOpen(true);
  };

  const openMobileViewPanel = (returnTarget: 'workspace' | 'library' = 'workspace') => {
    setMobileNestedReturnTarget(returnTarget);
    setMobileFolderSheetOpen(false);
    setMobileLibrarySheetOpen(false);
    setMobileFilterSheetOpen(false);
    setMobileViewSheetOpen(true);
  };

  const openMobileFilterPanel = (returnTarget: 'workspace' | 'library' = 'workspace') => {
    setMobileNestedReturnTarget(returnTarget);
    setMobileFolderSheetOpen(false);
    setMobileLibrarySheetOpen(false);
    setMobileViewSheetOpen(false);
    setMobileFilterSheetOpen(true);
  };

  const handleMobileViewBack = () => {
    if (mobileNestedReturnTarget === 'library') {
      setMobileViewSheetOpen(false);
      setMobileLibrarySheetOpen(true);
      return;
    }
    setMobileViewSheetOpen(false);
  };

  const handleMobileFilterBack = () => {
    if (mobileNestedReturnTarget === 'library') {
      setMobileFilterSheetOpen(false);
      setMobileLibrarySheetOpen(true);
      return;
    }
    setMobileFilterSheetOpen(false);
  };

  const classicMobileFolderSummary = isMobile ? (
    <div className="mt-2 space-y-2 border-t border-[hsl(var(--ui-line-soft))] pt-2">
      <button
        type="button"
        className="flex w-full items-center gap-2.5 text-left transition-colors"
        onClick={openMobileFolderPanel}
      >
        <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[10px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] text-muted-foreground">
          <FolderOpen className="h-3.5 w-3.5" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block text-[10px] font-medium text-muted-foreground/84">
            {t('documents.currentFolder', 'Current folder')} · {visibleDocs.length}
          </span>
          <span className="mt-0.5 block truncate text-[12px] font-semibold text-foreground">
            {currentFolderPath || '/'}
          </span>
        </span>
        <span className="shrink-0 rounded-full border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-2.5 py-1 text-[10px] font-medium text-muted-foreground">
          {t('documents.openFolderNavigator', 'Folders')}
        </span>
      </button>
      {userUploadRoot ? (
        <button
          type="button"
          className={`flex w-full items-center gap-2.5 rounded-[12px] border px-3 py-2 text-left transition-colors ${
            isBrowsingUserUploads
              ? 'border-primary/25 bg-primary/[0.06]'
              : 'border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] hover:bg-[hsl(var(--ui-surface-selected))]'
          }`}
          onClick={isBrowsingUserUploads ? handleUserUploadBack : openUserUploadLibrary}
        >
          <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[10px] border border-[hsl(var(--ui-line-soft))] bg-background text-muted-foreground">
            <Upload className="h-3.5 w-3.5" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block text-[10px] font-medium text-muted-foreground/84">
              {isBrowsingUserUploads
                ? t('documents.back', '返回')
                : t('documents.separateEntry', '独立入口')}
            </span>
            <span className="mt-0.5 block truncate text-[12px] font-semibold text-foreground">
              {isBrowsingUserUploads
                ? userUploadBackLabel
                : t('documents.userUploadsShort', 'Uploads')}
            </span>
          </span>
          {!isBrowsingUserUploads && userUploadFolderCount > 0 ? (
            <span className="shrink-0 rounded-full border border-[hsl(var(--ui-line-soft))] bg-background px-2 py-0.5 text-[10px] text-muted-foreground">
              {userUploadFolderCount}
            </span>
          ) : null}
        </button>
      ) : null}
    </div>
  ) : null;
  const isDesktopFoldersLayout = viewMode === 'folders' && !isMobile;
  const documentConsoleShellClass = 'rounded-[22px] border border-[hsl(var(--ui-line-soft))] bg-background';
  const documentConsoleSubpanelClass = 'rounded-[14px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))]';

  const renderDocumentPagination = (compact = false) => (
    <div
      className={
        compact
          ? 'flex items-center justify-between gap-3 rounded-[12px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-2.5 py-1.5'
          : 'flex items-center justify-between gap-3 rounded-[14px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-3 py-2'
      }
    >
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
      <div className={compact ? 'flex min-w-0 flex-1 items-center justify-between gap-3' : 'ml-auto flex items-center gap-2'}>
        {compact ? (
          <div className="min-w-0 flex-1">
            <div className="text-[11px] text-muted-foreground">
              {t('documents.pageSummaryCompact', '第 {{page}} / {{totalPages}} 页 · 共 {{total}} 条', {
                page: pagination.page,
                totalPages: pagination.totalPages,
                total: pagination.total,
              })}
            </div>
          </div>
        ) : null}
        <div className={compact ? 'flex shrink-0 items-center gap-1' : 'flex items-center gap-2'}>
          <Button
            size="sm"
            variant="ghost"
            className={compact ? 'h-8 rounded-[12px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel-strong))] px-3 text-[11px] text-muted-foreground shadow-none hover:bg-background hover:text-foreground' : undefined}
            disabled={pagination.page <= 1}
            onClick={() => setPagination((p) => ({ ...p, page: p.page - 1 }))}
          >
            {t('pagination.previous')}
          </Button>
          <span className={compact ? 'min-w-[3.5rem] text-center text-[11px] font-medium text-foreground' : 'min-w-[4.5rem] text-center text-sm text-muted-foreground'}>
            {compact
              ? `${pagination.page} / ${pagination.totalPages}`
              : `${pagination.page} / ${pagination.totalPages}`}
          </span>
          <Button
            size="sm"
            variant="ghost"
            className={compact ? 'h-8 rounded-[12px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel-strong))] px-3 text-[11px] text-muted-foreground shadow-none hover:bg-background hover:text-foreground' : undefined}
            disabled={pagination.page >= pagination.totalPages}
            onClick={() => setPagination((p) => ({ ...p, page: p.page + 1 }))}
          >
            {t('pagination.next')}
          </Button>
        </div>
      </div>
    </div>
  );

  const renderChildFolderPagination = (compact = false) => (
    <div
      className={
        compact
          ? 'mt-3 flex items-center justify-between gap-3 rounded-[12px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-2.5 py-1.5'
          : 'mt-3 flex items-center justify-between gap-3 rounded-[14px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-3 py-2'
      }
    >
      <div className="min-w-0 text-[11px] text-muted-foreground">
        {t('documents.folderPaginationSummary', 'Folders {{page}} / {{totalPages}} · {{count}} total', {
          page: childFolderPage,
          totalPages: childFolderTotalPages,
          count: visibleChildFolders.length,
        })}
      </div>
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          variant="ghost"
          className={compact ? 'h-8 rounded-[12px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel-strong))] px-3 text-[11px] text-muted-foreground shadow-none hover:bg-background hover:text-foreground' : undefined}
          disabled={childFolderPage <= 1}
          onClick={() => setChildFolderPage((prev) => Math.max(1, prev - 1))}
        >
          {t('pagination.previous')}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className={compact ? 'h-8 rounded-[12px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel-strong))] px-3 text-[11px] text-muted-foreground shadow-none hover:bg-background hover:text-foreground' : undefined}
          disabled={childFolderPage >= childFolderTotalPages}
          onClick={() => setChildFolderPage((prev) => Math.min(childFolderTotalPages, prev + 1))}
        >
          {t('pagination.next')}
        </Button>
      </div>
    </div>
  );

  const handleSelectViewMode = (mode: ViewMode) => {
    setOpenFileActionId(null);
    setViewMode(mode);
    setLineageDocId(null);
    if (mode === 'trash') {
      setArchivedPage(1);
    }
    setMobileViewSheetOpen(false);
  };

  const viewModeSwitcher = (
    <div className={cn(
      'grid items-center gap-1 rounded-[18px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] p-1',
      isMobile ? 'grid-cols-4 w-full' : 'grid-cols-4 w-auto',
    )}>
      {(['folders', 'aiWorkbench', 'lineage', 'trash'] as ViewMode[]).map((mode) => (
        <button
          key={mode}
          onClick={() => handleSelectViewMode(mode)}
          className={`min-w-0 rounded-[12px] px-3 py-1.5 text-[12px] font-medium transition-colors ${
            viewMode === mode
              ? 'bg-primary text-primary-foreground'
              : 'text-muted-foreground hover:bg-[hsl(var(--ui-surface-panel-strong))] hover:text-foreground'
          } ${isMobile ? 'flex-1' : ''}`}
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
          <SelectItem value="all">{t('documents.bindingFilterAll', '全部占用')}</SelectItem>
          <SelectItem value="bound">{t('documents.bindingFilterBound', '已绑定')}</SelectItem>
          <SelectItem value="read">{t('documents.bindingFilterRead', '被读取')}</SelectItem>
                <SelectItem value="draft">{t('documents.bindingDraft', '草稿协作')}</SelectItem>
                <SelectItem value="write">{t('documents.bindingWrite', '允许直写')}</SelectItem>
          <SelectItem value="unbound">{t('documents.bindingFilterUnbound', '未绑定')}</SelectItem>
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
      <div className={cn('flex items-center justify-between px-3 py-2.5', documentConsoleSubpanelClass)}>
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
        <div className={cn('min-h-[360px] overflow-hidden', documentConsoleShellClass)}>
          <AiWorkbench teamId={teamId} canManage={canManage} />
        </div>
      ) : viewMode === 'lineage' ? (
        lineageDocId ? (
          <div className={cn('min-h-[360px] overflow-hidden', documentConsoleShellClass)}>
            <DocumentLineage
              teamId={teamId}
              docId={lineageDocId}
              onNavigate={(id) => setLineageDocId(id)}
            />
          </div>
        ) : (
          <div className="space-y-4">
            <div className={cn('px-3.5 py-3', documentConsoleShellClass)}>
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
                  className={cn(
                    'w-full px-3.5 py-3 text-left transition-colors hover:bg-[hsl(var(--ui-surface-panel))]',
                    documentConsoleShellClass,
                  )}
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
                <div className="rounded-[18px] border border-dashed border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-3.5 py-4 text-[11px] text-muted-foreground">
                  {t('documents.noAiDocuments')}
                </div>
              )}
            </div>
          </div>
        )
      ) : viewMode === 'trash' ? (
        <div className="space-y-3">
          {archivedLoading ? (
            <div className={cn('px-3.5 py-5 text-center text-[11px] text-muted-foreground', documentConsoleShellClass)}>
              {t('common.loading')}
            </div>
          ) : archivedDocuments.length === 0 ? (
            <div className="rounded-[18px] border border-dashed border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-3.5 py-4 text-[11px] text-muted-foreground">
              {t('documents.noArchivedDocuments', '暂无归档文档')}
            </div>
          ) : (
            archivedDocuments.map((doc) => (
              <DocumentFileCard
                key={doc.id}
                name={doc.display_name || doc.name}
                mimeType={doc.mime_type}
                metaLabel={getDocumentMetaLabel(doc)}
                compact
                actionOpen={openFileActionId === doc.id}
                onOpen={() => {
                  handleDocClick(doc);
                  setMobileLibrarySheetOpen(false);
                }}
                onToggleActions={() => setOpenFileActionId((prev) => (prev === doc.id ? null : doc.id))}
                actions={buildDocumentActions(doc, { archived: true })}
              />
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
          <div className={cn('px-3.5 py-3', documentConsoleShellClass)}>
            <div className="flex items-center gap-3">
              <button
                type="button"
                className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[12px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] text-muted-foreground"
                onClick={() => {
                  setMobileLibrarySheetOpen(false);
                  openMobileFolderPanel();
                }}
              >
                <FolderOpen className="h-4 w-4" />
              </button>
              <div className="min-w-0 flex-1">
                <div className="flex min-w-0 items-center gap-2 text-[10px] font-medium text-muted-foreground">
                  <span>{t('documents.currentFolder', 'Current folder')}</span>
                  <span>•</span>
                  <span>{t('documents.folderResultCount', '{{count}} documents in the current folder', { count: visibleDocs.length })}</span>
                </div>
                <div className="mt-1 truncate text-[13px] font-semibold text-foreground">
                  {currentFolderPath || '/'}
                </div>
              </div>
              <Button
                size="sm"
                variant="ghost"
                className="h-8 rounded-[12px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-3 text-[11px] text-muted-foreground shadow-none hover:bg-background hover:text-foreground"
                onClick={() => {
                  setMobileLibrarySheetOpen(false);
                  openMobileFilterPanel('library');
                }}
              >
                <SlidersHorizontal className="mr-1.5 h-3.5 w-3.5" />
                {t('documents.quickFilters', '筛选')}
              </Button>
            </div>
          </div>
          <div className="space-y-2">
            {visibleDocs.length > 0 ? visibleDocs.map((doc) => (
              <DocumentFileCard
                key={doc.id}
                name={doc.display_name || doc.name}
                mimeType={doc.mime_type}
                metaLabel={getDocumentMetaLabel(doc)}
                thumbnailUrl={doc.mime_type.startsWith('image/') ? documentApi.getDownloadUrl(teamId, doc.id) : undefined}
                active={panel.doc?.id === doc.id}
                compact
                actionOpen={openFileActionId === doc.id}
                onOpen={() => {
                  handleDocClick(doc);
                  setMobileLibrarySheetOpen(false);
                }}
                onToggleActions={() => setOpenFileActionId((prev) => (prev === doc.id ? null : doc.id))}
                actions={buildDocumentActions(doc)}
              />
            )) : (
              <div className="space-y-3 rounded-[18px] border border-dashed border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-3.5 py-4">
                <div className="text-[11px] text-muted-foreground">
                  {visibleChildFolders.length > 0
                    ? t('documents.emptyCurrentFolderWithChildren', 'There are no direct files in the current folder. Continue into the next level.')
                    : t('documents.empty', '当前条件下没有文档')}
                </div>
                {renderChildFolderButtons(true)}
                {visibleChildFolders.length > childFolderPageSize ? renderChildFolderPagination(true) : null}
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
        <div className="mb-4 flex items-center gap-2 border-b border-[hsl(var(--ui-line-soft))] pb-3">
          {viewModeSwitcher}
        </div>
      )}

      {/* AI Workbench View */}
      {viewMode === 'aiWorkbench' && (
        <div className={cn('flex-1 min-h-0 overflow-hidden px-4 py-1', documentConsoleShellClass)}>
          <AiWorkbench teamId={teamId} canManage={canManage} />
        </div>
      )}

      {/* Lineage View */}
      {viewMode === 'lineage' && (
        <div className={cn('flex-1 overflow-auto p-4', documentConsoleShellClass)}>
          {lineageDocId ? (
            <DocumentLineage
              teamId={teamId}
              docId={lineageDocId}
              onNavigate={(id) => setLineageDocId(id)}
            />
          ) : (
            <div className="rounded-[18px] bg-[hsl(var(--ui-surface-panel))] py-8 text-center text-sm text-muted-foreground">
              {t('documents.noAiDocuments')}
            </div>
          )}
        </div>
      )}

      {/* Trash View */} 
      {viewMode === 'trash' && (
        <div className={cn('flex-1 overflow-auto p-4', documentConsoleShellClass)}>
          <div className="border-b border-[hsl(var(--ui-line-soft))] px-1 pb-3">
            <div className="text-[11px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
              {t('documents.viewMode.trash')}
            </div>
            <div className="mt-1 text-sm font-semibold text-foreground">{t('documents.archivedDocuments')}</div>
          </div>
          <div className="space-y-2 px-1 pt-3">
              {archivedLoading ? (
                <div className="rounded-[18px] bg-[hsl(var(--ui-surface-panel))] py-8 text-center text-sm text-muted-foreground">{t('common.loading')}</div>
              ) : archivedDocuments.length === 0 ? (
                <div className="rounded-[18px] bg-[hsl(var(--ui-surface-panel))] py-8 text-center text-sm text-muted-foreground">{t('documents.archivedEmpty')}</div>
              ) : (
                archivedDocuments.map((doc) => (
                  <DocumentFileCard
                    key={doc.id}
                    name={doc.display_name || doc.name}
                    mimeType={doc.mime_type}
                    metaLabel={getDocumentMetaLabel(doc)}
                    active={panel.doc?.id === doc.id}
                    actionOpen={openFileActionId === doc.id}
                    onOpen={() => handleDocClick(doc)}
                    onToggleActions={() => setOpenFileActionId((prev) => (prev === doc.id ? null : doc.id))}
                    actions={buildDocumentActions(doc, { archived: true })}
                  />
                ))
              )}

              {archivedTotalPages > 1 && (
                <div className="rounded-[16px] bg-[hsl(var(--ui-surface-panel))] px-3 py-2">
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-[11px] text-muted-foreground">{archivedPage} / {archivedTotalPages}</span>
                    <div className="flex items-center gap-2">
                      <Button size="sm" variant="outline" disabled={archivedPage <= 1} onClick={() => setArchivedPage(p => p - 1)}>
                        {t('pagination.previous')}
                      </Button>
                      <Button size="sm" variant="outline" disabled={archivedPage >= archivedTotalPages} onClick={() => setArchivedPage(p => p + 1)}>
                        {t('pagination.next')}
                      </Button>
                    </div>
                  </div>
                </div>
              )}
          </div>
        </div>
      )}

      {/* Folders View (original) */}
      {viewMode === 'folders' && (
      <div className={`flex flex-1 min-h-0 ${isMobile ? 'flex-col gap-4' : `overflow-hidden ${documentConsoleShellClass}`}`}>
      {!isMobile && showFolderTree ? (
        <div className="w-[330px] flex-shrink-0 min-h-0 border-r border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-2.5 py-2 xl:w-[350px]">
          {userUploadRoot ? (
            <div className="mb-2 border-b border-[hsl(var(--ui-line-soft))] pb-2">
              <button
                type="button"
                onClick={isBrowsingUserUploads ? handleUserUploadBack : openUserUploadLibrary}
                className={`flex w-full items-center gap-2.5 rounded-[14px] border px-3 py-2.5 text-left transition-colors ${
                  isBrowsingUserUploads
                    ? 'border-primary/25 bg-primary/[0.06]'
                    : 'border-[hsl(var(--ui-line-soft))] bg-background hover:bg-[hsl(var(--ui-surface-selected))]'
                }`}
              >
                <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[12px] bg-[hsl(var(--ui-surface-panel))] text-muted-foreground">
                  <Upload className="h-4 w-4" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block text-[10px] font-medium uppercase tracking-[0.08em] text-muted-foreground/82">
                    {isBrowsingUserUploads ? t('common.back', 'Back') : t('documents.standaloneEntry', 'Standalone entry')}
                  </span>
                  <span className="mt-0.5 block text-[12.5px] font-semibold text-foreground">
                    {isBrowsingUserUploads ? userUploadBackLabel : t('documents.userUploads', 'User uploads')}
                  </span>
                </span>
                {!isBrowsingUserUploads && userUploadFolderCount > 0 ? (
                  <span className="shrink-0 rounded-full border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-2 py-0.5 text-[10px] text-muted-foreground">
                    {userUploadFolderCount}
                  </span>
                ) : null}
              </button>
            </div>
          ) : null}
          {renderFolderNavigator('desktop')}
        </div>
      ) : null}

      {/* Document List */}
      <Card
        className={`flex min-h-0 flex-1 min-w-0 flex-col rounded-none border-0 bg-transparent shadow-none ${isDragging ? 'ring-2 ring-primary ring-dashed' : ''}`}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
      >
        <CardHeader className={`shrink-0 ${isDesktopFoldersLayout ? 'space-y-3 border-b border-[hsl(var(--ui-line-soft))] px-5 py-4' : isMobile ? 'space-y-2.5 px-0 py-0' : 'space-y-2.5 py-3'}`}>
          {isMobile ? (
            <>
              <div className={cn('px-3 py-2.5', documentConsoleShellClass)}>
                <div className="flex items-center gap-2">
                  <div className="relative min-w-0 flex-1">
                    <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground/80" />
                    <Input
                      placeholder={t('documents.search')}
                      value={searchQuery}
                      onChange={(e) => setSearchQuery(e.target.value)}
                      className="h-9 min-w-0 rounded-[13px] border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] pl-9 pr-3 text-[13px] shadow-none"
                    />
                  </div>
                  {canManage ? (
                    <Button size="sm" className="h-9 shrink-0 rounded-[13px] px-4 text-[12px] shadow-none" onClick={handleUploadClick} disabled={uploading}>
                      {uploading ? t('documents.uploading') : t('documents.upload')}
                    </Button>
                  ) : null}
                  <input
                    ref={fileInputRef}
                    type="file"
                    multiple
                    className="hidden"
                    onChange={handleFileChange}
                  />
                </div>
                <div className="mt-2 flex items-center gap-2">
                  <Select value={bindingFilter} onValueChange={(value) => setBindingFilter(value as BindingFilterMode)}>
                    <SelectTrigger className="h-8 min-w-0 flex-1 rounded-[12px] border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] text-[11px] shadow-none">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="all">{t('documents.bindingFilterAll', 'All usage')}</SelectItem>
                      <SelectItem value="bound">{t('documents.bindingFilterBound', 'Bound')}</SelectItem>
                      <SelectItem value="read">{t('documents.bindingFilterRead', 'Read')}</SelectItem>
                  <SelectItem value="draft">{t('documents.bindingDraft', 'Draft collaboration')}</SelectItem>
                  <SelectItem value="write">{t('documents.bindingWrite', 'Controlled write')}</SelectItem>
                      <SelectItem value="unbound">{t('documents.bindingFilterUnbound', 'Unbound')}</SelectItem>
                    </SelectContent>
                  </Select>
                  <Button
                    size="sm"
                    variant="outline"
                    className="h-8 shrink-0 rounded-[12px] border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-3 text-[11px] font-medium text-muted-foreground shadow-none"
                    onClick={() => openMobileFilterPanel('workspace')}
                  >
                    <SlidersHorizontal className="mr-1.5 h-3.5 w-3.5" />
                    {t('documents.quickFilters', '筛选')}
                  </Button>
                </div>
                {classicMobileFolderSummary}
              </div>
            </>
          ) : (
            <>
              <div className={`flex ${isDesktopFoldersLayout ? 'items-start justify-between gap-3' : 'items-center justify-between'}`}>
                <div className="flex min-w-0 items-center gap-2">
                  <CardTitle className="text-sm">{t('documents.files')}</CardTitle>
                  <Button
                    size="sm"
                    variant="outline"
                    className="h-8 rounded-[12px] border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] shadow-none"
                    onClick={() => setShowFolderTree((prev) => !prev)}
                  >
                    <FolderOpen className="mr-1.5 h-3.5 w-3.5" />
                    {showFolderTree
                      ? t('documents.hideFolderNavigator', '收起文件夹')
                      : t('documents.showFolderNavigator', '显示文件夹')}
                  </Button>
                </div>
                <div className={`flex items-center gap-2 flex-wrap ${isDesktopFoldersLayout ? 'justify-end' : ''}`}>
                  <Input
                    placeholder={t('documents.search')}
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    className={`${isDesktopFoldersLayout ? 'w-60' : 'w-36'} h-9 rounded-[14px] border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))]`}
                  />
                  {canManage && (
                    <Button size="sm" className="h-9 rounded-[14px] px-4 shadow-none" onClick={handleUploadClick} disabled={uploading}>
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
              <div className={`flex items-center gap-2 flex-wrap ${isDesktopFoldersLayout ? `px-3 py-2.5 ${documentConsoleSubpanelClass}` : ''}`}>
                <Select value={bindingFilter} onValueChange={(value) => setBindingFilter(value as BindingFilterMode)}>
                  <SelectTrigger className="h-8 w-32 rounded-[12px] border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel-strong))] shadow-none">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{t('documents.bindingFilterAll', 'All usage')}</SelectItem>
                    <SelectItem value="bound">{t('documents.bindingFilterBound', 'Bound')}</SelectItem>
                    <SelectItem value="read">{t('documents.bindingFilterRead', 'Read')}</SelectItem>
                  <SelectItem value="draft">{t('documents.bindingDraft', 'Draft collaboration')}</SelectItem>
                  <SelectItem value="write">{t('documents.bindingWrite', 'Controlled write')}</SelectItem>
                    <SelectItem value="unbound">{t('documents.bindingFilterUnbound', 'Unbound')}</SelectItem>
                  </SelectContent>
                </Select>
                <Select value={mimeFilter || '__all__'} onValueChange={v => setMimeFilter(v === '__all__' ? '' : v)}>
                  <SelectTrigger className="h-8 w-32 rounded-[12px] border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel-strong))] shadow-none">
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
                    <SelectTrigger className="h-8 w-32 rounded-[12px] border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel-strong))] shadow-none">
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
                  <SelectTrigger className="h-8 w-28 rounded-[12px] border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel-strong))] shadow-none">
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
                  <Button size="sm" variant="outline" className="h-8 rounded-[12px] border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel-strong))] shadow-none" onClick={() => setSelectionMode(true)}>
                    <CheckSquare className="w-4 h-4 mr-1" />
                    {t('documents.selectMode')}
                  </Button>
                )}
                {canManage && selectionMode && (
                  <Button size="sm" variant="ghost" className="h-8 rounded-[12px]" onClick={exitSelectionMode}>
                    <X className="w-4 h-4 mr-1" />
                    {t('documents.exitSelectMode')}
                  </Button>
                )}
              </div>
            </>
          )}
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
        <CardContent className={`min-h-0 flex-1 overflow-auto ${isDesktopFoldersLayout ? 'px-5 py-4' : isMobile ? 'px-0 pb-0 pt-2' : ''}`}>
          {loadingDocuments ? (
            <div className="mb-3 flex items-center gap-2 px-1 text-[11px] text-muted-foreground">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t('documents.refreshingCurrentFolder', 'Refreshing current folder...')}
            </div>
          ) : null}
          {isBrowsingUserUploads ? (
            <div className="mb-3 flex flex-wrap items-center justify-between gap-2 rounded-[14px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-3.5 py-2.5">
              <div className="min-w-0">
                <div className="text-[12px] font-semibold text-foreground">
                    {t('documents.userUploads', 'User uploads')}
                </div>
                <div className="mt-0.5 text-[10.5px] text-muted-foreground">
                  {t('documents.userUploadOverview', 'Upload folders {{folders}} · documents {{docs}} · uploaders {{uploaders}}', {
                    folders: userUploadOverview.folderCount,
                    docs: userUploadOverview.docCount,
                    uploaders: userUploadOverview.uploaderCount,
                  })}
                  {userUploadOverview.latestUpdatedAt ? ` · ${t('documents.latestUpdatedAt', 'Latest update {{time}}', { time: formatDate(userUploadOverview.latestUpdatedAt) })}` : ''}
                </div>
              </div>
              <Button
                size="sm"
                variant="outline"
                className="h-7.5 shrink-0 rounded-[11px] px-3 text-[10.5px]"
                onClick={handleUserUploadBack}
              >
                {userUploadBackLabel}
              </Button>
            </div>
          ) : null}
          {/* Breadcrumb */}
          {!isMobile && breadcrumbs.length > 0 && (
            <div className="flex items-center gap-1 text-xs text-muted-foreground mb-2 px-1">
              <button className="hover:text-foreground" onClick={() => setCurrentFolderPath(isBrowsingUserUploads ? USER_UPLOAD_ROOT_PATH : null)}>
                  {isBrowsingUserUploads ? t('documents.userUploads', 'User uploads') : t('documents.allFiles')}
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
              {t('documents.currentFilterResult', 'Filtered on this page: {{visible}} / {{total}}', {
                visible: visibleDocs.length,
                total: sortedDocs.length,
              })}
            </div>
          )}
          {isDragging && (
            <div className="flex items-center justify-center py-12 border-2 border-dashed border-primary rounded-lg mb-4">
              <span className="text-muted-foreground">{t('documents.dragDropHint')}</span>
            </div>
          )}
          {visibleDocs.length === 0 ? (
            <div className={cn(
              visibleChildFolders.length > 0
                ? 'w-full rounded-[16px] border border-[hsl(var(--ui-line-soft))] bg-background px-4 py-4'
                : isBrowsingUserUploads
                  ? 'w-full rounded-[16px] border border-[hsl(var(--ui-line-soft))] bg-background px-4 py-4'
                  : 'w-full rounded-[18px] border border-dashed border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-5 py-5',
            )}>
              <div className={cn(
                visibleChildFolders.length > 0 || isBrowsingUserUploads
                  ? 'text-left text-[11px] text-muted-foreground'
                  : 'text-center text-sm text-muted-foreground',
              )}>
                {bindingFilter === 'all'
                  ? (visibleChildFolders.length > 0
                    ? t('documents.emptyCurrentFolderWithChildren', 'There are no direct files in the current folder. Continue into the next level.')
                    : t('documents.empty'))
                  : bindingUsageLoading
                    ? t('documents.filteringByBindingUsage', 'Filtering by portal usage...')
                    : t('documents.emptyWithCurrentFilters', 'No documents match the current filters')}
              </div>
              {bindingFilter === 'all' && visibleChildFolders.length > 0 ? (
                <div className={cn(
                  'border-[hsl(var(--ui-line-soft))]',
                  'mt-3 pt-3',
                  !isBrowsingUserUploads && 'border-t',
                )}>
                  {renderChildFolderButtons()}
                  {visibleChildFolders.length > childFolderPageSize ? renderChildFolderPagination() : null}
                </div>
              ) : null}
            </div>
          ) : (
            <div className="space-y-1.5">
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
                <DocumentFileCard
                  key={doc.id}
                  name={doc.display_name || doc.name}
                  mimeType={doc.mime_type}
                  metaLabel={getDocumentMetaLabel(doc)}
                  thumbnailUrl={doc.mime_type.startsWith('image/') ? documentApi.getDownloadUrl(teamId, doc.id) : undefined}
                  active={panel.doc?.id === doc.id}
                  selectionMode={canManage && selectionMode}
                  selected={selectedIds.has(doc.id)}
                  onToggleSelect={() => toggleSelect(doc.id)}
                  actionOpen={openFileActionId === doc.id}
                  onOpen={() => handleDocClick(doc)}
                  onToggleActions={() => setOpenFileActionId((prev) => (prev === doc.id ? null : doc.id))}
                  actions={buildDocumentActions(doc)}
                  compact={isMobile}
                  footer={!isMobile ? renderDocumentCardFooter(doc) : undefined}
                />
              ))}
            </div>
          )}
        </CardContent>
        {showDocumentPagination && (
          <div
            className={`${isMobile ? 'px-0 pb-[calc(env(safe-area-inset-bottom,0px)+88px)] pt-2' : 'px-6 py-2'} shrink-0 border-t border-[hsl(var(--ui-line-soft))]`}
          >
            {renderDocumentPagination(true)}
          </div>
        )}
      </Card>

      {/* Right Panel: Preview / Edit / Versions / Diff */}
      {hasRightPanel && panel.doc && (
        <Card className={isMobile ? 'fixed inset-0 z-50 overflow-hidden rounded-none border-0' : 'relative w-[min(45%,420px)] min-w-[300px] rounded-none border-0 border-l border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel-strong))] shadow-none'}>
          {panel.mode === 'preview' && (
            <div className="flex h-full min-h-0 flex-col">
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
              <div className="shrink-0">
                {renderBindingUsageDetail(panel.doc)}
              </div>
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
        <DialogContent hideCloseButton className="left-0 top-0 h-[100dvh] max-h-[100dvh] w-screen max-w-none translate-x-0 translate-y-0 gap-0 overflow-x-hidden rounded-none border-0 px-0 pb-0 pt-0 sm:max-h-[100dvh] sm:max-w-none">
          <div className="flex h-full min-h-0 flex-col overflow-x-hidden bg-background">
            <div className="border-b border-border/55 px-4 py-3">
              <div className="flex items-center gap-3">
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  className="h-8 shrink-0 rounded-full px-2.5 text-[11px] text-muted-foreground"
                  onClick={() => setMobileFolderSheetOpen(false)}
                >
                  {t('documents.backToDocuments', '返回文档')}
                </Button>
                <div className="min-w-0">
                  <div className="text-[12px] font-semibold text-foreground">
                    {t('documents.folderNavigator', 'Folder navigator')}
                  </div>
                  <div className="mt-0.5 truncate text-[10px] text-muted-foreground/80">
                    {currentFolderPath || '/'}
                  </div>
                </div>
              </div>
            </div>
            <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto px-4 py-4">
              {renderFolderNavigator('mobile', { embedded: true, showHeader: false })}
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
            <div className="rounded-[14px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-3 py-2 text-[11px] text-muted-foreground">
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
        description={
          deleteFolderTarget
            ? `Deleting this will also remove documents under "${deleteFolderTarget.name}" and all of its subfolders. This action cannot be undone.`
            : undefined
        }
        variant="destructive"
        onConfirm={confirmDeleteFolder}
        loading={deletingFolder}
        confirmText={t('documents.deleteFolderAndDocuments', 'Delete folder and documents')}
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
                'Focus on advancing work around the current material first. Lists, filters, and view switching move into the supporting panel.',
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
                {t('documents.openFolderNavigator', 'Folder navigator')}
              </Button>
              {userUploadRoot ? (
                <Button variant="outline" className="h-9 justify-start rounded-[14px] px-3 text-[11px]" onClick={openUserUploadLibrary}>
                  <Upload className="mr-2 h-4 w-4" />
                  {t('documents.userUploads', 'User uploads')}
                </Button>
              ) : null}
              <Button variant="outline" className="h-9 justify-start rounded-[14px] px-3 text-[11px]" onClick={openMobileLibraryPanel}>
                <FolderOpen className="mr-2 h-4 w-4" />
                {t('documents.openLibrary', '打开文档面板')}
              </Button>
              <Button variant="outline" className="h-9 justify-start rounded-[14px] px-3 text-[11px]" onClick={() => openMobileViewPanel('workspace')}>
                <LayoutGrid className="mr-2 h-4 w-4" />
                {t('documents.quickViews', '切换视图')}
              </Button>
              <Button variant="outline" className="h-9 justify-start rounded-[14px] px-3 text-[11px]" onClick={() => openMobileFilterPanel('workspace')}>
                <SlidersHorizontal className="mr-2 h-4 w-4" />
                {t('documents.quickFilters', '筛选与排序')}
              </Button>
              <Button
                variant="outline"
                className="h-9 justify-start rounded-[14px] px-3 text-[11px]"
                onClick={() => navigate(`/teams/${teamId}?section=collaboration`)}
              >
                <MessageSquareText className="mr-2 h-4 w-4" />
                {t('documents.quickChat', '进入智能协作')}
              </Button>
            </div>
          )}
          stage={(
            <div className="flex h-full min-h-[320px] flex-col gap-3 p-3">
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
              </div>
            </div>
          )}
          rail={(
            <ManagementRail
              title={focusDocument ? t('documents.currentContext', '当前文档上下文') : t('documents.contextRail', '文档与产物上下文')}
              description={t(
                'documents.mobileConversationRail',
                'In conversation mode, the documents page only serves as material and artifact context: browsing, preview, versioning, and the AI workbench all appear as part of the current task context.',
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
                    <span className="text-muted-foreground">{t('documents.currentFolder', 'Current folder')}</span>
                    <span className="text-right font-semibold text-foreground">{currentFolderPath || '/'}</span>
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    className="mt-1 h-8 w-full justify-center rounded-[12px] text-[11px]"
                    onClick={openMobileFolderPanel}
                  >
                    {t('documents.openFolderNavigator', 'Folder navigator')}
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
                title={t('documents.folderNavigator', 'Folder navigator')}
                fullHeight
                onBack={() => setMobileFolderSheetOpen(false)}
                backLabel={t('common.back', '返回')}
                hideCloseButton
              >
                {renderFolderNavigator('mobile', { embedded: true, showHeader: false })}
              </BottomSheetPanel>
              <BottomSheetPanel
                open={mobileLibrarySheetOpen}
                onOpenChange={setMobileLibrarySheetOpen}
                title={t('documents.openLibrary', '文档面板')}
                fullHeight
                onBack={() => setMobileLibrarySheetOpen(false)}
                backLabel={t('common.back', '返回')}
                hideCloseButton
              >
                <div className="space-y-3">
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
                onBack={handleMobileViewBack}
                backLabel={t('common.back', '返回')}
                hideCloseButton
              >
                {viewModeSwitcher}
              </BottomSheetPanel>
              <BottomSheetPanel
                open={mobileFilterSheetOpen}
                onOpenChange={setMobileFilterSheetOpen}
                title={t('documents.quickFilters', '筛选与排序')}
                onBack={handleMobileFilterBack}
                backLabel={t('common.back', '返回')}
                hideCloseButton
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
