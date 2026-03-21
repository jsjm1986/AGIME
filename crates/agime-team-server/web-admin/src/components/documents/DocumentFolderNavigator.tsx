import { useEffect, useMemo, useRef, useState } from 'react';
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
  showHeader?: boolean;
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
  showHeader = true,
}: DocumentFolderNavigatorProps) {
  const { t } = useTranslation();
  const isMobileVariant = variant === 'mobile';
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(() => readExpandedState(storageKey));
  const [mobileActionPath, setMobileActionPath] = useState<string | null>(null);
  const rootRef = useRef<HTMLElement | null>(null);

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

  useEffect(() => {
    if (!isMobileVariant || !mobileActionPath) {
      return;
    }

    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as HTMLElement | null;
      if (!target) {
        return;
      }
      if (target.closest('[data-folder-action-menu]') || target.closest('[data-folder-action-trigger]')) {
        return;
      }
      setMobileActionPath(null);
    };

    const handleEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setMobileActionPath(null);
      }
    };

    window.addEventListener('pointerdown', handlePointerDown);
    window.addEventListener('keydown', handleEscape);
    return () => {
      window.removeEventListener('pointerdown', handlePointerDown);
      window.removeEventListener('keydown', handleEscape);
    };
  }, [isMobileVariant, mobileActionPath]);

  const hasFolders = nodes.length > 0;
  const currentLabel = useMemo(() => {
    if (!currentPath || currentPath === '/') {
      return t('documents.allFiles');
    }
    return currentPath;
  }, [currentPath, t]);

  const togglePath = (path: string) => {
    setMobileActionPath(null);
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
      <div key={node.id} className="relative space-y-0.5">
        <div
          className={cn(
            'group min-w-0 overflow-hidden rounded-[12px] transition-colors',
            isSelected
              ? 'bg-[hsl(var(--ui-surface-selected))] text-foreground'
              : 'text-foreground hover:bg-[hsl(var(--ui-surface-panel))]',
            isMobileVariant ? 'min-h-9.5' : 'min-h-9',
          )}
        >
          <div
            className="flex min-w-0 items-center gap-2 px-2.5 py-1.5"
            style={{ paddingLeft: `${depth * (isMobileVariant ? 14 : 12) + 10}px` }}
          >
            <button
              type="button"
              className={cn(
                'flex h-6.5 w-6.5 shrink-0 items-center justify-center rounded-full border border-transparent text-muted-foreground transition-colors hover:bg-[hsl(var(--ui-surface-panel-strong))] hover:text-foreground',
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
              onClick={() => {
                setMobileActionPath(null);
                onSelectPath(node.fullPath);
              }}
            >
              <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-[hsl(var(--ui-surface-panel))] text-muted-foreground">
                {node.is_system ? (
                  <Globe2 className="h-3.5 w-3.5" />
                ) : isSelected || isExpanded ? (
                  <FolderOpen className="h-3.5 w-3.5" />
                ) : (
                  <Folder className="h-3.5 w-3.5" />
                )}
              </span>
              <span className="min-w-0">
                <span className="block truncate text-[11.5px] font-medium leading-5">{node.name}</span>
                {isSelected ? (
                  <span className="block truncate text-[9.5px] text-muted-foreground">
                    {t('documents.currentFolder', '当前目录')}
                  </span>
                ) : null}
              </span>
            </button>

            {canManage && !node.is_system ? (
              isMobileVariant ? (
                showMobileActions ? (
                  <div
                    data-folder-action-menu
                    className="absolute right-2 top-8 z-20 w-[132px] overflow-hidden rounded-[12px] border border-[hsl(var(--ui-line-soft))] bg-background shadow-[0_12px_28px_rgba(15,23,42,0.12)]"
                  >
                    <div className="flex flex-col p-1.5">
                      {onCreateFolder ? (
                        <button
                          type="button"
                          className="inline-flex h-8 items-center rounded-[10px] px-2.5 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-[hsl(var(--ui-surface-panel))] hover:text-foreground"
                          onClick={() => {
                            setMobileActionPath(null);
                            onCreateFolder(node.fullPath);
                          }}
                        >
                          {t('documents.createFolder')}
                        </button>
                      ) : null}
                      {onRenameFolder ? (
                        <button
                          type="button"
                          className="inline-flex h-8 items-center rounded-[10px] px-2.5 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-[hsl(var(--ui-surface-panel))] hover:text-foreground"
                          onClick={() => {
                            setMobileActionPath(null);
                            onRenameFolder(node);
                          }}
                        >
                          {t('documents.renameFolder')}
                        </button>
                      ) : null}
                      {onDeleteFolder ? (
                        <button
                          type="button"
                          className="inline-flex h-8 items-center rounded-[10px] px-2.5 text-[11px] font-medium text-destructive transition-colors hover:bg-[hsl(var(--status-error-bg))] hover:text-destructive"
                          onClick={() => {
                            setMobileActionPath(null);
                            onDeleteFolder(node);
                          }}
                        >
                          {t('common.delete')}
                        </button>
                      ) : null}
                    </div>
                  </div>
                ) : (
                  <Button
                    data-folder-action-trigger
                    size="sm"
                    variant="ghost"
                    className="h-7 shrink-0 rounded-full border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-2.5 text-[10.5px] font-medium text-muted-foreground shadow-none hover:bg-background hover:text-foreground"
                    title={t('common.more', '更多')}
                    onClick={(event) => {
                      event.stopPropagation();
                      setMobileActionPath((prev) => (prev === node.fullPath ? null : node.fullPath));
                    }}
                  >
                    <ChevronDown className="mr-1 h-3.5 w-3.5" />
                    {t('common.more', '更多')}
                  </Button>
                )
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
        </div>
        {hasChildren && isExpanded ? (
          <div className="space-y-0.5">
            {sortNodes(node.children).map((child) => renderNode(child, depth + 1))}
          </div>
        ) : null}
      </div>
    );
  };

  return (
    <section
      ref={rootRef}
      className={cn(
        embedded
          ? 'flex min-h-0 flex-col bg-transparent'
          : 'flex min-h-0 flex-col rounded-[22px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel-strong))]',
        className,
      )}
    >
      <div
        className={cn(
          embedded ? 'px-3 pb-2 pt-1.5' : 'border-b border-[hsl(var(--ui-line-soft))] px-3 py-2',
          isMobileVariant ? 'space-y-2' : 'flex items-start justify-between gap-3',
        )}
      >
        {showHeader ? (
          <>
            <div className="min-w-0">
              <div className="text-[9px] font-semibold uppercase tracking-[0.08em] text-muted-foreground/80">
                {t('documents.folderNavigator', '文件夹导航')}
              </div>
              <div className="mt-0.5 truncate text-[11.5px] font-semibold text-foreground">
                {currentLabel}
              </div>
            </div>
            {canManage && onCreateFolder ? (
              <Button
                size="sm"
                variant="outline"
                className={cn('h-8 rounded-[12px] border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-3 text-[10.5px] shadow-none', isMobileVariant ? 'w-full justify-center' : '')}
                onClick={() => onCreateFolder(currentPath)}
              >
                <Plus className="mr-1.5 h-3.5 w-3.5" />
                {t('documents.createFolder')}
              </Button>
            ) : null}
          </>
        ) : canManage && onCreateFolder ? (
          <div className="flex items-center justify-end">
            <Button
              size="sm"
              variant="ghost"
              className="h-8 rounded-full px-3 text-[11px] text-muted-foreground shadow-none hover:bg-[hsl(var(--ui-surface-panel))] hover:text-foreground"
              onClick={() => onCreateFolder(currentPath)}
            >
              <Plus className="mr-1.5 h-3.5 w-3.5" />
              {t('documents.createFolder')}
            </Button>
          </div>
        ) : null}
      </div>

      <div className={cn('min-h-0 flex-1 overflow-x-hidden overflow-y-auto px-2.5 py-2.5', embedded && 'px-0 pb-0 pt-1')}>
        <div className="space-y-0.5">
          <button
            type="button"
            className={cn(
              'flex min-h-10 w-full items-center gap-2 rounded-[12px] px-3 py-2 text-left transition-colors',
              currentPath === null
                ? 'bg-[hsl(var(--ui-surface-selected))] text-foreground'
                : 'text-foreground hover:bg-[hsl(var(--ui-surface-panel))]',
            )}
            onClick={() => onSelectPath(null)}
          >
            <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-[hsl(var(--ui-surface-panel))] text-muted-foreground">
              <FolderOpen className="h-3.5 w-3.5" />
            </span>
            <span className="min-w-0 flex-1">
              <span className="block truncate text-[12px] font-medium">{t('documents.allFiles')}</span>
              <span className="block truncate text-[10px] text-muted-foreground">
                {t('documents.folderRootHint', '返回团队文档的根目录视图')}
              </span>
            </span>
          </button>
          {hasFolders ? sortNodes(nodes).map((node) => renderNode(node)) : (
            <div className="rounded-[14px] border border-dashed border-border/70 px-3 py-5 text-center text-[11px] text-muted-foreground">
              {t('documents.noFolders', '当前没有可浏览的文件夹')}
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
