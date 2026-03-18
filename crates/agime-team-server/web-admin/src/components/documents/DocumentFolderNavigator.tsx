import { useEffect, useMemo, useState } from 'react';
import { ChevronDown, ChevronRight, Folder, FolderOpen, Globe2, Pencil, Plus, Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { FolderTreeNode } from '../../api/documents';
import { cn } from '../../utils';
import { Button } from '../ui/button';

interface DocumentFolderNavigatorProps {
  nodes: FolderTreeNode[];
  currentPath: string | null;
  onSelectPath: (path: string | null) => void;
  canManage?: boolean;
  onCreateFolder?: (parentPath: string | null) => void;
  onRenameFolder?: (node: FolderTreeNode) => void;
  onDeleteFolder?: (node: FolderTreeNode) => void;
  storageKey: string;
  variant?: 'desktop' | 'mobile';
  className?: string;
  embedded?: boolean;
}

function readExpandedState(storageKey: string): Set<string> {
  if (typeof window === 'undefined') {
    return new Set();
  }

  try {
    const raw = window.localStorage.getItem(storageKey);
    if (!raw) {
      return new Set();
    }
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? new Set(parsed.filter((item) => typeof item === 'string')) : new Set();
  } catch {
    return new Set();
  }
}

function collectAncestorPaths(path: string | null): string[] {
  if (!path || path === '/') {
    return [];
  }
  const parts = path.split('/').filter(Boolean);
  const ancestors: string[] = [];
  for (let index = 0; index < parts.length; index += 1) {
    ancestors.push(`/${parts.slice(0, index + 1).join('/')}`);
  }
  return ancestors;
}

function sortNodes(nodes: FolderTreeNode[]): FolderTreeNode[] {
  return [...nodes].sort((a, b) => {
    const systemDelta = Number(Boolean(b.is_system)) - Number(Boolean(a.is_system));
    if (systemDelta !== 0) {
      return systemDelta;
    }
    return a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });
  });
}

export function DocumentFolderNavigator({
  nodes,
  currentPath,
  onSelectPath,
  canManage = false,
  onCreateFolder,
  onRenameFolder,
  onDeleteFolder,
  storageKey,
  variant = 'desktop',
  className,
  embedded = false,
}: DocumentFolderNavigatorProps) {
  const { t } = useTranslation();
  const isMobileVariant = variant === 'mobile';
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(() => readExpandedState(storageKey));
  const [mobileActionPath, setMobileActionPath] = useState<string | null>(null);

  useEffect(() => {
    setExpandedPaths(readExpandedState(storageKey));
  }, [storageKey]);

  useEffect(() => {
    const ancestors = collectAncestorPaths(currentPath);
    if (ancestors.length === 0) {
      return;
    }

    setExpandedPaths((prev) => {
      const next = new Set(prev);
      let changed = false;
      ancestors.forEach((path) => {
        if (!next.has(path)) {
          next.add(path);
          changed = true;
        }
      });
      return changed ? next : prev;
    });
  }, [currentPath]);

  useEffect(() => {
    if (typeof window === 'undefined') {
      return;
    }

    window.localStorage.setItem(storageKey, JSON.stringify(Array.from(expandedPaths)));
  }, [expandedPaths, storageKey]);

  const hasFolders = nodes.length > 0;
  const currentLabel = useMemo(() => {
    if (!currentPath || currentPath === '/') {
      return t('documents.allFiles');
    }
    return currentPath;
  }, [currentPath, t]);

  const togglePath = (path: string) => {
    setExpandedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  const renderNode = (node: FolderTreeNode, depth = 0) => {
    const isExpanded = expandedPaths.has(node.fullPath);
    const isSelected = currentPath === node.fullPath;
    const hasChildren = node.children.length > 0;
    const showMobileActions = isMobileVariant && canManage && !node.is_system && mobileActionPath === node.fullPath;

    return (
      <div key={node.id} className="space-y-1">
        <div
          className={cn(
            'group flex min-w-0 items-center gap-2 overflow-hidden rounded-[14px] border px-2 py-1.5 transition-colors',
            isSelected
              ? 'border-primary/35 bg-primary/8 text-foreground'
              : 'border-transparent bg-transparent text-foreground hover:border-border/50 hover:bg-muted/55',
            isMobileVariant ? 'min-h-11' : 'min-h-9',
          )}
          style={{ paddingLeft: `${depth * (isMobileVariant ? 14 : 12) + 10}px` }}
        >
          <button
            type="button"
            className={cn(
              'flex h-7 w-7 shrink-0 items-center justify-center rounded-full border border-transparent text-muted-foreground transition-colors hover:border-border/60 hover:bg-background',
              !hasChildren && 'pointer-events-none opacity-0',
            )}
            aria-label={isExpanded ? t('documents.collapseFolder', '收起文件夹') : t('documents.expandFolder', '展开文件夹')}
            onClick={() => hasChildren && togglePath(node.fullPath)}
          >
            {hasChildren ? (
              isExpanded ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />
            ) : null}
          </button>

          <button
            type="button"
            className="flex min-w-0 flex-1 items-center gap-2 overflow-hidden text-left"
            onClick={() => onSelectPath(node.fullPath)}
          >
            <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-[hsl(var(--ui-surface-panel-muted))/0.42] text-muted-foreground">
              {node.is_system ? (
                <Globe2 className="h-3.5 w-3.5" />
              ) : isSelected || isExpanded ? (
                <FolderOpen className="h-3.5 w-3.5" />
              ) : (
                <Folder className="h-3.5 w-3.5" />
              )}
            </span>
            <span className="min-w-0">
              <span className="block truncate text-[12px] font-medium">{node.name}</span>
              {isSelected ? (
                <span className="block truncate text-[10px] text-muted-foreground">
                  {t('documents.currentFolder', '当前目录')}
                </span>
              ) : null}
            </span>
          </button>

          {canManage && !node.is_system ? (
            isMobileVariant ? (
              <Button
                size="sm"
                variant="ghost"
                className="h-8 shrink-0 rounded-full px-3 text-[11px] text-muted-foreground"
                title={t('common.actions', '操作')}
                onClick={(event) => {
                  event.stopPropagation();
                  setMobileActionPath((prev) => (prev === node.fullPath ? null : node.fullPath));
                }}
              >
                {t('common.actions', '操作')}
              </Button>
            ) : (
              <div className="flex shrink-0 items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100">
                {onCreateFolder ? (
                  <Button
                    size="icon"
                    variant="ghost"
                    className="h-7 w-7 rounded-full"
                    title={t('documents.createFolder')}
                    onClick={() => onCreateFolder(node.fullPath)}
                  >
                    <Plus className="h-3.5 w-3.5" />
                  </Button>
                ) : null}
                {onRenameFolder ? (
                  <Button
                    size="icon"
                    variant="ghost"
                    className="h-7 w-7 rounded-full"
                    title={t('documents.renameFolder')}
                    onClick={() => onRenameFolder(node)}
                  >
                    <Pencil className="h-3.5 w-3.5" />
                  </Button>
                ) : null}
                {onDeleteFolder ? (
                  <Button
                    size="icon"
                    variant="ghost"
                    className="h-7 w-7 rounded-full text-destructive hover:text-destructive"
                    title={t('common.delete')}
                    onClick={() => onDeleteFolder(node)}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                ) : null}
              </div>
            )
          ) : null}
        </div>
        {showMobileActions ? (
          <div
            className="ml-10 flex flex-wrap gap-2 rounded-[14px] border border-border/60 bg-[hsl(var(--ui-surface-panel-muted))/0.32] p-2"
            style={{ marginLeft: `${depth * 14 + 46}px` }}
          >
            {onCreateFolder ? (
              <Button
                size="sm"
                variant="outline"
                className="h-8 rounded-[12px] px-3 text-[11px]"
                onClick={() => onCreateFolder(node.fullPath)}
              >
                <Plus className="mr-1.5 h-3.5 w-3.5" />
                {t('documents.createFolder')}
              </Button>
            ) : null}
            {onRenameFolder ? (
              <Button
                size="sm"
                variant="outline"
                className="h-8 rounded-[12px] px-3 text-[11px]"
                onClick={() => onRenameFolder(node)}
              >
                <Pencil className="mr-1.5 h-3.5 w-3.5" />
                {t('documents.renameFolder')}
              </Button>
            ) : null}
            {onDeleteFolder ? (
              <Button
                size="sm"
                variant="outline"
                className="h-8 rounded-[12px] px-3 text-[11px] text-destructive hover:text-destructive"
                onClick={() => onDeleteFolder(node)}
              >
                <Trash2 className="mr-1.5 h-3.5 w-3.5" />
                {t('common.delete')}
              </Button>
            ) : null}
          </div>
        ) : null}
        {hasChildren && isExpanded ? (
          <div className="space-y-1">
            {sortNodes(node.children).map((child) => renderNode(child, depth + 1))}
          </div>
        ) : null}
      </div>
    );
  };

  return (
    <section
      className={cn(
        embedded
          ? 'flex min-h-0 flex-col bg-transparent'
          : 'flex min-h-0 flex-col rounded-[22px] border border-border/65 bg-[linear-gradient(180deg,hsl(var(--ui-surface-panel))/0.99_0%,hsl(var(--ui-surface-panel))/0.95_100%)]',
        className,
      )}
    >
      <div className={cn(embedded ? 'px-3.5 pb-3 pt-2' : 'border-b border-border/55 px-3.5 py-3', isMobileVariant ? 'space-y-3' : 'flex items-start justify-between gap-3')}>
        <div className="min-w-0">
          <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
            {t('documents.folderNavigator', '文件夹导航')}
          </div>
          <div className="mt-1 truncate text-[12px] font-semibold text-foreground">
            {currentLabel}
          </div>
          <div className="mt-0.5 text-[11px] leading-4.5 text-muted-foreground">
            {t('documents.folderNavigatorHint', '展开树节点浏览目录，当前路径会保持高亮。')}
          </div>
        </div>
        {canManage && onCreateFolder ? (
          <Button
            size="sm"
            variant="outline"
            className={cn('h-8 rounded-[12px] text-[11px]', isMobileVariant ? 'w-full justify-center' : '')}
            onClick={() => onCreateFolder(currentPath)}
          >
            <Plus className="mr-1.5 h-3.5 w-3.5" />
            {t('documents.createFolder')}
          </Button>
        ) : null}
      </div>

      <div className={cn('min-h-0 flex-1 overflow-x-hidden overflow-y-auto px-2.5 py-2.5', embedded && 'px-0 pb-0 pt-1')}>
        <div className="space-y-1.5">
          <button
            type="button"
            className={cn(
              'flex min-h-10 w-full items-center gap-2 rounded-[14px] border px-3 py-2 text-left transition-colors',
              currentPath === null
                ? 'border-primary/35 bg-primary/8 text-foreground'
                : 'border-transparent bg-transparent text-foreground hover:border-border/50 hover:bg-muted/55',
            )}
            onClick={() => onSelectPath(null)}
          >
            <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-[hsl(var(--ui-surface-panel-muted))/0.42] text-muted-foreground">
              <FolderOpen className="h-3.5 w-3.5" />
            </span>
            <span className="min-w-0 flex-1">
              <span className="block truncate text-[12px] font-medium">{t('documents.allFiles')}</span>
              <span className="block truncate text-[10px] text-muted-foreground">
                {t('documents.folderRootHint', '返回团队文档的根目录视图')}
              </span>
            </span>
          </button>

          {hasFolders ? (
            sortNodes(nodes).map((node) => renderNode(node))
          ) : (
            <div className="rounded-[16px] border border-dashed border-border/70 px-3.5 py-4 text-[11px] text-muted-foreground">
              {t('documents.emptyFolderTree', '当前还没有文件夹，先创建一个目录开始整理文档。')}
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
